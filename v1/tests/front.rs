//! `v1::editor::Front` against fake LSP clients that behave as measured on
//! Helix 25.07, two `v0::live::Session`s, and pipes that deliver late.
//! Whatever the interleaving, every client's buffer ends as its session's
//! document and the sessions agree. Then the cases the protocol exists
//! for, by hand, and the strict readers.

use lsp_types::{Position, Range, TextDocumentContentChangeEvent as Change, TextEdit};
use proptest::prelude::*;
use std::collections::VecDeque;
use v0::live::{FromEditor, Patch, Session, ToEditor, Unit};
use v0::op::ReplicaId;
use v0::syntax::Grammar;
use v1::editor::{Encoding, Front, Out, Push};

// --- the fake client ------------------------------------------------------

/// What a client sends the server, in one stream: the order is the
/// protocol's (an echo comes before the response).
enum Wire {
    Change(i32, Vec<Change>),
    Answer(bool),
}

/// A client's column of `chars` characters from the line start.
fn units(enc: Encoding, line: &str) -> u32 {
    line.chars().map(|c| if enc == Encoding::Utf8 { c.len_utf8() } else { c.len_utf16() } as u32).sum()
}

/// The client's own position arithmetic, written apart from the crate's:
/// the position of byte `at` of `text`.
fn position(enc: Encoding, text: &str, at: usize) -> Position {
    let before = &text[..at];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    Position::new(line as u32, units(enc, &before[start..]))
}

/// The byte of `text` at a position.
fn byte(enc: Encoding, text: &str, p: Position) -> usize {
    let start: usize = text.split_inclusive('\n').take(p.line as usize).map(str::len).sum();
    let mut col = 0;
    for (i, c) in text[start..].char_indices() {
        if col == p.character {
            return start + i;
        }
        col += units(enc, c.encode_utf8(&mut [0; 4]));
    }
    assert_eq!(col, p.character, "the server sent a position outside the buffer");
    text.len()
}

/// The editor, its pipes, and the server's half: a front, a session.
struct Side {
    enc: Encoding,
    buffer: String,
    version: i32,
    /// Applied (or refused) a push; the response is still to be sent.
    answering: Option<bool>,
    refused: u32,
    applied: u32,
    front: Front,
    session: Session,
    to_server: VecDeque<Wire>,
    to_client: VecDeque<Push>,
    to_session: VecDeque<FromEditor>,
    to_front: VecDeque<ToEditor>,
    to_peer: VecDeque<Vec<v0::event::Event>>,
}

impl Side {
    fn new(enc: Encoding, buffer: &str, session: Session) -> Side {
        let front = Front::new(enc, buffer.to_owned(), 1);
        Side {
            enc,
            buffer: buffer.to_owned(),
            version: 1,
            answering: None,
            refused: 0,
            applied: 0,
            front,
            session,
            to_server: Default::default(),
            to_client: Default::default(),
            to_session: Default::default(),
            to_front: Default::default(),
            to_peer: Default::default(),
        }
    }

    fn range(&self, at: usize, del: usize) -> Range {
        Range::new(position(self.enc, &self.buffer, at), position(self.enc, &self.buffer, at + del))
    }

    /// The editor types: its buffer changes, its version moves, the
    /// server hears later. `kind` 0: one change; 1: two, the second in the
    /// coordinates the first left; 2: the whole document, without a range.
    fn type_(&mut self, at: usize, del: usize, text: &str, kind: u8) {
        let chars: Vec<usize> = self.buffer.char_indices().map(|(i, _)| i).chain([self.buffer.len()]).collect();
        let a = chars[at % chars.len()];
        let b = chars[(at % chars.len() + del).min(chars.len() - 1)];
        let mut changes = Vec::new();
        match kind {
            0 | 1 => {
                changes.push(Change { range: Some(self.range(a, b - a)), range_length: None, text: text.to_owned() });
                self.buffer.replace_range(a..b, text);
                if kind == 1 {
                    changes.push(Change { range: Some(self.range(0, 0)), range_length: None, text: "z".to_owned() });
                    self.buffer.insert(0, 'z');
                }
            }
            _ => {
                self.buffer.replace_range(a..b, text);
                changes.push(Change { range: None, range_length: None, text: self.buffer.clone() });
            }
        }
        self.version += 1;
        self.to_server.push_back(Wire::Change(self.version, changes));
    }

