//! Capture adapters: how observed editing becomes events.
//!
//! Everything above layer 0 is indifferent to which adapter ran — that is the
//! point of putting capture behind an interface. What differs is **fidelity**:
//!
//! | adapter | sees | cannot see |
//! |---|---|---|
//! | [`from_save`] (v0) | before/after text | which edit happened, and every move |
//! | live editor (v1) | each keystroke or edit command | — |
//!
//! A diff-based adapter reconstructs a plausible edit and bakes that
//! interpretation into the log forever (ADR-0003). It can never emit
//! [`Op::MoveLine`]: a move looks exactly like a delete plus an insert.

use crate::event::EventLog;
use crate::op::{Anchor, EventId, NodeId, Op, ReplicaId, Side};
use crate::replay::Atom;
use similar::{ChangeTag, TextDiff};
use std::collections::BTreeMap;

/// Fugue's placement rule, from Weidner & Kleppmann, *The Art of the Fugue*
/// (arXiv:2305.00583). To place a line between left neighbour `a` and right
/// neighbour `b`:
///
/// - if `a` is **not** an ancestor of `b`, become a **right child of `a`**;
/// - if `a` **is** an ancestor of `b`, become a **left child of `b`**.
///
/// Those two lines are the whole difference between two people's blocks staying
/// whole and being shuffled together. Reading order is the in-order traversal of
/// the resulting tree, so a subtree — one person's block — is always contiguous.
pub fn between(
    a: Anchor,
    b: Option<EventId>,
    parent_of: &dyn Fn(EventId) -> Option<Anchor>,
) -> (Anchor, Side) {
    match b {
        Some(b) if is_ancestor(a, b, parent_of) => (Anchor::After(b), Side::Left),
        _ => (a, Side::Right),
    }
}

/// Is `a` on the path from `b` up to the root of its document?
fn is_ancestor(a: Anchor, b: EventId, parent_of: &dyn Fn(EventId) -> Option<Anchor>) -> bool {
    // The document root is an ancestor of everything in it. That is what makes
    // an insert at position 0 a *left* child of the first line rather than a
    // right child of the root — and it is why typing upwards stays contiguous.
    if matches!(a, Anchor::DocStart(_)) {
        return true;
    }
    let mut cur = b;
    loop {
        match parent_of(cur) {
            Some(p) if p == a => return true,
            Some(Anchor::After(p)) => cur = p,
            _ => return false,
        }
    }
}

/// Degraded adapter: diff saved text against the materialised lines and mint
/// the ops that explain the difference.
///
/// **Contract.** The returned ops must be appended in order starting at
/// `base_seq`, because a line that follows another line of the same batch
/// anchors to an id this function *predicts*: the i-th op will be
/// `EventId { seq: base_seq + i, replica }`.
/// [`crate::event::EventLog::append_batch`] is the matching appender.
pub fn from_save(
    before: &[Atom],
    node: NodeId,
    after: &str,
    replica: ReplicaId,
    base_seq: u32,
    log: &EventLog,
) -> Vec<Op> {
    let old: Vec<&str> = before.iter().map(|a| a.line.as_str()).collect();
    let new: Vec<&str> = after.lines().collect();

    let mut ops: Vec<Op> = Vec::new();
    // Parents of atoms minted in *this* batch: not in the log yet, but already
    // legal ancestors for the lines that follow them.
    let mut local: BTreeMap<EventId, Anchor> = BTreeMap::new();
    let mut at = Anchor::DocStart(node);
    let mut old_idx = 0usize;

    for change in TextDiff::from_slices(&old, &new).iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal | ChangeTag::Delete => {
                if change.tag() == ChangeTag::Delete {
                    ops.push(Op::Delete { target: before[old_idx].id });
                }
                // A tombstone still anchors, so deleting and inserting at the
                // same spot stays well defined.
                at = Anchor::After(before[old_idx].id);
                old_idx += 1;
            }
            ChangeTag::Insert => {
                let right = before.get(old_idx).map(|a| a.id);
                let parent_of = |e: EventId| -> Option<Anchor> {
                    local.get(&e).copied().or_else(|| match log.events.get(&e).map(|ev| &ev.op) {
                        Some(Op::Insert { parent, .. } | Op::MoveLine { parent, .. }) => {
                            Some(*parent)
                        }
                        _ => None,
                    })
                };
                let (parent, side) = between(at, right, &parent_of);
                let mine = EventId { seq: base_seq + ops.len() as u32, replica };
                local.insert(mine, parent);
                ops.push(Op::Insert {
                    parent,
                    side,
                    line: change.value().trim_end_matches('\n').to_string(),
                });
                // The next line of this run sits to my right, so a pasted block
                // becomes one subtree and stays contiguous under any merge.
                at = Anchor::After(mine);
            }
        }
    }
    ops
}

/// The shape a live front-end calls: an already-known edit, no guessing.
/// Present so the interface is honest about what it is waiting for.
pub fn from_edit(_edit: Edit) -> Vec<Op> {
    todo!("v1: a front-end that reports edits instead of states")
}

/// What an editor actually knows it did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    TypedLine { after: Option<EventId>, node: NodeId, line: String },
    DeletedLine { target: EventId },
    MovedLine { target: EventId, parent: Anchor, side: Side },
    RenamedNode { node: NodeId, parent: NodeId, name: String },
}
