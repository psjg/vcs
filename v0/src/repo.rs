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

use crate::change::{components, Change, ChangeId, ChangeSet, Changes, Meta};
use crate::event::EventLog;
use crate::op::{EventId, ReplicaId};
use crate::replay::{Materialiser, WeaveReplay, Worktree};
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
        self.head.union(other)
    }

    /// Cherry-pick: `c` and what it needs, nothing else.
    pub fn adopt(&self, c: ChangeId) -> ChangeSet {
        let ids = self.head.ids().union(&self.changes.closure(c)).copied().collect();
        ChangeSet::new(ids, &self.changes).expect("a closure added to a closed set stays closed")
    }

    /// Revert: remove `c` and its dependents. Every surviving change keeps its
    /// id, because ids are content addresses and the log is append-only.
    pub fn drop_change(&self, c: ChangeId) -> ChangeSet {
        let doomed = self.changes.upward_closure(c);
        let ids = self.head.ids().difference(&doomed).copied().collect();
        ChangeSet::new(ids, &self.changes)
            .expect("removing a change and all its dependents leaves a closed set")
    }

    pub fn worktree(&self) -> Worktree {
        WeaveReplay.materialise(&self.head, &self.changes, &self.log)
    }

    /// Name events that already exist.
    ///
    /// Recording is **labelling, not writing** — the ops were appended when the
    /// editing happened. In v0 the diff adapter appends them moments earlier, so
    /// the difference is invisible; with a live front-end the events may be
    /// hours old and shared with peers before anyone names them.
    pub fn record(&mut self, events: BTreeSet<EventId>, meta: Meta) -> Vec<ChangeId> {
        // ADR-0007: the author is not asked what belongs together, it is derived
        // from what the edits actually reference. One change per component.
        let mut minted = Vec::new();
        for part in components(&events, &self.log) {
            let change = Change::new(part, meta.clone(), &self.log, &self.changes.owners());
            let id = change.id();
            self.changes.by_id.insert(id, change);
            minted.push(id);
        }
        let ids = self.head.ids().iter().copied().chain(minted.iter().copied()).collect();
        self.head = ChangeSet::new(ids, &self.changes)
            .expect("a new change's dependencies are already in head");
        minted
    }

    /// Events belonging to no change yet: the live edits, the work in progress.
    pub fn unnamed(&self) -> BTreeSet<EventId> {
        let named = self.changes.owners();
        self.log.events.keys().copied().filter(|e| !named.contains_key(e)).collect()
    }
}
