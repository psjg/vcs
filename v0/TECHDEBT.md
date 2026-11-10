# Tech debt

Shortcuts taken deliberately. Repay or record why not.

- **Capture is diff-at-record**, so the op log is a *reconstruction* of what
  happened, not a recording. Accepted for v0 (PRD non-goal); it is the exact
  weakness an event-sourced capture front-end removes.
- ~~**Line granularity**~~ — replaced by characters, ADR-0012.
- **No persistence format stability.** The on-disk log is serde JSON, one
  event per line in `events.jsonl` since 2026-11-10 (appended, never
  rewritten; repositories with an `events.json` must re-`init`); it will
  change without migration. `changes.json` and `head.json` are still
  rewritten whole per save: they grow with changes, not keystrokes.
- ~~**Paths are opaque strings**~~ -- 2026-11-10: `record` mints a `Dir`
  node per path component and hangs files under them. Before that a file in
  a subdirectory was created at the root under its last component (`status`
  showed it modified and deleted at once; `checkout` wrote a stray copy).
  Still none: empty directories, and removing a directory node when its
  last file goes.
- **No antichain minimisation of `deps`.** A change stores every dependency its
  ops reference, including ones implied transitively by another dependency.
  Deliberate: `closure(S)` is identical either way, so this is normalisation
  (smaller sets, canonical ids) rather than semantics. v0 instead *reports* how
  much minimisation would shrink the average dep set — data first, then decide.
- **Move is only as good as the capture.** A diff-based adapter cannot observe
  `MoveLine`; it sees a delete and an insert. v0 therefore under-reports moves,
  which is the sharpest illustration of what coarse capture costs and should be
  measured, not hidden.

## Found by using the CLI, 2026-09-21

- ~~**Silent data loss: removing a file loses concurrent edits to it.**~~
  **Fixed 2026-09-21, ADR-0011** — file-level conflicts and `Restore`. Alice
  renames `oud.txt` (capture sees Remove + Create), Bob edits `oud.txt` at the
  same time; after sync Bob's edit is gone and **no conflict is reported**.
  Conflicts are only detected on atoms, never on nodes, so a plain `rm`
  concurrent with an edit does the same. Correctness bug, not a heuristic gap —
  first in line.
