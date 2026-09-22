# v1 — inventory of the 2026-09-22 design conversation

Turn 0 of three: an inventory, not a vision. Ordered by theme, not by the
order things were said. One line per conclusion. `VISION.md` picks the
load-bearing ones; a later pass formalises them.

## A. Mathematical ground

- **A1 Algebra.** A carrier with total operations of fixed arity; axioms are
  equations (varieties, Birkhoff). Relations are not part of an algebra. An
  inverse *element* (of an operation) is not an inverse *map* (of a function);
  an involution is the special case f⁻¹ = f. A neutral element is required to
  even state inverses.
- **A2 States and deltas.** Point and arrow, as an affine space: point − point
  = delta, point + delta = point, point + point undefined unless the weights sum
  to 1. Three-way merge a + b − base and rebase new + (p − old) are affine
  combinations; that is why a merge needs a common ancestor. Choosing an origin
  (∅) turns it into a vector space; a shallow clone moves the origin and loses
  every combination that needs a point before the cut; invertible deltas let
  you walk back, in reverse order, once fetched.
- **A3 Snapshot / checkpoint / delta.** Snapshot = state as a value.
  Checkpoint = a materialised point in a system that logically consists of
  deltas (a cache of the fold); later: with a hash, a verifiable claim.
  Stored-vs-derived is a representation choice; point-vs-arrow is the
  structure.
- **A4 Structure of deltas.** States as objects, transactions as arrows: a
  category. Invertible only from a fixed source: a groupoid. Commutation is the
  exception and therefore the core: a conflict is non-commutation, a merge is a
  pushout (Mimram–Di Giusto); Pijul extends the category so pushouts exist,
  CRDTs make everything commute (join-semilattice), STM aborts.
- **A5 Three kinds of delta.** Constant map (blind write; idempotent, not
  invertible), partial bijection {0 ↦ 5} (inverse semigroup; Wagner–Preston;
  ESN: inverse semigroups ≅ inductive groupoids), translation x += 5 (abelian
  group action). Ladder: monoid with idempotents → inverse semigroup → group.
  Idempotence matters for effects (safe retry).

## B. v0, and what the mathematics adds

- **B1 What v0 is.** Event log (Fugue, character runs, Lamport), changes as
  labels over events, deps derived from `semantic_refs`, materialisation `M(S)`
  over dependency-closed subsets; merge/adopt/drop as set algebra. Conflicts
  derived, resolutions are patches. FINDINGS: event-level closure is flat;
  contamination came from commit batching; `record` splits per component
  (ADR-0007). Shelved: checkpoints as objects, structure/content split.
- **B2 The groupoid question.** Changes form a poset P; a `ChangeSet` is an
  ideal. adopt = S ∪ ↓c, drop = S \ ↑c. The inverse laws hold only on covers:
  drop∘adopt = id ⟺ c ∉ S ∧ deps(c) ⊆ S; adopt∘drop = id ⟺ c ∈ max(S). I5
  tests the lucky case.
- **B3 The real object.** The ideals of P form a distributive lattice
  (Birkhoff): thin (every diagram commutes; path-independence is free), join =
  merge, meet = common base (ADR-0014 already uses it), ideal ↔ antichain (the
  frontier is the normal form; the state vector is the same compression at
  event level). Proposals: one primitive `goto(S, T)`; `drop` returns the
  removed convex piece so the inverse always holds; typed covers; property
  tests over random posets; components = product decomposition; conflict = an
  incomparable pair on one atom, resolution = an upper bound.
- **B4 v1 core: one relation.** v0 has two orders (causal, semantic); the
  second replaces the first for `M` (I9) but not for conflicts and LWW.
  Principle: an op references everything it observes *and overwrites*
  (multi-value register). Then concurrent = incomparable, the concurrency
  table collapses to one rule, LWW disappears, `parents` and I8 disappear, the
  clock becomes a tie-break. Keep: changes as labels, component split, own
  materialiser. Drop: `deps` from the ChangeId hash.
- **B5 The four layers.** Ropes (cost) → CRDT (convergence, join-semilattice)
  → patches (partial order, ideals) → DAG (= the ideal lattice; in v0 still
  carrying a second relation). Out of scope: identity of definitions, effects.

