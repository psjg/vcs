//! The structure layer against its oracles: a fresh parse, and replay.

use proptest::prelude::*;
use v0::event::EventLog;
use v0::op::{Anchor, EventId, NodeId, NodeKind, Op, ReplicaId, Side};
use v0::replay;
use v0::sumtree::Bias;
use v0::sync;
use v0::syntax::{self, Grammar, Syntax};
use v0::weave::{Chars, Metrics, Weave};

/// One replica typing into one document: a log, the live weave, its tree.
struct Desk {
    log: EventLog,
    seq: u32,
    replica: ReplicaId,
    node: NodeId,
    weave: Weave,
    syntax: Syntax,
}

impl Desk {
    fn new(text: &str, grammar: Grammar) -> Desk {
        let (mut log, mut seq, replica) = (EventLog::default(), 1, ReplicaId(1));
        let node = NodeId(EventId { seq, replica });
        log.append(replica, &mut seq, Op::Create { node, parent: Op::ROOT, name: "f".into(), kind: NodeKind::File });
        let weave = Weave::new(node);
        let syntax = Syntax::of(&weave, grammar);
        let mut desk = Desk { log, seq, replica, node, weave, syntax };
        desk.type_at(0, text);
        desk
    }

    /// Another replica holding everything this one has.
    fn fork(&self, replica: u64, grammar: Grammar) -> Desk {
        let weave = self.weave.clone();
        let syntax = Syntax::of(&weave, grammar);
        Desk { log: self.log.clone(), seq: self.seq, replica: ReplicaId(replica), node: self.node, weave, syntax }
    }

    /// Append an op, apply it, let the tree follow: what a front-end does
    /// per keystroke.
    fn act(&mut self, op: Op) -> EventId {
        let id = self.log.append(self.replica, &mut self.seq, op.clone());
        self.take(id, &op);
        id
    }

    fn take(&mut self, id: EventId, op: &Op) {
        let before = self.weave.clone();
        let edits = self.weave.apply(id, op);
        self.syntax.follow(&before, &self.weave, &edits);
    }

    fn type_at(&mut self, offset: usize, text: &str) {
        let op = self.weave.insert_op(Chars(offset), text);
        self.act(op);
    }

    fn delete(&mut self, range: std::ops::Range<usize>) {
        for op in self.weave.delete_ops(Chars(range.start)..Chars(range.end)) {
            self.act(op);
        }
    }

    /// Pull everything `other` has, applying it in id order (causal).
    fn pull(&mut self, other: &Desk) {
        let incoming = sync::missing(&other.log, &sync::state_vector(&self.log));
        assert!(sync::integrate(&mut self.log, incoming.clone()).is_empty());
        for e in incoming {
            self.take(e.id, &e.op);
        }
    }

    fn find(&self, needle: &str) -> usize {
        let text = self.weave.text();
        text[..text.find(needle).unwrap_or_else(|| panic!("{needle:?} in {text:?}"))].chars().count()
    }

    fn sexp(&self) -> String {
        self.syntax.tree().root_node().to_sexp()
    }

    fn fresh_sexp(&self, grammar: Grammar) -> String {
        Syntax::of(&self.weave, grammar).tree().root_node().to_sexp()
    }
}

/// Pieces of Rust and of Markdown, some of them not ASCII, to cut and paste
/// into something that sometimes parses and mostly does not.
fn piece() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("fn "), Just("a"), Just("bé"), Just("("), Just(")"), Just(" {"), Just("}"), Just("let "), Just(" = "),
        Just("1"), Just(";"), Just("\n"), Just("\r\n"), Just(" "), Just("# "), Just("- "), Just("\"😀\""), Just("// c"),
    ]
}

