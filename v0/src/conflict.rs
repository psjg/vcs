//! Conflicts: derived from the set, identified stably, resolved by patches.
//!
//! A CRDT merge never fails, so a conflict is not an error state the way it is
//! in git. It is a **fact about a set of changes**: two changes that did not see
//! each other both replaced or moved the same line. That makes it:
//!
//! - **derived** — computed from the set, never stored, so there is no
//!   `MERGE_HEAD` to lose and nothing to keep in sync;
//! - **identified** — its id is a hash of the contested line and the changes
//!   involved, so every replica names the same conflict the same way;
//! - **persistent** — it stays open, and keeps showing up, until a change in the
//!   set declares that it resolves it.
//!
//! A resolution is an ordinary change: whatever the resolver did to the text,
//! plus an explicit dependency on every side. Nothing is rewritten, so history
//! records what actually happened — including who resolved it, why, and (derived
//! rather than claimed) which side's lines survived.

use crate::change::{parse_hex32, ChangeId, ChangeSet, Changes};
use crate::event::EventLog;
use crate::op::{EventId, Op, Pos};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Stable identity of a conflict: `blake3(contested atom, sorted sides)`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ConflictId(pub [u8; 32]);

impl std::fmt::Display for ConflictId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl ConflictId {
    pub fn short(&self) -> String {
        self.to_string()[..8].to_owned()
    }
}

impl Serialize for ConflictId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ConflictId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let hex = String::deserialize(d)?;
        parse_hex32(&hex).map(ConflictId).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Status {
    /// Nobody has resolved it yet. It keeps showing up.
    Open,
    /// One resolution, or a chain where the latest supersedes the earlier ones.
    Resolved(ChangeId),
    /// Two people resolved it concurrently and differently: a conflict about
    /// the conflict. Left silent, both of their choices could vanish at once.
    Contested(Vec<ChangeId>),
    /// Every side put the same text in: they agree, nobody has to choose.
    /// Derived, not recorded -- a zero-op resolution by the system -- and the
    /// materialiser shows the text once (see [`agreed_hidden`]). Drop one side
    /// and the agreement, and with it the deduplication, simply goes away.
    Agreed,
    /// None of the contested text survives: a later change, made after seeing
    /// every side, removed all of it. There is nothing left to choose between.
    Moot,
}

impl Status {
    /// Whether this still needs a human. Only open and contested conflicts do.
    pub fn is_open(&self) -> bool {
        matches!(self, Status::Open | Status::Contested(_))
    }
}

/// What is contested.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Kind {
    /// Two changes replaced or moved the same characters.
    Text,
    /// One change removed a file (or a directory above it) while another,
    /// concurrently, edited inside it. Left undetected this loses the edit
    /// silently — the bug that motivated file-level conflicts.
    Removal,
    /// Two changes renamed or moved the same file to different places.
    Rename,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Conflict {
    pub id: ConflictId,
    pub kind: Kind,
    /// The run holding the contested characters — or, for file conflicts, the
    /// contested node (a node is named by the event that created it).
    pub atom: EventId,
    /// For text conflicts, the contested characters of that run, by offset.
    /// `(0, 0)` for file conflicts.
    pub range: (u32, u32),
    pub sides: BTreeSet<ChangeId>,
    pub status: Status,
}

/// Every conflict in a change set, open or not.
pub fn conflicts(set: &ChangeSet, changes: &Changes, log: &EventLog) -> Vec<Conflict> {
    let ids: Vec<ChangeId> = set.ids().iter().copied().collect();
    let order = Order::new(&ids, changes, log);

    let mut out = Vec::new();
    for s in text_stretches(&ids, changes, log, &order) {
        let id = conflict_id(Kind::Text, s.run, s.range.0, &s.sides);
        let mut status = status_of(id, &ids, changes, &|a, b| order.before(a, b));
        if status == Status::Open && agreed(&s, changes, log).is_some() {
            status = Status::Agreed;
        }
        out.push(Conflict { id, kind: Kind::Text, atom: s.run, range: s.range, sides: s.sides, status });
    }

    // Moot needs the materialised head, which already folds in agreement, so
    // it is decided last -- and without re-entering `conflicts`.
    let events: Vec<EventId> = changes.events_of(set.ids()).into_iter().collect();
    let hidden = agreed_hidden(set, changes, log);
    let alive: BTreeSet<Pos> = crate::replay::materialise_hidden(&events, log, &BTreeSet::new(), &hidden)
        .files
        .values()
        .flatten()
        .map(|a| a.id)
        .collect();
    for c in out.iter_mut().filter(|c| c.status == Status::Open) {
        let stretch = Stretch { run: c.atom, range: c.range, sides: c.sides.clone() };
        let any_alive = c.sides.iter().any(|side| {
            replacement(*side, &stretch, changes, log)
                .iter()
                .flat_map(|e| (0..run_len(*e, log)).map(move |offset| Pos { event: *e, offset }))
                .any(|p| alive.contains(&p))
        });
        if !any_alive {
            c.status = Status::Moot;
        }
    }

    out.extend(file_conflicts(&ids, changes, log, &|a, b| order.before(a, b), &|a, b| order.concurrent(a, b)));
    for c in out.iter_mut().filter(|c| c.kind != Kind::Text) {
        c.status = status_of(c.id, &ids, changes, &|a, b| order.before(a, b));
    }
    out
}

