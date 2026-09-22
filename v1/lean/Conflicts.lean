/-
  v1/FORMAL.md §5 — conflicts, on top of Worlds.lean.
  Targets (what an op overwrites) are a separate type `T` that may include
  conflict ids, so that resolutions overwrite the conflict they resolve (5.7).
-/
import Worlds

namespace Conflicts
open Worlds
variable {α T : Type} [Poset α]

/-- what an event overwrites -/
structure Ops (α T : Type) where
  ow : α → T → Prop

/-- 5.2, default predicate without the type-specific `Agree` refinement -/
def Conf (O : Ops α T) (e f : α) : Prop :=
  ¬ e ≤ f ∧ ¬ f ≤ e ∧ ∃ t, O.ow e t ∧ O.ow f t

/-- 5.3/5.4: a conflict is a target with a set of sides; a resolution lies
    above every side and declares the conflict -/
structure Conflict (α T : Type) where
  target : T
  sides  : Set α

def IsResolution (O : Ops α T) (κ : Conflict α T) (cid : T) (r : α) : Prop :=
  (∀ s, κ.sides s → s ≤ r) ∧ O.ow r cid

def Open (O : Ops α T) (κ : Conflict α T) (cid : T) (S : Set α) : Prop :=
  (∀ s, κ.sides s → S s) ∧ ¬ ∃ r, S r ∧ IsResolution O κ cid r

/-- 5.5: dropping any side drops every resolution. -/
theorem drop_side_drops_resolution (O : Ops α T) (κ : Conflict α T) (cid : T)
    {S : Set α} {s r : α} (hs : κ.sides s) (hr : IsResolution O κ cid r) :
    ¬ drop S (fun x => x = s) r := by
  intro ⟨_, hnot⟩
  exact hnot ⟨s, rfl, hr.1 s hs⟩

/-- 5.5, second half: a resolution is an upper bound of the sides -/
theorem resolution_upper (O : Ops α T) (κ : Conflict α T) (cid : T) {r : α}
    (hr : IsResolution O κ cid r) : ∀ s, κ.sides s → s ≤ r := hr.1

/-- 5.6: agreement is defined through `M` on the two ideals `S \ ↑f`, `S \ ↑e`;
    it is symmetric and depends only on `M` and `S` (well-defined by 3.2). -/
def Agree {Doc : Type} (M : Set α → Doc) (S : Set α) (e f : α) : Prop :=
  M (drop S (fun x => x = f)) = M (drop S (fun x => x = e))

theorem agree_symm {Doc : Type} (M : Set α → Doc) (S : Set α) (e f : α) :
    Agree M S e f → Agree M S f e := fun h => h.symm

theorem agree_refl {Doc : Type} (M : Set α → Doc) (S : Set α) (e : α) : Agree M S e e := rfl

/-- 5.7: two incomparable resolutions of one conflict are in conflict on the
    conflict id — the same predicate, one level up. -/
theorem nested (O : Ops α T) (κ : Conflict α T) (cid : T) {r₁ r₂ : α}
    (h₁ : IsResolution O κ cid r₁) (h₂ : IsResolution O κ cid r₂)
    (hinc : ¬ r₁ ≤ r₂ ∧ ¬ r₂ ≤ r₁) : Conf O r₁ r₂ :=
  ⟨hinc.1, hinc.2, cid, h₁.2, h₂.2⟩

end Conflicts