/// One step of a session: type a piece, delete a stretch, or move a run to
/// the front of the document. Positions are taken modulo what is there.
#[derive(Clone, Debug)]
enum Step {
    Type(usize, &'static str),
    Delete(usize, usize),
    Move(usize),
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    let step = prop_oneof![
        4 => (0usize..400, piece()).prop_map(|(at, p)| Step::Type(at, p)),
        2 => (0usize..400, 1usize..6).prop_map(|(at, n)| Step::Delete(at, n)),
        1 => (0usize..400).prop_map(Step::Move),
    ];
    proptest::collection::vec(step, 1..40)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// After any sequence of edits -- inserts, deletes, moves -- the tree
    /// followed incrementally equals a fresh parse of the weave's text, for
    /// both grammars; and the weave's three reading functions agree with
    /// the text they read.
    #[test]
    fn a_followed_tree_equals_a_fresh_parse(steps in steps(), markdown in any::<bool>()) {
        let grammar = if markdown { Grammar::Markdown } else { Grammar::Rust };
        let mut desk = Desk::new("fn main() {\n    let a = 1;\n}\n", grammar);
        for step in steps {
            let len = desk.weave.metrics().chars;
            match step {
                Step::Type(at, text) => desk.type_at(at % (len + 1), text),
                Step::Delete(at, n) if len > 0 => {
                    let at = at % len;
                    desk.delete(at..(at + n).min(len));
                }
                Step::Move(at) if len > 0 => {
                    let run = desk.weave.pos_at(Chars(at % len), Bias::Right).expect("in range").event;
                    desk.act(Op::MoveRun { target: run, parent: Anchor::DocStart(desk.node), side: Side::Right });
                }
                _ => {}
            }
            prop_assert_eq!(desk.sexp(), desk.fresh_sexp(grammar), "after {:?}", desk.weave.text());
        }

        // The oracle for the weave's own part: the text.
        let text = desk.weave.text();
        let mut read = String::new();
        while read.len() < text.len() {
            let chunk = desk.weave.chunk_at(read.len());
            prop_assert!(!chunk.is_empty());
            read.push_str(chunk);
        }
        prop_assert_eq!(&read, &text);
        prop_assert_eq!(desk.weave.chunk_at(text.len()), "");
        for (n, (byte, _)) in text.char_indices().chain([(text.len(), ' ')]).enumerate() {
            prop_assert_eq!(desk.weave.metrics_at(Chars(n)), Metrics::of(&text[..byte]));
            prop_assert_eq!(desk.weave.chars_at_byte(byte), Chars(n));
        }
    }
}

/// A ref names the same node through edits around it and inside it, and
/// says so when the node no longer parses as one.
#[test]
fn a_node_ref_survives_edits_around_its_node() {
    let mut desk = Desk::new("fn a() {}\nfn b() { 1 }\n", Grammar::Rust);
    let at = desk.find("fn b");
    let b = desk.syntax.node_at(&desk.weave, Chars(at)..Chars(at + 12)).expect("a node");
    assert_eq!(desk.syntax.kind(&desk.weave, b), Some("function_item"));

    // Before it, after it, and inside it: the node moves and grows.
    desk.type_at(0, "// a comment\nfn zero() {}\n");
    let end = desk.weave.metrics().chars;
    desk.type_at(end, "fn c() {}\n");
    let inside = desk.find("1 }");
    desk.type_at(inside, "0 + ");
    assert_eq!(desk.syntax.kind(&desk.weave, b), Some("function_item"));
    let node = desk.syntax.resolve(&desk.weave, b).unwrap();
    assert_eq!(&desk.weave.text()[node.byte_range()], "fn b() { 0 + 1 }");
    assert_eq!(desk.sexp(), desk.fresh_sexp(Grammar::Rust));

    // Its keyword goes: what is left between the anchors is no node.
    let at = desk.find("fn b");
    desk.delete(at..at + 3);
    assert_eq!(desk.syntax.resolve(&desk.weave, b).map(|n| n.kind()), None, "{}", desk.sexp());
    // And comes back, typed anew. Fugue hangs the new text on the `b`, after
    // the tombstones of the old keyword, so it lies between the anchors: the
    // ref names the function again.
    let at = desk.find("b() {");
    desk.type_at(at, "fn ");
    assert_eq!(desk.syntax.kind(&desk.weave, b), Some("function_item"));
}

/// References by scope: shadowing, parameters, paths and fields.
#[test]
fn references_follow_scope_not_spelling() {
    let src = "\
fn foo() {}
mod other { pub fn foo() {} }
fn main() {
    foo();
    other::foo();
    let foo = 1;
    let s = S { foo };
    s.foo;
    { let foo = foo + 1; foo; }
    println!(\"{}\", foo);
    let f = |foo: u8| foo;
}
fn later(foo: u8) -> u8 { foo }
";
    let desk = Desk::new(src, Grammar::Rust);
    let refs = |needle: &str| -> Vec<usize> {
        let decl = desk.weave.pos_at(Chars(desk.find(needle) + needle.find("foo").unwrap()), Bias::Right).unwrap();
        desk.syntax.references(&desk.weave, decl).iter().map(|r| r.start.0).collect()
    };
    let at = |needle: &str| desk.find(needle) + needle.find("foo").unwrap();
    assert_eq!(refs("fn foo() {}\nmod"), [at("fn foo() {}\nmod"), at("    foo();")], "the function: itself and its one call");
    assert_eq!(
        refs("let foo = 1"),
        [at("let foo = 1"), at("S { foo }"), at("= foo + 1"), at("\"{}\", foo")],
        "the outer let: not the path, not the field, not the inner block's own foo"
    );
    assert_eq!(refs("let foo = foo + 1"), [at("let foo = foo + 1"), at("foo; }")]);
    assert_eq!(refs("|foo: u8|"), [at("|foo: u8|"), at("| foo;")]);
    assert_eq!(refs("later(foo"), [at("later(foo"), at("{ foo }\n")]);
    let not_a_decl = desk.weave.pos_at(Chars(at("    foo();")), Bias::Right).unwrap();
    assert!(desk.syntax.references(&desk.weave, not_a_decl).is_empty(), "a use declares nothing");
}

/// The acceptance test for intents: Alice renames `foo` to `bar` everywhere;
/// Bob, concurrently, adds a call to `foo`. After the merge Bob's call reads
/// `bar`, on both replicas, and merging the same histories twice mints no
/// second rename of it.
#[test]
fn a_rename_reaches_a_reference_added_concurrently() {
    let mut alice = Desk::new("fn foo() {}\nfn main() {\n    foo();\n}\n", Grammar::Rust);
    let mut bob = alice.fork(2, Grammar::Rust);

    // Alice: the intent, then its effect on what she can see.
    let decl = alice.weave.pos_at(Chars(alice.find("foo")), Bias::Right).unwrap();
    let effect = syntax::rename(&alice.weave, &alice.syntax, decl, "bar");
    assert_eq!(effect.len(), 4, "two references, an insert and a delete each: {effect:?}");
    let intent = alice.act(Op::Rename { decl, to: "bar".into() });
    for op in effect {
        alice.act(op);
    }
    assert_eq!(alice.weave.text(), "fn bar() {}\nfn main() {\n    bar();\n}\n");
    assert!(syntax::reapply(&alice.weave, &alice.syntax, &alice.log, intent).is_empty(), "nothing she had not seen");

    // Bob, meanwhile: one more call, and a `foo` of his own that is not hers.
    let end = bob.find("    foo();\n") + "    foo();\n".chars().count();
    bob.type_at(end, "    foo();\n    { let foo = 2; foo; }\n");

    alice.pull(&bob);
    bob.pull(&alice);
    let merged = "fn bar() {}\nfn main() {\n    bar();\n    foo();\n    { let foo = 2; foo; }\n}\n";
    assert_eq!(alice.weave.text(), merged, "the character merge alone leaves Bob's call stale");
    assert_eq!(bob.weave.text(), merged);

    let from_alice = syntax::reapply(&alice.weave, &alice.syntax, &alice.log, intent);
    let from_bob = syntax::reapply(&bob.weave, &bob.syntax, &bob.log, intent);
    assert_eq!(from_alice, from_bob, "derived, so the same on every replica: ids, parents, ops");
    assert_eq!(from_alice.len(), 2, "one stale reference: {from_alice:?}");
    for desk in [&mut alice, &mut bob] {
        assert!(sync::integrate(&mut desk.log, from_alice.clone()).is_empty(), "Lamport-valid, within limits");
        for e in &from_alice {
            desk.take(e.id, &e.op);
        }
        let want = "fn bar() {}\nfn main() {\n    bar();\n    bar();\n    { let foo = 2; foo; }\n}\n";
        assert_eq!(desk.weave.text(), want, "Bob's call reads bar; his own foo is his");
        assert_eq!(desk.sexp(), desk.fresh_sexp(Grammar::Rust));
        assert!(syntax::reapply(&desk.weave, &desk.syntax, &desk.log, intent).is_empty(), "and nothing is left to do");
    }

    // The same merge again, by the other side's events: they are the same
    // events, so the log takes none of them.
    let before = alice.log.events().len();
    sync::integrate(&mut alice.log, from_bob);
    assert_eq!(alice.log.events().len(), before, "no second rename");
    // And replay agrees with the live weaves: the derived events are
    // ordinary events.
    let all: Vec<EventId> = alice.log.events().keys().copied().collect();
    assert_eq!(replay::materialise_events(&all, &alice.log).files["f"], alice.weave.text());
}
