//! The editor's side: an LSP client (Helix, VS Code, Neovim's built-in)
//! speaking to us as its language server, translated to and from
//! [`v0::live`]'s messages. Pure: no I/O, so a fake client drives it in a
//! test. [`Front`] plays the part `editors/nvim` and `editors/emacs` play
//! for `v0 live`; the session counts in bytes ([`v0::live::Unit::Bytes`]).
//!
//! # The protocol, measured on Helix 25.07 (2026-11-11)
//!
//! - `workspace/applyEdit` carrying a `textDocument.version` the client has
//!   moved past is refused: `{applied: false, failureReason: "document has
//!   changed"}`. One carrying the current version is applied.
//! - An applied edit comes back as a `didChange` at version + 1, *before*
//!   the `applyEdit` response. That `didChange` is the echo, not typing.
//! - Without a version an edit is applied unchecked: never send one.
//! - Helix offers position encodings utf-8, utf-32 and utf-16.
//!
//! So there is at most one push in flight. `didChange`s that arrive while
//! one is are queued; when the answer comes, an applied push's first queued
//! change is its echo (it confirms: [`FromEditor::Seen`]) and the rest are
//! typing, against the pushed state; a refused push confirms nothing and
//! every queued change is typing against the state before it -- an edit
//! typed behind, which the session reads where it was typed. Session
//! messages that arrive meanwhile wait their turn; when one is taken, the
//! `acked` rule of [`v0::live::ToEditor::Apply`] decides whether it still
//! fits, as a plugin would.

use lsp_types::{Position, PositionEncodingKind, TextDocumentContentChangeEvent, TextEdit};
use v0::live::{FromEditor, ToEditor};

/// How the client counts columns: negotiated at `initialize`. utf-8 when
/// the client offers it (Helix, Neovim), else utf-16 (the default, VS Code).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Encoding {
    Utf8,
    Utf16,
}

impl Encoding {
    /// The encoding to choose from what a client offers.
    pub fn choose(offered: Option<&[PositionEncodingKind]>) -> Encoding {
        todo!("{offered:?}")
    }

    pub fn kind(self) -> PositionEncodingKind {
        todo!()
    }
}

/// An edit to push to the client: `workspace/applyEdit` on the live
/// document at `version`, the version the client is believed to be at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Push {
    pub version: i32,
    pub edits: Vec<TextEdit>,
}

/// One live document as the client shows it.
pub struct Front {
    // TODO: the client's buffer as last known (a mirror), its version, the
    // encoding, edits sent to the session, the last `Apply` confirmed, the
    // push in flight, queued didChanges, waiting session messages.
}

/// What the front has for the two sides after an event.
#[derive(Default, Debug)]
pub struct Out {
    pub session: Vec<FromEditor>,
    pub push: Option<Push>,
}

impl Front {
    /// The document as `didOpen` gave it. For a host, the session then
    /// needs this text as its first edit ([`Front::opened`]).
    pub fn new(encoding: Encoding, text: String, version: i32) -> Front {
        todo!("{encoding:?} {text} {version}")
    }

    /// A host's first message: the whole buffer typed into an empty
    /// document. A guest instead replaces the buffer with the session's
    /// text: [`Front::heard`] of [`ToEditor::Ready`] pushes it.
    pub fn opened(&mut self) -> Vec<FromEditor> {
        todo!()
    }

    /// The client's `didChange`.
    pub fn changed(&mut self, version: i32, changes: Vec<TextDocumentContentChangeEvent>) -> Out {
        todo!("{version} {changes:?}")
    }

    /// A message from the session.
    pub fn heard(&mut self, msg: ToEditor) -> Out {
        todo!("{msg:?}")
    }

    /// The client's answer to the push in flight.
    pub fn answered(&mut self, applied: bool) -> Out {
        todo!("{applied}")
    }

    /// `textDocument/rename` at `at`, in the client's coordinates.
    pub fn rename(&mut self, at: Position, to: String) -> FromEditor {
        todo!("{at:?} {to}")
    }

    /// `at`, in the client's coordinates, as a byte offset of the buffer as
    /// the client shows it. For requests passed through to the backend.
    pub fn offset(&self, at: Position) -> Option<usize> {
        todo!("{at:?}")
    }

    /// The buffer as the client is known to show it.
    pub fn text(&self) -> &str {
        todo!()
    }
}
