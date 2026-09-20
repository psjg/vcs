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
        let mut deps = BTreeSet::new();
        for id in &events {
            let Some(ev) = log.events.get(id) else { continue };
            for r in ev.op.refs() {
                // A reference into our own event set is internal, not a
                // dependency. A reference to an event nobody has named yet
                // cannot be a dependency either -- it is a live edit.
                if events.contains(&r) {
                    continue;
                }
                if let Some(owner) = owners.get(&r) {
                    deps.insert(*owner);
                }
            }
        }
        Self { events, deps, meta }
    }

    pub fn id(&self) -> ChangeId {
        ChangeId(*blake3::hash(&self.canonical()).as_bytes())
    }

    /// Canonical encoding. Must stay stable across versions or every id in an
    /// existing repository silently changes meaning.
    fn canonical(&self) -> Vec<u8> {
        // Deterministic because every collection here is ordered (`BTreeSet`)
        // and serde emits struct fields in declaration order.
        serde_json::to_vec(self).expect("a Change is always serialisable")
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
        let mut out = BTreeMap::new();
        for (id, c) in &self.by_id {
            for e in &c.events {
                out.insert(*e, *id);
            }
        }
        out
    }

    /// `c` and everything it transitively depends on: the smallest legal set
    /// containing `c`.
    pub fn closure(&self, c: ChangeId) -> BTreeSet<ChangeId> {
        self.reach(c, |ch| ch.deps.iter().copied().collect())
    }

    /// `c` and everything that transitively depends on it: what must go when
    /// `c` goes.
    pub fn upward_closure(&self, c: ChangeId) -> BTreeSet<ChangeId> {
        let mut dependents: BTreeMap<ChangeId, Vec<ChangeId>> = BTreeMap::new();
        for (id, ch) in &self.by_id {
            for d in &ch.deps {
                dependents.entry(*d).or_default().push(*id);
            }
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![c];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            stack.extend(dependents.get(&id).into_iter().flatten().copied());
        }
        seen
    }

    /// Transitive reachability from `c` through `step`.
    fn reach(&self, c: ChangeId, step: impl Fn(&Change) -> Vec<ChangeId>) -> BTreeSet<ChangeId> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![c];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(ch) = self.by_id.get(&id) {
                stack.extend(step(ch));
            }
        }
        seen
    }

    /// Every event named by a set of changes.
    pub fn events_of(&self, ids: &BTreeSet<ChangeId>) -> BTreeSet<EventId> {
        ids.iter()
            .filter_map(|id| self.by_id.get(id))
            .flat_map(|c| c.events.iter().copied())
            .collect()
    }

    /// How much smaller the derived dependency closure is than the causal one,
    /// per change. **The headline measurement of the spike** — near 1.0 means
    /// layer 1 buys nothing and the design is wrong.
    pub fn reduction_ratio(&self, log: &EventLog) -> BTreeMap<ChangeId, f64> {
        self.by_id
            .iter()
            .map(|(id, ch)| {
                // Both sides counted in events, so the ratio is like for like.
                let causal: BTreeSet<EventId> =
                    ch.events.iter().flat_map(|e| log.causal_closure(*e)).collect();
                let minimal = self.events_of(&self.closure(*id));
                let ratio = if minimal.is_empty() {
                    1.0
                } else {
                    causal.len() as f64 / minimal.len() as f64
                };
                (*id, ratio)
            })
            .collect()
    }
}

/// A set of changes closed under `deps` — the only thing that can be replayed.
/// The constructor *is* the invariant (**I4**).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangeSet(BTreeSet<ChangeId>);

impl ChangeSet {
    pub fn new(ids: BTreeSet<ChangeId>, changes: &Changes) -> Result<Self, NotClosed> {
        for id in &ids {
            let Some(ch) = changes.by_id.get(id) else { continue };
            for d in &ch.deps {
                if !ids.contains(d) {
                    return Err(NotClosed { change: *id, missing_dep: *d });
                }
            }
        }
        Ok(Self(ids))
    }

    pub fn ids(&self) -> &BTreeSet<ChangeId> {
        &self.0
    }

    /// Union of two closed sets is closed, so this cannot fail. `merge` is this
    /// function and nothing else.
    pub fn union(&self, other: &ChangeSet) -> ChangeSet {
        ChangeSet(self.0.union(&other.0).copied().collect())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotClosed {
    pub change: ChangeId,
    pub missing_dep: ChangeId,
}
