//! Xanadu on v0's atoms: a spike that changes nothing in v0.
//!
//! Xanadu wanted three things the web never got: an address for a *piece*
//! of a document that survives every edit, links that run both ways, and
//! transclusion -- a document showing another's text by reference, live,
//! not by copy. The W3C Web Annotation stack (TextQuoteSelector,
//! TextPositionSelector, RangeSelector, fuzzy re-anchoring) exists because
//! the web has no such address: it has documents, and documents change
//! under the link.
//!
//! v0 has the address already. Every character is an atom with a permanent
//! identity, [`Pos`] `{ event, offset }`, and the weave converts identity to
//! a position in O(log n) whatever has happened since ([`Weave::offset_of`]).
//! So a span of atoms is an address, a citation is a span plus the frontier
//! it was made at, "who cites this" is a lookup, and a quote is a span
//! rendered where another document says. This crate is that, in the
//! smallest form that runs, with the tests in `tests/claims.rs` as the
//! claims. What it does *not* settle is in `README.md`.
//!
//! Nothing here is stored in v0's log: spans, citations and views are held
//! by the caller. The point of the spike is that they *could* be plain ops
//! (VISION: `Cite` as the first link-only op); the mechanics do not depend
//! on it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use v0::event::EventLog;
use v0::op::{EventId, NodeId, NodeKind, Op, Pos, ReplicaId};
use v0::replay;
use v0::sumtree::Bias;
use v0::weave::{Chars, Weave};

/// Every document of a log, live: one weave per file node. A span resolves
/// in whichever document places its atoms *now* -- a run moved to another
/// document takes its citations along.
pub struct Docs {
    pub log: EventLog,
    weaves: BTreeMap<NodeId, Weave>,
}

impl Docs {
    /// Open every file of the log.
    pub fn open(log: EventLog) -> Docs {
        let all: Vec<EventId> = log.events().keys().copied().collect();
        let weaves = replay::weaves(&all, &log);
        let files = log.events().values().filter_map(|e| match &e.op {
            Op::Create { node, kind: NodeKind::File, .. } => Some(*node),
            _ => None,
        });
        let weaves = files.map(|n| (n, Weave::from_walk(n, &weaves, &log))).collect();
        Docs { log, weaves }
    }

    /// Record an op and show it everywhere. Every weave learns every run;
    /// each lays out only what its own walk reaches.
    pub fn append(&mut self, replica: ReplicaId, seq: &mut u32, op: Op) -> EventId {
        let id = self.log.append(replica, seq, op.clone());
        self.apply(id, &op);
        id
    }

    /// A new, empty file: its Create in the log and a weave for it. The weave
    /// is opened from a replay so that it knows every run of the repository
    /// already -- a move can bring any of them here.
    pub fn create(&mut self, replica: ReplicaId, seq: &mut u32, name: &str) -> NodeId {
        let node = NodeId(EventId { seq: *seq, replica });
        self.append(replica, seq, Op::Create { node, parent: Op::ROOT, name: name.into(), kind: NodeKind::File });
        let all: Vec<EventId> = self.log.events().keys().copied().collect();
        let weaves = replay::weaves(&all, &self.log);
        self.weaves.insert(node, Weave::from_walk(node, &weaves, &self.log));
        node
    }

    /// Take in an event from elsewhere (sync), in causal order.
    pub fn apply(&mut self, id: EventId, op: &Op) {
        for w in self.weaves.values_mut() {
            w.apply(id, op);
        }
    }

    pub fn weave(&self, node: NodeId) -> &Weave {
        &self.weaves[&node]
    }

    pub fn text(&self, node: NodeId) -> String {
        self.weaves[&node].text()
    }

    /// Where an atom is shown now: its document, visible offset, and whether
    /// it is alive. `None` while it is hidden (moved into hiding, or into a
    /// document not open here).
    pub fn locate(&self, p: Pos) -> Option<(NodeId, Chars, bool)> {
        self.weaves.iter().find_map(|(n, w)| w.offset_of(p).map(|(c, alive)| (*n, c, alive)))
    }

    /// The event that deleted an atom, if any: provenance for a hole. A scan
    /// of the log; a real store would index it.
    pub fn deleted_by(&self, p: Pos) -> Option<EventId> {
        self.log.events().values().find_map(|e| match e.op {
            Op::Delete { target, range } if target == p.event && (range.0..range.1).contains(&p.offset) => Some(e.id),
            _ => None,
        })
    }
}

