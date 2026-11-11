//! `v1 lsp --socket PATH -- SERVER ARGS...`: the language server an editor
//! is pointed at. Owns one live document (the first it opens with a
//! grammar), shares it with peers on PATH as `v0 live` does, and runs the
//! language's own SERVER for meaning (`v1::backend`). What it does not
//! take for itself is passed to SERVER, with positions converted.
//!
//! ```text
//! client <-stdio-> Doc { Front, Session } <-PATH-> peers
//!                        |
//!                        +-> Backend (SERVER, our child)
//! ```
//!
//! One thread decides everything: the client's messages, the peers' lines
//! and a quarter-second tick arrive on one channel. The backend is asked
//! synchronously from it, so a slow language server holds the loop up, and
//! with it the editor's pushes.
//!
//! # What the editor can do
//!
//! - typing: `didChange` through the [`Front`] to the session, to peers;
//!   what peers do comes back as `workspace/applyEdit`.
//! - `textDocument/rename`, `codeAction` + `workspace/executeCommand
//!   v1.intent`, `formatting` and `rangeFormatting`: intents. The reply to
//!   the request is empty (`{}`, `null`, `[]`): the effect is a push.
//!   `codeAction` lists the backend's actions with their edits stripped,
//!   each as a command `v1.intent` with arguments `[kind, title, range]`;
//!   an action that is a bare command is left out. The range is the
//!   client's at listing time and read against its buffer at execution.
//! - `v1.hold` and `v1.release` (`workspace/executeCommand`): go offline
//!   and come back ([`FromEditor::Hold`]).
//! - `hover`, `definition`, `typeDefinition`, `implementation`,
//!   `references`, `documentSymbol`, `documentHighlight` and `completion`
//!   are passed through: the backend is lent the client's buffer, every
//!   position in the request goes client -> bytes -> backend, and every
//!   position in the answer that lies in the live document goes back.
//! - the backend's diagnostics for the live document, whenever the client's
//!   buffer is what the backend last read.
//!
//! Strict: a request whose coordinates cannot be read (a push is in flight,
//! a position outside the buffer) is answered `ContentModified`, not
//! guessed. A `didChange` the front cannot follow ends the document: the
//! client is told, and every later request on it fails until the server is
//! restarted. Closing the document ends it too.

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, Position, PositionEncodingKind, Range, Uri,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::hash::Hash;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use v0::event::Event;
use v0::live::{FromEditor, Out as SessionOut, Session, ToEditor, Unit};
use v0::op::ReplicaId;
use v0::syntax::Grammar;
use v1::backend::{Backend, LspResolver};
use v1::editor::{Encoding, Front, Out, Push};

const USAGE: &str = "v1 lsp --socket PATH -- SERVER ARGS...";

/// LSP error codes.
const CONTENT_MODIFIED: i32 = -32801;
const REQUEST_FAILED: i32 = -32803;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;

/// How often the loop looks at the backend when nothing else happens.
const TICK: Duration = Duration::from_millis(250);

/// The methods passed to the backend, and advertised for it.
const PASSED: [&str; 8] = [
    "textDocument/hover",
    "textDocument/definition",
    "textDocument/typeDefinition",
    "textDocument/implementation",
    "textDocument/references",
    "textDocument/documentSymbol",
    "textDocument/documentHighlight",
    "textDocument/completion",
];

/// A request's failure: an LSP error code and its message.
type Fail = (i32, String);

/// What the loop waits on.
enum Ev {
    Client(Message),
    ClientGone,
    Peer(UnixStream),
    Wire(usize, String),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some("lsp"), Some("--socket"), Some(socket), Some("--"), Some(_)) =
        (args.first().map(String::as_str), args.get(1).map(String::as_str), args.get(2), args.get(3).map(String::as_str), args.get(4))
    else {
        eprintln!("usage: {USAGE}");
        return ExitCode::from(2);
    };
    match serve(Path::new(socket), &args[4..]) {
        Ok(clean) => ExitCode::from(if clean { 0 } else { 1 }),
        Err(e) => {
            eprintln!("v1 lsp: {e}");
            ExitCode::from(1)
        }
    }
}

