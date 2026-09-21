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

/// Which side of its parent a node sits on.
///
/// Fugue's "binary-ish" tree: many nodes may share one parent *and* one side,
/// and the reading order is an in-order traversal — left children, the node,
/// then right children. The side is what stops two people's concurrent blocks
/// from being shuffled together.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

/// One character's identity: the insert that created it, and its offset in
/// that insert's text.
///
/// This is Zed's anchor — an (insertion id, offset) pair — and how every
/// run-length-encoded list CRDT addresses text. An insert of N characters is one
/// event, not N; the characters inside it are addressed rather than stored as
/// events of their own. That keeps the log small while giving every character
/// an identity, which is what makes an edit *inside* a line mergeable.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Pos {
    pub event: EventId,
    pub offset: u32,
}

/// The parent a run attaches to. Never an index — indices are what make
/// concurrent edits fight.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Anchor {
    /// The left edge of a document.
    DocStart(NodeId),
    /// A character: the new run becomes its left or right child.
    At(Pos),
}

/// One indivisible edit.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Op {
    // --- sequence -----------------------------------------------------------
    /// A run of characters, placed by Fugue's rule (see
    /// [`crate::capture::between`]). Only the *first* character carries this
    /// parent and side; each following character is the implicit right child of
    /// the one before it, which is exactly what typing forward produces.
    Insert { parent: Anchor, side: Side, text: String },
    /// Characters `range.0 .. range.1` (by offset) of one insert's run.
    ///
    /// **Never trust the range.** It arrives over sync from peers, and a range of
    /// `(0, u32::MAX)` expanded naively is four billion positions — one such
    /// event runs every replica out of memory. Every consumer clamps it to the
    /// run's real length via [`clamp`].
    Delete { target: EventId, range: (u32, u32) },
    /// Identity-preserving move of a whole run, possibly into another document.
    /// Its dependency is the moved run, not the text around it. Whole runs only:
    /// moving part of a run means splitting it, and no capture adapter emits
    /// moves yet (TECHDEBT).
    MoveRun { target: EventId, parent: Anchor, side: Side },
    // --- tree ---------------------------------------------------------------
    Create { node: NodeId, parent: NodeId, name: String, kind: NodeKind },
    /// Rename *is* move. Cycles are resolved at replay (see [`crate::tree`]).
    MoveNode { node: NodeId, parent: NodeId, name: String },
    Remove { node: NodeId },
    /// Undo a removal while keeping the node's identity, so its lines and their
    /// history come back intact rather than as a new file. How a "keep the file"
    /// resolution of a remove-versus-edit conflict is expressed.
    Restore { node: NodeId },
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
            Anchor::At(p) => p.event,
        };
        match self {
            Op::Insert { parent, .. } => vec![anchor_ref(parent)],
            Op::Delete { target, .. } => vec![*target],
            Op::MoveRun { target, parent, .. } => vec![*target, anchor_ref(parent)],
            Op::Create { parent, .. } => vec![parent.0],
            Op::MoveNode { node, parent, .. } => vec![node.0, parent.0],
            Op::Remove { node } | Op::Restore { node } => vec![node.0],
            Op::SetMode { node, .. } => vec![node.0],
        }
    }

    /// The character positions this op touches, for deciding which edits
    /// belong together (ADR-0007). Finer than [`Op::refs`] on purpose: a file
    /// created in one go is a single run, so "refers to the same event" would
    /// weld every later edit anywhere in that file into one change. Two edits
    /// belong together when they touch the same *character*.
    pub fn positions(&self, run_len: &dyn Fn(EventId) -> u32) -> Vec<Pos> {
        match self {
            Op::Insert { parent: Anchor::At(p), .. } => vec![*p],
            Op::Delete { target, range } => {
                clamp(*range, run_len(*target)).map(|offset| Pos { event: *target, offset }).collect()
            }
            Op::MoveRun { target, parent, .. } => {
                let mut v = vec![Pos { event: *target, offset: 0 }];
                if let Anchor::At(p) = parent {
                    v.push(*p);
                }
                v
            }
            _ => Vec::new(),
        }
    }

    /// The root of the worktree: a node nobody created, so nothing depends on
    /// it. Referring to it yields no dependency, which is what stops every
    /// change from depending on the first one ever made.
    pub const ROOT: NodeId = NodeId(EventId { seq: 0, replica: ReplicaId(0) });
}

/// A delete range cut down to what the run actually holds. The only way a
/// range may be expanded into positions — see [`Op::Delete`].
pub fn clamp(range: (u32, u32), len: u32) -> std::ops::Range<u32> {
    let end = range.1.min(len);
    range.0.min(end)..end
}
