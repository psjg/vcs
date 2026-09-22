-- v1/FORMAL.md 7.6 — contracting a box of tasks. When does the coarse plan lie?
-- ready(D) = minimal elements of T \ D (leaves only, criteria ignored here).
-- Contract box B (containing its representative b) to b and compare.

sig Task {
  blockedBy : set Task
}
one sig B in Task {}                 -- the box's representative
sig Part in Task {}                  -- the box: representative plus parts
fact { B in Part }
fact Acyclic { no t : Task | t in t.^blockedBy }

-- the done-set: down-closed, box atomic
sig Done in Task {}
fact DoneIdeal { Done.blockedBy in Done }
fact BoxAtomic { Part in Done or no (Part & Done) }

fun q[t : Task] : Task { t in Part implies B else t }
fun qle : Task -> Task {              -- quotient relation: x' ≤ y' if some x ≤ y
  { u, v : Task | some x, y : Task | q[x] = u and q[y] = v and x in y.^blockedBy }
}

-- fine ready set and its image; coarse ready set computed on the quotient
fun readyFine : set Task { { t : Task - Done | no (t.blockedBy - Done) } }
fun imageReady : set Task { { u : Task | some t : readyFine | q[t] = u } }
fun qTasks : set Task { { u : Task | some t : Task | q[t] = u } }
fun qDone  : set Task { { u : Task | some t : Done | q[t] = u } }
fun readyCoarse : set Task {
  { u : qTasks - qDone | no v : qTasks - qDone | v->u in qle and v != u }
}

-- the boundary condition of 7.6: no blockedBy edge crosses the box boundary
pred noCrossEdges {
  all x : Part, y : Task - Part |
    (y in x.blockedBy implies all x2 : Part | y in x2.blockedBy) and
    (x in y.blockedBy implies all x2 : Part | x2 in y.blockedBy)
}

-- 7.6 as checked: with the boundary condition, contraction commutes with ready
assert Coherent {
  noCrossEdges implies imageReady = readyCoarse
}
check Coherent for 6

-- Outside the box the condition is not even needed (box atomicity suffices):
assert CoherentOutside {
  (imageReady - B) = (readyCoarse - B)
}
check CoherentOutside for 6

-- Inside it is: without the boundary condition, "some part is ready" and
-- "the box is ready" come apart. Ask Alloy for the counterexample.
pred incoherent { not noCrossEdges and imageReady != readyCoarse }
run incoherent for 4
