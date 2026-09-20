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
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct EventLog {
    pub events: BTreeMap<EventId, Event>,
}

impl EventLog {
    /// Append an op as this replica's next event, stamping the current frontier
    /// as its causal parents.
    pub fn append(&mut self, replica: ReplicaId, next_seq: &mut u32, op: Op) -> EventId {
        todo!()
    }

    /// Events no other event claims as a parent — the current heads.
    pub fn frontier(&self) -> Vec<EventId> {
        todo!()
    }

    /// `e` plus every event transitively reachable through `parents`.
    ///
    /// The baseline the spike measures against: this is what a naive
    /// event-sourced cherry-pick would have to take.
    pub fn causal_closure(&self, e: EventId) -> BTreeSet<EventId> {
        todo!()
    }
}
