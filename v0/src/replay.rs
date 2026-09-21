//! `M(S)` — materialisation as replay.
//!
//! Walk the events of a change set in a deterministic order, building a
//! throwaway structure (a weave of atoms plus a [`crate::tree::Tree`]), and read
//! the worktree off it. This is eg-walker's move: keep the log, build CRDT state
//! transiently at replay, discard it.
//!
//! Because `M` reads a **set**, `merge` needs no algorithm: union the sets and
//! the same walk yields the merged result. Commutativity is not implemented, it
//! is a consequence — which is why this module has no `merge` function.

use crate::change::{ChangeSet, Changes};
use crate::event::EventLog;
use crate::op::{Anchor, EventId, NodeId, NodeKind, Op, Side};
use crate::tree::{Node, Tree};
use std::collections::{BTreeMap, BTreeSet};

/// A line that has ever existed, alive or tombstoned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Atom {
    /// The insert that created it — also its permanent position identity.
    pub id: EventId,
    pub line: String,
}

/// What lands on disk: every file's lines, keyed by path.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Worktree {
    pub files: BTreeMap<String, Vec<String>>,
}

/// Swappable materialisers, so a Loro-backed implementation can be benchmarked
/// against ours once the semantics settle (ADR-0004). The trait is deliberately
/// one method wide: a materialiser is a pure function of a set.
pub trait Materialiser {
    fn materialise(&self, set: &ChangeSet, changes: &Changes, log: &EventLog) -> Worktree;
}

/// The reference implementation: order siblings at an anchor by [`EventId`]
/// descending, then walk each subtree.
///
/// The order must be total, deterministic and **permanent**. If two people
/// resolve the same conflict independently and the order is not fixed by the
/// structure, one gets `AXYB` and the other `AYXB`, and merging those has no
/// good answer.
#[derive(Clone, Copy, Default, Debug)]
pub struct WeaveReplay;

impl Materialiser for WeaveReplay {
    fn materialise(&self, set: &ChangeSet, changes: &Changes, log: &EventLog) -> Worktree {
        let events: Vec<EventId> = changes.events_of(set.ids()).into_iter().collect();
        materialise_events(&events, log)
    }
}

/// Replay a raw event set, ignoring change labels.
///
/// This is what a **live session** materialises: events that no change names yet
/// still have to render in the editor. Same function, different granularity —
/// the unification the project is built on.
pub fn materialise_events(events: &[EventId], log: &EventLog) -> Worktree {
    Worktree {
        files: materialise_atoms(events, log)
            .into_iter()
            .map(|(path, atoms)| (path, atoms.into_iter().map(|a| a.line).collect()))
            .collect(),
    }
}

/// A materialised state: which node each path resolves to, and the live atoms
/// of each file.
///
/// Capture needs both — the node to attach new lines to, the atoms to address
/// the existing ones.
#[derive(Clone, Default, Debug)]
pub struct Materialised {
    pub nodes: BTreeMap<String, NodeId>,
    pub files: BTreeMap<String, Vec<Atom>>,
}

/// The same walk, keeping each line's identity.
///
/// Capture needs this: to express "delete this line" it must name the atom, and
/// to express "insert here" it must name the atom to anchor to. Text alone is
/// not addressable — which is exactly the difference between an op log and a
/// pile of diffs.
pub fn materialise_atoms(events: &[EventId], log: &EventLog) -> BTreeMap<String, Vec<Atom>> {
    materialise(events, log).files
}

/// Everything a replay knows: paths, their nodes, and their live atoms.
pub fn materialise(events: &[EventId], log: &EventLog) -> Materialised {
    materialise_parts(events, log, &BTreeSet::new()).1
}

/// Materialise, but show the given removed nodes as if they were not removed.
///
/// This is how a file whose removal is disputed stays visible: the model says
/// it is gone, the person resolving the conflict still needs to read it.
pub fn materialise_with(events: &[EventId], log: &EventLog, revive: &BTreeSet<NodeId>) -> Materialised {
    materialise_parts(events, log, revive).1
}

