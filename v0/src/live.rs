//! A live session: one replica of one document, between an editor and its
//! peers. What an editor plugin talks to (`v0 live`, `editors/`).
//!
//! ```text
//! editor  <-- JSON lines -->  Session  <-- events -->  peers' Sessions
//! ```
//!
//! The session owns the weave, the log and the syntax tree. The editor owns
//! nothing but a buffer: it reports what was typed as *replace `del` units at
//! `at` with `text`*, and is told the same about what peers did. Pure: no
//! I/O here, so two sessions and two fake editors converge in a test.
//!
//! # The editor is always a little behind
//!
//! An edit is typed against the buffer as the editor shows it, and a peer's
//! edit may be on its way to that buffer at the same moment. Offsets from
//! the two would not agree. No transformation is needed to settle it,
//! because the weave is a CRDT: an op computed against *any* past state of
//! the document is valid in every later one. So the session keeps the state
//! before each message the editor has not confirmed ([`Session::unseen`]),
//! and every message from the editor says how far it had got (`seen`):
//!
//! - the editor applies a message only when the session had heard all it
//!   said (`acked` equals the count of edits it has sent), and drops it
//!   otherwise;
//! - an edit that arrives behind is read against the state the editor was
//!   in, and answered with one message that brings its buffer from there to
//!   now.
//!
//! An event typed behind also names only what the editor had shown as its
//! parents. That is what "had not seen the rename" means in
//! [`crate::syntax::reapply`], and a peer's rename that was still in the
//! pipe must not count as seen.

use crate::event::{Event, EventLog};
use crate::op::{EventId, NodeId, NodeKind, Op, ReplicaId};
use crate::sync;
use crate::syntax::{self, Grammar, Syntax};
use std::ops::Range;
use crate::weave::{Chars, Metrics, Weave};
use serde::{Deserialize, Serialize};

/// The largest text one insert event carries from an editor: a paste is cut
/// into events of at most this, well inside
/// [`crate::limits::EVENT_TEXT_BYTES_MAX`].
pub const INSERT_BYTES_MAX: usize = 1 << 16;

/// What the editor counts in: Emacs positions are characters, Neovim's
/// `on_bytes` are bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Chars,
    Bytes,
}

/// A line from the editor. `seen` is the number of the last
/// [`ToEditor::Apply`] it applied, zero for none.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FromEditor {
    /// Typed: `del` units at `at` became `text`.
    Edit { at: usize, del: usize, text: String, seen: u64 },
    /// Rename the identifier under `at` to `to`, as an intent.
    Rename { at: usize, to: String, seen: u64 },
    /// Applied message `seen`; nothing typed.
    Seen(u64),
    /// Go offline (`true`): keep what is typed here and what peers send
    /// apart until `false`. How two people get to be concurrent on purpose.
    Hold(bool),
}

/// One replacement in the editor's buffer, in its unit. The edits of a
/// message are sequential: each in the coordinates the one before left.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Patch {
    pub at: usize,
    pub del: usize,
    pub text: String,
}

/// A line to the editor.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToEditor {
    /// The session is up, and this is the document. A host's is empty: the
    /// editor sends its buffer as the first edit.
    Ready { role: &'static str, text: String },
    /// Message `n`: apply `edits`, unless you have sent more than `acked`
    /// edits and renames -- then drop it; what it meant comes again.
    Apply { n: u64, acked: u64, edits: Vec<Patch> },
    /// A request that could not be met; nothing changed.
    Error(String),
}

/// Where a rename's meaning comes from: which characters are the name the
/// user pointed at, and, after a merge, which occurrences of the old name
/// still mean that declaration.
///
/// v0's own answer is [`syntax::Scopes`]-based (one file, Rust only, by
/// syntax); a language server is the real one (`v1/`). Everything else a
/// rename needs -- the ops, their ids, who had seen what -- is decided
/// here, in [`Session`], the same whoever resolves.
pub trait Resolver: Send {
    /// The rename of the name at `at` in `doc`: the declaration's
    /// characters and every reference's, the declaration among them, in
    /// document order. `Err` when there is nothing there to rename.
    fn references(&mut self, doc: &Weave, syntax: &Syntax, at: Chars) -> Result<Refs, String>;

    /// For each of `candidates` -- occurrences of a name in `doc` -- whether
    /// it means the declaration whose name starts at `decl`. `doc` is the
    /// merged document with the rename's own effect taken out, so the
    /// declaration and the candidates read the same, old name.
    fn binds(&mut self, doc: &Weave, syntax: &Syntax, decl: Chars, candidates: &[Range<Chars>]) -> Vec<bool>;
}

