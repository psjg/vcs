# DESIGN — what v0 *is*

Standing document. The data structures, the operations over them, and the
invariants that must hold. Changes rarely; specs are written against it.

## Ubiquitous language

| term | meaning |
|---|---|
| **op** | one indivisible edit with an identity: insert a line, or delete one |
| **atom** | a line in the weave, alive or tombstoned, carrying the id of the op that created it |
| **weave** | every atom that has ever existed, in one permanent total order |
| **change** | a *named set* of ops — the unit a human records, reviews, adopts or drops |
| **dependency** | change A depends on B when an op of A anchors to or targets an atom created by B |
| **change set** | a dependency-closed set of changes; the only thing that can be materialised |
| **materialise** | `M(S)`: fold a change set into a document |

A change is **not** a snapshot and **not** a diff between two states. It is a
label over ops. That is the whole trick: labels can be unioned and subtracted
while the ops underneath keep their identity.

## Data structures

```rust
OpId    { replica: ReplicaId, seq: u32 }      // totally ordered, globally unique
Anchor  = Start | After(OpId)                  // where an insert attaches
Op      = Insert { id, anchor, line: String }
        | Delete { id, target: OpId }          // deletes are ops too, so they can be dropped
Change  { ops: Vec<Op>, deps: BTreeSet<ChangeId>, meta: Meta }
ChangeId = blake3(canonical(ops, deps, meta))  // content-addressed
Log      { changes: BTreeMap<ChangeId, Change> }
ChangeSet(BTreeSet<ChangeId>)                  // smart constructor: refuses non-closed sets
Weave    { atoms: Vec<Atom> }                  // the materialiser's working state
Document { lines: Vec<String> }                // what lands in the working copy
```

`ChangeId` being a content hash is what makes invariant **I3** structural rather
than enforced: dropping a change cannot alter any other change's identity,
because no other change's bytes mention it unless it is a genuine dependency.

## Ordering rule — why it must be universal

Two replicas that insert at the same anchor must land in the same order
*everywhere and forever*, or two people resolving the same conflict produce
`AXYB` and `AYXB` and merging those has no good answer. v0 orders siblings at
an anchor by `OpId` descending (later replica first, stable tiebreak on
`replica`). The property that matters is not *which* order but that it is
total, deterministic and permanent.

## Dependency derivation — the research question

For each op in change `C`:

- `Insert { anchor: After(x) }` depends on the change that created atom `x`
- `Delete { target: y }` depends on the change that created atom `y`

Collect those, drop `C` itself, then **minimise to an antichain**: if `X` is
reachable from `Y` in the dependency graph, `X` is implied and is removed.

This is a *structural* derivation — no causal order, no "everything I had
seen". Whether it is the right notion of minimal is exactly what the spike is
testing.

## Operations

```
materialise(set)        -> Document          M(S)
merge(a, b)             -> ChangeSet         a ∪ b            (both closed ⇒ union closed)
adopt(set, c)           -> ChangeSet         set ∪ closure(c)
drop(set, c)            -> ChangeSet         set \ upward_closure(c)
record(set, path, text) -> Change            diff against M(set), mint ops, derive deps
```

There is no `rebase`, no `cherry-pick` and no `revert`: `adopt` *is*
cherry-pick, `drop` *is* revert, and reordering is meaningless because `M` does
not read order.

## Invariants — the test suite is the deliverable

| # | invariant |
|---|---|
| **I1** | `M` is a pure function of the *set*: same set ⇒ byte-identical document |
| **I2** | merge is commutative, associative and idempotent |
| **I3** | `drop(S, C)` leaves every other change's `ChangeId` byte-identical |
| **I4** | a `ChangeSet` cannot be constructed non-dependency-closed (type-level) |
| **I5** | `adopt(drop(S, C), C) == S` for any `C ∈ S` |
| **I6** | concurrent insert blocks never interleave: two branches inserting at the same anchor yield one block then the other |
| **I7** | `drop` of a change nothing depends on changes only the lines that change touched |

**I6 is the one likely to fail.** RGA-family orderings are known to interleave
in some concurrent cases; Fugue (Weidner & Kleppmann) is the ordering that
provably does not. v0 implements the simple rule and lets the property test
answer it. A failure here is a *result*, not a bug — it tells us the spike
needs Fugue's anchor scheme.

## Shape of the code

Functional core, imperative shell:

- `op.rs`, `change.rs`, `weave.rs`, `repo.rs` — pure. No I/O, no clock, no env.
- `store.rs` — the only module that touches the filesystem.
- `main.rs` — argv in, exit code out.

Exit codes follow the pijul-skill lesson (conflicts are a *state*, not an
error): `0` success, `1` failure, `2` usage, `3` dependency violation.
