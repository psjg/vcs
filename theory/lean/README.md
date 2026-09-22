# lean/ — machine-checked parts of FORMAL.md, in two variants

| file | variant | proves |
|---|---|---|
| `Worlds.lean` | core Lean 4 | 2.2 (sublattice), 2.4, 2.6, 2.7(a)(b), 2.8, 2.9 |
| `Components.lean` | core | 2.12 |
| `Fugue.lean` | core | the Fugue traversal order on paths is a strict total order; 3.3a (order is world-independent); 3.4 (relabelling) — inserts/deletes only, no moves |
| `Conflicts.lean` | core (imports Worlds) | 5.5, 5.6 (well-definedness, symmetry), 5.7 |
| `Moves.lean` | core | 3.3b: highest-key winner for concurrent moves; I10 existence and uniqueness; winner depends only on the set of moves |
| `Holes.lean` | core (imports Worlds) | 3.5: an event with an unknown dependency is in no ideal; 3.6: structure ignores the body store, redaction is local |
| `Projection.lean` | core | 3.9: projections are folds; extending the path extends output and ids as a prefix |
| `Authority.lean` | core | 6.4 (validity is local), with and without revocations |
| `Contraction.lean` | core (imports Worlds) | 7.6: image of an ideal is a down-set of the quotient; outside-minimal elements stay minimal |
| `Dilworth.lean` | Mathlib | Dilworth's theorem (Perles), cover + disjoint decomposition, `exists_maxAntichain`, easy direction — for 7.4 |
| `WorldsMathlib.lean` | Mathlib | 2.2 (full: `DistribLattice (LowerSet α)` by instance; Birkhoff via `OrderIso.lowerSetSupIrred`), 2.4, 2.6, 2.7(a)(b), 2.8 |
| `test_worlds.py` | Hypothesis | 2.2 frontier round-trip, 2.7, 2.8, 2.9 over random posets |

Check: `lean Worlds.lean` etc. for core (`Conflicts.lean` needs `lean Worlds.lean -o Worlds.olean` first and `LEAN_PATH=.`) (Lean 4.34, via elan);
`lake env lean WorldsMathlib.lean` inside a `lake new … math` project for
the Mathlib file.

## Trade-off, measured on this repo's §2

| | core | Mathlib |
|---|---|---|
| install | 3.0 GB toolchain; seconds | + ~6.6 GB of oleans for the full cache (ran the sandbox out of disk); a targeted `cache get` of the order files is ~800 files, then ~30 s to build the helper packages |
| check time (§2 file) | 0.6 s | 1.6 s |
| proof length, cover laws | 33 / 37 lines | 32 / 32 lines |
| proof length, inverse pairs | 16 / 14 | 12 / 12 |
| 2.2 | sublattice only, 12 lines; frontier bijection not done | `inferInstance`; Birkhoff representation one line |
| convexity | own `Convex` | `Set.OrdConnected`, with `hC.out` doing the work |
| ↓ / ↑ | own `Down` / `Up` | `lowerClosure` / `upperClosure`, with `simp` lemmas |
| path independence (2.9) | 81 lines incl. `step`, list induction | not attempted; `List` API would help little |
| Dilworth (7.4) | not attempted | **not in Mathlib**, proved here in ~410 lines (Perles), two iterations; needs `Finset.card`, `exists_minimal`, `surj_on_of_inj_on_of_card_le`, `strongInduction` — the Finset library is what made it feasible |
| who can write it | any agent that can do `intro`/`cases`/`exact` | needs the Mathlib names (`SetLike.mem_coe`, `mem_lowerClosure`, `Set.disjoint_left`, …); first attempt failed on coercions after `ext` |
| trust base | Lean kernel + 300 own lines | Lean kernel + Mathlib |

Reading of the numbers: the core proofs are not longer where the statement
is elementary (cover laws, inverse pairs); they are longer where Mathlib
already has a theory (Birkhoff, lattice instances, `OrdConnected`). Mathlib
buys the *background* (2.2 in full) and standard names; it does not buy
shorter proofs of the project-specific laws, and it does not contain the one
classical theorem §7 needs (Dilworth).

Recommendation: keep the core variant as the canonical one for the
project-specific laws (§2.4–2.12, §6), because it is cheap, portable and
agent-writable; keep the Mathlib file as the bridge that shows the
project's `Ideal` *is* Mathlib's `LowerSet` and that 2.2 is Birkhoff, so
that anyone who wants the standard theory can import it. Dilworth is
the one place where Mathlib clearly won: the proof needed cardinality
bookkeeping over `Finset` that core Lean does not have.

## Dilworth: proof structure
Perles (1963), strong induction on `P` (`Finset.strongInduction` on ⊂):
*case 1* — some maximum antichain `B` is neither `minSet P` nor `maxSet P`;
then `P⁺ = ↑B ∩ P` and `P⁻ = ↓B ∩ P` are proper subsets covering `P`, each
with `B` as a maximum antichain; the inductive covers indexed by `B` are
glued chain-by-chain at their index (every chain of `P⁺` lies above its
index, every chain of `P⁻` below it); then re-index to the given `A`.
*case 2* — every maximum antichain is `minSet` or `maxSet`; take `x ≤ y`
minimal/maximal with one of them in `A`, remove `{x, y}`, show `A` minus
that element is a maximum antichain of the rest (a size-`|A|` antichain of
the rest would be a maximum antichain of `P` containing neither `x` nor
`y`, contradicting case 2), induct, add the chain `{x, y}`.
The decomposition form sends each element to one chain containing it and
filters. Reference formalisations consulted: Singh (Coq, 2016) and
Maadoori et al. (Isabelle/AFP, 2025), both in `dilworth-refs/`.

## What the machine changed in FORMAL.md
- 2.4 converse: neither convexity nor disjointness is needed.
- 2.9 step lemma: `T` need not be an ideal.
- 6.3: the chain direction was written backwards (root is lowest).
