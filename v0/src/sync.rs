//! Peer-to-peer sync, Yjs-shaped — and free, because of how ids are minted.
//!
//! `EventId = (replica, seq)` with monotone `seq` means `{replica -> max seq}`
//! describes a peer's holdings exactly. Two peers converge in one exchange each
//! way: send state vectors, send the difference. No server, no central order.
//!
//! Transport is out of scope (PRD). These functions are pure, so convergence is
//! provable between two in-memory replicas — invariants **I12** and **I13**.

use crate::event::{Event, EventLog};
use crate::op::ReplicaId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a peer holds: the highest sequence number seen per replica.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct StateVector(pub BTreeMap<ReplicaId, u32>);

/// Summarise a log. Cheap: one pass, one entry per replica.
pub fn state_vector(log: &EventLog) -> StateVector {
    todo!()
}

/// The events `theirs` is missing, oldest first so the receiver can integrate
/// them in one pass.
pub fn missing(log: &EventLog, theirs: &StateVector) -> Vec<Event> {
    todo!()
}

/// Absorb a peer's events.
///
/// Must be **idempotent and order-insensitive** (**I13**): receiving the same
/// events twice, or out of order, may not change the outcome. An event whose
/// parents are absent is still stored — the graph is allowed to have holes;
/// only [`crate::replay`] insists on dependency closure.
pub fn integrate(log: &mut EventLog, events: Vec<Event>) {
    todo!()
}
