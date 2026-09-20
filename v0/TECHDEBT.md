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
