//! Capture adapters: how observed editing becomes events.
//!
//! Everything above layer 0 is indifferent to which adapter ran — that is the
//! point of putting capture behind an interface. What differs is **fidelity**,
//! and the difference is measurable rather than theoretical:
//!
//! | adapter | sees | cannot see |
//! |---|---|---|
//! | [`from_save`] (v0) | before/after text | which edit happened, and every move |
//! | live editor (v1) | each keystroke or edit command | — |
//!
//! A diff-based adapter reconstructs a plausible edit and bakes that
//! interpretation into the log forever (ADR-0003). It can never emit
//! [`crate::op::Op::MoveLine`]: a move looks exactly like a delete plus an
//! insert. Measuring how many moves v0 loses is the cleanest statement of what
//! coarse capture costs.

use crate::op::{NodeId, Op, ReplicaId};
use crate::replay::Worktree;

/// Degraded adapter: diff the saved text against the materialised state and
/// mint the ops that explain the difference.
pub fn from_save(
    before: &Worktree,
    node: NodeId,
    after: &str,
    replica: ReplicaId,
    next_seq: &mut u32,
) -> Vec<Op> {
    todo!()
}

/// The shape a live front-end calls: an already-known edit, no guessing. Present
/// in v0 so the interface is honest about what it is waiting for.
pub fn from_edit(edit: Edit, replica: ReplicaId, next_seq: &mut u32) -> Vec<Op> {
    todo!()
}

/// What an editor actually knows it did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    TypedLine { after: Option<crate::op::EventId>, node: NodeId, line: String },
    DeletedLine { target: crate::op::EventId },
    MovedLine { target: crate::op::EventId, to: crate::op::Anchor },
    RenamedNode { node: NodeId, parent: NodeId, name: String },
}