/// Runs to hide so that agreed text appears once.
///
/// For every agreed stretch, keep the replacement of the side whose first run
/// has the lowest EventId and hide the others. A pure function of the set: it
/// is how `M(S)` shows an agreement, never something written down, so dropping
/// a side cannot leave a hole where the kept copy used to be.
pub fn agreed_hidden(set: &ChangeSet, changes: &Changes, log: &EventLog) -> BTreeSet<EventId> {
    let ids: Vec<ChangeId> = set.ids().iter().copied().collect();
    let order = Order::new(&ids, changes, log);
    let mut hidden = BTreeSet::new();
    for s in text_stretches(&ids, changes, log, &order) {
        let id = conflict_id(Kind::Text, s.run, s.range.0, &s.sides);
        // An explicit resolution outranks the system's agreement.
        if status_of(id, &ids, changes, &|a, b| order.before(a, b)) != Status::Open {
            continue;
        }
        if let Some(reps) = agreed(&s, changes, log) {
            let keep = reps.iter().map(|r| r[0]).min();
            for r in reps.iter().filter(|r| Some(r[0]) != keep) {
                hidden.extend(r.iter().copied());
            }
        }
    }
    hidden
}

/// A contested stretch of one run and the changes that fought over it.
struct Stretch {
    run: EventId,
    range: (u32, u32),
    sides: BTreeSet<ChangeId>,
}

/// Causal order between changes: did one author see the other's work?
struct Order<'a> {
    seen: BTreeMap<ChangeId, BTreeSet<EventId>>,
    changes: &'a Changes,
}

impl<'a> Order<'a> {
    fn new(ids: &[ChangeId], changes: &'a Changes, log: &EventLog) -> Self {
        Self { seen: ancestry(ids, changes, log), changes }
    }
    fn before(&self, a: &ChangeId, b: &ChangeId) -> bool {
        a != b && self.changes.by_id[a].events.iter().any(|e| self.seen[b].contains(e))
    }
    fn concurrent(&self, a: &ChangeId, b: &ChangeId) -> bool {
        !self.before(a, b) && !self.before(b, a)
    }
}

/// Every contested stretch: characters removed or moved by two changes that did
/// not see each other, where at least one of them put something in their place.
fn text_stretches(ids: &[ChangeId], changes: &Changes, log: &EventLog, order: &Order) -> Vec<Stretch> {
    let mut touched: BTreeMap<Pos, BTreeSet<ChangeId>> = BTreeMap::new();
    for id in ids {
        for e in &changes.by_id[id].events {
            let hit: Vec<Pos> = match log.events.get(e).map(|ev| &ev.op) {
                Some(Op::Delete { target, range }) => crate::op::clamp(*range, run_len(*target, log))
                    .map(|offset| Pos { event: *target, offset })
                    .collect(),
                Some(Op::MoveRun { target, .. }) => {
                    let n = run_len(*target, log);
                    (0..n).map(|offset| Pos { event: *target, offset }).collect()
                }
                _ => Vec::new(),
            };
            for p in hit {
                touched.entry(p).or_default().insert(*id);
            }
        }
    }

    let delete_only = |id: &ChangeId| {
        changes.by_id[id]
            .events
            .iter()
            .all(|e| matches!(log.events.get(e).map(|ev| &ev.op), Some(Op::Delete { .. })))
    };

    let mut contested: Vec<(Pos, BTreeSet<ChangeId>)> = Vec::new();
    for (pos, by) in touched {
        let sides: BTreeSet<ChangeId> = by
            .iter()
            .filter(|a| by.iter().any(|b| *a != b && order.concurrent(a, b)))
            .copied()
            .collect();
        // Two people deleting the same characters agree; nothing to choose.
        if sides.len() < 2 || sides.iter().all(delete_only) {
            continue;
        }
        contested.push((pos, sides));
    }

    // One stretch per contiguous run of positions fought over by the same
    // changes -- "both rewrote this word", not one conflict per letter.
    let mut out = Vec::new();
    let mut i = 0;
    while i < contested.len() {
        let (start, sides) = contested[i].clone();
        let mut end = start.offset + 1;
        let mut j = i + 1;
        while j < contested.len()
            && contested[j].0.event == start.event
            && contested[j].0.offset == end
            && contested[j].1 == sides
        {
            end += 1;
            j += 1;
        }
        out.push(Stretch { run: start.event, range: (start.offset, end), sides });
        i = j;
    }
    out
}

