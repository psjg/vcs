//! One document's weave, kept live: every character that ever existed, in
//! Fugue order, indexed so that identity and coordinates convert in O(log n).
//!
//! [`crate::replay`] builds the same sequence from scratch on every command and
//! throws it away — right for a batch tool, and it stays: it is the oracle this
//! module is tested against. A live editor cannot afford that. Every keystroke
//! arrives as a line and a UTF-16 column (LSP) and must become an anchor
//! ([`Pos`]); every remote edit arrives as an anchor and must become a line and
//! a column to show. Both directions are a seek here.
//!
//! Two [`SumTree`]s over the same fragments, the layout of Zed's `text` crate
//! (GPL-3.0 — the design is borrowed, none of the code):
//!
//! ```text
//! order  (document order)       [loc 1: run A 0..5 ✓][loc 2: run B 0..3 ✗][loc 3: run A 5..9 ✓]
//!                                   seek by: visible chars, UTF-16, line/col, Locator
//! index  (by identity)          [A 0..5 → loc 1][A 5..9 → loc 3][B 0..3 → loc 2]
//!                                   seek by: (run, offset)
//! ```
//!
//! `Pos → coordinates`: find the piece in `index`, take its [`Locator`], seek
//! `order` by it, read the coordinates off the summary before it. The other way
//! is one seek in `order`. The Locator is what joins the two trees: a dense key
//! that sorts like the document, so `order` is searchable by identity as well
//! as by position.
//!
//! Tombstones stay in `order` with `visible: false`. They take no room in any
//! text coordinate, but anchors pointing at deleted characters must still
//! resolve — a peer may have typed next to a character you have since deleted.
//!
//! Characters are grouped in [`Fragment`]s: a maximal stretch of one run with
//! one visibility, capped at [`MAX_FRAGMENT`] characters so a seek *inside* a
//! fragment is a short scan. (Ropey's chunks, diamond-types' run-length items.)

// Skeleton: parameters of unimplemented bodies stay unused until review.
#![allow(unused_variables)]

use crate::event::EventLog;
use crate::op::{Anchor, EventId, NodeId, Op, Pos, Side};
use crate::sumtree::{Bias, Dimension, Item, Summary, SumTree};
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

/// Longest fragment, in characters. Bounds the scan inside one fragment.
pub const MAX_FRAGMENT: u32 = 128;

// --- measures ---------------------------------------------------------------

/// How much text, in every unit something needs.
///
/// `lines` counts `\n` and nothing else; `last_*` is the length of the text
/// after the final `\n`. A `\r` is an ordinary character. For CRLF text that
/// gives the lines LSP counts; a lone `\r`, which LSP also breaks on, is the
/// front-end's to translate. Together they make a (line, column) position a
/// sum: see the [`Summary`] impl. Chars for v0's own offsets, UTF-16 for LSP,
/// bytes for slicing the `str`.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Metrics {
    pub bytes: usize,
    pub chars: usize,
    pub utf16: usize,
    pub lines: u32,
    pub last_chars: u32,
    pub last_utf16: u32,
}

impl Metrics {
    /// The metrics of a string, in one pass.
    pub fn of(text: &str) -> Self {
        todo!()
    }
}

impl Summary for Metrics {
    /// If `other` contains a newline, the column restarts at its own last line;
    /// otherwise columns add up.
    fn add(&mut self, other: &Self) {
        todo!()
    }
}

/// A place in a document as an editor names it: zero-based line, and column in
/// UTF-16 code units — what LSP's `Position` means, surrogate pairs and all.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct PointUtf16 {
    pub line: u32,
    pub col: u32,
}

/// An offset in visible characters: v0's own text coordinate.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct Chars(pub usize);

// --- the document-order tree ------------------------------------------------

/// A dense, totally ordered key: between any two there is always another.
///
/// Compared lexicographically. `between(a, b)` extends the shorter key rather
/// than renumbering anything, so assigning one never touches a neighbour.
/// Idea from Zed's `Locator`. Local to one [`Weave`] and never persisted or
/// synced — identity across replicas is [`Pos`]; this only orders fragments.
///
/// [`Locator::MIN`] and [`Locator::max`] are sentinels no fragment holds, so
/// "before the first" and "after the last" are ordinary `between` calls.
/// `Default` is `MIN`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct Locator(Vec<u32>);

