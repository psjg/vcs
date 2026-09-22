# v1 — vision

Turn 1 of three. `INVENTORY.md` is what was said; this is what it adds up
to. It is written to be disagreed with. A later pass formalises whatever
survives.

## The claim in one sentence

Everything a version control system, a forge, a planning tool and a
governance process do is navigation in the lattice of ideals of one poset —
the poset of signed, content-addressed events ordered by what they reference —
and the rest is representation, transport and law.

## 1. The object

**An event** is a header and, optionally, a body. The header holds the
address `(replica, hlc)`, the author key, the op kind, its shape (ranges,
sides, node refs), the references to other events, and the hash of the body.
The body is whatever a human typed. `EventId = blake3(header)`. Bodies are
addressed by their own hash and stored apart.

**One order.** An op references every event it observes and every event it
overwrites: a delete references the insert, a move references the previous
placement, a mode-set references the previous mode-set, a task split
references the task. That reference relation, transitively closed, is the
partial order P. There is no second, causal order. Two events are
*concurrent* iff they are incomparable in P. The hybrid logical clock is a
tie-break and an approximate date, never a semantic input.

**A world** is an ideal of P: a downward-closed set of events. `M(S)`
materialises a world deterministically, reading only the set. Worlds form a
distributive lattice: join is merge, meet is common base, an ideal is
determined by its frontier (the antichain of its maximal elements), and
between any two worlds there is at most one path — so every diagram commutes
and there is nothing to prove about commutation. Moving between worlds is one
operation, `goto(S, T)`, which touches only `T \ S` and `S \ T`.

**A change** is a label: a name applied, possibly long after the fact, to a
set of events. Its dependencies are derived from the events' references, not
declared. `record` splits pending events into the connected components of
the reference graph and offers one change per component. Changes are what
humans adopt, endorse and drop; events are what the lattice is made of.

**A conflict** is a derived object: two incomparable events that overwrite
the same thing, computed on demand, with a stable identity, open until a
change that depends on all sides declares it resolved. Which situations count
as conflicts is a predicate on incomparable pairs; the predicate is a design
choice per document type, not a property of the lattice.

**Addresses.** Every character, line, node and task has an identity: for text,
`Pos { event, offset }`. Coordinates (`file:line:col`) are converted to
identities at capture and never stored. A reference to "these lines" is
therefore a reference to atoms, and it survives edits, moves, renames and
deletion. References come in two strengths: **deps** count for closure (a
delete depends on the insert it kills); **links** count for the reverse
index but not for closure (an issue citing two hundred lines does not adopt
them). A **citation** is a link to a span of atoms together with the frontier
at which it was made, so `M` can show what the span said then, what it says
now, and the difference — the staleness of the comment. The reverse
direction, "someone said something about this", is the same index that
upward closure already uses, filtered. Quoting is transclusion: the citing
body contains the reference, not a copy, and `M` renders the atoms in place.
Atoms that are dead or absent from the ideal still resolve, to a hole with
provenance — the same rendering path redaction needs.

That is the whole model. What follows lists consequences.

## 2. What falls out

Each of these is a lattice operation, not a feature:

- **merge** — join of two ideals.
- **cherry-pick / adopt** — join with the closure of one change; cost bounded
  by that closure, not by the age of the repository.
- **revert** — a compensating change (ex nunc). **unrecord / drop** —
  removal of an upward closure (ex tunc). They are different operations and
  the model keeps them different.
- **rebase** — nothing; a change is the same events in another world.
  Stable change identity across rebases is structural, not a tool.
- **fork** — another ideal over the same poset. You fork a decision, not a
  project; the cost of disagreeing with X is |↑X|, computable before you
  decide; everything independent of X flows to both sides automatically.
- **checkpoint** — a frontier plus `hash(M(frontier))`: the normal form of a
  world, with a proof anyone can recompute.
- **partial clone** — you hold the header stream and fetch bodies lazily;
  you never need more than the closure of what you touch.