    /// The client's half: apply a push if its version is current, send the
    /// echo; the response is a step of its own, so the editor can type
    /// between the two.
    fn client_reads(&mut self) -> bool {
        if let Some(applied) = self.answering.take() {
            self.to_server.push_back(Wire::Answer(applied));
            return true;
        }
        let Some(push) = self.to_client.pop_front() else { return false };
        if push.version != self.version {
            self.refused += 1;
            self.answering = Some(false);
            return true;
        }
        let mut changes = Vec::new();
        for TextEdit { range, new_text } in push.edits {
            let (a, b) = (byte(self.enc, &self.buffer, range.start), byte(self.enc, &self.buffer, range.end));
            self.buffer.replace_range(a..b, &new_text);
            changes.push(Change { range: Some(range), range_length: None, text: new_text });
        }
        self.version += 1;
        self.applied += 1;
        self.to_server.push_back(Wire::Change(self.version, changes));
        self.answering = Some(true);
        true
    }

    fn route(&mut self, out: Out) {
        self.to_session.extend(out.session);
        self.to_client.extend(out.push);
    }

    fn front_reads_client(&mut self) -> bool {
        let Some(wire) = self.to_server.pop_front() else { return false };
        let out = match wire {
            Wire::Change(v, changes) => self.front.changed(v, changes),
            Wire::Answer(applied) => self.front.answered(applied),
        };
        self.route(out.expect("the client behaves as measured"));
        true
    }

    fn session_reads(&mut self) -> bool {
        let Some(msg) = self.to_session.pop_front() else { return false };
        let out = self.session.local(msg);
        self.to_front.extend(out.reply);
        if !out.send.is_empty() {
            self.to_peer.push_back(out.send);
        }
        true
    }

    fn front_hears(&mut self) -> bool {
        let Some(msg) = self.to_front.pop_front() else { return false };
        let out = self.front.heard(msg).expect("the session behaves");
        self.route(out);
        true
    }
}

fn deliver(from: &mut Side, to: &mut Side) -> bool {
    let Some(events) = from.to_peer.pop_front() else { return false };
    let out = to.session.remote(events);
    if !out.send.is_empty() {
        to.to_peer.push_back(out.send);
    }
    to.to_front.extend(out.reply);
    true
}

/// Alice hosts (utf-8, Helix); Bob joins (utf-16, VS Code) with a stale
/// buffer that the session's text replaces. Both settled on `text`.
fn pair(text: &str, grammar: Grammar) -> (Side, Side) {
    let host = Session::host(ReplicaId(1), grammar, Unit::Bytes, "f.rs");
    let mut a = Side::new(Encoding::Utf8, text, host);
    let first = a.front.opened();
    a.to_session.extend(first);
    a.session_reads();
    let guest = Session::guest(ReplicaId(2), grammar, Unit::Bytes, a.session.events()).unwrap();
    a.to_peer.clear();
    let mut b = Side::new(Encoding::Utf16, "stale\n", guest);
    let ready = ToEditor::Ready { role: "guest", text: b.session.text() };
    b.to_front.push_back(ready);
    settle(&mut a, &mut b);
    assert_eq!(b.buffer, text, "the guest's buffer is filled");
    (a, b)
}

/// Empty every pipe. A protocol that never goes quiet fails the test
/// rather than hang it.
fn settle(a: &mut Side, b: &mut Side) {
    let mut rounds = 0;
    while {
        rounds += 1;
        assert!(
            rounds < 300 && a.buffer.len() + b.buffer.len() < 1 << 16,
            "the pipes never emptied: the front and the session keep answering each other"
        );
        a.front_reads_client()
        | b.front_reads_client()
        | a.session_reads()
        | b.session_reads()
        | a.front_hears()
        | b.front_hears()
        | a.client_reads()
        | b.client_reads()
        | deliver(a, b)
        | deliver(b, a)
    } {}
}

