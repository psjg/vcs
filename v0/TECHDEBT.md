# Tech debt

Shortcuts taken deliberately. Repay or record why not.

- **Capture is diff-at-record**, so the op log is a *reconstruction* of what
  happened, not a recording. Accepted for v0 (PRD non-goal); it is the exact
  weakness an event-sourced capture front-end removes.
- **Line granularity**, not characters or AST nodes. Cheap, matches the VCS use
  case, and keeps the weave small enough to reason about.
- **No persistence format stability.** The on-disk log is serde JSON; it will
  change without migration.
- **Single file per repo is not assumed, but paths are opaque strings** — no
  rename tracking, no directory semantics.
- **No antichain minimisation of `deps`.** A change stores every dependency its
  ops reference, including ones implied transitively by another dependency.
  Deliberate: `closure(S)` is identical either way, so this is normalisation
  (smaller sets, canonical ids) rather than semantics. v0 instead *reports* how
  much minimisation would shrink the average dep set — data first, then decide.
- **Move is only as good as the capture.** A diff-based adapter cannot observe
  `MoveLine`; it sees a delete and an insert. v0 therefore under-reports moves,
  which is the sharpest illustration of what coarse capture costs and should be
  measured, not hidden.