/// Serve one client on stdio until it exits. True when it shut us down
/// first, as the protocol has it.
fn serve(socket: &Path, command: &[String]) -> Result<bool, String> {
    let (connection, _io) = Connection::stdio();
    let (id, params) = connection.initialize_start().map_err(|e| e.to_string())?;
    let offered: Option<Vec<PositionEncodingKind>> =
        serde_json::from_value(params["capabilities"]["general"]["positionEncodings"].clone()).ok();
    let encoding = Encoding::choose(offered.as_deref());
    let root = params["workspaceFolders"][0]["uri"]
        .as_str()
        .or(params["rootUri"].as_str())
        .map(decode);
    let caps = json!({
        "capabilities": {
            "positionEncoding": encoding.kind(),
            "textDocumentSync": {"openClose": true, "change": 2},
            "renameProvider": true,
            "codeActionProvider": true,
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": true,
            "executeCommandProvider": {"commands": ["v1.hold", "v1.release", "v1.intent"]},
            "hoverProvider": true,
            "definitionProvider": true,
            "typeDefinitionProvider": true,
            "implementationProvider": true,
            "referencesProvider": true,
            "documentSymbolProvider": true,
            "documentHighlightProvider": true,
            "completionProvider": {},
        },
        "serverInfo": {"name": "v1", "version": env!("CARGO_PKG_VERSION")},
    });
    connection.initialize_finish(id, caps).map_err(|e| e.to_string())?;

    let (events, inbox) = unbounded();
    let from_client = connection.receiver;
    let tx = events.clone();
    std::thread::spawn(move || {
        for m in from_client {
            if tx.send(Ev::Client(m)).is_err() {
                return;
            }
        }
        let _ = tx.send(Ev::ClientGone);
    });
    let mut server = Server {
        out: connection.sender,
        events,
        encoding,
        root,
        command: command.to_vec(),
        socket: socket.to_owned(),
        doc: None,
        down: false,
    };
    let clean = server.run(inbox);
    server.finish();
    Ok(clean)
}

struct Server {
    out: Sender<Message>,
    events: Sender<Ev>,
    encoding: Encoding,
    /// The workspace the client named.
    root: Option<PathBuf>,
    /// The language server's command line.
    command: Vec<String>,
    socket: PathBuf,
    doc: Option<Doc>,
    /// `shutdown` has been answered.
    down: bool,
}