/// A rename's reach in one document: what [`Resolver::references`] returns.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Refs {
    pub decl: Range<Chars>,
    pub all: Vec<Range<Chars>>,
}

/// What handling one input produced: events for the peers, lines for the
/// editor.
#[derive(Default, Debug)]
pub struct Out {
    pub send: Vec<Event>,
    pub reply: Vec<ToEditor>,
}

/// The document before a message the editor has not confirmed.
struct Unseen {
    n: u64,
    shown: Weave,
    /// The log's frontier then: what the editor had seen.
    frontier: Vec<EventId>,
}

pub struct Session {
    log: EventLog,
    replica: ReplicaId,
    grammar: Grammar,
    unit: Unit,
    weave: Weave,
    syntax: Syntax,
    unseen: Vec<Unseen>,
    /// Messages sent to the editor, and edits and renames heard from it.
    sent: u64,
    heard: u64,
    /// Every rename intent in the log: each is asked, after a merge, whether
    /// a reference it had not seen has turned up.
    renames: Vec<EventId>,
    resolver: Box<dyn Resolver>,
    /// Offline: what peers sent, and what is owed to them.
    held: Option<(Vec<Event>, Vec<Event>)>,
}

impl Session {
    /// Start a document: this replica mints its node.
    pub fn host(replica: ReplicaId, grammar: Grammar, unit: Unit, name: &str) -> Session {
        let mut log = EventLog::default();
        let node = NodeId(EventId { seq: 1, replica });
        log.append(replica, &mut 1, Op::Create { node, parent: Op::ROOT, name: name.to_owned(), kind: NodeKind::File });
        Session::over(log, node, replica, grammar, unit)
    }

    /// Join a document from everything a peer holds: its first file is the
    /// document.
    pub fn guest(replica: ReplicaId, grammar: Grammar, unit: Unit, events: Vec<Event>) -> Result<Session, String> {
        let node = events
            .iter()
            .find_map(|e| match e.op {
                Op::Create { node, kind: NodeKind::File, .. } => Some(node),
                _ => None,
            })
            .ok_or("the peer has no document")?;
        let mut s = Session::over(EventLog::default(), node, replica, grammar, unit);
        s.absorb(events);
        Ok(s)
    }

    fn over(log: EventLog, node: NodeId, replica: ReplicaId, grammar: Grammar, unit: Unit) -> Session {
        let weave = Weave::new(node);
        let syntax = Syntax::of(&weave, grammar);
        let (unseen, renames) = (Vec::new(), Vec::new());
        let resolver = Box::new(syntax::ScopeResolver);
        Session { log, replica, grammar, unit, weave, syntax, unseen, sent: 0, heard: 0, renames, resolver, held: None }
    }

    /// Resolve renames with `resolver` instead of v0's scope walk.
    pub fn with_resolver(mut self, resolver: Box<dyn Resolver>) -> Session {
        self.resolver = resolver;
        self
    }

    pub fn text(&self) -> String {
        self.weave.text()
    }

    pub fn tree(&self) -> String {
        self.syntax.tree().root_node().to_sexp()
    }

    /// Everything this replica holds, oldest first: what a joining peer is
    /// sent.
    /// Offline, that includes what peers sent meanwhile: this replica has
    /// not read it, but a host passes it on all the same.
    pub fn events(&self) -> Vec<Event> {
        let held = self.held.iter().flat_map(|(theirs, _)| theirs.iter().cloned());
        self.log.events().values().cloned().chain(held).collect()
    }

