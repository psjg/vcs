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