impl Server {
    /// The loop. True when the client said `shutdown` before `exit`.
    fn run(&mut self, inbox: Receiver<Ev>) -> bool {
        loop {
            match inbox.recv_timeout(TICK) {
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) | Ok(Ev::ClientGone) => return false,
                Ok(Ev::Client(Message::Notification(n))) if n.method == "exit" => return self.down,
                Ok(Ev::Client(m)) => self.client(m),
                Ok(Ev::Peer(stream)) => {
                    if let Some(d) = &mut self.doc {
                        d.peer(stream, &self.events);
                    }
                }
                Ok(Ev::Wire(from, line)) => self.doc.iter_mut().for_each(|d| d.wire(from, &line)),
            }
            if let Some(d) = &mut self.doc {
                d.diagnostics();
            }
        }
    }

    fn client(&mut self, m: Message) {
        match m {
            Message::Request(r) => {
                let id = r.id.clone();
                let answer = match self.request(&r.method, r.params) {
                    Ok(v) => Response::new_ok(id, v),
                    Err((code, why)) => Response::new_err(id, code, why),
                };
                let _ = self.out.send(answer.into());
            }
            Message::Response(r) => self.doc.iter_mut().for_each(|d| d.response(r.clone())),
            Message::Notification(n) => self.notification(n),
        }
    }

    fn notification(&mut self, n: Notification) {
        match n.method.as_str() {
            "textDocument/didOpen" => match serde_json::from_value(n.params) {
                Ok(p) => self.open(p),
                Err(e) => eprintln!("v1 lsp: didOpen: {e}"),
            },
            "textDocument/didChange" => match serde_json::from_value::<DidChangeTextDocumentParams>(n.params) {
                Ok(p) => self.doc.iter_mut().filter(|d| d.is_ours(p.text_document.uri.as_str())).for_each(|d| d.changed(p.clone())),
                Err(e) => eprintln!("v1 lsp: didChange: {e}"),
            },
            "textDocument/didClose" => {
                let uri = n.params["textDocument"]["uri"].as_str().unwrap_or_default().to_owned();
                self.doc.iter_mut().filter(|d| d.is_ours(&uri)).for_each(|d| d.fail("the document was closed".into()));
            }
            _ => {}
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, Fail> {
        if method == "shutdown" {
            self.down = true;
            return Ok(Value::Null);
        }
        let doc = self.doc.as_mut().ok_or((REQUEST_FAILED, "no live document is open".to_owned()))?;
        if method == "workspace/executeCommand" {
            return doc.execute(&params);
        }
        let uri = params["textDocument"]["uri"].as_str().ok_or((INVALID_PARAMS, format!("{method}: no document")))?;
        if !doc.is_ours(uri) {
            return Err((REQUEST_FAILED, "v1 lsp serves one live document, and this is not it".into()));
        }
        match method {
            "textDocument/rename" => doc.rename(&params),
            "textDocument/codeAction" => doc.actions(&params),
            "textDocument/formatting" => doc.format(None),
            "textDocument/rangeFormatting" => doc.format(Some(&params["range"])),
            m if PASSED.contains(&m) => doc.pass(m, params),
            m => Err((METHOD_NOT_FOUND, format!("{m}: v1 lsp does not serve this"))),
        }
    }

    /// The first document with a grammar becomes the live one: start its
    /// language server and join or host its session.
    fn open(&mut self, p: DidOpenTextDocumentParams) {
        let d = p.text_document;
        let name = d.uri.as_str().rsplit('/').next().map(decode_name).unwrap_or_default();
        if let Some(doc) = &mut self.doc {
            if doc.is_ours(d.uri.as_str()) {
                doc.fail("the document was opened again".into());
            }
            return;
        }
        let Some(grammar) = Grammar::for_name(&name) else { return };
        let root = self.root.clone().or_else(|| decode(d.uri.as_str()).parent().map(Path::to_owned)).unwrap_or_default();
        match Doc::start(self, &d.uri, &name, grammar, &root, &d.language_id, d.text, d.version) {
            Ok(doc) => self.doc = Some(doc),
            Err(e) => message(&self.out, 1, &format!("v1 lsp: {e}")),
        }
    }

    /// Leave: the language server stopped, the host's socket removed.
    fn finish(&mut self) {
        let Some(doc) = self.doc.take() else { return };
        let Doc { session, backend, role, socket, .. } = doc;
        drop(session);
        if role == "host" {
            let _ = std::fs::remove_file(socket);
        }
        if let Ok(b) = Arc::try_unwrap(backend) {
            b.into_inner().unwrap_or_else(|e| e.into_inner()).stop();
        }
    }
}

/// The live document: the client's view of it, the session that owns it,
/// the language server that judges it.
struct Doc {
    uri: Uri,
    front: Option<Front>,
    session: Session,
    backend: Arc<Mutex<Backend>>,
    /// The text the language server last read from us.
    lent: String,
    notes: Receiver<Notification>,
    /// The last diagnostics, not yet sent.
    pending: Option<Value>,
    peers: Vec<UnixStream>,
    role: &'static str,
    socket: PathBuf,
    /// The `applyEdit` awaiting its answer.
    apply: Option<RequestId>,
    ids: u64,
    out: Sender<Message>,
}

/// What a step leaves to do: messages for the session, pushes for the
/// client, and the session's refusal of the intent, if it made one.
#[derive(Default)]
struct Work {
    queue: VecDeque<FromEditor>,
    pushes: Vec<Push>,
    refused: Option<String>,
}

impl Doc {
    #[allow(clippy::too_many_arguments)]
    fn start(
        s: &Server,
        uri: &Uri,
        name: &str,
        grammar: Grammar,
        root: &Path,
        language: &str,
        text: String,
        version: i32,
    ) -> Result<Doc, String> {
        let backend = Backend::spawn(&s.command, root, uri.clone(), language, &text).map_err(|e| format!("{}: {e}", s.command[0]))?;
        let notes = backend.notifications.clone();
        let backend = Arc::new(Mutex::new(backend));
        let (session, role, peers) = join(&s.socket, name, grammar, &s.events)?;
        let session = session.with_resolver(Box::new(LspResolver { backend: backend.clone() }));
        let mut doc = Doc {
            uri: uri.clone(),
            front: Some(Front::new(s.encoding, text.clone(), version)),
            lent: text,
            session,
            backend,
            notes,
            pending: None,
            peers,
            role,
            socket: s.socket.clone(),
            apply: None,
            ids: 0,
            out: s.out.clone(),
        };
        // A host's buffer is the document; a guest's is replaced by it.
        let ready = ToEditor::Ready { role, text: doc.session.text() };
        doc.step(|f| {
            let mut out = f.heard(ready)?;
            if role == "host" {
                out.session.extend(f.opened());
            }
            Ok(out)
        });
        Ok(doc)
    }

    fn is_ours(&self, uri: &str) -> bool {
        same(uri, self.uri.as_str())
    }

    /// The document cannot go on: tell the client, and stop serving it.
    fn fail(&mut self, why: String) {
        if self.front.take().is_some() {
            message(&self.out, 1, &format!("v1 lsp: {why}; restart the language server to go on with this document"));
        }
    }

    fn front(&self) -> Result<&Front, Fail> {
        self.front.as_ref().ok_or((REQUEST_FAILED, "the document is out of step: restart the language server".to_owned()))
    }

    // --- session and client ---------------------------------------------

    /// Do something to the front, and carry it through. The session's
    /// refusal of an intent is returned; anything else that goes wrong ends
    /// the document.
    fn step(&mut self, f: impl FnOnce(&mut Front) -> Result<Out, String>) -> Option<String> {
        let out = match self.front.as_mut().map(f) {
            None => return None,
            Some(Ok(out)) => out,
            Some(Err(why)) => {
                self.fail(why.clone());
                return Some(why);
            }
        };
        let mut work = Work { queue: out.session.into(), pushes: out.push.into_iter().collect(), refused: None };
        self.run(&mut work);
        work.refused
    }

    /// Hand messages to the session until nothing is left, then bring the
    /// language server and the client up to date.
    fn run(&mut self, work: &mut Work) {
        while let Some(msg) = work.queue.pop_front() {
            let out = self.session.local(msg);
            self.absorb(out, work);
        }
        let text = self.session.text();
        let lent = self.backend().set_text(&text);
        match lent {
            Ok(()) => self.lent = text,
            Err(e) => message(&self.out, 2, &format!("v1 lsp: the language server: {e}")),
        }
        for p in work.pushes.drain(..) {
            self.push(p);
        }
    }

    /// The session's output: events to the peers, lines to the front.
    fn absorb(&mut self, out: SessionOut, work: &mut Work) {
        self.tell(None, &out.send);
        for reply in out.reply {
            match reply {
                ToEditor::Error(why) => work.refused = Some(why),
                reply => {
                    let Some(front) = self.front.as_mut() else { continue };
                    match front.heard(reply) {
                        Ok(more) => {
                            work.queue.extend(more.session);
                            work.pushes.extend(more.push);
                        }
                        Err(why) => {
                            self.fail(why.clone());
                            work.refused = Some(why);
                        }
                    }
                }
            }
        }
    }

    /// Events to every peer but `except`. A peer that has gone is not
    /// written to again; the rest carry on.
    fn tell(&mut self, except: Option<usize>, events: &[Event]) {
        if events.is_empty() {
            return;
        }
        let mut line = serde_json::to_vec(events).expect("events are plain data");
        line.push(b'\n');
        for (i, peer) in self.peers.iter_mut().enumerate() {
            if Some(i) != except {
                let _ = peer.write_all(&line);
            }
        }
    }

    /// A guest has connected: it is sent everything we hold.
    fn peer(&mut self, mut stream: UnixStream, events: &Sender<Ev>) {
        let mut line = serde_json::to_vec(&self.session.events()).expect("events are plain data");
        line.push(b'\n');
        if stream.write_all(&line).is_ok()
            && let Ok(read) = stream.try_clone()
        {
            listen(read, self.peers.len(), events.clone());
            self.peers.push(stream);
        }
    }

    /// A line from a peer. A host is also the relay between its guests.
    fn wire(&mut self, from: usize, line: &str) {
        let events: Vec<Event> = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(e) => return eprintln!("v1 lsp: a line from a peer was not events: {e}"),
        };
        self.tell(Some(from), &events);
        let out = self.session.remote(events);
        let mut work = Work::default();
        self.absorb(out, &mut work);
        self.run(&mut work);
    }

    /// `workspace/applyEdit`, at the version the client is believed to be.
    fn push(&mut self, p: Push) {
        let id = RequestId::from(format!("v1-{}", self.ids));
        self.ids += 1;
        self.apply = Some(id.clone());
        let doc = json!({"uri": self.uri.as_str(), "version": p.version});
        let edit = json!({"documentChanges": [{"textDocument": doc, "edits": p.edits}]});
        let _ = self.out.send(Request::new(id, "workspace/applyEdit".into(), json!({"label": "v1", "edit": edit})).into());
    }

    /// The client's answer to a request of ours: only `applyEdit` is asked.
    fn response(&mut self, r: Response) {
        if self.apply.as_ref() != Some(&r.id) {
            return;
        }
        self.apply = None;
        let result = match r.response_result {
            Ok(v) => v,
            Err(e) => return self.fail(format!("applyEdit failed: {}", e.message)),
        };
        let applied = result["applied"].as_bool();
        let Some(applied) = applied else { return self.fail("applyEdit was answered without a verdict".into()) };
        if let Some(why) = self.step(|f| f.answered(applied)) {
            message(&self.out, 1, &format!("v1 lsp: {why}"));
        }
    }

    fn changed(&mut self, p: DidChangeTextDocumentParams) {
        let (version, changes) = (p.text_document.version, p.content_changes);
        if let Some(why) = self.step(|f| f.changed(version, changes)) {
            self.fail(why);
        }
    }

    /// Go offline, or come back.
    fn hold(&mut self, on: bool) {
        let out = self.session.local(FromEditor::Hold(on));
        let mut work = Work::default();
        self.absorb(out, &mut work);
        self.run(&mut work);
    }

    // --- requests ---------------------------------------------------------

    fn backend(&self) -> MutexGuard<'_, Backend> {
        self.backend.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run an intent built by the front; the answer to its request is
    /// empty, since the effect comes as a push.
    fn intent(&mut self, make: impl FnOnce(&mut Front) -> Result<FromEditor, String>) -> Result<(), Fail> {
        let front = self.front.as_mut().ok_or((REQUEST_FAILED, "the document is out of step: restart the language server".to_owned()))?;
        let msg = make(front).map_err(|e| (CONTENT_MODIFIED, e))?;
        let mut work = Work { queue: [msg].into(), ..Work::default() };
        self.run(&mut work);
        match work.refused {
            Some(why) => Err((REQUEST_FAILED, why)),
            None => Ok(()),
        }
    }

    fn rename(&mut self, p: &Value) -> Result<Value, Fail> {
        let at: Position = parse(&p["position"])?;
        let to = p["newName"].as_str().ok_or((INVALID_PARAMS, "rename: no newName".to_owned()))?.to_owned();
        self.intent(|f| f.rename(at, to))?;
        Ok(json!({}))
    }

    fn format(&mut self, range: Option<&Value>) -> Result<Value, Fail> {
        let range: Range = match range {
            Some(r) => parse(r)?,
            None => self.front()?.whole(),
        };
        self.intent(|f| f.format(range))?;
        Ok(json!([]))
    }

    fn execute(&mut self, p: &Value) -> Result<Value, Fail> {
        let args = p["arguments"].as_array().map_or(&[][..], Vec::as_slice);
        match p["command"].as_str().unwrap_or_default() {
            "v1.hold" => self.hold(true),
            "v1.release" => self.hold(false),
            "v1.intent" => {
                let text = |i: usize| args.get(i).and_then(Value::as_str).map(str::to_owned);
                let (Some(kind), Some(title), Some(range)) = (text(0), text(1), args.get(2)) else {
                    return Err((INVALID_PARAMS, "v1.intent takes [kind, title, range]".into()));
                };
                let range: Range = parse(range)?;
                self.intent(|f| f.action(range, kind, title))?;
            }
            other => return Err((INVALID_PARAMS, format!("{other:?} is not a command of v1"))),
        }
        Ok(Value::Null)
    }

    /// Lend the language server the client's buffer, and give back the
    /// buffer it is, with the byte range `range` names in it.
    fn lend(&mut self, range: Option<&Range>) -> Result<(String, Option<std::ops::Range<usize>>), Fail> {
        let front = self.front()?;
        let mirror = front.text().to_owned();
        let at = |p: Position| front.offset(p).ok_or((CONTENT_MODIFIED, format!("{p:?} is not in the buffer, or a push is in flight")));
        let bytes = range.map(|r| Ok::<_, Fail>(at(r.start)?..at(r.end)?)).transpose()?;
        if bytes.as_ref().is_some_and(|b| b.end < b.start) {
            return Err((INVALID_PARAMS, "a range ends before it starts".into()));
        }
        self.backend().set_text(&mirror).map_err(|e| (REQUEST_FAILED, e))?;
        self.lent = mirror.clone();
        Ok((mirror, bytes))
    }

    /// The backend's code actions, each as a command that makes an intent.
    fn actions(&mut self, p: &Value) -> Result<Value, Fail> {
        let range: Range = parse(&p["range"])?;
        let only = match p["context"]["only"].as_array().map(Vec::as_slice) {
            Some([one]) => one.as_str().unwrap_or_default(),
            _ => "",
        };
        let (_, bytes) = self.lend(Some(&range))?;
        let listed = self.backend().actions(bytes.expect("a range was given"), only).map_err(|e| (REQUEST_FAILED, e))?;
        let mut out = Vec::new();
        for mut a in listed {
            let (Some(kind), Some(title)) = (a["kind"].as_str().map(str::to_owned), a["title"].as_str().map(str::to_owned)) else {
                continue; // a bare command: nothing to record as an intent
            };
            let o = a.as_object_mut().expect("a code action is an object");
            o.remove("edit");
            o.remove("data");
            o.insert("command".into(), json!({"title": title, "command": "v1.intent", "arguments": [kind, title, range]}));
            out.push(a);
        }
        Ok(Value::Array(out))
    }

    /// A request for the language server, with positions converted: the
    /// client's buffer -> bytes -> the server's, and back.
    fn pass(&mut self, method: &str, mut params: Value) -> Result<Value, Fail> {
        let (mirror, _) = self.lend(None)?;
        let ours = |uri: &str| self.is_ours(uri);
        let front = self.front()?;
        let backend = self.backend();
        walk(&mut params, &ours, &mut |p| {
            let at = front.offset(p).ok_or(format!("{p:?} is not in the buffer"))?;
            backend.position(at)
        })
        .map_err(|e| (CONTENT_MODIFIED, e))?;
        drop(backend);
        let mut answer = self.backend().request(method, params).map_err(|e| (REQUEST_FAILED, e))?;
        let backend = self.backend();
        walk(&mut answer, &ours, &mut |p| {
            let at = backend.offset(p).ok_or(format!("{p:?} is not in the document"))?;
            front.position(at).ok_or(format!("byte {at} is not a position of the buffer"))
        })
        .map_err(|e| (REQUEST_FAILED, e))?;
        debug_assert_eq!(front.text(), mirror);
        Ok(answer)
    }

    /// Send the backend's diagnostics for the live document, once the
    /// client's buffer is the text they were made for.
    fn diagnostics(&mut self) {
        let _ = self.backend().drain();
        while let Ok(n) = self.notes.try_recv() {
            if n.method == "textDocument/publishDiagnostics" && n.params["uri"].as_str().is_some_and(|u| self.is_ours(u)) {
                self.pending = Some(n.params);
            }
        }
        let Some(front) = &self.front else { return };
        if self.pending.is_none() || front.text() != self.lent || front.position(0).is_none() {
            return;
        }
        let mut params = self.pending.take().expect("checked");
        params.as_object_mut().expect("diagnostics are an object").remove("version");
        let backend = self.backend();
        let converted = walk(&mut params, &|_| true, &mut |p| {
            let at = backend.offset(p).ok_or(format!("{p:?} is not in the document"))?;
            front.position(at).ok_or(format!("byte {at} is not a position of the buffer"))
        });
        drop(backend);
        match converted {
            Ok(()) => {
                params["uri"] = json!(self.uri.as_str());
                let _ = self.out.send(Notification::new("textDocument/publishDiagnostics".into(), params).into());
            }
            Err(e) => eprintln!("v1 lsp: diagnostics dropped: {e}"),
        }
    }
}