fn materialise_parts(
    events: &[EventId],
    log: &EventLog,
    revive: &BTreeSet<NodeId>,
) -> (Tree, Materialised) {
    let present: BTreeSet<EventId> = events.iter().copied().collect();
    let op_of = |id: &EventId| log.events.get(id).map(|e| &e.op);

    // --- pass 1: the tree ---------------------------------------------------
    // Ordered by EventId so every replica visits moves in the same sequence;
    // that is what makes the cycle-skip deterministic (I11).
    let mut tree = Tree::default();
    for id in &present {
        match op_of(id) {
            Some(Op::Create { node, parent, name, kind }) => {
                tree.nodes.insert(
                    *node,
                    Node {
                        parent: *parent,
                        name: name.clone(),
                        kind: *kind,
                        mode: 0o644,
                        mode_set_by: None,
                    },
                );
            }
            Some(Op::MoveNode { node, parent, name }) => {
                tree.try_move(*node, *parent, name.clone());
            }
            // In EventId order, which is now causal (Lamport): a restore made
            // after seeing a removal always lands after it.
            Some(Op::Remove { node }) => {
                tree.removed.insert(*node);
            }
            Some(Op::Restore { node }) => {
                tree.removed.remove(node);
            }
            Some(Op::SetMode { node, mode }) => {
                if let Some(n) = tree.nodes.get_mut(node) {
                    // Last writer wins, and "last" means highest EventId --
                    // never wall-clock, which replicas do not agree on.
                    if n.mode_set_by.is_none_or(|prev| prev < *id) {
                        n.mode = *mode;
                        n.mode_set_by = Some(*id);
                    }
                }
            }
            _ => {}
        }
    }

    // --- pass 2: where each atom currently lives ----------------------------
    // An atom's anchor is its insert's, unless a MoveLine overrode it. Moves
    // are applied in EventId order, so the highest id wins by simply being
    // applied last -- and a move that would make an atom its own ancestor is
    // skipped, the same rule the tree uses.
    let mut anchor: BTreeMap<EventId, (Anchor, Side)> = BTreeMap::new();
    let mut line: BTreeMap<EventId, String> = BTreeMap::new();
    let mut dead: BTreeSet<EventId> = BTreeSet::new();
    for id in &present {
        if let Some(Op::Insert { parent, side, line: l }) = op_of(id) {
            anchor.insert(*id, (*parent, *side));
            line.insert(*id, l.clone());
        }
    }
    for id in &present {
        match op_of(id) {
            Some(Op::Delete { target }) => {
                dead.insert(*target);
            }
            Some(Op::MoveLine { target, parent, side }) => {
                if anchor.contains_key(target) && !anchors_under(&anchor, *parent, *target) {
                    anchor.insert(*target, (*parent, *side));
                }
            }
            _ => {}
        }
    }

    // --- pass 3: the permanent order ----------------------------------------
    // Fugue's tree walk. Children are grouped by (parent, side); reading order
    // is in-order — left children, the node itself, then right children — with
    // ties between same-parent-same-side siblings broken by EventId.
    let mut children: BTreeMap<(Anchor, Side), Vec<EventId>> = BTreeMap::new();
    for (atom, key) in &anchor {
        children.entry(*key).or_default().push(*atom);
    }
    for kids in children.values_mut() {
        kids.sort_unstable();
    }

    let mut files = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    for (node, n) in &tree.nodes {
        if n.kind != NodeKind::File {
            continue;
        }
        let path = if revive.contains(node) { tree.path_ignoring_removal(*node) } else { tree.path(*node) };
        let Some(path) = path else { continue };
        let mut atoms = Vec::new();
        walk_side(Anchor::DocStart(*node), Side::Right, &children, &line, &dead, &mut atoms);
        files.insert(path.clone(), atoms);
        nodes.insert(path, *node);
    }
    (tree, Materialised { nodes, files })
}

/// Pre-order walk: emit a living atom, then everything anchored to it.
///
/// A tombstoned atom still anchors its children -- deleting a line must not
/// orphan the lines someone else wrote after it.
fn walk_side(
    parent: Anchor,
    side: Side,
    children: &BTreeMap<(Anchor, Side), Vec<EventId>>,
    line: &BTreeMap<EventId, String>,
    dead: &BTreeSet<EventId>,
    out: &mut Vec<Atom>,
) {
    for atom in children.get(&(parent, side)).into_iter().flatten() {
        let me = Anchor::After(*atom);
        walk_side(me, Side::Left, children, line, dead, out);
        if !dead.contains(atom) {
            if let Some(l) = line.get(atom) {
                out.push(Atom { id: *atom, line: l.clone() });
            }
        }
        walk_side(me, Side::Right, children, line, dead, out);
    }
}

/// Would anchoring at `to` put us inside `target`'s own subtree?
fn anchors_under(
    anchor: &BTreeMap<EventId, (Anchor, Side)>,
    to: Anchor,
    target: EventId,
) -> bool {
    let mut cur = to;
    loop {
        match cur {
            Anchor::DocStart(_) => return false,
            Anchor::After(e) if e == target => return true,
            Anchor::After(e) => match anchor.get(&e) {
                Some((next, _)) => cur = *next,
                None => return false,
            },
        }
    }
}

/// Which document an event belongs to, found by walking its anchor chain up to
/// a document start. Tree ops name their node directly.
///
/// Needed wherever a line-level fact has to be lifted to a file-level one — most
/// importantly conflict detection, where an edit *inside* a file must be seen to
/// collide with a concurrent removal *of* that file.
pub fn document_of(e: EventId, log: &EventLog) -> Option<NodeId> {
    let mut cur = e;
    for _ in 0..=log.events.len() {
        match &log.events.get(&cur)?.op {
            Op::Insert { parent, .. } | Op::MoveLine { parent, .. } => match parent {
                Anchor::DocStart(n) => return Some(*n),
                Anchor::After(next) => cur = *next,
            },
            Op::Delete { target } => cur = *target,
            Op::Create { node, .. }
            | Op::MoveNode { node, .. }
            | Op::Remove { node }
            | Op::Restore { node }
            | Op::SetMode { node, .. } => return Some(*node),
        }
    }
    None
}

/// The tree a set of events produces — including removed nodes, which stay in
/// the map and are only marked. Conflict detection needs their ancestry.
pub fn tree_of(events: &[EventId], log: &EventLog) -> Tree {
    materialise_parts(events, log, &BTreeSet::new()).0
}
