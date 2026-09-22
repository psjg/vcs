/-
  v1/FORMAL.md §3 — the text materialiser, modelled intrinsically.

  A Fugue node is identified by its path from the root: a list of
  (side, key) steps. Reading order is the in-order traversal: left subtree,
  node, right subtree, siblings on one side ordered by key. We define that
  order directly on paths and prove it is a strict total order. Because the
  order is a property of two paths alone, it is the same in every world:
  M(S) is the restriction of one global order to the live atoms of S.

  Scope: inserts and deletes (v1 core). Moves change a node's path and are
  not modelled here.
-/
namespace Fugue

/-- keys are (hlc, id): a strict total order with decidable equality -/
class Key (K : Type) where
  lt : K → K → Prop
  irrefl : ∀ a, ¬ lt a a
  trans : ∀ {a b c}, lt a b → lt b c → lt a c
  total : ∀ a b, lt a b ∨ a = b ∨ lt b a
  deq : DecidableEq K

variable {K : Type} [Key K]
attribute [instance] Key.deq

inductive Side | L | R
  deriving DecidableEq

abbrev Step (K : Type) := Side × K
abbrev Path (K : Type) := List (Step K)

/-- one step of the traversal order -/
def stepLt (x y : Step K) : Prop :=
  (x.1 = Side.L ∧ y.1 = Side.R) ∨ (x.1 = y.1 ∧ Key.lt x.2 y.2)

/-- the in-order traversal as a relation on paths -/
def lt : Path K → Path K → Prop
  | [], [] => False
  | [], y :: _ => y.1 = Side.R          -- right descendants come after the node
  | x :: _, [] => x.1 = Side.L          -- left descendants come before it
  | x :: xs, y :: ys => if x = y then lt xs ys else stepLt x y

theorem stepLt_irrefl (x : Step K) : ¬ stepLt x x := by
  intro h
  rcases h with ⟨h1, h2⟩ | ⟨_, h2⟩
  · rw [h1] at h2; cases h2
  · exact Key.irrefl _ h2

theorem stepLt_trans {x y z : Step K} (h1 : stepLt x y) (h2 : stepLt y z) : stepLt x z := by
  rcases h1 with ⟨a1, b1⟩ | ⟨a1, b1⟩ <;> rcases h2 with ⟨a2, b2⟩ | ⟨a2, b2⟩
  · rw [b1] at a2; cases a2
  · exact Or.inl ⟨a1, a2 ▸ b1⟩
  · exact Or.inl ⟨a1 ▸ a2, b2⟩
  · exact Or.inr ⟨a1.trans a2, Key.trans b1 b2⟩

theorem stepLt_total (x y : Step K) : stepLt x y ∨ x = y ∨ stepLt y x := by
  obtain ⟨sx, kx⟩ := x; obtain ⟨sy, ky⟩ := y
  cases sx <;> cases sy
  · rcases Key.total kx ky with h | h | h
    · exact Or.inl (Or.inr ⟨rfl, h⟩)
    · exact Or.inr (Or.inl (by rw [h]))
    · exact Or.inr (Or.inr (Or.inr ⟨rfl, h⟩))
  · exact Or.inl (Or.inl ⟨rfl, rfl⟩)
  · exact Or.inr (Or.inr (Or.inl ⟨rfl, rfl⟩))
  · rcases Key.total kx ky with h | h | h
    · exact Or.inl (Or.inr ⟨rfl, h⟩)
    · exact Or.inr (Or.inl (by rw [h]))
    · exact Or.inr (Or.inr (Or.inr ⟨rfl, h⟩))

/-! ### `lt` is a strict total order on all paths -/

theorem lt_irrefl : ∀ p : Path K, ¬ lt p p
  | [] => fun h => h
  | x :: xs => by
    intro h
    simp only [lt, if_pos rfl] at h
    exact lt_irrefl xs h

theorem lt_trans : ∀ {p q r : Path K}, lt p q → lt q r → lt p r
  | [], [], _, h1, _ => (h1 : False).elim
  | [], y :: ys, [], h1, h2 => by
    simp only [lt] at h1 h2; rw [h1] at h2; cases h2
  | [], y :: ys, z :: zs, h1, h2 => by
    simp only [lt] at h1 h2 ⊢
    by_cases hyz : y = z
    · rw [← hyz]; exact h1
    · rw [if_neg hyz] at h2
      rcases h2 with ⟨_, h⟩ | ⟨h, _⟩
      · exact h
      · rw [← h]; exact h1
  | x :: xs, [], [], h1, h2 => (h2 : False).elim
  | x :: xs, [], z :: zs, h1, h2 => by
    simp only [lt] at h1 h2 ⊢
    by_cases hxz : x = z
    · rw [← hxz] at h2; rw [h1] at h2; cases h2
    · rw [if_neg hxz]; exact Or.inl ⟨h1, h2⟩
  | x :: xs, y :: ys, [], h1, h2 => by
    simp only [lt] at h1 h2 ⊢
    by_cases hxy : x = y
    · rw [hxy]; exact h2
    · rw [if_neg hxy] at h1
      rcases h1 with ⟨h, _⟩ | ⟨h, _⟩
      · exact h
      · rw [h]; exact h2
  | x :: xs, y :: ys, z :: zs, h1, h2 => by
    simp only [lt] at h1 h2 ⊢
    by_cases hxy : x = y <;> by_cases hyz : y = z
    · subst hxy; subst hyz
      rw [if_pos rfl] at h1 h2 ⊢; exact lt_trans h1 h2
    · subst hxy
      rw [if_pos rfl] at h1; rw [if_neg hyz] at h2 ⊢; exact h2
    · subst hyz
      rw [if_neg hxy] at h1; rw [if_pos rfl] at h2; rw [if_neg hxy]; exact h1
    · rw [if_neg hxy] at h1; rw [if_neg hyz] at h2
      have hxz : x ≠ z := by
        intro e; subst e
        exact stepLt_irrefl x (stepLt_trans h1 h2)
      rw [if_neg hxz]; exact stepLt_trans h1 h2

