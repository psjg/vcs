/-
  v1/FORMAL.md 3.5 / 3.6 — holes and redaction.
  A log with holes: only some events are known. Ideals live inside the known
  part; an event whose dependency is a hole is in no ideal. Bodies are a
  separate store; the rendered *structure* of a world ignores them.
-/
import Worlds

namespace Holes
open Worlds
variable {α : Type} [Poset α]

/-- an ideal of the known part of a log -/
def KnownIdeal (Known S : Set α) : Prop := (∀ x, S x → Known x) ∧ IsIdeal S

/-- 3.5: an event with an unknown dependency belongs to no ideal -/
theorem hole_excludes {Known S : Set α} (h : KnownIdeal Known S) {e r : α}
    (hre : r ≤ e) (hr : ¬ Known r) : ¬ S e :=
  fun he => hr (h.1 r (h.2 e r he hre))

/-- 3.6: a world's structure is a function of the world; the rendering
    consults a body store only per atom. Removing a body changes the
    rendering at that atom alone. -/
def render {Body : Type} (structure_ : List α) (bodies : α → Option Body) : List (α × Option Body) :=
  structure_.map (fun a => (a, bodies a))

theorem redact_local {Body : Type} (st : List α) (bodies : α → Option Body) (t : α)
    [DecidableEq α] :
    let bodies' := fun a => if a = t then none else bodies a
    ∀ a, a ∈ st → a ≠ t →
      (a, bodies a) ∈ render st bodies ∧ (a, bodies a) ∈ render st bodies' := by
  intro bodies' a ha hne
  constructor
  · exact List.mem_map.mpr ⟨a, ha, rfl⟩
  · refine List.mem_map.mpr ⟨a, ha, ?_⟩
    simp [bodies', hne]

/-- the structure itself never mentions bodies: same world, same structure,
    whatever the store says -/
theorem structure_indep {Body : Type} (st : List α) (b b' : α → Option Body) :
    (render st b).map Prod.fst = (render st b').map Prod.fst := by
  simp [render, List.map_map]

end Holes
