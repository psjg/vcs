//! Live sessions against fake editors: buffers that type, and pipes that
//! deliver late. Whatever the interleaving, every buffer ends as its
//! session's document, the sessions agree, and the tree is a fresh parse's.

use proptest::prelude::*;
use std::collections::VecDeque;
use std::ops::Range;
use v0::event::{Event, EventLog};
use v0::live::{self, FromEditor, Patch, Resolver, Session, ToEditor, Unit};
use v0::limits::EVENT_TEXT_BYTES_MAX;
use v0::op::{Intent, Op, ReplicaId};
use v0::replay;
use v0::syntax::{Grammar, ScopeResolver, Syntax};
use v0::weave::{Chars, Weave};

/// An editor, its session, and the pipes between them and to the peer.
struct Seat {
    unit: Unit,
    buffer: String,
    sent: u64,
    seen: u64,
    dropped: u32,
    session: Session,
    to_session: VecDeque<FromEditor>,
    to_editor: VecDeque<ToEditor>,
    to_peer: VecDeque<Vec<v0::event::Event>>,
}

impl Seat {
    fn new(session: Session, unit: Unit) -> Seat {
        let buffer = session.text();
        let (to_session, to_editor, to_peer) = Default::default();
        Seat { unit, buffer, sent: 0, seen: 0, dropped: 0, session, to_session, to_editor, to_peer }
    }

    /// A character offset of the buffer in the editor's unit.
    fn units(&self, chars: usize) -> usize {
        match self.unit {
            Unit::Chars => chars,
            Unit::Bytes => self.buffer.chars().take(chars).map(char::len_utf8).sum(),
        }
    }

    fn byte(&self, units: usize) -> usize {
        match self.unit {
            Unit::Chars => self.buffer.chars().take(units).map(char::len_utf8).sum(),
            Unit::Bytes => units,
        }
    }

    fn splice(&mut self, p: &Patch) {
        let (a, b) = (self.byte(p.at), self.byte(p.at + p.del));
        self.buffer.replace_range(a..b, &p.text);
    }

    /// Type: the buffer changes first, the session hears of it later.
    fn type_(&mut self, at: usize, del: usize, text: &str) {
        let len = self.buffer.chars().count();
        let at = at % (len + 1);
        let del = del.min(len - at);
        let p = Patch { at: self.units(at), del: self.units(at + del) - self.units(at), text: text.to_owned() };
        self.splice(&p);
        self.sent += 1;
        self.to_session.push_back(FromEditor::Edit { at: p.at, del: p.del, text: p.text, seen: self.seen });
    }

    fn rename(&mut self, needle: &str, to: &str) {
        let at = self.buffer.find(needle).expect("the name is in the buffer");
        let at = self.units(self.buffer[..at].chars().count());
        self.sent += 1;
        self.to_session.push_back(FromEditor::Rename { at, to: to.to_owned(), seen: self.seen });
    }

    /// The first `needle` of the buffer, in the editor's units.
    fn over(&self, needle: &str) -> (usize, usize) {
        let at = self.buffer[..self.buffer.find(needle).expect("in the buffer")].chars().count();
        (self.units(at), self.units(at + needle.chars().count()))
    }

    fn format(&mut self, needle: &str) {
        let (at, end) = self.over(needle);
        self.sent += 1;
        self.to_session.push_back(FromEditor::Format { at, end, seen: self.seen });
    }

    fn action(&mut self, needle: &str, kind: &str) {
        let (at, end) = self.over(needle);
        self.sent += 1;
        let (kind, title) = (kind.to_owned(), format!("do {kind}"));
        self.to_session.push_back(FromEditor::Action { at, end, kind, title, seen: self.seen });
    }

    /// Type `text` just before the first `needle`.
    fn type_before(&mut self, needle: &str, text: &str) {
        let at = self.buffer[..self.buffer.find(needle).expect("in the buffer")].chars().count();
        self.type_(at, 0, text);
    }

    /// The plugin's half of the protocol.
    fn editor_reads(&mut self) -> bool {
        let Some(msg) = self.to_editor.pop_front() else { return false };
        match msg {
            ToEditor::Apply { n, acked, edits } if acked == self.sent => {
                edits.iter().for_each(|p| self.splice(p));
                self.seen = n;
                self.to_session.push_back(FromEditor::Seen(n));
            }
            ToEditor::Apply { .. } => self.dropped += 1,
            other => panic!("{other:?}"),
        }
        true
    }

    fn session_reads(&mut self) -> bool {
        let Some(msg) = self.to_session.pop_front() else { return false };
        let out = self.session.local(msg);
        self.to_editor.extend(out.reply);
        if !out.send.is_empty() {
            self.to_peer.push_back(out.send);
        }
        true
    }

