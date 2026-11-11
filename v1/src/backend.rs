//! The language's side: its own language server (rust-analyzer,
//! basedpyright, ...) run as our child, with us as its client. We lend it
//! the live document, as whatever text we need judged, and ask it what a
//! name means or what a verb does. [`LspResolver`] is the
//! [`v0::live::Resolver`] that does so.
//!
//! The server sees one document, at the path the editor opened, in the
//! workspace the editor named, so it resolves it in its crate or package.
//! Its text is set whole (`didChange` with full content) before every
//! question ([`Backend::set_text`]), and only when it differs. A server that
//! is still loading answers with nothing or with `ContentModified` (-32801)
//! or `RequestCancelled` (-32800): ask again, within [`ANSWER_WAIT_MAX`].
//! "Still loading" is what the server says when it says anything
//! (rust-analyzer's `experimental/serverStatus`: not quiescent), and else
//! that it has not yet given any non-empty answer.
//!
//! Synchronous: one request at a time, and while it waits we answer what the
//! server asks of its client (`workspace/configuration`, progress and
//! registration: all with nothing) and keep its notifications, diagnostics
//! among them, on [`Backend::notifications`].
//!
//! Positions are the server's: it picks utf-8 or utf-16 from what we offer
//! ([`Backend::position`]). A position or a message that does not fit is an
//! error, never a guess. Lines end at `\n`, `\r\n` or `\r`, as LSP counts
//! them, and as [`crate::editor`] reads the client's.

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use lsp_server::{Message, Notification, Request, RequestId, Response, ResponseError};
use lsp_types::{Position, Uri};
use serde_json::{Value, json};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::editor::{Encoding, byte, position};
use v0::live::{Resolver, span};
use v0::op::Intent;
use v0::syntax::Syntax;
use v0::weave::{Chars, Weave};

/// How long one question may wait for a server that is loading.
pub const ANSWER_WAIT_MAX: Duration = Duration::from_secs(60);

/// How long to wait between asking again.
const RETRY_PAUSE: Duration = Duration::from_millis(50);

/// `ContentModified` and `RequestCancelled`: the server changed its mind
/// about the text under the question; ask again.
const RETRY_CODES: [i32; 2] = [-32801, -32800];

/// Replacements of byte ranges of the current text.
pub type Edits = Vec<(Range<usize>, String)>;

/// A language server, running, with one document open.
pub struct Backend {
    child: Child,
    stdin: Option<ChildStdin>,
    inbox: Receiver<Message>,
    notes: Sender<Notification>,
    /// Everything the server notified us of, in order: diagnostics, log
    /// messages, status.
    pub notifications: Receiver<Notification>,
    next_id: i32,
    uri: Uri,
    /// The document as the server has it, and its version.
    text: String,
    version: i32,
    encoding: Encoding,
    /// `Some(quiescent)` once the server reports its status.
    quiescent: Option<bool>,
    /// A non-empty answer has been given.
    warm: bool,
}

