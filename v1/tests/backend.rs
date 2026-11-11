//! The backend against real language servers: rust-analyzer (in `pkg env
//! rust`) and basedpyright (`nix shell nixpkgs#basedpyright`). No mocks, and
//! a missing server fails the test; it never skips.
//!
//! Each test makes a project under the temp directory, starts a server in
//! it, and lets two [`Session`]s resolve with it, sharing the one server:
//! every question sets the document's text first, so sharing is sound.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use v0::event::Event;
use v0::live::{FromEditor, Patch, Session, ToEditor, Unit};
use v0::op::ReplicaId;
use v0::syntax::Grammar;
use v1::backend::{Backend, LspResolver, file_uri};

/// A project on disk, removed again when dropped.
struct Project {
    root: PathBuf,
    file: PathBuf,
}

impl Project {
    /// `files` are written under a fresh directory; `file` is the document.
    fn new(name: &str, files: &[(&str, &str)], file: &str) -> Project {
        let root = std::env::temp_dir().join(format!("v1-backend-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let root = root.canonicalize().unwrap();
        Project { file: root.join(file), root }
    }

    fn rust(name: &str, text: &str) -> Project {
        let manifest = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
        Project::new(name, &[("Cargo.toml", &manifest), ("src/main.rs", text)], "src/main.rs")
    }

    fn python(name: &str, text: &str) -> Project {
        Project::new(name, &[("main.py", text)], "main.py")
    }

    fn backend(&self, command: &[&str], language_id: &str, text: &str) -> Arc<Mutex<Backend>> {
        let command: Vec<String> = command.iter().map(|s| s.to_string()).collect();
        let b = Backend::spawn(&command, &self.root, file_uri(&self.file), language_id, text).expect("the server starts");
        Arc::new(Mutex::new(b))
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn resolver(b: &Arc<Mutex<Backend>>) -> Box<LspResolver> {
    Box::new(LspResolver { backend: b.clone() })
}

/// An editor's buffer, its session, and the pipes (characters, like Emacs).
struct Seat {
    buffer: String,
    sent: u64,
    seen: u64,
    session: Session,
    to_session: VecDeque<FromEditor>,
    to_editor: VecDeque<ToEditor>,
    to_peer: VecDeque<Vec<Event>>,
}

impl Seat {
    fn new(session: Session) -> Seat {
        let buffer = session.text();
        Seat { buffer, sent: 0, seen: 0, session, to_session: Default::default(), to_editor: Default::default(), to_peer: Default::default() }
    }

    fn byte(&self, units: usize) -> usize {
        self.buffer.chars().take(units).map(char::len_utf8).sum()
    }

    fn chars_before(&self, needle: &str) -> usize {
        self.buffer[..self.buffer.rfind(needle).unwrap_or_else(|| panic!("{needle:?} in {:?}", self.buffer))].chars().count()
    }

    fn type_(&mut self, at: usize, del: usize, text: &str) {
        let p = Patch { at, del, text: text.to_owned() };
        self.splice(&p);
        self.sent += 1;
        self.to_session.push_back(FromEditor::Edit { at, del, text: p.text, seen: self.seen });
    }

    /// Type `text` just before the last `needle`.
    fn type_before(&mut self, needle: &str, text: &str) {
        self.type_(self.chars_before(needle), 0, text);
    }

    /// Rename the name under the last `needle`, `inside` chars into it.
    fn rename(&mut self, needle: &str, inside: usize, to: &str) {
        let at = self.chars_before(needle) + inside;
        self.sent += 1;
        self.to_session.push_back(FromEditor::Rename { at, to: to.to_owned(), seen: self.seen });
    }

    fn format(&mut self, needle: &str) {
        let at = self.chars_before(needle);
        self.sent += 1;
        let end = at + needle.chars().count();
        self.to_session.push_back(FromEditor::Format { at, end, seen: self.seen });
    }

    fn action(&mut self, needle: &str, kind: &str, title: &str) {
        let at = self.chars_before(needle);
        self.sent += 1;
        let end = at + needle.chars().count();
        let (kind, title) = (kind.to_owned(), title.to_owned());
        self.to_session.push_back(FromEditor::Action { at, end, kind, title, seen: self.seen });
    }

    fn splice(&mut self, p: &Patch) {
        let (a, b) = (self.byte(p.at), self.byte(p.at + p.del));
        self.buffer.replace_range(a..b, &p.text);
    }

    fn editor_reads(&mut self) -> bool {
        let Some(msg) = self.to_editor.pop_front() else { return false };
        match msg {
            ToEditor::Apply { n, acked, edits } if acked == self.sent => {
                edits.iter().for_each(|p| self.splice(p));
                self.seen = n;
                self.to_session.push_back(FromEditor::Seen(n));
            }
            ToEditor::Apply { .. } => {}
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
    if !out.send.is_empty() {
        to.to_peer.push_back(out.send);
    }
    to.to_editor.extend(out.reply);
    true
}

fn settle(a: &mut Seat, b: &mut Seat) {
    while a.session_reads() | b.session_reads() | a.editor_reads() | b.editor_reads() | deliver(a, b) | deliver(b, a) {}
}

/// Alice hosts, Bob joins; both on `text`, both resolving with `backend`.
fn pair(text: &str, grammar: Grammar, name: &str, backend: &Arc<Mutex<Backend>>) -> (Seat, Seat) {
    let host = Session::host(ReplicaId(1), grammar, Unit::Chars, name).with_resolver(resolver(backend));
    let mut alice = Seat::new(host);
    alice.type_(0, 0, text);
    alice.session_reads();
    let bob = Session::guest(ReplicaId(2), grammar, Unit::Chars, alice.session.events()).unwrap();
    alice.to_peer.clear();
    (alice, Seat::new(bob.with_resolver(resolver(backend))))
}

/// Both go offline, `alice` and `bob` type, both come back, everything
/// settles, and the two read alike.
fn concurrently(alice: &mut Seat, bob: &mut Seat, a: impl FnOnce(&mut Seat), b: impl FnOnce(&mut Seat)) {
    alice.say(FromEditor::Hold(true));
    bob.say(FromEditor::Hold(true));
    a(alice);
    b(bob);
    settle(alice, bob);
    alice.say(FromEditor::Hold(false));
    bob.say(FromEditor::Hold(false));
    settle(alice, bob);
    assert_eq!(alice.buffer, alice.session.text(), "Alice's buffer is her session's document");
    assert_eq!(bob.buffer, bob.session.text(), "Bob's buffer is his session's document");
    assert_eq!(alice.buffer, bob.buffer, "the replicas converge");
    let fresh = Session::guest(ReplicaId(9), Grammar::Rust, Unit::Chars, alice.session.events());
    assert_eq!(fresh.map(|s| s.text()), Ok(alice.buffer.clone()), "and a newcomer reads the same from the events alone");
}

/// Shut the server down, once the sessions that shared it are gone.
fn stop(backend: Arc<Mutex<Backend>>) {
    Arc::into_inner(backend).expect("the sessions are dropped").into_inner().unwrap().stop();
}

const RUST: &[&str] = &["rust-analyzer"];
const PYTHON: &[&str] = &["basedpyright-langserver", "--stdio"];

/// A document with characters of every width before the names, so a
/// position in the server's encoding is not a byte or a char offset.
const WIDE: &str = "// h\u{e9}llo \u{2192} \u{1f642}\nfn foo() -> i32 { 1 }\nfn main() {\n    let a = foo();\n    let foo = 2;\n    let _ = (a, foo);\n}\n";

/// Positions round-trip through the server's encoding, and a definition
/// found from a call and from the declaration is the same place.
#[test]
fn locate_agrees_from_a_call_and_from_the_declaration() {
    let p = Project::rust("locate", WIDE);
    let backend = p.backend(RUST, "rust", WIDE);
    let mut guard = backend.lock().unwrap();
    let b = &mut *guard;
    for (at, _) in WIDE.char_indices() {
        assert_eq!(b.offset(b.position(at)), Some(at), "byte {at}");
    }
    let decl = WIDE.find("foo()").unwrap();
    let call = WIDE.rfind("foo()").unwrap();
    let from_call = b.definition(call + 1).unwrap();
    let from_decl = b.definition(decl).unwrap();
    assert_eq!(from_call, vec![decl..decl + 3]);
    assert_eq!(from_call, from_decl);
    // The shadowing `let foo` is another thing: it points at itself.
    let local = WIDE.find("let foo").unwrap() + 4;
    assert_eq!(b.definition(local).unwrap(), vec![local..local + 3]);
    assert!(b.definition(WIDE.find("fn main").unwrap() + 3).unwrap().len() == 1);
    drop(guard);

    // And through the resolver, a session renames the same from either.
    let result = |inside_of: &str, inside: usize| {
        let (mut a, _) = pair(WIDE, Grammar::Rust, "f.rs", &backend);
        a.rename(inside_of, inside, "bar");
        a.session_reads();
        a.editor_reads();
        a.buffer
    };
    let want = WIDE.replace("fn foo", "fn bar").replace("a = foo()", "a = bar()");
    assert_eq!(result("foo() ->", 1), want, "from the declaration");
    assert_eq!(result("foo();", 2), want, "from the call");
    stop(backend);
}

/// The acceptance scene. Alice renames `foo` to `bar`; Bob, meanwhile, adds
/// a call to `foo` and a `let foo = 1; foo` of his own. Bob's call reads
/// `bar`; his own `foo` is his.
#[test]
fn a_rename_reaches_a_call_added_concurrently_and_not_a_shadowing_let() {
    let text = "fn foo() {}\nfn main() {\n    foo();\n}\n";
    let p = Project::rust("rename", text);
    let backend = p.backend(RUST, "rust", text);
    let (mut a, mut b) = pair(text, Grammar::Rust, "f.rs", &backend);
    concurrently(
        &mut a,
        &mut b,
        |a| a.rename("foo", 0, "bar"),
        |b| b.type_before("}\n", "    foo();\n    { let foo = 1; foo; }\n"),
    );
    let want = "fn bar() {}\nfn main() {\n    bar();\n    bar();\n    { let foo = 1; foo; }\n}\n";
    assert_eq!(a.buffer, want);
    let d = a.session.derivations();
    assert_eq!((d.gone, d.refused, d.capped), (0, 0, 0), "{d:?}");
    assert!(a.session.derive().send.is_empty() && b.session.derive().send.is_empty(), "nothing is left to derive");
    drop((a, b));
    stop(backend);
}

/// The same in Python: `def foo` renamed reaches a call added meanwhile; a
/// parameter `foo` of another function is not the function.
#[test]
fn a_python_rename_reaches_a_call_added_concurrently() {
    let text = "def foo():\n    return 1\n\n\ndef other(foo):\n    return foo\n\n\nx = foo()\n";
    let p = Project::python("python", text);
    let backend = p.backend(PYTHON, "python", text);
    let (mut a, mut b) = pair(text, Grammar::Python, "f.py", &backend);
    let late = "\n\ndef late():\n    return foo()\n";
    concurrently(&mut a, &mut b, |a| a.rename("foo", 0, "bar"), |b| b.type_(b.buffer.chars().count(), 0, late));
    let want = "def bar():\n    return 1\n\n\ndef other(foo):\n    return foo\n\n\nx = bar()\n\n\ndef late():\n    return bar()\n";
    assert_eq!(a.buffer, want);
    let d = a.session.derivations();
    assert_eq!((d.gone, d.refused, d.capped), (0, 0, 0), "{d:?}");
    drop((a, b));
    stop(backend);
}

/// Alice formats a range; Bob, meanwhile, types a badly formatted line
/// inside it. It comes out formatted, and what is outside the range is not
/// touched.
#[test]
fn a_format_reaches_a_line_typed_inside_its_range() {
    let text = "fn main() {\nlet   a=1;\nlet   b  =  2;\nlet c=3;\n}\n";
    let p = Project::rust("format", text);
    let backend = p.backend(RUST, "rust", text);
    let (mut a, mut b) = pair(text, Grammar::Rust, "f.rs", &backend);
    concurrently(
        &mut a,
        &mut b,
        |a| a.format("let   a=1;\nlet   b  =  2;"),
        |b| b.type_before("let   b", "let   d=4;\n"),
    );
    let want = "fn main() {\n    let a = 1;\n    let d = 4;\n    let b = 2;\nlet c=3;\n}\n";
    assert_eq!(a.buffer, want);
    drop((a, b));
    stop(backend);
}

/// What identifies a code action across calls. Measured on rust-analyzer
/// 2026-06-01: see the assertions and `MEASURED` below.
#[test]
fn a_code_action_is_found_again_by_its_kind_and_title() {
    let text = "fn main() {\n    let x = 1 + 2 * 3;\n    println!(\"{}\", x);\n}\n";
    let p = Project::rust("actions", text);
    let backend = p.backend(RUST, "rust", text);
    let mut b = backend.lock().unwrap();
    let var = text.find("2 * 3").unwrap();
    let stmt = text.find("let x").unwrap();
    for (range, title) in [(var..var + 5, "Extract into variable"), (stmt..stmt + 18, "Extract into function")] {
        let first = b.actions(range.clone(), "").unwrap();
        // Another document version in between: the text is set again.
        b.set_text(&format!("{text}\n")).unwrap();
        b.set_text(text).unwrap();
        let second = b.actions(range.clone(), "").unwrap();
        let key = |a: &serde_json::Value| (a["kind"].as_str().unwrap_or("").to_owned(), a["title"].as_str().unwrap_or("").to_owned());
        let keys: Vec<_> = first.iter().map(key).collect();
        let again: Vec<_> = second.iter().map(key).collect();
        assert_eq!(keys, again, "the same actions, in the same order");
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "kind and title identify an action in a listing: {keys:?}");
        let data = |l: &[serde_json::Value]| l.iter().map(|a| a["data"].clone()).collect::<Vec<_>>();
        eprintln!("MEASURED {title}: keys {keys:?}\n  data first  {:?}\n  data second {:?}", data(&first), data(&second));
        let edits = b.action(range.clone(), "refactor.extract", title).unwrap();
        assert!(!edits.is_empty(), "{title}");
    }
    assert!(b.action(var..var + 5, "refactor.extract", "No such action").is_err());
    drop(b);
    stop(backend);
}

/// A code action as an intent, through a session: extract into variable.
#[test]
fn a_code_action_runs_as_an_intent() {
    let text = "fn main() {\n    let x = 1 + 2 * 3;\n    println!(\"{}\", x);\n}\n";
    let p = Project::rust("intent", text);
    let backend = p.backend(RUST, "rust", text);
    let (mut a, mut b) = pair(text, Grammar::Rust, "f.rs", &backend);
    concurrently(
        &mut a,
        &mut b,
        |a| a.action("2 * 3", "refactor.extract", "Extract into variable"),
        |b| b.type_before("println", "// note\n    "),
    );
    assert!(a.buffer.contains("let var_name = 2 * 3;") && a.buffer.contains("// note\n"), "{}", a.buffer);
    drop((a, b));
    stop(backend);
}

/// A rename that would touch another file is refused, not cut down.
#[test]
fn a_rename_across_files_is_refused() {
    let text = "mod other;\npub fn foo() {}\nfn main() {\n    foo();\n}\n";
    let manifest = "[package]\nname = \"cross\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    let other = "pub fn g() {\n    crate::foo();\n}\n";
    let files = [("Cargo.toml", manifest), ("src/main.rs", text), ("src/other.rs", other)];
    let p = Project::new("cross", &files, "src/main.rs");
    let backend = p.backend(RUST, "rust", text);
    let (mut a, _) = pair(text, Grammar::Rust, "f.rs", &backend);
    a.rename("foo", 0, "bar");
    a.session_reads();
    assert_eq!(a.session.text(), text, "nothing changed");
    let Some(ToEditor::Error(e)) = a.to_editor.pop_front() else { panic!("an error to the editor") };
    assert!(e.contains("cross-file"), "{e}");
    drop(a);
    stop(backend);
}
