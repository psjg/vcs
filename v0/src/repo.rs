//! The four operations — all of them set algebra over [`ChangeSet`].
//!
//! ```text
//! merge(a, b)   = a ∪ b
//! adopt(set, c) = set ∪ closure(c)
//! drop(set, c)  = set \ upward_closure(c)
//! record(...)   = diff against M(set), mint ops, derive deps
//! ```
//!
//! There is no rebase, no cherry-pick and no revert here, because `adopt` *is*
//! cherry-pick, `drop` *is* revert, and reordering is meaningless: `M` never
//! reads order.

use crate::change::{Change, ChangeId, ChangeSet, Log, Meta};
use crate::op::ReplicaId;
use crate::weave::Document;

/// A working copy: the log, which changes are currently in effect, and who we
/// are when minting ops.
#[derive(Clone, Debug)]
pub struct Repo {
    pub log: Log,
    pub head: ChangeSet,
    pub replica: ReplicaId,
    /// Next sequence number for this replica. Monotone, never reused.
    pub next_seq: u32,
}

impl Repo {
    /// Union of two change sets. Closed, because the union of two closed sets is.
    pub fn merge(&self, other: &ChangeSet) -> ChangeSet {
        todo!()
    }

    /// Cherry-pick: `c` and everything it needs, nothing else.
    pub fn adopt(&self, c: ChangeId) -> ChangeSet {
        todo!()
    }

    /// Remove `c` and everything that depends on it. Every surviving change
    /// keeps its id (**I3**) because ids are content addresses.
    pub fn drop_change(&self, c: ChangeId) -> ChangeSet {
        todo!()
    }

    /// Materialise the current head.
    pub fn document(&self) -> Document {
        todo!()
    }

    /// Record `text` as a new change against the current head.
    ///
    /// This is the honest weak point of v0 (ADR-0003): it *reconstructs* ops by
    /// diffing the working copy against `M(head)`, so the interpretation of the
    /// edit is baked in here. Everything downstream is indifferent to where the
    /// ops came from — a live capture front-end replaces this function alone.
    pub fn record(&mut self, text: &str, meta: Meta) -> Change {
        todo!()
    }
}

/// Line diff between the materialised document and new text, expressed as the
/// ops needed to get from one to the other.
///
/// Kept separate from [`Repo::record`] so the diff heuristic can be swapped or
/// benchmarked without touching the model.
pub fn diff_to_ops(
    before: &Document,
    after: &str,
    replica: ReplicaId,
    next_seq: &mut u32,
) -> Vec<crate::op::Op> {
    todo!()
}
