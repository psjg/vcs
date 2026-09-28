# spikes/ — "implementation = model", in two languages

The question (VISION §7, FORMAL §3): can the implementation carry the model's
invariants itself, so that "the weave computes `lt`" is a theorem about the
code rather than a property test? Two one-day spikes, same content in each:
the world operations of §2, and the weave's ordering of §3.3 with a sorted
insertion that keeps its invariant.

## Dafny (`dafny/`)

- `Worlds.dfy`: the order abstract (two axioms, as the Lean `Poset` class),
  `IsIdeal`, `Convex`, `Adopt`, `Drop`; lemmas `AdoptIdeal`, `DropIdeal`,
  `AdoptDiff`, `DropDiff` (2.8), `CoverB` (2.7(b)). **5 verified**, three
  edit rounds: two were missing preconditions the SMT solver asked for
  (`S <= U`), one was a ghost/compiled boundary (below).
- `Fugue.dfy`: `Lt` on `seq<Step>` as a *compiled* function — the one the
  weave would call — with `Irrefl`, `Trans`, `Total` proved by induction
  (each lemma is three lines: Dafny finds the rest), `Sorted`, `Insert`,
  `InsertSorted`, `InsertMembers`. **9 verified, first try.** Sabotaging
  `Insert` (insert first unconditionally) fails verification at the
  postcondition, as it should.

Finding: with the order left abstract, `Adopt`/`Drop` cannot be compiled —
a ghost predicate cannot be called from executable code. To make them
executable one needs a *decidable* `Le` (closure over a finite `dep` map)
and a proof that it agrees with the abstract order. That is exactly the
"implementation = model" obligation, made visible by the type checker:
the implementation must compute the order, and the proof that it computes
the *right* order is a separate theorem. Dafny makes that theorem cheap
where the objects are `seq`/`set` and the reasoning is first-order.

## ATS2 (`ats/`)

- `sorted.dats`: a list of keys indexed by its static contents (`ilist`)
  and length; dataprops `LB` (lower bound), `ORD` (sorted), `INSERT`
  (insertion relation); proof functions `lb_weaken`, `lb_insert`; the
  executable `insert` returns the new list *together with* `ORD ys` and
  `INSERT (x, xs, ys)`. Compiles to C via `patscc`, runs (`2 4 5 9`).
  Two edit rounds (a missing static binder in a pattern). Sabotaging the
  comparison fails with an unsolved constraint `x <= y` at the exact line.

Finding: ATS can carry the invariant *in the implementation* — the
proof is part of the value the function returns, and the constraint
solver checks arithmetic side conditions for free. But every proof is
written by hand as a recursive proof function with an explicit
termination metric, and the statics can only speak about ints, bools,
addresses and datasorts you define (`ilist`); there is no set theory, so
`IsIdeal` over a world of events would have to be encoded as a datasort
of event-lists with a dataprop for down-closure — feasible, laborious.
Paths with (side, key) steps would need a custom datasort, since `ilist`
is ints only.

## What this says about FPL's type system

Both spikes agree on the *shape* of what is needed; they differ on who
does the work.

| need | Dafny | ATS2 |
|---|---|---|
| a computable order with a proof it matches the model | required by the ghost/compiled split; proof via SMT | required by construction; proof by hand |
| sortedness / ideal-ness as a type of the result | postcondition (`ensures`) | proof term in the return tuple |
| arithmetic side conditions (`x <= y`, indices) | automatic | automatic (statics + constraint solver) |
| induction over lists/paths | automatic given a 3-line skeleton | manual, with termination metric |
| set-level invariants (`IsIdeal`) | native (`set<Ev>`) | not expressible in the statics without encoding |
| linear resources (the append-only log, buffers) | not in the language | native (`viewtype`, `@`) |
| running the result | compiles to C#/Go/…; ghost erased | compiles to C; proofs erased |

Requirements for FPL, derived:

1. **Refinement of ints/bools by an SMT-backed solver** (both have it;
   both spikes leaned on it for every `x <= y`).
2. **Postconditions or proof-carrying returns for compiled functions** —
   either style works; the erasure at runtime is the same.
3. **A decidable order in the runtime and a stated theorem relating it to
   the abstract one** — the one obligation neither language lets you skip.
4. **Set-valued ghost state**, or the discipline is unbearable: `IsIdeal`
   should be writable as a predicate over a set, not encoded as a datasort.
5. **Automation for structural induction** — Dafny's cost per lemma was
   minutes; ATS's was the whole spike.
6. **Linear types are a separate axis** and only ATS has them; they are
   what would make "a body is served once and then purged" a type rather
   than a policy.

Reading: Dafny answers "is it worth proving the implementation?" — yes,
cheaply, for the order and the data structures. ATS answers "can the
implementation *be* the proof?" — yes, expensively, and only for what its
statics can say. FPL should take Dafny's automation and set-level ghost
state and ATS's linearity; the proof-carrying-return style is a matter of
taste, and the ghost/compiled boundary is the thing to get right first.