- **per-hunk review** — accepting a sub-ideal. The events keep their author;
  the acceptance keeps its reviewer; nothing is rewritten.
- **squash / unsquash** — coarser or finer labels over the same events.
  History is never destroyed; the "ideal history" is a labelling, stored
  beside other labellings.
- **undo** — a path down the lattice at event granularity, in a multi-writer
  setting, by subtraction.
- **mirror** — a second materialiser rendering an ideal as a git history,
  deterministic and append-only. Forges are projections; the poset is the
  truth; every change has exactly one home.
- **two-way links** — an issue, review comment or document cites atoms; the
  code shows what was said about it; both are one index. Comments never
  drift off their lines because there are no lines, only atoms.
- **literate and living documents** — a document transcludes a span of code
  and always shows it current, or as of a chosen frontier.
- **redaction** — a `Redact` event referencing the target, with a reason.
  Bodies disappear; headers, ids and every closure stay byte-identical. What
  remains is a signed hole. History is not falsified; it is not distributed.

## 3. Document types

The op signature is open. Each document type brings its ops, its `M`, and its
conflict predicate; the lattice, the labelling and the governance above are
shared:

- **text** — Fugue over character runs (v0's weave).
- **tree** — create / move / remove / restore with cycle-safe moves.
- **blob** — a multi-value register over content-defined chunks; every
  concurrent write is a conflict; locks are a social object, not a mechanism.
- **task graph** — tasks with two edges, `blockedBy` and `partOf`, and an
  acceptance criterion. Done-sets are ideals under `blockedBy`; `ready` is the
  set of covers restricted to leaves with a criterion; width is the antichain;
  Dilworth gives the number of parallel lanes. A task with parts is itself a
  task graph (a fixed-point type; HTN, operadic substitution). The 100% rule
  is checked on close: the parent's criterion must hold once its parts are
  fulfilled. A task is fulfilled by a change; planned dependencies are later
  compared with the real ones the code produced.
- **collaboration objects** — issues, reviews, endorsements, attestations,
  bounties, post-mortems: ops referencing the events they are about, so
  adopt and drop apply to them too.

Two document types in one log means two materialisers in one `M`. That is
the one place the "one relation" principle gets an exception, and it is
designed in from the start rather than bolted on.

## 4. Identity, trust, secrecy

- **Identity is a key.** No accounts, no registry, no global names; petnames
  locally. A delegation chain runs identity → device → session, each link a
  signed certificate in the log with scope and expiry, scope monotone
  downward. A session certificate for an agent carries harness, model,
  effort, and hashes of its configuration and task. The harness holds the
  key; the model does not.
- **Integrity is a hash chain per replica.** Signing two different events at
  the same position is provable by anyone who sees both. No consensus is
  needed because no total order is needed.
- **Attestation, not per-keystroke signing.** An attestation is a signature
  over a Merkle root of event ids, with a declared scope ("build logic, not
  crypto"). Recording a change attests it; review is an attestation by a
  human; a capability may require that agent-written events be covered by a
  human attestation before a label like `main` applies.
- **Rooms.** Bodies are encrypted always; headers are encrypted for
  non-members. Relays and seeders hold ciphertext addressed by hash and can
  verify nothing else, so the network grows without trust growing with it.
- **Capabilities, not ACLs.** Who may endorse, label, resolve, redact or
  disclose is a signed claim in the log, checked locally by every peer.
- **Time.** Hybrid logical clocks in the header: an approximate, bounded,
  signed date. Verified time is an external attestation.
- **Minimisation.** Serving policy is a function of trust level to (serve,
  log, retain, who reads). The default level logs nothing persistently. Any
  escalation is a signed, scoped, expiring op. Logs are events; the log logs
  its own reads; it closes over transfers and escapes over local reads.

## 5. Governance stance

- **There is no `main`.** There are signed endorsements of frontiers. A
  recipe is a frontier; a source is `M(recipe)`; a binary is a reproducible
  build of it; trust in a binary reduces to trust in keys plus recomputation.
  A distribution is somebody's endorsements plus recipes.
- **Exit is partial.** Because a fork is one decision, voice and exit become
  the same act: recording a change. Cheap partial exit is what keeps
  stewards honest (Hirschman). Governance of the powers themselves is data in
  the same lattice (Hohfeld's powers; Nomic's self-amendment).
- **The operations are old.** Void, annulment, termination, conversion,
  ratification, prescription — the obligations-law vocabulary maps one-to-one
  onto drop, revert, relabel, attest and expiry, with retroactivity, standing
  and time bars as parameters. Statutory rules are computed; judgement is
  attested. The system enforces nothing; it makes acts legible, verifiable
  and reputation-bearing, and leaves enforcement to communities and courts
  (lex mercatoria, Ostrom).
- **Labour is legible.** Review, triage, carrying a patchset, resolving a
  conflict: all are signed events. Work that could not be counted can now be
  paid for, whatever the payment rail. A bounty is a signed promise with a
  human acceptance as its oracle.
- **Just culture.** Attestations prove a judgement within a scope; they do
  not prove who held the key unless a documented procedure discloses it, and
  they never prove the judgement was right. Post-mortems are changes.
  Responses are graduated capability changes, all reversible.

## 6. What is deliberately not promised

- A copy already made cannot be recalled. Provenance travels with it;
  possession is not provable in either direction.
- Members see metadata. Traffic analysis by outsiders is not addressed.
- The model knows nothing of what a session certificate claims beyond that
  the device signed the claim.
- Judgement stays human. Tests filter; attestations decide; conflicts of
  taste are not computed.
- Un-forkability of institutions — review capacity, hardware access, trust in
  people — is not touched. The cost of carrying disagreement becomes constant
  instead of growing; that is all.
- Effects are not reversible. They live in the lattice as events without
  groupoid arrows; compensation is a new event, not an undo.

## 7. Why this is one thing

Every item above is an instance of five words: ideal, cover, join, meet,
attestation. Merge is join. Base is meet. Ready is covers. A fork, a
recipe, a checkpoint and a review are ideals or frontiers. Endorsement,
acceptance, ratification and escalation are attestations. What makes the
whole coherent is not that the parts are similar but that they are the same
construction applied to different document types, and that the construction
is thin: there is never more than one way to get from one world to another,
so no part of the system has to argue with another part about order.

The price of that thinness is paid once, at capture: an op must say what it
observes and overwrites. Live capture can pay it; diff capture can only
guess. That is why v1 starts at the front end.

## 8. What v1 builds, in order

1. **Hashes.** Header/body split, `EventId = blake3(header)`, blob store,
   `Redact`, the two pinning tests (redaction leaves ids intact; semantics are
   address-independent).
2. **One relation.** Ops reference what they overwrite; `parents` and LWW
   removed; the conflict predicate as an explicit, per-type function; refs
   split into deps and links; `Cite { spans, at }` as the first link-only op,
   with the reverse index and the then/now/diff rendering.
3. **Worlds.** `goto`, join, meet, frontier as the stored form, checkpoints as
   objects, incremental `M` from a checkpoint; property tests over random
   posets for the cover laws and path independence.
4. **Attestation and delegation.** Keys, session certificates, attestations
   over Merkle roots, capabilities, hash chains per replica.
5. **Task graph as a document type.** Two edges, criteria, `ready`, the 100%
   check, planned-vs-actual dependency report.
6. **Transport and mirror.** Noise links, state-vector or Merkle
   reconciliation, dumb relays, the deterministic git projection.
7. **Live capture** via an LSP server owning the weave as buffer.

Research, not build: typed worlds (tests as types), locality as sheaves,
homotopical conflict structure, the runtime as a world.

## 9. The name

v0 was a spike whose name was a version number. v1 keeps the joke because it
is also the plan: the object above is small enough that a rewrite is cheaper
than a migration, and the next rewrite will be v2.