    /// One line from the editor.
    pub fn local(&mut self, msg: FromEditor) -> Out {
        match msg {
            FromEditor::Seen(seen) => {
                self.unseen.retain(|u| u.n > seen);
                Out::default()
            }
            FromEditor::Hold(true) => {
                self.held.get_or_insert_with(Default::default);
                Out::default()
            }
            FromEditor::Hold(false) => {
                let Some((theirs, ours)) = self.held.take() else { return Out::default() };
                let mut out = self.remote(theirs);
                out.send.splice(0..0, ours);
                out
            }
            FromEditor::Edit { at, del, text, seen } => self.typed(seen, |s, shown, _| {
                let (a, b) = (s.offset(shown, at)?, s.offset(shown, at + del)?);
                Ok((true, Typed::Replace(a..b, text)))
            }),
            FromEditor::Rename { at, to, seen } => {
                // The resolver is borrowed out for the call: `typed` holds the
                // session.
                let mut resolver = std::mem::replace(&mut self.resolver, Box::new(syntax::ScopeResolver));
                let out = self.typed(seen, |s, shown, syntax| {
                    let refs = resolver.references(shown, syntax, s.offset(shown, at)?)?;
                    let from: String = slice(shown, refs.decl.clone());
                    let decl = shown.pos_at(refs.decl.start, crate::sumtree::Bias::Right).ok_or("no declaration there")?;
                    let effect = refs.all.into_iter().rev().flat_map(|r| syntax::replace(shown, r, &to)).collect();
                    Ok((false, Typed::Rename(Op::Rename { decl, from, to: to.clone() }, effect)))
                });
                self.resolver = resolver;
                out
            }
        }
    }

    /// An edit or a rename from the editor: find the state it was made in,
    /// turn it into events there, and tell the editor whatever its buffer
    /// then lacks. `read` says what was asked, and whether the editor's
    /// buffer already shows it.
    fn typed(
        &mut self,
        seen: u64,
        read: impl FnOnce(&Session, &Weave, &Syntax) -> Result<(bool, Typed), String>,
    ) -> Out {
        self.heard += 1;
        self.unseen.retain(|u| u.n > seen);
        let behind = !self.unseen.is_empty();
        let (mut shown, mut frontier) = match self.unseen.first() {
            Some(u) => (u.shown.clone(), u.frontier.clone()),
            None => (self.weave.clone(), self.log.frontier()),
        };
        // Behind, the tree in hand is of a later document.
        let fresh = behind.then(|| Syntax::of(&shown, self.grammar));
        let (in_buffer, typed) = match read(self, &shown, fresh.as_ref().unwrap_or(&self.syntax)) {
            Ok(x) => x,
            // Refused: the editor is where it was, and so are the states
            // kept for it.
            Err(why) => return Out { send: Vec::new(), reply: vec![ToEditor::Error(why)] },
        };
        self.unseen.clear();
        let before = (shown.clone(), frontier.clone());
        let mut out = Out::default();
        let mut patches = Vec::new();
        let mut act = |s: &mut Session, shown: &mut Weave, op: Op, links: Vec<EventId>| {
            let id = EventId { seq: s.log.lamport_next(), replica: s.replica };
            let e = Event { id, parents: std::mem::replace(&mut frontier, vec![id]), op, links };
            s.log.insert(e.clone());
            shown.apply(id, &e.op);
            patches.extend(s.take(&e));
            out.send.push(e);
        };
        match typed {
            // The effect links to its intent: that is how a merge finds the
            // document as it read before the rename ([`Session::reapply`]).
            Typed::Rename(intent, effect) => {
                let rename = EventId { seq: self.log.lamport_next(), replica: self.replica };
                act(self, &mut shown, intent, Vec::new());
                effect.into_iter().for_each(|op| act(self, &mut shown, op, vec![rename]));
            }
            Typed::Replace(range, text) => {
                for op in shown.delete_ops(range.clone()) {
                    act(self, &mut shown, op, Vec::new());
                }
                let mut at = range.start;
                for piece in pieces(&text) {
                    let op = shown.insert_op(at, piece);
                    act(self, &mut shown, op, Vec::new());
                    at.0 += piece.chars().count();
                }
            }
        }
        // Typed behind, it may be a use of a name that a rename the editor
        // had not shown has since changed.
        if behind {
            let (_, derived) = self.reapply();
            out.send.extend(derived);
        }
        // What the editor shows now, against the document.
        let (shown, frontier) = if in_buffer { (shown, frontier) } else { before };
        let edits = match (behind, in_buffer) {
            (false, true) => Vec::new(),
            (false, false) => patches,
            (true, _) => self.diff(&shown.text(), &self.weave.text()).into_iter().collect(),
        };
        // Behind, a message goes out even when it has nothing to change:
        // until the editor confirms one, what it has seen is `frontier`,
        // not the log's.
        if behind || !edits.is_empty() {
            out.reply.push(self.message(shown, frontier, edits));
        }
        if let Some((_, ours)) = &mut self.held {
            ours.append(&mut out.send);
        }
        out
    }

