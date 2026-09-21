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

## The same events, labelled three ways — and the contamination disappears

The import inherits git's commit boundaries, so the measurement above is partly
git's batching rather than anything inherent. A repository born in v0 is not
forced to batch: the log is continuous and a change is a label applied
afterwards. Re-partitioning the *same* event log tests that directly.

Average events pulled in when adopting one change:

| labelling | fpl first ⅓ | fpl last ⅓ | wiki first ⅓ | wiki last ⅓ |
|---|---|---|---|---|
| as committed (git) | 222 | 1 663 | 1 434 | 4 335 |
| split per file | 103 | 168 | 110 | 77 |
| **split per dependency component** | 110 | **123** | 129 | **70** |
| event-level, no labels at all | 142 | 228 | 163 | 119 |

**Flat in the tidy repo, shrinking in the messy one.** The contamination was
never a property of change granularity — it was git's commit batching, imported
faithfully. Splitting a commit into the connected components of its
semantic-reference graph removes it entirely, and does so *better* than dropping
labels altogether, because a component shares its dependencies instead of each
event dragging its own chain.

### Which makes the property antifragile rather than a hazard

`record` does not have to take the author's word for what belongs together. It
can partition pending events into dependency-connected components and offer one
change per component, for the human to name. Hygiene stops being a discipline
and becomes a computation — a symbolic gate where there used to be a habit.

The uncomfortable corollary above therefore inverts: a system in which commit
hygiene is a performance characteristic, *and* which computes that hygiene for
you, is strictly better than one where the cost is hidden and nobody can act on
it. See ADR-0007.

## What it would have cost to fix the other way

Tracking dependencies per event means adopting a change may pull only *part* of
another change — a fragment of somebody's commit. The document stays coherent
(the fragment is dependency-closed), but "I took commit X" stops being true.
Pijul keeps changes whole and pays the contamination. The measurement said the
bill was large — but the re-labelling experiment says the bill is avoidable
without touching what a change *means*. ADR-0006 is therefore **withdrawn**: we
keep whole changes and get boundedness, by choosing better boundaries rather
than abandoning them.

## Caveats

- The import is linear (`--first-parent`), so the causal closure is by
  construction the whole prefix. That is faithful to how git adoption works,
  but it is not a comparison against a cleverly pruned causal baseline.
- Capture is diff-at-commit (ADR-0003), so every `MoveLine` in these histories
  was lost. Git's own rename detection *was* imported, as `MoveNode`.
- Both repositories are small and one of them is this project's own wiki.
  Nothing here generalises to a kernel-sized history yet.


## Does commit discipline show up in the data?

`fpl` was written under an enforced atomic-commit and stacked-diff discipline;
the wiki was not. If discipline helps, it should be visible as fewer dependency
components per commit and fewer commits welding unrelated files.

| | fpl (disciplined) | wiki (not) |
|---|---|---|
| components per commit, mean | 4.33 | 4.35 |
| components per commit, median | 2 | 1 |
| files per commit, mean | 2.22 | 2.55 |
| commits welding ≥2 files into ≥2 components | **55%** | 44% |

