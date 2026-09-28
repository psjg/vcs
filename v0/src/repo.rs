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
use crate::replay::{state_hash, Materialiser, TreeHash, WeaveReplay, Worktree};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A world, named by its frontier, with the hash of what it materialises to.
///
/// `frontier` is the head's maximal changes: the antichain that determines
/// the closed set (theory/FORMAL.md 2.2, ideal ↔ antichain), so it is the
/// normal form of a head, not a copy of it. `tree` is [`state_hash`] of
/// `M(frontier)`: what the world *is*, recomputable by anyone who holds the
/// changes. Two replicas that reach the same world by any route compute the
/// same checkpoint (**I16**). It is verifiable, not attributable: there is
/// no signature here, and `tree` says nothing about who vouches for the
/// state; an attestation over it is a separate record.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub frontier: BTreeSet<ChangeId>,
    pub tree: TreeHash,
}

impl Checkpoint {
    /// The world the frontier names: its closure, as a closed set.
    pub fn world(&self, changes: &Changes) -> Result<ChangeSet, crate::change::NotClosed> {
        let ids = self.frontier.iter().flat_map(|c| changes.closure(*c)).collect();
        ChangeSet::new(ids, changes)
    }

    /// Does the claim hold against this repository's changes and log? A fresh
    /// materialisation of the frontier's world hashes to `tree`.
    pub fn holds(&self, repo: &Repo) -> bool {
        self.world(&repo.changes)
            .map(|w| state_hash(&WeaveReplay.materialise(&w, &repo.changes, &repo.log)) == self.tree)
            .unwrap_or(false)
    }
}

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

    /// The head as a [`Checkpoint`]: its frontier and the hash of its worktree.
    pub fn checkpoint(&self) -> Checkpoint {
        let ids = self.head.ids();
        let below: BTreeSet<ChangeId> =
            ids.iter().filter_map(|c| self.changes.by_id.get(c)).flat_map(|c| c.deps.iter().copied()).collect();
        let frontier = ids.iter().filter(|c| !below.contains(c)).copied().collect();
        Checkpoint { frontier, tree: state_hash(&self.worktree()) }
    }

    /// Name events that already exist.
    ///
    /// Recording is **labelling, not writing** — the ops were appended when the
    /// editing happened. In v0 the diff adapter appends them moments earlier, so
    /// the difference is invisible; with a live front-end the events may be
    /// hours old and shared with peers before anyone names them.
    pub fn record(
        &mut self,
        events: BTreeSet<EventId>,
        hints: &[BTreeSet<EventId>],
        meta: Meta,
    ) -> Vec<ChangeId> {
        // ADR-0007: the author is not asked what belongs together, it is derived
        // from what the edits actually reference. One change per component.
        let mut minted = Vec::new();
        for part in components(&events, &self.log, hints) {
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

    /// Record a resolution: one change — never split, it is one decision — that
    /// declares which conflicts it resolves and depends on every side of them.
    ///
    /// The declared dependency is what structure alone cannot give. A resolver
    /// who keeps Bob's line and deletes Alice's only *references* Alice's change,
    /// yet the decision is meaningless without both: adopting the resolution
    /// must bring the conflict along, and dropping either side must take the
    /// resolution with it.
    ///
    /// `events` may be empty. Keeping both sides exactly as they are is a real
    /// decision, and it deserves a record and a reason like any other.
    pub fn resolve(
        &mut self,
        events: BTreeSet<EventId>,
        resolved: &[crate::conflict::Conflict],
        mut meta: Meta,
    ) -> ChangeId {
        meta.resolves = resolved.iter().map(|c| c.id).collect();
        let mut change = Change::new(events, meta, &self.log, &self.changes.owners());
        for c in resolved {
            change.deps.extend(c.sides.iter().copied());
        }
        let id = change.id();
        self.changes.by_id.insert(id, change);
        let ids = self.head.ids().iter().copied().chain([id]).collect();
        self.head = ChangeSet::new(ids, &self.changes)
            .expect("a resolution's sides are already in head");
        id
    }

    /// Events belonging to no change yet: the live edits, the work in progress.
    pub fn unnamed(&self) -> BTreeSet<EventId> {
        let named = self.changes.owners();
        self.log.events().keys().copied().filter(|e| !named.contains_key(e)).collect()
    }
}
