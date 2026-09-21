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
///
/// The events are private so that [`EventLog::insert`] is the only way in:
/// the frontier and the clock are kept alongside them, per insert, instead of
/// being recomputed over every event on every append -- which cost ~270 µs a
/// keystroke at 7 000 events and grew with history (FINDINGS). Neither is
/// stored; loading rebuilds them through the same `insert`.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(from = "Vec<Event>", into = "Vec<Event>")]
pub struct EventLog {
    events: BTreeMap<EventId, Event>,
    /// Present events no present event names as a parent.
    heads: BTreeSet<EventId>,
    /// Named as a parent, not (yet) present. When one arrives it is already
    /// claimed, so it never becomes a head. Only sync's holes live here, so it
    /// stays small; everything else is O(parents) per insert.
    missing: BTreeSet<EventId>,
    /// One past the highest `seq` seen: the Lamport clock.
    next: u32,
}

impl From<Vec<Event>> for EventLog {
    fn from(events: Vec<Event>) -> Self {
        let mut log = Self::default();
        log.extend(events);
        log
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
        self.insert(Event { id, parents, op });
        id
    }

    /// Take in one event, keeping the frontier and the clock. A repeat is a
    /// no-op (sync re-delivers); returns whether it was new.
    pub fn insert(&mut self, e: Event) -> bool {
        if self.events.contains_key(&e.id) {
            return false;
        }
        for p in &e.parents {
            if !self.heads.remove(p) && !self.events.contains_key(p) {
                self.missing.insert(*p);
            }
        }
        if !self.missing.remove(&e.id) {
            self.heads.insert(e.id);
        }
        self.next = self.next.max(e.id.seq + 1);
        self.events.insert(e.id, e);
        true
    }

    /// [`EventLog::insert`] each, in order.
    pub fn extend(&mut self, events: impl IntoIterator<Item = Event>) {
        for e in events {
            self.insert(e);
        }
    }

    /// Every event, by id.
    pub fn events(&self) -> &BTreeMap<EventId, Event> {
        &self.events
    }

    /// Events named as a parent but not present: the holes a partial sync
    /// leaves. Kept, not counted, so it is O(1).
    pub fn holes(&self) -> usize {
        self.missing.len()
    }

    /// Append a batch in order, minting exactly the ids a capture adapter
    /// predicted. See [`crate::capture::from_save`] for the contract.
    pub fn append_batch(&mut self, replica: ReplicaId, next_seq: &mut u32, ops: Vec<Op>) -> Vec<EventId> {
        ops.into_iter().map(|op| self.append(replica, next_seq, op)).collect()
    }

    /// The next Lamport time: one past the highest `seq` seen from anyone.
    pub fn lamport_next(&self) -> u32 {
        self.next.max(1)
    }

    /// Events no other event claims as a parent — the current heads.
    pub fn frontier(&self) -> Vec<EventId> {
        self.heads.iter().copied().collect()
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