**No.** The two are indistinguishable, and on the welding measure the
disciplined repo is slightly *worse*. `fpl`'s better-looking growth earlier
(222 → 1663 against the wiki's 1434 → 4335) is a matter of scale — smaller
files, fewer events per change, a shorter history — not hygiene.

Two readings, and they are not exclusive:

1. **The metric cannot see intent.** It measures whether edits *refer* to each
   other. A human-atomic commit — a fix, its test, and a line of docs — touches
   three files that reference nothing in common, so it splits into three
   components and scores exactly like a careless batch. Machine atomicity and
   human atomicity are different predicates.
2. **Stacked diffs optimise for the opposite property.** A stack is a
   deliberate dependency *chain*: each diff builds on the one below so it can be
   reviewed in order. v0 rewards *independence*. A perfect stack is a maximal
   dependency chain — the most expensive shape there is for adoption. The two
   disciplines are not merely different, they pull against each other.

The tool whose model does line up is **GitButler**: virtual branches keep one
working directory and assign each hunk to a lane, so what you commit is a set of
independent changes rather than a chronological batch. That is component
partitioning done by hand, with ownership remembered between edits. v0's
contribution is to *derive* the assignment GitButler asks you to drag —
[docs](https://docs.gitbutler.com/features/virtual-branches/virtual-branches).

## I6: the ordering does interleave, and exactly where the theory says

Two peers typing three lines each from a shared anchor:

- **forward** (each line anchored to the one just typed): blocks stay
  contiguous. Subtree contiguity is enough.
- **backward** (each line inserted above the last, so all share one anchor):

```
["base", "b3", "a3", "b2", "a2", "b1", "a1"]
```

Perfect alternation. This is the case the Fugue paper shows RGA-family
orderings get wrong, and v0 uses an RGA-family ordering. Recorded as a
characterisation test (`i6_backward_typed_blocks_interleave_until_fugue`) that
asserts the defect, so it turns red the day the ordering is fixed.


## Closing the move invariants

I5, I7, I10 and I11 were the last unexecuted parts of the model. All four now
run, and the first attempt at I10 was **vacuous**: both peers moved the same
line to destinations that happened to read back in the original order, so the
test passed while proving nothing. A deliberate vacuity guard — assert that the
fixture actually changed something — caught it.

With visible destinations:

```
line move: ["one","two","three"]   ->  ["one","three","two"]
node move: ["one/f1","two/f2"]     ->  ["two/f2", "two/one/f1"]
```

- **I10** the moved line exists exactly once, and the higher `EventId` wins on
  every replica. Duplication is structurally impossible here: an atom is one
  entry in the position map, so a move overwrites rather than copies.
- **I11** two peers each moving one directory into the other: the first move
  applies, the second is declined because it would close a cycle, and both files
  stay reachable. Kleppmann's rule, working without coordination.
- **I5** `adopt(drop(S, C), C) == S`, for the set *and* the materialised worktree.
- **I7** dropping a change leaves a file it never touched byte-identical.

One thing worth stating plainly rather than dressing up: the "both replicas
agree" assertions in the move tests are **structural, not earned**. Once two
peers hold the same event set, I1 makes identical output a theorem. What those
assertions really test is that a decision — which move to decline, which move
wins — is a function of the set and not of arrival order.

The tree walks are now bounded rather than `loop`-until-root: a function whose
job is to enforce the no-cycles invariant must not assume it.


## What building the CLI found

The first real `record` split a one-line edit into **two changes**. Replacing a
line is a `Delete` naming the old atom and an `Insert` that Fugue placed against
the *next* line: they reference different atoms and never reference each other,
so nothing structural ties them together. You could adopt the delete without
the insert. Coherent document, wrong edit.

Two repairs, and the second is a design admission:

1. **Share a referenced line, share a component.** Two events naming the same
   atom are working on the same place. Restricted to *line* referents — the
   first attempt counted node references too, and since the tree root is not an
   event at all, every file created in one go welded into a single change.
2. **Capture contributes what it observed.** The diff adapter knows which ops
   came from one hunk; the graph cannot recover it. `from_save` now returns
   those groups and `record` unions them before deriving the rest.

That second point is the honest shape of the thing: **structure derives what it
can, and the adapter supplies what only it saw.** A live editor knows "this was
one replace" for the same reason, so the interface is the same either way.

### The price, measured

| fpl | before the fix | after |
|---|---|---|
| component split, first ⅓ → last ⅓ | 110 → 123 | 111 → **154** |
| changes produced | 329 | 200 |
| components per commit | 4.33 | 2.63 |

Coarser grouping costs some boundedness — growth 1.12× becomes 1.39×. Still far
from git's own boundaries (7.6×) and comparable to per-file (1.63×), and now the
changes correspond to edits a human would recognise. Worth it, but it is a
trade, not a free improvement.

## The CLI, end to end

```
alice$ v0 record -m "first cut"        # two unrelated files
recorded 2 change(s)                   # ADR-0007, visible
alice$ v0 record -m "better greeting"  # one replaced line
recorded 1 change(s): 2 events, 1 deps

bob$ v0 sync ../alice                  # no server, state vectors only
pulled 9 events (0 -> 9), head is 3 changes
bob$ v0 record -m "bob adds a line"
alice$ v0 sync ../bob
pulled 1 events (9 -> 10), head is 4 changes
alice$ v0 drop d09a659e                # the line disappears
alice$ v0 adopt d09a659e               # and comes back
```

`v0 deps` on the greeting change reports **6 events derived against 9 the author
had seen**, and does not drag in the unrelated file — cherry-pick, from the
command line, on a real working copy.

## The live weave, per keystroke (2026-09-21)

Measured with `examples/weave_bench` (Apple silicon, release, TigerStyle
allocator): fpl's text repeated to 20 KB, 200 KB and 2 MB, fragmented by
5000 warm-up keystrokes typed in bursts at random spots, then 2000 timed
keystrokes. `bench/weave.sh` sweeps the sum tree's branching factor B and
the fragment cap F; three runs each, medians. Raw data in `bench/results/`.

**The first sweep measured a bug, not the structure.** Applying a keystroke
grew with B squared -- 22 µs at B = 8, 52 at 16, 170 at 32, 666 at 64 -- and
barely with the document. The profile (`sample`) put it in allocation and in
`FragmentSummary::add`: a `Locator` was a `Vec`, cloned into every summary on
every sum; and `split` rebuilt each half child by child, O(B²) per level.
With `Arc<[u32]>` keys and a split that makes one root over whole children,
B = 16 on 200 KB went from 52 µs to 5.9 µs.

**After the fix** (B = 16, F = 128, 200 KB, p50):

| step | cost |
|---|---|
| LSP position → offset | 42 ns |
| offset → op | 0.2 µs |
| op → weave | 5.9 µs |
| offset → LSP position | 42 ns |
| a whole keystroke | 6.2 µs (p99 23 µs) |
| backspace | 11.7 µs |

- **Document size hardly matters**: a keystroke costs 5.4-6.8 µs at 20 KB and
  at 2 MB alike. That is the O(log n) the structure was for.
- **B**: 8 and 16 are tied within ~10 %; 32 costs ~25 % more, 64 about
  double (a wider node is copied on every path copy). **B stays 16**: tied
  for fastest, half the depth of 8.
- **F** barely moves keystrokes. It trades fragments (2 MB: 68 824 at F = 32,
  22 088 at 128, 10 404 at 512) against opening (from_walk 82 / 99 / 122 ms)
  and relayout. **F stays 128**, the middle of a flat curve.
- **The real bottleneck is not the weave.** `EventLog::append` costs ~270 µs
  at 7 000 events -- 45 keystrokes' worth of weave -- because it recomputes
  the frontier and the Lamport clock over every event, every call. It grows
  linearly with history; at 70 000 events it would be ~3 ms a keystroke.
  Fix before any live front-end (TECHDEBT).
- **A move is a full relayout**: 5.6 ms at 20 KB, 32 ms at 200 KB, 280 ms at
  2 MB. Fine while moves are rare; for a large file, a noticeable pause.
- **Opening** 200 KB: replay 23 ms + from_walk 13 ms; 2 MB: 185 + 100 ms.

Every configuration stays two to three orders of magnitude inside a 16 ms
frame. One sweep row (B = 16, F = 128, 20 KB) came out ten times slower in
every column, replay included; five reruns matched its neighbours, so it
was the machine, and it is left in the raw data as it was measured.

**Fixed the same evening.** `EventLog` now keeps its frontier (`heads`, plus
`missing` for parents a partial sync has not delivered yet) and its Lamport
clock per insert, behind a private event map with `insert` as the only way
in; loading rebuilds them through the same `insert`. A property test holds
both to their definitions -- recomputed from every event -- across replicas
exchanging random subsets in reverse order. `append`: **270 µs → 84 ns** at
7 000 events, **125 ns** at 52 000: what is left is the event map's own
O(log n).