impl Locator {
    pub const MIN: Locator = Locator(Vec::new());

    /// Sorts above every key `between` produces: those never start with
    /// `u32::MAX`.
    pub fn max() -> Locator {
        Locator(vec![u32::MAX])
    }

    /// A key strictly between `a` and `b`. Requires `a < b`.
    pub fn between(a: &Locator, b: &Locator) -> Locator {
        todo!()
    }
}

/// A stretch of one insert's run, all visible or all tombstoned.
#[derive(Clone, Debug)]
pub struct Fragment {
    pub loc: Locator,
    /// The insert this text came from, and where in its text this piece starts,
    /// in characters. The fragment holds `Pos{run, start} ..` onwards.
    pub run: EventId,
    pub start: u32,
    /// The whole run's text, shared by every fragment cut from it: splitting a
    /// fragment copies no text.
    pub text: Arc<str>,
    /// This fragment's slice of `text`, in bytes.
    pub bytes: Range<u32>,
    pub visible: bool,
}

impl Fragment {
    /// The characters this fragment holds.
    pub fn str(&self) -> &str {
        &self.text[self.bytes.start as usize..self.bytes.end as usize]
    }

    /// Cut after `n` characters: `(first n, the rest)`. Both keep `run`, the
    /// right half gets `start + n` and a Locator between this and the next.
    fn split(&self, n: u32, right_loc: Locator) -> (Fragment, Fragment) {
        todo!()
    }
}

/// What `order`'s nodes know about their subtree.
#[derive(Clone, Default, Debug)]
pub struct FragmentSummary {
    /// Visible text only: tombstones take no room in any coordinate.
    pub text: Metrics,
    /// Every character, tombstones included.
    pub atoms: usize,
    /// The largest Locator below: what makes `order` seekable by Locator.
    pub max_loc: Locator,
}

impl Summary for FragmentSummary {
    fn add(&mut self, other: &Self) {
        todo!()
    }
}

impl Item for Fragment {
    type Summary = FragmentSummary;
    fn summary(&self) -> FragmentSummary {
        todo!()
    }
}

impl Dimension<FragmentSummary> for Chars {
    fn add_summary(&mut self, s: &FragmentSummary) {
        self.0 += s.text.chars;
    }
}

impl Dimension<FragmentSummary> for PointUtf16 {
    fn add_summary(&mut self, s: &FragmentSummary) {
        todo!()
    }
}

impl Dimension<FragmentSummary> for Locator {
    fn add_summary(&mut self, s: &FragmentSummary) {
        todo!()
    }
}

// --- the identity index -----------------------------------------------------

/// Where a stretch of a run lives in `order`. One per fragment; kept in step
/// with every split.
#[derive(Clone, Debug)]
pub struct Piece {
    pub run: EventId,
    pub start: u32,
    pub len: u32,
    pub loc: Locator,
}

/// The largest `(run, start)` below: `index` is sorted by it.
#[derive(Clone, Default, Debug)]
pub struct PieceSummary {
    pub max_key: Option<(EventId, u32)>,
}

impl Summary for PieceSummary {
    fn add(&mut self, other: &Self) {
        todo!()
    }
}

impl Item for Piece {
    type Summary = PieceSummary;
    fn summary(&self) -> PieceSummary {
        todo!()
    }
}

/// Seeking `index` by identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct Key(pub Option<(EventId, u32)>);

impl Dimension<PieceSummary> for Key {
    fn add_summary(&mut self, s: &PieceSummary) {
        todo!()
    }
}

// --- the weave --------------------------------------------------------------

/// What an applied op did to the visible text, in the coordinates the text had
/// *before* it: replace `old` with `new_len` characters. What a live front-end
/// forwards to the editor when a peer's edit lands.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Edit {
    pub old: Range<usize>,
    pub new_len: usize,
}

