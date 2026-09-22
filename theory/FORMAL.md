# v1 — formal core

Sections 1–3 of the formalisation: events and their order, worlds, and
materialisation. Later sections (labels, conflicts, authority, task graph,
time and redaction) build on these and are not written until these survive
review.

Machine-checked, in two variants (see `lean/README.md` for the trade-off):
- core Lean 4, no Mathlib: 2.2 (sublattice half), 2.4 (both directions),
  2.6, 2.7(a), 2.7(b), 2.8, 2.9 (`lean/Worlds.lean`), 2.12
  (`lean/Components.lean`), the Fugue traversal order as a strict total
  order plus 3.3a and 3.4 (`lean/Fugue.lean`, inserts and deletes only),
  moves and I10 (`lean/Moves.lean`), holes and redaction locality
  (`lean/Holes.lean`), projections append-only (`lean/Projection.lean`),
  5.5–5.7 (`lean/Conflicts.lean`), 6.4 with and without revocations
  (`lean/Authority.lean`), 7.6 (`lean/Contraction.lean`);
- Mathlib: 2.2 in full (distributive lattice by instance, Birkhoff
  representation from `Mathlib.Order.Birkhoff`), 2.4, 2.6, 2.7(a), 2.7(b),
  2.8 (`lean/WorldsMathlib.lean`); Dilworth's theorem, cover and
  decomposition form, for 7.4 (`lean/Dilworth.lean`).
The same statements are exercised over random finite posets by
`lean/test_worlds.py` (Hypothesis).

Conventions. Every numbered statement ends with a tag: **[def]**, **[thm]**
with a proof sketch, or **[rem]**. Statements that become executable carry
**→ proptest** (a property test over random instances) or **→ invariant** (a
check the implementation enforces at runtime). Sets are finite throughout.

---

## 1. Events and the reference order

**1.1 Hash. [def]** Fix a collision-resistant hash `H : bytes → Id`. We
assume *hash acyclicity*: no finite sequence of byte strings
`h₀, …, hₙ` exists with `H(hᵢ)` occurring as a substring of `h_{i+1}` (indices
mod n+1). This follows from collision resistance up to negligible probability
and is the only cryptographic assumption sections 1–3 use.

**1.2 Event. [def]** An *event* is a pair `(header, body?)`. A header is a
tuple

```
header = (addr, author, kind, shape, deps, links, body_hash?)
addr   = (replica, hlc)          — for sync and tie-breaks only
deps   ⊆ Id                      — what the op depends on and overwrites
links  ⊆ Id                      — what the op refers to without depending on it
```

with `deps ∩ links = ∅`. The *identity* of an event is `id(e) = H(header)`.
If a body is present, `body_hash = H(body)` and the body is stored by that
hash. The body never enters `id(e)` except through `body_hash`.

**1.3 Log. [def]** A *log* `E` is a finite set of events with distinct ids.
`E` need not be closed under references: an id in `deps(e)` that names no
event in `E` is a *hole*.

**1.4 Well-foundedness. [thm]** On any log, the relation
`r ⇝ e  iff  r ∈ deps(e) ∪ links(e)` is acyclic.
*Proof.* A cycle `e₀ ⇝ e₁ ⇝ … ⇝ e₀` means `id(eᵢ)` occurs in `header(e_{i+1})`,
so `H(header(eᵢ))` is a substring of `header(e_{i+1})` around the cycle,
contradicting 1.1. ∎
**→ invariant** (cheap form): every event checks `hlc(r) < hlc(e)` for all
`r ∈ deps(e) ∪ links(e)` (this is I14 of v0 restated; it is implied by 1.4 when
clocks are honest and detects dishonest clocks otherwise).

**1.5 Dependency order. [def]** Let `≤` be the reflexive–transitive closure of
`{(r, e) : r ∈ deps(e)}` on `E`. Write `P = (E, ≤)`. Links do not enter `≤`.

**1.6 [thm]** `≤` is a partial order. *Proof.* Reflexive and transitive by
construction; antisymmetric by 1.4. ∎

**1.7 Concurrency. [def]** `e ∥ f` iff neither `e ≤ f` nor `f ≤ e`. This is
the only notion of "concurrent" in v1. There is no causal order.

**1.8 Address order. [def]** `(r, h) ≤ₐ (r', h')` iff `r = r'` and `h ≤ h'`.
This is a disjoint union of chains, one per replica. It is used by sync (a
state vector is a down-set of `≤ₐ`) and by tie-breaks, and by nothing
semantic.

**1.9 [rem]** `≤` and `≤ₐ` are unrelated orders on the same set. In
particular a replica's own events need not form a chain under `≤`, and a
sync down-set (everything a peer holds) is in general *not* an ideal of `P`:
it may contain holes. Sections 2–3 assume a log without holes; 3.5 says what
`M` does with holes.

