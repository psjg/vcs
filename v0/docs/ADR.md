# Architecture decision records

Append-only. Newest last.

## ADR-0001 — own minimal weave, not an off-the-shelf CRDT library

**Status:** accepted, 2026-09-20.

Default policy is buy-over-build, and Loro/Automerge/Yjs are hardened, fast and
maintained. We still write our own sequence core, for one reason: **the spike's
research question is replay over arbitrary dependency-closed subsets, and
production CRDT libraries deliberately forbid exactly that.** They enforce
causal closure — you may not apply an op without everything that causally
preceded it — because that is what makes convergence provable. Our layer 1
needs the opposite: `unrecord` materialises `S \ upward_closure(G)`, a subset
that is dependency-closed but *not* causally closed.

Fighting a library's core invariant to run the experiment costs more than
writing ~400 lines of weave. Keep `Weave` behind a narrow interface so a Loro
backend can be measured against it once the semantics are settled.

**Cost if wrong:** we own a sequence CRDT's correctness. Mitigated by the
property tests being the deliverable, not the code.

## ADR-0002 — line granularity

**Status:** accepted, 2026-09-20.

Ops address lines, not characters. Halves the state, matches how developers
talk about diffs, and keeps a weave printable during debugging. Character
granularity is a mechanical change later (the op type gains a byte range);
AST granularity is a different project (see Unison).

## ADR-0003 — capture by diff at record time

**Status:** accepted, 2026-09-20.

v0 reconstructs ops by diffing the working file against the materialised state,
exactly as git does — and inherits the known weakness: the interpretation of an
edit is baked in at record time. The model does not depend on this; a live
capture front-end (editor plugin, FS interceptor) can replace it without
touching layers 1 and 2. Recording it as an ADR so the limitation is not later
mistaken for a property of the design.

## ADR-0004 — supersedes ADR-0001: why the materialiser is ours

**Status:** accepted, 2026-09-20. Supersedes ADR-0001's reasoning, keeps its conclusion.

ADR-0001 argued that off-the-shelf CRDT libraries "enforce causal closure" and
therefore cannot materialise our subsets. That is a metadata check, not a law,
and it was the wrong justification.

The real reason is narrower and survives scrutiny: `M(S)` applies **our**
ordering rule to **our** event identities over an **arbitrary** dependency-closed
subset. That is the semantics under test. A library's value — incremental merge,
columnar encoding, years of fuzzing — is orthogonal to it. So the materialiser is
ours, `Materialiser` is a trait, and a Loro-backed implementation is a
*performance* comparison to run once the semantics settle, not a dependency to
design around.

Corollary: owning the materialiser means the **op set is a design choice**, which
is what makes ADR-0005 possible.

## ADR-0005 — first-class move ops

**Status:** accepted, 2026-09-20.

`MoveLine` and `MoveNode` are ops, not delete-plus-insert. Rationale: a move
expressed as delete+insert destroys identity, which is why git needs similarity
heuristics (`-M`, `--follow`) to *guess* renames. As an op it is a fact, its
dependency is the moved atom rather than the surrounding text, and concurrent
moves get answers from the literature (Kleppmann's cycle-safe tree move;
deterministic winner-by-id for lines) instead of ad-hoc rules.

**Cost:** three new concurrency cases (move/move, move/delete, move-into-own-
subtree), paid for in invariants I10–I11. Accepted because rename and
code-motion tracking is a core VCS job that every snapshot system fakes.

## ADR-0006 — dependencies per event, not per change

**Status:** WITHDRAWN, 2026-09-20, superseded by ADR-0007 before anyone had to
decide. The re-labelling experiment showed the problem it solved was not
inherent — see FINDINGS. Kept for the record because the reasoning was sound
given the data available at the time.

Measurement (docs/FINDINGS.md) shows the change-level dependency closure grows
with history at the same rate as the causal closure it was supposed to replace —
222 → 1663 events over 76 commits in `fpl`, 1434 → 4335 over 211 in the wiki.
The event-level closure is flat over the same histories (142 → 228, 163 → 119).

Cause: a change is adopted whole, so a commit touching several unrelated files
welds their histories together, permanently, for every later change.

**Proposal:** keep `Change` as the human-facing label, but compute closure over
events via `Op::refs` rather than over changes via `deps`. `ChangeSet` becomes a
dependency-closed *event* set.

