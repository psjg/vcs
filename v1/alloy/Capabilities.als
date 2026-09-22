-- v1/FORMAL.md §6 — delegation chains, scope monotonicity, revocation.
-- Bounded relational check: can a valid event exceed its root's scope?
-- Can two logs that agree below e disagree on e's validity?

open util/ordering[Clock] as clk

sig Clock {}
sig Op {}

sig Cert {
  parent : lone Cert,          -- root has no parent
  scope  : set Op,
  expiry  : one Clock
}

-- 6.2 well-formedness, enforced as a fact (integrate refuses otherwise)
fact WellFormed {
  all c : Cert | some c.parent implies {
    c.scope in c.parent.scope
    lte[c.expiry, c.parent.expiry]
  }
  no c : Cert | c in c.^parent            -- chains are acyclic
}

sig Revocation {
  target : one Cert,
  at     : one Clock
}

sig Event {
  cert  : one Cert,
  op    : one Op,
  clock : one Clock
}

-- a log is a set of certs and revocations a peer holds
sig Log {
  certs : set Cert,
  revs  : set Revocation
}

pred chainOk[L : Log, c : Cert] {
  c in L.certs
  all d : c.^parent | d in L.certs
}

pred revokedBefore[L : Log, e : Event, c : Cert] {
  some r : L.revs | r.target = c and lt[r.at, e.clock]
}

-- 6.3
pred valid[L : Log, e : Event] {
  chainOk[L, e.cert]
  all c : e.cert + e.cert.^parent | lt[e.clock, c.expiry]
  e.op in e.cert.scope
  no c : e.cert + e.cert.^parent | revokedBefore[L, e, c]
}

fun root[c : Cert] : Cert { c.*parent - (c.*parent).parent }

-- Theorem: a valid event's op lies in the scope of the root of its chain
-- (scope monotonicity composes along the chain)
assert RootScope {
  all L : Log, e : Event | valid[L, e] implies e.op in root[e.cert].scope
}

-- 6.4: two logs that agree on e's chain and on revocations of it agree on validity
assert Locality {
  all L, L2 : Log, e : Event |
    ((e.cert + e.cert.^parent) & L.certs = (e.cert + e.cert.^parent) & L2.certs) and
    (all r : Revocation | r.target in (e.cert + e.cert.^parent) implies (r in L.revs iff r in L2.revs))
    implies (valid[L, e] iff valid[L2, e])
}

-- What if well-formedness were NOT enforced? Then escalation is possible.
-- (run this to see a counterexample: a child with a wider scope than its parent)
pred escalation {
  some c : Cert | some c.parent and not (c.scope in c.parent.scope)
}

check RootScope for 6
check Locality for 6
run escalation for 4   -- expected: no instance, because WellFormed forbids it
