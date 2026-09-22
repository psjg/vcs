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

use crate::event::EventLog;
use crate::op::{Anchor, EventId, NodeId, Op, Pos, Side};
use crate::replay::Weaves;
use crate::sumtree::{Bias, Dimension, Item, Summary, SumTree};
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

/// Longest fragment, in characters. Bounds the scan inside one fragment. A
/// guess until the benchmark says otherwise; it sweeps this by building with
/// `V0_MAX_FRAGMENT` set.
pub const MAX_FRAGMENT: u32 = match option_env!("V0_MAX_FRAGMENT") {
    Some(s) => crate::sumtree::parse_usize(s) as u32,
    None => 128,
};

// --- measures ---------------------------------------------------------------

/// How much text, in every unit something needs.
///
/// Lines break the way LSP breaks them: at `\r\n`, `\r` or `\n`. `lines`
/// counts breaks; `last_*` is the length of the text after the final one.
/// Together they make a (line, column) position a sum: see the [`Summary`]
/// impl. Chars for v0's own offsets, UTF-16 for LSP, bytes for slicing.
///
/// A `\r\n` can be torn across two fragments -- typed by two people, or with
/// deleted text between. Each half counts as a break on its own, so the sum
/// has to know where the halves meet: `first_lf` and `last_cr` are that
/// memory, and `add` counts a torn pair once.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Metrics {
    pub bytes: usize,
    pub chars: usize,
    pub utf16: usize,
    pub lines: u32,
    pub last_chars: u32,
    pub last_utf16: u32,
    /// The last line's length in bytes: tree-sitter's column.
    pub last_bytes: u32,
    first_lf: bool,
    last_cr: bool,
}

impl Metrics {
    /// The metrics of a string, in one pass.
    pub fn of(text: &str) -> Self {
        let mut m = Metrics {
            bytes: text.len(),
            first_lf: text.starts_with('\n'),
            last_cr: text.ends_with('\r'),
            ..Metrics::default()
        };
        let mut prev = None;
        for c in text.chars() {
            m.chars += 1;
            m.utf16 += c.len_utf16();
            match c {
                // The break of a `\r\n` was counted at its `\r`.
                '\n' if prev == Some('\r') => {}
                '\r' | '\n' => {
                    m.lines += 1;
                    m.last_chars = 0;
                    m.last_utf16 = 0;
                    m.last_bytes = 0;
                }
                _ => {
                    m.last_chars += 1;
                    m.last_utf16 += c.len_utf16() as u32;
                    m.last_bytes += c.len_utf8() as u32;
                }
            }
            prev = Some(c);
        }
        m
    }
}