## C. Beyond the four layers

- **C1 Definitions.** Ops on syntax/definitions (tree-sitter, LSP, Unison);
  kern as the layer that gives atoms meaning. A rename is one op.
- **C2 Typed worlds.** A change is a Hoare triple over `M`; not every join is
  allowed; a semantic conflict is a predicate computed by tests/typechecker;
  CI is the typing of the lattice; bisect is the cover where typing breaks.
- **C3 Locality.** Components are the simplest case of sheaves: local worlds,
  merge = gluing, conflict = failure of the gluing condition, contamination as
  a topological invariant.
- **C4 Runtime.** Code and program state as ideals in one poset; effects as a
  sub-poset without groupoid arrows (the Temporal separation).
- **C5 Techniques.** Confluently persistent data structures (Driscoll et al.,
  Fiat–Kaplan: the theory of the weave as a version DAG); differential
  dataflow / DBSP (incremental computation over partially ordered times;
  Z-sets as an abelian group for the derived layer); prolly trees and Merkle
  search trees (history-independent hashes, reconciliation without state
  vectors); delta lenses (text ↔ structure). OT and snapshot models add
  nothing.
- **C6 Homotopy.** Worlds/patches/patch laws as 0/1/2-cells;
  Angiuli–Morehouse–Licata–Harper: patch theory as a HIT, a model is a functor
  (= ADR-0004; v0 computes the 2-cells instead of storing them, hence thin).
  Merge = Kan filler of a horn; conflict = a horn that does not fill;
  Darcs/Pijul/v0 as three answers. Resolutions as 2-cells (`Agreed` = a
  2-cell, `Contested` = none); nested conflicts are uniform. Rebase/cherry-pick
  = transport. Univalence: representation changes are paths with a proof
  obligation.

## D. The practical list and the three decisions

Recipes/forks (ideal + reproducible build); p2p with the workflow in the repo
(COBs as op families); rooms, chunk, unchunk, burn; ideal history as a
labelling; monorepos / partial / gossip (product decomposition, seeds);
signing; binary data (register ops + content-defined chunking, locks as a
COB); workflows (stable change identity is structural); malicious peers (not
solvable; provenance is); package manager; IPFS (complement); Nelson
(transclusion via run identity).

- **Three decisions:** content-addressed events with a structure/content
  split; an extensible op signature; transport.
- **Two tensions:** burn vs truth vs hashes (resolved by the split plus a
  signed hole, Fossil's shun); text vs blob (two materialisers in one log).

## E. Hashes, time, crypto

- **E1 Header/body.** EventId = blake3(header); body separate; rule: anything
  a human typed is body. Redaction leaves every id intact (new invariant).
  `Redact` is an op with a reason; `M` renders holes. Address `(replica, seq)`
  in the header, identity = hash. Sign per attestation (Merkle root), not per
  keystroke. ChangeId without deps. Two stores (header stream, blob store).
  Two pinning tests.
- **E2 Time.** Hybrid logical clock: Lamport property, approximate wall clock,
  bounded against capture; no semantic function depends on `physical`
  (invariant + property test). Verified time remains an external attestation.
- **E3 Cypherpunk without the cringe.** Identity = key, petnames, no registry.
  Delegation chain identity → device → session; session certificates carry
  harness/model/effort/config/task (as body hashes), scope monotone, depth
  bounded; the harness signs, not the model. Human review = attestation;
  capability rule "agent events must be covered by a human". Hash chains per
  replica → provable equivocation (SSB). Rooms: wrapped group key or MLS;
  bodies always encrypted, headers encrypted for non-members; dumb relays seed
  ciphertext. Capabilities, not ACLs (SPKI/UCAN). Noise transport. Primitives:
  blake3, ed25519, x25519, XChaCha20-Poly1305, HPKE, Noise, MLS. Threat model
  explicit in DESIGN, including what is not promised.

## F. Governance and law

- **F1 `main` does not exist.** Only signed endorsements of frontiers; recipe
  → source → binary as a reproducible chain; distributions are someone's main
  plus recipes.
- **F2 Blast radius.** Serve logs + purge attestations; non-possession is
  unprovable; claims are what law works with. Layered: room members (free),
  trusted peers (logged with retention), strangers (ciphertext or nothing);
  escalation on suspicion is itself a signed, scoped, expiring op. Recursion:
  closes over transfers (the log logs its own reads, CT model), escapes over
  local reads.
- **F3 Obligations law as an algebra.** Legal fact / legal effect = log /
  ideal. Void (never in the ideal), annulment (drop, ex tunc, with standing and
  grounds), termination (revert, ex nunc, restoration obligations = saga
  compensation), partial nullity (component split), conversion (relabel),
  ratification (attestation), prescription (capability expiry). Hohfeld:
  capabilities = powers, immunity = non-domination; Nomic/Suber: powers over
  powers. Catala: statutory law formalisable, interpretive law not; the cut
  coincides with computed/attested.
- **F4 Comparative and other families.** Functional method (Zweigert–Kötz),
  DCFR/PECL/UNIDROIT as shared vocabulary; retroactivity, standing,
  prescription, partiality as parameters. Isnad / jarh wa ta'dil (provenance
  with a grade per link), madhhabs/fatwa (followed authority), Talmud (dissent
  preserved), canon law (dispensation, sanatio in radice = ratification ex
  tunc), lex mercatoria (reputation instead of coercion,
  Milgrom–North–Weingast), Ostrom (eight principles), restorative custom.
