//! The append-only event graph: layer 0.
//!
//! Every edit lands here the moment it happens, whether or not a human ever
//! names it. Live collaboration and version history are the same store read at
//! two granularities — that unification is the project's wager.

use crate::op::{EventId, Op, ReplicaId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One appended edit.
///
/// `parents` is the author's frontier at append time: everything they had seen.
/// It is free to record and **far too large to use as a dependency** — adopting
/// an event by its causal parents drags in the author's entire history. Layer 1
/// exists to replace it with something smaller.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub parents: Vec<EventId>,
    pub op: Op,
}

/// The whole graph. Append-only: nothing here is ever rewritten, which is what
/// makes `drop` safe on shared history.
///
/// Stored as a flat list rather than a map, because an `EventId` is a struct
/// and JSON keys must be strings. The list is the honest shape anyway: a log.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(from = "Vec<Event>", into = "Vec<Event>")]
pub struct EventLog {
    pub events: BTreeMap<EventId, Event>,
}

impl From<Vec<Event>> for EventLog {
    fn from(events: Vec<Event>) -> Self {
        Self { events: events.into_iter().map(|e| (e.id, e)).collect() }
    }
}

impl From<EventLog> for Vec<Event> {
    fn from(log: EventLog) -> Self {
        log.events.into_values().collect()
    }
}

impl Event {
    /// The first parent or referenced event this one is not younger than, if
    /// any. Lamport order says an event's `seq` is above everything it had
    /// seen, and it must have seen what it refers to. Checkable from the ids
    /// alone -- the referenced events need not be present -- so a receiver can
    /// judge each event on arrival.
    pub fn lamport_violation(&self) -> Option<EventId> {
        let refs = self.op.refs();
        self.parents.iter().chain(&refs).copied().find(|r| *r != Op::ROOT.0 && r.seq >= self.id.seq)
    }
}

impl EventLog {
    /// Append an op as this replica's next event, stamping the current frontier
    /// as its causal parents.
    pub fn append(&mut self, replica: ReplicaId, next_seq: &mut u32, op: Op) -> EventId {
        // A Lamport clock, not a counter: anything minted now must sort after
        // everything this replica has seen, whoever minted it. Without this, a
        // replica with a low counter that acts *after* reading someone else's
        // work still sorts before it, and every "last writer wins" rule — renames,
        // modes, restores — silently prefers the older write.
        let seq = (*next_seq).max(self.lamport_next());
        let id = EventId { seq, replica };
        *next_seq = seq + 1;
        let parents = self.frontier();
        self.events.insert(id, Event { id, parents, op });
        id
    }

    /// Append a batch in order, minting exactly the ids a capture adapter
    /// predicted. See [`crate::capture::from_save`] for the contract.
    pub fn append_batch(&mut self, replica: ReplicaId, next_seq: &mut u32, ops: Vec<Op>) -> Vec<EventId> {
        ops.into_iter().map(|op| self.append(replica, next_seq, op)).collect()
    }

    /// The next Lamport time: one past the highest `seq` seen from anyone.
    pub fn lamport_next(&self) -> u32 {
        self.events.keys().map(|id| id.seq + 1).max().unwrap_or(1)
    }

    /// Events no other event claims as a parent — the current heads.
    pub fn frontier(&self) -> Vec<EventId> {
        let claimed: BTreeSet<EventId> =
            self.events.values().flat_map(|e| e.parents.iter().copied()).collect();
        self.events.keys().copied().filter(|id| !claimed.contains(id)).collect()
    }

    /// `e` plus every event transitively reachable through `parents`.
    ///
    /// The baseline the spike measures against: this is what a naive
    /// event-sourced cherry-pick would have to take.
    pub fn causal_closure(&self, e: EventId) -> BTreeSet<EventId> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![e];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(ev) = self.events.get(&id) {
                stack.extend(ev.parents.iter().copied());
            }
        }
        seen
    }

    /// `e` plus everything reachable through [`crate::op::Op::refs`].
    ///
    /// The *event-granular* minimum: what this event needs if dependencies were
    /// tracked per event rather than per change. Comparing it with the
    /// change-granular closure measures what grouping costs — a change is taken
    /// whole, including events nothing referenced.
    pub fn semantic_closure(&self, e: EventId) -> BTreeSet<EventId> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![e];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(ev) = self.events.get(&id) {
                stack.extend(ev.op.refs());
            }
        }
        seen.retain(|id| self.events.contains_key(id));
        seen
    }
}
