//! Identities and edit operations.
//!
//! An op is the smallest thing with a name. Everything above this module is
//! bookkeeping over sets of these.

use serde::{Deserialize, Serialize};

/// Who minted an op. One per working copy, not per human: two clones by the
/// same person must not mint colliding ids.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ReplicaId(pub u64);

/// A globally unique, totally ordered op identity.
///
/// The order is `(seq, replica)` — sequence first so that a replica's own ops
/// stay in mint order, `replica` only as a tiebreak between concurrent ops.
/// This total order is what makes sibling ordering at an anchor deterministic
/// (see DESIGN.md, *Ordering rule*).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct OpId {
    pub seq: u32,
    pub replica: ReplicaId,
}

/// Where an insert attaches in the weave.
///
/// Left-origin: an insert names the atom it follows, never an index. Indices
/// are the thing that makes concurrent edits fight; an anchor is stable for the
/// lifetime of the document.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Anchor {
    /// Before every atom — the document's left edge.
    Start,
    /// Directly after the atom created by this op.
    After(OpId),
}

/// One indivisible edit.
///
/// A delete carries its *own* id rather than mutating the insert it targets.
/// That is what lets a delete be dropped independently of the line it removed —
/// the property `git revert` fakes by writing a new commit.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Op {
    Insert { id: OpId, anchor: Anchor, line: String },
    Delete { id: OpId, target: OpId },
}

impl Op {
    /// The identity of this op.
    pub fn id(&self) -> OpId {
        todo!()
    }

    /// The op this one refers to, if any — an insert's anchor or a delete's
    /// target. This single function is where dependency derivation gets its
    /// input; everything else in [`crate::change`] is set arithmetic over it.
    pub fn refers_to(&self) -> Option<OpId> {
        todo!()
    }
}
