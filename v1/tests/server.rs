//! `v1 lsp` as an editor sees it: a fake LSP client over stdio, behaving as
//! measured on Helix 25.07 (an applied edit is echoed as a `didChange`
//! before the response), against the real binary and a real rust-analyzer.
//! A missing server fails the test; it never skips.
//!
//! The client counts columns in utf-8, which `v1 lsp` chooses when it is
//! offered; the tests' text is ASCII, so bytes are columns.

use crossbeam_channel::{Receiver, RecvTimeoutError, unbounded};
use lsp_server::{Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

/// A project on disk, removed again when dropped; `dir/s` is the socket.
struct Project {
    root: PathBuf,
    doc: PathBuf,
    socket: PathBuf,
}

impl Project {
    fn rust(name: &str, text: &str) -> Project {
        let manifest = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
        Project::new(name, &[("Cargo.toml", &manifest), ("src/main.rs", text)], "src/main.rs")
    }

    /// `files` under a fresh directory; `doc` is the one the editor opens.
    fn new(name: &str, files: &[(&str, &str)], doc: &str) -> Project {
        let id = std::process::id();
        let root = std::env::temp_dir().join(format!("v1-server-{id}-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        // A Unix socket's path is short: not under the project.
        let socket = PathBuf::from(format!("/tmp/v1-server-{id}-{}", name.split('-').next().unwrap())).join("s");
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let root = root.canonicalize().unwrap();
        Project { doc: root.join(doc), root, socket }
    }

    /// Meet `host` on its socket.
    fn joining(mut self, host: &Project) -> Project {
        self.socket = host.socket.clone();
        self
    }

    fn file(&self) -> PathBuf {
        self.doc.clone()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(self.socket.parent().unwrap());
    }
}

/// The editor: a buffer, a version, and the server on its pipes.
struct Client {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Message>,
    next: i32,
    uri: String,
    buffer: String,
    version: i32,
    notes: Vec<Notification>,
}

fn uri_of(path: &Path) -> String {
    format!("file://{}", path.display())
}

impl Client {
    /// Start `v1 lsp` for `p`'s document, initialize, and open it with
    /// `text`. Returns the client and the initialize result.
    fn open(p: &Project, text: &str) -> (Client, Value) {
        Client::open_with(p, &["rust-analyzer"], "rust", text)
    }

    /// [`Client::open`], with `command` as the language server of
    /// `language_id`.
    fn open_with(p: &Project, command: &[&str], language_id: &str, text: &str) -> (Client, Value) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_v1"))
            .args(["lsp", "--socket"])
            .arg(&p.socket)
            .arg("--")
            .args(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("v1 starts");
        let stdin = child.stdin.take().unwrap();
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = unbounded();
        std::thread::spawn(move || {
            while let Ok(Some(m)) = Message::read(&mut out) {
                if tx.send(m).is_err() {
                    break;
                }
            }
        });
        let mut c = Client { child, stdin, rx, next: 1, uri: uri_of(&p.file()), buffer: text.to_owned(), version: 1, notes: vec![] };
        let root = uri_of(&p.root);
        let init = c
            .call(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root,
                    "workspaceFolders": [{"uri": root, "name": "p"}],
                    "capabilities": {"general": {"positionEncodings": ["utf-8", "utf-32", "utf-16"]}},
                }),
            )
            .expect("initialize");
        c.send(Notification::new("initialized".into(), json!({})));
        let doc = json!({"uri": c.uri, "languageId": language_id, "version": 1, "text": text});
        c.send(Notification::new("textDocument/didOpen".into(), json!({"textDocument": doc})));
        (c, init)
    }

    fn send(&mut self, m: impl Into<Message>) {
        m.into().write(&mut self.stdin).expect("the server is there");
    }

    /// A request, and its result or error message; the server's own
    /// requests that come meanwhile are answered.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = RequestId::from(self.next);
        self.next += 1;
        self.send(Request::new(id.clone(), method.into(), params));
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            match self.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Message::Response(r)) if r.id == id => return r.response_result.map_err(|e| format!("{} ({})", e.message, e.code)),
                Ok(m) => self.incoming(m),
                Err(e) => panic!("{method}: no answer: {e}"),
            }
        }
    }

    /// Take what the server sent. An `applyEdit` is applied as Helix does:
    /// refused when it names another version; else applied, echoed as a
    /// `didChange` at the next version, and answered.
    fn incoming(&mut self, m: Message) {
        match m {
            Message::Request(r) if r.method == "workspace/applyEdit" => {
                let change = &r.params["edit"]["documentChanges"][0];
                assert_eq!(change["textDocument"]["uri"], self.uri.as_str());
                let current = change["textDocument"]["version"].as_i64() == Some(self.version.into());
                if current {
                    for e in change["edits"].as_array().unwrap() {
                        let (a, b) = (self.byte(&e["range"]["start"]), self.byte(&e["range"]["end"]));
                        self.buffer.replace_range(a..b, e["newText"].as_str().unwrap());
                        self.version += 1;
                        let ch = json!({"range": e["range"], "text": e["newText"]});
                        let doc = json!({"uri": self.uri, "version": self.version});
                        self.send(Notification::new("textDocument/didChange".into(), json!({"textDocument": doc, "contentChanges": [ch]})));
                    }
                }
                self.send(Response::new_ok(r.id, json!({"applied": current})));
            }
            Message::Request(r) => self.send(Response::new_err(r.id, -32601, format!("{}: not supported", r.method))),
            Message::Notification(n) => self.notes.push(n),
            Message::Response(_) => {}
        }
    }

    /// Keep listening until `done`, at most `secs`.
    fn until(&mut self, what: &str, secs: u64, done: impl Fn(&Client) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while !done(self) {
            match self.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(m) => self.incoming(m),
                Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(e) => panic!("waiting for {what}: {e}; the buffer reads {:?}", self.buffer),
            }
        }
    }

    fn byte(&self, p: &Value) -> usize {
        let (line, col) = (p["line"].as_u64().unwrap() as usize, p["character"].as_u64().unwrap() as usize);
        self.buffer.split_inclusive('\n').take(line).map(str::len).sum::<usize>() + col
    }

    fn position(&self, at: usize) -> Value {
        let before = &self.buffer[..at];
        let start = before.rfind('\n').map_or(0, |i| i + 1);
        json!({"line": before.matches('\n').count(), "character": at - start})
    }

    /// The range of the last `needle` in the buffer.
    fn range(&self, needle: &str) -> Value {
        let at = self.buffer.rfind(needle).unwrap_or_else(|| panic!("{needle:?} in {:?}", self.buffer));
        json!({"start": self.position(at), "end": self.position(at + needle.len())})
    }

    /// Type `text` at byte `at`.
    fn type_(&mut self, at: usize, text: &str) {
        let p = self.position(at);
        self.buffer.insert_str(at, text);
        self.version += 1;
        let ch = json!({"range": {"start": p, "end": p}, "text": text});
        let doc = json!({"uri": self.uri, "version": self.version});
        self.send(Notification::new("textDocument/didChange".into(), json!({"textDocument": doc, "contentChanges": [ch]})));
    }

    fn doc(&self) -> Value {
        json!({"uri": self.uri})
    }

    fn command(&mut self, command: &str, arguments: Value) -> Result<Value, String> {
        self.call("workspace/executeCommand", json!({"command": command, "arguments": arguments}))
    }

    /// `shutdown` and `exit`: the exit status.
    fn quit(mut self) -> std::process::ExitStatus {
        self.call("shutdown", Value::Null).expect("shutdown");
        self.send(Notification::new("exit".into(), Value::Null));
        self.child.wait().unwrap()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const PROGRAM: &str = "fn foo() {}\nfn main() {\n    foo();\n}\n";

/// A rename is an intent: the request is answered with nothing, and the
/// effect arrives as a push. Hover passes through with its range in the
/// client's columns. The server leaves cleanly and takes its socket.
#[test]
fn a_rename_answers_empty_and_arrives_as_a_push() {
    let p = Project::rust("rename", PROGRAM);
    let (mut c, init) = Client::open(&p, PROGRAM);
    assert_eq!(init["capabilities"]["positionEncoding"], "utf-8");
    let call = c.range("foo()");
    // Hover, passed through: its range is the call's name.
    let hover = c.call("textDocument/hover", json!({"textDocument": c.doc(), "position": call["start"]})).unwrap();
    let name = json!({"start": call["start"], "end": {"line": 2, "character": 7}});
    assert_eq!(hover["range"], name, "{hover}");
    assert!(hover["contents"].to_string().contains("foo"), "{hover}");
    // Definition: the declaration, in this document.
    let def = c.call("textDocument/definition", json!({"textDocument": c.doc(), "position": call["start"]})).unwrap();
    let def = serde_json::to_string(&def).unwrap();
    assert!(def.contains(r#""start":{"character":3,"line":0}"#), "{def}");

    let answer = c.call("textDocument/rename", json!({"textDocument": c.doc(), "position": call["start"], "newName": "bar"}));
    assert_eq!(answer, Ok(json!({})));
    c.until("the rename", 60, |c| !c.buffer.contains("foo"));
    assert_eq!(c.buffer, "fn bar() {}\nfn main() {\n    bar();\n}\n");

    // A rename of nothing is refused with a reason, not applied.
    let nothing = json!({"line": 1, "character": 8});
    let refused = c.call("textDocument/rename", json!({"textDocument": c.doc(), "position": nothing, "newName": "x"}));
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(c.buffer, "fn bar() {}\nfn main() {\n    bar();\n}\n");

    let socket = p.socket.clone();
    assert!(socket.exists(), "the host listens");
    assert!(c.quit().success());
    assert!(!socket.exists(), "the host removes its socket");
}

/// Code actions are listed with their edits stripped, each a command
/// `v1.intent`; running one is an intent, and its effect is a push.
#[test]
fn a_code_action_is_listed_as_a_command_and_runs_as_an_intent() {
    let text = "fn main() {\n    let x = 1 + 2 * 3;\n    println!(\"{}\", x);\n}\n";
    let p = Project::rust("action", text);
    let (mut c, init) = Client::open(&p, text);
    assert!(init["capabilities"]["executeCommandProvider"]["commands"].to_string().contains("v1.intent"));
    let range = c.range("2 * 3");
    let listed = c.call("textDocument/codeAction", json!({"textDocument": c.doc(), "range": range, "context": {"diagnostics": []}}));
    let listed = listed.unwrap();
    let actions = listed.as_array().unwrap();
    assert!(!actions.is_empty());
    for a in actions {
        assert!(a.get("edit").is_none(), "{a}");
        assert_eq!(a["command"]["command"], "v1.intent", "{a}");
        assert_eq!(a["command"]["arguments"][2], range, "{a}");
    }
    let extract = actions.iter().find(|a| a["title"] == "Extract into variable").expect("extract into variable");
    let (name, args) = (extract["command"]["command"].as_str().unwrap(), extract["command"]["arguments"].clone());
    assert_eq!(c.command(name, args), Ok(Value::Null));
    c.until("the action", 60, |c| c.buffer.contains("let var_name = 2 * 3;"));
}

/// `textDocument/formatting` is an intent too: answered `[]`, effect pushed.
#[test]
fn a_format_answers_empty_and_arrives_as_a_push() {
    let text = "fn main() {\nlet   a=1;\n}\n";
    let p = Project::rust("format", text);
    let (mut c, _) = Client::open(&p, text);
    let answer = c.call("textDocument/formatting", json!({"textDocument": c.doc(), "options": {"tabSize": 4, "insertSpaces": true}}));
    assert_eq!(answer, Ok(json!([])));
    c.until("the format", 60, |c| c.buffer.contains("let a = 1;"));
    assert_eq!(c.buffer, "fn main() {\n    let a = 1;\n}\n");
}

/// Diagnostics of the backend reach the client, in the client's columns.
/// The error is one of syntax: rust-analyzer's diagnostics of types depend
/// on the crate graph, and whether a second publish follows the first,
/// made before the graph is built, is a race of its own (a type mismatch
/// came in about two runs of three, and never later than a few seconds,
/// so waiting longer did not help; measured 2026-11-11).
#[test]
fn the_backends_diagnostics_reach_the_client() {
    let text = "fn main() {\n    let x = ;\n}\n";
    let p = Project::rust("diag", text);
    let (mut c, _) = Client::open(&p, text);
    c.until("diagnostics", 90, |c| {
        c.notes.iter().any(|n| n.method == "textDocument/publishDiagnostics" && !n.params["diagnostics"].as_array().unwrap().is_empty())
    });
    let n = c.notes.iter().rev().find(|n| n.method == "textDocument/publishDiagnostics").unwrap();
    assert_eq!(n.params["uri"], c.uri.as_str());
    let found = n.params["diagnostics"].as_array().unwrap().iter().any(|d| d["range"]["start"]["line"] == 1);
    assert!(found, "{}", n.params);
}

/// Two clients on one socket: the guest's buffer is replaced by the
/// document; what one types the other has. Then both go offline: one
/// renames while the other adds a call, and when they are back the call
/// reads `bar` in both.
#[test]
fn a_guest_is_filled_and_a_held_rename_reaches_a_call_it_had_not_seen() {
    let (a, b) = (Project::rust("host", PROGRAM), Project::rust("guest", ""));
    let b = b.joining(&a);
    let (mut alice, _) = Client::open(&a, PROGRAM);
    // The guest joins once the host listens.
    let start = Instant::now();
    while !a.socket.exists() {
        assert!(start.elapsed() < Duration::from_secs(90), "the host does not listen");
        std::thread::sleep(Duration::from_millis(20));
    }
    let (mut bob, _) = Client::open(&b, "");
    bob.until("the fill", 60, |c| c.buffer == PROGRAM);

    alice.type_(0, "// alice\n");
    bob.until("alice's line", 30, |c| c.buffer.starts_with("// alice\n"));
    let end = bob.buffer.len();
    bob.type_(end, "// bob\n");
    alice.until("bob's line", 30, |c| c.buffer.ends_with("// bob\n"));
    assert_eq!(alice.buffer, bob.buffer);

    assert_eq!(alice.command("v1.hold", json!([])), Ok(Value::Null));
    assert_eq!(bob.command("v1.hold", json!([])), Ok(Value::Null));
    let call = alice.range("foo()");
    let answer = alice.call("textDocument/rename", json!({"textDocument": alice.doc(), "position": call["start"], "newName": "bar"}));
    assert_eq!(answer, Ok(json!({})));
    alice.until("her own rename", 60, |c| !c.buffer.contains("foo"));
    let at = bob.buffer.find("    foo();\n").unwrap() + "    foo();\n".len();
    bob.type_(at, "    foo();\n");
    assert!(bob.buffer.contains("foo"), "apart: bob has not seen the rename");

    assert_eq!(alice.command("v1.release", json!([])), Ok(Value::Null));
    assert_eq!(bob.command("v1.release", json!([])), Ok(Value::Null));
    let want = "// alice\nfn bar() {}\nfn main() {\n    bar();\n    bar();\n}\n// bob\n";
    alice.until("the merge", 60, |c| c.buffer == want);
    bob.until("the merge", 60, |c| c.buffer == want);
}

/// A definition in another file is in that file's columns, which the
/// backend counts its own way: the client counts utf-8 and basedpyright
/// utf-16, and `é` is two bytes and one unit. The position is converted by
/// reading the file; the client's own file is not affected.
#[test]
fn a_definition_in_another_file_is_in_the_clients_columns() {
    let main = "from lib import foo\nprint(foo)\n";
    let lib = "\u{e9} = 1; foo = 2\n";
    let p = Project::new("foreign", &[("main.py", main), ("lib.py", lib)], "main.py");
    let (mut c, init) = Client::open_with(&p, &["basedpyright-langserver", "--stdio"], "python", main);
    assert_eq!(init["capabilities"]["positionEncoding"], "utf-8");
    let at = json!({"line": 1, "character": 7});
    let def = c.call("textDocument/definition", json!({"textDocument": c.doc(), "position": at})).unwrap();
    let def = def.as_array().map_or(vec![def.clone()], Clone::clone);
    let range = |l: &Value| l.get("targetSelectionRange").or(l.get("range")).cloned().unwrap();
    let uri = |l: &Value| l.get("targetUri").or(l.get("uri")).and_then(Value::as_str).unwrap().to_owned();
    let found = def.iter().find(|l| uri(l).ends_with("lib.py")).unwrap_or_else(|| panic!("{def:?}"));
    let col = lib.find("foo").unwrap();
    assert_eq!(range(found)["start"], json!({"line": 0, "character": col}), "{found}");
    assert_eq!(range(found)["end"], json!({"line": 0, "character": col + 3}), "{found}");
}
