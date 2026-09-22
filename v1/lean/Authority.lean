/-
  v1/FORMAL.md §6 — validity is locally decidable (6.4), core Lean.
  A certificate chain is a list of events; each link must be present in the
  log, well-formed, and lie below the event being judged.
-/
namespace Authority
variable {Ev : Type}

structure Judge (Ev : Type) where
  le        : Ev → Ev → Prop                 -- the dependency order
  wellFormed: Ev → Ev → Prop                 -- child, parent
  inScope   : Ev → Ev → Prop                 -- event, leaf certificate
  isRoot    : Ev → Prop

/-- a chain `c₀ ⊑ c₁ ⊑ … ⊑ root`, every link present in log `L` -/
def ChainOk (J : Judge Ev) (L : Ev → Prop) : List Ev → Prop
  | [] => False
  | [r] => L r ∧ J.isRoot r
  | c :: p :: rest => L c ∧ J.wellFormed c p ∧ J.le p c ∧ ChainOk J L (p :: rest)

/-- 6.3, without revocations: `e` is valid in `L` via chain `ch` -/
def ValidVia (J : Judge Ev) (L : Ev → Prop) (e : Ev) (ch : List Ev) : Prop :=
  match ch with
  | [] => False
  | c :: _ => J.le c e ∧ J.inScope e c ∧ ChainOk J L ch

def Valid (J : Judge Ev) (L : Ev → Prop) (e : Ev) : Prop := ∃ ch, ValidVia J L e ch

/-- every link of an accepted chain lies below the head (the leaf certificate) -/
theorem chain_below (J : Judge Ev) (htrans : ∀ {a b c}, J.le a b → J.le b c → J.le a c)
    (hrefl : ∀ a, J.le a a) (L : Ev → Prop) :
    ∀ (ch : List Ev) (c : Ev), ChainOk J L (c :: ch) →
      ∀ x, x ∈ c :: ch → J.le x c := by
  intro ch
  induction ch with
  | nil =>
    intro c _ x hx
    have h : x = c := List.mem_singleton.mp hx
    rw [h]; exact hrefl c
  | cons p rest ih =>
    intro c hok x hx
    obtain ⟨_, _, hpc, hrest⟩ := hok
    cases List.mem_cons.mp hx with
    | inl h => rw [h]; exact hrefl c
    | inr h => exact htrans (ih p hrest x h) hpc

/-- 6.4: two logs that agree below `e` agree on the validity of `e`. -/
theorem valid_local (J : Judge Ev) (htrans : ∀ {a b c}, J.le a b → J.le b c → J.le a c)
    (hrefl : ∀ a, J.le a a) (L L' : Ev → Prop) (e : Ev)
    (hagree : ∀ x, J.le x e → (L x ↔ L' x)) :
    Valid J L e → Valid J L' e := by
  intro ⟨ch, hv⟩
  refine ⟨ch, ?_⟩
  match ch, hv with
  | c :: rest, ⟨hce, hsc, hok⟩ =>
    refine ⟨hce, hsc, ?_⟩
    have below : ∀ x, x ∈ c :: rest → J.le x e :=
      fun x hx => htrans (chain_below J htrans hrefl L rest c hok x hx) hce
    -- transport ChainOk across logs that agree on everything below e
    have : ∀ (l : List Ev), (∀ x, x ∈ l → J.le x e) → ChainOk J L l → ChainOk J L' l := by
      intro l
      induction l with
      | nil => intro _ h; exact h
      | cons a t ih =>
        intro hb hok
        match t, hok with
        | [], ⟨hLa, hr⟩ => exact ⟨(hagree a (hb a (by simp))).1 hLa, hr⟩
        | p :: r, ⟨hLa, hwf, hle, hrest⟩ =>
          exact ⟨(hagree a (hb a (by simp))).1 hLa, hwf, hle,
                 ih (fun x hx => hb x (List.mem_cons_of_mem a hx)) hrest⟩
    exact this (c :: rest) below hok

end Authority

/-! ### 6.3/6.4 with revocations -/
namespace Authority
variable {Ev : Type}

/-- a revocation targets a certificate at some clock; it is present or not in a log -/
structure Revoke (Ev : Type) where
  rev    : Ev → Ev → Prop      -- `rev v c`: event v revokes certificate c
  before : Ev → Ev → Prop      -- `before v e`: v's clock precedes e's

/-- a chain link is revoked for e if the log holds a revocation of it before e -/
def LinkRevoked (Rv : Revoke Ev) (L : Ev → Prop) (e c : Ev) : Prop :=
  ∃ v, L v ∧ Rv.rev v c ∧ Rv.before v e

def ValidViaR (J : Judge Ev) (Rv : Revoke Ev) (L : Ev → Prop) (e : Ev) (ch : List Ev) : Prop :=
  ValidVia J L e ch ∧ ∀ c, c ∈ ch → ¬ LinkRevoked Rv L e c

def ValidR (J : Judge Ev) (Rv : Revoke Ev) (L : Ev → Prop) (e : Ev) : Prop :=
  ∃ ch, ValidViaR J Rv L e ch

/-- 6.4 with revocations: validity depends on `↓e` and on the revocations
    that target links of the chain before `e`; two logs agreeing on both
    agree on validity. -/
theorem validR_local (J : Judge Ev) (Rv : Revoke Ev)
    (htrans : ∀ {a b c}, J.le a b → J.le b c → J.le a c) (hrefl : ∀ a, J.le a a)
    (L L' : Ev → Prop) (e : Ev)
    (hagree : ∀ x, J.le x e → (L x ↔ L' x))
    (hrev : ∀ v c, J.le c e → Rv.rev v c → Rv.before v e → (L v ↔ L' v)) :
    ValidR J Rv L e → ValidR J Rv L' e := by
  intro ⟨ch, hv, hnr⟩
  have hv' : Valid J L' e := valid_local J htrans hrefl L L' e hagree ⟨ch, hv⟩
  -- valid_local reuses the same chain: recover it
  refine ⟨ch, ?_, ?_⟩
  · -- ValidVia transports along the same chain (same argument as valid_local)
    match ch, hv with
    | c :: rest, ⟨hce, hsc, hok⟩ =>
      refine ⟨hce, hsc, ?_⟩
      have below : ∀ x, x ∈ c :: rest → J.le x e :=
        fun x hx => htrans (chain_below J htrans hrefl L rest c hok x hx) hce
      have : ∀ (l : List Ev), (∀ x, x ∈ l → J.le x e) → ChainOk J L l → ChainOk J L' l := by
        intro l
        induction l with
        | nil => intro _ h; exact h
        | cons a t ih =>
          intro hb hok
          match t, hok with
          | [], ⟨hLa, hr⟩ => exact ⟨(hagree a (hb a (by simp))).1 hLa, hr⟩
          | p :: r, ⟨hLa, hwf, hle, hrest⟩ =>
            exact ⟨(hagree a (hb a (by simp))).1 hLa, hwf, hle,
                   ih (fun x hx => hb x (List.mem_cons_of_mem a hx)) hrest⟩
      exact this (c :: rest) below hok
  · intro c hc ⟨v, hLv, hrv, hbv⟩
    match ch, hv with
    | c0 :: rest, ⟨hce, _, hok⟩ =>
      have hcle : J.le c e := htrans (chain_below J htrans hrefl L rest c0 hok c hc) hce
      exact hnr c hc ⟨v, (hrev v c hcle hrv hbv).2 hLv, hrv, hbv⟩

end Authority