impl Summary for Metrics {
    /// If `other` breaks a line, the column restarts at its own last line;
    /// otherwise columns add up. Empty text is the identity, flags included.
    fn add(&mut self, other: &Self) {
        if other.chars == 0 {
            return;
        }
        if self.chars == 0 {
            *self = *other;
            return;
        }
        let torn_crlf = self.last_cr && other.first_lf;
        self.bytes += other.bytes;
        self.chars += other.chars;
        self.utf16 += other.utf16;
        if other.lines > 0 {
            // other.lines >= 1 whenever the pair is torn: its `\n` counted.
            self.lines += other.lines - u32::from(torn_crlf);
            self.last_chars = other.last_chars;
            self.last_utf16 = other.last_utf16;
            self.last_bytes = other.last_bytes;
        } else {
            self.last_chars += other.last_chars;
            self.last_utf16 += other.last_utf16;
            self.last_bytes += other.last_bytes;
        }
        self.last_cr = other.last_cr;
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
/// Compared lexicographically, so a key that is a prefix of another sorts
/// first. `between(a, b)` extends a key rather than renumbering anything, so
/// assigning one never touches a neighbour. Idea from Zed's `Locator`. Local to
/// one [`Weave`] and never persisted or synced — identity across replicas is
/// [`Pos`]; this only orders fragments.
///
/// [`Locator::min`] and [`Locator::max`] are sentinels no fragment holds, so
/// "before the first" and "after the last" are ordinary `between` calls.
/// `Default` is `min`.
///
/// Shared, not owned: every summary up the tree holds its subtree's largest
/// key, so keys are cloned on every sum. An `Arc` makes that a count bump; a
/// `Vec` made it an allocation, which the profile showed was most of a
/// keystroke's cost.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct Locator(Arc<[u32]>);

impl Locator {
    pub fn min() -> Locator {
        Locator::default()
    }

    /// Sorts above every key `between` produces: those never start with
    /// `u32::MAX`.
    pub fn max() -> Locator {
        Locator(Arc::from([u32::MAX]))
    }

    /// A key strictly between `a` and `b`. Requires `a < b`.
    ///
    /// Digit by digit: copy `a` while there is no room, take the midpoint as
    /// soon as there is. Where `a` has run out it reads as 0s; once the copy
    /// has dropped below `b`'s digit, `b` stops bounding (`hi` is 2^32). Each
    /// level of nesting halves the gap, so a key grows by one digit per ~32
    /// insertions at one spot.
    ///
    /// No key ever ends in 0 -- the midpoint is always above `lo` -- and that
    /// is load-bearing: nothing sorts strictly between `k` and `k ++ [0]`.
    pub fn between(a: &Locator, b: &Locator) -> Locator {
        debug_assert!(a < b, "between({a:?}, {b:?})");
        let mut out = Vec::new();
        let mut bounded = true;
        for i in 0.. {
            let lo = a.0.get(i).map_or(0, |&d| i64::from(d));
            let hi = if bounded { b.0.get(i).map_or(0, |&d| i64::from(d)) } else { 1 << 32 };
            if hi - lo >= 2 {
                out.push((lo + (hi - lo) / 2) as u32);
                return Locator(Arc::from(out));
            }
            // No room at this digit: follow `a` and look one level deeper.
            bounded &= lo == hi;
            out.push(lo as u32);
        }
        unreachable!("the loop returns once there is room, which a < b guarantees")
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

    fn len(&self) -> u32 {
        self.str().chars().count() as u32
    }

    /// Cut after `n` characters, `0 < n < len`: `(first n, the rest)`. The
    /// right half starts at `start + n` and takes `right_loc`.
    fn split(&self, n: u32, right_loc: Locator) -> (Fragment, Fragment) {
        let s = self.str();
        let cut = self.bytes.start + s.char_indices().nth(n as usize).map_or(s.len(), |(b, _)| b) as u32;
        let left = Fragment { bytes: self.bytes.start..cut, ..self.clone() };
        let right = Fragment { loc: right_loc, start: self.start + n, bytes: cut..self.bytes.end, ..self.clone() };
        (left, right)
    }
}

/// What `order`'s nodes know about their subtree.
#[derive(Clone, Default, Debug)]
pub struct FragmentSummary {
    /// Visible text only: tombstones take no room in any coordinate.
    pub text: Metrics,
    /// The largest Locator below: what makes `order` seekable by Locator.
    pub max_loc: Locator,
}

impl Summary for FragmentSummary {
    fn add(&mut self, other: &Self) {
        self.text.add(&other.text);
        if other.max_loc > self.max_loc {
            self.max_loc = other.max_loc.clone();
        }
    }
}

impl Item for Fragment {
    type Summary = FragmentSummary;
    fn summary(&self) -> FragmentSummary {
        let text = if self.visible { Metrics::of(self.str()) } else { Metrics::default() };
        FragmentSummary { text, max_loc: self.loc.clone() }
    }
}

impl Dimension<FragmentSummary> for Chars {
    fn add_summary(&mut self, s: &FragmentSummary) {
        self.0 += s.text.chars;
    }
}

impl Dimension<FragmentSummary> for Locator {
    fn add_summary(&mut self, s: &FragmentSummary) {
        if s.max_loc > *self {
            *self = s.max_loc.clone();
        }
    }
}

/// Seeking `order` by LSP position. It carries whole [`Metrics`] rather than a
/// bare point, because only the metrics know whether a torn `\r\n` is being
/// glued back together; it compares by the point alone.
#[derive(Clone, Default, Debug)]
struct PointSeek(Metrics);

impl PointSeek {
    fn at(p: PointUtf16) -> Self {
        PointSeek(Metrics { lines: p.line, last_utf16: p.col, ..Metrics::default() })
    }
    fn key(&self) -> (u32, u32) {
        (self.0.lines, self.0.last_utf16)
    }
}

impl PartialEq for PointSeek {
    fn eq(&self, o: &Self) -> bool {
        self.key() == o.key()
    }
}
impl Eq for PointSeek {}
impl PartialOrd for PointSeek {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for PointSeek {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.key().cmp(&o.key())
    }
}

impl Dimension<FragmentSummary> for PointSeek {
    fn add_summary(&mut self, s: &FragmentSummary) {
        self.0.add(&s.text);
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

/// The largest `(run, end)` below, `end` exclusive: `index` is sorted by it,
/// so the piece holding `(run, offset)` is the first whose end passes it.
#[derive(Clone, Default, Debug)]
pub struct PieceSummary {
    pub max_key: Option<(EventId, u32)>,
}

impl Summary for PieceSummary {
    fn add(&mut self, other: &Self) {
        if other.max_key > self.max_key {
            self.max_key = other.max_key;
        }
    }
}

impl Item for Piece {
    type Summary = PieceSummary;
    fn summary(&self) -> PieceSummary {
        PieceSummary { max_key: Some((self.run, self.start + self.len)) }
    }
}

/// Seeking `index` by identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct Key(pub Option<(EventId, u32)>);

impl Dimension<PieceSummary> for Key {
    fn add_summary(&mut self, s: &PieceSummary) {
        if s.max_key > self.0 {
            self.0 = s.max_key;
        }
    }
}

// --- the weave --------------------------------------------------------------

/// What an applied op did to the visible text: replace the characters `old`
/// with `text`. What a live front-end forwards to the editor when a peer's
/// edit lands.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Edit {
    pub old: Range<usize>,
    pub text: String,
}

/// One document, live.
///
/// A weave knows the state of *every* run in the repository -- text, deletes,
/// anchor, moves -- and lays out only its own document: the characters its
/// walk from `DocStart(node)` reaches. That is what makes a move between
/// documents, or into and out of hiding, an ordinary placement here, as it is
/// in replay. It costs memory per open document (TECHDEBT: share the run
/// state between a repository's weaves).
#[derive(Clone)]
pub struct Weave {
    pub node: NodeId,
    order: SumTree<Fragment>,
    index: SumTree<Piece>,
    /// Each run's current place in the Fugue tree: its insert's anchor, or the
    /// winning `MoveRun`'s. Placing a new run needs its neighbours' ancestry
    /// ([`crate::capture::between`]), and that is not in the trees.
    parents: BTreeMap<EventId, (Anchor, Side)>,
    /// The Fugue tree's explicit children, siblings sorted by EventId -- the
    /// map replay's pass 3 builds. The trees say where a run *is*; this says
    /// where a new one *goes*.
    children: BTreeMap<(Anchor, Side), Vec<EventId>>,
    /// Each run's length in characters: where its last character is.
    lens: BTreeMap<EventId, u32>,
    /// Each run's text, and the stretches of it deleted (clamped, possibly
    /// overlapping). The runs' own state, apart from where they are laid out:
    /// a run a move has hidden keeps it, so a later move can bring it back.
    texts: BTreeMap<EventId, Arc<str>>,
    dead: BTreeMap<EventId, Vec<(u32, u32)>>,
    /// Where each run was inserted, before any move.
    inserted: BTreeMap<EventId, (Anchor, Side)>,
    /// Every move of a run of this document, by its event: `(target, parent,
    /// side)`. Kept so a move arriving out of EventId order can be placed
    /// among the others (see [`Weave::apply`]).
    moves: BTreeMap<EventId, (EventId, Anchor, Side)>,
    /// Moves laid out by relocating one block, and by laying the document out
    /// afresh: the two paths of [`Weave::apply`], counted so a test can see
    /// that both are taken.
    pub relocated: u32,
    pub relaid: u32,
}

/// Where a new fragment goes: right after `left` or right before `right`,
/// the two being adjacent. `None` is the edge of the document.
struct Gap {
    left: Option<Locator>,
    right: Option<Locator>,
}

impl Weave {
    /// An empty document.
    pub fn new(node: NodeId) -> Self {
        Weave {
            node,
            order: SumTree::new(),
            index: SumTree::new(),
            parents: BTreeMap::new(),
            children: BTreeMap::new(),
            lens: BTreeMap::new(),
            texts: BTreeMap::new(),
            dead: BTreeMap::new(),
            inserted: BTreeMap::new(),
            moves: BTreeMap::new(),
            relocated: 0,
            relaid: 0,
        }
    }

    /// Open a document from a replay ([`crate::replay::weaves`]), tombstones
    /// included. O(the repository's text), not the document's: every run is
    /// learnt (see [`Weave`]), and the document laid out in one pass.
    ///
    /// The fast path for opening, against folding [`Weave::apply`]. The runs' places in the Fugue tree, and the
    /// moves that put them there, come from the same replay, so they are the
    /// ones its walk followed; the runs' texts and first anchors come from
    /// `log`.
    pub fn from_walk(node: NodeId, weaves: &Weaves, log: &EventLog) -> Self {
        let mut w = Weave::new(node);
        // Every run of the repository, not just this document's: a move can
        // bring any of them here, from another document or from hiding.
        let all = weaves.walks.values().flatten().map(|(a, alive)| (a.id, *alive));
        let hidden = weaves.hidden.iter().flat_map(|(run, alive)| {
            alive.iter().enumerate().map(|(k, a)| (Pos { event: *run, offset: k as u32 }, *a))
        });
        for (p, alive) in all.chain(hidden) {
            w.learn(p.event, log, weaves);
            if !alive {
                w.dead.entry(p.event).or_default().push((p.offset, p.offset + 1));
            }
        }
        w.moves = weaves.moves.clone();
        for (run, key) in &w.parents {
            w.children.entry(*key).or_default().push(*run);
        }
        let walk = weaves.walks.get(&node).map(Vec::as_slice).unwrap_or_default();
        let frags = lay_out(walk.iter().map(|(a, alive)| (a.id, *alive)), &|e| w.texts.get(&e).cloned());
        w.set_fragments(frags);
        w
    }

    /// Take in a run from the log, placed where replay says.
    fn learn(&mut self, run: EventId, log: &EventLog, weaves: &Weaves) {
        if self.lens.contains_key(&run) {
            return;
        }
        let Some(Op::Insert { parent, side, text }) = log.events().get(&run).map(|e| &e.op) else { return };
        let Some(place) = weaves.anchors.get(&run) else { return };
        self.texts.insert(run, Arc::from(text.as_str()));
        self.lens.insert(run, text.chars().count() as u32);
        self.inserted.insert(run, (*parent, *side));
        self.parents.insert(run, *place);
    }

    /// Replace both trees with `frags`, in document order, keyed afresh.
    fn set_fragments(&mut self, mut frags: Vec<Fragment>) {
        for (i, f) in frags.iter_mut().enumerate() {
            f.loc = Locator(Arc::from([i as u32 + 1]));
        }
        let mut pieces: Vec<Piece> =
            frags.iter().map(|f| Piece { run: f.run, start: f.start, len: f.len(), loc: f.loc.clone() }).collect();
        pieces.sort_by_key(|p| (p.run, p.start));
        self.order = SumTree::from_items(frags);
        self.index = SumTree::from_items(pieces);
    }

    /// Apply one event's op, local or remote alike, in causal order. Returns
    /// what happened to the visible text: edits to apply *in the order given*,
    /// each in the coordinates the previous one left. One delete can hide
    /// several separate stretches -- a run with other people's text inside it.
    ///
    /// Folding `apply` over any causal order of a set's events gives what
    /// [`crate::replay`] gives for the set: the convergence property, and the
    /// test. Ops about other documents, tree ops, repeats (sync re-delivers),
    /// and inserts anchored where replay would never walk are all no-ops.
    ///
    /// **Insert** follows replay's `Walk::run`, the spec: a character `(r,k)`
    /// reads as its left children, itself, its right children smaller than
    /// `r`, the rest of its run (`(r,k+1)`, the *implicit* child, keyed by
    /// `r`), then its right children larger than `r`. The new run lands just
    /// before the subtree of its next-larger sibling, or, being the largest,
    /// just after the subtree of the anchor's side.
    ///
    /// **Delete**: cut out each clamped stretch and hide it.
    ///
    /// **MoveRun**: replay applies moves in EventId order, skipping any that
    /// would put a run inside its own subtree, while `apply` sees causal
    /// order. `parents` is always that fold of `inserted` and `moves` (a move
    /// naming a run not yet known is not recorded, so an insert never has a
    /// move to catch up on), so a move arriving in id order lands on it
    /// directly: it wins unless it is cyclic. One arriving after a larger id
    /// -- Kleppmann's undo, do, redo -- refolds the lot. Then, if exactly one
    /// run's anchor changed, its subtree, one contiguous block of the document
    /// (Fugue), is lifted out and set down at the new anchor: O(k log n) for
    /// k fragments in the block, reported as the block's removal and
    /// insertion. If several changed (a late move that beat another), the
    /// document is laid out afresh with replay's own walk, O(n), reported as
    /// one replacement trimmed of what did not change. [`Weave::relocated`]
    /// and [`Weave::relaid`] count the two.
    pub fn apply(&mut self, id: EventId, op: &Op) -> Vec<Edit> {
        match op {
            Op::Insert { parent, side, text } => self.insert(id, *parent, *side, text).into_iter().collect(),
            Op::Delete { target, range } => self.delete(*target, *range),
            Op::MoveRun { target, parent, side } => self.move_run(id, *target, *parent, *side),
            _ => Vec::new(),
        }
    }

    fn move_run(&mut self, id: EventId, target: EventId, parent: Anchor, side: Side) -> Vec<Edit> {
        if self.moves.contains_key(&id) || !self.lens.contains_key(&target) {
            return Vec::new();
        }
        let late = self.moves.range(id..).next().is_some();
        self.moves.insert(id, (target, parent, side));
        let changed: Vec<(EventId, (Anchor, Side))> = if late {
            let mut won = self.inserted.clone();
            for (t, p, s) in self.moves.values() {
                if won.contains_key(t) && !crate::replay::anchors_under(&won, *p, *t) {
                    won.insert(*t, (*p, *s));
                }
            }
            won.into_iter().filter(|(run, to)| self.parents[run] != *to).collect()
        } else if self.parents[&target] == (parent, side) || crate::replay::anchors_under(&self.parents, parent, target) {
            Vec::new()
        } else {
            vec![(target, (parent, side))]
        };
        match changed.as_slice() {
            [] => Vec::new(),
            &[(run, to)] => self.relocate(run, to),
            _ => {
                let before: Vec<char> = self.text().chars().collect();
                for (run, to) in changed {
                    self.unhang(run);
                    self.hang(run, to);
                }
                self.relayout();
                let after: Vec<char> = self.text().chars().collect();
                replacement(&before, &after).into_iter().collect()
            }
        }
    }

    /// One run's anchor changed: carry its subtree, one contiguous block of
    /// the document, from where it is to where it goes. Either end may lie
    /// outside this document (hidden, or another document): then the block is
    /// only lifted, or only set down, laid out fresh by replay's walk of the
    /// subtree.
    fn relocate(&mut self, run: EventId, to: (Anchor, Side)) -> Vec<Edit> {
        let mut edits = Vec::new();
        let shown = self.offset_of(Pos { event: run, offset: 0 }).is_some();
        // Lift first: with the block gone, the trees and `children` agree, and
        // the new anchor's subtree -- which may enclose the old place -- can
        // be walked without stepping into it.
        let lifted = shown.then(|| self.lift(run));
        self.unhang(run);
        if let Some((offset, frags)) = &lifted {
            let gone: usize = frags.iter().filter(|f| f.visible).map(|f| f.len() as usize).sum();
            if gone > 0 {
                edits.push(Edit { old: *offset..*offset + gone, text: String::new() });
            }
        }
        if self.placed(to.0, to.1) {
            let gap = self.gap_for(run, to.0, to.1);
            let frags = lifted.map(|(_, f)| f).unwrap_or_else(|| self.walk(Some(run)));
            let text: String = frags.iter().filter(|f| f.visible).map(Fragment::str).collect();
            let offset = self.splice(gap, frags);
            if !text.is_empty() {
                edits.push(Edit { old: offset..offset, text });
            }
        }
        self.hang(run, to);
        self.relocated += 1;
        edits
    }

    /// Take `run`'s subtree out of both trees: the character offset it began
    /// at, and its fragments in document order.
    fn lift(&mut self, run: EventId) -> (usize, Vec<Fragment>) {
        let root = Pos { event: run, offset: 0 };
        let first = self.cut(self.subtree_first(root), false).right.expect("the block starts a fragment");
        let last = self.cut(self.subtree_last(root), true).left.expect("the block ends a fragment");
        let (before, rest) = self.order.split(&first, Bias::Left);
        let (block, after) = rest.split(&last, Bias::Right);
        let offset = before.summary().text.chars;
        self.order = before;
        self.order.append(after);
        let frags: Vec<Fragment> = block.iter().cloned().collect();
        for f in &frags {
            let (left, rest) = self.index.split(&Key(Some((f.run, f.start))), Bias::Right);
            let (_, rest) = rest.split(&Key(Some((f.run, f.start + f.len()))), Bias::Right);
            self.index = left;
            self.index.append(rest);
        }
        (offset, frags)
    }

    /// Set `frags` down in `gap`, in order, keyed between its edges, and index
    /// them. Returns the character offset they begin at.
    fn splice(&mut self, gap: Gap, mut frags: Vec<Fragment>) -> usize {
        let mut lo = gap.left.unwrap_or_else(Locator::min);
        let hi = gap.right.clone().unwrap_or_else(Locator::max);
        for f in &mut frags {
            f.loc = Locator::between(&lo, &hi);
            lo = f.loc.clone();
        }
        let (mut left, right) = match &gap.right {
            Some(loc) => self.order.split(loc, Bias::Left),
            None => (self.order.clone(), SumTree::new()),
        };
        let offset = left.summary().text.chars;
        for f in &frags {
            let piece = Piece { run: f.run, start: f.start, len: f.len(), loc: f.loc.clone() };
            let (mut before, rest) = self.index.split(&Key(Some((piece.run, piece.start))), Bias::Right);
            before.push(piece);
            before.append(rest);
            self.index = before;
        }
        left.append(SumTree::from_items(frags));
        left.append(right);
        self.order = left;
        offset
    }

    /// Take `run` out of its anchor's children.
    fn unhang(&mut self, run: EventId) {
        let key = self.parents[&run];
        self.children.get_mut(&key).expect("a run is among its anchor's children").retain(|k| *k != run);
    }

    /// Anchor `run` at `to`: its parent, and its sorted place among the siblings.
    fn hang(&mut self, run: EventId, to: (Anchor, Side)) {
        self.parents.insert(run, to);
        let kids = self.children.entry(to).or_default();
        let at = kids.partition_point(|k| *k < run);
        kids.insert(at, run);
    }

    /// Is something anchored here laid out in this document? Replay's walk
    /// reaches the right of the document's start, and every character of a
    /// laid-out run; not the start's left, nor past a run's end, nor into a
    /// run that is hidden or in another document.
    fn placed(&self, parent: Anchor, side: Side) -> bool {
        match parent {
            Anchor::DocStart(n) => n == self.node && side == Side::Right,
            Anchor::At(p) => self.offset_of(p).is_some(),
        }
    }

    /// Lay the document out afresh from the tree, with replay's walk,
    /// keeping every character's visibility.
    fn relayout(&mut self) {
        let frags = self.walk(None);
        self.set_fragments(frags);
        self.relaid += 1;
    }

    /// Replay's walk over this weave's tree -- the whole document, or one
    /// run's subtree -- as unkeyed fragments.
    fn walk(&self, from: Option<EventId>) -> Vec<Fragment> {
        let alive = |p: Pos| !self.dead.get(&p.event).is_some_and(|d| d.iter().any(|(a, b)| (*a..*b).contains(&p.offset)));
        let len = |e: EventId| self.lens.get(&e).copied();
        let walk = match from {
            None => crate::replay::walk(self.node, &self.children, &len, &alive),
            Some(run) => crate::replay::walk_run(run, &self.children, &len, &alive),
        };
        lay_out(walk.into_iter(), &|e| self.texts.get(&e).cloned())
    }

    fn insert(&mut self, id: EventId, parent: Anchor, side: Side, text: &str) -> Option<Edit> {
        if self.lens.contains_key(&id) || text.is_empty() {
            return None;
        }
        // Every run is known, wherever it is; only those this document's walk
        // reaches are laid out. A move may bring the others, or what they hang
        // under, into view later.
        let gap = self.placed(parent, side).then(|| self.gap_for(id, parent, side));
        self.hang(id, (parent, side));
        self.inserted.insert(id, (parent, side));
        let len = text.chars().count() as u32;
        self.lens.insert(id, len);
        let run: Arc<str> = Arc::from(text);
        self.texts.insert(id, Arc::clone(&run));
        let gap = gap?;

        // Fill the gap with fragments of at most MAX_FRAGMENT.
        let mut cuts: Vec<u32> = text.char_indices().map(|(b, _)| b as u32).step_by(MAX_FRAGMENT as usize).collect();
        cuts.push(text.len() as u32);
        let mut frags = Vec::new();
        for (i, w) in cuts.windows(2).enumerate() {
            let start = i as u32 * MAX_FRAGMENT;
            frags.push(Fragment { loc: Locator::min(), run: id, start, text: Arc::clone(&run), bytes: w[0]..w[1], visible: true });
        }
        let offset = self.splice(gap, frags);
        Some(Edit { old: offset..offset, text: text.to_owned() })
    }

    /// Where run `id` goes, anchored at `(parent, side)`: just before the
    /// subtree of its next-larger sibling, or, being the largest of its group,
    /// just after what the group follows. Computed before `id` joins
    /// `children`. Follows replay's `Walk::run`: at `(r,k)`, left children,
    /// the character, right children smaller than `r`, the rest of the run
    /// (the implicit child, keyed `r`), then right children larger than `r`.
    /// An insert is younger than its anchor (Lamport, enforced at sync as
    /// I14), so it joins the last group; a move can hang an old run under a
    /// young one, into the group before the rest of the run.
    fn gap_for(&mut self, id: EventId, parent: Anchor, side: Side) -> Gap {
        let explicit = self.children.get(&(parent, side)).cloned().unwrap_or_default();
        let next = explicit.iter().copied().find(|s| *s > id);
        let before_run = |w: &mut Self, s: EventId| w.cut(w.subtree_first(Pos { event: s, offset: 0 }), false);
        match (parent, side) {
            (Anchor::At(p), Side::Left) => match next {
                Some(s) => before_run(self, s),
                None => self.cut(p, false),
            },
            (Anchor::At(p), Side::Right) => {
                let rest = Pos { event: p.event, offset: p.offset + 1 };
                match next {
                    // An older sibling comes next: same group, before the rest.
                    Some(s) if s < p.event => before_run(self, s),
                    // An old run, largest of the group before the rest of the run.
                    _ if id < p.event && rest.offset < self.lens[&p.event] => self.cut(self.subtree_first(rest), false),
                    Some(s) => before_run(self, s),
                    None => self.cut(self.subtree_last(p), true),
                }
            }
            (Anchor::DocStart(_), _) => match (next, explicit.last()) {
                (Some(s), _) => before_run(self, s),
                (None, Some(last)) => self.cut(self.subtree_last(Pos { event: *last, offset: 0 }), true),
                (None, None) => Gap { left: None, right: None },
            },
        }
    }

    /// The first character read in the subtree of `(r,k)`: down its smallest
    /// left children. O(depth).
    fn subtree_first(&self, mut p: Pos) -> Pos {
        while let Some(s) = self.children.get(&(Anchor::At(p), Side::Left)).and_then(|v| v.first()) {
            p = Pos { event: *s, offset: 0 };
        }
        p
    }

    /// The last character read in the subtree of `(r,k)`: the first `k' >= k`
    /// with right children read after the rest of the run -- any, at the run's
    /// last character; before it, those larger than `r` (a move can hang
    /// smaller ones there, and those are read *before* the rest) -- then down
    /// the largest of them. The first such `k'` wins because replay emits
    /// those deferred groups innermost first. Scans the run's anchor points:
    /// O(anchors in the runs on the path), not O(depth) (TECHDEBT).
    fn subtree_last(&self, mut p: Pos) -> Pos {
        loop {
            let n = self.lens[&p.event];
            let lo = (Anchor::At(p), Side::Right);
            let hi = (Anchor::At(Pos { event: p.event, offset: n - 1 }), Side::Right);
            // The range also holds each character's *left* children -- Left
            // sorts before Right -- and those are read before it, not after.
            let next = self.children.range(lo..=hi).find_map(|((a, side), kids)| {
                let (Anchor::At(q), Side::Right) = (a, side) else { return None };
                let last = *kids.last()?;
                (q.offset + 1 == n || last > q.event).then_some(last)
            });
            match next {
                Some(s) => p = Pos { event: s, offset: 0 },
                None => return Pos { event: p.event, offset: n - 1 },
            }
        }
    }

    /// Make a fragment boundary right before (or after) character `p`, and
    /// name the fragments on either side of it.
    fn cut(&mut self, p: Pos, after: bool) -> Gap {
        let piece = self.index.find(&Key(Some((p.event, p.offset))), Bias::Right).expect("p is in the weave").item.clone();
        let at = p.offset - piece.start + u32::from(after);
        let (upto, rest) = self.order.split(&piece.loc, Bias::Right);
        let next = rest.first().map(|f| f.loc.clone());
        if at == piece.len {
            return Gap { left: Some(piece.loc), right: next };
        }
        if at == 0 {
            let before = self.order.split(&piece.loc, Bias::Left).0;
            return Gap { left: before.last().map(|f| f.loc.clone()), right: Some(piece.loc) };
        }
        // Mid-fragment: split it, in both trees.
        let hi = next.clone().unwrap_or_else(Locator::max);
        let right_loc = Locator::between(&piece.loc, &hi);
        let (mut left, frag) = upto.split(&piece.loc, Bias::Left);
        let frag = frag.first().expect("the piece's fragment").clone();
        let (a, b) = frag.split(at, right_loc.clone());
        left.push(a);
        left.push(b);
        left.append(rest);
        self.order = left;
        let (mut left, rest) = self.index.split(&Key(Some((p.event, piece.start))), Bias::Right);
        let (_, rest) = rest.split(&Key(Some((p.event, piece.start + piece.len))), Bias::Right);
        left.push(Piece { len: at, ..piece.clone() });
        left.push(Piece { start: piece.start + at, len: piece.len - at, loc: right_loc.clone(), ..piece.clone() });
        left.append(rest);
        self.index = left;
        Gap { left: Some(piece.loc), right: Some(right_loc) }
    }

    fn delete(&mut self, target: EventId, range: (u32, u32)) -> Vec<Edit> {
        let Some(n) = self.lens.get(&target).copied() else { return Vec::new() };
        let range = crate::op::clamp(range, n);
        if range.is_empty() {
            return Vec::new();
        }
        self.dead.entry(target).or_default().push((range.start, range.end));
        if self.offset_of(Pos { event: target, offset: 0 }).is_none() {
            return Vec::new(); // hidden: its state is updated, nothing shows
        }
        let mut edits = Vec::new();
        let mut at = range.start;
        while at < range.end {
            // Isolate the fragment that starts at `at` and ends by the range's
            // end. The left half of a split keeps its Locator, so `loc` stays
            // the fragment's name through the second cut.
            let loc = self.cut(Pos { event: target, offset: at }, false).right.expect("a fragment starts at `at`");
            let end = self.fragment(&loc).start + self.fragment(&loc).len();
            if range.end < end {
                self.cut(Pos { event: target, offset: range.end - 1 }, true);
            }
            let frag = self.fragment(&loc).clone();
            at = frag.start + frag.len();
            if !frag.visible {
                continue;
            }
            let (mut before, rest) = self.order.split(&loc, Bias::Left);
            let (_, after) = rest.split(&loc, Bias::Right);
            let offset = before.summary().text.chars;
            edits.push(Edit { old: offset..offset + frag.len() as usize, text: String::new() });
            before.push(Fragment { visible: false, ..frag });
            before.append(after);
            self.order = before;
        }
        edits
    }

    fn fragment(&self, loc: &Locator) -> &Fragment {
        self.order.find(loc, Bias::Left).expect("a live locator").item
    }

    // --- reading --------------------------------------------------------

    /// The metrics of the visible text before `offset`: one seek, every
    /// coordinate system read off it. What the structure layer converts an
    /// [`Edit`] with ([`crate::syntax::Syntax::follow`]).
    #[allow(unused_variables)] // skeleton
    pub fn metrics_at(&self, offset: Chars) -> Metrics {
        todo!("find by Chars, Bias::Left; `before` plus the part of the fragment up to offset")
    }

    /// The visible text from byte `at` to the end of its fragment: the
    /// parser's read callback. O(log n) per call, so a parse of a document
    /// costs its fragments, not its bytes squared.
    #[allow(unused_variables)] // skeleton
    pub fn chunk_at(&self, at: usize) -> &str {
        todo!("a Bytes dimension over FragmentSummary; slice the fragment's str")
    }

    /// Every fragment, tombstones included, in document order. For
    /// inspection and tests; editing goes through ops.
    pub fn fragments(&self) -> impl Iterator<Item = &Fragment> {
        self.order.iter()
    }

    /// The visible text.
    pub fn text(&self) -> String {
        self.order.iter().filter(|f| f.visible).map(Fragment::str).collect()
    }

    /// The visible text's size, in every unit.
    pub fn metrics(&self) -> Metrics {
        self.order.summary().text
    }

    // --- converting -----------------------------------------------------

    /// Where a character is, in visible characters before it, and whether it
    /// is itself visible. Resolves tombstones too: a deleted character sits
    /// where it would be. `None` if the weave has never seen it.
    pub fn offset_of(&self, p: Pos) -> Option<(Chars, bool)> {
        let piece = self.index.find(&Key(Some((p.event, p.offset))), Bias::Right)?.item;
        if piece.run != p.event || piece.start > p.offset {
            return None;
        }
        let hit = self.order.find(&piece.loc, Bias::Left)?;
        let inside = if hit.item.visible { (p.offset - piece.start) as usize } else { 0 };
        Some((Chars(hit.before.text.chars + inside), hit.item.visible))
    }

    /// The visible character just after `offset` (`Bias::Right`) or just
    /// before it (`Bias::Left`). `None` past either end.
    pub fn pos_at(&self, offset: Chars, bias: Bias) -> Option<Pos> {
        let k = match bias {
            Bias::Right => offset.0,
            Bias::Left => offset.0.checked_sub(1)?,
        };
        // `Right` skips tombstones: they end where they start, never past `k`.
        let hit = self.order.find(&Chars(k), Bias::Right)?;
        let inside = (k - hit.before.text.chars) as u32;
        Some(Pos { event: hit.item.run, offset: hit.item.start + inside })
    }

    /// An LSP position as a character offset. A column past the end of its
    /// line clamps to the line's end, a line past the end to the end of the
    /// text, as LSP specifies. A point inside a `\r\n` cannot be named: the
    /// first line-start after the `\r` is past the `\n`.
    pub fn offset_of_point(&self, p: PointUtf16) -> Chars {
        let target = PointSeek::at(p);
        let Some(hit) = self.order.find(&target, Bias::Right) else { return Chars(self.metrics().chars) };
        let mut m = hit.before.text;
        let mut at = m.chars;
        for c in hit.item.str().chars() {
            let here = PointSeek(m);
            let mid_crlf = c == '\n' && m.last_cr;
            if !mid_crlf && (here >= target || (m.lines == p.line && matches!(c, '\r' | '\n'))) {
                return Chars(at);
            }
            m.add(&Metrics::of(c.encode_utf8(&mut [0; 4])));
            at += 1;
        }
        Chars(at)
    }

    /// The LSP position of a character offset. Past the end, the end.
    pub fn point_of_offset(&self, offset: Chars) -> PointUtf16 {
        let m = match self.order.find(&offset, Bias::Right) {
            None => self.metrics(),
            Some(hit) => {
                let mut m = hit.before.text;
                let n = offset.0 - hit.before.text.chars;
                let s = hit.item.str();
                let end = s.char_indices().nth(n).map_or(s.len(), |(b, _)| b);
                m.add(&Metrics::of(&s[..end]));
                m
            }
        };
        PointUtf16 { line: m.lines, col: m.last_utf16 }
    }

    // --- producing ops (the live capture path) -------------------------

    /// The op for typing `text` at visible `offset`: the anchor and side
    /// [`crate::capture::between`] picks for the visible characters on either
    /// side. Pure: the caller appends it to the log, then `apply`s it.
    pub fn insert_op(&self, offset: Chars, text: &str) -> Op {
        let a = self.pos_at(offset, Bias::Left).map_or(Anchor::DocStart(self.node), Anchor::At);
        let b = self.pos_at(offset, Bias::Right);
        let run_parent = |e: EventId| self.parents.get(&e).map(|(a, _)| *a);
        let (parent, side) = crate::capture::between(a, b, &run_parent);
        Op::Insert { parent, side, text: text.to_owned() }
    }

    /// The ops for deleting the visible characters in `range`: one `Delete`
    /// per stretch of one run, each range already exact.
    pub fn delete_ops(&self, range: Range<Chars>) -> Vec<Op> {
        let mut ops: Vec<Op> = Vec::new();
        for k in range.start.0..range.end.0 {
            let Some(p) = self.pos_at(Chars(k), Bias::Right) else { break };
            match ops.last_mut() {
                Some(Op::Delete { target, range }) if *target == p.event && range.1 == p.offset => range.1 += 1,
                _ => ops.push(Op::Delete { target: p.event, range: (p.offset, p.offset + 1) }),
            }
        }
        ops
    }
}

/// Group a walk into maximal fragments of at most [`MAX_FRAGMENT`]:
/// consecutive characters of one run, adjacent in it, with one visibility.
/// Locators are left for the caller to assign.
fn lay_out(walk: impl Iterator<Item = (Pos, bool)>, text: &dyn Fn(EventId) -> Option<Arc<str>>) -> Vec<Fragment> {
    // Per run: its text and the byte offset of every character, plus the end
    // -- so a fragment's byte range is two lookups.
    let mut runs: BTreeMap<EventId, (Arc<str>, Vec<u32>)> = BTreeMap::new();
    let mut frags: Vec<Fragment> = Vec::new();
    for (Pos { event: run, offset }, alive) in walk {
        if let std::collections::btree_map::Entry::Vacant(slot) = runs.entry(run) {
            let Some(t) = text(run) else { continue };
            let mut at: Vec<u32> = t.char_indices().map(|(b, _)| b as u32).collect();
            at.push(t.len() as u32);
            slot.insert((t, at));
        }
        let (t, at) = &runs[&run];
        let extends = frags
            .last()
            .is_some_and(|f| f.run == run && f.visible == alive && f.start + f.len() == offset && f.len() < MAX_FRAGMENT);
        if extends {
            frags.last_mut().expect("checked above").bytes.end = at[offset as usize + 1];
        } else {
            let bytes = at[offset as usize]..at[offset as usize + 1];
            frags.push(Fragment { loc: Locator::min(), run, start: offset, text: Arc::clone(t), bytes, visible: alive });
        }
    }
    frags
}

/// One edit turning `before` into `after`: the changed middle, with the common
/// prefix and suffix trimmed. `None` if they are equal.
fn replacement(before: &[char], after: &[char]) -> Option<Edit> {
    let pre = before.iter().zip(after).take_while(|(a, b)| a == b).count();
    let room = before.len().min(after.len()) - pre;
    let suf = before.iter().rev().zip(after.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
    if pre == before.len() && pre == after.len() {
        return None;
    }
    let text = after[pre..after.len() - suf].iter().collect();
    Some(Edit { old: pre..before.len() - suf, text })
}
