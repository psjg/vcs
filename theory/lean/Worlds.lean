/-
  v1/FORMAL.md §2 — worlds as ideals of a poset.
  Self-contained: core Lean 4, no Mathlib. Sets are predicates `α → Prop`.
-/

class Poset (α : Type) extends LE α where
  refl  : ∀ a : α, a ≤ a
  trans : ∀ {a b c : α}, a ≤ b → b ≤ c → a ≤ c
  antisymm : ∀ {a b : α}, a ≤ b → b ≤ a → a = b

namespace Worlds
variable {α : Type} [Poset α]

abbrev Set (α : Type) := α → Prop

theorem set_ext {S T : Set α} (h : ∀ x, S x ↔ T x) : S = T :=
  funext (fun x => propext (h x))

/-- 2.1 -/
def IsIdeal (S : Set α) : Prop := ∀ e r, S e → r ≤ e → S r
/-- ↓C -/
def Down (C : Set α) (x : α) : Prop := ∃ c, C c ∧ x ≤ c
/-- ↑C -/
def Up (C : Set α) (x : α) : Prop := ∃ c, C c ∧ c ≤ x
/-- order-convexity -/
def Convex (C : Set α) : Prop := ∀ a b c, C a → C c → a ≤ b → b ≤ c → C b
/-- 2.6 -/
def adopt (S C : Set α) : Set α := fun x => S x ∨ Down C x
def drop  (S C : Set α) : Set α := fun x => S x ∧ ¬ Up C x
def diff  (T S : Set α) : Set α := fun x => T x ∧ ¬ S x

/-! ## 2.2 (the sublattice half) -/

theorem ideal_union {S T : Set α} (hS : IsIdeal S) (hT : IsIdeal T) :
    IsIdeal (fun x => S x ∨ T x) := by
  intro e r h hle
  cases h with
  | inl hs => exact Or.inl (hS e r hs hle)
  | inr ht => exact Or.inr (hT e r ht hle)

theorem ideal_inter {S T : Set α} (hS : IsIdeal S) (hT : IsIdeal T) :
    IsIdeal (fun x => S x ∧ T x) := by
  intro e r ⟨hs, ht⟩ hle
  exact ⟨hS e r hs hle, hT e r ht hle⟩

theorem down_ideal (C : Set α) : IsIdeal (Down C) := by
  intro e r ⟨c, hc, hec⟩ hre
  exact ⟨c, hc, Poset.trans hre hec⟩

/-! ## 2.6 -/

theorem adopt_ideal {S : Set α} (C : Set α) (hS : IsIdeal S) : IsIdeal (adopt S C) :=
  ideal_union hS (down_ideal C)

theorem drop_ideal {S : Set α} (C : Set α) (hS : IsIdeal S) : IsIdeal (drop S C) := by
  intro e r ⟨hs, hnot⟩ hle
  refine ⟨hS e r hs hle, ?_⟩
  intro ⟨c, hc, hcr⟩
  exact hnot ⟨c, hc, Poset.trans hcr hle⟩

/-! ## 2.4 convex difference -/

theorem diff_convex {S T : Set α} (hS : IsIdeal S) (hT : IsIdeal T) :
    Convex (diff T S) := by
  intro a b c ⟨_, hnsa⟩ ⟨htc, _⟩ hab hbc
  refine ⟨hT c b htc hbc, ?_⟩
  intro hsb
  exact hnsa (hS b a hsb hab)

/-- converse of 2.4: for an ideal S and convex D disjoint from S,
    S ∪ D is an ideal iff ↓D \ D ⊆ S. -/
theorem union_ideal_iff {S D : Set α} (hS : IsIdeal S)
    (hdisj : ∀ x, ¬ (S x ∧ D x)) :
    IsIdeal (fun x => S x ∨ D x) ↔ (∀ x, Down D x → ¬ D x → S x) := by
  constructor
  · intro h x ⟨d, hd, hxd⟩ hnd
    have := h d x (Or.inr hd) hxd
    cases this with
    | inl hs => exact hs
    | inr hd' => exact absurd hd' hnd
  · intro h e r he hre
    cases he with
    | inl hs => exact Or.inl (hS e r hs hre)
    | inr hd =>
      by_cases hr : D r
      · exact Or.inr hr
      · exact Or.inl (h r ⟨e, hd, hre⟩ hr)

