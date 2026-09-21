//! Capture adapters: how observed editing becomes events.
//!
//! Everything above layer 0 is indifferent to which adapter ran — that is the
//! point of putting capture behind an interface. What differs is **fidelity**:
//!
//! | adapter | sees | cannot see |
//! |---|---|---|
//! | [`from_save`] (v0) | before/after text | which edit happened, and every move |
//! | live editor (v1) | each keystroke or edit command | — |
//!
//! A diff-based adapter reconstructs a plausible edit and bakes that
//! interpretation into the log forever (ADR-0003). It can never emit
//! [`Op::MoveLine`]: a move looks exactly like a delete plus an insert.

use crate::event::EventLog;
use crate::op::{Anchor, EventId, NodeId, Op, Pos, ReplicaId, Side};
use crate::replay::Atom;
use similar::{ChangeTag, DiffTag, TextDiff};
use unicode_segmentation::UnicodeSegmentation;
use std::collections::BTreeMap;

/// Fugue's placement rule, from Weidner & Kleppmann, *The Art of the Fugue*
/// (arXiv:2305.00583). To place a run between left neighbour `a` and right
/// neighbour `b`:
///
/// - if `a` is **not** an ancestor of `b`, become a **right child of `a`**;
/// - if `a` **is** an ancestor of `b`, become a **left child of `b`**.
///
/// Those two lines are the whole difference between two people's text staying
/// whole and being shuffled together. Reading order is the in-order traversal of
/// the tree, so a subtree — one person's run — is always contiguous.
pub fn between(a: Anchor, b: Option<Pos>, run_parent: &dyn Fn(EventId) -> Option<Anchor>) -> (Anchor, Side) {
    match b {
        Some(b) if is_ancestor(a, b, run_parent) => (Anchor::At(b), Side::Left),
        _ => (a, Side::Right),
    }
}

/// Is `a` on the path from `b` up to the root of its document?
///
/// Walks runs, not characters: within one run every earlier character is an
/// ancestor of every later one, so the question per run is a comparison, and
/// the walk is as deep as runs are nested rather than as long as the text.
fn is_ancestor(a: Anchor, b: Pos, run_parent: &dyn Fn(EventId) -> Option<Anchor>) -> bool {
    // The document root is an ancestor of everything in it. That is what makes
    // an insert at position 0 a *left* child of the first character, and why
    // typing upward stays contiguous.
    let Anchor::At(pa) = a else { return true };
    let mut cur = b;
    loop {
        if pa.event == cur.event && pa.offset < cur.offset {
            return true;
        }
        match run_parent(cur.event) {
            Some(Anchor::At(q)) if q == pa => return true,
            Some(Anchor::At(q)) => cur = q,
            _ => return false,
        }
    }
}

/// Ops explaining one save, plus which of them came from the same hunk.
///
/// The grouping is what the adapter *observed* and the graph cannot derive: a
/// replaced word is a delete and an insert that touch different characters.
pub struct Captured {
    pub ops: Vec<Op>,
    /// Indices into `ops`, one group per changed hunk.
    pub hunks: Vec<Vec<usize>>,
}

/// Degraded adapter: diff saved text against the materialised characters and
/// mint the ops that explain the difference.
///
/// Two levels, the standard trick: lines first, to find the hunks cheaply, then
/// characters *inside* each changed hunk. Diffing a whole file character by
/// character is quadratic in its length and hangs on the first large rewrite.
///
/// **Contract.** Append the ops in order starting at `base_seq`: a run that
/// follows another run of this batch anchors to an id predicted as
/// `EventId { seq: base_seq + i, replica }`.
pub fn from_save(
    before: &[Atom],
    node: NodeId,
    after: &str,
    replica: ReplicaId,
    base_seq: u32,
    log: &EventLog,
) -> Captured {
    let old: String = before.iter().map(|a| a.ch).collect();
    let old_starts = line_starts(&old);
    let new_chars: Vec<char> = after.chars().collect();
    let new_starts = line_starts(after);

    let mut b = Builder {
        before,
        node,
        replica,
        base_seq,
        log,
        ops: Vec::new(),
        local: BTreeMap::new(),
        at: Anchor::DocStart(node),
        idx: 0,
        pending_delete: None,
        pending_insert: String::new(),
    };
    let mut hunks = Vec::new();

    for op in TextDiff::from_lines(old.as_str(), after).ops() {
        let (o, n) = (op.old_range(), op.new_range());
        let (oc, nc) = (old_starts[o.start]..old_starts[o.end], new_starts[n.start]..new_starts[n.end]);
        if op.tag() == DiffTag::Equal {
            if !oc.is_empty() {
                b.at = Anchor::At(before[oc.end - 1].id);
            }
            b.idx = oc.end;
            continue;
        }
        let first = b.ops.len();
        let old_seg: String = old.chars().skip(oc.start).take(oc.len()).collect();
        let new_seg: String = new_chars[nc].iter().collect();
        let old_tokens = tokens_with_identity(&old_seg, &before[oc.clone()]);
        let new_tokens: Vec<&str> = new_seg.split_word_bounds().collect();
        // Words, not characters, inside a changed hunk. Identity stays per
        // character; only the *diff* is coarser. A character diff reuses letters
        // opportunistically -- "hello world" to "goedendag" keeps an `e` and a
        // `d` -- and two concurrent rewrites of one word then each delete the
        // letters the other kept. Measured: they merged into "goedniag". With
        // words, a replaced word goes out whole and comes in whole, so the same
        // situation is a clean conflict instead of letter soup.
        for c in TextDiff::from_slices(&old_tokens, &new_tokens).iter_all_changes() {
            let n = c.value().chars().count();
            match c.tag() {
                ChangeTag::Equal => {
                    b.flush();
                    b.idx += n;
                    b.at = Anchor::At(before[b.idx - 1].id);
                }
                ChangeTag::Delete => {
                    b.flush_insert();
                    for _ in 0..n {
                        b.delete(before[b.idx].id);
                        b.at = Anchor::At(before[b.idx].id);
                        b.idx += 1;
                    }
                }
                ChangeTag::Insert => {
                    b.flush_delete();
                    b.pending_insert.push_str(c.value());
                }
            }
        }
        b.flush();
        hunks.push((first..b.ops.len()).collect());
    }
    Captured { ops: b.ops, hunks }
}

