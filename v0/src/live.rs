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
//! parents. That is what "had not seen the intent" means below, and a
//! peer's intent that was still in the pipe must not count as seen.
//!
//! # Intents
//!
//! An intent ([`Op::Intent`]) is an event that says what was meant: rename
//! this, run this code action on this range, format this range. Its
//! effect is ordinary character ops, recorded after it, each linking to it
//! ([`Event::links`]). Recording one: the anchors are resolved in the
//! state the editor showed, the [`Resolver`] runs the verb there, and the
//! effect is the canonical diff ([`hunks`]) from that state's text to the
//! text the verb gave. ADR-0017 has the why; at merge, for each intent `I`
//! ([`Session::derive`]):
//!
//! 1. *before* is the weave of every event except those linking to `I`:
//!    the document as it would read had `I`'s effect never been applied,
//!    other intents' effects included.
//! 2. `I`'s anchors are resolved in *before*; an anchor that is gone skips
//!    `I` ([`Derivations::gone`]).
//! 3. The resolver runs `I`'s verb on *before*; its replacements applied
//!    to *before*'s text give `T`. A refusal or a malformed answer skips
//!    `I` ([`Derivations::refused`]).
//! 4. `M` is the merged document's text, and the derived ops are the
//!    canonical character diff from `M` to `T` ([`hunks`]).
//! 5. **The guard** keeps convergence independent of the resolver: two
//!    replicas whose resolvers disagree get different `T`s, and without it
//!    a diff could rewrite text everyone had agreed on. A character of `M`
//!    is *unseen* when the event that typed it did not causally see `I`
//!    (its [`EventLog::causal_closure`] lacks `I`). A hunk is allowed iff
//!    every character it deletes is unseen, and it deletes at least one,
//!    or a character of `M` next to it is unseen, or the text it inserts
//!    is a substring of the text another allowed hunk deletes (a move: text
//!    typed inside a range that a code action moved follows it). The rest
//!    are dropped ([`Derivations::dropped`]).
//! 6. Each allowed hunk becomes ops on the merged weave ([`hunk_ops`]),
//!    each an event with id [`syntax::derived_id`], parents its op's
//!    [`Op::refs`] and `I`, linking to `I`.
//! 7. Intents go in [`EventId`] order, each seeing what the ones before it
//!    derived, in passes until one derives nothing: at most
//!    [`PASSES_MAX`] ([`Derivations::capped`] counts a merge cut short).
//!
//! Idempotent by construction: derived events link to `I`, so *before*
//! and `T` are what they were, and once they are in `M` the diff is empty.

use crate::event::{Event, EventLog};
use crate::op::{EventId, Intent, NodeId, NodeKind, Op, Pos, ReplicaId};
use crate::sumtree::Bias;
use crate::sync;
use crate::syntax::{self, Grammar, Syntax};
use crate::weave::{Chars, Metrics, Weave};
use serde::{Deserialize, Serialize};
use similar::{Algorithm, DiffTag, TextDiff};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

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
    /// Run the code action of kind `kind` titled `title` on the units
    /// `at .. end`, as an intent.
    Action { at: usize, end: usize, kind: String, title: String, seen: u64 },
    /// Format the units `at .. end`, as an intent.
    Format { at: usize, end: usize, seen: u64 },
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

/// Where an intent's meaning comes from: which declaration a name the user
/// pointed at means, and what an intent's verb does to a document.
///
/// v0's own answer is [`syntax::ScopeResolver`] (one file, Rust only, by
/// syntax, renames only); a language server is the real one (`v1/`).
/// Everything else an intent needs -- the ops, their ids, who had seen
/// what, which of a resolver's changes a merge may make -- is decided here,
/// in [`Session`], the same whoever resolves.
pub trait Resolver: Send {
    /// The declaration of the name at `at` in `doc`: its characters. `Err`
    /// when there is nothing there to rename.
    fn locate(&mut self, doc: &Weave, syntax: &Syntax, at: Chars) -> Result<Range<Chars>, String>;

    /// Run `intent`'s verb on `doc`: what it changes, as replacements of
    /// characters of `doc` by text, sorted and non-overlapping. The
    /// intent's anchors are in `doc` (a range's: [`span`]). `Err` when the
    /// verb cannot run there; an answer out of order or out of the
    /// document is refused by the caller, never repaired.
    fn run(&mut self, intent: &Intent, doc: &Weave, syntax: &Syntax) -> Result<Vec<(Range<Chars>, String)>, String>;
}

