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
//! [`Op::MoveLine`]: a move looks exactly like a delete plus an insert.

use crate::op::{Anchor, EventId, NodeId, Op, ReplicaId};
use crate::replay::Atom;
use similar::{ChangeTag, TextDiff};

/// Degraded adapter: diff saved text against the materialised lines and mint
/// the ops that explain the difference.
///
/// **Contract.** The returned ops must be appended in order starting at
/// `base_seq`, because an insert that follows another insert in the same batch
/// anchors to an id this function *predicts*: the i-th op will be
/// `EventId { seq: base_seq + i, replica }`. [`crate::event::EventLog::append_batch`]
/// is the matching appender and the only supported way to land them.
pub fn from_save(before: &[Atom], node: NodeId, after: &str, replica: ReplicaId, base_seq: u32) -> Vec<Op> {
    let old: Vec<&str> = before.iter().map(|a| a.line.as_str()).collect();
    let new: Vec<&str> = after.lines().collect();

    let mut ops: Vec<Op> = Vec::new();
    // Where an insert attaches: the last atom we walked past, so new lines land
    // after the line they follow. A tombstone still anchors, so deleting and
    // inserting at the same spot stays well defined.
    let mut at = Anchor::DocStart(node);
    let mut old_idx = 0usize;

    for change in TextDiff::from_slices(&old, &new).iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                at = Anchor::After(before[old_idx].id);
                old_idx += 1;
            }
            ChangeTag::Delete => {
                ops.push(Op::Delete { target: before[old_idx].id });
                at = Anchor::After(before[old_idx].id);
                old_idx += 1;
            }
            ChangeTag::Insert => {
                let mine = EventId { seq: base_seq + ops.len() as u32, replica };
                ops.push(Op::Insert { anchor: at, line: change.value().trim_end_matches('\n').to_string() });
                // The next line of this run chains to me, keeping a pasted
                // block contiguous instead of anchoring it all at one point.
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
    MovedLine { target: EventId, to: Anchor },
    RenamedNode { node: NodeId, parent: NodeId, name: String },
}