    /// Events from a peer: take what is new, let every rename reach the
    /// references it had not seen, and tell the editor.
    pub fn remote(&mut self, events: Vec<Event>) -> Out {
        if let Some((theirs, _)) = &mut self.held {
            theirs.extend(events);
            return Out::default();
        }
        let before = (self.weave.clone(), self.log.frontier());
        let mut edits = self.absorb(events);
        let (more, derived) = self.reapply();
        edits.extend(more);
        let mut out = Out { send: derived, reply: Vec::new() };
        if !edits.is_empty() {
            out.reply.push(self.message(before.0, before.1, edits));
        }
        out
    }

    /// Let every rename reach the references it had not seen: what that
    /// did here, and the events to send.
    ///
    /// Two replicas that derive from the same document mint the same
    /// events, so sending them is usually a repeat. But they derive when
    /// they merge, and by then one may hold more than the other: a `let
    /// foo` its peer has not seen yet claims the call, and it derives
    /// nothing. Sent, the effect is in every log whoever saw it first.
    fn reapply(&mut self) -> (Vec<Patch>, Vec<Event>) {
        let (mut edits, mut sent) = (Vec::new(), Vec::new());
        for rename in self.renames.clone() {
            let mut derived = self.derive(rename);
            derived.retain(|e| !self.log.events().contains_key(&e.id));
            edits.extend(self.absorb(derived.clone()));
            sent.extend(derived);
        }
        (edits, sent)
    }