- **F5 Forks and Hirschman.** A fork is an ideal over the same poset; you
  fork a decision; cost = |↑X|, computable in advance; soft forks become free;
  partial exit disciplines governance. Out of scope: the institution (review,
  infra, trademark). Individual soft forks: the maintenance half is solved,
  the compile half needs fine build caches (ladder: link → per unit → per
  action → per definition; header contamination as the ceiling); reproducible
  builds make caches shareable over the same gossip; voice and exit become the
  same act; Illich.
- **F6 Labour made legible.** Task/Fulfils/Accept; the test as acceptance
  criterion (C2); a bounty as a signed promise with human acceptance, money
  outside (Ethereum at most as dumb escrow with the `Accept` as oracle; the
  DAO fork proves judgement lives outside code); invisible work becomes
  countable; a DAO in the literal sense without a blockchain. Accountability:
  scope in attestations, just culture, pseudonymous by default with identity
  disclosable by procedure, post-mortem as a change, graduated responses
  (Ostrom); the CRA makes liability real. Key sentence: an attestation proves
  a judgement within a scope, not who, not that it was right.

## G. The session export and the WBS

- **G1 Six lessons.** The leaked commit (redaction on forges is impossible
  without Support; header/body would have solved it); "every change has
  exactly one home" with deterministic, append-only projections (a mirror is a
  second materialiser); a task points at an identity of ours, not a forge's;
  plan-as-patch and the reducer tension resolved (reducer = `M`, rewrites =
  data); per-hunk acceptance is native (a sub-ideal, the events stay the
  same); Radicle's trade (unforgeable vs editable) lifted via `M`. Four days
  of bridge-building = the bill for three identities.
- **G2 WBS.** Done-sets are ideals; `ready` = covers; width = antichain;
  Dilworth gives the number of lanes. A WBS is deliverable-oriented: scope =
  acceptance criterion; the 100% rule becomes a check on close. `ready` = leaf
  ∧ unblocked ∧ has criterion; `idea` derived; shaping = writing the
  criterion. Planned vs actual deps measurable retroactively (the reduction
  ratio one level up); `partOf` vs components measures whether the plan cuts
  at the joints. Rewrites are changes on a document type.
- **G3 DAGtaak / taakDAG / taakDAGenDAG.** Fixed-point type (Leaf | Box with
  interior); HTN and operad (substitution; 100% = boundary condition); zoom
  levels as a tower of quotients with a coherence requirement (derived state
  commutes with contraction); the history is the ideal lattice over rewrite
  events, the same construction as the world DAG. Cross-edges between boxes:
  allow and count, or forbid.
