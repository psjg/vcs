/-
  v1/FORMAL.md 7.6 — coherence with contraction.
  Contract a box (a set of tasks containing its representative b) to the
  point b. Under two conditions — no blockedBy edge crosses the box boundary
  except uniformly, and the done-set contains the box wholly or not at all —
  ideals map to ideals and minimal elements outside the box stay minimal.
-/
import Worlds

namespace Contraction
open Worlds
variable {α : Type} [Poset α]

variable (box : α → Prop) (b : α) [DecidablePred box]

def q (x : α) : α := if box x then b else x

/-- the quotient relation -/
def qle (u v : α) : Prop := ∃ x y, q box b x = u ∧ q box b y = v ∧ x ≤ y

def image (D : Set α) : Set α := fun u => ∃ x, D x ∧ q box b x = u

def IsDown (R : α → α → Prop) (S : Set α) : Prop := ∀ u v, S u → R v u → S v

/-- the box is done as a whole or not at all -/
def BoxAtomic (D : Set α) : Prop := (∀ x, box x → D x) ∨ (∀ x, box x → ¬ D x)

theorem q_eq_self {x : α} (h : ¬ box x) : q box b x = x := by simp [q, h]
theorem q_eq_b {x : α} (h : box x) : q box b x = b := by simp [q, h]

/-- 7.6 (i): the image of an ideal is a down-set of the quotient -/
theorem image_down (hb : box b) (D : Set α) (hD : IsIdeal D) (hat : BoxAtomic box D) :
    IsDown (qle box b) (image box b D) := by
  intro u v ⟨x, hxD, hqx⟩ ⟨y', x', hqy', hqx', hle⟩
  refine ⟨y', ?_, hqy'⟩
  by_cases hbx' : box x'
  · -- u = b, so x is in the box too, so the box is done, so x' ∈ D
    have hu : u = b := by rw [← hqx']; exact q_eq_b box b hbx'
    have hbx : box x := by
      apply Classical.byContradiction; intro hn
      have : u = x := by rw [← hqx]; exact q_eq_self box b hn
      exact hn (by rw [← this, hu]; exact hb)
    rcases hat with hall | hnone
    · exact hD x' y' (hall x' hbx') hle
    · exact absurd hxD (hnone x hbx)
  · have : x' = u := by rw [← hqx']; exact (q_eq_self box b hbx').symm
    subst this
    by_cases hbx : box x
    · have : x' = b := by rw [← hqx]; exact q_eq_b box b hbx
      exact absurd (this ▸ hb) hbx'
    · have : x = x' := by rw [← hqx]; exact (q_eq_self box b hbx).symm
      rw [this] at hxD
      exact hD x' y' hxD hle

/-- 7.6 (ii): an element outside the box that is minimal in the complement
    of D stays minimal after contraction -/
theorem min_preserved (hb : box b) (D : Set α) (hat : BoxAtomic box D)
    {x : α} (hbx : ¬ box x) (hxD : ¬ D x)
    (hmin : ∀ y, y ≤ x → ¬ D y → y = x) :
    ∀ v, image box b (fun z => ¬ D z) v → qle box b v (q box b x) → v = q box b x := by
  intro v ⟨z, hzD, hqz⟩ ⟨y', x', hqy', hqx', hle⟩
  rw [q_eq_self box b hbx] at hqx' ⊢
  have hx' : x' = x := by
    by_cases h : box x'
    · have : b = x := by rw [← hqx']; exact (q_eq_b box b h).symm
      exact absurd (this ▸ hb) hbx
    · rw [← hqx']; exact (q_eq_self box b h).symm
  subst hx'
  by_cases hby : box y'
  · -- v = b; the witness z is in the box, so the box is not done, so y' ∉ D
    have hv : v = b := by rw [← hqy']; exact q_eq_b box b hby
    have hbz : box z := by
      apply Classical.byContradiction; intro hn
      have : v = z := by rw [← hqz]; exact q_eq_self box b hn
      exact hn (by rw [← this, hv]; exact hb)
    rcases hat with hall | hnone
    · exact absurd (hall z hbz) hzD
    · have := hmin y' hle (hnone y' hby)
      exact absurd (this ▸ hby) hbx
  · have hv : v = y' := by rw [← hqy']; exact q_eq_self box b hby
    subst hv
    have hzv : z = v := by
      by_cases h : box z
      · have : v = b := by rw [← hqz]; exact q_eq_b box b h
        exact absurd (this ▸ hb) hby
      · rw [← hqz]; exact (q_eq_self box b h).symm
    subst hzv
    exact hmin z hle hzD

end Contraction
