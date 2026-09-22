/-
  v1/FORMAL.md 3.3 with moves, and I10.
  An atom's placement in a world is its base path unless the world holds
  moves of it, in which case the move with the highest key wins. The winner
  is a function of the *set* of moves present, so placement is a set-function
  of the world, and each atom has exactly one placement (I10).
-/
namespace Moves

class Key (K : Type) where
  lt : K → K → Prop
  irrefl : ∀ a, ¬ lt a a
  trans : ∀ {a b c}, lt a b → lt b c → lt a c
  total : ∀ a b, lt a b ∨ a = b ∨ lt b a

variable {K P : Type} [Key K]

/-- a move: a key and a destination path -/
structure Move (K P : Type) where
  key : K
  dest : P

def keyLe (m m' : Move K P) : Prop := Key.lt m'.key m.key ∨ m'.key = m.key

/-- `Winner l m`: m is in l and no move in l has a higher key -/
def Winner (l : List (Move K P)) (m : Move K P) : Prop :=
  m ∈ l ∧ ∀ m' ∈ l, keyLe m m'

/-- the winner exists for a nonempty list -/
theorem winner_exists : ∀ (l : List (Move K P)), l ≠ [] → ∃ m, Winner l m
  | [], h => absurd rfl h
  | [m], _ => ⟨m, by simp, fun m' hm' => by
      have : m' = m := List.mem_singleton.mp hm'
      rw [this]; exact Or.inr rfl⟩
  | m :: n :: rest, _ => by
    obtain ⟨w, hw⟩ := winner_exists (n :: rest) (by simp)
    rcases Key.total w.key m.key with h | h | h
    · -- m beats the winner of the rest
      refine ⟨m, by simp, ?_⟩
      intro m' hm'
      rcases List.mem_cons.mp hm' with e | e
      · rw [e]; exact Or.inr rfl
      · rcases hw.2 m' e with h' | h'
        · exact Or.inl (Key.trans h' h)
        · exact Or.inl (h' ▸ h)
    · refine ⟨m, by simp, ?_⟩
      intro m' hm'
      rcases List.mem_cons.mp hm' with e | e
      · rw [e]; exact Or.inr rfl
      · rcases hw.2 m' e with h' | h'
        · exact Or.inl (h ▸ h')
        · exact Or.inr (h' ▸ h)
    · refine ⟨w, List.mem_cons_of_mem m hw.1, ?_⟩
      intro m' hm'
      rcases List.mem_cons.mp hm' with e | e
      · rw [e]; exact Or.inl h
      · exact hw.2 m' e

/-- with distinct keys the winner's key is unique, hence the winner is
    determined up to key -/
theorem winner_key_unique {l : List (Move K P)} {m m' : Move K P}
    (h : Winner l m) (h' : Winner l m') : m.key = m'.key := by
  rcases h.2 m' h'.1 with a | a <;> rcases h'.2 m h.1 with b | b
  · exact absurd (Key.trans a b) (Key.irrefl _)
  · exact b
  · exact a.symm
  · exact a.symm

/-- the winner depends only on membership: two lists with the same elements
    have the same winners (the set-function property, 3.2, for moves) -/
theorem winner_perm {l l' : List (Move K P)} (hmem : ∀ m, m ∈ l ↔ m ∈ l')
    {m : Move K P} (h : Winner l m) : Winner l' m :=
  ⟨(hmem m).1 h.1, fun m' hm' => h.2 m' ((hmem m').2 hm')⟩

/-- placement of one atom in a world: base path, or the winning move's destination -/
def Placed (base : P) (moves : List (Move K P)) (p : P) : Prop :=
  (moves = [] ∧ p = base) ∨ (∃ m, Winner moves m ∧ p = m.dest)

/-- I10, existence: every atom is placed somewhere -/
theorem placed_exists (base : P) (moves : List (Move K P)) : ∃ p, Placed base moves p := by
  by_cases h : moves = []
  · exact ⟨base, Or.inl ⟨h, rfl⟩⟩
  · obtain ⟨m, hm⟩ := winner_exists moves h
    exact ⟨m.dest, Or.inr ⟨m, hm, rfl⟩⟩

/-- I10, uniqueness, given that keys identify moves (content addressing:
    equal keys means equal events, hence equal destinations) -/
theorem placed_unique (base : P) (moves : List (Move K P))
    (hkey : ∀ m ∈ moves, ∀ m' ∈ moves, m.key = m'.key → m.dest = m'.dest)
    {p q : P} (hp : Placed base moves p) (hq : Placed base moves q) : p = q := by
  rcases hp with ⟨he, hp⟩ | ⟨m, hm, hp⟩ <;> rcases hq with ⟨he', hq⟩ | ⟨m', hm', hq⟩
  · rw [hp, hq]
  · rw [he] at hm'; exact absurd hm'.1 (by simp)
  · rw [he'] at hm; exact absurd hm.1 (by simp)
  · rw [hp, hq]; exact hkey m hm.1 m' hm'.1 (winner_key_unique hm hm')

end Moves
