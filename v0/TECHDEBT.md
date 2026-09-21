# Tech debt

Shortcuts taken deliberately. Repay or record why not.

- **Capture is diff-at-record**, so the op log is a *reconstruction* of what
  happened, not a recording. Accepted for v0 (PRD non-goal); it is the exact
  weakness an event-sourced capture front-end removes.
- ~~**Line granularity**~~ — replaced by characters, ADR-0012.
- **No persistence format stability.** The on-disk log is serde JSON; it will
  change without migration.
- **Single file per repo is not assumed, but paths are opaque strings** — no
  rename tracking, no directory semantics.
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
- **A rename severs history.** Diff capture never emits `MoveNode`: the new file
  is born with zero dependencies and fresh atoms, so `deps` and any future blame
  start at the rename. Fix at capture: detect remove+create with similar content
  in one record and emit `MoveNode` plus a diff against the old atoms (git's
  `-M`, but decided once and recorded as a fact).
- **A moved line is torn into two independently adoptable changes.** Delete at
  the old position and insert at the new one share no referent and sit in
  different hunks, so ADR-0007 splits them; adopting one half loses the line. A
  concurrent edit of the moved line yields a real but misleading conflict. Fix
  at capture: identical delete/insert pairs in one record become `MoveLine`.
- **Adjacent concurrent inserts are not flagged.** Two peers inserting different
  lines at the same spot merge silently in a deterministic order. Manyana's
  "too near" heuristic (adjacent or whitespace-separated) is the reference.
- **Conflict survival is judged by line identity.** An inline edit that extends
  one side's line reads as that side losing (ADR-0009).
- **Import is quadratic.** `import-git` re-materialises the whole log for every
  commit: 21 s for fpl's 76 commits, ~90 s for 250 wiki commits. Incremental
  materialisation would fix it; irrelevant until histories get large.
- **Line content is not integrity-checked, and `author` is `$USER`.** `ChangeId`
  hashes event ids, deps and meta but no text, and nothing is signed. Both wait
  on the structure/content split — see `../shelved/history-presentation-redaction-provenance.md`.


## After character granularity, 2026-09-21

- **"Too near" is more urgent now.** Two edits a few characters apart on one
  line merge silently where the line model called them a conflict. Usually
  right (different words), sometimes not (`x = 1` changed to `x = 2` on one side
  and `y = 1` on the other in the same expression). Manyana's adjacency rule is
  the reference.
- **`MoveRun` moves whole runs only.** Moving part of a run needs run splitting.
  Nothing emits moves yet, so it is latent.
- **Recursion depth across runs.** The walk loops within a run but recurses from
  one run to another. A live editor emitting one event per keystroke makes each
  keystroke its own run, and a long typing session nests them. Needs an explicit
  stack before live capture lands.
- **Per-structure limits (the rest of TigerStyle).** ADR-0013 bounds the
  process; nothing yet bounds individual structures — events per sync, run
  length, file size, changes per record. Fixed, named limits would make a
  wasteful data model visible as a limit being hit rather than as a larger
  number in a budget.
- **No size limits on incoming events.** Ranges are clamped, but nothing bounds a
  run's length or an event's size at sync. Put a limit on everything that comes
  from outside (TigerStyle / Power of 10).
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
