//! Changes: a named set of ops, and the dependency relation between them.
//!
//! This module holds the spike's actual research question. A CRDT op log gives
//! *causal* parents — everything a replica had seen. Patch theory needs
//! *minimal* dependencies — what a change genuinely requires. The difference is
//! the difference between a cherry-pick that drags the whole history along and
//! one that takes what it needs.

use crate::op::{Op, OpId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Content address of a change: `blake3` over its canonical encoding.
///
/// Content-addressed on purpose. Because no change's bytes mention a change it
/// does not depend on, dropping one cannot perturb another's identity —
/// invariant **I3** is then structural rather than enforced.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ChangeId(pub [u8; 32]);

/// Human-facing metadata. Part of the hash: editing a message mints a new change.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Meta {
    pub message: String,
    pub author: String,
}

/// A named set of ops plus the minimal set of changes it depends on.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Change {
    pub ops: Vec<Op>,
    pub deps: BTreeSet<ChangeId>,
    pub meta: Meta,
}

impl Change {
    /// Mint a change from ops, deriving `deps` from the ops themselves.
    ///
    /// `owners` maps every op id already in the log to the change that minted
    /// it — the only context needed, and notably *not* a causal history.
    pub fn new(ops: Vec<Op>, meta: Meta, owners: &BTreeMap<OpId, ChangeId>) -> Self {
        todo!()
    }

    /// This change's content address.
    pub fn id(&self) -> ChangeId {
        todo!()
    }

    /// Canonical byte encoding. Must be stable across versions of the program
    /// or every id in an existing repository changes meaning.
    fn canonical(&self) -> Vec<u8> {
        todo!()
    }
}

/// Every change known to a repository, dependency-indexed.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Log {
    pub changes: BTreeMap<ChangeId, Change>,
}

impl Log {
    /// Which change minted each op id.
    pub fn owners(&self) -> BTreeMap<OpId, ChangeId> {
        todo!()
    }

    /// `c` plus everything it transitively depends on. Materialising this is
    /// always legal; it is the smallest legal set containing `c`.
    pub fn closure(&self, c: ChangeId) -> BTreeSet<ChangeId> {
        todo!()
    }

    /// `c` plus everything that transitively depends on `c` — what must go if
    /// `c` goes.
    pub fn upward_closure(&self, c: ChangeId) -> BTreeSet<ChangeId> {
        todo!()
    }

    /// Reduce a dependency set to an antichain: drop any member reachable from
    /// another member, since it is already implied.
    pub fn minimise(&self, deps: BTreeSet<ChangeId>) -> BTreeSet<ChangeId> {
        todo!()
    }
}

/// A dependency-closed set of changes — the only thing that can be materialised.
///
/// The constructor is the invariant (**I4**): there is no way to hold a
/// `ChangeSet` that names a change without its dependencies, so no code
/// downstream needs to check.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangeSet(BTreeSet<ChangeId>);

impl ChangeSet {
    /// Fails with the offending pair when `ids` is not dependency-closed.
    pub fn new(ids: BTreeSet<ChangeId>, log: &Log) -> Result<Self, NotClosed> {
        todo!()
    }

    pub fn ids(&self) -> &BTreeSet<ChangeId> {
        todo!()
    }
}

/// A change was named without one of its dependencies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotClosed {
    pub change: ChangeId,
    pub missing_dep: ChangeId,
}
