# Handoff — v0, 2026-11-10

Read this first in a new session. Everything below is in the repo; nothing lives
only in a chat. Supersedes the handoff of 2026-09-21, which predates links
(I15), checkpoints (I16), I17, the xanadu spike and the theory merge.

## What this is

`v0/` is a version-control spike: an append-only, peer-to-peer event log
(character-level Fugue CRDT, Lamport-ordered, state-vector sync) with patch
theory on top — changes are labels over events, dependencies are derived, and
merge / cherry-pick / revert are set algebra over `M(S)`. The name is a joke
that is also the plan: rewrites become `v1`, `v2`. v1 is designed in
`theory/` (VISION, FORMAL, THEORY.org) and will be written in FPL; v0 stays
as the oracle and is not rewritten.

Read in this order: `v0/PRD.md` → `v0/DESIGN.md` (invariants I1–I17) →
`v0/docs/ADR.md` (0001–0015, newest last) → `v0/docs/FINDINGS.md`
(measurements) → `v0/TECHDEBT.md` → `theory/VISION.md` §8 (what moved to v1).
Parked designs: `shelved/`. Xanadu on the atoms: `v0/xanadu/transclusion/`.

## Where v0 stands (measured 2026-11-10)

- `cargo nextest run`: 84 passed, 3 skipped (the structure-layer skeleton).
- `v0/xanadu/transclusion`: 9 of 9.
- Every PRD question has a FINDINGS section; I1–I16 each have a test
  (I15 is `a_link_is_replicated_and_never_closed_over`); I17 is held by
  construction only.
- CLI: `init status record log show deps adopt drop checkout sync conflicts
  resolve checkpoint reduction`. The PRD's `merge` is `sync` + `adopt` (FINDINGS, "The
  CLI, end to end").
- Live weave: ~6 µs a keystroke at any size, moves 42–69 µs, `append` 84 ns
  (FINDINGS 2026-09-21). Not wired to any front-end; replay is the oracle.

By its own PRD the spike is answered. What follows is finishing, not research.

## Working on it

- `cd ~/dev/vcs` loads the dotfiles `rust` profile via direnv. `v0` on PATH
  is `bin/v0`, which rebuilds when sources changed.
- Tests: `cd v0 && pkg env rust -- cargo nextest run`. Nextest kills anything
  past 30 s; never plain `cargo test` for new, possibly-hanging tests.
- Memory: every v0 process has a TigerStyle budget (`--memory MB`, default
  512, exit 5 when exhausted). Profile with `--features system-alloc`.
- Measure: `cargo run --release --bin import-git -- <repo>` (fpl is the
  reference; its FINDINGS numbers must not move without a reason).
  Keystrokes: `bench/weave.sh`, raw data in `bench/results/`.
- Commits: `~/.claude/bin/acommit`; body lines ≤ 72; the hook rejects bundled
  commits — split instead of overriding. The repo is block-first
  (`.git/info/exclude` ignores `*`): new top-level paths need a `!/path` line.

## Hard-won lessons

- A test fixture shorthand (`WHOLE = (0, u32::MAX)`) bypassed its resolver and
  expanded four billion positions; the Mac froze. Hence `op::clamp`, the memory
  budget, and nextest timeouts. Run new tests under a limit.
- `EventId.seq` must stay a Lamport clock, or every last-writer-wins rule
  silently prefers the older write. It must also exceed everything an event
  links to: you cannot cite the unseen (f51098a).
- "Insert after X" is not "insert between X and Y": always place through
  `capture::between` (Fugue's rule).
- Line diffs → word diffs → tokens cut at authoring-event boundaries: each step
  came from a failing test (letter soup "goedniag", fused words across authors).
- The first keystroke benchmark measured a bug (a `Vec` cloned into every
  summary), not the structure. Profile before trusting a sweep.
- `~/dev/dotfiles` is shared with other sessions and holds foreign uncommitted
  work. Commit there only from a private index (`GIT_INDEX_FILE`).

## Finishing v0, in order

Each item lands as its own commit with its test; TECHDEBT and DESIGN move
with it. Scope is "finish what v0 began"; anything VISION §8 assigns to v1
(hashes, one relation, worlds/`goto`, attestation, transport, LSP capture)
stays out, however tempting.

1. ~~I17 as a test, append-only event file~~ — done 2026-11-10:
   `events.jsonl`, one event per line, torn tail never served or built on
   (`i17_a_torn_append_is_never_served_and_never_built_on`).
2. ~~`checkpoint` in the CLI~~ — done 2026-11-10: `v0 checkpoint [TREE]`.
3. ~~Explicit stack in the run walk~~ — done 2026-11-10 (`tests/deep.rs`).
4. **Limits on everything from outside.** Run length, event size, events per
   sync, changes per record: fixed, named, refused at `integrate`, visible
   as a limit hit rather than a larger number in the budget (TigerStyle).
5. **"Too near" conflicts.** Adjacent concurrent edits merge silently;
   Manyana's adjacency rule is the reference. It changes the conflict
   definition, so it gets ADR-0016 first, then the predicate.
6. **Rename and move at capture.** Remove+create with similar content in one
   record becomes `MoveNode` plus a diff against the old atoms; identical
   delete/insert pairs become `MoveRun`. Decided once, recorded as a fact
   (TECHDEBT "a rename severs history", "a moved line is torn").
7. **The structure layer** (`syntax.rs`, `Op::Rename`, three skeleton tests).
   Memory of the design session says it must fit v1's definition identity;
   VISION puts ops on definitions under "beyond the four layers". Decide
   with the human whether v0 finishes it or shelves it with a note; the
   skeleton must not stay half-built in either case.

Known residue that stays unless a profile says otherwise: `max_loc` as
`Vec`, `BTreeMap` parents/children, one run store per open weave,
`subtree_last` scanning anchor points, `fresh()` test dirs never removed.

The human's playground (`/tmp/v0play/{alice,bob}`) predates ADR-0012: reset
both with `rm -rf .v0 && ~/dev/vcs/bin/v0 init`.