fn agree(a: &Side, b: &Side) {
    assert_eq!(a.buffer, a.session.text(), "Alice's buffer is her session's document");
    assert_eq!(b.buffer, b.session.text(), "Bob's buffer is his session's document");
    assert_eq!(a.front.text(), a.buffer, "the mirror is the buffer");
    assert_eq!(b.front.text(), b.buffer);
    assert_eq!(a.session.text(), b.session.text(), "the sessions converge");
}

// --- random ---------------------------------------------------------------

#[derive(Clone, Debug)]
enum Step {
    Type(bool, usize, usize, &'static str, u8),
    FrontReadsClient(bool),
    SessionReads(bool),
    FrontHears(bool),
    ClientReads(bool),
    Deliver(bool),
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    let piece = prop::sample::select(vec!["x", "fn f() {}\n", "é", "→ ", "\n", "let a = 1;", "", "🙂", "𝔘a"]);
    proptest::collection::vec(
        prop_oneof![
            3 => (any::<bool>(), 0usize..200, 0usize..4, piece, 0u8..3).prop_map(|(w, a, d, t, k)| Step::Type(w, a, d, t, k)),
            2 => any::<bool>().prop_map(Step::FrontReadsClient),
            2 => any::<bool>().prop_map(Step::SessionReads),
            2 => any::<bool>().prop_map(Step::FrontHears),
            2 => any::<bool>().prop_map(Step::ClientReads),
            2 => any::<bool>().prop_map(Step::Deliver),
        ],
        1..120,
    )
}

proptest! {
    /// Two editors type at once, one counting utf-8 and one utf-16, while
    /// every pipe -- client to front, front to session, session to front,
    /// front to client, peer to peer -- delivers when it pleases: pushes
    /// are refused and echoed and queued behind typing, and all of it
    /// lands.
    #[test]
    fn buffers_converge_whatever_the_pipes_do(steps in steps()) {
        let (mut a, mut b) = pair("fn main() {\n    let é = 1;\n}\n", Grammar::Rust);
        for step in steps {
            match step {
                Step::Type(w, at, del, t, k) => if w { a.type_(at, del, t, k) } else { b.type_(at, del, t, k) },
                Step::FrontReadsClient(w) => { if w { a.front_reads_client() } else { b.front_reads_client() }; }
                Step::SessionReads(w) => { if w { a.session_reads() } else { b.session_reads() }; }
                Step::FrontHears(w) => { if w { a.front_hears() } else { b.front_hears() }; }
                Step::ClientReads(w) => { if w { a.client_reads() } else { b.client_reads() }; }
                Step::Deliver(w) => { if w { deliver(&mut a, &mut b) } else { deliver(&mut b, &mut a) }; }
            }
        }
        settle(&mut a, &mut b);
        agree(&a, &b);
    }
}

// --- by hand, with sessions -----------------------------------------------

/// A push refused because the client typed: Bob's edit is on its way to
/// Alice's buffer, the push for it is on its way to her client, and she
/// types. Her client refuses; her typing is read where it was typed; one
/// push, current this time, catches her up.
#[test]
fn a_push_refused_because_the_client_typed_is_followed_by_the_catch_up() {
    let (mut a, mut b) = pair("abc\n", Grammar::Markdown);
    b.type_(0, 0, "B", 0);
    b.front_reads_client();
    b.session_reads();
    deliver(&mut b, &mut a);
    a.front_hears(); // the push is in the pipe to her client
    assert_eq!(a.to_client.len(), 1);
    a.type_(3, 0, "A", 0); // after the "c" she sees
    settle(&mut a, &mut b);
    assert_eq!(a.refused, 1, "vacuity: the push was refused, not applied first");
    assert_eq!(a.buffer, "BabcA\n");
    agree(&a, &b);
}

/// Two typing lines of the same pair cross while pushes fly both ways, on
/// multi-change and whole-document didChanges, with non-BMP text.
#[test]
fn typing_in_every_shape_converges() {
    let (mut a, mut b) = pair("a🙂b\n", Grammar::Markdown);
    a.type_(1, 1, "𝔘", 1);
    b.type_(3, 0, "é", 2);
    b.type_(0, 0, "→", 0);
    settle(&mut a, &mut b);
    agree(&a, &b);
    assert!(a.buffer.contains('𝔘') && a.buffer.contains('é') && a.buffer.contains('→'), "{:?}", a.buffer);
}

// --- by hand, the front alone ---------------------------------------------

fn at(line: u32, character: u32) -> Position {
    Position::new(line, character)
}

fn change(range: Option<Range>, text: &str) -> Change {
    Change { range, range_length: None, text: text.to_owned() }
}

fn ins(line: u32, character: u32, text: &str) -> Change {
    change(Some(Range::new(at(line, character), at(line, character))), text)
}

fn apply(n: u64, acked: u64, at: usize, del: usize, text: &str) -> ToEditor {
    ToEditor::Apply { n, acked, edits: vec![Patch { at, del, text: text.to_owned() }] }
}

fn edit(at: usize, del: usize, text: &str, seen: u64) -> FromEditor {
    FromEditor::Edit { at, del, text: text.to_owned(), seen }
}

fn replace(version: i32, from: Position, to: Position, text: &str) -> Push {
    Push { version, edits: vec![TextEdit { range: Range::new(from, to), new_text: text.to_owned() }] }
}

/// An applied push, then typing, all before the answer: the first queued
/// change is the echo and confirms; the rest is typing, against the pushed
/// text, and carries the message's number as what it had seen.
#[test]
fn an_echo_and_typing_in_one_queue() {
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    let out = f.heard(apply(1, 0, 0, 0, "X")).unwrap();
    assert_eq!(out.push, Some(replace(1, at(0, 0), at(0, 0), "X")));
    assert!(out.session.is_empty());
    assert!(f.changed(2, vec![ins(0, 0, "X")]).unwrap().session.is_empty(), "the echo waits for the answer");
    assert!(f.changed(3, vec![ins(0, 4, "!")]).unwrap().session.is_empty(), "so does what was typed after it");
    let out = f.answered(true).unwrap();
    assert_eq!(out.session, vec![FromEditor::Seen(1), edit(4, 0, "!", 1)]);
    assert_eq!(f.text(), "Xabc!\n");
}

/// LSP does not order the response and the echo. A client that answers
/// first: the front keeps waiting, and the next `didChange` is the echo.
/// Typing after it goes on as typing.
#[test]
fn an_answer_before_the_echo_waits_for_it() {
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    f.heard(apply(1, 0, 0, 0, "X")).unwrap();
    let out = f.answered(true).unwrap();
    assert!(out.session.is_empty() && out.push.is_none(), "the push is not yet confirmed");
    assert!(f.offset(at(0, 0)).is_none(), "and still in flight");
    let out = f.changed(2, vec![ins(0, 0, "X")]).unwrap();
    assert_eq!(out.session, vec![FromEditor::Seen(1)]);
    let out = f.changed(3, vec![ins(0, 4, "!")]).unwrap();
    assert_eq!(out.session, vec![edit(4, 0, "!", 1)]);
    assert_eq!(f.text(), "Xabc!\n");
}

/// The echo only has to be later than the push: LSP asks versions to
/// increase, not to step by one. The next push is made at the version the
/// client has.
#[test]
fn an_echo_may_skip_versions() {
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    f.heard(apply(1, 0, 0, 0, "X")).unwrap();
    f.heard(apply(2, 0, 4, 0, "Y")).unwrap();
    f.changed(5, vec![ins(0, 0, "X")]).unwrap();
    let out = f.answered(true).unwrap();
    assert_eq!(out.session, vec![FromEditor::Seen(1)]);
    assert_eq!(out.push, Some(replace(5, at(0, 4), at(0, 4), "Y")));
}

/// A refused push confirms nothing; the queue is typing against the text
/// before the push, and carries the `seen` of before.
#[test]
fn a_refused_push_confirms_nothing() {
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    f.heard(apply(1, 0, 0, 0, "X")).unwrap();
    f.changed(2, vec![ins(0, 3, "!")]).unwrap();
    let out = f.answered(false).unwrap();
    assert_eq!(out.session, vec![edit(3, 0, "!", 0)]);
    assert_eq!(f.text(), "abc!\n");
    assert!(out.push.is_none());
}

/// Session messages wait while a push is in flight, in order; when one is
/// taken the acked rule decides, and a dropped one is skipped.
#[test]
fn waiting_messages_are_taken_in_order_under_the_acked_rule() {
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    f.heard(apply(1, 0, 0, 0, "X")).unwrap();
    assert!(f.heard(apply(2, 0, 4, 0, "Y")).unwrap().push.is_none(), "waits");
    assert!(f.heard(apply(3, 0, 5, 0, "Z")).unwrap().push.is_none(), "waits");
    f.changed(2, vec![ins(0, 0, "X")]).unwrap();
    f.changed(3, vec![ins(0, 4, "!")]).unwrap();
    // One edit was typed since, so 2 and 3 (acked 0) do not fit.
    let out = f.answered(true).unwrap();
    assert_eq!(out.session, vec![FromEditor::Seen(1), edit(4, 0, "!", 1)]);
    assert!(out.push.is_none(), "both were dropped");

    // Without typing: taken in order, one push at a time.
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    f.heard(apply(1, 0, 0, 0, "X")).unwrap();
    f.heard(apply(2, 0, 4, 0, "Y")).unwrap();
    f.heard(apply(3, 0, 5, 0, "Z")).unwrap();
    f.changed(2, vec![ins(0, 0, "X")]).unwrap();
    let out = f.answered(true).unwrap();
    assert_eq!(out.session, vec![FromEditor::Seen(1)]);
    assert_eq!(out.push, Some(replace(2, at(0, 4), at(0, 4), "Y")), "the push after the echo is at version + 1");
    f.changed(3, vec![ins(0, 4, "Y")]).unwrap();
    let out = f.answered(true).unwrap();
    assert_eq!(out.push, Some(replace(3, at(0, 5), at(0, 5), "Z")));
    assert_eq!(out.session, vec![FromEditor::Seen(2)]);
}

/// An `Apply` with no edits is confirmed at once, no push; one that does
/// not fit is not.
#[test]
fn an_empty_apply_is_confirmed_without_a_push() {
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    let out = f.heard(ToEditor::Apply { n: 4, acked: 0, edits: vec![] }).unwrap();
    assert_eq!((out.session, out.push), (vec![FromEditor::Seen(4)], None));
    // Typed since 4, `seen` is 4.
    let out = f.changed(2, vec![ins(0, 0, "q")]).unwrap();
    assert_eq!(out.session, vec![edit(0, 0, "q", 4)]);
    let out = f.heard(ToEditor::Apply { n: 5, acked: 0, edits: vec![] }).unwrap();
    assert!(out.session.is_empty() && out.push.is_none(), "acked 0, but one edit was sent");
    // A patch that changes nothing in the end is also just confirmed.
    let out = f.heard(apply(6, 1, 0, 1, "q")).unwrap();
    assert_eq!(out.session, vec![FromEditor::Seen(6)]);
}

/// utf-16 columns with a character outside the BMP: two code units for
/// the client, four bytes for the session. Both directions, both encodings.
#[test]
fn columns_count_in_the_negotiated_encoding() {
    let text = "a🙂b\né\n";
    let mut f = Front::new(Encoding::Utf16, text.into(), 1);
    assert_eq!(f.offset(at(0, 3)), Some(5), "after the emoji");
    assert_eq!(f.offset(at(1, 1)), Some(9), "after é, on line 1");
    assert_eq!(f.offset(at(2, 0)), Some(10), "the empty last line");
    assert_eq!(f.offset(at(0, 2)), None, "inside the surrogate pair");
    assert_eq!(f.offset(at(0, 5)), None, "past the line");
    assert_eq!(f.offset(at(3, 0)), None, "no such line");
    let out = f.changed(2, vec![ins(0, 3, "X")]).unwrap();
    assert_eq!(out.session, vec![edit(5, 0, "X", 0)]);
    let out = f.heard(apply(1, 1, 5, 1, "")).unwrap();
    assert_eq!(out.push, Some(replace(2, at(0, 3), at(0, 4), "")), "the pushed edit is in utf-16 too");

    let u8 = Front::new(Encoding::Utf8, text.into(), 1);
    assert_eq!(u8.offset(at(0, 5)), Some(5));
    assert_eq!(u8.offset(at(0, 3)), None, "inside the emoji");
    assert_eq!(u8.offset(at(1, 2)), Some(9));
}

/// The changes of one didChange are sequential, the second in the
/// coordinates the first left.
#[test]
fn the_changes_of_one_didchange_are_sequential() {
    let mut f = Front::new(Encoding::Utf8, "abc\ndef\n".into(), 1);
    let second = change(Some(Range::new(at(2, 0), at(2, 1))), "");
    let out = f.changed(2, vec![ins(0, 1, "XY\nZ"), second]).unwrap();
    assert_eq!(out.session, vec![edit(1, 0, "XY\nZ", 0), edit(8, 1, "", 0)]);
    assert_eq!(f.text(), "aXY\nZbc\nef\n");
}

/// A change without a range replaces the document: the session hears what
/// differs; the same text is nothing.
#[test]
fn a_whole_document_change_is_the_difference() {
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    let out = f.changed(2, vec![change(None, "abXc\n")]).unwrap();
    assert_eq!(out.session, vec![edit(2, 0, "X", 0)]);
    let out = f.changed(3, vec![change(None, "abXc\n")]).unwrap();
    assert!(out.session.is_empty());
    let out = f.changed(4, vec![change(None, "é\n")]).unwrap();
    assert_eq!(out.session, vec![edit(0, 4, "é", 0)]);
    assert_eq!(f.text(), "é\n");
}

/// A host sends its buffer as its first edit; a guest's first fill is a
/// push of the whole difference, and nothing if the buffer is the text.
#[test]
fn host_opens_and_guest_is_filled() {
    let mut host = Front::new(Encoding::Utf8, "fn f() {}\n".into(), 1);
    assert_eq!(host.opened(), vec![edit(0, 0, "fn f() {}\n", 0)]);
    assert!(Front::new(Encoding::Utf8, String::new(), 1).opened().is_empty());
    let out = host.heard(ToEditor::Ready { role: "host", text: String::new() }).unwrap();
    assert!(out.push.is_none(), "a host's Ready asks nothing");
    // The host's next message must carry acked = 1.
    let out = host.heard(apply(1, 1, 0, 0, "//")).unwrap();
    assert!(out.push.is_some());

    let mut guest = Front::new(Encoding::Utf16, "old\n".into(), 7);
    let out = guest.heard(ToEditor::Ready { role: "guest", text: "new🙂\n".into() }).unwrap();
    assert_eq!(out.push, Some(replace(7, at(0, 0), at(0, 3), "new🙂")));
    // The fill is not an Apply: no Seen. The client then types.
    guest.changed(8, vec![change(Some(Range::new(at(0, 0), at(0, 3))), "new🙂")]).unwrap();
    guest.changed(9, vec![ins(0, 5, "!")]).unwrap();
    let out = guest.answered(true).unwrap();
    assert_eq!(out.session, vec![edit(7, 0, "!", 0)]);

    let mut same = Front::new(Encoding::Utf8, "same\n".into(), 1);
    let out = same.heard(ToEditor::Ready { role: "guest", text: "same\n".into() }).unwrap();
    assert!(out.push.is_none() && out.session.is_empty());
}

/// A refused fill: what the guest typed into the stale buffer goes with it
/// and is not sent; the fill is pushed again against what is there.
#[test]
fn a_refused_fill_is_pushed_again() {
    let mut f = Front::new(Encoding::Utf8, "old\n".into(), 1);
    f.heard(ToEditor::Ready { role: "guest", text: "new\n".into() }).unwrap();
    f.changed(2, vec![ins(0, 0, "zz")]).unwrap();
    let out = f.answered(false).unwrap();
    assert!(out.session.is_empty(), "typed into the old buffer: not the session's");
    assert_eq!(out.push, Some(replace(2, at(0, 0), at(0, 5), "new")));
}

/// Line endings: `\r\n` counts as one terminator and no column; an edit
/// never starts or ends between the two.
#[test]
fn crlf_is_one_terminator() {
    let mut f = Front::new(Encoding::Utf8, "ab\r\ncd\r\n".into(), 1);
    assert_eq!(f.offset(at(1, 1)), Some(5));
    assert_eq!(f.offset(at(0, 3)), None, "the terminator is not a column");
    // The session replaces "\n" by "x": the pushed edit takes the "\r" too.
    let out = f.heard(apply(1, 0, 3, 1, "x")).unwrap();
    assert_eq!(out.push, Some(replace(1, at(0, 2), at(1, 0), "\rx")));
}

/// Strict readers: nothing is guessed.
#[test]
fn a_bad_position_or_message_is_an_error() {
    let mut f = Front::new(Encoding::Utf16, "a🙂\n".into(), 1);
    assert!(f.changed(2, vec![ins(0, 2, "x")]).is_err(), "inside a surrogate pair");
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    assert!(f.changed(2, vec![ins(0, 9, "x")]).is_err(), "past the line's end");
    assert!(f.changed(2, vec![ins(5, 0, "x")]).is_err(), "no such line");
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    assert!(f.changed(2, vec![change(Some(Range::new(at(0, 2), at(0, 1))), "")]).is_err(), "backwards");
    assert!(f.changed(1, vec![ins(0, 0, "x")]).is_err(), "the version does not advance");
    assert!(f.answered(true).is_err(), "no push in flight");
    assert!(f.heard(ToEditor::Error("no".into())).is_err());
    assert!(f.heard(apply(1, 0, 2, 9, "")).is_err(), "a patch outside the buffer");
    let mut f = Front::new(Encoding::Utf8, "é\n".into(), 1);
    assert!(f.heard(apply(1, 0, 1, 0, "x")).is_err(), "a patch inside a character");

    // An echo that is not the push, or not there.
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    f.heard(apply(1, 0, 0, 0, "X")).unwrap();
    f.answered(true).unwrap();
    assert!(f.answered(true).is_err(), "a second answer to one push");
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    f.heard(apply(1, 0, 0, 0, "X")).unwrap();
    f.changed(2, vec![ins(0, 0, "Y")]).unwrap();
    assert!(f.answered(true).is_err(), "applied, but the echo says something else");
    let mut f = Front::new(Encoding::Utf8, "abc\n".into(), 1);
    f.heard(apply(1, 0, 0, 0, "X")).unwrap();
    f.answered(true).unwrap();
    assert!(f.changed(2, vec![ins(0, 0, "Y")]).is_err(), "the next didChange is the echo, and it is not the push");
}

/// A rename is an intent at a byte; refused while a push is in flight,
/// because the client's coordinates are then ambiguous.
#[test]
fn rename_converts_the_position() {
    let mut f = Front::new(Encoding::Utf16, "🙂 foo\n".into(), 1);
    assert_eq!(f.rename(at(0, 3), "bar".into()).unwrap(), FromEditor::Rename { at: 5, to: "bar".into(), seen: 0 });
    assert!(f.rename(at(0, 99), "bar".into()).is_err());
    f.heard(apply(1, 1, 0, 0, "x")).unwrap();
    assert!(f.rename(at(0, 3), "bar".into()).is_err(), "a push is in flight");
    assert_eq!(f.offset(at(0, 3)), None);
}
