# Editor plugins for `v0 live`

Two editors on one document, each a replica: what is typed in one appears in
the other, and a rename is an intent that reaches code the other side typed
without having seen it. Neovim and Emacs, because both report edits exactly
and both run headless, so `tests/editors.rs` drives the real editors.

The plugins hold no document. `v0 live <socket> <file name>` does
(`src/live.rs`): the plugin starts one per buffer and exchanges JSON lines
with it. Whoever finds no one on the socket hosts; the next one joins and
gets the host's text. `.rs` and `.md` files only: those have a grammar.

## Try it

Two terminals, `v0` on PATH (`~/dev/vcs/bin/v0`).

Neovim, hosting:

    nvim --cmd 'set rtp+=~/dev/vcs/v0/editors/nvim' -c 'lua require("v0live")' /tmp/alice/demo.rs
    :V0Live /tmp/demo.sock

Emacs, joining (an empty buffer; it is filled from the host):

    emacs -Q -l ~/dev/vcs/v0/editors/emacs/v0-live.el /tmp/bob/demo.rs
    M-x v0-live-start RET /tmp/demo.sock RET

Then, for the rename: `:V0Hold` in Neovim and `M-x v0-live-hold` in Emacs
(both offline); in Neovim put the cursor on `foo` and `:V0Rename bar`; in
Emacs type another call to `foo`; `:V0Release` and `M-x v0-live-release`.
The call typed in Emacs reads `bar` in both.

## The video

`demo.sh` plays the rename scene in both editors, side by side in tmux, and
renders it: `echo 'v0/editors/demo.sh v0/target/demo' | pkg try asciinema
asciinema-agg` writes `v0/target/demo/v0-live.mp4` (37 s). It types into the
real editors through tmux, on a tmux server of its own.

## The protocol

One JSON object per line, one spelling each (`tests/live.rs` pins them).
Offsets are in the editor's unit: characters by default, bytes with
`--unit bytes`.

| from the editor | |
|---|---|
| `{"edit":{"at":A,"del":D,"text":T,"seen":N}}` | D units at A became T |
| `{"rename":{"at":A,"to":T,"seen":N}}` | rename the name at A |
| `{"seen":N}` | applied message N |
| `{"hold":true}` / `{"hold":false}` | go offline / come back |

| to the editor | |
|---|---|
| `{"ready":{"role":R,"text":T}}` | up; a guest shows T, a host sends its buffer |
| `{"apply":{"n":N,"acked":K,"edits":[{"at","del","text"}]}}` | apply, in order |
| `{"error":"..."}` | refused; nothing changed |

`seen` is the last `apply` the editor applied. An editor applies an `apply`
only if `acked` equals the number of edits and renames it has sent, and
drops it otherwise: it has typed since, the offsets no longer fit, and the
session sends what the message meant again once it has heard the edit. That
rule is the whole of what a third plugin has to get right.
