//! Live sessions against fake editors: buffers that type, and pipes that
//! deliver late. Whatever the interleaving, every buffer ends as its
//! session's document, the sessions agree, and the tree is a fresh parse's.

use proptest::prelude::*;
use std::collections::VecDeque;
use v0::event::{Event, EventLog};
use v0::live::{FromEditor, Patch, Resolver, Session, ToEditor, Unit};
use v0::op::{Intent, Op, ReplicaId};
use v0::replay;
use v0::syntax::{Grammar, ScopeResolver};

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
