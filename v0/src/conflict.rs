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
use crate::op::{EventId, Op};
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
}

/// What is contested.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Kind {
    /// Two changes replaced or moved the same line.
    Line,
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
    /// The contested line — or, for file conflicts, the contested node (a node
    /// is named by the event that created it).
    pub atom: EventId,
    pub sides: BTreeSet<ChangeId>,
    pub status: Status,
}

/// Every conflict in a change set, open or not.
pub fn conflicts(set: &ChangeSet, changes: &Changes, log: &EventLog) -> Vec<Conflict> {
    let ids: Vec<ChangeId> = set.ids().iter().copied().collect();
    let seen = ancestry(&ids, changes, log);
    let before = |a: &ChangeId, b: &ChangeId| {
        a != b && changes.by_id[a].events.iter().any(|e| seen[b].contains(e))
    };
    let concurrent = |a: &ChangeId, b: &ChangeId| !before(a, b) && !before(b, a);

    // Which changes removed or moved each atom.
    let mut touched: BTreeMap<EventId, BTreeSet<ChangeId>> = BTreeMap::new();
    for id in &ids {
        for e in &changes.by_id[id].events {
            match log.events.get(e).map(|ev| &ev.op) {
                Some(Op::Delete { target } | Op::MoveLine { target, .. }) => {
                    touched.entry(*target).or_default().insert(*id);
                }
                _ => {}
            }
        }
    }

    let delete_only = |id: &ChangeId| {
        changes.by_id[id]
            .events
            .iter()
            .all(|e| matches!(log.events.get(e).map(|ev| &ev.op), Some(Op::Delete { .. })))
    };

    let mut out = Vec::new();
    for (atom, by) in touched {
        let sides: BTreeSet<ChangeId> = by
            .iter()
            .filter(|a| by.iter().any(|b| concurrent(a, b) && *a != b))
            .copied()
            .collect();
        // Two people deleting the same line agree; there is nothing to choose.
        if sides.len() < 2 || sides.iter().all(delete_only) {
            continue;
        }
        let id = conflict_id(Kind::Line, atom, &sides);

        let resolutions: Vec<ChangeId> = ids
            .iter()
            .filter(|c| changes.by_id[c].meta.resolves.contains(&id))
            .copied()
            .collect();
        // A later resolution supersedes an earlier one it has seen; two that
        // have not seen each other are contested.
        let latest: Vec<ChangeId> = resolutions
            .iter()
            .filter(|r| !resolutions.iter().any(|s| before(r, s)))
            .copied()
            .collect();
        let status = match latest.as_slice() {
            [] => Status::Open,
            [one] => Status::Resolved(*one),
            many => Status::Contested(many.to_vec()),
        };
        out.push(Conflict { id, kind: Kind::Line, atom, sides, status });
    }

    out.extend(file_conflicts(&ids, changes, log, &before, &concurrent));
    for c in out.iter_mut().filter(|c| c.kind != Kind::Line) {
        c.status = status_of(c.id, &ids, changes, &before);
    }
    out
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
        let id = conflict_id(Kind::Removal, node.0, &sides);
        out.push(Conflict { id, kind: Kind::Removal, atom: node.0, sides, status: Status::Open });
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
        let id = conflict_id(Kind::Rename, node.0, &sides);
        out.push(Conflict { id, kind: Kind::Rename, atom: node.0, sides, status: Status::Open });
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
pub fn side_lines(c: &Conflict, changes: &Changes, log: &EventLog) -> BTreeMap<ChangeId, Vec<EventId>> {
    c.sides
        .iter()
        .map(|s| {
            let ins = changes.by_id[s]
                .events
                .iter()
                .filter(|e| matches!(log.events.get(e).map(|ev| &ev.op), Some(Op::Insert { .. })))
                .copied()
                .collect();
            (*s, ins)
        })
        .collect()
}

fn conflict_id(kind: Kind, atom: EventId, sides: &BTreeSet<ChangeId>) -> ConflictId {
    let mut h = blake3::Hasher::new();
    h.update(&[kind as u8]);
    h.update(&atom.seq.to_le_bytes());
    h.update(&atom.replica.0.to_le_bytes());
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
