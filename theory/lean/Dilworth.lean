/-
  Dilworth's theorem (chain-cover form, Perles' proof), for FORMAL.md 7.4.
  Statement: for every finite poset P and every maximum antichain A ⊆ P there is
  a family of chains indexed by A, each containing its index, covering P.
  Hence min #chains = width.  Lean 4 + Mathlib.
-/
import Mathlib.Data.Finset.Card
import Mathlib.Data.Finset.Max
import Mathlib.Order.Preorder.Finite
import Mathlib.Data.Fintype.Card
import Mathlib.Data.Finset.Powerset
import Mathlib.Data.Finset.Max

open Classical

namespace Dilworth
variable {α : Type*} [PartialOrder α]

def IsChainF (C : Finset α) : Prop := ∀ a ∈ C, ∀ b ∈ C, a ≤ b ∨ b ≤ a
def IsAntichainF (A : Finset α) : Prop := ∀ a ∈ A, ∀ b ∈ A, a ≤ b → a = b
def MaxAntichain (P A : Finset α) : Prop :=
  A ⊆ P ∧ IsAntichainF A ∧ ∀ B ⊆ P, IsAntichainF B → B.card ≤ A.card
/-- a chain cover of `P` indexed by `A`, every chain containing its index -/
def CoverBy (P A : Finset α) (c : α → Finset α) : Prop :=
  (∀ a ∈ A, IsChainF (c a) ∧ c a ⊆ P ∧ a ∈ c a) ∧ (∀ x ∈ P, ∃ a ∈ A, x ∈ c a)
noncomputable def minSet (P : Finset α) : Finset α := P.filter (fun x => ∀ z ∈ P, z ≤ x → z = x)
noncomputable def maxSet (P : Finset α) : Finset α := P.filter (fun x => ∀ z ∈ P, x ≤ z → z = x)

/-! ### small lemmas -/

theorem chain_meets_antichain {C A : Finset α} (hC : IsChainF C) (hA : IsAntichainF A)
    {a b : α} (ha : a ∈ A) (hb : b ∈ A) (haC : a ∈ C) (hbC : b ∈ C) : a = b := by
  rcases hC a haC b hbC with h | h
  · exact hA a ha b hb h
  · exact (hA b hb a ha h).symm