/-! ## 2.7 cover laws -/

/-- 2.7(a): drop(adopt(S,C),C) = S  iff  C ∩ S = ∅ ∧ ↓C \ C ⊆ S. -/
theorem cover_a {S C : Set α} (hS : IsIdeal S) (hC : Convex C) :
    drop (adopt S C) C = S ↔
      (∀ x, ¬ (C x ∧ S x)) ∧ (∀ x, Down C x → ¬ C x → S x) := by
  constructor
  · intro heq
    constructor
    · intro x ⟨hcx, hsx⟩
      have hx : drop (adopt S C) C x := by rw [heq]; exact hsx
      exact hx.2 ⟨x, hcx, Poset.refl x⟩
    · intro x hdx hncx
      have hx : drop (adopt S C) C x := by
        refine ⟨Or.inr hdx, ?_⟩
        intro ⟨c', hc', hc'x⟩
        obtain ⟨c, hc, hxc⟩ := hdx
        exact hncx (hC c' x c hc' hc hc'x hxc)
      rw [heq] at hx
      exact hx
  · intro ⟨hdisj, hbelow⟩
    apply set_ext
    intro x
    constructor
    · intro ⟨hadopt, hnup⟩
      cases hadopt with
      | inl hs => exact hs
      | inr hd =>
        by_cases hcx : C x
        · exact absurd ⟨x, hcx, Poset.refl x⟩ hnup
        · exact hbelow x hd hcx
    · intro hsx
      refine ⟨Or.inl hsx, ?_⟩
      intro ⟨c, hc, hcx⟩
      exact hdisj c ⟨hc, hS x c hsx hcx⟩

/-- 2.7(b): adopt(drop(S,C),C) = S  iff  C ⊆ S ∧ S \ C is an ideal. -/
theorem cover_b {S C : Set α} (hS : IsIdeal S) (hC : Convex C) :
    adopt (drop S C) C = S ↔
      (∀ x, C x → S x) ∧ IsIdeal (fun x => S x ∧ ¬ C x) := by
  constructor
  · intro heq
    constructor
    · intro x hcx
      have hx : adopt (drop S C) C x := Or.inr ⟨x, hcx, Poset.refl x⟩
      rw [heq] at hx
      exact hx
    · intro e r ⟨hse, hnce⟩ hre
      refine ⟨hS e r hse hre, ?_⟩
      intro hcr
      have he : adopt (drop S C) C e := by rw [heq]; exact hse
      cases he with
      | inl hdrop => exact hdrop.2 ⟨r, hcr, hre⟩
      | inr hdown =>
        obtain ⟨c, hc, hec⟩ := hdown
        exact hnce (hC r e c hcr hc hre hec)
  · intro ⟨hsub, hideal⟩
    apply set_ext
    intro x
    constructor
    · intro h
      cases h with
      | inl hdrop => exact hdrop.1
      | inr hdown =>
        obtain ⟨c, hc, hxc⟩ := hdown
        exact hS c x (hsub c hc) hxc
    · intro hsx
      by_cases hup : Up C x
      · obtain ⟨c, hc, hcx⟩ := hup
        by_cases hcx' : C x
        · exact Or.inr ⟨x, hcx', Poset.refl x⟩
        · exact absurd hc (hideal x c ⟨hsx, hcx'⟩ hcx).2
      · exact Or.inl ⟨hsx, hup⟩

/-! ## 2.8 inverse pairs -/

theorem adopt_diff {S T : Set α} (hT : IsIdeal T) (hsub : ∀ x, S x → T x) :
    adopt S (diff T S) = T := by
  apply set_ext
  intro x
  constructor
  · intro h
    cases h with
    | inl hs => exact hsub x hs
    | inr hd =>
      obtain ⟨d, ⟨htd, _⟩, hxd⟩ := hd
      exact hT d x htd hxd
  · intro htx
    by_cases hsx : S x
    · exact Or.inl hsx
    · exact Or.inr ⟨x, ⟨htx, hsx⟩, Poset.refl x⟩

theorem drop_diff {S T : Set α} (hS : IsIdeal S) (hsub : ∀ x, S x → T x) :
    drop T (diff T S) = S := by
  apply set_ext
  intro x
  constructor
  · intro ⟨htx, hnup⟩
    by_cases hsx : S x
    · exact hsx
    · exact absurd ⟨x, ⟨htx, hsx⟩, Poset.refl x⟩ hnup
  · intro hsx
    refine ⟨hsub x hsx, ?_⟩
    intro ⟨d, ⟨_, hnsd⟩, hdx⟩
    exact hnsd (hS x d hsx hdx)

/-! ## 2.9 path independence — the single step, and the fold -/

/-- one cover step: adopting a minimal remaining element adds exactly it -/
theorem step {S T : Set α} (hS : IsIdeal S) (hT : IsIdeal T)
    (hsub : ∀ x, S x → T x) (e : α) (he : T e)
    (hmin : ∀ r, r ≤ e → r ≠ e → S r) :
    adopt S (fun x => x = e) = (fun x => S x ∨ x = e) ∧
    IsIdeal (fun x => S x ∨ x = e) ∧ (∀ x, (S x ∨ x = e) → T x) := by
  refine ⟨?_, ?_, ?_⟩
  · apply set_ext
    intro x
    constructor
    · intro h
      cases h with
      | inl hs => exact Or.inl hs
      | inr hd =>
        obtain ⟨c, hce, hxc⟩ := hd
        subst hce
        by_cases hx : x = c
        · exact Or.inr hx
        · exact Or.inl (hmin x hxc hx)
    · intro h
      cases h with
      | inl hs => exact Or.inl hs
      | inr hx => exact Or.inr ⟨e, rfl, by rw [hx]; exact Poset.refl e⟩
  · intro a r ha hra
    cases ha with
    | inl hs => exact Or.inl (hS a r hs hra)
    | inr hae =>
      subst hae
      by_cases hr : r = a
      · exact Or.inr hr
      · exact Or.inl (hmin r hra hr)
  · intro x h
    cases h with
    | inl hs => exact hsub x hs
    | inr hx => rw [hx]; exact he

/-- a list is a "linear extension" of the events it carries if no later
    element lies below an earlier one -/
def MinFirst : List α → Prop
  | [] => True
  | e :: L => (∀ r, r ∈ L → ¬ r ≤ e) ∧ MinFirst L

def foldAdopt (S : Set α) : List α → Set α
  | [] => S
  | e :: L => foldAdopt (adopt S (fun x => x = e)) L

/-- 2.9: folding single-event adopts along any min-first enumeration of
    T \ S reaches exactly T. -/
theorem path_independence {S T : Set α} (hS : IsIdeal S) (hT : IsIdeal T)
    (hsub : ∀ x, S x → T x) (L : List α)
    (hmem : ∀ x, x ∈ L ↔ diff T S x) (hmf : MinFirst L) :
    foldAdopt S L = T := by
  induction L generalizing S with
  | nil =>
    apply set_ext
    intro x
    constructor
    · exact hsub x
    · intro htx
      by_cases hsx : S x
      · exact hsx
      · exact absurd ((hmem x).2 ⟨htx, hsx⟩) (by simp)
  | cons e L ih =>
    have he : T e := ((hmem e).1 (by simp)).1
    have hmin : ∀ r, r ≤ e → r ≠ e → S r := by
      intro r hre hne
      by_cases hsr : S r
      · exact hsr
      · have hr : r ∈ e :: L := (hmem r).2 ⟨hT e r he hre, hsr⟩
        cases List.mem_cons.mp hr with
        | inl h => exact absurd h hne
        | inr h => exact absurd hre (hmf.1 r h)
    obtain ⟨heq, hideal, hsub'⟩ := step hS hT hsub e he hmin
    show foldAdopt (adopt S (fun x => x = e)) L = T
    rw [heq]
    apply ih hideal hsub'
    · intro x
      constructor
      · intro hx
        have hx' : x ∈ e :: L := List.mem_cons_of_mem e hx
        obtain ⟨htx, hnsx⟩ := (hmem x).1 hx'
        refine ⟨htx, ?_⟩
        intro h
        cases h with
        | inl hs => exact hnsx hs
        | inr hxe => subst hxe; exact hmf.1 x hx (Poset.refl x)
      · intro ⟨htx, hn⟩
        have hnsx : ¬ S x := fun hs => hn (Or.inl hs)
        have hx : x ∈ e :: L := (hmem x).2 ⟨htx, hnsx⟩
        cases List.mem_cons.mp hx with
        | inl h => exact absurd (Or.inr h) hn
        | inr h => exact h
    · exact hmf.2

end Worlds