// --- addresses --------------------------------------------------------------

/// Xanadu's address: a stretch of text named by the identities of its first
/// and last character. Nothing in it is a coordinate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub first: Pos,
    pub last: Pos,
}

/// A dead atom inside a span, and who killed it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hole {
    pub at: Pos,
    pub by: Option<EventId>,
}

/// What a span shows now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Resolved {
    /// The span's atoms sit in `node` at `range`; `text` is what is alive
    /// there, and `holes` the dead atoms of the span, in order.
    Text { node: NodeId, range: Range<Chars>, text: String, holes: Vec<Hole> },
    /// The ends are no longer a stretch: in different documents, one of them
    /// hidden, or moved past each other. Open question 2 (README).
    Torn(&'static str),
}

impl Span {
    /// The address of what is at `range` in `node` right now.
    pub fn of(docs: &Docs, node: NodeId, range: Range<Chars>) -> Span {
        let w = docs.weave(node);
        let first = w.pos_at(range.start, Bias::Right).expect("range starts inside the text");
        let last = w.pos_at(Chars(range.end.0 - 1), Bias::Right).expect("range ends inside the text");
        Span { first, last }
    }

    pub fn resolve(&self, docs: &Docs) -> Resolved {
        let (Some((n1, c1, a1)), Some((n2, c2, a2))) = (docs.locate(self.first), docs.locate(self.last)) else {
            return Resolved::Torn("an end is hidden");
        };
        if n1 != n2 {
            return Resolved::Torn("the ends are in different documents");
        }
        // A dead atom's offset is where it would be: the next live one's.
        let end = Chars(c2.0 + usize::from(a2));
        if end.0 < c1.0 + usize::from(a1) {
            return Resolved::Torn("the ends have moved past each other");
        }
        let w = docs.weave(n1);
        let text: String = w.text().chars().skip(c1.0).take(end.0 - c1.0).collect();
        // The dead atoms between the ends, by walking the fragments in order.
        let (mut inside, mut holes) = (false, Vec::new());
        for f in w.fragments() {
            for k in 0..f.str().chars().count() as u32 {
                let p = Pos { event: f.run, offset: f.start + k };
                inside |= p == self.first;
                if inside && !f.visible {
                    holes.push(Hole { at: p, by: docs.deleted_by(p) });
                }
                if p == self.last {
                    return Resolved::Text { node: n1, range: c1..end, text, holes };
                }
            }
        }
        Resolved::Torn("the last atom was not reached")
    }

    /// The span's text now, or the reason there is none.
    pub fn text(&self, docs: &Docs) -> Result<String, &'static str> {
        match self.resolve(docs) {
            Resolved::Text { text, .. } => Ok(text),
            Resolved::Torn(why) => Err(why),
        }
    }

    /// Does this span cover atom `p`, in document order, now?
    pub fn covers(&self, docs: &Docs, p: Pos) -> bool {
        match (self.resolve(docs), docs.locate(p)) {
            (Resolved::Text { node, range, .. }, Some((n, c, _))) => n == node && range.contains(&c),
            _ => false,
        }
    }
}

// --- citations --------------------------------------------------------------

/// A span plus the frontier it was made at: what was meant *then*. The
/// frontier is the log's heads, the antichain normal form of a world.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cite {
    pub span: Span,
    pub at: Vec<EventId>,
}

impl Cite {
    pub fn now(docs: &Docs, span: Span) -> Cite {
        Cite { span, at: docs.log.frontier() }
    }

    /// What the span said at the frontier: the world of that time, replayed
    /// from scratch. That is the cost v1's `goto` removes; here it is paid.
    pub fn then(&self, docs: &Docs) -> String {
        let world: BTreeSet<EventId> = self.at.iter().flat_map(|h| docs.log.causal_closure(*h)).collect();
        let events: Vec<EventId> = world.into_iter().collect();
        let weaves = replay::weaves(&events, &docs.log);
        let mut out = String::new();
        for walk in weaves.walks.values() {
            let mut inside = false;
            for (atom, alive) in walk {
                inside |= atom.id == self.span.first;
                if inside && *alive {
                    out.push(atom.ch);
                }
                if atom.id == self.span.last {
                    return out;
                }
            }
        }
        out
    }

    /// `Some((then, now))` when the cited text has changed: the staleness
    /// of a comment, as a computation.
    pub fn stale(&self, docs: &Docs) -> Option<(String, String)> {
        let then = self.then(docs);
        let now = self.span.text(docs).unwrap_or_default();
        (then != now).then_some((then, now))
    }
}

