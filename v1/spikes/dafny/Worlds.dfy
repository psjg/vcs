// Spike: "implementation = model" in Dafny.
// FORMAL.md §2 with the order left abstract (a preorder given by axioms, as
// the Lean `Poset` class does) and the executable set operations verified
// against the laws by the SMT solver.

module Worlds {
  type Ev = int

  ghost predicate Le(a: Ev, b: Ev)          // the dependency order: abstract
  lemma {:axiom} LeRefl(a: Ev) ensures Le(a, a)
  lemma {:axiom} LeTrans(a: Ev, b: Ev, c: Ev) requires Le(a, b) && Le(b, c) ensures Le(a, c)

  ghost predicate IsIdeal(S: set<Ev>) { forall e, r :: e in S && Le(r, e) ==> r in S }
  ghost predicate Convex(C: set<Ev>) { forall a, b, c :: a in C && c in C && Le(a, b) && Le(b, c) ==> b in C }

  // U is the finite universe of known events; closures are taken inside it
  ghost function Down(C: set<Ev>, U: set<Ev>): set<Ev> { set x | x in U && exists c :: c in C && Le(x, c) }
  ghost function Up(C: set<Ev>, U: set<Ev>): set<Ev>   { set x | x in U && exists c :: c in C && Le(c, x) }

  // 2.6 — the operations as the implementation writes them
  ghost function Adopt(S: set<Ev>, C: set<Ev>, U: set<Ev>): set<Ev> { S + Down(C, U) }
  ghost function Drop(S: set<Ev>, C: set<Ev>, U: set<Ev>): set<Ev>  { S - Up(C, U) }

  lemma AdoptIdeal(S: set<Ev>, C: set<Ev>, U: set<Ev>)
    requires IsIdeal(S) && IsIdeal(U)
    ensures IsIdeal(Adopt(S, C, U))
  {
    forall e, r | e in Adopt(S, C, U) && Le(r, e) ensures r in Adopt(S, C, U) {
      if e !in S { var c :| c in C && Le(e, c); LeTrans(r, e, c); }
    }
  }

  lemma DropIdeal(S: set<Ev>, C: set<Ev>, U: set<Ev>)
    requires IsIdeal(S) && S <= U
    ensures IsIdeal(Drop(S, C, U))
  {
    forall e, r | e in Drop(S, C, U) && Le(r, e) ensures r in Drop(S, C, U) {
      assert r in S;
      if r in Up(C, U) { var c :| c in C && Le(c, r); LeTrans(c, r, e); assert e in Up(C, U); assert false; }
    }
  }

  // 2.8: the convex difference is the exact inverse
  lemma DropDiff(S: set<Ev>, T: set<Ev>, U: set<Ev>)
    requires IsIdeal(S) && S <= T && T <= U
    ensures Drop(T, T - S, U) == S
  {
    forall x | x in Drop(T, T - S, U) ensures x in S {
      if x !in S { LeRefl(x); assert x in Up(T - S, U); }
    }
    forall x | x in S ensures x in Drop(T, T - S, U) {
      if x in Up(T - S, U) { var d :| d in T - S && Le(d, x); assert d in S; }
    }
  }

  lemma AdoptDiff(S: set<Ev>, T: set<Ev>, U: set<Ev>)
    requires IsIdeal(T) && S <= T && T <= U
    ensures Adopt(S, T - S, U) == T
  {
    forall x | x in T ensures x in Adopt(S, T - S, U) {
      if x !in S { LeRefl(x); assert x in Down(T - S, U); }
    }
    forall x | x in Adopt(S, T - S, U) ensures x in T {
      if x !in S { var d :| d in T - S && Le(x, d); }
    }
  }

  // 2.7(b), the direction the implementation needs before an `unrecord`:
  // if C ⊆ S and S \ C is an ideal then adopt(drop(S, C), C) = S
  lemma CoverB(S: set<Ev>, C: set<Ev>, U: set<Ev>)
    requires IsIdeal(S) && C <= S && S <= U && IsIdeal(S - C)
    ensures Adopt(Drop(S, C, U), C, U) == S
  {
    forall x | x in S ensures x in Adopt(Drop(S, C, U), C, U) {
      if x in Up(C, U) {
        var c :| c in C && Le(c, x);
        if x !in C { assert c in S - C; assert false; }
        LeRefl(x);
      }
    }
    forall x | x in Adopt(Drop(S, C, U), C, U) ensures x in S {
      if x !in Drop(S, C, U) { var c :| c in C && Le(x, c); }
    }
  }
}