/// Join the session on `socket` as its guest, or host it if no one
/// listens, exactly as `v0 live` does.
fn join(socket: &Path, name: &str, grammar: Grammar, events: &Sender<Ev>) -> Result<(Session, &'static str, Vec<UnixStream>), String> {
    let io = |e: std::io::Error| format!("{}: {e}", socket.display());
    let replica = {
        use std::hash::Hasher;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let mut h = std::hash::DefaultHasher::new();
        (std::process::id(), now).hash(&mut h);
        ReplicaId(h.finish())
    };
    // Two that start together must not both find no one there: finding out
    // and listening is one step, under a lock beside the socket.
    let lock = std::fs::File::create(socket.with_extension("lock")).map_err(io)?;
    lock.lock().map_err(io)?;
    match UnixStream::connect(socket) {
        Ok(host) => {
            // The host's first line is everything it holds.
            let mut first = String::new();
            let mut reader = BufReader::new(host.try_clone().map_err(io)?);
            reader.read_line(&mut first).map_err(io)?;
            let held = serde_json::from_str(&first).map_err(|e| format!("from the host: {e}"))?;
            let session = Session::guest(replica, grammar, Unit::Bytes, held)?;
            let tx = events.clone();
            std::thread::spawn(move || {
                for line in reader.lines().map_while(Result::ok) {
                    if tx.send(Ev::Wire(0, line)).is_err() {
                        break;
                    }
                }
            });
            Ok((session, "guest", vec![host]))
        }
        Err(_) => {
            // No one there: a socket file left by a host that died is in the way.
            let _ = std::fs::remove_file(socket);
            let listener = UnixListener::bind(socket).map_err(io)?;
            let tx = events.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().map_while(Result::ok) {
                    if tx.send(Ev::Peer(stream)).is_err() {
                        break;
                    }
                }
            });
            Ok((Session::host(replica, grammar, Unit::Bytes, name), "host", Vec::new()))
        }
    }
}

