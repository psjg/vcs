//! The materialiser `M(S)`.
//!
//! A weave holds every atom that has ever existed in one permanent total order.
//! Materialising is then a filter, not a merge: walk the weave, keep the atoms
//! whose creating op is in the set and whose deleting op is not.
//!
//! This is why `merge` needs no algorithm. `M` reads a *set*; union two sets and
//! the same walk produces the merged document. Commutativity is not implemented,
//! it is a consequence.

use crate::change::{ChangeSet, Log};
use crate::op::OpId;

/// One line that has ever existed, alive or tombstoned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Atom {
    /// The insert that created it — also its position identity.
    pub id: OpId,
    pub line: String,
    /// Deletes targeting this atom. An atom is alive when none of them is in
    /// the materialised set.
    pub deleted_by: Vec<OpId>,
}

/// The materialised working-copy content.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Document {
    pub lines: Vec<String>,
}

/// Every atom of a log, in the permanent order.
#[derive(Clone, Default, Debug)]
pub struct Weave {
    pub atoms: Vec<Atom>,
}

impl Weave {
    /// Build the weave of every op in `log`.
    ///
    /// Order is decided here and only here: siblings sharing an anchor sort by
    /// [`OpId`] descending, then each is followed by its own subtree. The order
    /// must not depend on which subset will later be materialised, or `M` stops
    /// being a pure function of the set (**I1**).
    pub fn build(log: &Log) -> Self {
        todo!()
    }

    /// `M(S)` — keep atoms created by the set and not deleted by it.
    pub fn materialise(&self, set: &ChangeSet, log: &Log) -> Document {
        todo!()
    }
}