**1.10 Dep-completeness. [def]** A log is *dep-complete* if `deps(e) ⊆ ids(E)`
for all `e ∈ E`. Links may dangle.

---

## 2. Worlds

Throughout §2, `E` is dep-complete and `P = (E, ≤)`.

**2.1 Ideal. [def]** `S ⊆ E` is an *ideal* (a world) if `e ∈ S` and `r ≤ e`
imply `r ∈ S`. Write `I(P)` for the set of ideals, `↓X` for the down-closure
of `X`, `↑X` for its up-closure, `max(S)` for the maximal elements of `S`.

**2.2 Birkhoff. [thm]** `(I(P), ⊆)` is a distributive lattice with
`S ∨ T = S ∪ T`, `S ∧ T = S ∩ T`, bottom `∅`, top `E`. The map
`S ↦ max(S)` is a bijection from ideals to antichains, with inverse
`A ↦ ↓A`.
*Proof.* Down-sets are closed under union and intersection, so `I(P)` is a
sublattice of the powerset lattice, hence distributive. For a finite poset
every element of `S` lies below some maximal element of `S`, so `S = ↓max(S)`;
conversely `max(↓A) = A` for an antichain `A`. ∎
**→ proptest** frontier round-trip: `↓max(S) = S` and `max(↓A) = A`.
**→ rem** the *frontier* `max(S)` is the stored normal form of a world.

**2.3 Thin. [thm]** Regard `I(P)` as a category with one arrow `S → T` iff
`S ⊆ T`. Every diagram in it commutes. *Proof.* At most one arrow between any
two objects. ∎
**[rem]** This is the formal content of "commutation is a property of `M`,
not an operation": there is nothing to reorder because paths are unique.

**2.4 Convex difference. [thm]** For ideals `S ⊆ T`, `D = T \ S` is
order-convex (`a ≤ b ≤ c` with `a, c ∈ D` implies `b ∈ D`), `S ∪ D = T`, and
`S ∩ D = ∅`. Conversely, for an ideal `S` and any `D`,
`S ∪ D` is an ideal iff `↓D \ D ⊆ S`. (Neither convexity nor disjointness is
needed for the converse; the machine proof made that visible.)
*Proof.* Convexity: `b ≤ c ∈ T` gives `b ∈ T`; `b ∈ S` would give `a ∈ S`.
Converse: `S ∪ D` is down-closed iff every element below some `d ∈ D` lies in
`S ∪ D`, i.e. `↓D ⊆ S ∪ D`, i.e. `↓D \ D ⊆ S`. ∎

**2.5 goto. [def]** `goto(S, T) = (T \ S, S \ T)`: the events to add and the
events to remove. By 2.4 both parts are convex; `T \ S` is added, `S \ T`
removed, and `T = (S \ (S \ T)) ∪ (T \ S)`.
**[rem]** `goto` is the unique arrow of the *pair groupoid* on `I(P)`. Every
navigation (merge, adopt, drop, revert to a checkpoint, switch fork) is an
instance.

**2.6 adopt, drop. [def]** For `C ⊆ E`:
`adopt(S, C) = S ∪ ↓C`, `drop(S, C) = S \ ↑C`. Both are ideals. *Proof.*
Union of ideals; complement of an up-set within an ideal. ∎

**2.7 Cover laws. [thm]** Let `C` be convex. Then

(a) `drop(adopt(S, C), C) = S`  iff  `C ∩ S = ∅` and `↓C \ C ⊆ S`;

(b) `adopt(drop(S, C), C) = S`  iff  `C ⊆ S` and `S \ C` is an ideal.

*Proof.* (a) If `C ∩ S ≠ ∅`, drop removes an element of `S`. If `C ∩ S = ∅`
then `↑C ∩ S = ∅` (an ideal containing something above `c` contains `c`), and
`↑C ∩ ↓C = C` by convexity, so the result is `(S ∪ ↓C) \ C = S ∪ (↓C \ C)`,
which equals `S` iff `↓C \ C ⊆ S`.
(b) If `C ⊄ S`, adopt adds something outside `S`. If `C ⊆ S`, the result is
`(S \ ↑C) ∪ ↓C`, which equals `S` iff `↑C ∩ S ⊆ ↓C`, i.e. `↑C ∩ S = C`, i.e.
nothing in `S` lies strictly above `C`, i.e. `S \ C` is down-closed. ∎
**→ proptest** both directions of both laws over random posets, random ideals
and random convex `C`.
**[rem]** For a single event `c`: (a) says `c` is absent and its dependencies
present; (b) says `c ∈ max(S)`. v0's I5 tests (b) on one fixture; this is
the law it was testing.

**2.8 Inverse pairs. [thm]** For ideals `S ⊆ T` with `D = T \ S`:
`adopt(S, D) = T` and `drop(T, D) = S`. *Proof.* 2.4 and 2.7 with `C = D`:
`↓D \ D ⊆ S` because `T` is an ideal, and `T \ D = S` is an ideal. ∎
**[rem]** This is the statement "the true inverse of drop is the removed set,
not the label". An implementation that returns `D` from `drop` and accepts
`D` in `adopt` never needs the preconditions of 2.7.