/// Read a guest's lines into the loop, as peer `i`.
fn listen(stream: UnixStream, i: usize, tx: Sender<Ev>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if tx.send(Ev::Wire(i, line)).is_err() {
                break;
            }
        }
    });
}

/// `window/showMessage`: 1 error, 2 warning.
fn message(out: &Sender<Message>, kind: u8, text: &str) {
    eprintln!("{text}");
    let _ = out.send(Notification::new("window/showMessage".into(), json!({"type": kind, "message": text})).into());
}

fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, Fail> {
    serde_json::from_value(v.clone()).map_err(|e| (INVALID_PARAMS, format!("{v}: {e}")))
}

/// Every position in `v` that lies in the live document, replaced by what
/// `f` makes of it. A position is an object of `line` and `character`
/// alone. An object that names another file by `uri` or `targetUri` is left
/// as it is, but for its `originSelectionRange`, which is always here.
fn walk(v: &mut Value, ours: &dyn Fn(&str) -> bool, f: &mut dyn FnMut(Position) -> Result<Position, String>) -> Result<(), String> {
    match v {
        Value::Array(a) => a.iter_mut().try_for_each(|x| walk(x, ours, f)),
        Value::Object(o) if o.len() == 2 && o.get("line").is_some_and(Value::is_u64) && o.get("character").is_some_and(Value::is_u64) => {
            let p: Position = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
            *v = serde_json::to_value(f(p)?).map_err(|e| e.to_string())?;
            Ok(())
        }
        Value::Object(o) => {
            let foreign = ["uri", "targetUri"].iter().any(|k| o.get(*k).and_then(Value::as_str).is_some_and(|u| !ours(u)));
            for (k, x) in o.iter_mut() {
                if !foreign || k == "originSelectionRange" {
                    walk(x, ours, f)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The path a `file://` uri names, percent-decoded.
fn decode(uri: &str) -> PathBuf {
    let raw = uri.strip_prefix("file://").unwrap_or(uri).as_bytes();
    let mut path = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let hex = raw.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (raw[i], hex) {
            (b'%', Some(b)) => {
                path.push(b);
                i += 3;
            }
            (b, _) => {
                path.push(b);
                i += 1;
            }
        }
    }
    PathBuf::from(String::from_utf8_lossy(&path).into_owned())
}

/// A uri's last segment, decoded.
fn decode_name(segment: &str) -> String {
    decode(segment).to_string_lossy().into_owned()
}

/// Do two uris name the same file? Symlinks resolved when it exists, so a
/// temp directory behind a link is the same place.
fn same(a: &str, b: &str) -> bool {
    let canonical = |u: &str| {
        let p = decode(u);
        p.canonicalize().unwrap_or(p)
    };
    a == b || canonical(a) == canonical(b)
}