    fn say(&mut self, msg: FromEditor) {
        self.to_session.push_back(msg);
    }
}

fn deliver(from: &mut Seat, to: &mut Seat) -> bool {
    let Some(events) = from.to_peer.pop_front() else { return false };
    let out = to.session.remote(events);
    // Only a rename's derived effect is sent on a peer's word.
    if !out.send.is_empty() {
        to.to_peer.push_back(out.send);
    }
    to.to_editor.extend(out.reply);
    true
}

/// Alice hosts and counts bytes (Neovim); Bob joins and counts characters
/// (Emacs). Both start on `text`, and resolve with v0's scope walk.
fn pair(text: &str, grammar: Grammar) -> (Seat, Seat) {
    pair_with(text, grammar, || Box::new(ScopeResolver))
}

/// [`pair`], each session resolving with its own `resolver()`.
fn pair_with(text: &str, grammar: Grammar, resolver: fn() -> Box<dyn Resolver>) -> (Seat, Seat) {
    let host = Session::host(ReplicaId(1), grammar, Unit::Bytes, "f.rs").with_resolver(resolver());
    let mut alice = Seat::new(host, Unit::Bytes);
    alice.type_(0, 0, text);
    alice.session_reads();
    let bob = Session::guest(ReplicaId(2), grammar, Unit::Chars, alice.session.events()).unwrap();
    alice.to_peer.clear();
    (alice, Seat::new(bob.with_resolver(resolver()), Unit::Chars))
}

/// Empty every pipe between any number of seats: a batch one sends goes to
/// all the others.
fn settle_all(seats: &mut [Seat]) {
    loop {
        let mut busy = false;
        for i in 0..seats.len() {
            busy |= seats[i].session_reads() | seats[i].editor_reads();
            while let Some(batch) = seats[i].to_peer.pop_front() {
                busy = true;
                for j in (0..seats.len()).filter(|j| *j != i) {
                    let out = seats[j].session.remote(batch.clone());
                    if !out.send.is_empty() {
                        seats[j].to_peer.push_back(out.send);
                    }
                    seats[j].to_editor.extend(out.reply);
                }
            }
        }
        if !busy {
            return;
        }
    }
}

/// Empty every pipe.
fn settle(a: &mut Seat, b: &mut Seat) {
    while a.session_reads() | b.session_reads() | a.editor_reads() | b.editor_reads() | deliver(a, b) | deliver(b, a) {}
}

fn agree(a: &Seat, b: &Seat, grammar: Grammar) {
    assert_eq!(a.buffer, a.session.text(), "Alice's buffer is her session's document");
    assert_eq!(b.buffer, b.session.text(), "Bob's buffer is his session's document");
    assert_eq!(a.session.text(), b.session.text(), "the sessions converge");
    let fresh = Session::guest(ReplicaId(9), grammar, Unit::Chars, a.session.events()).unwrap();
    assert_eq!(fresh.text(), a.session.text(), "and a newcomer reads the same from the events alone");
    assert_eq!(a.session.tree(), fresh.tree(), "the followed tree is a fresh parse");
    assert_eq!(b.session.tree(), fresh.tree());
}

