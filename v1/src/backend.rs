//! The language's side: its own language server (rust-analyzer,
//! basedpyright, ...) run as our child, with us as its client. We lend it
//! the live document, as whatever text we need judged, and ask it what a
//! name means. [`LspResolver`] is the [`v0::live::Resolver`] that does so.
//!
//! The server sees one document, at the path the editor opened, in the
//! workspace the editor named, so it resolves it in its crate or package.
//! Its text is set whole (`didChange` with full content) before every
//! question; between questions it is the session's document
//! ([`Backend::set_text`]). A server that is still loading answers with
//! nothing or with `ContentModified` (-32801): ask again, within a bound.

use lsp_types::{Location, Position, Uri};
use std::ops::Range;
use std::path::Path;
use std::time::Duration;
use v0::live::{Refs, Resolver};
use v0::syntax::Syntax;
use v0::weave::{Chars, Weave};

/// How long one question may wait for a server that is loading.
pub const ANSWER_WAIT_MAX: Duration = Duration::from_secs(60);

/// A language server, running, with one document open.
pub struct Backend {
    // TODO: the child, its connection, the next request id, the uri, the
    // text it holds and its version, its position encoding, and where its
    // notifications (diagnostics) go.
}

impl Backend {
    /// Start `command` (e.g. `["rust-analyzer"]`) in `root`, initialize it
    /// (offering utf-8 and utf-16 positions), and open `uri` with `text`.
    pub fn spawn(command: &[String], root: &Path, uri: Uri, language_id: &str, text: &str) -> std::io::Result<Backend> {
        todo!("{command:?} {root:?} {uri:?} {language_id} {text}")
    }

    /// Make the server's copy of the document read `text`.
    pub fn set_text(&mut self, text: &str) {
        todo!("{text}")
    }

    /// `textDocument/definition` at byte `at` of the current text: where it
    /// points, in this document, as byte offsets.
    pub fn definition(&mut self, at: usize) -> Result<Vec<Range<usize>>, String> {
        todo!("{at}")
    }

    /// `textDocument/references` at byte `at`, the declaration included,
    /// in this document, as byte ranges.
    pub fn references(&mut self, at: usize) -> Result<Vec<Range<usize>>, String> {
        todo!("{at}")
    }

    /// Any other request, passed through: `params` with positions already
    /// in this server's coordinates for its current text.
    pub fn request(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        todo!("{method} {params}")
    }

    /// The position of byte `at` of the current text, in the server's
    /// encoding; and back.
    pub fn position(&self, at: usize) -> Position {
        todo!("{at}")
    }

    pub fn offset(&self, at: Position) -> Option<usize> {
        todo!("{at:?}")
    }

    /// Shut the server down: `shutdown`, `exit`, wait.
    pub fn stop(self) {
        todo!()
    }
}

/// A rename's meaning, from the language server.
pub struct LspResolver {
    pub backend: std::sync::Arc<std::sync::Mutex<Backend>>,
}

impl Resolver for LspResolver {
    /// The declaration is where `definition` at `at` points; the
    /// references are `references` at it, this document only.
    fn references(&mut self, doc: &Weave, syntax: &Syntax, at: Chars) -> Result<Refs, String> {
        todo!("{:?} {:?} {at:?}", doc.text(), syntax.tree())
    }

    /// A candidate binds when `definition` at it, in `doc`, points at the
    /// declaration.
    fn binds(&mut self, doc: &Weave, syntax: &Syntax, decl: Chars, candidates: &[Range<Chars>]) -> Vec<bool> {
        todo!("{:?} {:?} {decl:?} {candidates:?}", doc.text(), syntax.tree())
    }
}

#[allow(unused)]
fn _location(_: Location) {}