**Cost:** adopting a change may take only part of another change. The resulting
document is coherent, but "I adopted commit X" is no longer true, and a UI has
to explain partial ancestry. Pijul chose the other side of this trade and pays
the contamination instead.

**Not decided here.** The data is unambiguous about the cost; what a change
*means* is the user's call.


## ADR-0007 — `record` partitions by dependency component

**Status:** accepted, 2026-09-20.

`record` does not take a set of events on trust. It partitions the pending
events into connected components of the semantic-reference graph and produces
**one change per component**, which the human names.

Measured justification (FINDINGS): adopting a change costs 222 → 1663 events
across `fpl`'s history under git's own commit boundaries, and 110 → 123 under
component splitting. Flat, on the same events. In the messier history the cost
*falls*, 1434 → 4335 becoming 129 → 70.

**Consequences.**

- A change stays a whole, human-meaningful unit, so ADR-0006's trade never has
  to be made.
- Commit hygiene becomes a computation rather than a discipline: you cannot
  accidentally weld unrelated work together, because the machine draws the
  boundary from what the edits actually reference.
- One "commit" may become several changes. The UI has to make that pleasant —
  name them together, adopt them together if you like — but they stay
  independently adoptable, which is the entire point.
- The rule is a heuristic about *reference*, not about *intent*: two edits that
  belong together conceptually but touch nothing in common will be split. The
  escape hatch is an explicit "these are one change" override, which must be
  recorded as such rather than silently merging the components.


## ADR-0008 — Fugue for the ordering

**Status:** accepted, 2026-09-20. Closes the last open question in the
materialisation model.

The first ordering was RGA-family: one anchor per line, siblings sorted by
`EventId`. Measured behaviour (FINDINGS): two peers typing three lines each
*forwards* from a shared anchor stayed contiguous, but typing *backwards* —
every line inserted above the last, so all share one anchor — produced a perfect
alternation, `base b3 a3 b2 a2 b1 a1`.

Replaced with **Fugue**: a node carries a parent *and a side*, reading order is
the in-order traversal, and placement uses `between(a, b)`. Both interleaving
tests now pass.

**Consequences.**

- `Op::Insert` gains a `side`, and `MoveLine` with it. Placement is decided at
  capture, so replay stays a pure walk and `refs()` — the dependency input — is
  unchanged: still one parent per line.
- Capture needs ancestry, which it reads straight from the log, because every
  insert stores its own parent. No extra index.
- A caller can no longer say "put this after X" and be right by default. The
  naive right-child placement is wrong exactly when X is an ancestor of the line
  that follows it, and it broke one of our own tests within minutes of the rule
  landing. `capture::between` is the only correct way in.
- Import numbers are unchanged (222 → 1683 against 222 → 1663), as expected:
  Fugue changes *where lines sit*, not *what depends on what*.


## ADR-0009 — conflicts are derived objects, resolved by patches

**Status:** accepted, 2026-09-21.

Between pijul (a conflict is state in the model) and Manyana (the model is
conflict-free, a conflict is a view), v0 takes a third position that the
set-reading materialiser makes available:

- A conflict is **derived** from the change set: two changes that did not see
  each other both deleted or moved the same line, and at least one of them put
  something in its place. Two concurrent deletions agree and are not a conflict.
- It has a **stable identity**, `blake3(contested atom, sorted sides)`, so every
  replica computes the same conflict with the same id without storing anything.
- It stays **open** until a change in the set declares `resolves: [id]`. Open
  conflicts are drawn as markers on checkout, `record` refuses text that still
  contains them, and `v0 conflicts` exits 4 — a state to handle, not a crash.
- A **resolution is a patch**: whatever the resolver did to the text — pick one
  side, keep both, merge them inline, or change nothing — plus a *declared*
  dependency on every side. Adopting a resolution therefore brings its conflict;
  dropping either side drops the resolution.
- Two **concurrent resolutions** of one conflict are themselves surfaced
  (`Contested`), because left silent they could delete both people's choices.
- Nothing is rewritten. Who resolved it and why are in the change; *which side
  survived* is derived from the text rather than claimed.

**Why not git's model.** A git conflict has no identity: it lives in
`MERGE_HEAD` and disappears at commit. The merge commit records the outcome, not
which conflict it settled, which side won, or why. `rerere` remembers
resolutions only locally.

