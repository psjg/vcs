//! Layer 1: labels over the log, and the dependency reduction.
//!
//! A change is **not** a snapshot and **not** a diff. It is a name for a set of
//! events that already exist. Labels can be unioned and subtracted while the
//! events underneath keep their identity — that is the whole trick, and the
//! reason `drop` does not rewrite anything.

use crate::event::EventLog;
use crate::op::EventId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Content address: `blake3` over the canonical encoding.
///
/// Because no change's bytes mention a change it does not depend on, dropping
/// one cannot perturb another's identity — invariant **I3** is structural, not
/// enforced by code that could forget.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ChangeId(pub [u8; 32]);

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Meta {
    pub message: String,
    pub author: String,
}

/// A named set of events plus the changes it depends on.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Change {
    pub events: BTreeSet<EventId>,
    pub deps: BTreeSet<ChangeId>,
    pub meta: Meta,
}

impl Change {
    /// Name a set of events, deriving `deps` from the ops themselves.
    ///
    /// The rule in full: for every event in the set, map each
    /// [`crate::op::Op::refs`] entry to the change owning that event; drop
    /// self-references. No antichain minimisation — `closure` is identical
    /// either way, so minimisation is normalisation, not semantics (TECHDEBT).
    ///
    /// `owners` is the only context needed, and notably *not* causal history.
    pub fn new(
        events: BTreeSet<EventId>,
        meta: Meta,
        log: &EventLog,
        owners: &BTreeMap<EventId, ChangeId>,
    ) -> Self {
        todo!()
    }

    pub fn id(&self) -> ChangeId {
        todo!()
    }

    /// Canonical encoding. Must stay stable across versions or every id in an
    /// existing repository silently changes meaning.
    fn canonical(&self) -> Vec<u8> {
        todo!()
    }
}

/// Every change a repository knows.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Changes {
    pub by_id: BTreeMap<ChangeId, Change>,
}

impl Changes {
    /// Which change named each event. Events named by no change are live edits
    /// that nobody has recorded yet — ordinary, not an error.
    pub fn owners(&self) -> BTreeMap<EventId, ChangeId> {
        todo!()
    }

    /// `c` and everything it transitively depends on: the smallest legal set
    /// containing `c`.
    pub fn closure(&self, c: ChangeId) -> BTreeSet<ChangeId> {
        todo!()
    }

    /// `c` and everything that transitively depends on it: what must go when
    /// `c` goes.
    pub fn upward_closure(&self, c: ChangeId) -> BTreeSet<ChangeId> {
        todo!()
    }

    /// How much smaller the derived dependency closure is than the causal one,
    /// per change. **The headline measurement of the spike** — near 1.0 means
    /// layer 1 buys nothing and the design is wrong.
    pub fn reduction_ratio(&self, log: &EventLog) -> BTreeMap<ChangeId, f64> {
        todo!()
    }
}

/// A set of changes closed under `deps` — the only thing that can be replayed.
/// The constructor *is* the invariant (**I4**).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangeSet(BTreeSet<ChangeId>);

impl ChangeSet {
    pub fn new(ids: BTreeSet<ChangeId>, changes: &Changes) -> Result<Self, NotClosed> {
        todo!()
    }

    pub fn ids(&self) -> &BTreeSet<ChangeId> {
        todo!()
    }

    /// Union of two closed sets is closed, so this cannot fail. `merge` is this
    /// function and nothing else.
    pub fn union(&self, other: &ChangeSet) -> ChangeSet {
        todo!()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotClosed {
    pub change: ChangeId,
    pub missing_dep: ChangeId,
}
