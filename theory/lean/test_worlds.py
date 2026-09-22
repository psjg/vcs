"""Hypothesis checks for FORMAL.md §2 over random finite posets."""
from hypothesis import given, settings, strategies as st
import itertools

@st.composite
def posets(draw, n=7):
    # random DAG on 0..n-1 with edges i<j only; le = reflexive-transitive closure
    edges = {(i, j) for i in range(n) for j in range(i+1, n) if draw(st.booleans())}
    le = {(i, i) for i in range(n)} | set(edges)
    changed = True
    while changed:
        changed = False
        for (a, b), (c, d) in itertools.product(list(le), list(le)):
            if b == c and (a, d) not in le:
                le.add((a, d)); changed = True
    return n, frozenset(le)

def down(le, X): return {x for (x, c) in le if c in X}
def up(le, X):   return {x for (c, x) in le if c in X}
def is_ideal(le, S): return down(le, S) <= S
def adopt(le, S, C): return S | down(le, C)
def drop(le, S, C):  return S - up(le, C)
def convex(le, C):   return all(b in C for a in C for c in C for b in range(100) if (a, b) in le and (b, c) in le)
def random_ideal(draw, n, le):
    A = {i for i in range(n) if draw(st.booleans())}
    return down(le, A)

@settings(max_examples=300)
@given(posets(), st.data())
def test_frontier_roundtrip(P, data):
    n, le = P; S = random_ideal(data.draw, n, le)
    mx = {s for s in S if not any((s, t) in le and s != t for t in S)}
    assert down(le, mx) == S

@settings(max_examples=300)
@given(posets(), st.data())
def test_cover_laws(P, data):
    n, le = P; S = random_ideal(data.draw, n, le)
    C = {i for i in range(n) if data.draw(st.booleans())}
    C = {b for b in range(n) if any((a, b) in le and (b, c) in le for a in C for c in C)}  # convex hull
    assert convex(le, C)
    a_lhs = drop(le, adopt(le, S, C), C) == S
    a_rhs = not (C & S) and (down(le, C) - C) <= S
    assert a_lhs == a_rhs
    b_lhs = adopt(le, drop(le, S, C), C) == S
    b_rhs = C <= S and is_ideal(le, S - C)
    assert b_lhs == b_rhs

@settings(max_examples=300)
@given(posets(), st.data())
def test_inverse_pairs_and_paths(P, data):
    n, le = P
    S = random_ideal(data.draw, n, le); T = S | random_ideal(data.draw, n, le)
    D = T - S
    assert adopt(le, S, D) == T and drop(le, T, D) == S
    # two linear extensions of D
    for _ in range(2):
        rem, cur, order = set(D), set(S), []
        while rem:
            mins = [d for d in rem if not any((r, d) in le and r != d for r in rem)]
            e = data.draw(st.sampled_from(sorted(mins))); rem.discard(e); order.append(e)
            cur = adopt(le, cur, {e})
        assert cur == T

if __name__ == "__main__":
    for t in [test_frontier_roundtrip, test_cover_laws, test_inverse_pairs_and_paths]:
        t(); print("ok", t.__name__)