/// One document, live.
#[derive(Clone)]
pub struct Weave {
    pub node: NodeId,
    order: SumTree<Fragment>,
    index: SumTree<Piece>,
    /// Each run's current place in the Fugue tree: its insert's anchor, or the
    /// latest `MoveRun`'s. Placing a new run needs its neighbours' ancestry
    /// ([`crate::capture::between`]), and that is not in the trees.
    parents: BTreeMap<EventId, (Anchor, Side)>,
    /// The Fugue tree's explicit children, siblings sorted by EventId -- the
    /// map replay's pass 3 builds. The trees say where a run *is*; this says
    /// where a new one *goes*.
    children: BTreeMap<(Anchor, Side), Vec<EventId>>,
}

impl Weave {
    /// An empty document.
    pub fn new(node: NodeId) -> Self {
        todo!()
    }

    /// Build from the replay's walk, tombstones included, in O(n): consecutive
    /// atoms of one run with one visibility become one fragment.
    ///
    /// The fast path for opening a document. Needs a replay variant that keeps
    /// dead atoms (today's walk drops them): `(atom, visible)` in order.
    pub fn from_walk(node: NodeId, walk: &[(Pos, char, bool)], log: &EventLog) -> Self {
        todo!()
    }

    /// Apply one event's op, local or remote alike. Ops about other documents
    /// and tree ops are ignored (`None`).
    ///
    /// The *slow* path for building, the only path for editing. Folding `apply`
    /// over any causal order of a set's events must give what
    /// [`crate::replay`] gives for the set — the convergence property, and the
    /// test.
    ///
    /// **Insert** of run `e` at `(parent, side)`, by the walk's own rule
    /// (replay's `Walk::run`): siblings in EventId order, a run's implicit
    /// right child (its next character) keyed by the run's own id.
    /// - a *larger* sibling `s` exists: `e` lands right before the first
    ///   fragment of `s`'s subtree -- `s`'s leftmost descendant down its left
    ///   sides, found through `children`, then one `index` seek;
    /// - `e` is the largest: it lands right after the end of the subtree under
    ///   `(parent, side)`, found by following the rightmost child down, then
    ///   one `index` seek.
    ///
    /// Both walks cost the depth of the Fugue tree, not the length of the
    /// text. Local typing is the cheap case: a new event has the largest id,
    /// and `between` already chose the parent that puts it at the cursor.
    ///
    /// **Delete**: split at the clamped range's ends, mark the middle
    /// invisible. **MoveRun**: the run's subtree is contiguous, so cut it out
    /// and place it as an insert; every moved fragment gets a new Locator,
    /// O(fragments moved).
    pub fn apply(&mut self, id: EventId, op: &Op) -> Option<Edit> {
        todo!()
    }

    // --- reading --------------------------------------------------------

    /// The visible text.
    pub fn text(&self) -> String {
        todo!()
    }

    /// The visible text's size, in every unit.
    pub fn metrics(&self) -> Metrics {
        todo!()
    }

    // --- converting -----------------------------------------------------

    /// Where a character is, in visible characters before it, and whether it
    /// is itself visible. Resolves tombstones too: a deleted character sits
    /// where it would be. `None` if the weave has never seen it.
    pub fn offset_of(&self, p: Pos) -> Option<(Chars, bool)> {
        todo!()
    }

    /// The visible character at `offset`. `Bias::Left` takes the one before a
    /// boundary, `Bias::Right` the one after it.
    pub fn pos_at(&self, offset: Chars, bias: Bias) -> Option<Pos> {
        todo!()
    }

    /// An LSP position as a character offset. A column past the end of its
    /// line clamps to the line's end, as LSP specifies.
    pub fn offset_of_point(&self, p: PointUtf16) -> Chars {
        todo!()
    }

    pub fn point_of_offset(&self, offset: Chars) -> PointUtf16 {
        todo!()
    }

    // --- producing ops (the live capture path) -------------------------

    /// The op for typing `text` at visible `offset`: the anchor and side
    /// [`crate::capture::between`] picks for the visible characters on either
    /// side. Pure: the caller appends it to the log, then `apply`s it.
    pub fn insert_op(&self, offset: Chars, text: &str) -> Op {
        todo!()
    }

    /// The ops for deleting the visible characters in `range`: one `Delete`
    /// per run it crosses, each range already exact.
    pub fn delete_ops(&self, range: Range<Chars>) -> Vec<Op> {
        todo!()
    }
}
