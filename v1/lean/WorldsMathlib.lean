/-
  v1/FORMAL.md §2 — the Mathlib variant.
  Worlds are `LowerSet α`; ↓ and ↑ are `lowerClosure`/`upperClosure`;
  convexity is `Set.OrdConnected`.
-/
import Mathlib.Order.UpperLower.Closure
import Mathlib.Order.Birkhoff

open Set

namespace WorldsM
variable {α : Type*} [PartialOrder α]

/-! ## 2.2: for free from Mathlib -/
example : DistribLattice (LowerSet α) := inferInstance
example : CompleteLattice (LowerSet α) := inferInstance

/-! ## 2.6 -/
def adopt (S : LowerSet α) (C : Set α) : LowerSet α := S ⊔ lowerClosure C
/-- `S \ ↑C`, as a lower set -/
def drop (S : LowerSet α) (C : Set α) : LowerSet α :=
  ⟨(S : Set α) \ upperClosure C, by
    intro a b hba ⟨hs, hnot⟩
    refine ⟨S.lower hba hs, ?_⟩
    intro ⟨c, hc, hcb⟩
    exact hnot ⟨c, hc, hcb.trans hba⟩⟩

@[simp] lemma mem_adopt {S : LowerSet α} {C : Set α} {x : α} :
    x ∈ adopt S C ↔ x ∈ S ∨ ∃ c ∈ C, x ≤ c := by
  simp [adopt, LowerSet.mem_sup_iff, mem_lowerClosure]

@[simp] lemma mem_drop {S : LowerSet α} {C : Set α} {x : α} :
    x ∈ drop S C ↔ x ∈ S ∧ ¬ ∃ c ∈ C, c ≤ x := by
  show x ∈ (S : Set α) \ upperClosure C ↔ _
  simp [mem_upperClosure]

/-! ## 2.4 -/
theorem diff_ordConnected (S T : LowerSet α) : ((T : Set α) \ S).OrdConnected := by
  constructor
  intro a ⟨_, hnsa⟩ c ⟨htc, _⟩ b ⟨hab, hbc⟩
  exact ⟨T.lower hbc htc, fun hsb => hnsa (S.lower hab hsb)⟩

/-! ## 2.7 -/
theorem cover_a (S : LowerSet α) {C : Set α} (hC : C.OrdConnected) :
    drop (adopt S C) C = S ↔
      Disjoint C (S : Set α) ∧ (∀ x, (∃ c ∈ C, x ≤ c) → x ∉ C → x ∈ S) := by
  constructor
  · intro heq
    refine ⟨?_, ?_⟩
    · rw [Set.disjoint_left]
      intro x hcx hsx
      have : x ∈ drop (adopt S C) C := by rw [heq]; exact hsx
      exact (mem_drop.1 this).2 ⟨x, hcx, le_rfl⟩
    · intro x ⟨c, hc, hxc⟩ hncx
      have : x ∈ drop (adopt S C) C := by
        rw [mem_drop]
        refine ⟨mem_adopt.2 (Or.inr ⟨c, hc, hxc⟩), ?_⟩
        intro ⟨c', hc', hc'x⟩
        exact hncx (hC.out hc' hc ⟨hc'x, hxc⟩)
      rw [heq] at this; exact this
  · intro ⟨hdisj, hbelow⟩
    ext x
    simp only [SetLike.mem_coe]
    rw [mem_drop, mem_adopt]
    constructor
    · rintro ⟨h | ⟨c, hc, hxc⟩, hnup⟩
      · exact h
      · by_cases hcx : x ∈ C
        · exact absurd ⟨x, hcx, le_rfl⟩ hnup
        · exact hbelow x ⟨c, hc, hxc⟩ hcx
    · intro hsx
      refine ⟨Or.inl hsx, ?_⟩
      rintro ⟨c, hc, hcx⟩
      exact Set.disjoint_left.1 hdisj hc (S.lower hcx hsx)

theorem cover_b (S : LowerSet α) {C : Set α} (hC : C.OrdConnected) :
    adopt (drop S C) C = S ↔
      C ⊆ (S : Set α) ∧ IsLowerSet ((S : Set α) \ C) := by
  constructor
  · intro heq
    refine ⟨?_, ?_⟩
    · intro x hcx
      have : x ∈ adopt (drop S C) C := mem_adopt.2 (Or.inr ⟨x, hcx, le_rfl⟩)
      rw [heq] at this; exact this
    · intro e r hre ⟨hse, hnce⟩
      refine ⟨S.lower hre hse, ?_⟩
      intro hcr
      have he : e ∈ adopt (drop S C) C := by rw [heq]; exact hse
      rcases mem_adopt.1 he with hd | ⟨c, hc, hec⟩
      · exact (mem_drop.1 hd).2 ⟨r, hcr, hre⟩
      · exact hnce (hC.out hcr hc ⟨hre, hec⟩)
  · intro ⟨hsub, hideal⟩
    ext x
    simp only [SetLike.mem_coe]
    rw [mem_adopt, mem_drop]
    constructor
    · rintro (⟨hsx, _⟩ | ⟨c, hc, hxc⟩)
      · exact hsx
      · exact S.lower hxc (hsub hc)
    · intro hsx
      by_cases hup : ∃ c ∈ C, c ≤ x
      · obtain ⟨c, hc, hcx⟩ := hup
        by_cases hcx' : x ∈ C
        · exact Or.inr ⟨x, hcx', le_rfl⟩
        · exact absurd hc (hideal hcx ⟨hsx, hcx'⟩).2
      · exact Or.inl ⟨hsx, hup⟩

/-! ## 2.8 -/
theorem adopt_diff {S T : LowerSet α} (hsub : S ≤ T) :
    adopt S ((T : Set α) \ S) = T := by
  ext x; simp only [SetLike.mem_coe]; rw [mem_adopt]
  constructor
  · rintro (hs | ⟨d, ⟨htd, _⟩, hxd⟩)
    · exact hsub hs
    · exact T.lower hxd htd
  · intro htx
    by_cases hsx : x ∈ S
    · exact Or.inl hsx
    · exact Or.inr ⟨x, ⟨htx, hsx⟩, le_rfl⟩

theorem drop_diff {S T : LowerSet α} (hsub : S ≤ T) :
    drop T ((T : Set α) \ S) = S := by
  ext x; simp only [SetLike.mem_coe]; rw [mem_drop]
  constructor
  · rintro ⟨htx, hnup⟩
    by_contra hsx
    exact hnup ⟨x, ⟨htx, hsx⟩, le_rfl⟩
  · intro hsx
    refine ⟨hsub hsx, ?_⟩
    rintro ⟨d, ⟨_, hnsd⟩, hdx⟩
    exact hnsd (S.lower hdx hsx)

/-! ## 2.2, the other half: Birkhoff from Mathlib.
    A finite distributive lattice is the lattice of lower sets of its
    sup-irreducibles — the abstract form of "worlds ↔ ideals of a poset". -/
noncomputable example {L : Type*} [DistribLattice L] [Fintype L] [DecidablePred (SupIrred (α := L))]
    [OrderBot L] : L ≃o LowerSet {a : L // SupIrred a} :=
  OrderIso.lowerSetSupIrred

end WorldsM
