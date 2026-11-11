# v1

Live capture through an LSP server that owns the weave (VISION section 8,
item 7 first). Rust, on top of v0's core (`../v0`: the weave, the log, the
session) until v1's own items 1 to 6 exist.

```text
editor <-LSP-> editor::Front <-> v0::live::Session <-socket-> peers
                                       |
                         backend::LspResolver <-LSP-> rust-analyzer,
                                                      basedpyright, ...
```

`v1 lsp --socket PATH -- SERVER ARGS...` is the language server an editor is
pointed at. Toward the editor it is a server: it owns the document, and the
editor's buffer is a view that lags it. Toward the language's own server
(`SERVER ARGS`) it is the client: it lends that server the document to ask
what a name means. The first file opened with a tree-sitter grammar
(`.rs`, `.py`, `.md`) becomes the live document. Whoever finds nobody
listening on PATH hosts it; the others join, are filled with the host's
text, and relay through it.

An edit that means something (a rename, a code action, a format) is an
event in the log that says what was meant; what it did to the text is
recorded after it. So a rename made while a peer was offline still reaches
the call the peer added meanwhile. That is the point, and
`tests/helix.rs` shows it with two real editors.

## Helix

`~/.config/helix/languages.toml` (Helix 25.07), the socket the same for
everyone who edits together:

```toml
[language-server.v1-rust]
command = "v1"
args = ["lsp", "--socket", "/tmp/doc.sock", "--", "rust-analyzer"]

[language-server.v1-python]
command = "v1"
args = ["lsp", "--socket", "/tmp/doc.sock", "--", "basedpyright-langserver", "--stdio"]

[[language]]
name = "rust"
language-servers = ["v1-rust"]

[[language]]
name = "python"
language-servers = ["v1-python"]
```

Automatic pairs, completion and format on save get in the way of a test
that types characters; the test turns them off (`editor.auto-pairs`,
`editor.auto-completion`, `editor.auto-format` all `false`). Nothing in
`v1` needs that.

## What works

Measured in `tests/server.rs` (a fake client over stdio, rust-analyzer) and
`tests/helix.rs` (two Helix instances in tmux; Rust and Python):

- Typing in either editor reaches the other, in both directions; a guest's
  buffer is replaced by the document when it joins.
- Rename (`space r` in Helix: the rename-to prompt starts with the name
  under the cursor, so `C-u` clears it first). The reply is an empty
  `WorkspaceEdit`; the effect arrives as a `workspace/applyEdit` push. A
  rename of something that is no declaration, or that would touch another
  file, is answered with an error saying why.
- Code actions (`space a`): the backend's list with the edits stripped, each
  a command `v1.intent` carrying `[kind, title, range]`; choosing one is an
  intent. Tested in `tests/server.rs`; not driven through Helix's menu.
- `:format` and range formatting. rust-analyzer refuses range formatting
  on a stable rustfmt, so `Backend::format` formats the whole document and
  keeps the edits inside the lines of the range.
- Going offline and coming back: `:lsp-workspace-command v1.hold` and
  `v1.release`. Both ends hold, make their edits apart, release, and the
  rename has reached the call that had not seen it.
- Hover, definition, type definition, implementation, references, document
  symbols, document highlights and completion are passed to the backend,
  with positions converted client -> bytes -> backend and back. Locations
  in other files are returned as the backend gave them.
- The backend's diagnostics for the document, in the client's columns, once
  the client's buffer is the text they were made for.
- `shutdown` and `exit` stop the backend and, for the host, remove the
  socket.

## What does not

- One live document per server process. Other files opened in the editor
  get no answers from `v1 lsp`; and closing the live document, or opening
  it again, ends it (the client is told; restart the server).
- A `didChange` the front cannot follow (a position outside its mirror, a
  version that does not advance) ends the document the same way. There is
  no resync.
- A refused push with an empty queue (a client that refuses without having
  typed) is not retried; the session's message is lost until the next
  typing. Helix always has a preceding `didChange` in that case.
- No timeout for a `workspace/applyEdit` whose answer never comes.
- The loop is one thread and the backend is asked synchronously: a slow
  language server holds the editor's pushes up. A first legitimate empty
  answer from a server that reports no status (basedpyright) costs up to a
  minute.
- Diagnostics the backend made for older text may be shown in the columns
  of the newer, until it publishes again.
- A code action's range is the client's at listing time, read against its
  buffer at execution.
- Not passed through: formatting options (the backend gets tabs of 4,
  spaces), `prepareRename`, completion resolve, signature help, semantic
  tokens, inlay hints, workspace-wide requests. The set of advertised
  capabilities is fixed at `initialize`, before the backend is known.
- Only `utf-8` and `utf-16` positions; `utf-32` is never chosen.
- Not measured: documents beyond a few lines, symlinked project roots other
  than macOS's `/var`, a backend that ignores `shutdown`, lone `\r` line
  endings through the front.

## Tests

```sh
cd v1 && nix shell nixpkgs#basedpyright -c pkg env rust -- cargo nextest run
```

They start real programs (rust-analyzer, basedpyright, Helix, tmux) and fail
when one is missing. `.config/nextest.toml` gives those that start a language
server 60 s periods and kills them after three.
