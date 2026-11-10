//! Peer-to-peer sync, Yjs-shaped — and free, because of how ids are minted.
//!
//! `EventId = (replica, seq)` with monotone `seq` means `{replica -> max seq}`
//! describes a peer's holdings exactly. Two peers converge in one exchange each
//! way: send state vectors, send the difference. No server, no central order.
//!
//! Transport is out of scope (PRD). These functions are pure, so convergence is
//! provable between two in-memory replicas — invariants **I12** and **I13**.

use crate::event::{Event, EventLog};
use crate::limits::{Over, SYNC_EVENTS_MAX};
use crate::op::{EventId, ReplicaId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a peer holds: the highest sequence number seen per replica.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct StateVector(pub BTreeMap<ReplicaId, u32>);

/// Summarise a log. Cheap: one pass, one entry per replica.
pub fn state_vector(log: &EventLog) -> StateVector {
    let mut sv = BTreeMap::new();
    for id in log.events().keys() {
        let hi = sv.entry(id.replica).or_insert(id.seq);
        *hi = (*hi).max(id.seq);
    }
    StateVector(sv)
}

/// The events `theirs` is missing, oldest first so the receiver can integrate
/// them in one pass -- at most [`SYNC_EVENTS_MAX`] of them. Oldest first in
/// id order is a prefix of every replica's events, so what a capped exchange
/// leaves behind is exactly what the next state vector asks for.
pub fn missing(log: &EventLog, theirs: &StateVector) -> Vec<Event> {
    log.events()
        .values()
        .filter(|e| match theirs.0.get(&e.id.replica) {
            // They hold everything up to `hi` from this replica. A replica's
            // seqs are monotone (Lamport) but not dense -- they jump when it
            // catches up -- and that is fine: "every event of r above hi" is
            // still exactly what they lack, because sync always sends a
            // replica's events as a contiguous suffix.
            Some(hi) => e.id.seq > *hi,
            None => true,
        })
        .take(SYNC_EVENTS_MAX)
        .cloned()
        .collect()
}

/// An event turned away at the door, and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Refused {
    pub event: EventId,
    pub because: Why,
}

/// Why an event was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Why {
    /// It claims to have seen, or refers to, this event without being younger
    /// than it: a broken Lamport order (**I14**).
    NotYounger(EventId),
    /// It exceeds a fixed limit ([`crate::limits`]).
    Over(Over),
    /// It came after the first [`SYNC_EVENTS_MAX`] of one exchange. Nothing
    /// is wrong with it; a peer that pages never sends it.
    Surplus,
}

impl std::fmt::Display for Why {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Why::NotYounger(e) => {
                write!(f, "it is not younger than {e:?}, which it claims to have seen (broken Lamport order)")
            }
            Why::Over(o) => write!(f, "{o}"),
            Why::Surplus => write!(f, "more than {SYNC_EVENTS_MAX} events in one exchange"),
        }
    }
}

/// Absorb a peer's events; return the ones refused.
///
/// Must be **idempotent and order-insensitive** (**I13**): receiving the same
/// events twice, or out of order, may not change the outcome. An event whose
/// parents are absent is still stored — the graph is allowed to have holes;
/// only [`crate::replay`] insists on dependency closure.
///
/// An event that breaks the Lamport order is refused (**I14**): everything
/// downstream relies on "an event is younger than what it refers to" -- last
/// writer wins, and the live weave's placement rule. Judged per event from ids
/// alone, so refusing keeps I13 intact. So is an event over a limit
/// ([`Event::over_limit`]), by the same argument.
pub fn integrate(log: &mut EventLog, events: Vec<Event>) -> Vec<Refused> {
    let mut refused = Vec::new();
    for (i, e) in events.into_iter().enumerate() {
        let why = if i >= SYNC_EVENTS_MAX {
            Some(Why::Surplus)
        } else {
            e.lamport_violation().map(Why::NotYounger).or_else(|| e.over_limit().map(Why::Over))
        };
        if let Some(because) = why {
            refused.push(Refused { event: e.id, because });
            continue;
        }
        // Idempotent and order-insensitive by construction: an event is keyed
        // by an id that is unique and immutable, so re-receiving it is a no-op
        // and arrival order cannot matter.
        log.insert(e);
    }
    refused
}
