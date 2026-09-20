# Findings

Measurements, not opinions. Reproduce with
`cargo run --release --bin import-git -- <repo> [--max N]`.

## The question

Does deriving dependencies from semantic references beat taking an event's
recorded causal parents? And does the answer survive a real history?

## Two imported histories, 2026-09-20

`psjg/fpl` — 76 commits, 13 307 events, 54 files. A young, tidy repo.
`the LLM wiki` — 211 commits, 26 522 events, 128 files. Agent-written commits
that routinely touch a page plus the index plus the log: deliberately messy.

| | fpl | wiki |
|---|---|---|
| reduction `causal / change-level`, median | **7.9×** | **4.7×** |
| reduction, mean | 20.1× | 110.0× |
| granularity cost `change / event-level`, median | **2.25×** | **29.96×** |
| granularity cost, max | 133× | 3372× |
| dependencies per change, mean | 1.51 | 1.55 |
| changes depending on nothing | 26% | 7% |

## The result that changed the design

Average events pulled in when adopting one change, early history versus late:

| | fpl first ⅓ | fpl last ⅓ | wiki first ⅓ | wiki last ⅓ |
|---|---|---|---|---|
| causal closure | 1 741 | 10 619 | 6 829 | 22 133 |
| change-level closure | 222 | 1 663 | 1 434 | 4 335 |
| **event-level closure** | 142 | **228** | 163 | **119** |

**The causal closure grows with history. So does the change-level closure —
at the same rate. The event-level closure is flat, and in the messy repo it
shrinks.**

The constant factor was never the interesting claim; *boundedness* was. A
cherry-pick whose cost grows with the age of the repository is the thing git
already has. Event-level dependency tracking is the only variant here that does
not have it.

## Why change granularity fails: contamination

A change is adopted whole, so one commit touching five unrelated files welds
their histories together. Every later change to file A then transitively
depends on everything those commits did to B, C, D and E. The effect compounds:
each multi-file commit is another weld.

That is exactly what the granularity column measures, and why the tidy repo
pays 2.25× while the agent-written wiki — whose commits habitually touch a page,
the index and the log together — pays 29.96× at the median and 3372× at worst.

Corollary, uncomfortable and worth stating: **a repository's commit hygiene
becomes a performance characteristic of its version control.** Squashing
unrelated work into one commit is not only bad review practice, it permanently
degrades every future cherry-pick.

## What it costs to fix

Tracking dependencies per event means adopting a change may pull only *part* of
another change — a fragment of somebody's commit. The document stays coherent
(the fragment is dependency-closed), but "I took commit X" stops being true.
Pijul keeps changes whole and pays the contamination. The measurement says the
bill is large. See ADR-0006, which is *proposed*, not accepted: it is a taste
call about what a change means, not a technical one.

## Caveats

- The import is linear (`--first-parent`), so the causal closure is by
  construction the whole prefix. That is faithful to how git adoption works,
  but it is not a comparison against a cleverly pruned causal baseline.
- Capture is diff-at-commit (ADR-0003), so every `MoveLine` in these histories
  was lost. Git's own rename detection *was* imported, as `MoveNode`.
- Both repositories are small and one of them is this project's own wiki.
  Nothing here generalises to a kernel-sized history yet.