/// The runs one side put in place of a contested stretch: its inserts anchored
/// on or right next to the stretch, plus any of its runs chained onto those.
fn replacement(side: ChangeId, s: &Stretch, changes: &Changes, log: &EventLog) -> Vec<EventId> {
    let inserts: Vec<(EventId, Pos)> = changes.by_id[&side]
        .events
        .iter()
        .filter_map(|e| match log.events.get(e).map(|ev| &ev.op) {
            Some(Op::Insert { parent: crate::op::Anchor::At(p), .. }) => Some((*e, *p)),
            _ => None,
        })
        .collect();
    let mut picked: BTreeSet<EventId> = inserts
        .iter()
        .filter(|(_, p)| p.event == s.run && p.offset + 1 >= s.range.0 && p.offset <= s.range.1)
        .map(|(e, _)| *e)
        .collect();
    loop {
        let more: Vec<EventId> = inserts
            .iter()
            .filter(|(e, p)| !picked.contains(e) && picked.contains(&p.event))
            .map(|(e, _)| *e)
            .collect();
        if more.is_empty() {
            break;
        }
        picked.extend(more);
    }
    picked.into_iter().collect()
}

/// `Some(each side's replacement runs)` when every side put in the same,
/// non-empty text; `None` otherwise.
fn agreed(s: &Stretch, changes: &Changes, log: &EventLog) -> Option<Vec<Vec<EventId>>> {
    let reps: Vec<Vec<EventId>> = s.sides.iter().map(|side| replacement(*side, s, changes, log)).collect();
    let text = |runs: &Vec<EventId>| -> String {
        runs.iter()
            .filter_map(|e| match log.events.get(e).map(|ev| &ev.op) {
                Some(Op::Insert { text, .. }) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    };
    let first = text(reps.first()?);
    (!first.is_empty() && reps.iter().all(|r| !r.is_empty() && text(r) == first)).then_some(reps)
}

/// How many characters an insert's run holds.
fn run_len(e: EventId, log: &EventLog) -> u32 {
    match log.events.get(&e).map(|ev| &ev.op) {
        Some(Op::Insert { text, .. }) => text.chars().count() as u32,
        _ => 0,
    }
}

/// Removal-versus-edit and rename-versus-rename, lifted from lines to files.
fn file_conflicts(
    ids: &[ChangeId],
    changes: &Changes,
    log: &EventLog,
    before: &dyn Fn(&ChangeId, &ChangeId) -> bool,
    concurrent: &dyn Fn(&ChangeId, &ChangeId) -> bool,
) -> Vec<Conflict> {
    use crate::op::NodeId;
    let events: Vec<EventId> = changes.events_of(&ids.iter().copied().collect()).into_iter().collect();
    let tree = crate::replay::tree_of(&events, log);

    let mut removers: BTreeMap<NodeId, BTreeSet<ChangeId>> = BTreeMap::new();
    let mut movers: BTreeMap<NodeId, BTreeMap<ChangeId, (NodeId, String)>> = BTreeMap::new();
    let mut touched: BTreeMap<ChangeId, BTreeSet<NodeId>> = BTreeMap::new();
    for id in ids {
        for e in &changes.by_id[id].events {
            let Some(op) = log.events.get(e).map(|ev| &ev.op) else { continue };
            match op {
                Op::Remove { node } => {
                    removers.entry(*node).or_default().insert(*id);
                }
                Op::MoveNode { node, parent, name } => {
                    movers.entry(*node).or_default().insert(*id, (*parent, name.clone()));
                    touched.entry(*id).or_default().insert(*node);
                }
                // Creating something inside a directory is editing that directory.
                Op::Create { parent, .. } => {
                    touched.entry(*id).or_default().insert(*parent);
                }
                _ => {
                    if let Some(doc) = crate::replay::document_of(*e, log) {
                        touched.entry(*id).or_default().insert(doc);
                    }
                }
            }
        }
    }

    let mut out = Vec::new();
    for (node, by) in &removers {
        // Anyone who, without having seen a removal, worked on this node or on
        // anything below it.
        let editors: BTreeSet<ChangeId> = touched
            .iter()
            .filter(|(c, _)| !by.contains(c))
            .filter(|(c, nodes)| {
                by.iter().any(|r| concurrent(r, c))
                    && nodes.iter().any(|n| tree.is_ancestor(*node, *n))
            })
            .map(|(c, _)| *c)
            .collect();
        if editors.is_empty() {
            continue; // removed by one or more people who all agree
        }
        let mut sides: BTreeSet<ChangeId> =
            by.iter().filter(|r| editors.iter().any(|e| concurrent(r, e))).copied().collect();
        sides.extend(editors);
        let id = conflict_id(Kind::Removal, node.0, 0, &sides);
        out.push(Conflict { id, kind: Kind::Removal, atom: node.0, range: (0, 0), sides, status: Status::Open });
    }
    for (node, by) in &movers {
        let sides: BTreeSet<ChangeId> = by
            .iter()
            .filter(|&(a, ta)| by.iter().any(|(b, tb)| a != b && ta != tb && concurrent(a, b)))
            .map(|(c, _)| *c)
            .collect();
        if sides.len() < 2 {
            continue;
        }
        let id = conflict_id(Kind::Rename, node.0, 0, &sides);
        out.push(Conflict { id, kind: Kind::Rename, atom: node.0, range: (0, 0), sides, status: Status::Open });
    }
    let _ = before;
    out
}

/// Open, resolved, or contested — the same rule for every kind of conflict.
fn status_of(
    id: ConflictId,
    ids: &[ChangeId],
    changes: &Changes,
    before: &dyn Fn(&ChangeId, &ChangeId) -> bool,
) -> Status {
    let resolutions: Vec<ChangeId> =
        ids.iter().filter(|c| changes.by_id[c].meta.resolves.contains(&id)).copied().collect();
    let latest: Vec<ChangeId> = resolutions
        .iter()
        .filter(|r| !resolutions.iter().any(|s| before(r, s)))
        .copied()
        .collect();
    match latest.as_slice() {
        [] => Status::Open,
        [one] => Status::Resolved(*one),
        many => Status::Contested(many.to_vec()),
    }
}

/// Lines each side inserted — what a checkout shows between markers.
/// Characters each side inserted — what a checkout shows, and what the audit
/// counts as surviving or not. Per character, so extending someone's line reads
/// as keeping most of it rather than as "their line lost".
pub fn side_chars(c: &Conflict, changes: &Changes, log: &EventLog) -> BTreeMap<ChangeId, Vec<Pos>> {
    c.sides
        .iter()
        .map(|s| {
            let chars = changes.by_id[s]
                .events
                .iter()
                .filter(|e| matches!(log.events.get(e).map(|ev| &ev.op), Some(Op::Insert { .. })))
                .flat_map(|e| (0..run_len(*e, log)).map(move |offset| Pos { event: *e, offset }))
                .collect();
            (*s, chars)
        })
        .collect()
}

fn conflict_id(kind: Kind, atom: EventId, offset: u32, sides: &BTreeSet<ChangeId>) -> ConflictId {
    let mut h = blake3::Hasher::new();
    h.update(&[kind as u8]);
    h.update(&atom.seq.to_le_bytes());
    h.update(&atom.replica.0.to_le_bytes());
    h.update(&offset.to_le_bytes());
    for s in sides {
        h.update(&s.0);
    }
    ConflictId(*h.finalize().as_bytes())
}

/// Every event each change's author had seen.
fn ancestry(
    ids: &[ChangeId],
    changes: &Changes,
    log: &EventLog,
) -> BTreeMap<ChangeId, BTreeSet<EventId>> {
    ids.iter()
        .map(|id| {
            let seen = changes.by_id[id].events.iter().flat_map(|e| log.causal_closure(*e)).collect();
            (*id, seen)
        })
        .collect()
}