**2.9 Path independence. [thm]** For ideals `S ⊆ T`, let `e₁, …, eₙ` be any
linear extension of `(T \ S, ≤)`. Then
`adopt(…adopt(adopt(S, e₁), e₂)…, eₙ) = T`, and every step satisfies 2.7(a).
*Proof.* Induction: `Sₖ = S ∪ {e₁, …, eₖ}` is an ideal because `e_{k+1}` is
minimal among the remaining events, so `↓e_{k+1} \ {e_{k+1}} ⊆ Sₖ`. ∎
**→ proptest** two random linear extensions reach the same `T`.
**[rem]** This is 2.3 made computational, and it is the only sense in which
"reorder" exists. (The step lemma does not need `T` to be an ideal; only the
end-to-end statement does.)

**2.10 Merge is affine. [thm]** For ideals `A, B` with `base = A ∩ B`:
`A ∪ B = base ⊎ (A \ base) ⊎ (B \ base)`, the two differences disjoint.
*Proof.* `(A \ base) ∩ (B \ base) = (A ∩ B) \ base = ∅`. ∎
**[rem]** The three-way merge formula `a + b − base` of the informal
discussion is literal here, and `base = A ∧ B` is the meet, not a separately
chosen ancestor.

**2.11 Components. [thm]** If `P` is a disjoint union of posets `P₁ ⊔ … ⊔ Pₖ`
(no `≤` edges between them), then `I(P) ≅ I(P₁) × … × I(Pₖ)`.
*Proof.* A down-set of a disjoint union is a disjoint union of down-sets. ∎
**[rem]** ADR-0007's per-component `record` is the operational face of this;
independent components are independent coordinates of a world, and can be
synced, checkpointed and reasoned about separately.

**2.12 Components of pending work are convex. [thm]** Let `R` be an ideal
(the recorded world) and `Q = E \ R` the pending events. Then `Q` is an
up-set, and every connected component `K` of the graph `(Q, deps|Q)` is
convex in `P`.
*Proof.* `Q` is an up-set because `R` is a down-set. Let `a ≤ b ≤ c` with
`a, c ∈ K`. Then `b ∈ Q` (else `a ∈ R`). The chain of dep-edges witnessing
`a ≤ b` lies in `Q` (up-set), so every event on it is dep-connected to `a`,
hence in `K`; in particular `b ∈ K`. ∎
**[rem]** This is the decision 2.7 needed: `record` as in ADR-0007 already
produces convex changes, provided components are taken over `deps` alone,
not over `links`. Links must never be used to group events into changes.

---

## 3. Materialisation

**3.1 Document type. [def]** A document type `τ` is a triple
`(Op_τ, Doc_τ, M_τ)`: a set of op kinds and shapes, a set of documents
(including documents with holes, 3.5), and a *materialiser*
`M_τ : I(P) → Doc_τ` defined on the ideals of the sub-poset of events of
kind in `Op_τ` belonging to one document. A log carries several types; `M`
is their product.

**3.2 Set-function. [thm] (I1)** `M_τ(S)` depends only on the set `S`: its
events' headers and bodies and the order `≤` restricted to `S`.
*Proof.* By definition: `M_τ` is specified as a function of the set (3.3),
never of an arrival order or a replay order. ∎
**→ proptest** materialise a random ideal after permuting the log's storage
order and after receiving its events in a different sync order; documents
are byte-identical (I12, I13 of v0 as one test).

**3.3 Intrinsic order for text. [def]** For the text type, atoms are
`Pos = (event, offset)` inside insert runs. Each run has a Fugue parent and
side (in its shape). `M_text(S)` is the in-order traversal of the Fugue tree
spanned by the runs in `S`, with same-parent, same-side siblings ordered by
`(hlc, id)`, omitting atoms killed by a delete in `S` and relocating runs
moved by a move in `S`. This is a definition, not a replay: any replay order
that rebuilds the same tree computes the same traversal.
**Machine-checked (without moves):** `lean/Fugue.lean` models a node by its
path from the root (a list of (side, key) steps) and defines the traversal
order `lt` directly on paths. Proved: `lt` is a strict total order on all
paths (`lt_irrefl`, `lt_trans`, `lt_total`), so "in-order traversal" is a
well-defined function of the set of present atoms.

**3.3a Order is global. [thm]** For two atoms `p, q` the relation `lt p q`
does not mention any world: if both are live in `S` and in `T`, they are in
the same order in `M(S)` and `M(T)`. Growing a world never reorders atoms;
it only inserts between and kills. (`before_stable` in `lean/Fugue.lean`.)
**[rem]** This is the atom-level form of "reorder is free", and the reason a
citation (3.8) can be rendered at any frontier: the span's internal order is
fixed at capture. It holds because a node's path is fixed at capture; moves
change paths and are the one op that breaks it, which is why moves need
their own winner rule (I10).

