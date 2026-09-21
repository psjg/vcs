# DESIGN — what v0 *is*

Standing document: the data structures, the operations over them, and the
invariants that must hold.

## The three layers

```
  layer 2   identity      content-addressed definitions (Unison)      — out of scope
  layer 1   meaning       which SUBSETS of the log are coherent       — THE SPIKE
  layer 0   capture       append-only event graph, replayable         — substrate
```

Layer 0 is what an event-sourced system already gives you: every edit appended
with an identity and the **causal parents** the author had seen. Layer 1 is the
only thing patch theory still has to contribute once layer 0 exists.

## Ubiquitous language

| term | meaning |
|---|---|
| **event** | one appended edit: an identity, its causal parents, and an op |
| **op** | the payload: insert a line after an anchor, or delete one |
| **event graph** | the whole append-only log, a DAG by causal parents |
| **causal deps** | what the author had seen — recorded, free, and far too large |
| **minimal deps** | what the change actually requires — *derived*, and the point |
| **change** | a label over a set of events: the unit humans record and adopt |
| **change set** | a set of changes closed under *minimal* deps — materialisable |
| **replay** | `M(S)`: fold a change set into a document by re-walking its events |

## Layer 0 — the event graph

```rust
EventId { replica: ReplicaId, seq: u32 }        // totally ordered, globally unique
Event   { id: EventId, parents: Vec<EventId>, op: Op }
EventLog{ events: BTreeMap<EventId, Event> }    // append-only, never rewritten
```

`parents` is the author's frontier at append time — exactly what a live capture
front-end produces for free, and exactly what makes cherry-pick useless if you
take it literally: adopting one event would drag in everything its author had
ever seen.

### The op set

A mature CRDT is not `insert-after` plus `delete`. Since we own the materialiser
(ADR-0004) the op set is a *design choice*, and for a version control system the
choice that earns its keep is **first-class move**:

```rust
Op =
    // --- sequence: lines inside documents -------------------------------
    | Insert   { anchor: Anchor, line: String }   // anchor names an atom, never an index
    | Delete   { target: EventId }
    | MoveLine { target: EventId, to: Anchor }    // identity preserved, ACROSS documents too
    // --- tree: the working copy is a tree of nodes ----------------------
    | Create   { node: NodeId, parent: NodeId, name: String, kind: NodeKind }
    | MoveNode { node: NodeId, parent: NodeId, name: String }   // rename == move
    | Remove   { node: NodeId }
    // --- register: per-node metadata ------------------------------------
    | SetMode  { node: NodeId, mode: u32 }        // last-writer-wins by EventId

Anchor  = DocStart(NodeId) | After(EventId)
```

**Why move is the load-bearing addition.** Expressed as delete-plus-insert, a
moved function loses its identity, so every system downstream has to *guess* it
was a move: `git log --follow` and `-M` are heuristics over similarity scores.
As an op, a move is a fact. The consequences are concrete:

- a rename is `MoveNode`, so rename tracking stops being detection;
- moving a function to another file is a `MoveLine` batch whose **dependency is
  the moved atom, not the surrounding text** — which makes it cherry-pickable;
- concurrent moves get the literature's answers rather than ad-hoc rules:
  Kleppmann's cycle-safe move for trees, deterministic winner-by-id for lines.

**Cost, stated plainly.** Every op family adds concurrency cases (move/move,
move/delete, move-into-own-subtree). That cost is paid in invariants I10–I11,
not in cleverness. Rich-text marks, counters and maps beyond `SetMode` are out
of scope: they buy nothing for code.

### Concurrency rules

| situation | rule |
|---|---|
| concurrent `Insert` at one anchor | total order by `EventId`; siblings never interleave (**I6**) |
| concurrent `MoveLine` of one atom | highest `EventId` wins; the atom exists once, never duplicated (**I10**) |
| `MoveLine` concurrent with `Delete` | delete wins — a dead atom is dead wherever it was moved |
| concurrent `MoveNode` forming a cycle | on replay, skip the move that would close the cycle, deterministically by `EventId` (**I11**) |
| concurrent `SetMode` | last writer wins by `EventId` |

Events are appended by a **capture adapter**. v0 ships the degraded one
(diff-at-save, ADR-0003); a live editor or FS interceptor is a drop-in
replacement that appends to the same log. Nothing above layer 0 knows which
adapter ran — but note that a diff-based adapter can only *guess* `MoveLine`,
which is the clearest illustration of what coarse capture costs.

## Layer 1 — the reduction, which is the whole spike

Two relations over the same graph:

```rust
causal_parents(e)   // recorded at capture: e.parents — everything the author had seen
semantic_refs(e)    // read off the op itself:
                    //   Insert   -> anchor          MoveLine -> target + destination anchor
                    //   Delete   -> target          MoveNode -> node + new parent
                    //   Create   -> parent node     Remove   -> node
                    //   SetMode  -> node
```

**The claim under test:** the minimal dependency of a change is generated by
`semantic_refs`, not by `causal_parents` — and the reduction is enormous.

```rust
Change   { events: BTreeSet<EventId>, deps: BTreeSet<ChangeId>, meta }
ChangeId = blake3(canonical(events, deps, meta))   // content address
ChangeSet(BTreeSet<ChangeId>)                      // closed under `deps`; smart constructor
```

`deps` is derived: map each event's `semantic_refs` to the change that owns the
referenced event and drop self-references. That is the whole rule.