#[derive(Clone, Debug)]
enum Step {
    Type(bool, usize, usize, &'static str),
    SessionReads(bool),
    EditorReads(bool),
    Deliver(bool),
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    let piece = prop::sample::select(vec!["x", "fn f() {}\n", "é", "→ ", "\n", "let a = 1;", "", "🙂"]);
    proptest::collection::vec(
        prop_oneof![
            3 => (any::<bool>(), 0usize..200, 0usize..4, piece).prop_map(|(w, at, del, t)| Step::Type(w, at, del, t)),
            2 => any::<bool>().prop_map(Step::SessionReads),
            2 => any::<bool>().prop_map(Step::EditorReads),
            2 => any::<bool>().prop_map(Step::Deliver),
        ],
        1..80,
    )
}

proptest! {
    /// Two editors type at once while every pipe delivers when it pleases:
    /// edits cross peers' edits on the way to the buffer, and all of it
    /// still lands.
    #[test]
    fn buffers_converge_whatever_the_pipes_do(steps in steps()) {
        let (mut a, mut b) = pair("fn main() {\n    let é = 1;\n}\n", Grammar::Rust);
        for step in steps {
            match step {
                Step::Type(who, at, del, text) => if who { a.type_(at, del, text) } else { b.type_(at, del, text) },
                Step::SessionReads(who) => { if who { a.session_reads() } else { b.session_reads() }; }
                Step::EditorReads(who) => { if who { a.editor_reads() } else { b.editor_reads() }; }
                Step::Deliver(who) => { if who { deliver(&mut a, &mut b) } else { deliver(&mut b, &mut a) }; }
            }
        }
        settle(&mut a, &mut b);
        agree(&a, &b, Grammar::Rust);
    }
}

/// The case the protocol exists for, by hand: Bob's edit is on its way to
/// Alice's buffer when she types. Her editor drops his message, her session
/// reads her edit against the buffer she had, and one message brings her
/// buffer to the merged document.
#[test]
fn an_edit_typed_behind_is_read_where_it_was_typed() {
    let (mut a, mut b) = pair("abc\n", Grammar::Markdown);
    b.type_(0, 0, "B");
    b.session_reads();
    deliver(&mut b, &mut a); // in Alice's pipe, not in her buffer
    a.type_(3, 0, "A"); // after the "c" she sees, which is not offset 3 any more
    settle(&mut a, &mut b);
    assert_eq!(a.dropped, 1, "vacuity: the stale message was dropped, not applied");
    assert_eq!(a.buffer, "BabcA\n");
    agree(&a, &b, Grammar::Markdown);
}

/// The demonstration, as the editors run it: both go offline, Alice renames
/// `foo`, Bob adds a call to `foo`, both come back. Bob's call reads `bar`
/// in both buffers.
#[test]
fn a_rename_reaches_a_call_typed_offline_in_the_other_editor() {
    let (mut a, mut b) = pair("fn foo() {}\nfn main() {\n    foo();\n}\n", Grammar::Rust);
    a.say(FromEditor::Hold(true));
    b.say(FromEditor::Hold(true));
    // Alice renames from a use, not the declaration: the cursor is anywhere.
    a.rename("foo();", "bar");
    let end = b.buffer.rfind("}\n").unwrap();
    b.type_(b.buffer[..end].chars().count(), 0, "    foo();\n");
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "fn bar() {}\nfn main() {\n    bar();\n}\n", "her editor is told the rename's effect");
    assert_eq!(b.buffer, "fn foo() {}\nfn main() {\n    foo();\n    foo();\n}\n", "offline: nothing of hers yet");

    a.say(FromEditor::Hold(false));
    b.say(FromEditor::Hold(false));
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "fn bar() {}\nfn main() {\n    bar();\n    bar();\n}\n");
    agree(&a, &b, Grammar::Rust);
}

/// A rename's reach is judged by what the editor had shown, not by what its
/// session held: Alice's rename is in Bob's pipe when he types a call to
/// the old name, so he had not seen it, and his call is renamed.
#[test]
fn a_rename_still_in_the_pipe_was_not_seen() {
    let (mut a, mut b) = pair("fn foo() {}\nfn main() {\n}\n", Grammar::Rust);
    a.rename("foo", "bar");
    a.session_reads();
    deliver(&mut a, &mut b); // Bob's session has the rename; his buffer does not
    let at = b.buffer.rfind("}\n").unwrap();
    b.type_(b.buffer[..at].chars().count(), 0, "    foo();\n");
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "fn bar() {}\nfn main() {\n    bar();\n}\n");
    agree(&a, &b, Grammar::Rust);
}

/// An editor that has lost its place is told so, and nothing changes.
#[test]
fn an_offset_outside_the_document_is_refused() {
    let (mut a, _) = pair("é\n", Grammar::Markdown);
    for at in [1, 99] {
        let out = a.session.local(FromEditor::Edit { at, del: 0, text: "x".into(), seen: 0 });
        assert!(matches!(out.reply[..], [ToEditor::Error(_)]), "byte {at}: {:?}", out.reply);
        assert!(out.send.is_empty());
    }
    assert_eq!(a.session.text(), "é\n");
    let out = a.session.local(FromEditor::Rename { at: 0, to: "x".into(), seen: 0 });
    assert!(matches!(out.reply[..], [ToEditor::Error(_)]), "Markdown has no names to rename");
}