/// The characters `first ..= last` name in `doc`. A deleted anchor still
/// has a place -- where it would be -- so a range that lost an end is what
/// is left of it, as [`Syntax::resolve`] reads a node. `None` when the
/// weave has never seen an anchor, or nothing is left between them.
pub fn span(doc: &Weave, first: Pos, last: Pos) -> Option<Range<Chars>> {
    let (start, _) = doc.offset_of(first)?;
    let (end, visible) = doc.offset_of(last)?;
    let end = Chars(end.0 + usize::from(visible));
    (start < end).then_some(start..end)
}

/// The most passes one merge makes over the intents ([`Session::derive`]).
pub const PASSES_MAX: usize = 8;

/// What merges declined to derive, counted over a session's life. Nothing
/// is printed: a session is pure, and these are how a test or a front end
/// sees it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Derivations {
    /// Intents skipped because an anchor was gone in the document before
    /// them.
    pub gone: u64,
    /// Intents skipped because the resolver refused, or answered with
    /// replacements out of order or outside the document.
    pub refused: u64,
    /// Hunks the guard dropped.
    pub dropped: u64,
    /// Merges cut off after [`PASSES_MAX`] passes that each derived
    /// something.
    pub capped: u64,
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
    /// Every intent in the log, in the order a merge asks them.
    intents: BTreeSet<EventId>,
    resolver: Box<dyn Resolver>,
    derivations: Derivations,
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
        let resolver = Box::new(syntax::ScopeResolver);
        let (unseen, intents, derivations) = (Vec::new(), BTreeSet::new(), Derivations::default());
        Session {
            log,
            replica,
            grammar,
            unit,
            weave,
            syntax,
            unseen,
            sent: 0,
            heard: 0,
            intents,
            resolver,
            derivations,
            held: None,
        }
    }

    /// Resolve intents with `resolver` instead of v0's scope walk.
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

    /// What merges have declined to derive so far.
    pub fn derivations(&self) -> Derivations {
        self.derivations
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
            FromEditor::Rename { at, to, seen } => self.intent(seen, |s, resolver, shown, syntax| {
                let name = resolver.locate(shown, syntax, s.offset(shown, at)?)?;
                let decl = shown.pos_at(name.start, Bias::Right).ok_or("no declaration there")?;
                Ok(Intent::Rename { decl, from: slice(shown, name), to })
            }),
            FromEditor::Action { at, end, kind, title, seen } => self.intent(seen, |s, _, shown, _| {
                let (first, last) = s.range(shown, at, end)?;
                Ok(Intent::Action { first, last, kind, title })
            }),
            FromEditor::Format { at, end, seen } => self.intent(seen, |s, _, shown, _| {
                let (first, last) = s.range(shown, at, end)?;
                Ok(Intent::Format { first, last })
            }),
        }
    }

    /// An intent from the editor: `read` says what was meant, in the state
    /// the editor showed; the resolver runs it there, and its effect is the
    /// canonical diff ([`hunks`]) from that state to what the verb gave.
    fn intent(
        &mut self,
        seen: u64,
        read: impl FnOnce(&Session, &mut dyn Resolver, &Weave, &Syntax) -> Result<Intent, String>,
    ) -> Out {
        // The resolver is borrowed out for the call: `typed` holds the
        // session.
        let mut resolver = std::mem::replace(&mut self.resolver, Box::new(syntax::ScopeResolver));
        let out = self.typed(seen, |s, shown, syntax| {
            let intent = read(s, resolver.as_mut(), shown, syntax)?;
            let to = replaced(shown, &resolver.run(&intent, shown, syntax)?)?;
            let effect = hunks(&shown.text(), &to).iter().flat_map(|h| hunk_ops(shown, h)).collect();
            Ok((false, Typed::Intent(intent, effect)))
        });
        self.resolver = resolver;
        out
    }

    /// An edit or an intent from the editor: find the state it was made in,
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
            // document as it read before the intent ([`Session::derive`]).
            Typed::Intent(intent, effect) => {
                let id = EventId { seq: self.log.lamport_next(), replica: self.replica };
                act(self, &mut shown, Op::Intent(intent), Vec::new());
                effect.into_iter().for_each(|op| act(self, &mut shown, op, vec![id]));
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
        // Typed behind, it is concurrent with any intent the editor had not
        // shown: a merge.
        if behind {
            let (_, derived) = self.settle();
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

    /// Events from a peer: take what is new, let every intent reach the
    /// text it had not seen ([`Session::derive`]), and tell the editor.
    pub fn remote(&mut self, events: Vec<Event>) -> Out {
        if let Some((theirs, _)) = &mut self.held {
            theirs.extend(events);
            return Out::default();
        }
        let before = (self.weave.clone(), self.log.frontier());
        let mut edits = self.absorb(events);
        let (more, derived) = self.settle();
        edits.extend(more);
        let mut out = Out { send: derived, reply: Vec::new() };
        if !edits.is_empty() {
            out.reply.push(self.message(before.0, before.1, edits));
        }
        out
    }

    /// Merge what this replica holds with itself: let every intent reach
    /// the text it had not seen, as a peer's events would. `send` is what
    /// that derived; empty when nothing was left to do, which is what a
    /// second call always is. Offline, nothing: the merge waits.
    pub fn derive(&mut self) -> Out {
        self.remote(Vec::new())
    }

    /// The fixpoint (module doc, step 7): every intent in [`EventId`]
    /// order, each one's derived events absorbed before the next is asked,
    /// in passes until one derives nothing. What that did here, and the
    /// events to send.
    ///
    /// Two replicas that derive from the same log mint the same events, so
    /// sending them is usually a repeat. But they derive when they merge,
    /// and by then one may hold more than the other: a `let foo` its peer
    /// has not seen yet claims a call, and it derives nothing. Sent, the
    /// effect is in every log whoever derived it first.
    fn settle(&mut self) -> (Vec<Patch>, Vec<Event>) {
        let (mut edits, mut sent) = (Vec::new(), Vec::new());
        for _ in 0..PASSES_MAX {
            let mut quiet = true;
            for intent in self.intents.clone() {
                let mut derived = self.derive_one(intent);
                derived.retain(|e| !self.log.events().contains_key(&e.id));
                if !derived.is_empty() {
                    quiet = false;
                    edits.extend(self.absorb(derived.clone()));
                    sent.extend(derived);
                }
            }
            if quiet {
                return (edits, sent);
            }
        }
        self.derivations.capped += 1;
        (edits, sent)
    }

    /// Steps 1 to 6 of the module doc for one intent: the events that bring
    /// the merged document to what `intent` means of it, as far as the
    /// guard allows. Not yet absorbed; some may be in the log already.
    fn derive_one(&mut self, intent: EventId) -> Vec<Event> {
        let Some(Op::Intent(meant)) = self.log.events().get(&intent).map(|e| e.op.clone()) else {
            return Vec::new();
        };
        let mut before = Weave::new(self.weave.node);
        for e in self.log.events().values().filter(|e| !e.links.contains(&intent)) {
            before.apply(e.id, &e.op);
        }
        let anchored = match &meant {
            Intent::Rename { decl, .. } => matches!(before.offset_of(*decl), Some((_, true))),
            Intent::Action { first, last, .. } | Intent::Format { first, last } => span(&before, *first, *last).is_some(),
        };
        if !anchored {
            self.derivations.gone += 1;
            return Vec::new();
        }
        let syntax = Syntax::of(&before, self.grammar);
        let Ok(to) = self.resolver.run(&meant, &before, &syntax).and_then(|r| replaced(&before, &r)) else {
            self.derivations.refused += 1;
            return Vec::new();
        };
        let all = hunks(&self.weave.text(), &to);

        // The guard (step 5). Whether an event saw `intent` is asked once.
        let mut saw: BTreeMap<EventId, bool> = BTreeMap::new();
        let mut unseen = |k: usize| {
            let typed = self.weave.pos_at(Chars(k), Bias::Right).expect("a character of M").event;
            !*saw.entry(typed).or_insert_with(|| self.log.causal_closure(typed).contains(&intent))
        };
        let len = self.weave.metrics().chars;
        let clean: Vec<bool> = all.iter().map(|h| h.old.clone().all(&mut unseen)).collect();
        let first: Vec<bool> = all
            .iter()
            .zip(&clean)
            .map(|(h, &clean)| {
                let near = (h.old.start > 0 && unseen(h.old.start - 1)) || (h.old.end < len && unseen(h.old.end));
                clean && (!h.old.is_empty() || near)
            })
            .collect();
        let moved: Vec<String> =
            all.iter().zip(&first).filter(|(_, ok)| **ok).map(|(h, _)| slice(&self.weave, Chars(h.old.start)..Chars(h.old.end))).collect();
        let allowed: Vec<&Hunk> = all
            .iter()
            .enumerate()
            .filter(|&(i, h)| first[i] || (clean[i] && moved.iter().any(|gone| gone.contains(h.text.as_str()))))
            .map(|(_, h)| h)
            .collect();
        self.derivations.dropped += (all.len() - allowed.len()) as u64;

        let mut out = Vec::new();
        for op in allowed.into_iter().flat_map(|h| hunk_ops(&self.weave, h)) {
            let mut parents = op.refs();
            parents.push(intent);
            parents.sort();
            parents.dedup();
            out.push(Event { id: syntax::derived_id(intent, &op), parents, op, links: vec![intent] });
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
        if matches!(e.op, Op::Intent(_)) {
            self.intents.insert(e.id);
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

    /// The characters `at .. end` of `shown`, in the editor's units, as an
    /// intent names them: by their first and last character. An empty range
    /// has neither, and is refused.
    fn range(&self, shown: &Weave, at: usize, end: usize) -> Result<(Pos, Pos), String> {
        let (a, b) = (self.offset(shown, at)?, self.offset(shown, end)?);
        if a >= b {
            return Err(format!("the range {at}..{end} holds no character"));
        }
        let first = shown.pos_at(a, Bias::Right).ok_or("no character there")?;
        let last = shown.pos_at(b, Bias::Left).ok_or("no character there")?;
        Ok((first, last))
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
    Intent(Intent, Vec<Op>),
}

/// The visible characters `range` of `weave`.
fn slice(weave: &Weave, range: Range<Chars>) -> String {
    let (a, b) = (weave.metrics_at(range.start).bytes, weave.metrics_at(range.end).bytes);
    weave.text()[a..b].to_owned()
}

/// `doc`'s text with a resolver's `replacements` made. Refused, not
/// repaired, when one ends before it starts, ends past the document, or
/// starts before the one before it ends.
fn replaced(doc: &Weave, replacements: &[(Range<Chars>, String)]) -> Result<String, String> {
    let (text, len) = (doc.text(), doc.metrics().chars);
    let byte = |k: Chars| doc.metrics_at(k).bytes;
    let (mut out, mut at) = (String::with_capacity(text.len()), Chars(0));
    for (r, with) in replacements {
        if r.start < at || r.end < r.start || r.end.0 > len {
            return Err(format!("a replacement of {}..{} after one ending at {}, in {len} characters", r.start.0, r.end.0, at.0));
        }
        out.push_str(&text[byte(at)..byte(r.start)]);
        out.push_str(with);
        at = r.end;
    }
    out.push_str(&text[byte(at)..]);
    Ok(out)
}

/// One stretch where two texts differ: the characters `old` of the first,
/// and the text the second has in their place.
#[derive(Debug)]
struct Hunk {
    old: Range<usize>,
    text: String,
}

/// The canonical character diff from `m` to `t`. **Part of the protocol:**
/// two replicas with equal `m` and `t` must mint identical ops, whose ids
/// are content hashes ([`syntax::derived_id`]). So it is pinned: `similar`'s
/// [`TextDiff`] with [`Algorithm::Myers`] over characters (`diff_chars`:
/// Unicode scalar values, not graphemes), with no deadline (a timed-out
/// Myers approximates, differently on a slower machine); each maximal run
/// of ops that are not `Equal` is one hunk, its deleted characters of `m`
/// and the text inserted there. Hunks are in order and never touch.
fn hunks(m: &str, t: &str) -> Vec<Hunk> {
    let diff = TextDiff::configure().algorithm(Algorithm::Myers).diff_chars(m, t);
    let new: Vec<char> = t.chars().collect();
    let mut out: Vec<Hunk> = Vec::new();
    let mut open = false;
    for op in diff.ops() {
        let (tag, old, ins) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            open = false;
            continue;
        }
        let text: String = new[ins].iter().collect();
        match out.last_mut() {
            Some(h) if open => {
                h.old.end = old.end;
                h.text.push_str(&text);
            }
            _ => out.push(Hunk { old, text }),
        }
        open = true;
    }
    out
}

/// One hunk as ops on `weave`, whose text the hunk's offsets are in: a
/// replacement through [`syntax::replace`] (the deletes alone when it
/// inserts nothing), an insertion through [`Weave::insert_op`] at its
/// offset. Computed against `weave` as it is, so the ops of several hunks
/// may be computed first and applied after, in any order.
fn hunk_ops(weave: &Weave, h: &Hunk) -> Vec<Op> {
    if h.old.is_empty() {
        vec![weave.insert_op(Chars(h.old.start), &h.text)]
    } else {
        syntax::replace(weave, Chars(h.old.start)..Chars(h.old.end), &h.text)
    }
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