*No antichain minimisation.* Reducing `deps` to the changes not implied by
others is a **normalisation, not a semantic step** — `closure(S)` is identical
either way, since a transitively implied dependency is pulled in regardless.
It buys smaller stored sets and a canonical id, and costs a reachability pass
plus a class of subtle bugs. v0 keeps the raw derived set and *measures* what
minimisation would have saved (TECHDEBT). Because `ChangeId` is a content hash and no change's bytes mention a change it
does not depend on, dropping one cannot perturb another's identity (**I3**).

The headline measurement is a ratio: `|causal closure| / |minimal closure|`
over real histories. If that number is ~1 the layer buys nothing and the design
is wrong. If it is large, cherry-pick is cheap and the wager holds.

## Liveness: the log is shared before any change exists

`EventId = (replica, seq)` with monotone `seq` means a map `{replica -> max seq}`
is a complete **state vector**: it says exactly which events a peer holds. That
is Yjs's mechanism, and it falls out of the identity scheme rather than being
bolted on.

```rust
StateVector(BTreeMap<ReplicaId, u32>)
state_vector(log)          -> StateVector          // what I have
missing(log, &theirs)      -> Vec<Event>           // what they lack
integrate(log, events)     -> ()                   // idempotent, order-insensitive
```

Two peers converge in one exchange each way: send state vectors, send the
difference. No server, no central ordering — the same shape as `y-webrtc`.

**The consequence for versioning is the whole point of this project.** Live
edits append events that belong to *no change at all*. A change is a label
applied to a subset of the log, possibly long after the typing happened. So:

- collaborating and committing stop being two mechanisms over two stores;
- `record` is renaming, not writing — the ops already exist;
- an unnamed event is ordinary, not an error; the log is simply ahead.

Transport is out of scope for v0 (PRD non-goal). The sync *core* is pure, so
convergence is a property test between two in-memory replicas — no network
needed to prove the design (**I12**, **I13**).

## Materialisation is replay, not a merge algorithm

```rust
M(set) -> Document      // replay the set's events in deterministic topological order
```

Replay walks the events of a set — ordered by `(semantic dependency, then
EventId)`, never by wall-clock or by causal order — and builds a throwaway
sequence structure (a weave of atoms) to read the document off. This is
eg-walker's move: keep the log, build the CRDT state transiently at replay,
discard it.

Because `M` reads a **set**, merge needs no algorithm and commutation is not an
operation but a property:

```
merge(a, b)   = M(a ∪ b)
adopt(set, c) = M(set ∪ closure(c))
drop(set, c)  = M(set \ upward_closure(c))
reorder       = meaningless — M never reads order
```

Ordering is **Fugue** (Weidner & Kleppmann, arXiv:2305.00583). Each line is a
node with a parent *and a side*; reading order is the in-order traversal — left
children, the node, then right children — with ties between same-parent,
same-side siblings broken by `EventId`. Placement follows one rule, applied at
capture time:

> to insert between left neighbour `a` and right neighbour `b`: if `a` is **not**
> an ancestor of `b`, become a **right child of `a`**; if it **is**, become a
> **left child of `b`**.

Those two lines are what keep one person's block contiguous under merge. The
order must also be total, deterministic and permanent, or two people resolving
the same conflict produce `AXYB` and `AYXB` and merging *those* has no good
answer.

## Why the materialiser is ours and not Loro's

Not because libraries "enforce causal closure" — that is a metadata check, not a
law. It is simpler: `M` applies **our** ordering rule to **our** event
identities over an **arbitrary** subset. A library's value (incremental merge,
compact encoding, years of fuzzing) is orthogonal to that and is a *performance*
swap, not a semantic one — see ADR-0004. `Materialiser` is a trait so a
Loro-backed implementation can be benchmarked against ours once the semantics
settle.

## Invariants — the test suite is the deliverable

| # | invariant |
|---|---|
| **I1** | `M` is a pure function of the *set*: same set ⇒ byte-identical document |
| **I2** | merge is commutative, associative and idempotent |
| **I3** | `drop(S, C)` leaves every other change's `ChangeId` byte-identical |
| **I4** | a `ChangeSet` cannot be constructed unclosed (type-level) |
| **I5** | `adopt(drop(S, C), C) == S` |
| **I6** | concurrent insert blocks never interleave — typed forwards *or* backwards |
| **I7** | `drop` only perturbs lines the dropped change touched |
| **I8** | minimal deps ⊆ causal deps — the reduction never invents a dependency |
| **I9** | replaying a minimal closure yields the same lines for the change's own events as replaying the causal closure — *the reduction loses nothing* |
| **I10** | concurrent `MoveLine` of one atom leaves exactly one copy of it |
| **I11** | concurrent `MoveNode` never produces a cycle or an orphan, and every replica skips the same move |
| **I12** | after a bidirectional sync, two replicas materialise byte-identical worktrees |
| **I13** | `integrate` is idempotent and order-insensitive: replaying a peer's events twice, or in any order, changes nothing |
| **I14** | every stored event is younger (higher `seq`) than its causal parents and everything its op refers to; `integrate` refuses one that is not, and the CLI refuses any change needing it |

**I9 is the spike.** It is the executable form of "minimal dependencies are
enough". **I6** is expected to fail: RGA-family orderings interleave in some
concurrent cases and Fugue is the ordering that provably does not — a red result
there is a finding, not a bug.

## Shape of the code

Functional core, imperative shell.

- `event.rs`, `op.rs`, `change.rs`, `tree.rs`, `replay.rs`, `sync.rs`, `repo.rs` — pure.
- `capture.rs` — adapters that turn observed edits into events. Swappable.
- `store.rs` — the only module that touches the filesystem.
- `main.rs` — argv in, exit code out. `0` ok, `1` failure, `2` usage, `3` not closed.
