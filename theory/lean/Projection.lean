/-
  v1/FORMAL.md 3.9 — projections are deterministic and append-only.
  A projection folds a path of worlds into a chain of commits, each commit
  id a hash of (parent id, content of the step). Determinism is being a
  function; append-only is that extending the path extends the output as a
  prefix, and leaves every earlier id unchanged.
-/
namespace Projection

variable {W C Id : Type}

/-- one exported step: content of the step plus the hash rule -/
structure Rule (W C Id : Type) where
  content : W → W → C           -- diff from previous world to this one
  hash    : Id → C → Id         -- commit id from parent id and content
  root    : Id

/-- project a path starting from `prev`, threading the parent id -/
def run (R : Rule W C Id) : W → Id → List W → List (Id × C)
  | _, _, [] => []
  | prev, pid, w :: ws =>
    let c := R.content prev w
    let id := R.hash pid c
    (id, c) :: run R w id ws

def project (R : Rule W C Id) (start : W) (π : List W) : List (Id × C) :=
  run R start R.root π

/-- append-only: projecting an extended path yields the old projection as a
    prefix, with the same ids -/
theorem run_append (R : Rule W C Id) :
    ∀ (π ρ : List W) (prev : W) (pid : Id),
      ∃ (w' : W) (id' : Id), run R prev pid (π ++ ρ) = run R prev pid π ++ run R w' id' ρ
  | [], ρ, prev, pid => ⟨prev, pid, by simp [run]⟩
  | w :: ws, ρ, prev, pid => by
    obtain ⟨w', id', h⟩ := run_append R ws ρ w (R.hash pid (R.content prev w))
    exact ⟨w', id', by simp only [List.cons_append, run]; rw [h]⟩

theorem project_append (R : Rule W C Id) (start : W) (π ρ : List W) :
    ∃ tail, project R start (π ++ ρ) = project R start π ++ tail := by
  obtain ⟨_, _, h⟩ := run_append R π ρ start R.root
  exact ⟨_, h⟩

/-- determinism, stated as congruence: equal inputs, equal output (it is a
    function; recorded so the property has a name to cite) -/
theorem project_det (R : Rule W C Id) (start : W) (π : List W) :
    project R start π = project R start π := rfl

/-- ids are content-addressed along the chain: the k-th id depends only on
    the first k steps -/
theorem ids_prefix (R : Rule W C Id) (start : W) (π ρ : List W) :
    ∃ tail, (project R start (π ++ ρ)).map Prod.fst = (project R start π).map Prod.fst ++ tail := by
  obtain ⟨tail, h⟩ := project_append R start π ρ
  exact ⟨tail.map Prod.fst, by rw [h, List.map_append]⟩

end Projection