**3.3b Moves. [def/thm]** A move of atom `a` carries a key and a
destination path. In a world `S`, `a` is placed at its base path if `S`
holds no move of `a`, else at the destination of the move with the highest
key among those in `S`. **I10:** every atom has exactly one placement.
Placement is a function of the *set* of moves in `S`, not of any order in
which they arrived. *Proof.* The highest key exists for a nonempty finite
set and is unique because keys are totally ordered; content addressing makes
equal keys equal moves. ∎ **Machine-checked:** `winner_exists`,
`winner_key_unique`, `winner_perm`, `placed_exists`, `placed_unique` in
`lean/Moves.lean`. With moves, 3.3a weakens to: two atoms keep their order
between worlds that hold the same moves of both.
**[rem]** v0's `replay.rs` walks a linear extension to build the tree; 3.3
says the walk order is irrelevant because the tree is determined by the set.
The weave (`weave.rs`) is an incremental representation of the same tree.

**3.4 Address independence. [thm]** Let `φ` be a bijection on replica ids and
`ψ` a strictly monotone map on hybrid clocks, applied to every `addr`. Then
`M(φψ(S)) = M(S)` for every ideal `S`.
*Proof.* `≤` is unchanged (it reads `deps`, not `addr`); sibling ties are
broken by `(hlc, id)` and `ψ` preserves the order of `hlc`; `id` values change
but their relative order is used only after `hlc` ties, which `ψ` also
preserves. ∎
**→ proptest** relabel replicas and shift clocks; documents identical.
**→ invariant** no semantic function reads `physical` except through the
order that `hlc` already imposes.
**Machine-checked:** `lt_relabel` in `lean/Fugue.lean` — a strictly
monotone, injective relabelling of keys preserves the traversal order.

**3.5 Holes. [def]** A log may lack a body (redaction, lazy fetch) or an event
(a hole). `Doc_τ` includes documents in which an atom is rendered as a
*gap* `⊥(id, length?, body_hash?)` carrying whatever the header states.
`M_τ` on an ideal with missing bodies renders the affected atoms as gaps and
is otherwise unchanged. On a log with holes `M_τ` is defined on the ideals of
the dep-complete part; an event whose dependency is a hole is not in any
ideal until the hole is filled. **Machine-checked:** `hole_excludes`,
`redact_local`, `structure_indep` in `lean/Holes.lean` — the structure of a
world never reads the body store, and removing one body changes the
rendering at that atom alone.

**3.6 Redaction. [thm]** Removing a body from the store changes no `id`, no
`≤`, no ideal, no closure, no frontier. *Proof.* None of these read bodies;
`body_hash` remains in the header. ∎
**→ proptest** redact a random body; every id, closure and frontier
byte-identical; `M` differs only at the redacted atoms, which are gaps.

**3.7 Monotonicity. [rem]** `M` is not monotone on documents (adding a delete
shrinks the text). What is monotone is the *atom set*: `S ⊆ T` implies
`atoms(S) ⊆ atoms(T)` where killed atoms are still atoms. This is the level
at which citations (3.8) and links resolve, and it is why "nothing is ever
deleted" is true at the atom level and false at the document level.

**3.8 Citation. [def]** A *citation* is a link `(span, F)` where `span` is a
set of atom ids and `F` a frontier. For a current ideal `S`:
`then = M(↓F)|span`, `now = M(S)|span`, `stale = diff(then, now)`.
Atoms absent from `S` or killed in `S` render as gaps with provenance. The
reverse index `cited-by(a) = { e : a ∈ span(e) }` is the link relation
restricted to citations.
**[rem]** Because `span` names atoms, not coordinates, the citation survives
every edit; because `F` is an ideal, `then` is reproducible. This is the
whole of "robust anchoring" in the poset.