/// Split old text into words that never span two runs.
///
/// The old side carries identity and the new side does not. After a merge, two
/// people's words can sit flush against each other -- "goedendag" and "hoi"
/// become the single word "goedendaghoi" -- and a word diff would then replace
/// both as one, deleting characters of the side the resolver meant to keep.
/// Cutting tokens at every change of authoring event keeps each person's
/// characters matchable on their own.
fn tokens_with_identity<'s>(text: &'s str, atoms: &[Atom]) -> Vec<&'s str> {
    let mut out = Vec::new();
    let mut ci = 0; // character index into `atoms`
    for word in text.split_word_bounds() {
        let mut start = 0; // byte offset within `word`
        let mut prev: Option<EventId> = None;
        for (bi, _) in word.char_indices() {
            let ev = atoms[ci].id.event;
            if prev.is_some_and(|p| p != ev) {
                out.push(&word[start..bi]);
                start = bi;
            }
            prev = Some(ev);
            ci += 1;
        }
        out.push(&word[start..]);
    }
    out
}

/// Character offsets at which each line starts, plus one past the end — the
/// same line split `similar` uses (terminators included).
/// The char offset where each line starts, plus the end -- cutting lines
/// exactly where `similar`'s line diff cuts them: after `\n`, and after a
/// `\r` not followed by `\n`. Cutting only at `\n` disagreed on old Mac
/// line endings, and the diff's line numbers then indexed past this table.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    let mut chars = text.chars().peekable();
    let mut at = 0;
    while let Some(c) = chars.next() {
        at += 1;
        let breaks = c == '\n' || (c == '\r' && chars.peek() != Some(&'\n'));
        if breaks {
            starts.push(at);
        }
    }
    if starts.last() != Some(&at) {
        starts.push(at);
    }
    starts
}

/// Accumulates one save's ops, coalescing consecutive characters into runs and
/// consecutive deletions of one run into ranges.
struct Builder<'a> {
    before: &'a [Atom],
    node: NodeId,
    replica: ReplicaId,
    base_seq: u32,
    log: &'a EventLog,
    ops: Vec<Op>,
    /// Parents of runs minted in *this* batch: not in the log yet, but already
    /// ancestors of what follows them.
    local: BTreeMap<EventId, Anchor>,
    /// The left neighbour: the last character walked past, alive or deleted.
    at: Anchor,
    /// Next character of `before`.
    idx: usize,
    pending_delete: Option<(EventId, u32, u32)>,
    pending_insert: String,
}

impl Builder<'_> {
    fn delete(&mut self, p: Pos) {
        match &mut self.pending_delete {
            Some((e, _, end)) if *e == p.event && *end == p.offset => *end += 1,
            _ => {
                self.flush_delete();
                self.pending_delete = Some((p.event, p.offset, p.offset + 1));
            }
        }
    }

    fn flush_delete(&mut self) {
        if let Some((target, start, end)) = self.pending_delete.take() {
            self.ops.push(Op::Delete { target, range: (start, end) });
        }
    }

    fn flush_insert(&mut self) {
        if self.pending_insert.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending_insert);
        let right = self.before.get(self.idx).map(|a| a.id);
        let (local, log) = (&self.local, self.log);
        let run_parent = |e: EventId| -> Option<Anchor> {
            local.get(&e).copied().or_else(|| match log.events().get(&e).map(|ev| &ev.op) {
                Some(Op::Insert { parent, .. } | Op::MoveRun { parent, .. }) => Some(*parent),
                _ => None,
            })
        };
        let (parent, side) = between(self.at, right, &run_parent);
        let mine = EventId { seq: self.base_seq + self.ops.len() as u32, replica: self.replica };
        let len = text.chars().count() as u32;
        self.local.insert(mine, parent);
        self.ops.push(Op::Insert { parent, side, text });
        // What follows this run sits to its right: a pasted block is one
        // subtree, contiguous under any merge.
        self.at = Anchor::At(Pos { event: mine, offset: len - 1 });
        let _ = self.node;
    }

    fn flush(&mut self) {
        self.flush_delete();
        self.flush_insert();
    }
}

/// The shape a live front-end calls: an already-known edit, no guessing.
/// Present so the interface is honest about what it is waiting for.
pub fn from_edit(_edit: Edit) -> Vec<Op> {
    todo!("v1: a front-end that reports edits instead of states")
}

/// What an editor actually knows it did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    Typed { after: Option<Pos>, node: NodeId, text: String },
    Deleted { target: EventId, range: (u32, u32) },
    Moved { target: EventId, parent: Anchor, side: Side },
    RenamedNode { node: NodeId, parent: NodeId, name: String },
}