- ~~**A rename severs history.**~~ Fixed 2026-11-10: `capture::renames`
  pairs a vanished file with an arrived one (identical, else most alike at
  or above half its lines, git's `-M` default) and `record` writes a
  `MoveNode` plus a diff against the old atoms, as one change. A concurrent
  edit to the old path lands in the renamed file. It is a guess, made once:
  a wrong pairing is in the log as a fact, and the remedy is `drop`.
- **A moved line is torn into two independently adoptable changes.** Delete at
  the old position and insert at the new one share no referent and sit in
  different hunks, so ADR-0007 splits them; adopting one half loses the line. A
  concurrent edit of the moved line yields a real but misleading conflict.
  **Not fixed at capture, deliberately (2026-11-10).** `MoveRun` moves a
  whole run with everything anchored in it, and a line is rarely a run: a
  file recorded in one go is one run. Detecting identical delete/insert
  pairs would fire only for lines that happen to be their own run and
  nothing else. The real fix is a move of a *range* of a run -- an op and a
  replay change -- and with live capture a move is a fact anyway, so it
  belongs to v1's front-end, not to a second guess here.
- ~~**Adjacent concurrent inserts are not flagged.**~~ Fixed 2026-11-10,
  ADR-0016: `Kind::Near`.
- **Conflict survival is judged by line identity.** An inline edit that extends
  one side's line reads as that side losing (ADR-0009).
- **Import is quadratic.** `import-git` re-materialises the whole log for every
  commit: 21 s for fpl's 76 commits, ~90 s for 250 wiki commits. Incremental
  materialisation would fix it; irrelevant until histories get large.
- **Line content is not integrity-checked, and `author` is `$USER`.** `ChangeId`
  hashes event ids, deps and meta but no text, and nothing is signed. Both wait
  on the structure/content split — see `../shelved/history-presentation-redaction-provenance.md`.


## After character granularity, 2026-09-21

- **"Too near" stops at adjacency** (ADR-0016). Edits with nothing between
  them conflict; edits a few characters apart on one line still merge
  silently, `x = 1` changed to `x = 2` on one side and `y = 1` on the other
  included. That one needs syntax, not a character class.
- **Near-conflict detection walks a document per concurrent pair.** Fine
  while resolve-first keeps pairs few; an index when a profile says so.
- **`MoveRun` moves whole runs only.** Moving part of a run needs run splitting.
  Nothing emits moves yet, so it is latent.
- ~~**Recursion depth across runs.**~~ Fixed 2026-11-10: the walk keeps its
  own stack on the heap; `tests/deep.rs` replays and opens a 100 000-keystroke
  session typed forwards and backwards. Replay at 200 KB stayed at ~25 ms.
- ~~**Per-structure limits**, **no size limits on incoming events**~~ --
  `limits.rs`, 2026-11-10: text and name bytes, parents and links per event,
  events per exchange, bytes per tracked file; an event over one is refused
  at `integrate` like a Lamport violation. Still unbounded: a peer's
  *changes* (message length, events per change), since the CLI's `sync`
  reads the peer's whole store anyway and labels have no door of their own
  yet; and changes per `record`, which is local.
- **Replay allocates freely.** It builds a throwaway structure per call — the
  textbook arena case (allocate, read, free at once). Worth it once measured.
- **Closure sizes are in events**, and an event is now a run, so closure and
  growth figures are not comparable with the line-era numbers in FINDINGS.
  Measure in characters or changes when comparing across granularities.
- **On-disk format changed.** Repositories created before ADR-0012 cannot be
  read; re-`init`.

## Tooling, 2026-09-21

- **Profile memory with `--features system-alloc`.** The TigerStyle allocator is
  opaque to Instruments and the macOS memory tools (ADR-0013 addendum). The
  budget and the profiling view are mutually exclusive in one binary; a
  Tracy `ProfiledAllocator` around talc could show both, if it is ever worth it.
- **Run tests with `pkg env rust -- cargo nextest run`.** `cargo test` has no
  timeout; nextest kills a test after 30 s.
- **v0 is on PATH through `~/dev/vcs/.envrc`**, not `cargo install`. After
  `direnv allow`, `v0` resolves to `v0/target/release/v0`; build it first.

## Hooks, 2026-09-21

- Only `pre-record` exists. `post-sync`/`post-checkout` (rebuild after the
  working copy changed) wait for a use.
- Under `--despite-conflicts` the hook sees files still showing markers, which
  `record` then skips: it judges slightly more than is recorded.

## Live weave, 2026-09-21

- `FragmentSummary::max_loc` is a `Vec`: every summary sum clones one. An
  `Arc<[u32]>` or a fixed-width key when a profile says so.
- `Weave::parents`/`children` are `BTreeMap`s, so a snapshot clones them in
  O(runs) while the trees clone in O(1).
- Every open weave holds the whole repository's run state. Fine while
  documents are few; a repository-level run store shared by its weaves,
  when a profile says so. (A move carries only its block now; only a late
  move that undoes another still lays the whole document out.)
- `subtree_last` scans a run's anchor points, so placing after a run with
  many children anchored in it costs their number, not the tree's depth.
- The CLI tests' `fresh()` directories in `$TMPDIR` are never removed; a
  day of test runs left dozens behind.

## Structure layer, 2026-11-10

Built to the skeleton's contract (`syntax.rs`, four tests). What it is not:

- **Not in the CLI.** Diff capture cannot see a rename intent, so nothing
  records `Op::Rename`, and nothing runs `reapply` at sync. The open
  question is ownership, not plumbing: `M` reads change sets, and a derived
  event belongs to no change. v1's answer is content-addressed ids
  (`theory/VISION.md`); v0 has the library and the acceptance test.
- **Scopes are syntax, not semantics.** tree-sitter-rust ships no
  `locals.scm` (the skeleton assumed one), so `Scopes` is a hand-written
  walk: one file, no paths, no `use`, no struct-literal shorthand, no
  tuple-struct constructors, macro tokens taken as uses; an associated
  function or constant cannot be renamed at all (it is reached by path or
  method call, which this does not resolve). A review found six more edges
  the same day, now pinned in `tests/syntax.rs`; expect others. A language
  server is the real resolver.
- **Rows count a lone `\r` as a line break** (LSP's rule, `Metrics`), where
  tree-sitter counts only `\n`. Nothing here reads a point back, so it only
  skews tree-sitter's own position reports; a grammar whose scanner reads
  columns (Markdown's does) could notice in a file with old Mac line ends.
- **Markdown is the block grammar only**; the inline grammar is a second
  parse per paragraph that nothing needs yet.
- **tree-sitter allocates with C `malloc`**, outside the TigerStyle budget.
- **`references` and `reapply` copy the document's text** once per call to
  read node text. Fine for an intent; not for a keystroke path.
- **`follow` folds one `apply`'s edits into one covering edit.** A move
  across a large file marks everything between as changed for the
  incremental parse. Correct, and slower than two edits would be.

## Live sessions and the editor plugins, 2026-11-10

`v0 live`, `src/live.rs`, `editors/`. A demonstration of the weave and the
structure layer in two real editors, not a product:

- **A session is not a repository.** In memory, gone with its editors;
  nothing is written to `.v0`, no change is recorded. Joining the two is
  the same open question as intents in the CLI (who owns a derived event),
  plus one more: which change a keystroke belongs to.
- **The transport is undefended.** A Unix socket, one JSON line per batch,
  a line as long as the peer makes it; a guest is sent the host's whole
  log in one line. The host relays between guests and takes the session
  with it when it exits. Events still pass `integrate`'s limits.
- **A weave clone per event, and per unconfirmed message.** `Weave` is not
  a persistent structure (its maps are copied), so a keystroke costs the
  document's size here; `tests/live.rs` runs 256 small cases in ~7 s.
  Share the run state (above) before using this on a large file.
- **Every remote batch re-asks every rename** (`reapply`: a scope walk
  and a copy of the text each). Index renames by the name they left.
- **Neovim diffs the whole buffer per change** against a shadow string and
  replaces whole lines for a peer's edit, so the cursor does not follow an
  insert before it on its own line. Emacs is exact.
- **Undo is the editor's own.** It undoes peers' edits too, as new edits
  of this replica. An undo that names events is v1's.
- **A rename covers one file,** like the layer under it.