    /// The effect of `rename` on occurrences of the old name it had not
    /// seen and that still mean its declaration, as derived events.
    ///
    /// Generic over grammars and resolvers. The question "does this `foo`
    /// mean that declaration" is asked of the document as it would read had
    /// the rename never happened: every event except the rename's effect
    /// (the events linking to it). There the declaration and the stale use
    /// read the same name, and any resolver can answer. A candidate is a
    /// named leaf of the tree that reads the old name and was typed, every
    /// character of it, by someone who had not seen the rename.
    fn derive(&mut self, rename: EventId) -> Vec<Event> {
        let Some(Op::Rename { decl, from, to }) = self.log.events().get(&rename).map(|e| e.op.clone()) else {
            return Vec::new();
        };
        // The declaration must still read the new name here.
        let Some((end, _)) = self.weave.offset_of(decl) else { return Vec::new() };
        let len = to.chars().count();
        if end.0 < len || slice(&self.weave, Chars(end.0 - len)..end) != to {
            return Vec::new();
        }

        let mut before = Weave::new(self.weave.node);
        for e in self.log.events().values().filter(|e| !e.links.contains(&rename)) {
            before.apply(e.id, &e.op);
        }
        let Some((at, true)) = before.offset_of(decl) else { return Vec::new() };
        let tree = Syntax::of(&before, self.grammar);
        let decl_range = at..Chars(at.0 + from.chars().count());
        let text = before.text();
        let unseen = |range: &Range<Chars>| {
            (range.start.0..range.end.0)
                .filter_map(|k| before.pos_at(Chars(k), crate::sumtree::Bias::Right))
                .all(|p| !self.log.causal_closure(p.event).contains(&rename))
        };
        let mut candidates = Vec::new();
        let mut cursor = tree.tree().walk();
        'walk: loop {
            let n = cursor.node();
            if n.is_named() && n.child_count() == 0 && text.get(n.byte_range()) == Some(from.as_str()) {
                let r = before.chars_at_byte(n.start_byte())..before.chars_at_byte(n.end_byte());
                if r != decl_range && unseen(&r) {
                    candidates.push(r);
                }
            }
            if cursor.goto_first_child() {
                continue;
            }
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    break 'walk;
                }
            }
        }
        if candidates.is_empty() {
            return Vec::new();
        }
        let bound = self.resolver.binds(&before, &tree, decl_range.start, &candidates);
        let mut out = Vec::new();
        // Back in the merged document: a candidate's characters are nobody's
        // effect, so they are there, where they are.
        for r in candidates.iter().zip(bound).filter(|(_, b)| *b).map(|(r, _)| r).rev() {
            let first = before.pos_at(r.start, crate::sumtree::Bias::Right);
            let Some((start, true)) = first.and_then(|p| self.weave.offset_of(p)) else { continue };
            for op in syntax::replace(&self.weave, start..Chars(start.0 + (r.end.0 - r.start.0)), &to) {
                let mut parents = op.refs();
                parents.push(rename);
                parents.sort();
                parents.dedup();
                out.push(Event { id: syntax::derived_id(rename, &op), parents, op, links: vec![rename] });
            }
        }
        out
    }

    /// Integrate events and apply the new ones, oldest first.
    fn absorb(&mut self, mut events: Vec<Event>) -> Vec<Patch> {
        events.retain(|e| !self.log.events().contains_key(&e.id));
        events.sort_by_key(|e| e.id);
        let mut edits = Vec::new();
        for page in events.chunks(crate::limits::SYNC_EVENTS_MAX) {
            let refused: Vec<EventId> = sync::integrate(&mut self.log, page.to_vec()).iter().map(|r| r.event).collect();
            for e in page.iter().filter(|e| !refused.contains(&e.id)) {
                edits.extend(self.take(e));
            }
        }
        edits
    }

    /// Apply one event of the log to the weave and the tree; what it did,
    /// as one replacement.
    fn take(&mut self, e: &Event) -> Option<Patch> {
        if matches!(e.op, Op::Rename { .. }) {
            self.renames.push(e.id);
        }
        let before = self.weave.clone();
        let edits = self.weave.apply(e.id, &e.op);
        self.syntax.follow(&before, &self.weave, &edits);
        let span = syntax::dirty(&edits)?;
        let grown = self.weave.metrics().chars as isize - before.metrics().chars as isize;
        let old_end = before.metrics_at(Chars((span.end as isize - grown) as usize));
        let (start, new_end) = (self.weave.metrics_at(Chars(span.start)), self.weave.metrics_at(Chars(span.end)));
        let mut text = String::with_capacity(new_end.bytes - start.bytes);
        while start.bytes + text.len() < new_end.bytes {
            let chunk = self.weave.chunk_at(start.bytes + text.len());
            text.push_str(&chunk[..chunk.len().min(new_end.bytes - start.bytes - text.len())]);
        }
        Some(Patch { at: self.units(&start), del: self.units(&old_end) - self.units(&start), text })
    }

    /// Number a message and remember the document before it.
    fn message(&mut self, shown: Weave, frontier: Vec<EventId>, edits: Vec<Patch>) -> ToEditor {
        self.sent += 1;
        self.unseen.push(Unseen { n: self.sent, shown, frontier });
        ToEditor::Apply { n: self.sent, acked: self.heard, edits }
    }

    fn units(&self, m: &Metrics) -> usize {
        match self.unit {
            Unit::Chars => m.chars,
            Unit::Bytes => m.bytes,
        }
    }

    /// An editor's offset in `shown`, in characters. An offset outside the
    /// document, or inside a character, means the editor and the session no
    /// longer agree: refused, loudly, rather than guessed at.
    fn offset(&self, shown: &Weave, at: usize) -> Result<Chars, String> {
        let m = shown.metrics();
        let inside = match self.unit {
            Unit::Chars => at <= m.chars,
            Unit::Bytes => shown.is_char_boundary(at),
        };
        if !inside {
            return Err(format!("offset {at} is not in the document as this session has it"));
        }
        Ok(match self.unit {
            Unit::Chars => Chars(at),
            Unit::Bytes => shown.chars_at_byte(at),
        })
    }

    /// The one replacement that turns `a` into `b`: what differs between
    /// their common prefix and suffix.
    fn diff(&self, a: &str, b: &str) -> Option<Patch> {
        if a == b {
            return None;
        }
        let head: usize = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum();
        let (ta, tb) = (&a[head..], &b[head..]);
        let tail: usize =
            ta.chars().rev().zip(tb.chars().rev()).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum();
        let (gone, text) = (&ta[..ta.len() - tail], &tb[..tb.len() - tail]);
        let count = |s: &str| match self.unit {
            Unit::Chars => s.chars().count(),
            Unit::Bytes => s.len(),
        };
        Some(Patch { at: count(&a[..head]), del: count(gone), text: text.to_owned() })
    }
}

/// What the editor asked for, in the state it asked in.
enum Typed {
    Replace(std::ops::Range<Chars>, String),
    /// The intent, and its effect on what the editor showed.
    Rename(Op, Vec<Op>),
}

/// The visible characters `range` of `weave`.
fn slice(weave: &Weave, range: Range<Chars>) -> String {
    let (a, b) = (weave.metrics_at(range.start).bytes, weave.metrics_at(range.end).bytes);
    weave.text()[a..b].to_owned()
}

/// A text in pieces of at most [`INSERT_BYTES_MAX`], cut between
/// characters.
fn pieces(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut cut = rest.len().min(INSERT_BYTES_MAX);
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let (piece, tail) = rest.split_at(cut);
        rest = tail;
        Some(piece)
    })
}
