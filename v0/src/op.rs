//! Identities and edit operations — the payload of an event.
//!
//! The op set is a design choice, not an inheritance (ADR-0004/0005). It covers
//! three families: a sequence of lines inside documents, a tree of nodes, and a
//! register for per-node metadata. **Move is first class in both the sequence
//! and the tree**, because a move expressed as delete-plus-insert destroys
//! identity — which is exactly why git has to guess renames with similarity
//! heuristics.

use serde::{Deserialize, Serialize};

/// One working copy. Not one human: two clones by the same person must not mint
/// colliding ids.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ReplicaId(pub u64);

/// A globally unique, totally ordered event identity.
///
/// Ordered `(seq, replica)`. Because `seq` is monotone per replica, the map
/// `{replica -> max seq}` is a complete state vector — see [`crate::sync`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct EventId {
    pub seq: u32,
    pub replica: ReplicaId,
}

/// A node in the worktree: the event that created it names it forever, so a
/// rename cannot break identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct NodeId(pub EventId);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum NodeKind {
    Dir,
    File,
}

/// Where a line attaches. Never an index — indices are what make concurrent
/// edits fight.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Anchor {
    /// The left edge of a document.
    DocStart(NodeId),
    /// Directly after the atom created by this event.
    After(EventId),
}

/// One indivisible edit.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Op {
    // --- sequence -----------------------------------------------------------
    Insert { anchor: Anchor, line: String },
    Delete { target: EventId },
    /// Identity-preserving move of one atom, possibly into another document.
    /// Its dependency is the moved atom, not the text around it — which is what
    /// makes moved code cherry-pickable.
    MoveLine { target: EventId, to: Anchor },
    // --- tree ---------------------------------------------------------------
    Create { node: NodeId, parent: NodeId, name: String, kind: NodeKind },
    /// Rename *is* move. Cycles are resolved at replay (see [`crate::tree`]).
    MoveNode { node: NodeId, parent: NodeId, name: String },
    Remove { node: NodeId },
    // --- register -----------------------------------------------------------
    SetMode { node: NodeId, mode: u32 },
}

impl Op {
    /// The events this op **semantically** refers to: an anchor, a target, a
    /// parent node.
    ///
    /// This one function is the entire input to dependency derivation. Note
    /// what it is not: the author's causal history. That distinction — recorded
    /// causal parents versus derived semantic references — is the spike.
    pub fn refs(&self) -> Vec<EventId> {
        let anchor_ref = |a: &Anchor| match *a {
            Anchor::DocStart(n) => n.0,
            Anchor::After(e) => e,
        };
        match self {
            Op::Insert { anchor, .. } => vec![anchor_ref(anchor)],
            Op::Delete { target } => vec![*target],
            Op::MoveLine { target, to } => vec![*target, anchor_ref(to)],
            Op::Create { parent, .. } => vec![parent.0],
            Op::MoveNode { node, parent, .. } => vec![node.0, parent.0],
            Op::Remove { node } => vec![node.0],
            Op::SetMode { node, .. } => vec![node.0],
        }
    }

    /// The root of the worktree: a node nobody created, so nothing depends on
    /// it. Referring to it yields no dependency, which is what stops every
    /// change from depending on the first one ever made.
    pub const ROOT: NodeId = NodeId(EventId { seq: 0, replica: ReplicaId(0) });
}