**3.9 Projection. [def]** A *projection* is a materialiser into another
history model: `M_git(S, ℓ, π)` takes an ideal, a labelling `ℓ` (which events
form which changes) and a chosen path `π` through `I(P)` and produces a git
history. It is *deterministic* if it is a function of `(S, ℓ, π)`, and
*append-only* if extending `π` extends the output as a prefix.
**→ proptest** re-exporting the same `(S, ℓ, π)` yields identical object ids
(the session's mirror test, stated as a law).
**Machine-checked:** `lean/Projection.lean` models a projection as a fold
over the path threading a parent id through a hash rule; `project_append`
and `ids_prefix` show that extending the path extends the output and its ids
as a prefix — append-only is a property of the fold, not of git.

**3.10 Open. [rem]** Three things §3 does not settle:
(i) whether `M_τ` should be required to be *incremental* (a function of
`goto(S, T)` and `M(S)`), which is a performance property but shapes the
weave API;
(ii) the exact codomain for types without atoms (rendered media), where
citations degrade to selectors on a frontier;
(iii) is settled by 3.11 below.

**3.11 Several types in one ideal. [def/thm]** `P` is one poset over all
events regardless of type. For a type `τ` let `E_τ` be its events and
`P_τ = (E_τ, ≤|E_τ)`. For an ideal `S` of `P`, `S ∩ E_τ` is an ideal of
`P_τ`, and `M(S) = ∏_τ M_τ(S ∩ E_τ)`. A cross-type dependency (a task
`Fulfils` a change) is an ordinary edge of `≤`; it makes ideals of `P` that
contain the task also contain the change, and it makes `S ∩ E_task` an ideal
whether or not that edge is considered.
*Proof.* Restriction of a down-set to a subset is a down-set in the induced
order. ∎

---

## What §1–3 claim, in one paragraph

A log is a set of content-addressed events; the dependency references
induce a partial order (1.6) with no causal order beside it (1.7). Worlds are
ideals of that order; they form a distributive lattice (2.2) that is thin
(2.3), so navigation is one operation (2.5) whose inverses are exact when the
delta is the convex difference (2.8) and path-independent otherwise (2.9).
Merge is the join and is literally affine (2.10); independent components are
a product (2.11). Materialisation is a function of the set (3.2),
independent of addresses (3.4), total on holes (3.5), unchanged by redaction
(3.6), and monotone on atoms (3.7), which is what makes citations stable
(3.8). The two questions raised while writing — convexity of changes and
cross-type ideals — turned out to be theorems (2.12, 3.11), not decisions.

---

## 4. Labels

**4.1 Change. [def]** A *change* is a convex set `C ⊆ E` together with
metadata `meta` (message, author key), with
`ChangeId(C) = H(sorted ids(C), H(meta))`. Dependencies are not part of the
id (they are derived, 4.3).

**4.2 Labelling. [def]** A *labelling* `ℓ` is a set of pairwise disjoint
changes. Events outside `⋃ℓ` are *unlabelled*; that is the normal state of
live work. Several labellings may coexist over one log; a labelling is data,
not structure.

**4.3 Derived dependencies. [def]** For changes `C, C'` in `ℓ`:
`C' ≺ C` iff `C' ≠ C` and some `e ∈ C` has `r ∈ deps(e)` with `r ∈ C'`.
`deps_ℓ(C) = { C' : C' ≺ C }`, and `closure_ℓ(C)` is its transitive closure
including `C`.

**4.4 Change-level closure over-approximates. [thm]**
`↓C ⊆ ⋃ closure_ℓ(C)`, and `⋃ closure_ℓ(C)` is an ideal of `P` whenever every
event's dependencies are labelled. Equality holds iff every change in
`closure_ℓ(C)` lies entirely inside `↓C`.
*Proof.* Every dep-edge out of `⋃ closure_ℓ(C)` lands in a change of the
closure by construction, so the union is down-closed; it contains `C`, hence
`↓C`. Equality iff no change of the closure contributes an event outside
`↓C`. ∎
**[rem]** The ratio `|⋃ closure_ℓ(C)| / |↓C|` is FINDINGS' contamination,
stated as a lattice quantity. 2.12 says component-split labelling keeps
changes convex; it does not by itself make the ratio 1, which is why FINDINGS
measures it.

**4.5 Content identity. [thm] (I3)** `ChangeId(C)` depends only on `C`'s
events and metadata. Dropping, relabelling or redacting any other change
leaves it unchanged. *Proof.* By 4.1 and 3.6. ∎ **→ proptest**

**4.6 Ideal history. [def]** An *ideal history* is a pair `(ℓ, π)`: a
labelling and a monotone path `π : ∅ = S₀ ⊆ S₁ ⊆ … ⊆ Sₙ` in `I(P)` whose
steps are unions of whole changes of `ℓ`. It is a view; the projection 3.9
renders it. Squash and unsquash are changes of `ℓ`; reordering is a change
of `π`; neither touches `E`.

**4.7 Relabelling is free. [thm]** For labellings `ℓ, ℓ'` and any ideal `S`,
`M(S)` is independent of which labelling is in force. *Proof.* `M` reads
`S`, not `ℓ` (3.2). ∎ **[rem]** This is "conversion" (3:42 BW): the same
acts under a different description, with identical effect.

---

## 5. Conflicts

**5.1 Overwrite. [def]** Each op kind declares which of its deps it
*overwrites* (a delete overwrites the run it kills; a move overwrites the
previous placement; a register-set overwrites the previous set; an insert
overwrites nothing). Write `ow(e) ⊆ deps(e)`.

**5.2 Conflict predicate. [def]** A type `τ` supplies
`Conf_τ ⊆ { (e, f) : e ∥ f }`. The *default* predicate is
`Conf(e, f) iff e ∥ f ∧ ow(e) ∩ ow(f) ≠ ∅ ∧ ¬Agree(e, f)`, where `Agree` is
type-specific (two identical deletes agree; two identical replacements agree
under `M`, 5.6). Other predicates (adjacency, semantic, test-based) are
additional members of the same family; the lattice does not care which are
enabled.

**5.3 Conflict object. [def]** A conflict is a derived value
`κ = (target, sides)` with `target ∈ ⋂ ow(sides)` and `sides` a maximal set of
pairwise `Conf`-related events on that target; `id(κ) = H(target, sorted sides)`.
Conflicts are not events; they are computed from `S`.

**5.4 Open, resolved. [def]** `κ` is *present* in `S` iff `sides(κ) ⊆ S`. A
*resolution* of `κ` is an event `r` with `sides(κ) ⊆ ↓r` and
`id(κ) ∈ resolves(r)` (a field of its shape). `κ` is *open* in `S` iff present
and no resolution of `κ` lies in `S`.

**5.5 Resolutions are upper bounds; dropping a side drops them. [thm]**
Every resolution `r` of `κ` satisfies `s ≤ r` for all `s ∈ sides(κ)`. Hence
`r ∈ ↑s`, and `drop(S, s)` removes `r`. *Proof.* Immediate from 5.4 and 2.6. ∎
**→ proptest** (ADR-0009's "dropping either side drops the resolution").
**Machine-checked:** `drop_side_drops_resolution`, `resolution_upper` in
`lean/Conflicts.lean`.

**5.6 Agreement is a 2-cell. [def/thm]** Sides `e, f` *agree* in `S` iff
`M(S \ ↑f) = M(S \ ↑e)` on the atoms of `target`. Agreement is an equivalence
on sides; an agreed conflict is closed by definition and `M` renders one
representative (the least by `(hlc, id)`). *Proof of well-definedness.*
Both sets are ideals (2.6) and `M` is a function of them (3.2). ∎
**[rem]** In the homotopical reading, `e` and `f` are two fillers of the same
horn and `Agree` is the existence of a 2-cell between them, computed rather
than stored. **Machine-checked:** `Agree`, `agree_symm`, `agree_refl` in
`lean/Conflicts.lean`, for an arbitrary set-function `M`; and in cubical
Agda (`agda/Horn.agda`, `--safe`): `fillHorn` (in a groupoid every horn
fills — merge by rebasing through the inverse, with the square proved from
the groupoid laws), `rebase` (cherry-pick as transport), `Agree` as
`cong M p ≡ cong M q` with its equivalence laws, and `agree-of-2cell` (a
2-cell between patches is preserved by every `M`: functoriality, ADR-0004).

**5.7 Nesting is uniform. [thm]** Two resolutions `r₁ ∥ r₂` of the same `κ`
both overwrite `id(κ)` (5.4 makes `resolves` an overwrite); hence
`Conf(r₁, r₂)` holds by 5.2 with target `id(κ)`, and the resulting conflict
has the same form as any other. *Proof.* Apply 5.2–5.3 with conflict ids
admitted as targets. ∎
**[rem]** This is `Contested` (ADR-0014) with no special case: a conflict of
resolutions is a conflict one level up, and the level is unbounded.
**Machine-checked:** `nested` in `lean/Conflicts.lean`.

---

## 6. Authority

**6.1 Keys and certificates. [def]** Let `K` be public keys. A *certificate*
is an event `c` with shape `(subject : K, scope, until : hlc)` signed by
`author(c)`, and `deps(c) = { parent }` for a parent certificate, or `∅` for
a root (self-signed identity). The *delegation order* `⊑` is `≤` restricted
to certificates.

**6.2 Monotone scope. [def/invariant]** A certificate is *well-formed* iff
`scope(c) ⊆ scope(parent(c))` and `until(c) ≤ until(parent(c))`. `integrate`
refuses ill-formed certificates. **→ invariant**

**6.3 Validity. [def]** An event `e` with `author(e) = subject(c)` is *valid*
in a log `E` iff there is a chain `root ⊑ … ⊑ parent(c) ⊑ c ⊑ e` in `E` of
well-formed certificates (each parent a dependency of its child, so the root
is lowest and `c ≤ e`), `hlc(e) < until` of each, `op(e) ∈ scope(c)`, and no
revocation event for any link precedes `e` in the chain's replica clock.
(The first draft wrote the chain the other way round; the machine proof
caught it.)

**6.4 Local decidability. [thm]** Validity of `e` depends only on `↓e ∩ Cert`
and revocations linking into it. *Proof.* 6.3 reads only the chain, which
lies in `↓c ⊆ ↓e`. ∎ **[rem]** No peer needs the whole log to judge an event.
**Machine-checked:** `valid_local` (no revocations) and `validR_local`
(with revocations: two logs agreeing on `↓e` and on revocations of chain
links before `e` agree on validity) in `lean/Authority.lean`.
**Bounded check (Alloy 6, `alloy/Capabilities.als`, scope 6):** `RootScope`
(a valid event's op lies in the scope of the root of its chain — scope
monotonicity composes) and `Locality` (6.4 as stated) have no
counterexample; `escalation` (a child wider than its parent) has no
instance once 6.2 is a fact.

**6.5 Attestation. [def]** An attestation is an event `a` with
`links(a) = X` (a set of event ids), shape `(root = MerkleRoot(X), scope)`,
signed by `author(a)`. `covered(e) = { a : e ∈ links(a) }`. Recording a
change attests its events; review is an attestation by another key.

**6.6 Capability. [def]** A capability is a predicate
`cap(label, S, E) ∈ {true, false}` over an ideal `S`, the log, and a label
(`main`, `release`, …), built from validity (6.3) and coverage (6.5). Example:
*human-covered*: every `e ∈ S` authored by an agent session has some
`a ∈ covered(e)` whose author is a human session. Capabilities are checked
locally by every peer; an endorsement (F1) is an attestation of a frontier by
a key that `cap` accepts.

**6.7 Equivocation. [def/thm]** Each replica chains its headers
(`prev_hash` in the header). Two events from one replica with equal `hlc` and
different ids constitute a proof of equivocation exhibitable by any peer that
holds both. *Proof.* Both are signed by the same key over conflicting chain
positions. ∎ **→ invariant**

---

## 7. Task graph

**7.1 Type. [def]** `Task ::= Leaf(crit?) | Box(crit?, interior : DAG of Task)`.
A task is an event of the task type; `crit` is an acceptance criterion (a
test id, or "attestation required"). Two edge kinds: `blockedBy` (a dep
between tasks) and `partOf` (a dep from part to box). Flatten: let `T` be the
set of tasks and `≤_b` the closure of `blockedBy`.

**7.2 Done-sets are ideals. [thm]** A set `D ⊆ T` of done tasks is
down-closed under `≤_b` (nothing is done before its blockers). Hence
`D ∈ I(T, ≤_b)` and everything of §2 applies. ∎

**7.3 Ready. [def]** `ready(D) = { t ∈ min(T \ D) : t is a Leaf ∧ crit(t) defined }`.
`min(T \ D)` is the antichain of covers of `D` (2.7(a)). Boxes are never
ready; a leaf without a criterion is not ready ("idea" is derived).

**7.4 Width and lanes. [thm]** The maximum number of tasks that can be in
progress concurrently equals the width of `(T \ D, ≤_b)`; by Dilworth's
theorem it equals the minimum number of chains covering `T \ D`, i.e. the
number of sequential lanes into which the remaining work can be partitioned
— and the lanes can be taken pairwise disjoint (decomposition form).
∎ **[rem]** Computable from `blockedBy` alone; no durations.
**Machine-checked:** `lean/Dilworth.lean` (Lean 4 + Mathlib, ~410 lines,
Perles' proof): for every finite poset and every maximum antichain `A`
there is a chain cover indexed by `A` with each chain containing its index
(`dilworth`), no cover is smaller than any antichain (`antichain_le_cover`),
a maximum antichain exists (`exists_maxAntichain`), and the chains can be
made disjoint (`disjoint_cover`, `decomposition`).

**7.5 Derived box state. [def]** `done(b)` for a box `b` iff `done(p)` for
every part `p` of `b`, recursively. `crit(b)` is checked when the last part
completes (the 100% check, 7.7).

**7.6 Coherence with contraction. [thm]** Let `q : T → T/b` contract box `b`
to a point. If no `blockedBy` edge crosses the boundary of `b` except edges
whose endpoint is `b` itself (i.e. edges into or out of the box as a whole),
then `q` maps ideals to ideals and `min(q(T \ D)) = q(min(T \ D))` up to the
contracted point. *Proof sketch.* Under the boundary condition, `≤_b` on
`T/b` is the quotient order, and down-closure commutes with the quotient. A
cross edge from inside `b` to outside would create a chain through `b`
invisible after contraction, breaking the second equality. ∎
**[rem]** This is the formal content of "the WBS should cut at the joints":
cross-boundary `blockedBy` edges are exactly what makes the coarse plan lie.
**→ invariant** count or forbid cross-boundary edges.
**Machine-checked:** `lean/Contraction.lean` — with the box done as a whole
or not at all, `image_down` (the image of an ideal is a down-set of the
quotient relation) and `min_preserved` (an element outside the box that is
minimal in the complement stays minimal after contraction). The boundary
condition enters through the second theorem's case analysis; its converse
(minimality reflected back) is not yet done.
**Bounded check (Alloy 6, `alloy/Contraction.als`, scope 6):** with the
box done as a whole or not at all, `imageReady = readyCoarse` holds *outside
the box without any boundary condition* (`CoherentOutside`), and *including
the box* exactly under the boundary condition (`Coherent`); without it Alloy
produces the counterexample (`incoherent`): a part blocked from outside
while another part is free — "some part is ready" and "the box is ready"
come apart. So 7.6's condition is needed only for the box's own readiness;
box atomicity already makes the coarse plan honest everywhere else. This
sharpens the theorem and is not yet reflected in the Lean file.

**7.7 Fulfilment and the 100% check. [def]** `Fulfils(t, C)` is a dep from
task `t` to change `C`. A leaf is done when a fulfilment is accepted (an
attestation, 6.5, in scope of `crit(t)`). The *100% check* for box `b`:
`crit(b)` holds on `M(S ∪ ⋃{ C : Fulfils(p, C), p part of b })`.

**7.8 Planned versus actual. [def]** For `t₁ ≤_b t₂` with `Fulfils(t₁, C₁)`,
`Fulfils(t₂, C₂)`: the plan edge is *real* iff `C₁ ≺* C₂` in `P`, *spurious*
otherwise; an edge `C₁ ≺* C₂` with `t₁ ∥_b t₂` is a *missed* dependency. The
three counts are the plan's reduction ratio. **→ report**

**7.9 Rewrites are changes. [rem]** Split, merge, re-parent and criterion
edits are ops on the task type with `ow` as in 5.1; two concurrent splits of
one task conflict by 5.2; the history of the plan is `I(P_task)`.

---

## 8. Time and redaction

**8.1 Hybrid logical clock. [def]** `hlc = (physical : ms, logical : ℕ)`,
ordered lexicographically. Minting: `physical := max(wall, max seen)`,
`logical := prev.logical + 1` if `physical` unchanged else `0`.

**8.2 Lamport property. [thm]** If every replica mints as in 8.1 and refuses
events with `physical` more than `δ` ahead of its wall clock, then
`r ∈ deps(e) ∪ links(e)` implies `hlc(r) < hlc(e)`, and every replica's
`physical` stays within `δ` of the honest wall clock. *Proof.* Standard
(Kulkarni et al. 2014). ∎ **→ invariant** (1.4's cheap form).

**8.3 Semantic independence. [invariant]** No function of §2–7 reads
`physical` or `logical` except through the order `≤` on `hlc` used for
sibling ties (3.3). **→ proptest** (3.4 already covers it).

**8.4 Redact. [def]** `Redact` is an event with `deps = { target }`, shape
`(reason_hash)`, and no body. Its effect is policy: peers that hold a
`Redact` for `t` delete and stop serving `body(t)`. Its semantic effect is
nil (3.6). A *purge attestation* is an attestation (6.5) linking the `Redact`
with shape `(deleted_at : hlc)`.

**8.5 Blast radius. [def]** For a `Redact` of `t`: `served(t)` is the set of
keys the room's serve log records as recipients of `body(t)`;
`purged(t)` the authors of purge attestations; `radius(t) = served(t) \ purged(t)`.
**[rem]** A claim set, not a proof: possession is unprovable in either
direction. Serve logs are room-scoped by default (F2).

---

## 9. The dynamic half (model-checked, not proved)

**9.1 Sync. [model]** `tla/Sync.tla` models replicas gossiping headers and
bodies over a network that reorders across pairs, duplicates (re-gossip),
and loses a bounded number of messages; redaction as an event that purges
locally and on receipt (8.4); a serve log written at send time (8.5); and a
Byzantine replica minting two events at one sequence number (6.7). TLC
checks: `TypeOK`; `PurgeHonoured` (nobody holds a body whose redaction they
have seen); `ServeLogAccounts` (every body holder is its origin or a
recorded recipient — F2's claim as an invariant); `Monotone` (I13: logs only
grow); `Converge` (I12: after appends stop, logs agree, under fairness of
delivery); `EquivocationCaught` (every honest replica eventually flags the
equivocator). Results and the configuration are in `tla/`.

## What §4–8 add, in one paragraph

Changes are convex sets with content ids (4.1) and derived dependencies
(4.3); change-level closure is a bounded over-approximation of the ideal
(4.4); labellings and paths are views (4.6–4.7). Conflicts are derived from
a per-type predicate on incomparable pairs (5.2), resolutions are upper
bounds (5.5), agreement is computed equality under `M` (5.6), and nesting
needs no special case (5.7). Authority is a chain of certificates inside
`≤` (6.1), locally decidable (6.4), with attestations as links (6.5) and
capabilities as predicates over ideals (6.6). The task graph is a second
poset whose done-sets are ideals (7.2), whose `ready` is the cover antichain
(7.3), whose parallelism is Dilworth's width (7.4), and whose zoom levels are
quotients coherent exactly when no dependency crosses a box boundary (7.6).
Time is a bounded Lamport clock with no semantic role (8.2–8.3); redaction
is a policy event with no semantic effect (8.4).
