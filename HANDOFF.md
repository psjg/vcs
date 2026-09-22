# Handoff — v0, 2026-09-21

Read this first in a new session. Everything below is in the repo; nothing lives
only in a chat.

## What this is

`v0/` is a version-control spike: an append-only, peer-to-peer event log
(character-level Fugue CRDT, Lamport-ordered, state-vector sync) with patch
theory on top — changes are labels over events, dependencies are derived, and
merge / cherry-pick / revert are set algebra over `M(S)`. The name is a joke
that is also the plan: rewrites become `v1`, `v2`.

Read in this order: `v0/PRD.md` → `v0/DESIGN.md` → `v0/docs/ADR.md` (0001–0014,
newest last) → `v0/docs/FINDINGS.md` (measurements) → `v0/TECHDEBT.md`.
Parked designs: `shelved/` (history presentation + redaction + crypto;
granularity + structure + checkpoints).

## Working on it

- `cd ~/dev/vcs` loads the dotfiles `rust` profile via direnv (it extends
  `systems`: samply, tracy, bloaty, llvm). `v0` on PATH is `bin/v0`, which
  rebuilds when sources changed. Outside the repo: `~/dev/vcs/bin/v0`.
- Tests: `cd v0 && cargo nextest run` — 45 tests; nextest kills anything past
  30 s. Never plain `cargo test` for new, possibly-hanging tests.
- Memory: every v0 process has a TigerStyle budget (`--memory MB`, default 512,
  exit 5 when exhausted). Profile memory with `--features system-alloc`.
- Measure: `cargo run --release --bin import-git -- <repo>` (fpl is the
  reference; its numbers are in FINDINGS and must not move without a reason).
- Commits: `~/.claude/bin/acommit`; body lines ≤ 72; the hook rejects bundled
  commits — split instead of overriding. The repo is block-first
  (`.git/info/exclude` ignores `*`): new top-level paths need a `!/path` line.

## Hard-won lessons from today

- A test fixture shorthand (`WHOLE = (0, u32::MAX)`) bypassed its resolver and
  expanded four billion positions; the Mac froze. Hence `op::clamp`, the memory
  budget, and nextest timeouts. Run new tests under a limit.
- `EventId.seq` must stay a Lamport clock, or every last-writer-wins rule
  silently prefers the older write.
- "Insert after X" is not "insert between X and Y": always place through
  `capture::between` (Fugue's rule).
- Line diffs → word diffs → tokens cut at authoring-event boundaries: each step
  came from a failing test (letter soup "goedniag", fused words across authors).
- `~/dev/dotfiles` is shared with other sessions and holds foreign uncommitted
  work. Commit there only from a private index (`GIT_INDEX_FILE`), then align
  the shared index entry for your own paths.

## Open, in rough priority

1. **v1: live capture** instead of diffing saves. Removes the diff-guessing
   behind most capture fixes and makes moves and renames facts. Not an
   editor plugin. Candidates, cheapest verifier first:
   - an ed/sam-style command protocol on stdin (`x/re/ c/text/` maps onto
     Insert/Delete against anchors), which tests drive with no editor;
   - a small LSP server that consumes `textDocument/didChange` incremental
     edits: every editor already sends per-keystroke deltas that way, so one
     server covers all of them with no plugins. Evaluate before writing any.
   The human types in "any editor", which favours the LSP route.
   Live capture needs anchor <-> line:col (UTF-16) in O(log n): a sum tree
   of runs, built by us (the human prefers no lock-in), borrowing designs.
   Checked 2026-09-21: Zed's `sum_tree` is Apache-2.0 (on crates.io as
   `zed-sum-tree` 0.2.0, which trails Zed's main) -- code may be borrowed with
   attribution. Zed's `text` and `rope` are GPL-3.0: ideas only (fragments
   tree plus insertions tree keyed by insertion id, visible and deleted text
   kept apart). Also study: ropey (chunk metrics), diamond-types
   `content-tree`/jumprope (run-length items), loro `generic-btree` (MIT,
   built for a CRDT). The current `Vec<Atom>` replay stays as the oracle.
   **Built 2026-09-21:** `sumtree.rs` (own persistent B-tree, B a const
   parameter) and `weave.rs` (live document: from_walk, coordinates incl.
   LSP UTF-16 points with \n/\r\n/\r breaks, apply, insert_op/delete_ops),
   converging with replay over any causal order -- moves included (within
   and between documents, into hiding and back), and sync refuses events
   that break the Lamport order (I14). The human shelved the benchmark
   and front-end until MoveRun was tested; that is done, so **next** is
   the per-keystroke benchmark -- **done**: a keystroke is ~6 µs at any
   document size, B = 16 and MAX_FRAGMENT = 128 confirmed by data
   (FINDINGS). `EventLog::append` is incremental now (270 µs -> 84 ns),
   and a move carries only its block (32 ms -> 42 µs at 200 KB), with
   the full relayout kept as the fallback for a late move that undoes
   another. **Next:** the ed/sam or LSP front-end, with an append-only
   event file instead of rewriting events.json per save.
2. **"Too near" conflicts** — adjacent concurrent edits merge silently
   (TECHDEBT). A heuristic that changes the conflict definition: its own ADR.
3. **Hooks** (ADR-0015): `.v0/hooks/pre-record` exists. Next ones only on need.
4. **Structure/content split** (events commit to `hash(content)`): the
   foundation for redaction, integrity and signing — see `shelved/`.
5. Per-structure limits (the rest of TigerStyle), checkpoints as first-class
   objects, rename/move detection in capture (only if no front-end).

The human's playground (`/tmp/v0play/{alice,bob}`) predates today's conflict
changes: reset both with `rm -rf .v0 && ~/dev/vcs/bin/v0 init`.
