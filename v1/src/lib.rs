//! v1, starting at VISION §8 item 7: live capture via an LSP server owning
//! the weave as buffer. Written in Rust, not FPL (PSJ, 2026-11-11), and on
//! top of v0's core until items 1-6 replace it.
//!
//! ```text
//! editor <-LSP-> editor::Front <-> v0::live::Session <-socket-> peers
//!                                        |
//!                          backend::LspResolver <-LSP-> rust-analyzer,
//!                                                       basedpyright, ...
//! ```
//!
//! Two LSP connections, one each way. Toward the editor, `v1 lsp` is the
//! language server: it owns the document, and the editor's buffer is a
//! view that lags it ([`editor`]). Toward the language's own server it is
//! the client: it lends that server the document to ask what a name means
//! ([`backend`]). The weave and the merge are language-blind; tree-sitter
//! finds the candidates; the language server decides what they mean. So a
//! language works when it has both a tree-sitter grammar and a server.
pub mod backend;
pub mod editor;