impl Backend {
    /// Start `command` (e.g. `["rust-analyzer"]`) in `root`, initialize it
    /// (offering utf-8 and utf-16 positions), and open `uri` with `text`.
    /// rust-analyzer is also waited for until it is quiescent.
    pub fn spawn(command: &[String], root: &Path, uri: Uri, language_id: &str, text: &str) -> std::io::Result<Backend> {
        let fail = |e: String| std::io::Error::other(e);
        let (program, args) = command.split_first().ok_or_else(|| fail("no command".into()))?;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take();
        let mut out = std::io::BufReader::new(child.stdout.take().expect("piped"));
        let (tx, inbox) = unbounded();
        std::thread::spawn(move || {
            while let Ok(Some(m)) = Message::read(&mut out) {
                if tx.send(m).is_err() {
                    break;
                }
            }
        });
        let (notes, notifications) = unbounded();
        let (encoding, quiescent, warm, next_id, version) = (Encoding::Utf16, None, false, 1, 1);
        let text = text.to_owned();
        let mut b = Backend { child, stdin, inbox, notes, notifications, next_id, uri, text, version, encoding, quiescent, warm };
        let root_uri = file_uri(root);
        let name = root.file_name().map_or("root".into(), |n| n.to_string_lossy().into_owned());
        let init = b
            .call(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri.as_str(),
                    "workspaceFolders": [{"uri": root_uri.as_str(), "name": name}],
                    "capabilities": capabilities(),
                    "initializationOptions": {"checkOnSave": false},
                }),
                Instant::now() + ANSWER_WAIT_MAX,
            )
            .map_err(|e| fail(format!("initialize: {}", e.message())))?;
        if init["capabilities"]["positionEncoding"] == "utf-8" {
            b.encoding = Encoding::Utf8;
        }
        b.notify("initialized", json!({})).map_err(fail)?;
        let open = json!({"uri": b.uri.as_str(), "languageId": language_id, "version": b.version, "text": b.text});
        b.notify("textDocument/didOpen", json!({"textDocument": open})).map_err(fail)?;
        if init["serverInfo"]["name"] == "rust-analyzer" {
            let deadline = Instant::now() + ANSWER_WAIT_MAX;
            while b.quiescent != Some(true) {
                b.pump(deadline).map_err(fail)?;
            }
        }
        Ok(b)
    }

    /// Make the server's copy of the document read `text`: a full-content
    /// `didChange` at the next version, only when it differs.
    pub fn set_text(&mut self, text: &str) -> Result<(), String> {
        if self.text == text {
            return Ok(());
        }
        self.version += 1;
        self.text = text.into();
        let change = json!({"textDocument": {"uri": self.uri.as_str(), "version": self.version}, "contentChanges": [{"text": text}]});
        self.notify("textDocument/didChange", change)
    }

    /// `textDocument/definition` at byte `at` of the current text: where it
    /// points, in this document, as byte ranges (a link's name, not its
    /// body). Locations in other files are left out.
    pub fn definition(&mut self, at: usize) -> Result<Vec<Range<usize>>, String> {
        let params = json!({"textDocument": self.doc(), "position": self.position(at)?});
        let found = match self.request("textDocument/definition", params)? {
            Value::Array(a) => a,
            Value::Null => vec![],
            one => vec![one],
        };
        let mut here = vec![];
        for l in &found {
            let (uri, range) = match l.get("targetUri") {
                Some(uri) => (uri, &l["targetSelectionRange"]),
                None => (&l["uri"], &l["range"]),
            };
            if self.is_here(uri.as_str().ok_or("a location without a uri")?) {
                here.push(self.bytes(range)?);
            }
        }
        Ok(here)
    }

    /// `textDocument/rename` at byte `at`: the edits for this document.
    /// `Err("cross-file")` when it touches another: the ADR refuses those.
    pub fn rename(&mut self, at: usize, to: &str) -> Result<Edits, String> {
        let params = json!({"textDocument": self.doc(), "position": self.position(at)?, "newName": to});
        let edit = self.request("textDocument/rename", params)?;
        if edit.is_null() {
            return Err("nothing to rename there".into());
        }
        self.edits(&edit)
    }

    /// The code action of `kind` and `title` over the byte range, as edits.
    /// `codeAction/resolve` fetches the edit when the listing has none.
    /// Exactly one action must match, and it must be enabled.
    pub fn action(&mut self, range: Range<usize>, kind: &str, title: &str) -> Result<Edits, String> {
        let listed = self.actions(range, kind)?;
        let mut found = listed.iter().filter(|a| a["title"] == title && a["kind"].as_str().unwrap_or("") == kind);
        let action = found.next().ok_or_else(|| format!("no code action {kind:?} {title:?} there"))?;
        if found.next().is_some() {
            return Err(format!("code action {kind:?} {title:?} is not unique there"));
        }
        if let Some(why) = action.get("disabled") {
            return Err(format!("code action disabled: {}", why["reason"]));
        }
        let resolved;
        let edit = match action.get("edit") {
            Some(edit) => edit,
            None => {
                resolved = self.request("codeAction/resolve", action.clone())?;
                resolved.get("edit").ok_or("the code action has no edit")?
            }
        };
        self.edits(edit)
    }

    /// `textDocument/codeAction` over the byte range, restricted to `kind`
    /// unless that is empty: the list as the server gave it.
    pub fn actions(&mut self, range: Range<usize>, kind: &str) -> Result<Vec<Value>, String> {
        let mut context = json!({"diagnostics": []});
        if !kind.is_empty() {
            context["only"] = json!([kind]);
        }
        let params = json!({"textDocument": self.doc(), "range": self.range(&range)?, "context": context});
        match self.request("textDocument/codeAction", params)? {
            Value::Array(a) => Ok(a),
            Value::Null => Ok(vec![]),
            other => Err(format!("a code action list was expected, not {other}")),
        }
    }

    /// Format the byte range: `textDocument/rangeFormatting`, or
    /// `formatting` when it is the whole document.
    ///
    /// Some servers cannot format a range: rust-analyzer answers `-32600`
    /// unless rustfmt is a nightly (measured, rust-analyzer 2026-06-01 with
    /// rustfmt 1.95 stable). Then the whole document is formatted and the
    /// edits kept are those that lie within the lines the range touches
    /// (from the start of the first to the end of the last, its `\n`
    /// excluded, so the next line's indent is not ours); an edit across that
    /// border is dropped, never cut.
    pub fn format(&mut self, range: Range<usize>) -> Result<Edits, String> {
        let options = json!({"tabSize": 4, "insertSpaces": true});
        let whole = json!({"textDocument": self.doc(), "options": options});
        if range != (0..self.text.len()) {
            let ranged = json!({"textDocument": self.doc(), "range": self.range(&range)?, "options": options});
            if let Ok(edits) = self.request("textDocument/rangeFormatting", ranged) {
                return self.text_edits(edits.as_array().ok_or("a list of edits was expected")?);
            }
        }
        let edits = self.request("textDocument/formatting", whole)?;
        let edits = self.text_edits(edits.as_array().ok_or("a list of edits was expected")?)?;
        let start = self.text[..range.start].rfind('\n').map_or(0, |i| i + 1);
        let end = self.text[range.end..].find('\n').map_or(self.text.len(), |i| range.end + i);
        let whole_doc = range == (0..self.text.len());
        Ok(edits.into_iter().filter(|(r, _)| whole_doc || (start <= r.start && r.end <= end)).collect())
    }

    /// Any other request, passed through: `params` with positions already
    /// in this server's coordinates for its current text. Asked again while
    /// the server is loading (see the module doc), at most
    /// [`ANSWER_WAIT_MAX`].
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let deadline = Instant::now() + ANSWER_WAIT_MAX;
        loop {
            self.drain()?;
            match self.call(method, params.clone(), deadline) {
                Err(Failure::Rpc(e)) if RETRY_CODES.contains(&e.code) && Instant::now() < deadline => {}
                Ok(v) if empty(&v) && self.loading() && Instant::now() < deadline => {}
                Ok(v) => {
                    self.warm |= !empty(&v);
                    return Ok(v);
                }
                Err(e) => return Err(format!("{method}: {}", e.message())),
            }
            std::thread::sleep(RETRY_PAUSE);
        }
    }

    /// How the server counts columns: what it picked of utf-8 and utf-16.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The position of byte `at` of the current text, in the server's
    /// encoding. An error when `at` is not a character boundary, or is
    /// between the `\r` and the `\n` of a line terminator.
    pub fn position(&self, at: usize) -> Result<Position, String> {
        position(self.encoding, &self.text, at).ok_or_else(|| format!("byte {at} is not a position of the document"))
    }

    /// The byte of the current text a position names; `None` when the line
    /// or the column is outside the text or inside a character.
    pub fn offset(&self, at: Position) -> Option<usize> {
        byte(self.encoding, &self.text, at)
    }

    /// Shut the server down: `shutdown`, `exit`, close its stdin, wait.
    pub fn stop(mut self) {
        let _ = self.call("shutdown", Value::Null, Instant::now() + Duration::from_secs(10));
        let _ = self.notify("exit", Value::Null);
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
            std::thread::sleep(RETRY_PAUSE);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    // --- internals ----------------------------------------------------

    fn doc(&self) -> Value {
        json!({"uri": self.uri.as_str()})
    }

    fn range(&self, r: &Range<usize>) -> Result<Value, String> {
        Ok(json!({"start": self.position(r.start)?, "end": self.position(r.end)?}))
    }

    /// An LSP range as bytes of the current text.
    fn bytes(&self, r: &Value) -> Result<Range<usize>, String> {
        let at = |p: &Value| {
            let p: Position = serde_json::from_value(p.clone()).map_err(|e| format!("a bad position {p}: {e}"))?;
            self.offset(p).ok_or_else(|| format!("position {}:{} is not in the document", p.line, p.character))
        };
        let (start, end) = (at(&r["start"])?, at(&r["end"])?);
        if end < start {
            return Err(format!("a range backwards: {r}"));
        }
        Ok(start..end)
    }

    /// Is `uri` this document? Compared as files, so a symlinked temp
    /// directory is the same place.
    fn is_here(&self, uri: &str) -> bool {
        uri == self.uri.as_str() || canonical(uri) == canonical(self.uri.as_str())
    }

    /// A `WorkspaceEdit`'s edits for this document, sorted.
    fn edits(&self, edit: &Value) -> Result<Edits, String> {
        let mut out = vec![];
        let mut files = |uri: &str, edits: &Value| -> Result<(), String> {
            let edits = edits.as_array().ok_or("a list of edits was expected")?;
            if !edits.is_empty() && !self.is_here(uri) {
                return Err("cross-file".into());
            }
            out.extend(self.text_edits(edits)?);
            Ok(())
        };
        if let Some(changes) = edit["changes"].as_object() {
            changes.iter().try_for_each(|(uri, edits)| files(uri, edits))?;
        }
        for c in edit["documentChanges"].as_array().into_iter().flatten() {
            if c.get("kind").is_some() {
                return Err("cross-file: a file operation".into());
            }
            files(c["textDocument"]["uri"].as_str().ok_or("an edit without a uri")?, &c["edits"])?;
        }
        out.sort_by_key(|e| (e.0.start, e.0.end));
        Ok(out)
    }

    fn text_edits(&self, edits: &[Value]) -> Result<Edits, String> {
        let one = |e: &Value| Ok((self.bytes(&e["range"])?, e["newText"].as_str().ok_or("an edit without newText")?.to_owned()));
        let mut out = edits.iter().map(one).collect::<Result<Edits, String>>()?;
        out.sort_by_key(|e| (e.0.start, e.0.end));
        Ok(out)
    }

    fn loading(&self) -> bool {
        self.quiescent.map_or(!self.warm, |q| !q)
    }

    fn send(&mut self, m: impl Into<Message>) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("the server is shut down")?;
        m.into().write(stdin).map_err(|e| format!("the server is gone: {e}"))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(Notification::new(method.into(), params))
    }

    /// One request, once: its response or why there is none.
    fn call(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, Failure> {
        let id = RequestId::from(self.next_id);
        self.next_id += 1;
        self.send(Request::new(id.clone(), method.into(), params)).map_err(Failure::Other)?;
        loop {
            if let Some(r) = self.pump(deadline).map_err(Failure::Other)?
                && r.id == id
            {
                return r.response_result.map_err(Failure::Rpc);
            }
        }
    }

    /// Handle what is already waiting, without blocking: the server's
    /// notifications reach [`Backend::notifications`] even when nobody
    /// asks it anything.
    pub fn drain(&mut self) -> Result<(), String> {
        while let Ok(m) = self.inbox.try_recv() {
            self.handle(m)?;
        }
        Ok(())
    }

    /// Wait for one message and handle it; a response comes back.
    fn pump(&mut self, deadline: Instant) -> Result<Option<Response>, String> {
        match self.inbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(m) => self.handle(m),
            Err(RecvTimeoutError::Timeout) => Err("the server did not answer in time".into()),
            Err(RecvTimeoutError::Disconnected) => Err("the server exited".into()),
        }
    }

    /// The client's half: answer what the server asks, note its status,
    /// keep the rest.
    fn handle(&mut self, m: Message) -> Result<Option<Response>, String> {
        match m {
            Message::Response(r) => return Ok(Some(r)),
            Message::Request(r) => {
                let answer = match r.method.as_str() {
                    "workspace/configuration" => {
                        let n = r.params["items"].as_array().map_or(0, Vec::len);
                        Response::new_ok(r.id, vec![Value::Null; n])
                    }
                    "window/workDoneProgress/create" | "client/registerCapability" | "client/unregisterCapability" => {
                        Response::new_ok(r.id, Value::Null)
                    }
                    other => Response::new_err(r.id, -32601, format!("{other}: not supported by this client")),
                };
                self.send(answer)?;
            }
            Message::Notification(n) => {
                if n.method == "experimental/serverStatus" {
                    self.quiescent = n.params["quiescent"].as_bool().or(self.quiescent);
                }
                let _ = self.notes.send(n);
            }
        }
        Ok(None)
    }
}

