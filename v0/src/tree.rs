//! The worktree as a replicated tree, with a move that cannot break.
//!
//! Concurrent moves are where naive tree replication produces cycles, orphans
//! or duplicated subtrees — Google Drive and Dropbox have both shipped the bug.
//! We use Kleppmann's rule: replay moves in a deterministic order and **skip any
//! move that would close a cycle**. Every replica skips the same one, so they
//! converge without coordination (**I11**).
//!
//! Reference: Kleppmann et al., *A highly-available move operation for
//! replicated trees* (2021).

use crate::op::{EventId, NodeId, NodeKind};
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node {
    pub parent: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub mode: u32,
    /// Event of the last [`crate::op::Op::SetMode`] — the register's timestamp.
    pub mode_set_by: Option<EventId>,
}

/// Tree state during a replay. Transient, like the weave: built, read, dropped.
#[derive(Clone, Default, Debug)]
pub struct Tree {
    pub nodes: BTreeMap<NodeId, Node>,
}

impl Tree {
    /// Apply a move, or decline it because it would make `node` its own
    /// ancestor. Returns whether it was applied — replay records the decision
    /// so it can be reported rather than silently swallowed.
    pub fn try_move(&mut self, node: NodeId, parent: NodeId, name: String) -> bool {
        todo!()
    }

    /// Is `maybe_ancestor` on the path from `node` to the root?
    pub fn is_ancestor(&self, maybe_ancestor: NodeId, node: NodeId) -> bool {
        todo!()
    }

    /// Path from the root, for rendering a worktree.
    pub fn path(&self, node: NodeId) -> Option<String> {
        todo!()
    }
}