**Known limit.** Survival is judged by line identity. An inline edit that
extends one side's line counts as replacing it — the audit trail says "0/1 of
its lines survive" for a side whose content was kept and extended. Character-
or token-level capture would fix this; line-level diff capture cannot.


## ADR-0010 — event sequence numbers are a Lamport clock

**Status:** accepted, 2026-09-21.

Every "last writer wins" rule in v0 — a file renamed twice, a mode set twice, a
removal and a restore — decides by `EventId` order. `seq` was a per-replica
counter, so a replica with a low counter that acted *after* reading someone
else's work still sorted before it and lost. A failing test showed it: a rename
made after seeing another rename lost to it (`busy.txt` instead of `final.txt`).

`append` now mints `max(own counter, highest seq seen + 1)`. Causally later
events always sort later, which makes `EventId` order a linear extension of
causal order. A replica's seqs stay monotone but are no longer dense; state
vector sync is unaffected, because it only ever asks for "every event of r above
hi".

## ADR-0011 — file-level conflicts, and `Restore`

**Status:** accepted, 2026-09-21. Fixes silent data loss found by using the CLI.

Conflicts existed only on lines, so removing (or renaming — which diff capture
sees as remove plus create) a file while someone else edited it lost the edit,
with no conflict reported. Two new kinds:

- **Removal** — a change removes a node while a concurrent change edits that node
  or anything below it (a line inside, a file created in a directory, a rename).
  Concurrent removals of the same node agree and are not a conflict.
- **Rename** — concurrent moves of one node to different places.

A disputed removal is *shown*: checkout keeps the file on disk, framed by
markers naming who removed it and who edited it, so the resolver can read it.
Resolutions are the usual patches: delete the file to accept the removal (an
empty resolution, still recorded with a reason), keep it, or carry the edit
elsewhere. Keeping it is a new op, **`Restore`**, which un-removes the node with
its identity — the same lines, the same atoms, the same history — rather than
creating a new file that happens to share a name. Capture emits it whenever a
removed file's path reappears on disk.

Found alongside and fixed: checkout never deleted a file whose removal arrived
from a peer, because it asked the materialiser for "every path ever" and the
materialiser omits removed files by design. The next `record` then saw the stray
file and restored it, silently undoing the peer's removal. Pinned by a CLI test
that fails on the old code.


## ADR-0012 — character granularity (supersedes ADR-0002)

**Status:** accepted, 2026-09-21.

Lines were chosen for cheapness (ADR-0002), and Pike's critique of line-oriented
tools applied: the conflict audit misread an inline extension as a side "losing"
(ADR-0009), moves could only be whole lines, and two edits to different words
on one line were a conflict when they are not one.

**Model.** A character's identity is `Pos { event, offset }` — Zed's (insertion
id, offset) anchor. An `Insert` is one event carrying a *run* of text; only its
first character has an explicit Fugue parent and side, and each following
character is the implicit right child of the one before. `Delete` names a range
within one run. `MoveRun` moves a whole run (partial moves would split runs, and
no adapter emits moves yet). `Op::refs()` still returns event ids, so dependency
derivation, closures and components are untouched — verified: on fpl the
unit-free statistics (200 changes, 2.63 components per commit, 26% independent,
55% welding) are identical before and after. Import time fell from 21 s to 3.4 s
because an event is now a run rather than a line.

**Capture: identity per character, diffing per word, tokens cut at authors.**
Two findings from tests written to justify this change, both real:

1. *Character diffs make letter soup.* Two concurrent rewrites of "hello world"
   (to "goedendag" and "hoi") merged into `"goedniag"`: a character diff keeps
   stray letters it happens to share with the base, and each side deleted the
   letters the other kept. Inside a changed hunk the diff now works on Unicode
   words; a replaced word leaves whole and arrives whole, and the same situation
   is a clean conflict.
2. *Words fuse across authors.* After that merge the two words sit flush —
   `"goedendaghoi"` is one word to a segmenter — so a resolver keeping Alice's
   word and dropping Bob's saw one word replaced and deleted Alice's characters
   too. Old-side tokens are now also cut wherever the authoring event changes.
   With both fixes the audit reports Alice 9/9 characters kept, Bob 0.

A live editor front-end has neither problem: it knows which characters were
typed. Both fixes are costs of diff capture.

