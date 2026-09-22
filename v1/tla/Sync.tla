------------------------------- MODULE Sync -------------------------------
(* v1/FORMAL.md, the dynamic half: gossip sync over a lossy, reordering,     *)
(* duplicating network; redaction as an event with purge-on-receipt; a serve *)
(* log that must account for every body holder (F2, 8.5); and equivocation  *)
(* detection from per-replica sequence numbers (6.7).                         *)
EXTENDS Naturals, FiniteSets

CONSTANTS Replicas, MaxSeq, Byz, MaxDrops   \* Byz ⊆ Replicas may equivocate

Ids == [org: Replicas, seq: 1..MaxSeq, c: {0, 1}]     \* c: content variant (equivocation)
Kinds == {"ins", "redact"}

VARIABLES
  log,      \* log[r]     : set of headers r holds
  body,     \* body[r]    : set of ids whose body r holds
  seq,      \* seq[r]     : next sequence number of r
  net,      \* gossip messages in flight: at most one per (from, to); reorder across pairs
  drops,    \* number of messages lost so far (bounded, so liveness is meaningful)
  served,   \* served[r]  : set of <<id, to>>: r served body of id to `to`
  flagged   \* flagged[r] : set of origins r has caught equivocating

vars == <<log, body, seq, net, drops, served, flagged>>

Header(id, kind, target) == [id |-> id, kind |-> kind, target |-> target]

Init ==
  /\ log = [r \in Replicas |-> {}]
  /\ body = [r \in Replicas |-> {}]
  /\ seq = [r \in Replicas |-> 1]
  /\ net = {}
  /\ drops = 0
  /\ served = [r \in Replicas |-> {}]
  /\ flagged = [r \in Replicas |-> {}]

RedactedAt(r) == {h.target : h \in {g \in log[r] : g.kind = "redact"}}

\* --- local actions -------------------------------------------------------
Append(r) ==
  /\ seq[r] <= MaxSeq
  /\ LET id == [org |-> r, seq |-> seq[r], c |-> 0] IN
     /\ log' = [log EXCEPT ![r] = @ \cup {Header(id, "ins", id)}]
     /\ body' = [body EXCEPT ![r] = @ \cup {id}]
  /\ seq' = [seq EXCEPT ![r] = @ + 1]
  /\ UNCHANGED <<net, drops, served, flagged>>

\* a Byzantine replica mints a second event with the same seq and other content
Equivocate(r) ==
  /\ r \in Byz
  /\ seq[r] > 1
  /\ LET id == [org |-> r, seq |-> seq[r] - 1, c |-> 1] IN
     /\ Header(id, "ins", id) \notin log[r]
     /\ log' = [log EXCEPT ![r] = @ \cup {Header(id, "ins", id)}]
     /\ body' = [body EXCEPT ![r] = @ \cup {id}]
  /\ UNCHANGED <<seq, net, drops, served, flagged>>

Redact(r, h) ==
  /\ h \in log[r] /\ h.kind = "ins"
  /\ h.id \notin RedactedAt(r)
  /\ seq[r] <= MaxSeq
  /\ LET id == [org |-> r, seq |-> seq[r], c |-> 0] IN
     log' = [log EXCEPT ![r] = @ \cup {Header(id, "redact", h.id)}]
  /\ body' = [body EXCEPT ![r] = @ \ {h.id}]      \* purge locally
  /\ seq' = [seq EXCEPT ![r] = @ + 1]
  /\ UNCHANGED <<net, drops, served, flagged>>

\* --- network --------------------------------------------------------------
Gossip(r, to) ==
  /\ r # to
  /\ ~ \E m \in net : m.from = r /\ m.to = to      \* one outstanding message per pair
  /\ LET bodiesToSend == body[r] \ RedactedAt(r) IN
     /\ net' = net \cup {[from |-> r, to |-> to, hs |-> log[r], bs |-> bodiesToSend]}
     /\ served' = [served EXCEPT ![r] = @ \cup {<<id, to>> : id \in bodiesToSend}]
  /\ UNCHANGED <<log, body, seq, drops, flagged>>

Deliver(m) ==
  /\ m \in net
  /\ LET s == m.to
         newLog == log[s] \cup m.hs
         redacted == {h.target : h \in {g \in newLog : g.kind = "redact"}}
         newBody == (body[s] \cup m.bs) \ redacted          \* purge on receipt
         equiv == {h.id.org : h \in {g \in newLog :
                     \E g2 \in newLog : g2.id.org = g.id.org /\ g2.id.seq = g.id.seq /\ g2.id # g.id}}
     IN /\ log' = [log EXCEPT ![s] = newLog]
        /\ body' = [body EXCEPT ![s] = newBody]
        /\ flagged' = [flagged EXCEPT ![s] = @ \cup equiv]
  /\ net' = net \ {m}                                    \* at-most-once; dup = re-gossip
  /\ UNCHANGED <<seq, drops, served>>

Drop(m) ==
  /\ m \in net
  /\ drops < MaxDrops
  /\ net' = net \ {m}
  /\ drops' = drops + 1
  /\ UNCHANGED <<log, body, seq, served, flagged>>

Next ==
  \/ \E r \in Replicas : Append(r) \/ Equivocate(r)
  \/ \E r \in Replicas : \E h \in log[r] : Redact(r, h)
  \/ \E r \in Replicas : \E to \in Replicas : Gossip(r, to)
  \/ \E m \in net : Deliver(m) \/ Drop(m)

Msgs == [from: Replicas, to: Replicas, hs: SUBSET [id: Ids, kind: Kinds, target: Ids], bs: SUBSET Ids]
DeliverPair(r, to) == \E m \in net : m.from = r /\ m.to = to /\ Deliver(m)
Fair == /\ \A r \in Replicas : \A to \in Replicas : WF_vars(Gossip(r, to))
        /\ \A r \in Replicas : \A to \in Replicas : WF_vars(DeliverPair(r, to))
Spec == Init /\ [][Next]_vars /\ Fair

\* --- properties ------------------------------------------------------------
TypeOK ==
  /\ \A r \in Replicas : body[r] \subseteq {h.id : h \in log[r]}
  /\ \A r \in Replicas : seq[r] \in 1..(MaxSeq + 1)

\* I13: logs only grow, whatever the network does
Monotone == [][\A r \in Replicas : log[r] \subseteq log'[r]]_vars

\* 8.4: nobody holds a body whose redaction they have seen
PurgeHonoured == \A r \in Replicas : body[r] \cap RedactedAt(r) = {}

\* F2/8.5: every holder of a body is its origin or a recorded recipient
ServeLogAccounts ==
  \A r \in Replicas : \A id \in body[r] :
    id.org = r \/ \E s \in Replicas : <<id, r>> \in served[s]

\* I12: after appends stop, logs converge (needs fairness on delivery)
Quiescent == \A r \in Replicas : seq[r] = MaxSeq + 1
Converge == <>[](Quiescent => \A r \in Replicas : \A s \in Replicas : log[r] = log[s])

\* 6.7: an equivocation is eventually caught by every honest replica
EquivocationCaught ==
  \A r \in Byz : (\E s \in Replicas : \E h \in log[s] : h.id.org = r /\ h.id.c = 1)
    ~> (\A s \in Replicas \ Byz : r \in flagged[s])
=============================================================================
