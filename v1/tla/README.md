# tla/ — the dynamic half of FORMAL.md (§9)

`Sync.tla` models gossip sync over a network that reorders across pairs,
duplicates (re-gossip) and loses a bounded number of messages; redaction as
an event that purges locally and on receipt; a serve log written at send
time; and a Byzantine replica minting two events at one sequence number.

One configuration per question, because the joint state space does not fit:

| config | replicas | checks | result |
|---|---|---|---|
| `Converge.cfg` | a, b; MaxSeq 2; 1 drop | `TypeOK`, `PurgeHonoured`, `Converge` (I12) | no error, 10,270 states, 16 s |
| `Equivocation.cfg` | a, b (b Byzantine); MaxSeq 2 | `TypeOK`, `EquivocationCaught` (6.7) | no error, 95,397 states, 71 s |
| `Safety.cfg` | a, b, c (c Byzantine); MaxSeq 1; symmetry on {a, b} | `TypeOK`, `PurgeHonoured`, `ServeLogAccounts` (F2), `Monotone` (I13) | see below |

Run with `tlc -workers auto -config <cfg> Sync.tla` (in `nix develop
.#protocol`). Symmetry is used only for the safety run; it is not sound for
liveness, which is why the runs are split.

History: the first version, with unbounded loss, gave a genuine
counterexample to `Converge` in one minute — no delivery fairness survives
unbounded drops. The network is therefore bounded: one outstanding message
per pair, at-most-once delivery with duplication as re-gossip, `MaxDrops`.
`Sync.cfg` and `Sync_small.cfg` are the joint configurations; they run for
hours and are kept for reference.
