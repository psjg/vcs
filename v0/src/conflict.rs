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

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Conflict {
    pub id: ConflictId,
    /// The line both sides replaced or moved.
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
        let id = conflict_id(atom, &sides);

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
        out.push(Conflict { id, atom, sides, status });
    }
    out
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

fn conflict_id(atom: EventId, sides: &BTreeSet<ChangeId>) -> ConflictId {
    let mut h = blake3::Hasher::new();
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