/// Links run both ways: every citation whose span covers `p` now. Xanadu's
/// two-way links are a query over the same table, not a second structure.
pub fn cited_by<'a>(cites: &'a [Cite], docs: &Docs, p: Pos) -> Vec<&'a Cite> {
    cites.iter().filter(|c| c.span.covers(docs, p)).collect()
}

// --- transclusion -------------------------------------------------------------

/// The character a citing document holds where a quote goes: an atom of its
/// own whose content is elsewhere. Unicode's object replacement character.
pub const QUOTE: char = '\u{FFFC}';

/// A document showing spans of others in itself, by reference. Each quote
/// *is* an atom of the document -- a [`QUOTE`] character with an identity --
/// so it has a place, edges, and can be deleted like anything else; the
/// span it stands for is looked up by that identity.
pub struct View {
    pub into: NodeId,
    pub quotes: BTreeMap<Pos, Span>,
}

/// One stretch of a view's text and where it comes from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Segment {
    pub node: NodeId,
    pub range: Range<Chars>,
    pub quoted: bool,
}

impl View {
    pub fn new(into: NodeId) -> View {
        View { into, quotes: BTreeMap::new() }
    }

    /// Quote `span` at offset `at` of the document: type the quote atom
    /// there, and remember what it stands for.
    pub fn quote(&mut self, docs: &mut Docs, replica: ReplicaId, seq: &mut u32, at: Chars, span: Span) -> Pos {
        let op = docs.weave(self.into).insert_op(at, &QUOTE.to_string());
        let id = docs.append(replica, seq, op);
        let marker = Pos { event: id, offset: 0 };
        self.quotes.insert(marker, span);
        marker
    }

    /// The view as stretches of real documents, in reading order. A quote
    /// whose atom is deleted is gone; a torn one contributes nothing (a real
    /// renderer would show a hole with the reason).
    pub fn segments(&self, docs: &Docs) -> Vec<Segment> {
        let own = docs.weave(self.into);
        let len = Chars(own.text().chars().count());
        let mut marks: Vec<(Chars, &Span)> =
            self.quotes.iter().filter_map(|(m, span)| match own.offset_of(*m) {
                Some((c, true)) => Some((c, span)),
                _ => None,
            }).collect();
        marks.sort_by_key(|(c, _)| *c);
        let mut segs = Vec::new();
        let mut from = Chars(0);
        for (at, span) in marks {
            segs.push(Segment { node: self.into, range: from..at, quoted: false });
            if let Resolved::Text { node, range, .. } = span.resolve(docs) {
                segs.push(Segment { node, range, quoted: true });
            }
            from = Chars(at.0 + 1); // past the quote atom itself
        }
        segs.push(Segment { node: self.into, range: from..len, quoted: false });
        segs.into_iter().filter(|s| s.range.start < s.range.end).collect()
    }

    pub fn text(&self, docs: &Docs) -> String {
        self.segments(docs)
            .iter()
            .map(|s| docs.text(s.node).chars().skip(s.range.start.0).take(s.range.end.0 - s.range.start.0).collect::<String>())
            .collect()
    }

    /// Where an edit at a composed offset lands. Strictly inside a quote it
    /// lands in the quoted document, on the atoms themselves: that is what
    /// transclusion means, and what makes a citing document mutable through
    /// its quotes (open question 1). At a quote's edge it stays in the citing
    /// document, before or after the quote atom: typing next to a quote must
    /// not grow the source.
    pub fn locate(&self, docs: &Docs, offset: Chars) -> (NodeId, Chars) {
        let mut seen = 0;
        for s in self.segments(docs) {
            let n = s.range.end.0 - s.range.start.0;
            let hit = if s.quoted { offset.0 > seen && offset.0 < seen + n } else { offset.0 >= seen && offset.0 <= seen + n };
            if hit {
                return (s.node, Chars(s.range.start.0 + offset.0 - seen));
            }
            seen += n;
        }
        (self.into, Chars(docs.text(self.into).chars().count()))
    }

    /// Type `text` at a composed offset, as an editor on the view would.
    pub fn insert(&self, docs: &mut Docs, replica: ReplicaId, seq: &mut u32, offset: Chars, text: &str) -> EventId {
        let (node, at) = self.locate(docs, offset);
        let op = docs.weave(node).insert_op(at, text);
        docs.append(replica, seq, op)
    }
}
