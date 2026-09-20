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

use crate::op::{EventId, NodeId, NodeKind, Op};
use std::collections::{BTreeMap, BTreeSet};

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
    /// Nodes removed, with their subtrees.
    pub removed: BTreeSet<NodeId>,
    /// Moves declined because they would have closed a cycle. Reported rather
    /// than swallowed: a user whose move silently vanished deserves to know.
    pub declined: Vec<NodeId>,
}

impl Tree {
    /// Apply a move, or decline it because it would make `node` its own
    /// ancestor. Returns whether it was applied — replay records the decision
    /// so it can be reported rather than silently swallowed.
    pub fn try_move(&mut self, node: NodeId, parent: NodeId, name: String) -> bool {
        // Kleppmann's rule: a move that would make `node` its own ancestor is
        // skipped. Because replay visits moves in `EventId` order on every
        // replica, they all skip the same one and converge without talking.
        if self.is_ancestor(node, parent) {
            self.declined.push(node);
            return false;
        }
        if let Some(n) = self.nodes.get_mut(&node) {
            n.parent = parent;
            n.name = name;
            true
        } else {
            false
        }
    }

    /// Is `maybe_ancestor` on the path from `node` to the root?
    pub fn is_ancestor(&self, maybe_ancestor: NodeId, node: NodeId) -> bool {
        let mut cur = node;
        // Bounded: the no-cycles invariant is what this function protects, so
        // it must not assume the invariant it is enforcing.
        for _ in 0..=self.nodes.len() {
            if cur == maybe_ancestor {
                return true;
            }
            match self.nodes.get(&cur) {
                Some(n) if n.parent != cur => cur = n.parent,
                _ => return false,
            }
        }
        true // only reachable if a cycle already exists: refuse the move
    }

    /// Path from the root, for rendering a worktree.
    pub fn path(&self, node: NodeId) -> Option<String> {
        let mut parts = Vec::new();
        let mut cur = node;
        for _ in 0..=self.nodes.len() {
            if self.removed.contains(&cur) {
                return None;
            }
            if cur == Op::ROOT {
                break;
            }
            let n = self.nodes.get(&cur)?;
            parts.push(n.name.clone());
            cur = n.parent;
        }
        if cur != Op::ROOT {
            return None; // ran out of steps: unreachable from the root
        }
        parts.reverse();
        Some(parts.join("/"))
    }
}