/// The lines themselves: one spelling each, and nothing else is read.
#[test]
fn the_wire_format_is_one_spelling_per_message() {
    let edit: FromEditor = serde_json::from_str(r#"{"edit":{"at":1,"del":2,"text":"x","seen":3}}"#).unwrap();
    assert_eq!(edit, FromEditor::Edit { at: 1, del: 2, text: "x".into(), seen: 3 });
    assert_eq!(serde_json::from_str::<FromEditor>(r#"{"seen":4}"#).unwrap(), FromEditor::Seen(4));
    assert_eq!(serde_json::from_str::<FromEditor>(r#"{"hold":true}"#).unwrap(), FromEditor::Hold(true));
    assert!(serde_json::from_str::<FromEditor>(r#"{"edit":{"at":1,"del":2,"text":"x","seen":3,"more":1}}"#).is_err());
    let apply = ToEditor::Apply { n: 1, acked: 2, edits: vec![Patch { at: 0, del: 1, text: "y".into() }] };
    assert_eq!(serde_json::to_string(&apply).unwrap(), r#"{"apply":{"n":1,"acked":2,"edits":[{"at":0,"del":1,"text":"y"}]}}"#);
    let ready = ToEditor::Ready { role: "host", text: String::new() };
    assert_eq!(serde_json::to_string(&ready).unwrap(), r#"{"ready":{"role":"host","text":""}}"#);
}

// --- found in review ----------------------------------------------------------

/// A rename's effect is derived when a replica merges, and replicas merge
/// at different moments. Alice derives it for Bob's call before she hears
/// of the `foo` he then declares in front of it; Bob, who hears of her
/// rename last, finds his call claimed by that `foo` and derives nothing.
/// So the derived events are sent, and both end as the one who saw it
/// first.
#[test]
fn a_rename_derived_on_one_side_reaches_the_other() {
    let (mut a, mut b) = pair("fn foo() {}\nfn main() {\n}\n", Grammar::Rust);
    a.rename("foo", "bar");
    a.session_reads(); // her rename waits in the pipe to Bob
    let at = b.buffer.rfind("}\n").unwrap();
    b.type_(at, 0, "    foo();\n");
    b.session_reads();
    deliver(&mut b, &mut a);
    assert!(a.session.text().contains("    bar();\n"), "vacuity: Alice derived it: {}", a.session.text());
    b.type_(at, 0, "    let foo = || {};\n");
    b.session_reads();
    deliver(&mut b, &mut a);
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "fn bar() {}\nfn main() {\n    let foo = || {};\n    bar();\n}\n");
    agree(&a, &b, Grammar::Rust);
}

/// A request that is refused changes nothing, the states kept for the
/// editor included: its next edit, still behind, is still read where it
/// was typed.
#[test]
fn a_refused_request_keeps_the_editor_s_place() {
    let (mut a, mut b) = pair("abc\n", Grammar::Markdown);
    b.type_(0, 0, "x");
    b.session_reads();
    deliver(&mut b, &mut a); // unread in Alice's pipe
    a.sent += 1;
    a.say(FromEditor::Rename { at: 0, to: "q".into(), seen: 0 });
    a.session_reads();
    assert!(matches!(a.to_editor.pop_back(), Some(ToEditor::Error(_))));
    a.type_(3, 0, "A");
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "xabcA\n");
    agree(&a, &b, Grammar::Markdown);
}

/// Behind, and the catch-up has nothing to change: both deleted the same
/// character. The editor has still not seen the peer's event, and its next
/// event must not name it as a parent until it has confirmed a message.
#[test]
fn an_empty_catch_up_is_still_a_message_to_confirm() {
    let (mut a, mut b) = pair("abc\n", Grammar::Markdown);
    b.type_(1, 1, "");
    b.session_reads();
    deliver(&mut b, &mut a); // unread in Alice's pipe
    let theirs = a.session.events().last().unwrap().id;
    a.type_(1, 1, "");
    a.session_reads();
    assert!(
        matches!(a.to_editor.back(), Some(ToEditor::Apply { edits, acked: 2, .. }) if edits.is_empty()),
        "{:?}",
        a.to_editor
    );
    // Typed before reading it: still behind.
    a.type_(0, 0, "A");
    a.session_reads();
    let mine = a.session.events().into_iter().max_by_key(|e| e.id).unwrap();
    assert!(!mine.parents.contains(&theirs), "not seen, so not a parent: {mine:?}");
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "Aac\n");
    agree(&a, &b, Grammar::Markdown);
}

/// A field is not a name the scope walk resolves: asked to rename one, the
/// session refuses rather than rename the local spelled like it.
#[test]
fn a_field_is_not_renamed_as_the_local_of_its_name() {
    let src = "struct S { foo: i32 }\nfn f(s: S) { let foo = 1; s.foo; foo; }\n";
    let (mut a, _) = pair(src, Grammar::Rust);
    let at = src.find("s.foo").unwrap() + 2;
    let out = a.session.local(FromEditor::Rename { at, to: "bar".into(), seen: 0 });
    assert!(matches!(out.reply[..], [ToEditor::Error(_)]), "{:?}", out.reply);
    assert_eq!(a.session.text(), src);
}

/// v0's scope walk renames only: asked to format or to run a code action,
/// it says so, and nothing is recorded.
#[test]
fn the_scope_walk_refuses_a_format_and_an_action() {
    let (mut a, _) = pair("fn f() { let x=1; }\n", Grammar::Rust);
    let format = FromEditor::Format { at: 0, end: 4, seen: 0 };
    let action = FromEditor::Action { at: 0, end: 4, kind: "refactor.extract".into(), title: "x".into(), seen: 0 };
    for msg in [format, action] {
        let out = a.session.local(msg);
        assert!(matches!(&out.reply[..], [ToEditor::Error(why)] if why.contains("language server")), "{:?}", out.reply);
        assert!(out.send.is_empty());
    }
    let out = a.session.local(FromEditor::Format { at: 2, end: 2, seen: 0 });
    assert!(matches!(&out.reply[..], [ToEditor::Error(why)] if why.contains("no character")), "an empty range: {:?}", out.reply);
    assert_eq!(a.session.text(), "fn f() { let x=1; }\n");
}

/// A host that is offline still passes on what it holds for itself: a
/// guest that joins meanwhile is sent the events the host has not read.
#[test]
fn a_joiner_gets_what_an_offline_host_is_holding() {
    let (mut a, mut b) = pair("abc\n", Grammar::Markdown);
    a.say(FromEditor::Hold(true));
    a.session_reads();
    b.type_(0, 0, "x");
    b.session_reads();
    deliver(&mut b, &mut a);
    assert_eq!(a.session.text(), "abc\n", "vacuity: held, not read");
    let carol = Session::guest(ReplicaId(3), Grammar::Markdown, Unit::Chars, a.session.events()).unwrap();
    assert_eq!(carol.text(), "xabc\n");
}

// --- intents, through the session -----------------------------------------------

/// The acceptance test for intents: Alice renames `foo` to `bar` everywhere;
/// Bob, concurrently, adds a call to `foo` and a `foo` of his own. After the
/// merge Bob's call reads `bar` and his own `foo` is his, on both replicas;
/// replay agrees; and nothing is left to derive.
#[test]
fn a_rename_reaches_a_reference_added_concurrently() {
    let (mut a, mut b) = pair("fn foo() {}\nfn main() {\n    foo();\n}\n", Grammar::Rust);
    a.say(FromEditor::Hold(true));
    b.say(FromEditor::Hold(true));
    a.rename("foo", "bar");
    let end = b.buffer.rfind("}\n").unwrap();
    b.type_(end, 0, "    foo();\n    { let foo = 2; foo; }\n");
    settle(&mut a, &mut b);

    // The intent, then its effect on what she could see, linking to it.
    let events = a.session.events();
    let intent = events.iter().find(|e| matches!(e.op, Op::Intent(_))).expect("an intent");
    let Op::Intent(Intent::Rename { from, to, .. }) = &intent.op else { panic!("{intent:?}") };
    assert_eq!((from.as_str(), to.as_str()), ("foo", "bar"));
    let effect: Vec<&Event> = events.iter().filter(|e| e.links == [intent.id]).collect();
    assert_eq!(effect.len(), 4, "two references, an insert and a delete each: {effect:?}");

    a.say(FromEditor::Hold(false));
    b.say(FromEditor::Hold(false));
    settle(&mut a, &mut b);
    let want = "fn bar() {}\nfn main() {\n    bar();\n    bar();\n    { let foo = 2; foo; }\n}\n";
    assert_eq!(a.buffer, want, "Bob's call reads bar; his own foo is his");
    agree(&a, &b, Grammar::Rust);
    assert!(a.session.derive().send.is_empty() && b.session.derive().send.is_empty(), "nothing is left to do");
    let log = EventLog::from(a.session.events());
    let all: Vec<_> = log.events().keys().copied().collect();
    assert_eq!(replay::materialise_events(&all, &log).files["f.rs"], want, "derived events are ordinary events");
}

/// A name typed in pieces is renamed all the same where the rename had not
/// seen it; an old name completed by someone who *had* seen the rename was
/// written on purpose, and is left alone -- the guard drops that hunk.
#[test]
fn a_rename_knows_who_had_seen_it() {
    let (mut a, mut b) = pair("fn f(){} fn fo(){} fn main(){fo();}", Grammar::Rust);
    a.type_(4, 0, "oo");
    settle(&mut a, &mut b);
    a.say(FromEditor::Hold(true));
    b.say(FromEditor::Hold(true));
    a.rename("foo", "bar");
    b.type_before("fo();", "foo();");
    settle(&mut a, &mut b);
    a.say(FromEditor::Hold(false));
    b.say(FromEditor::Hold(false));
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "fn bar(){} fn fo(){} fn main(){bar();fo();}");
    agree(&a, &b, Grammar::Rust);

    let at = a.buffer.find("fo();").unwrap() + 2;
    a.type_(at, 0, "o");
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "fn bar(){} fn fo(){} fn main(){bar();foo();}", "written on purpose");
    assert!(b.session.derivations().dropped > 0, "vacuity: Bob's merge wanted it renamed");
    agree(&a, &b, Grammar::Rust);
}

// --- the intent rule (ADR-0017) ---------------------------------------------------

/// Resolvers for the rule's tests: verbs whose effect is easy to read.
#[derive(Clone, Copy)]
enum Stub {
    /// Format: a space either side of each `=` in the range that has none.
    Spaced,
    /// Format: a space either side of each `=` in the range, spaced or not.
    /// Never idempotent, so it shows a merge that re-reads its own effect.
    Respaced,
    /// Format, carelessly: every unspaced `=` in the document, whatever the
    /// range. It rewrites text typed after it, by people who had seen it.
    Everywhere,
    /// An action that moves the range's text to the end of the document.
    Mover,
    /// Format: each `=` in the range becomes more text than one event may
    /// carry.
    Big,
}

impl Resolver for Stub {
    fn locate(&mut self, _: &Weave, _: &Syntax, _: Chars) -> Result<Range<Chars>, String> {
        Err("a stub renames nothing".into())
    }

    fn run(&mut self, intent: &Intent, doc: &Weave, _: &Syntax) -> Result<Vec<(Range<Chars>, String)>, String> {
        let (Intent::Format { first, last } | Intent::Action { first, last, .. }) = intent else {
            return Err("a stub renames nothing".into());
        };
        let range = live::span(doc, *first, *last).ok_or("the range is gone")?;
        let text: Vec<char> = doc.text().chars().collect();
        let end = Chars(text.len());
        let range = match self {
            Stub::Mover => return Ok(vec![(range.clone(), String::new()), (end..end, text[range.start.0..range.end.0].iter().collect())]),
            Stub::Everywhere => Chars(0)..end,
            Stub::Spaced | Stub::Respaced | Stub::Big => range,
        };
        let spaced = |k: usize| k > 0 && text[k - 1] == ' ' && text.get(k + 1) == Some(&' ');
        let again = matches!(self, Stub::Respaced);
        let with = match self {
            Stub::Big => "é".repeat(EVENT_TEXT_BYTES_MAX / 2 + 1),
            _ => " = ".to_owned(),
        };
        Ok((range.start.0..range.end.0)
            .filter(|&k| text[k] == '=' && (again || !spaced(k)))
            .map(|k| (Chars(k)..Chars(k + 1), with.clone()))
            .collect())
    }
}

/// Both offline, Alice runs `alice` while Bob runs `bob`; then both come
/// back and every pipe empties.
fn concurrently(a: &mut Seat, b: &mut Seat, alice: impl FnOnce(&mut Seat), bob: impl FnOnce(&mut Seat)) {
    a.say(FromEditor::Hold(true));
    b.say(FromEditor::Hold(true));
    alice(a);
    bob(b);
    settle(a, b);
    a.say(FromEditor::Hold(false));
    b.say(FromEditor::Hold(false));
    settle(a, b);
}

/// (a) Deriving twice derives nothing. The formatter is not idempotent,
/// so a merge that read its own derived events back would format again.
#[test]
fn deriving_twice_derives_nothing() {
    let (mut a, mut b) = pair_with("a=1\nb=2\n", Grammar::Markdown, || Box::new(Stub::Respaced));
    concurrently(&mut a, &mut b, |a| a.format("a=1\nb=2\n"), |b| b.type_before("b=2", "x=9\n"));
    assert_eq!(a.buffer, "a = 1\nx = 9\nb = 2\n", "vacuity: the merge derived");
    agree(&a, &b, Grammar::Markdown);
    for s in [&mut a, &mut b] {
        assert!(s.session.derive().send.is_empty(), "a second merge has nothing to do");
        assert_eq!(s.session.derivations().capped, 0);
    }
    let fresh = Session::guest(ReplicaId(9), Grammar::Markdown, Unit::Chars, a.session.events()).unwrap();
    let mut fresh = fresh.with_resolver(Box::new(Stub::Respaced));
    assert!(fresh.derive().send.is_empty(), "nor does a newcomer's first");
}

/// (b) The fixpoint: Alice renames `foo`, Bob renames `baz`, and Carol,
/// seeing neither, calls both. Each rename is asked of a document that has
/// the other's effect, so neither undoes the other, and all three end the
/// same in a bounded number of passes.
#[test]
fn concurrent_renames_of_two_names_meet_in_a_third_peer_s_code() {
    let (a, b) = pair("fn foo() {}\nfn baz() {}\nfn main() {\n}\n", Grammar::Rust);
    let carol = Session::guest(ReplicaId(3), Grammar::Rust, Unit::Chars, a.session.events()).unwrap();
    let mut seats = [a, b, Seat::new(carol, Unit::Chars)];
    for s in &mut seats {
        s.say(FromEditor::Hold(true));
    }
    seats[0].rename("foo", "bar");
    seats[1].rename("baz", "qux");
    let end = seats[2].buffer.rfind("}\n").unwrap();
    seats[2].type_(end, 0, "    foo();\n    baz();\n");
    settle_all(&mut seats);
    for s in &mut seats {
        s.say(FromEditor::Hold(false));
    }
    settle_all(&mut seats);
    let want = "fn bar() {}\nfn qux() {}\nfn main() {\n    bar();\n    qux();\n}\n";
    for s in &seats {
        assert_eq!(s.buffer, want);
        assert_eq!(s.session.text(), want);
        assert_eq!(s.session.derivations().capped, 0, "settled within the bound");
    }
    agree(&seats[0], &seats[1], Grammar::Rust);
    agree(&seats[0], &seats[2], Grammar::Rust);
}

/// (c) The guard: a careless formatter also rewrites `z=9`, which Alice
/// typed after formatting. That hunk is dropped -- she had seen the format
/// and wrote it so -- while Bob's concurrent line is formatted, and the
/// sessions converge.
#[test]
fn the_guard_keeps_text_typed_after_the_intent() {
    let (mut a, mut b) = pair_with("a=1\nb=2\n", Grammar::Markdown, || Box::new(Stub::Everywhere));
    let alice = |a: &mut Seat| {
        a.format("a=1\n");
        while a.session_reads() | a.editor_reads() {}
        let end = a.buffer.chars().count();
        a.type_(end, 0, "z=9\n");
    };
    concurrently(&mut a, &mut b, alice, |b| b.type_(0, 0, "y=8\n"));
    assert_eq!(a.buffer, "y = 8\na = 1\nb = 2\nz=9\n");
    for s in [&a, &b] {
        assert!(s.session.derivations().dropped >= 2, "vacuity: {:?}", s.session.derivations());
    }
    agree(&a, &b, Grammar::Markdown);
}

/// (d) Format at merge: a line Bob typed inside Alice's range while she
/// formatted it is formatted too; his line outside the range is not.
#[test]
fn a_format_reaches_a_line_typed_inside_its_range() {
    let (mut a, mut b) = pair_with("a=1\nb=2\nc=3\n", Grammar::Markdown, || Box::new(Stub::Spaced));
    let bob = |b: &mut Seat| {
        b.type_before("b=2", "x=9\n");
        let end = b.buffer.chars().count();
        b.type_(end, 0, "y=8\n");
    };
    concurrently(&mut a, &mut b, |a| a.format("a=1\nb=2\n"), bob);
    assert_eq!(a.buffer, "a = 1\nx = 9\nb = 2\nc=3\ny=8\n");
    agree(&a, &b, Grammar::Markdown);
}

/// (e) The move case of the guard: Alice's action moves a line to the end;
/// the word Bob typed inside it meanwhile is left behind by the character
/// merge, and the derived effect takes it to where the line went.
#[test]
fn a_word_typed_inside_a_moved_range_follows_it() {
    let (mut a, mut b) = pair_with("two three\nlong rest of the document here\n", Grammar::Markdown, || Box::new(Stub::Mover));
    concurrently(&mut a, &mut b, |a| a.action("two three\n", "move.end"), |b| b.type_before("three", "X "));
    assert_eq!(a.buffer, "long rest of the document here\ntwo X three\n");
    agree(&a, &b, Grammar::Markdown);
}

/// (f) Two replicas holding the same events derive the same events: ids,
/// parents, ops, order. So does a third that joins with them all.
#[test]
fn two_replicas_derive_identical_events() {
    let (mut a, mut b) = pair_with("a=1\nb=2\n", Grammar::Markdown, || Box::new(Stub::Spaced));
    a.format("a=1\nb=2\n");
    a.session_reads();
    b.type_before("b=2", "x=9\n");
    b.session_reads();
    let to_bob: Vec<Event> = a.to_peer.drain(..).flatten().collect();
    let to_alice: Vec<Event> = b.to_peer.drain(..).flatten().collect();
    let mut all = a.session.events();
    all.extend(to_alice.clone());
    let from_a = a.session.remote(to_alice);
    let from_b = b.session.remote(to_bob);
    assert!(!from_a.send.is_empty(), "vacuity: something was derived");
    assert_eq!(from_a.send, from_b.send);
    let carol = Session::guest(ReplicaId(3), Grammar::Markdown, Unit::Chars, all).unwrap();
    assert_eq!(carol.with_resolver(Box::new(Stub::Spaced)).derive().send, from_a.send);

    a.to_editor.extend(from_a.reply);
    b.to_editor.extend(from_b.reply);
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "a = 1\nx = 9\nb = 2\n");
    agree(&a, &b, Grammar::Markdown);
}

/// (g) An effect larger than an event may carry is cut into events that
/// do, as typing is (`INSERT_BYTES_MAX`). One event over
/// `EVENT_TEXT_BYTES_MAX` is refused by every peer but its origin, whose
/// document then differs.
#[test]
fn an_effect_over_the_event_limit_reaches_every_peer() {
    let (mut a, mut b) = pair_with("a=1\n", Grammar::Markdown, || Box::new(Stub::Big));
    a.format("a=1\n");
    settle(&mut a, &mut b);
    assert!(a.buffer.len() > EVENT_TEXT_BYTES_MAX, "vacuity: the line became large");
    // Not `assert_eq!`: it would print a megabyte.
    let (x, y) = (a.session.text(), b.session.text());
    assert!(x == y, "the sessions converge: {} and {} bytes", x.len(), y.len());
    agree(&a, &b, Grammar::Markdown);
}

/// (g), the other path: the effect Bob derives at the merge for a line he
/// typed inside Alice's range is cut too.
#[test]
fn a_derived_effect_over_the_event_limit_reaches_every_peer() {
    let (mut a, mut b) = pair_with("a=1\nbb\n", Grammar::Markdown, || Box::new(Stub::Big));
    concurrently(&mut a, &mut b, |a| a.format("a=1\nbb\n"), |b| b.type_before("bb", "x=9\n"));
    assert!(a.buffer.len() > 2 * EVENT_TEXT_BYTES_MAX, "vacuity: both lines became large");
    let (x, y) = (a.session.text(), b.session.text());
    assert!(x == y, "the sessions converge: {} and {} bytes", x.len(), y.len());
    agree(&a, &b, Grammar::Markdown);
}

// --- what ADR-0017 measured and does not solve ------------------------------------
//
// Characterisation tests: they pin the outcomes the ADR reports as limits, so
// that narrowing the guard or reconciling intents shows up as a failure to
// read and then update, not as a silent change.

/// (h) Two intents on one target collide: Alice renames `foo` to `bar` while
/// Bob renames it to `baz`. Each one's *before* holds the other's effect,
/// which deleted the declaration, so both skip as gone, and the call that
/// both effects reached reads `barbaz`. Nothing reconciles the two; when
/// this test fails because the sessions now agree on one name, or report a
/// conflict, ADR-0017 "what it does not solve" is out of date.
#[test]
fn two_renames_of_one_name_leave_both_names() {
    let (mut a, mut b) = pair("fn foo() {}\nfn main() {\n    foo();\n}\n", Grammar::Rust);
    concurrently(&mut a, &mut b, |a| a.rename("foo", "bar"), |b| b.rename("foo", "baz"));
    assert_eq!(a.buffer, "fn barbaz() {}\nfn main() {\n    barbaz();\n}\n");
    agree(&a, &b, Grammar::Rust);
}

/// (i) Text older than an intent is unseen by it. Alice formats, then types
/// `=b` after the `1` the format saw. She wrote it so, yet the next merge
/// (her own edit's) formats the edge that touches the `1`: the hunk is next
/// to an "unseen" character, because the `1` is older than the format. The
/// other edge, between two characters that did see it, is dropped. Harmless
/// for a formatter, wrong in principle; narrowing "unseen" to "concurrent
/// with the intent" would fail this test, and that is the point of it.
#[test]
fn text_typed_after_a_format_is_formatted_where_it_touches_older_text() {
    let (mut a, mut b) = pair_with("a=1\nb=2\n", Grammar::Markdown, || Box::new(Stub::Spaced));
    a.format("a=1\nb=2\n");
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "a = 1\nb = 2\n", "vacuity: the format ran");
    a.type_before("\nb = 2", "=b");
    settle(&mut a, &mut b);
    assert_eq!(a.buffer, "a = 1 =b\nb = 2\n");
    assert!(a.session.derivations().dropped >= 1, "vacuity: the far edge was refused by the guard");
    agree(&a, &b, Grammar::Markdown);
}
