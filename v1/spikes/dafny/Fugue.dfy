// Spike: the weave's ordering, as executable code with its laws attached.
// `Lt` is a compiled function — the one the weave would call — and Dafny
// proves it is a strict total order on paths (FORMAL 3.3 / lean/Fugue.lean).

module Fugue {
  datatype Side = L | R
  datatype Step = Step(side: Side, key: int)

  predicate StepLt(x: Step, y: Step)
  {
    (x.side == L && y.side == R) || (x.side == y.side && x.key < y.key)
  }

  // the in-order traversal as a relation on paths
  predicate Lt(p: seq<Step>, q: seq<Step>)
    decreases |p|
  {
    if |p| == 0 && |q| == 0 then false
    else if |p| == 0 then q[0].side == R
    else if |q| == 0 then p[0].side == L
    else if p[0] == q[0] then Lt(p[1..], q[1..])
    else StepLt(p[0], q[0])
  }

  lemma Irrefl(p: seq<Step>)
    ensures !Lt(p, p)
    decreases |p|
  {
    if |p| > 0 { Irrefl(p[1..]); }
  }

  lemma Trans(p: seq<Step>, q: seq<Step>, r: seq<Step>)
    requires Lt(p, q) && Lt(q, r)
    ensures Lt(p, r)
    decreases |p| + |q| + |r|
  {
    if |p| > 0 && |q| > 0 && |r| > 0 && p[0] == q[0] && q[0] == r[0] {
      Trans(p[1..], q[1..], r[1..]);
    }
  }

  lemma Total(p: seq<Step>, q: seq<Step>)
    ensures Lt(p, q) || p == q || Lt(q, p)
    decreases |p|
  {
    if |p| > 0 && |q| > 0 && p[0] == q[0] {
      Total(p[1..], q[1..]);
      if p[1..] == q[1..] { assert p == [p[0]] + p[1..] == [q[0]] + q[1..] == q; }
    }
  }

  // and the piece of implementation that uses it: sorted insertion into the
  // materialised sequence, with sortedness as the maintained invariant
  predicate Sorted(xs: seq<seq<Step>>)
  {
    forall i, j :: 0 <= i < j < |xs| ==> Lt(xs[i], xs[j])
  }

  function Insert(xs: seq<seq<Step>>, p: seq<Step>): seq<seq<Step>>
    decreases |xs|
  {
    if |xs| == 0 then [p]
    else if Lt(p, xs[0]) then [p] + xs
    else [xs[0]] + Insert(xs[1..], p)
  }

  lemma InsertSorted(xs: seq<seq<Step>>, p: seq<Step>)
    requires Sorted(xs)
    requires forall i :: 0 <= i < |xs| ==> xs[i] != p
    ensures Sorted(Insert(xs, p))
    decreases |xs|
  {
    if |xs| == 0 { }
    else if Lt(p, xs[0]) {
      forall j | 1 <= j < |xs| ensures Lt(p, xs[j]) { Trans(p, xs[0], xs[j]); }
    } else {
      Total(p, xs[0]);
      assert Lt(xs[0], p);
      InsertSorted(xs[1..], p);
      var ys := Insert(xs[1..], p);
      forall j | 0 <= j < |ys| ensures Lt(xs[0], ys[j]) {
        InsertMembers(xs[1..], p, ys[j]);
        if ys[j] == p { } else { var k :| 0 <= k < |xs[1..]| && xs[1..][k] == ys[j]; }
      }
    }
  }

  lemma InsertMembers(xs: seq<seq<Step>>, p: seq<Step>, y: seq<Step>)
    requires y in Insert(xs, p)
    ensures y == p || y in xs
    decreases |xs|
  {
    if |xs| == 0 { }
    else if Lt(p, xs[0]) { }
    else { if y != xs[0] { InsertMembers(xs[1..], p, y); } }
  }
}