theorem lt_total : ∀ p q : Path K, lt p q ∨ p = q ∨ lt q p
  | [], [] => Or.inr (Or.inl rfl)
  | [], y :: ys => by
    simp only [lt]
    cases h : y.1
    · exact Or.inr (Or.inr rfl)
    · exact Or.inl rfl
  | x :: xs, [] => by
    simp only [lt]
    cases h : x.1
    · exact Or.inl rfl
    · exact Or.inr (Or.inr rfl)
  | x :: xs, y :: ys => by
    simp only [lt]
    by_cases hxy : x = y
    · subst hxy; rw [if_pos rfl, if_pos rfl]
      rcases lt_total xs ys with h | h | h
      · exact Or.inl h
      · exact Or.inr (Or.inl (by rw [h]))
      · exact Or.inr (Or.inr h)
    · rw [if_neg hxy, if_neg (Ne.symm hxy)]
      rcases stepLt_total x y with h | h | h
      · exact Or.inl h
      · exact absurd h hxy
      · exact Or.inr (Or.inr h)

/-! ### 3.4 address independence: a monotone relabelling of keys preserves the order -/

def relabel (f : K → K) : Path K → Path K := List.map (fun s => (s.1, f s.2))

theorem lt_relabel (f : K → K) (hf : ∀ a b, Key.lt a b → Key.lt (f a) (f b))
    (hinj : ∀ a b, f a = f b → a = b) :
    ∀ p q : Path K, lt p q → lt (relabel f p) (relabel f q)
  | [], [], h => (h : False).elim
  | [], y :: ys, h => by simp only [lt, relabel, List.map] at h ⊢; exact h
  | x :: xs, [], h => by simp only [lt, relabel, List.map] at h ⊢; exact h
  | x :: xs, y :: ys, h => by
    simp only [lt, relabel, List.map] at h ⊢
    by_cases hxy : x = y
    · subst hxy; rw [if_pos rfl] at h; rw [if_pos rfl]
      exact lt_relabel f hf hinj xs ys h
    · rw [if_neg hxy] at h
      have hne : (x.1, f x.2) ≠ (y.1, f y.2) := by
        intro e
        apply hxy
        have e1 : x.1 = y.1 := (Prod.mk.inj e).1
        have e2 : f x.2 = f y.2 := (Prod.mk.inj e).2
        obtain ⟨sx, kx⟩ := x; obtain ⟨sy, ky⟩ := y
        simp only at e1 e2
        rw [e1, hinj _ _ e2]
      rw [if_neg hne]
      rcases h with ⟨a, b⟩ | ⟨a, b⟩
      · exact Or.inl ⟨a, b⟩
      · exact Or.inr ⟨a, hf _ _ b⟩

/-! ### 3.2 / 3.7: M is the global order restricted to a world's live atoms -/

/-- a world, for the text type: which atoms exist and which are killed -/
structure World (K : Type) where
  present : Path K → Prop
  killed  : Path K → Prop

def live (W : World K) (p : Path K) : Prop := W.present p ∧ ¬ W.killed p

/-- M(W): the live atoms in traversal order — a relation, not a construction,
    so it is a function of the world by definition and depends on nothing
    else (3.2). Bodies are not mentioned (3.6). -/
def before (W : World K) (p q : Path K) : Prop := live W p ∧ live W q ∧ lt p q

/-- 3.7, sharpened: the order of two atoms is the same in every world where
    both are live. Growing the world never reorders anything. -/
theorem before_stable (W W' : World K) {p q : Path K}
    (hp : live W' p) (hq : live W' q) (h : before W p q) : before W' p q :=
  ⟨hp, hq, h.2.2⟩

/-- monotone at the atom level: present atoms only grow along `⊆` -/
theorem present_mono (W W' : World K) (hW : ∀ p, W.present p → W'.present p)
    (p : Path K) (h : W.present p) : W'.present p := hW p h

end Fugue
