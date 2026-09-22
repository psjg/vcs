//! The structure layer: a syntax tree that follows a weave.
//!
//! The weave says *where* and remembers it: characters with anchors that
//! survive every edit. The tree says *what*: nodes, recomputed after every
//! edit, with no identity of their own. This module is the bridge, in both
//! directions, and each crossing is O(log n):
//!
//! - node -> its span -> the anchors of its first and last character
//!   ([`Weave::pos_at`]): a [`NodeRef`], the only form a node takes outside
//!   this module and the only one an op may name;
//! - anchors -> offsets ([`Weave::offset_of`]) -> the node on that span
//!   (`descendant_for_byte_range`).
//!
//! The weave is also the parser's input: tree-sitter reads text through a
//! callback that asks for the chunk at a byte ([`Weave::chunk_at`]).
//!
//! **Structural ops refer to anchors, never to nodes.** A [`Op::Rename`]
//! names the anchor of the declared identifier; applying it finds the node,
//! its references through the grammar's scope query, their anchors, and
//! from those the plain character ops that do the work. The character CRDT
//! stays the ground truth of the text; this layer is how intents are
//! computed, and where they are re-computed when a merge brings a
//! reference the intent had not seen (see [`reapply`]).
//!
//! Structure is a property of the document type, not of the store. A
//! document without a grammar has none of this and loses nothing else;
//! lines and paragraphs as nodes are the next rung down (TECHDEBT), not a
//! trait to design ahead of them.
//!
//! The oracle: after any sequence of edits, the tree followed incrementally
//! equals a fresh parse of the text (`tests/syntax.rs`).

// Skeleton: signatures with their contracts, bodies to follow.
#![allow(unused_variables, dead_code)]

use crate::event::{Event, EventLog};
use crate::op::{EventId, Op, Pos};
use crate::weave::{Chars, Edit, Weave};
use std::ops::Range;
use tree_sitter::{Language, Parser, Tree};

/// Which grammar a document is read with. Rust first; Markdown second, and
/// it is two grammars (block and inline) behind one name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grammar {
    Rust,
    Markdown,
}

impl Grammar {
    /// The grammar for a file name, by extension. `None`: no structure.
    pub fn for_name(name: &str) -> Option<Grammar> {
        todo!("rs -> Rust, md -> Markdown")
    }

    fn language(self) -> Language {
        todo!("tree_sitter_rust::LANGUAGE / tree_sitter_md::LANGUAGE")
    }

    /// The grammar's scope query (`locals.scm`): definitions, references and
    /// the scopes that bind them. What `references` resolves with.
    fn locals(self) -> &'static str {
        todo!("tree_sitter_rust::LOCALS_QUERY; Markdown has none")
    }
}

/// A syntax node named by anchors: the first and last character of its span.
/// Stable across edits, unlike a `tree_sitter::Node`, and re-found from the
/// weave in O(log n). The node it names may have grown, shrunk or ceased to
/// parse as that kind; [`Syntax::resolve`] says.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct NodeRef {
    pub first: Pos,
    pub last: Pos,
}

/// One document's tree, kept in step with its weave.
pub struct Syntax {
    grammar: Grammar,
    parser: Parser,
    tree: Tree,
}

impl Syntax {
    /// Parse a weave's whole text. O(n); the once-per-open cost.
    pub fn of(weave: &Weave, grammar: Grammar) -> Syntax {
        todo!()
    }

    /// Follow the edits one [`Weave::apply`] reported, from the weave as it
    /// was (`before`, an O(1) clone taken first) to the weave as it is.
    /// Each edit becomes a tree-sitter `InputEdit` -- bytes and (row, byte
    /// column) at its start, old end and new end -- read off
    /// [`Weave::metrics_at`] in the two weaves; then one incremental reparse
    /// over `after`. Returns the nodes whose text changed, as refs.
    pub fn follow(&mut self, before: &Weave, after: &Weave, edits: &[Edit]) -> Vec<NodeRef> {
        todo!()
    }

    /// The smallest named node spanning `range`, as a ref.
    pub fn node_at(&self, weave: &Weave, range: Range<Chars>) -> Option<NodeRef> {
        todo!()
    }

    /// The node a ref names now, if its span still parses as one node.
    pub fn resolve<'a>(&'a self, weave: &Weave, node: NodeRef) -> Option<tree_sitter::Node<'a>> {
        todo!()
    }

    /// The kind of the node a ref names now (`function_item`, ...).
    pub fn kind(&self, weave: &Weave, node: NodeRef) -> Option<&'static str> {
        todo!()
    }

    /// Every reference to the identifier declared at `decl`, the declaration
    /// included, as character ranges: the grammar's scope query, resolved
    /// within this document. Rust only; cross-file binding is a language
    /// server's job and out of scope here.
    pub fn references(&self, weave: &Weave, decl: Pos) -> Vec<Range<Chars>> {
        todo!()
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }
}

// --- intents ----------------------------------------------------------------

/// Expand a rename at record time: for every reference of the identifier
/// declared at `decl`, a delete of its characters and an insert of `to`. The
/// intent itself is recorded alongside as [`Op::Rename`]; these are its
/// effect, ordinary events in the weave.
pub fn rename(weave: &Weave, syntax: &Syntax, decl: Pos, to: &str) -> Vec<Op> {
    todo!()
}

/// The id of the effect a rename has on one reference it had not seen: the
/// same on every replica that computes it, so two merges of the same
/// histories mint the same event and it dedups, rather than renaming twice.
/// Lamport-valid: later than both the rename and the reference's insert.
pub fn derived_id(rename: EventId, at: Pos) -> EventId {
    todo!("seq = max(rename.seq, at.event.seq) + 1; replica = hash(rename, at)")
}

/// At merge: references of the renamed identifier that still read the old
/// name -- inserted concurrently with the rename -- get its effect, as
/// events with [`derived_id`]s. What makes a rename an intent rather than a
/// batch of edits: the acceptance test is Alice renaming `foo` to `bar`
/// while Bob adds a call to `foo`, and Bob's call reading `bar` after the
/// merge.
pub fn reapply(weave: &Weave, syntax: &Syntax, log: &EventLog, rename: EventId) -> Vec<Event> {
    todo!()
}