**Rendering.** A text conflict is a stretch of characters widened to whole lines.
Each side's section is the head *with the other sides taken out*, and a `before`
section is the head with all sides taken out — diff3 from set subtraction, with
no merge algorithm.

**Robustness, learned the hard way.** A test that bypassed the fixture sent
`Delete { range: (0, u32::MAX) }`; replay expanded it into four billion
positions and ran the developer's machine out of memory. The same event from a
hostile peer would do that to every replica. Every expansion of a delete range
now goes through `op::clamp`, and a regression test sends the oversized range on
purpose.


## ADR-0013 — a TigerStyle memory budget, reserved at startup

**Status:** accepted, 2026-09-21.

**Why.** A test expanded a delete range of `(0, u32::MAX)` into four billion
positions and ran the developer's Mac out of memory; the machine froze, with the
test billed to the Claude app as 7.96 GB. The range bug is fixed (ADR-0012), but
the lesson is the one TigerBeetle's style guide states: put a limit on
everything, and allocate memory at startup.

**Why not the kernel.** Measured on macOS 27: `ulimit -v` and `ulimit -d` are
refused (`setrlimit failed: invalid argument`) and a child allocates 300 MB
regardless. Linux can do it (`RLIMIT_AS`, or better cgroups v2 `memory.max`),
but a kernel kill is a bare `SIGKILL` with no explanation. The limit belongs in
the process, where it can say what happened, identically on every OS.

**What.** `budget.rs` makes talc over one reserved region the global allocator.
The first allocation (before `main`) claims a 2 MB bootstrap block; `main`
reserves the rest of the budget in one piece from `--memory MB`, `V0_MEMORY_MB`
or 512 MB; after that the OS is never asked again. Exhaustion prints the budget,
the failed request and how to raise it, then exits with code 5 — assembled on
the stack and written with `write(2)`, since the allocator is what failed and
must not be re-entered.

**Measured.** With the clamp removed on purpose, the test that froze the machine
now dies in 1.5 s at 68 MB under a 64 MB budget, with the message. Recording a
47 MB file under `--memory 16` fails the same way at a 3 MB peak. The suite runs
with peaks of 5 and 11 MB.

**Not full TigerStyle.** TigerBeetle also never allocates *inside* its budget,
because every structure has a fixed capacity. v0 still allocates and frees
within the region. Like the JVM's `-Xmx`, the budget bounds the heap, not
thread stacks or the binary.

**Addendum, 2026-09-21: profiling builds.** The budget hides allocations from
every macOS memory tool: `leaks --atExit` on `v0 status` reports one 524 331 KB
block (the reserved region) in the budgeted build, against 11 KB of individual
live allocations in a build with the `system-alloc` feature. That feature swaps
the budget for the system allocator so Instruments Allocations, `heap`, `leaks`
and `malloc_history` can see inside; `v0 status` then says "NO budget", and the
budget test is skipped. Never ship it.

Time is bounded the same way, by `.config/nextest.toml`: a test is killed after
30 s. Verified with a probe that sleeps 60 s — `TIMEOUT [30.004s]`.


## ADR-0014 — conflict hygiene: resolve first, agreement is derived, diff3 by author

**Status:** accepted, 2026-09-21. Driven by a playground session that ended with
five conflicts on one line and rendered text nobody had typed.

1. **Resolve first.** `record`, `sync` and `adopt` refuse (exit 4) while any
   conflict is open, as git refuses a merge over unmerged paths.
   `--despite-conflicts` pushes through with a warning every time, and `record`
   then skips files still showing markers.
2. **Resolve many at once.** `resolve [<id>...] [--file PATH] -m WHY`, default
   every open conflict: one edit, one reason, one change depending on all sides.
3. **Agreement is a derived, zero-op resolution.** When every side's replacement
   for a contested stretch is the same text, the conflict is `Agreed` and the
   materialiser hides all but the lowest-EventId side's runs. It must stay
   derived: a *recorded* dedupe would hard-code which copy survives, and dropping
   the kept side later would lose the text entirely. `Moot`: none of any side's
   replacement text survives. Both count as closed.
4. **Diff3 by author.** Each rendered section is the materialised causal past of
   that side's change — exactly what its author had — and `before` is the
   intersection of those pasts. Subtracting the other sides from the head only
   worked for two sides.

Verified: import numbers on fpl unchanged to the last digit (materialisation
only); a three-way conflict renders every section byte-exact; dropping either
side of an agreement leaves the text once.
