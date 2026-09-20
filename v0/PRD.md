# v0 — a version control spike

A [version control system] whose name is a version number. Each rewrite gets the next one.

## The wager

Patch theory and CRDTs are usually presented as rivals. They are layers:

0. **capture** — an append-only log of operations with identity (the CRDT layer)
1. **meaning** — which *subsets* of that log are coherent changes (the patch-theory layer)
2. **identity** — where a definition's identity lives (Unison's content-addressing; out of scope here)

Over an event graph with a deterministic materialisation function `M(S)` on
dependency-closed subsets, the heavy machinery of darcs-style patch theory
collapses into set operations:

```
merge(A, B)   = M(S_a ∪ S_b)
cherry-pick G = M(S_target ∪ closure(G))
unrecord G    = M(S \ upward_closure(G))
reorder       = free — M does not depend on order
```

Commutation stops being an operation and becomes a property of `M`. What
remains of patch theory is exactly one question: **what is the minimal
dependency relation?**

## What this spike must answer

1. Can a change's **minimal dependencies** be derived structurally from the ops
   themselves (anchor/target references), rather than from causal order?
2. Does `unrecord` leave every other change **byte-identical**? (The property
   git cannot offer and pijul sells.)
3. Does the weave keep concurrent insert blocks **non-interleaved**?
4. What does it cost when capture is coarse (diff-at-record) instead of live?
5. Do two replicas that sync by state vector converge — i.e. is the same log
   both the collaboration substrate and the version store?

## Success

A green property-test suite asserting the algebra (see DESIGN.md §Invariants),
driven through a CLI that can `init / record / log / adopt / drop / merge`
across two working copies on a plain POSIX filesystem.

## Explicit non-goals for v0

- *Transport* for liveness: no sockets, no WebRTC, no server. The sync **core**
  is in scope and property-tested between in-memory replicas; only the wire is
  deferred.
- A live capture front-end (editor plugin). The model does not care where ops
  come from; v0 ships the degraded diff-at-save adapter.
- Filesystem or kernel integration (FUSE/FSKit). See the wiki page
  `time-travel operating systems` for why that is a separate research problem.
- Networking, auth, a server, binary files, conflict *resolution* UX.
- Performance. Correctness of the algebra first; benchmarks once it works.
