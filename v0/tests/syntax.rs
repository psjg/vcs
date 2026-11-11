//! The structure layer against its oracles: a fresh parse, and the scope
//! walk's renames. How a rename reaches text it had not seen is a
//! session's, and tested there (`tests/live.rs`).

use proptest::prelude::*;
use v0::event::EventLog;
use v0::live::{FromEditor, Session, ToEditor, Unit};
use v0::op::{Anchor, EventId, NodeId, NodeKind, Op, ReplicaId, Side};
use v0::sumtree::Bias;
use v0::syntax::{Grammar, Syntax};
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

/// The text after renaming the `name` in the first `needle` of `src` to
/// `to`, the way an editor asks a session: what a wrong resolution shows up
/// in. A refused rename leaves `src`.
fn renamed(src: &str, needle: &str, name: &str, to: &str) -> String {
    let mut s = Session::host(ReplicaId(1), Grammar::Rust, Unit::Chars, "f.rs");
    s.local(FromEditor::Edit { at: 0, del: 0, text: src.into(), seen: 0 });
    let at = src[..src.find(needle).unwrap() + needle.find(name).unwrap()].chars().count();
    let out = s.local(FromEditor::Rename { at, to: to.into(), seen: 0 });
    match &out.reply[..] {
        [ToEditor::Error(_)] => assert_eq!(s.text(), src, "refused, so unchanged"),
        _ => assert!(!out.send.is_empty(), "recorded: {:?}", out.reply),
    }
    s.text()
}

/// Found in review: each of these renamed an identifier that did not mean
/// the declaration. One line per edge, pinned.
#[test]
fn a_rename_leaves_what_only_looks_like_a_reference() {
    // A struct pattern's shorthand binds a new `foo`.
    let src = "fn foo(){panic!()} struct S{foo:fn()} fn main(){let S{foo}=S{foo:||{}};foo();}";
    assert_eq!(renamed(src, "fn foo", "foo", "bar"), src.replacen("fn foo", "fn bar", 1));
    // The rest of a let chain sees the binding.
    let src = "fn main(){let x=0; if let Some(x)=Some(1) && x>0 {assert_eq!(x,1);}}";
    assert_eq!(renamed(src, "let x=0", "x", "y"), src.replacen("let x=0", "let y=0", 1));
    // A nested fn sees items, never the locals around it.
    let src = "const x:i32=2; fn main(){let x=1; fn inner()->i32{x}}";
    assert_eq!(renamed(src, "let x=1", "x", "y"), src.replacen("let x=1", "let y=1", 1));
    // An associated function is not in scope by its bare name: it claims no
    // call, and is itself not renamed here at all.
    let src = "fn foo()->i32{1} struct S; impl S{fn foo()->i32{2} fn call()->i32{foo()}}";
    assert_eq!(renamed(src, "impl S{fn foo", "foo", "bar"), src);
    assert_eq!(renamed(src, "fn foo()->i32{1}", "foo", "bar"), src.replacen("fn foo", "fn bar", 1).replace("{foo()}", "{bar()}"));
    // A type reached by path is someone else's.
    let src = "struct Foo; mod other{pub struct Foo;} fn f(_:other::Foo){}";
    assert_eq!(renamed(src, "struct Foo;", "Foo", "Bar"), src.replacen("struct Foo", "struct Bar", 1));
    // A generic parameter is its own declaration.
    let src = "type T=u8; fn f<T>(x:T)->T{x}";
    assert_eq!(renamed(src, "type T", "T", "U"), src.replacen("type T", "type U", 1));
    assert_eq!(renamed(src, "f<T>", "T", "U"), "type T=u8; fn f<U>(x:U)->U{x}");
}
