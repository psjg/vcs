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

**Status:** PROPOSED, 2026-09-20. Needs a human call: it changes what a change *is*.

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