theorem exists_min_below (P : Finset α) {b : α} (hb : b ∈ P) :
    ∃ z ∈ minSet P, z ≤ b := by
  have hne : (P.filter (fun x => x ≤ b)).Nonempty := ⟨b, by simp [hb]⟩
  obtain ⟨z, hz⟩ := Finset.exists_minimal hne
  have hzP : z ∈ P := (Finset.mem_filter.1 hz.1).1
  have hzb : z ≤ b := (Finset.mem_filter.1 hz.1).2
  refine ⟨z, ?_, hzb⟩
  rw [minSet, Finset.mem_filter]
  refine ⟨hzP, ?_⟩
  intro w hw hwz
  have hw' : w ∈ P.filter (fun x => x ≤ b) := by
    rw [Finset.mem_filter]; exact ⟨hw, le_trans hwz hzb⟩
  exact le_antisymm hwz (hz.2 hw' hwz)

theorem exists_max_above (P : Finset α) {b : α} (hb : b ∈ P) :
    ∃ z ∈ maxSet P, b ≤ z := by
  have hne : (P.filter (fun x => b ≤ x)).Nonempty := ⟨b, by simp [hb]⟩
  obtain ⟨z, hz⟩ := Finset.exists_maximal hne
  have hzP : z ∈ P := (Finset.mem_filter.1 hz.1).1
  have hbz : b ≤ z := (Finset.mem_filter.1 hz.1).2
  refine ⟨z, ?_, hbz⟩
  rw [maxSet, Finset.mem_filter]
  refine ⟨hzP, ?_⟩
  intro w hw hzw
  have hw' : w ∈ P.filter (fun x => b ≤ x) := by
    rw [Finset.mem_filter]; exact ⟨hw, le_trans hbz hzw⟩
  exact le_antisymm (hz.2 hw' hzw) hzw

theorem minSet_subset_antichain_eq {P A : Finset α} (hAP : A ⊆ P) (hA : IsAntichainF A)
    (h : minSet P ⊆ A) : A = minSet P := by
  apply Finset.Subset.antisymm _ h
  intro a ha
  obtain ⟨z, hz, hza⟩ := exists_min_below P (hAP ha)
  have : z = a := hA z (h hz) a ha hza
  rw [← this]; exact hz

theorem maxSet_subset_antichain_eq {P A : Finset α} (hAP : A ⊆ P) (hA : IsAntichainF A)
    (h : maxSet P ⊆ A) : A = maxSet P := by
  apply Finset.Subset.antisymm _ h
  intro a ha
  obtain ⟨z, hz, haz⟩ := exists_max_above P (hAP ha)
  have : a = z := hA a ha z (h hz) haz
  rw [this]; exact hz

/-- re-index a cover from one maximum antichain to another of the same size -/
theorem reindex {P A B : Finset α} {c : α → Finset α} (hcov : CoverBy P B c)
    (hAP : A ⊆ P) (hA : IsAntichainF A) (hcard : B.card ≤ A.card) :
    ∃ c', CoverBy P A c' := by
  -- for each a ∈ A pick the chain that contains it
  have hpick : ∀ a ∈ A, ∃ b ∈ B, a ∈ c b := fun a ha => hcov.2 a (hAP ha)
  let g : (a : α) → a ∈ A → α := fun a ha => Classical.choose (hpick a ha)
  have hg : ∀ a (ha : a ∈ A), g a ha ∈ B ∧ a ∈ c (g a ha) :=
    fun a ha => Classical.choose_spec (hpick a ha)
  have ginj : ∀ a₁ a₂ (h₁ : a₁ ∈ A) (h₂ : a₂ ∈ A), g a₁ h₁ = g a₂ h₂ → a₁ = a₂ := by
    intro a₁ a₂ h₁ h₂ heq
    have hb := (hg a₁ h₁).1
    have hC := (hcov.1 _ hb).1
    have h2 : a₂ ∈ c (g a₁ h₁) := by rw [heq]; exact (hg a₂ h₂).2
    exact chain_meets_antichain hC hA h₁ h₂ (hg a₁ h₁).2 h2
  have gsurj := Finset.surj_on_of_inj_on_of_card_le g (fun a ha => (hg a ha).1) ginj hcard
  refine ⟨fun a => if h : a ∈ A then c (g a h) else ∅, ?_, ?_⟩
  · intro a ha
    simp only [dif_pos ha]
    exact ⟨(hcov.1 _ (hg a ha).1).1, (hcov.1 _ (hg a ha).1).2.1, (hg a ha).2⟩
  · intro x hx
    obtain ⟨b, hb, hxb⟩ := hcov.2 x hx
    obtain ⟨a, ha, hab⟩ := gsurj b hb
    refine ⟨a, ha, ?_⟩
    simp only [dif_pos ha]
    rw [← hab]; exact hxb

/-! ### the theorem -/

theorem dilworth (P : Finset α) : ∀ A, MaxAntichain P A → ∃ c, CoverBy P A c := by
  induction P using Finset.strongInduction with
  | H P ih =>
  intro A hA
  obtain ⟨hAP, hAac, hAmax⟩ := hA
  by_cases hPe : P = ∅
  · refine ⟨fun _ => ∅, ?_, ?_⟩
    · intro a ha; exact absurd (hAP ha) (by rw [hPe]; simp)
    · intro x hx; exact absurd hx (by rw [hPe]; simp)
  -- P nonempty
  by_cases hB : ∃ B, MaxAntichain P B ∧ B ≠ minSet P ∧ B ≠ maxSet P
  · -- Case 1
    obtain ⟨B, ⟨hBP, hBac, hBmax⟩, hBmin, hBmaxs⟩ := hB
    let Pp := P.filter (fun x => ∃ b ∈ B, b ≤ x)
    let Pm := P.filter (fun x => ∃ b ∈ B, x ≤ b)
    have hPpP : Pp ⊆ P := Finset.filter_subset _ _
    have hPmP : Pm ⊆ P := Finset.filter_subset _ _
    have hBPp : B ⊆ Pp := fun b hb => Finset.mem_filter.2 ⟨hBP hb, b, hb, le_rfl⟩
    have hBPm : B ⊆ Pm := fun b hb => Finset.mem_filter.2 ⟨hBP hb, b, hb, le_rfl⟩
    -- a minimal element outside B is not in Pp
    have hPp_ssub : Pp ⊂ P := by
      rw [Finset.ssubset_iff_subset_ne]
      refine ⟨hPpP, ?_⟩
      intro heq
      apply hBmin
      apply minSet_subset_antichain_eq hBP hBac
      intro z hz
      have hzP : z ∈ P := (Finset.mem_filter.1 hz).1
      have hzPp : z ∈ Pp := by rw [heq]; exact hzP
      obtain ⟨b, hb, hbz⟩ := (Finset.mem_filter.1 hzPp).2
      have : b = z := (Finset.mem_filter.1 hz).2 b (hBP hb) hbz
      rw [← this]; exact hb
    have hPm_ssub : Pm ⊂ P := by
      rw [Finset.ssubset_iff_subset_ne]
      refine ⟨hPmP, ?_⟩
      intro heq
      apply hBmaxs
      apply maxSet_subset_antichain_eq hBP hBac
      intro z hz
      have hzP : z ∈ P := (Finset.mem_filter.1 hz).1
      have hzPm : z ∈ Pm := by rw [heq]; exact hzP
      obtain ⟨b, hb, hzb⟩ := (Finset.mem_filter.1 hzPm).2
      have : b = z := (Finset.mem_filter.1 hz).2 b (hBP hb) hzb
      rw [← this]; exact hb
    have hBmaxPp : MaxAntichain Pp B :=
      ⟨hBPp, hBac, fun C hC hCac => hBmax C (hC.trans hPpP) hCac⟩
    have hBmaxPm : MaxAntichain Pm B :=
      ⟨hBPm, hBac, fun C hC hCac => hBmax C (hC.trans hPmP) hCac⟩
    obtain ⟨cp, hcp⟩ := ih Pp hPp_ssub B hBmaxPp
    obtain ⟨cm, hcm⟩ := ih Pm hPm_ssub B hBmaxPm
    -- chains of the upper cover sit above their index, lower ones below
    have above : ∀ b ∈ B, ∀ x ∈ cp b, b ≤ x := by
      intro b hb x hx
      have hxPp : x ∈ Pp := (hcp.1 b hb).2.1 hx
      obtain ⟨b', hb', hb'x⟩ := (Finset.mem_filter.1 hxPp).2
      rcases (hcp.1 b hb).1 b (hcp.1 b hb).2.2 x hx with h | h
      · exact h
      · have : b' = b := hBac b' hb' b hb (le_trans hb'x h)
        rw [← this]; exact hb'x
    have below : ∀ b ∈ B, ∀ x ∈ cm b, x ≤ b := by
      intro b hb x hx
      have hxPm : x ∈ Pm := (hcm.1 b hb).2.1 hx
      obtain ⟨b', hb', hxb'⟩ := (Finset.mem_filter.1 hxPm).2
      rcases (hcm.1 b hb).1 b (hcm.1 b hb).2.2 x hx with h | h
      · have : b = b' := hBac b hb b' hb' (le_trans h hxb')
        rw [this]; exact hxb'
      · exact h
    have hcovB : CoverBy P B (fun b => cp b ∪ cm b) := by
      refine ⟨?_, ?_⟩
      · intro b hb
        refine ⟨?_, ?_, ?_⟩
        · intro u hu v hv
          rcases Finset.mem_union.1 hu with hu | hu <;> rcases Finset.mem_union.1 hv with hv | hv
          · exact (hcp.1 b hb).1 u hu v hv
          · exact Or.inr (le_trans (below b hb v hv) (above b hb u hu))
          · exact Or.inl (le_trans (below b hb u hu) (above b hb v hv))
          · exact (hcm.1 b hb).1 u hu v hv
        · intro u hu
          rcases Finset.mem_union.1 hu with hu | hu
          · exact hPpP ((hcp.1 b hb).2.1 hu)
          · exact hPmP ((hcm.1 b hb).2.1 hu)
        · exact Finset.mem_union.2 (Or.inl (hcp.1 b hb).2.2)
      · intro x hx
        -- x is comparable with some b ∈ B, else B ∪ {x} is a bigger antichain
        have hcomp : ∃ b ∈ B, b ≤ x ∨ x ≤ b := by
          by_contra hno
          push_neg at hno
          have hxB : x ∉ B := fun h => (hno x h).1 le_rfl
          have hac : IsAntichainF (insert x B) := by
            intro u hu v hv huv
            rcases Finset.mem_insert.1 hu with hu | hu <;> rcases Finset.mem_insert.1 hv with hv | hv
            · rw [hu, hv]
            · exact absurd huv (by rw [hu]; exact (hno v hv).2)
            · exact absurd huv (by rw [hv]; exact (hno u hu).1)
            · exact hBac u hu v hv huv
          have hsub : insert x B ⊆ P := Finset.insert_subset hx hBP
          have := hBmax _ hsub hac
          rw [Finset.card_insert_of_notMem hxB] at this
          omega
        obtain ⟨b, hb, h | h⟩ := hcomp
        · obtain ⟨b', hb', hxb'⟩ := hcp.2 x (Finset.mem_filter.2 ⟨hx, b, hb, h⟩)
          exact ⟨b', hb', Finset.mem_union.2 (Or.inl hxb')⟩
        · obtain ⟨b', hb', hxb'⟩ := hcm.2 x (Finset.mem_filter.2 ⟨hx, b, hb, h⟩)
          exact ⟨b', hb', Finset.mem_union.2 (Or.inr hxb')⟩
    exact reindex hcovB hAP hAac (hAmax B hBP hBac)
  · -- Case 2: every maximum antichain is minSet P or maxSet P
    push_neg at hB
    have hAmm : A = minSet P ∨ A = maxSet P := by
      by_contra h
      push_neg at h
      exact h.2 (hB A ⟨hAP, hAac, hAmax⟩ h.1)
    -- a comparable pair x ≤ y, x minimal, y maximal, with one of them in A
    have hpair : ∃ x y, x ∈ minSet P ∧ y ∈ maxSet P ∧ x ≤ y ∧ (x ∈ A ∨ y ∈ A) := by
      obtain ⟨p, hp⟩ := Finset.nonempty_iff_ne_empty.2 hPe
      rcases hAmm with h | h
      · obtain ⟨x, hx, _⟩ := exists_min_below P hp
        obtain ⟨y, hy, hxy⟩ := exists_max_above P ((Finset.mem_filter.1 hx).1)
        exact ⟨x, y, hx, hy, hxy, Or.inl (by rw [h]; exact hx)⟩
      · obtain ⟨y, hy, _⟩ := exists_max_above P hp
        obtain ⟨x, hx, hxy⟩ := exists_min_below P ((Finset.mem_filter.1 hy).1)
        exact ⟨x, y, hx, hy, hxy, Or.inr (by rw [h]; exact hy)⟩
    obtain ⟨x, y, hxmin, hymax, hxy, hxyA⟩ := hpair
    have hxP : x ∈ P := (Finset.mem_filter.1 hxmin).1
    have hyP : y ∈ P := (Finset.mem_filter.1 hymax).1
    -- the index in A that the pair removes
    let a0 := if x ∈ A then x else y
    have ha0A : a0 ∈ A := by
      simp only [a0]; split_ifs with h
      · exact h
      · rcases hxyA with h' | h'; exact absurd h' h; exact h'
    have ha0xy : a0 = x ∨ a0 = y := by
      simp only [a0]; split_ifs
      · exact Or.inl rfl
      · exact Or.inr rfl
    -- A contains at most one of x, y: if both, then x = y
    have hboth : x ∈ A → y ∈ A → x = y := fun hx hy => hAac x hx y hy hxy
    let P' := P \ {x, y}
    let A' := A.erase a0
    have hP'P : P' ⊆ P := Finset.sdiff_subset
    have hP'_ssub : P' ⊂ P := by
      rw [Finset.ssubset_iff_subset_ne]
      refine ⟨hP'P, ?_⟩
      intro heq
      have : x ∈ P' := by rw [heq]; exact hxP
      exact (Finset.mem_sdiff.1 this).2 (by simp)
    have hA'P' : A' ⊆ P' := by
      intro a ha
      obtain ⟨hne, haA⟩ := Finset.mem_erase.1 ha
      rw [Finset.mem_sdiff]
      refine ⟨hAP haA, ?_⟩
      intro hmem
      rcases Finset.mem_insert.1 hmem with h | h
      · -- a = x
        subst h
        rcases ha0xy with h0 | h0
        · exact hne h0.symm
        · -- a0 = y ∈ A and x ∈ A ⇒ x = y ⇒ a0 = x
          have hyA : y ∈ A := h0 ▸ ha0A
          exact hne (by rw [h0]; exact hboth haA hyA)
      · have h : a = y := Finset.mem_singleton.1 h
        subst h
        rcases ha0xy with h0 | h0
        · have hxA : x ∈ A := h0 ▸ ha0A
          exact hne (by rw [h0]; exact (hboth hxA haA).symm)
        · exact hne h0.symm
    have hA'max : MaxAntichain P' A' := by
      refine ⟨hA'P', fun u hu v hv => hAac u (Finset.mem_of_mem_erase hu) v (Finset.mem_of_mem_erase hv), ?_⟩
      intro C hC hCac
      by_contra hlt
      push_neg at hlt
      rw [Finset.card_erase_of_mem ha0A] at hlt
      have hCP : C ⊆ P := hC.trans hP'P
      have hCle : C.card ≤ A.card := hAmax C hCP hCac
      have hAne : 0 < A.card := Finset.card_pos.2 ⟨a0, ha0A⟩
      have hCeq : C.card = A.card := by omega
      have hCmax : MaxAntichain P C := ⟨hCP, hCac, fun D hD hDac => hCeq ▸ hAmax D hD hDac⟩
      have hCmm : C = minSet P ∨ C = maxSet P := by
        by_contra h
        push_neg at h
        exact h.2 (hB C hCmax h.1)
      rcases hCmm with h | h
      · have : x ∈ C := by rw [h]; exact hxmin
        exact (Finset.mem_sdiff.1 (hC this)).2 (by simp)
      · have : y ∈ C := by rw [h]; exact hymax
        exact (Finset.mem_sdiff.1 (hC this)).2 (by simp)
    obtain ⟨c', hc'⟩ := ih P' hP'_ssub A' hA'max
    refine ⟨fun a => if a = a0 then {x, y} else c' a, ?_, ?_⟩
    · intro a ha
      by_cases h : a = a0
      · simp only [if_pos h]
        refine ⟨?_, ?_, ?_⟩
        · intro u hu v hv
          rcases Finset.mem_insert.1 hu with hu | hu <;> rcases Finset.mem_insert.1 hv with hv | hv
          · rw [hu, hv]; exact Or.inl le_rfl
          · rw [hu, Finset.mem_singleton.1 hv]; exact Or.inl hxy
          · rw [Finset.mem_singleton.1 hu, hv]; exact Or.inr hxy
          · rw [Finset.mem_singleton.1 hu, Finset.mem_singleton.1 hv]; exact Or.inl le_rfl
        · intro u hu
          rcases Finset.mem_insert.1 hu with hu | hu
          · rw [hu]; exact hxP
          · rw [Finset.mem_singleton.1 hu]; exact hyP
        · rw [h]; rcases ha0xy with h0 | h0 <;> rw [h0] <;> simp
      · simp only [if_neg h]
        have ha' : a ∈ A' := Finset.mem_erase.2 ⟨h, ha⟩
        exact ⟨(hc'.1 a ha').1, (hc'.1 a ha').2.1.trans hP'P, (hc'.1 a ha').2.2⟩
    · intro z hz
      by_cases hzxy : z ∈ ({x, y} : Finset α)
      · refine ⟨a0, ha0A, ?_⟩
        simp only [if_pos rfl]; exact hzxy
      · have hzP' : z ∈ P' := Finset.mem_sdiff.2 ⟨hz, hzxy⟩
        obtain ⟨a, ha, hza⟩ := hc'.2 z hzP'
        refine ⟨a, Finset.mem_of_mem_erase ha, ?_⟩
        simp only [if_neg (Finset.ne_of_mem_erase ha)]; exact hza

/-- Corollary: the easy direction. Any antichain is no larger than any cover. -/
theorem antichain_le_cover {P A B : Finset α} {c : α → Finset α} (hcov : CoverBy P B c)
    (hAP : A ⊆ P) (hA : IsAntichainF A) : A.card ≤ B.card := by
  have hpick : ∀ a ∈ A, ∃ b ∈ B, a ∈ c b := fun a ha => hcov.2 a (hAP ha)
  let g : α → α := fun a => if h : a ∈ A then Classical.choose (hpick a h) else a
  apply Finset.card_le_card_of_injOn g
  · intro a ha
    have ha' : a ∈ A := ha
    simp only [g, dif_pos ha']
    exact (Classical.choose_spec (hpick a ha')).1
  · intro a₁ h₁ a₂ h₂ heq
    have h₁' : a₁ ∈ A := h₁
    have h₂' : a₂ ∈ A := h₂
    simp only [g, dif_pos h₁', dif_pos h₂'] at heq
    have hb := (Classical.choose_spec (hpick a₁ h₁')).1
    have hC := (hcov.1 _ hb).1
    have h2 : a₂ ∈ c (Classical.choose (hpick a₁ h₁')) := by
      rw [heq]; exact (Classical.choose_spec (hpick a₂ h₂')).2
    exact chain_meets_antichain hC hA h₁' h₂' (Classical.choose_spec (hpick a₁ h₁')).2 h2

end Dilworth

/-! ### Dilworth in the classical form: min #chains = width -/
namespace Dilworth
variable {α : Type*} [PartialOrder α]

/-- Every finite poset has a maximum antichain. -/
theorem exists_maxAntichain (P : Finset α) : ∃ A, MaxAntichain P A := by
  classical
  let S := P.powerset.filter IsAntichainF
  have hS : S.Nonempty := ⟨∅, by simp [S, IsAntichainF]⟩
  obtain ⟨A, hA, hmax⟩ := Finset.exists_max_image S Finset.card hS
  have hA' := Finset.mem_filter.1 hA
  refine ⟨A, Finset.mem_powerset.1 hA'.1, hA'.2, ?_⟩
  intro B hB hBac
  exact hmax B (Finset.mem_filter.2 ⟨Finset.mem_powerset.2 hB, hBac⟩)

/-- width = size of a maximum antichain; the theorem gives a cover of exactly that
    many chains, and no cover can be smaller. -/
theorem width_eq_min_cover (P : Finset α) :
    ∃ A c, MaxAntichain P A ∧ CoverBy P A c ∧
      ∀ B (c' : α → Finset α), CoverBy P B c' → A.card ≤ B.card := by
  obtain ⟨A, hA⟩ := exists_maxAntichain P
  obtain ⟨c, hc⟩ := dilworth P A hA
  exact ⟨A, c, hA, hc, fun B c' hc' => antichain_le_cover hc' hA.1 hA.2.1⟩

end Dilworth

/-! ### Decomposition form: the chains can be made pairwise disjoint -/
namespace Dilworth
variable {α : Type*} [PartialOrder α]

theorem disjoint_cover {P B : Finset α} {c : α → Finset α} (hcov : CoverBy P B c) :
    ∃ d : α → Finset α,
      (∀ b ∈ B, IsChainF (d b) ∧ d b ⊆ P) ∧ (∀ x ∈ P, ∃ b ∈ B, x ∈ d b) ∧
      (∀ b₁ ∈ B, ∀ b₂ ∈ B, b₁ ≠ b₂ → Disjoint (d b₁) (d b₂)) := by
  classical
  -- send every element to one chain that contains it
  let pick : α → α := fun x => if h : x ∈ P then Classical.choose (hcov.2 x h) else x
  have hpick : ∀ x (h : x ∈ P), pick x ∈ B ∧ x ∈ c (pick x) := by
    intro x h
    simp only [pick, dif_pos h]
    exact Classical.choose_spec (hcov.2 x h)
  refine ⟨fun b => (c b).filter (fun x => pick x = b), ?_, ?_, ?_⟩
  · intro b hb
    refine ⟨?_, ?_⟩
    · intro u hu v hv
      exact (hcov.1 b hb).1 u (Finset.mem_filter.1 hu).1 v (Finset.mem_filter.1 hv).1
    · intro u hu
      exact (hcov.1 b hb).2.1 (Finset.mem_filter.1 hu).1
  · intro x hx
    exact ⟨pick x, (hpick x hx).1, Finset.mem_filter.2 ⟨(hpick x hx).2, rfl⟩⟩
  · intro b₁ _ b₂ _ hne
    rw [Finset.disjoint_left]
    intro x h₁ h₂
    have e₁ := (Finset.mem_filter.1 h₁).2
    have e₂ := (Finset.mem_filter.1 h₂).2
    exact hne (e₁.symm.trans e₂)

/-- Dilworth's decomposition theorem: P is the disjoint union of width(P) chains. -/
theorem decomposition (P : Finset α) :
    ∃ (A : Finset α) (d : α → Finset α), MaxAntichain P A ∧
      (∀ b ∈ A, IsChainF (d b) ∧ d b ⊆ P) ∧ (∀ x ∈ P, ∃ b ∈ A, x ∈ d b) ∧
      (∀ b₁ ∈ A, ∀ b₂ ∈ A, b₁ ≠ b₂ → Disjoint (d b₁) (d b₂)) := by
  obtain ⟨A, hA⟩ := exists_maxAntichain P
  obtain ⟨c, hc⟩ := dilworth P A hA
  obtain ⟨d, hd⟩ := disjoint_cover hc
  exact ⟨A, d, hA, hd⟩

end Dilworth
