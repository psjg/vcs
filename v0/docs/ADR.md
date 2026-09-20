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
