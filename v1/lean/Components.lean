/-
  v1/FORMAL.md 2.12 — components of pending work are convex.
  Here `≤` is the reflexive–transitive closure of a dependency relation,
  so that a chain witness is available.
-/
namespace Components
variable {α : Type} (dep : α → α → Prop)

/-- reflexive–transitive closure: `Path a b` is `a ≤ b` -/
inductive Path : α → α → Prop
  | refl (a : α) : Path a a
  | step {a b c : α} : dep a b → Path b c → Path a c

/-- connectivity inside Q through dep-edges in either direction -/
inductive Conn (Q : α → Prop) : α → α → Prop
  | refl {a : α} : Q a → Conn Q a a
  | fwd {a b c : α} : Q a → Q b → dep a b → Conn Q b c → Conn Q a c
  | bwd {a b c : α} : Q a → Q b → dep b a → Conn Q b c → Conn Q a c

def UpSet (Q : α → Prop) : Prop := ∀ a b, Q a → Path dep a b → Q b

/-- every node on a dep-path out of a pending node is pending and connected
    to its start within the pending set -/
theorem path_conn {Q : α → Prop} (hQ : UpSet dep Q) {a b : α}
    (ha : Q a) (p : Path dep a b) : Conn dep Q a b := by
  induction p with
  | refl a => exact Conn.refl ha
  | @step x y z hxy _ ih =>
    have hy : Q y := hQ x y ha (Path.step hxy (Path.refl y))
    exact Conn.fwd ha hy hxy (ih hy)

theorem conn_trans {Q : α → Prop} {a b c : α}
    (h1 : Conn dep Q a b) (h2 : Conn dep Q b c) : Conn dep Q a c := by
  induction h1 with
  | refl _ => exact h2
  | fwd ha hb hab _ ih => exact Conn.fwd ha hb hab (ih h2)
  | bwd ha hb hba _ ih => exact Conn.bwd ha hb hba (ih h2)

theorem conn_symm {Q : α → Prop} {a b : α} (h : Conn dep Q a b) :
    Conn dep Q b a := by
  induction h with
  | refl ha => exact Conn.refl ha
  | fwd ha hb hab _ ih => exact conn_trans dep ih (Conn.bwd hb ha hab (Conn.refl ha))
  | bwd ha hb hba _ ih => exact conn_trans dep ih (Conn.fwd hb ha hba (Conn.refl ha))

/-- 2.12: if `a` and `c` are in one component of the pending set and
    `a ≤ b ≤ c`, then `b` is in that component. -/
theorem component_convex {Q : α → Prop} (hQ : UpSet dep Q) {a b c : α}
    (ha : Q a) (_hc : Q c) (_hac : Conn dep Q a c)
    (hab : Path dep a b) (_hbc : Path dep b c) :
    Q b ∧ Conn dep Q a b :=
  ⟨hQ a b ha hab, path_conn dep hQ ha hab⟩

end Components
