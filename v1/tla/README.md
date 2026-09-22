# tla/ — the dynamic half of FORMAL.md (§9)

`Sync.tla` models gossip sync, redaction purge, the serve log and
equivocation detection. Check with TLC:

    tlc -workers auto -config Sync_small.cfg Sync.tla   # 3 replicas, MaxSeq 1, one drop
    tlc -workers auto -config Sync.cfg Sync.tla         # MaxSeq 2, two drops; large

Properties: `TypeOK`, `PurgeHonoured`, `ServeLogAccounts` (invariants);
`Monotone`, `Converge`, `EquivocationCaught` (temporal). The first version
of the model, with unbounded loss, produced a genuine counterexample to
`Converge` in one minute: no delivery fairness survives unbounded drops.
The network is therefore bounded (one outstanding message per pair,
at-most-once delivery with duplication as re-gossip, `MaxDrops`).
