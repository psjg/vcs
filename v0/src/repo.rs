//! The user-facing operations. All of them are set algebra.
//!
//! ```text
//! merge(a, b)   = a ∪ b
//! adopt(set, c) = set ∪ closure(c)
//! drop(set, c)  = set \ upward_closure(c)
//! ```
//!
//! There is no `rebase`, no `cherry-pick` and no `revert` in this API, and that
//! is not minimalism for its own sake: `adopt` *is* cherry-pick, `drop` *is*
//! revert, and reordering is meaningless because `M` never reads order. A whole
//! class of git hazards — rewritten hashes, divergent duplicates, "never rebase
//! a shared branch" — does not exist here to be guarded against.

use crate::change::{Change, ChangeId, ChangeSet, Changes, Meta};
use crate::event::EventLog;
use crate::op::{EventId, ReplicaId};
use crate::replay::Worktree;
use std::collections::BTreeSet;

/// One working copy.
#[derive(Clone, Debug)]
pub struct Repo {
    pub log: EventLog,
    pub changes: Changes,
    /// Changes currently in effect.
    pub head: ChangeSet,
    pub replica: ReplicaId,
    /// Next sequence number for this replica. Monotone, never reused — the
    /// state vector in [`crate::sync`] depends on it.
    pub next_seq: u32,
}

impl Repo {
    pub fn merge(&self, other: &ChangeSet) -> ChangeSet {
        todo!()
    }

    /// Cherry-pick: `c` and what it needs, nothing else.
    pub fn adopt(&self, c: ChangeId) -> ChangeSet {
        todo!()
    }

    /// Revert: remove `c` and its dependents. Every surviving change keeps its
    /// id, because ids are content addresses and the log is append-only.
    pub fn drop_change(&self, c: ChangeId) -> ChangeSet {
        todo!()
    }

    pub fn worktree(&self) -> Worktree {
        todo!()
    }

    /// Name events that already exist.
    ///
    /// Recording is **labelling, not writing** — the ops were appended when the
    /// editing happened. In v0 the diff adapter appends them moments earlier, so
    /// the difference is invisible; with a live front-end the events may be
    /// hours old and shared with peers before anyone names them.
    pub fn record(&mut self, events: BTreeSet<EventId>, meta: Meta) -> Change {
        todo!()
    }

    /// Events belonging to no change yet: the live edits, the work in progress.
    pub fn unnamed(&self) -> BTreeSet<EventId> {
        todo!()
    }
}