impl Drop for Backend {
    /// A backend not stopped does not leave its server running.
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Why a call has no result.
enum Failure {
    Rpc(ResponseError),
    Other(String),
}

impl Failure {
    fn message(&self) -> String {
        match self {
            Failure::Rpc(e) => format!("{} ({})", e.message, e.code),
            Failure::Other(m) => m.clone(),
        }
    }
}

fn empty(v: &Value) -> bool {
    v.is_null() || v.as_array().is_some_and(Vec::is_empty)
}

/// A `file://` uri for an absolute path, percent-encoded.
pub fn file_uri(path: &Path) -> Uri {
    let mut s = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => s.push(b as char),
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    Uri::from_str(&s).expect("an encoded path is a uri")
}

/// The file a `file://` uri names, symlinks resolved when it exists.
fn canonical(uri: &str) -> PathBuf {
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
    let path = PathBuf::from(String::from_utf8_lossy(&path).into_owned());
    path.canonicalize().unwrap_or(path)
}

/// What a plain client can do, as far as these questions go: code actions
/// as literals with a lazy `edit`, definitions as links.
fn capabilities() -> Value {
    let kinds = ["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source"];
    json!({
        "general": {"positionEncodings": ["utf-8", "utf-16"]},
        "workspace": {"configuration": true, "workspaceFolders": true},
        "window": {"workDoneProgress": true},
        "textDocument": {
            "synchronization": {},
            "definition": {"linkSupport": true},
            "rename": {"prepareSupport": false},
            "formatting": {},
            "rangeFormatting": {},
            "codeAction": {
                "codeActionLiteralSupport": {"codeActionKind": {"valueSet": kinds}},
                "dataSupport": true,
                "resolveSupport": {"properties": ["edit"]},
                "disabledSupport": true,
            },
        },
        "experimental": {"serverStatusNotification": true},
    })
}

/// An intent's meaning, from the language server.
pub struct LspResolver {
    pub backend: Arc<Mutex<Backend>>,
}

impl LspResolver {
    fn lock(&self) -> std::sync::MutexGuard<'_, Backend> {
        self.backend.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Resolver for LspResolver {
    /// The declaration is where `definition` at `at` points, in `doc`.
    fn locate(&mut self, doc: &Weave, _: &Syntax, at: Chars) -> Result<Range<Chars>, String> {
        let mut b = self.lock();
        b.set_text(&doc.text())?;
        let found = b.definition(doc.metrics_at(at).bytes)?;
        let first = found.first().ok_or("no declaration in this document there")?;
        Ok(doc.chars_at_byte(first.start)..doc.chars_at_byte(first.end))
    }

    /// Rename, code action and format run as the server's own verbs on
    /// `doc`; its edits to this document are the replacements.
    fn run(&mut self, intent: &Intent, doc: &Weave, _: &Syntax) -> Result<Vec<(Range<Chars>, String)>, String> {
        let text = doc.text();
        let bytes = |r: Range<Chars>| doc.metrics_at(r.start).bytes..doc.metrics_at(r.end).bytes;
        let range = |first, last| span(doc, first, last).map(bytes).ok_or("the range is gone");
        let mut b = self.lock();
        b.set_text(&text)?;
        let edits = match intent {
            Intent::Rename { decl, from, to } => {
                let Some((at, true)) = doc.offset_of(*decl) else { return Err("the declaration is gone".into()) };
                let at = doc.metrics_at(at).bytes;
                if !text[at..].starts_with(from.as_str()) {
                    return Err(format!("the declaration no longer reads {from:?}"));
                }
                b.rename(at, to)?
            }
            Intent::Action { first, last, kind, title } => b.action(range(*first, *last)?, kind, title)?,
            Intent::Format { first, last } => b.format(range(*first, *last)?)?,
        };
        Ok(edits.into_iter().map(|(r, t)| (doc.chars_at_byte(r.start)..doc.chars_at_byte(r.end), t)).collect())
    }
}
