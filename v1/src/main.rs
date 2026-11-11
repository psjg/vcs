//! `v1 lsp --socket PATH -- SERVER ARGS...`: the language server an editor
//! is pointed at. Owns one live document (the first it opens with a
//! grammar), shares it with peers on PATH as `v0 live` does, and runs the
//! language's own SERVER for meaning (`v1::backend`). Everything else the
//! editor asks is passed through to SERVER.

fn main() {
    todo!("the loop: client on stdio, the session, peers on the socket, the backend")
}
