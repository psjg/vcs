# Architecture decision records

Append-only. Newest last.

## ADR-0001 — own minimal weave, not an off-the-shelf CRDT library

**Status:** accepted, 2026-09-20.

Default policy is buy-over-build, and Loro/Automerge/Yjs are hardened, fast and
maintained. We still write our own sequence core, for one reason: **the spike's
research question is replay over arbitrary dependency-closed subsets, and
production CRDT libraries deliberately forbid exactly that.** They enforce
causal closure — you may not apply an op without everything that causally
preceded it — because that is what makes convergence provable. Our layer 1
needs the opposite: `unrecord` materialises `S \ upward_closure(G)`, a subset
that is dependency-closed but *not* causally closed.

Fighting a library's core invariant to run the experiment costs more than
writing ~400 lines of weave. Keep `Weave` behind a narrow interface so a Loro
backend can be measured against it once the semantics are settled.

**Cost if wrong:** we own a sequence CRDT's correctness. Mitigated by the
property tests being the deliverable, not the code.

## ADR-0002 — line granularity

**Status:** accepted, 2026-09-20.

Ops address lines, not characters. Halves the state, matches how developers
talk about diffs, and keeps a weave printable during debugging. Character
granularity is a mechanical change later (the op type gains a byte range);
AST granularity is a different project (see Unison).

## ADR-0003 — capture by diff at record time

**Status:** accepted, 2026-09-20.

v0 reconstructs ops by diffing the working file against the materialised state,
exactly as git does — and inherits the known weakness: the interpretation of an
edit is baked in at record time. The model does not depend on this; a live
capture front-end (editor plugin, FS interceptor) can replace it without
touching layers 1 and 2. Recording it as an ADR so the limitation is not later
mistaken for a property of the design.

## ADR-0004 — supersedes ADR-0001: why the materialiser is ours

**Status:** accepted, 2026-09-20. Supersedes ADR-0001's reasoning, keeps its conclusion.

ADR-0001 argued that off-the-shelf CRDT libraries "enforce causal closure" and
therefore cannot materialise our subsets. That is a metadata check, not a law,
and it was the wrong justification.

The real reason is narrower and survives scrutiny: `M(S)` applies **our**
ordering rule to **our** event identities over an **arbitrary** dependency-closed
subset. That is the semantics under test. A library's value — incremental merge,
columnar encoding, years of fuzzing — is orthogonal to it. So the materialiser is
ours, `Materialiser` is a trait, and a Loro-backed implementation is a
*performance* comparison to run once the semantics settle, not a dependency to
design around.

Corollary: owning the materialiser means the **op set is a design choice**, which
is what makes ADR-0005 possible.

## ADR-0005 — first-class move ops

**Status:** accepted, 2026-09-20.

`MoveLine` and `MoveNode` are ops, not delete-plus-insert. Rationale: a move
expressed as delete+insert destroys identity, which is why git needs similarity
heuristics (`-M`, `--follow`) to *guess* renames. As an op it is a fact, its
dependency is the moved atom rather than the surrounding text, and concurrent
moves get answers from the literature (Kleppmann's cycle-safe tree move;
deterministic winner-by-id for lines) instead of ad-hoc rules.

**Cost:** three new concurrency cases (move/move, move/delete, move-into-own-
subtree), paid for in invariants I10–I11. Accepted because rename and
code-motion tracking is a core VCS job that every snapshot system fakes.

## ADR-0006 — dependencies per event, not per change

**Status:** WITHDRAWN, 2026-09-20, superseded by ADR-0007 before anyone had to
decide. The re-labelling experiment showed the problem it solved was not
inherent — see FINDINGS. Kept for the record because the reasoning was sound
given the data available at the time.

Measurement (docs/FINDINGS.md) shows the change-level dependency closure grows
with history at the same rate as the causal closure it was supposed to replace —
222 → 1663 events over 76 commits in `fpl`, 1434 → 4335 over 211 in the wiki.
The event-level closure is flat over the same histories (142 → 228, 163 → 119).

Cause: a change is adopted whole, so a commit touching several unrelated files
welds their histories together, permanently, for every later change.

**Proposal:** keep `Change` as the human-facing label, but compute closure over
events via `Op::refs` rather than over changes via `deps`. `ChangeSet` becomes a
dependency-closed *event* set.

**Cost:** adopting a change may take only part of another change. The resulting
document is coherent, but "I adopted commit X" is no longer true, and a UI has
to explain partial ancestry. Pijul chose the other side of this trade and pays
the contamination instead.

**Not decided here.** The data is unambiguous about the cost; what a change
*means* is the user's call.


## ADR-0007 — `record` partitions by dependency component

**Status:** accepted, 2026-09-20.

`record` does not take a set of events on trust. It partitions the pending
events into connected components of the semantic-reference graph and produces
**one change per component**, which the human names.

Measured justification (FINDINGS): adopting a change costs 222 → 1663 events
across `fpl`'s history under git's own commit boundaries, and 110 → 123 under
component splitting. Flat, on the same events. In the messier history the cost
*falls*, 1434 → 4335 becoming 129 → 70.

**Consequences.**

- A change stays a whole, human-meaningful unit, so ADR-0006's trade never has
  to be made.
- Commit hygiene becomes a computation rather than a discipline: you cannot
  accidentally weld unrelated work together, because the machine draws the
  boundary from what the edits actually reference.
- One "commit" may become several changes. The UI has to make that pleasant —
  name them together, adopt them together if you like — but they stay
  independently adoptable, which is the entire point.
- The rule is a heuristic about *reference*, not about *intent*: two edits that
  belong together conceptually but touch nothing in common will be split. The
  escape hatch is an explicit "these are one change" override, which must be
  recorded as such rather than silently merging the components.


## ADR-0008 — Fugue for the ordering

**Status:** accepted, 2026-09-20. Closes the last open question in the
materialisation model.

The first ordering was RGA-family: one anchor per line, siblings sorted by
`EventId`. Measured behaviour (FINDINGS): two peers typing three lines each
*forwards* from a shared anchor stayed contiguous, but typing *backwards* —
every line inserted above the last, so all share one anchor — produced a perfect
alternation, `base b3 a3 b2 a2 b1 a1`.

Replaced with **Fugue**: a node carries a parent *and a side*, reading order is
the in-order traversal, and placement uses `between(a, b)`. Both interleaving
tests now pass.

**Consequences.**

- `Op::Insert` gains a `side`, and `MoveLine` with it. Placement is decided at
  capture, so replay stays a pure walk and `refs()` — the dependency input — is
  unchanged: still one parent per line.
- Capture needs ancestry, which it reads straight from the log, because every
  insert stores its own parent. No extra index.
- A caller can no longer say "put this after X" and be right by default. The
  naive right-child placement is wrong exactly when X is an ancestor of the line
  that follows it, and it broke one of our own tests within minutes of the rule
  landing. `capture::between` is the only correct way in.
- Import numbers are unchanged (222 → 1683 against 222 → 1663), as expected:
  Fugue changes *where lines sit*, not *what depends on what*.


## ADR-0009 — conflicts are derived objects, resolved by patches

**Status:** accepted, 2026-09-21.

Between pijul (a conflict is state in the model) and Manyana (the model is
conflict-free, a conflict is a view), v0 takes a third position that the
set-reading materialiser makes available:

- A conflict is **derived** from the change set: two changes that did not see
  each other both deleted or moved the same line, and at least one of them put
  something in its place. Two concurrent deletions agree and are not a conflict.
- It has a **stable identity**, `blake3(contested atom, sorted sides)`, so every
  replica computes the same conflict with the same id without storing anything.
- It stays **open** until a change in the set declares `resolves: [id]`. Open
  conflicts are drawn as markers on checkout, `record` refuses text that still
  contains them, and `v0 conflicts` exits 4 — a state to handle, not a crash.
- A **resolution is a patch**: whatever the resolver did to the text — pick one
  side, keep both, merge them inline, or change nothing — plus a *declared*
  dependency on every side. Adopting a resolution therefore brings its conflict;
  dropping either side drops the resolution.
- Two **concurrent resolutions** of one conflict are themselves surfaced
  (`Contested`), because left silent they could delete both people's choices.
- Nothing is rewritten. Who resolved it and why are in the change; *which side
  survived* is derived from the text rather than claimed.

**Why not git's model.** A git conflict has no identity: it lives in
`MERGE_HEAD` and disappears at commit. The merge commit records the outcome, not
which conflict it settled, which side won, or why. `rerere` remembers
resolutions only locally.

**Known limit.** Survival is judged by line identity. An inline edit that
extends one side's line counts as replacing it — the audit trail says "0/1 of
its lines survive" for a side whose content was kept and extended. Character-
or token-level capture would fix this; line-level diff capture cannot.


## ADR-0010 — event sequence numbers are a Lamport clock

**Status:** accepted, 2026-09-21.

Every "last writer wins" rule in v0 — a file renamed twice, a mode set twice, a
removal and a restore — decides by `EventId` order. `seq` was a per-replica
counter, so a replica with a low counter that acted *after* reading someone
else's work still sorted before it and lost. A failing test showed it: a rename
made after seeing another rename lost to it (`busy.txt` instead of `final.txt`).

`append` now mints `max(own counter, highest seq seen + 1)`. Causally later
events always sort later, which makes `EventId` order a linear extension of
causal order. A replica's seqs stay monotone but are no longer dense; state
vector sync is unaffected, because it only ever asks for "every event of r above
hi".

## ADR-0011 — file-level conflicts, and `Restore`

**Status:** accepted, 2026-09-21. Fixes silent data loss found by using the CLI.

Conflicts existed only on lines, so removing (or renaming — which diff capture
sees as remove plus create) a file while someone else edited it lost the edit,
with no conflict reported. Two new kinds:

- **Removal** — a change removes a node while a concurrent change edits that node
  or anything below it (a line inside, a file created in a directory, a rename).
  Concurrent removals of the same node agree and are not a conflict.
- **Rename** — concurrent moves of one node to different places.

A disputed removal is *shown*: checkout keeps the file on disk, framed by
markers naming who removed it and who edited it, so the resolver can read it.
Resolutions are the usual patches: delete the file to accept the removal (an
empty resolution, still recorded with a reason), keep it, or carry the edit
elsewhere. Keeping it is a new op, **`Restore`**, which un-removes the node with
its identity — the same lines, the same atoms, the same history — rather than
creating a new file that happens to share a name. Capture emits it whenever a
removed file's path reappears on disk.

Found alongside and fixed: checkout never deleted a file whose removal arrived
from a peer, because it asked the materialiser for "every path ever" and the
materialiser omits removed files by design. The next `record` then saw the stray
file and restored it, silently undoing the peer's removal. Pinned by a CLI test
that fails on the old code.


## ADR-0012 — character granularity (supersedes ADR-0002)

**Status:** accepted, 2026-09-21.

Lines were chosen for cheapness (ADR-0002), and Pike's critique of line-oriented
tools applied: the conflict audit misread an inline extension as a side "losing"
(ADR-0009), moves could only be whole lines, and two edits to different words
on one line were a conflict when they are not one.

**Model.** A character's identity is `Pos { event, offset }` — Zed's (insertion
id, offset) anchor. An `Insert` is one event carrying a *run* of text; only its
first character has an explicit Fugue parent and side, and each following
character is the implicit right child of the one before. `Delete` names a range
within one run. `MoveRun` moves a whole run (partial moves would split runs, and
no adapter emits moves yet). `Op::refs()` still returns event ids, so dependency
derivation, closures and components are untouched — verified: on fpl the
unit-free statistics (200 changes, 2.63 components per commit, 26% independent,
55% welding) are identical before and after. Import time fell from 21 s to 3.4 s
because an event is now a run rather than a line.

**Capture: identity per character, diffing per word, tokens cut at authors.**
Two findings from tests written to justify this change, both real:

1. *Character diffs make letter soup.* Two concurrent rewrites of "hello world"
   (to "goedendag" and "hoi") merged into `"goedniag"`: a character diff keeps
   stray letters it happens to share with the base, and each side deleted the
   letters the other kept. Inside a changed hunk the diff now works on Unicode
   words; a replaced word leaves whole and arrives whole, and the same situation
   is a clean conflict.
2. *Words fuse across authors.* After that merge the two words sit flush —
   `"goedendaghoi"` is one word to a segmenter — so a resolver keeping Alice's
   word and dropping Bob's saw one word replaced and deleted Alice's characters
   too. Old-side tokens are now also cut wherever the authoring event changes.
   With both fixes the audit reports Alice 9/9 characters kept, Bob 0.

A live editor front-end has neither problem: it knows which characters were
typed. Both fixes are costs of diff capture.

**Rendering.** A text conflict is a stretch of characters widened to whole lines.
Each side's section is the head *with the other sides taken out*, and a `before`
section is the head with all sides taken out — diff3 from set subtraction, with
no merge algorithm.

**Robustness, learned the hard way.** A test that bypassed the fixture sent
`Delete { range: (0, u32::MAX) }`; replay expanded it into four billion
positions and ran the developer's machine out of memory. The same event from a
hostile peer would do that to every replica. Every expansion of a delete range
now goes through `op::clamp`, and a regression test sends the oversized range on
purpose.


## ADR-0013 — a TigerStyle memory budget, reserved at startup

**Status:** accepted, 2026-09-21.

**Why.** A test expanded a delete range of `(0, u32::MAX)` into four billion
positions and ran the developer's Mac out of memory; the machine froze, with the
test billed to the Claude app as 7.96 GB. The range bug is fixed (ADR-0012), but
the lesson is the one TigerBeetle's style guide states: put a limit on
everything, and allocate memory at startup.

**Why not the kernel.** Measured on macOS 27: `ulimit -v` and `ulimit -d` are
refused (`setrlimit failed: invalid argument`) and a child allocates 300 MB
regardless. Linux can do it (`RLIMIT_AS`, or better cgroups v2 `memory.max`),
but a kernel kill is a bare `SIGKILL` with no explanation. The limit belongs in
the process, where it can say what happened, identically on every OS.

**What.** `budget.rs` makes talc over one reserved region the global allocator.
The first allocation (before `main`) claims a 2 MB bootstrap block; `main`
reserves the rest of the budget in one piece from `--memory MB`, `V0_MEMORY_MB`
or 512 MB; after that the OS is never asked again. Exhaustion prints the budget,
the failed request and how to raise it, then exits with code 5 — assembled on
the stack and written with `write(2)`, since the allocator is what failed and
must not be re-entered.

**Measured.** With the clamp removed on purpose, the test that froze the machine
now dies in 1.5 s at 68 MB under a 64 MB budget, with the message. Recording a
47 MB file under `--memory 16` fails the same way at a 3 MB peak. The suite runs
with peaks of 5 and 11 MB.

**Not full TigerStyle.** TigerBeetle also never allocates *inside* its budget,
because every structure has a fixed capacity. v0 still allocates and frees
within the region. Like the JVM's `-Xmx`, the budget bounds the heap, not
thread stacks or the binary.

**Addendum, 2026-09-21: profiling builds.** The budget hides allocations from
every macOS memory tool: `leaks --atExit` on `v0 status` reports one 524 331 KB
block (the reserved region) in the budgeted build, against 11 KB of individual
live allocations in a build with the `system-alloc` feature. That feature swaps
the budget for the system allocator so Instruments Allocations, `heap`, `leaks`
and `malloc_history` can see inside; `v0 status` then says "NO budget", and the
budget test is skipped. Never ship it.

Time is bounded the same way, by `.config/nextest.toml`: a test is killed after
30 s. Verified with a probe that sleeps 60 s — `TIMEOUT [30.004s]`.


## ADR-0014 — conflict hygiene: resolve first, agreement is derived, diff3 by author

**Status:** accepted, 2026-09-21. Driven by a playground session that ended with
five conflicts on one line and rendered text nobody had typed.

1. **Resolve first.** `record`, `sync` and `adopt` refuse (exit 4) while any
   conflict is open, as git refuses a merge over unmerged paths.
   `--despite-conflicts` pushes through with a warning every time, and `record`
   then skips files still showing markers.
2. **Resolve many at once.** `resolve [<id>...] [--file PATH] -m WHY`, default
   every open conflict: one edit, one reason, one change depending on all sides.
3. **Agreement is a derived, zero-op resolution.** When every side's replacement
   for a contested stretch is the same text, the conflict is `Agreed` and the
   materialiser hides all but the lowest-EventId side's runs. It must stay
   derived: a *recorded* dedupe would hard-code which copy survives, and dropping
   the kept side later would lose the text entirely. `Moot`: none of any side's
   replacement text survives. Both count as closed.
4. **Diff3 by author.** Each rendered section is the materialised causal past of
   that side's change — exactly what its author had — and `before` is the
   intersection of those pasts. Subtracting the other sides from the head only
   worked for two sides.

Verified: import numbers on fpl unchanged to the last digit (materialisation
only); a three-way conflict renders every section byte-exact; dropping either
side of an agreement leaves the text once.


## ADR-0015 — hooks: local, obeyed, and never carried by sync

**Status:** accepted, 2026-09-21. Asked for by the human: red tests should stop
a change the way open conflicts do.

1. **One hook, `.v0/hooks/pre-record`**, run by `record` and `resolve` before
   they capture anything. Nonzero exit refuses the command with exit 6 and
   nothing is minted. A resolution is an edit like any other, so it is guarded
   too. No hook per command on speculation: others wait for a need.
2. **The working copy is the thing tested.** v0 has no staging area, so the
   files on disk are exactly what `record` turns into events; unlike git's
   pre-commit, the hook needs no export of an index to be honest.
3. **Local, never synced.** `.v0/` is neither scanned nor exchanged, so a peer
   cannot make you run their code — the reason git never clones hooks either.
   Sharing a hook is done the way a team shares any script: track it, and link
   it into `.v0/hooks/` by hand.
4. **A hook that cannot run is a refusal, not a skip.** git silently ignores a
   non-executable hook; a guard you believe in that never fires is worse than
   none.
5. **`--despite-hooks`** records anyway and warns every time — the same shape,
   and the same reason, as `--despite-conflicts` (ADR-0014).

Environment: cwd and `V0_ROOT` are the canonical repository root;
`V0_ACTION` is `record` or `resolve`; `V0_MESSAGE` the `-m` text. Stdin is
closed, output passes through. Six CLI tests pin each point above.


## ADR-0016 — too near: adjacent concurrent edits are a conflict

**Status:** accepted by the agent finishing v0, 2026-11-10; the human has
not reviewed it. It changes what a conflict is, so it is the one decision of
that session to read first. Reversible: conflicts are derived, never stored.

**The gap.** ADR-0009 calls a conflict two unseen changes that removed or
moved *the same characters*. Two people who each add a line at the same spot
share no character, so they merged silently, in the order the event ids
happened to give (TECHDEBT, 2026-09-21, twice). Fugue keeps each block whole
(I6); it does not make the order between the blocks anybody's decision.

**The rule** (`Kind::Near`). For two changes that did not see each other, in
one document's reading order with tombstones: a character is *touched* by a
change that inserted, deleted or moved it; a character *separates* when
neither touched it, both had seen it, and neither had seen it deleted. The
pair is too near when a stretch between two separators holds touches of both
and no character touched by both (that is ADR-0009's conflict already). Two
changes that only delete are not in conflict, as before: they cannot
disagree about what the text should say instead.

1. **It reads the two changes, their pasts and the reading order.** Text a
   third party types or deletes meanwhile neither joins nor separates them,
   so the conflict's identity -- the first touched character of the stretch,
   and the sides -- does not move when others edit nearby. Pinned by a test
   with a third writer. The exception is a third party *moving* one side's
   text: a move changes the reading order, the edits no longer touch, and
   the conflict is gone without a resolution (found in review, 2026-11-10;
   accepted, since the adjacency it reported no longer exists).
2. **Any character separates, a single space included.** Changing two
   neighbouring words still merges; that is what ADR-0012 bought and
   `different_words_on_one_line_merge_cleanly` pins. A wider rule -- spaces
   and punctuation transparent, the reading of Manyana's "adjacent or
   whitespace-separated" that TECHDEBT pointed at -- was built first and
   turned exactly that test red.
3. **`x = 1` edited at both ends stays a silent merge.** Whether two edits a
   few characters apart are one expression is a question about syntax. No
   character class answers it without also flagging `hello world`; the
   structure layer could (two edits under one smallest node), and that is
   where it belongs if it is ever wanted.
4. **Overlapping stretches are one conflict** with all their sides: three
   people at one spot is one decision (ADR-0014's lesson).
5. **Everything else is ADR-0009 and ADR-0014 unchanged:** drawn as markers
   with diff3 by author, resolve-first, settled by a change that says so.
   There is no derived `Agreed` or `Moot` for this kind: two people typing
   the same line at one spot get asked once.

**Cost.** One walk of a document per concurrent pair touching it, on every
command that derives conflicts. Pairs are few while resolve-first holds; a
long-lived fork would want an index. Import numbers cannot move: the
importer never derives conflicts.

**Tests.** Six in `tests/algebra.rs` (the rule, its identity, the space, the
deletions, three sides, resolution) and one end to end in `tests/cli.rs`.

## ADR-0017 — intents: one rule for every verb, guarded by who had seen it

**Status:** accepted by the agent, 2026-11-11; the human has not reviewed
it. It replaces the rename-only derive of the live session, and it fixes
what may change text everyone agreed on, so read it before ADR-0016's
conflicts are asked to catch what it lets through. Reversible: derived
events are ordinary events, and the event format already changed for it.

**The gap.** v0 had one intent, `Op::Rename`, and a merge rule written for
it: walk the tree for leaves reading the old name, ask the resolver which
of them bind. A code action or a format has no candidates to walk; each
new verb would have needed a rule of its own, and a language server (v1)
answers verbs, not "does this leaf bind".

**The rule** (`src/live.rs`, module doc). An intent (`Op::Intent`: rename,
code action, format) is an event that says what was meant; its effect is
ordinary character ops linking to it. At merge, for each intent `I`: build
*before*, every event except those linking to `I`; resolve `I`'s anchors
there (gone: skip); let the resolver run `I`'s verb on *before*, giving
`T` (refused: skip); diff the merged text `M` to `T`; keep the hunks the
guard allows, as events with content-hashed ids linking to `I`; go on to
the next intent, in passes, until a pass derives nothing.

1. **Diff determinism.** The derived ops are a canonical character diff:
   `similar`'s `TextDiff`, `Algorithm::Myers`, over chars (Unicode scalar
   values, not graphemes), no deadline, each maximal run of non-equal ops
   one hunk. It is part of the protocol: two replicas with equal `M` and
   `T` must mint identical ops, because `syntax::derived_id` hashes them.
   The effect recorded with an intent is the same diff, from the state the
   editor showed to the verb's result, so a coarse answer (a formatter
   that replaces the whole range) still keeps every character it did not
   change, and what was typed against those characters stays put.
2. **The guard** keeps convergence independent of the resolver. Two
   replicas whose language servers disagree get different `T`s; without a
   guard a diff could rewrite text both had agreed on, each side undoing
   the other. A character is *unseen* when the event that typed it did not
   causally see `I`. A hunk is allowed iff every character it deletes is
   unseen and it deletes one, or a character next to it is unseen, or its
   text is a substring of what another allowed hunk deletes (a move: a
   word typed inside a range that an action moved follows the range). An
   intent's effect and derived events see it, so nothing it derived is
   rewritten by it again.
3. **The fixpoint order.** Intents in `EventId` order; each one's derived
   events are absorbed before the next is asked, so it reads them; passes
   repeat until one derives nothing, at most eight, a counter when the
   bound is hit. *Before* keeps other intents' effects: a rename asked of a
   document without a concurrent rename's effect would undo it (the guard
   lets it, since that effect did not see this intent).
4. **Anchors as `Pos`, never offsets.** A rename names its declaration's
   first character; an action or a format the first and last character of
   its range, as `NodeRef` names a node. A range that lost an end is what
   is left of it; a rename whose declaration is deleted, or no longer reads
   `from`, is skipped.
5. **Cross-file edits are refused.** A replacement is a character range of
   the one document; a resolver's answer out of order, overlapping or out
   of the document is refused, never clamped. A language server's
   `WorkspaceEdit` touching other files has nowhere to go here (TECHDEBT).

**It replaces** the candidate walk and `Resolver::binds`, `Resolver::
references` and `Refs`, `syntax::rename` and `syntax::reapply`. The
resolver is two methods: `locate` (the declaration under a cursor) and
`run` (an intent's replacements). v0's scope walk implements rename and
refuses the rest; v1's language server is to implement all three.

**What it does not solve.**

- **Two intents on one target still collide at character level.**
  Measured: Alice renames `foo` to `bar` while Bob renames it to `baz`.
  Each intent's *before* holds the other's effect, which deleted the
  declaration, so both are skipped as gone, and both references read
  `barbaz`. Two inserts at one anchor are ADR-0016's `Near`, which is what
  would report it (a live session derives no conflicts); there is no
  intent-to-intent reconciliation. Two equal formats of one range settled
  to one `a = 1` in the same probe; formats with different settings were
  not tried.
- **Text older than an intent counts as unseen.** Its events did not see
  the intent either. So a character typed *after* an intent, next to text
  the intent saw, can be reached through the adjacency clause: measured,
  `a=b` typed into a range formatted earlier is formatted at the next
  merge. Harmless for a formatter, wrong in principle; narrowing *unseen*
  to "concurrent with the intent" would, by reasoning not yet tested, also
  stop a rename reaching an old call that a concurrent deletion of a
  shadowing `let` exposed.
- **The diff is not the verb.** Myers aligns on equal characters; when a
  move is shorter than what it moves past, the recorded effect moves the
  other text. The text is the verb's; the identity of the characters is
  the diff's.
- **Cost.** Every intent on every merge: a weave rebuilt from the log, a
  parse, a resolver call, a whole-document character diff, and a causal
  closure per distinct event under a hunk.

**Tests.** `tests/live.rs`: the rename tests through the generic path (the
old derive's oracle) and one per pin: idempotence, the fixpoint with a
third peer, the guard, a format, the move case, identical derivations on
two replicas and a joiner.

## ADR-0018 — `v0_code`: verbs are data, notes carry the reason

**Status:** proposed, 2026-11-11. The direction is the human's, given in
conversation the same day; this text is the agent's and unreviewed.
Nothing here is built. It amends ADR-0017: the rule, the diff, the guard
and the fixpoint stay; the closed `Intent` enum, the one-way link and the
missing reason change.

**The aim.** A history that reads as a chain of small, named steps, each
with its reason, written by a human in an editor or by an agent through a
CLI or MCP, and working for every language that has a tree-sitter grammar
and a language server. An intent is the granular commit. It is an event
to study afterwards (why did the code grow this way), to trace, and to
re-run elsewhere.

**1. The vocabulary is open.** An intent is data, not an enum variant:

    Intent { verb: String, anchors: Vec<Pos>, args: Map<String, Value> }

The core reads only `anchors` (for `Op::refs` and I14) and the links. A
*verb definition* is data too: the name, how many anchors, a schema for
`args`, and who executes it. `v0_code` is the standard set of
definitions; a language (FPL) or a project adds its own without a change
to the core. The schema of a verb is the tool definition a front offers:
MCP tool, CLI flags, editor command.

**2. Four ways to execute**, named by the definition:

| Executor | Standard verbs | The effect comes from |
|---|---|---|
| none | `declare.class`, `declare.fn`, ... | what the author typed |
| tree | `node.move`, `node.delete`, `node.swap`, `node.wrap` | us, on a node's range; any grammar |
| server | `rename`, `format`, `refactor.extract.func`, `.var`, `.const`, `refactor.inline` | the language server |
| steps | a `node.move`, then the server's import fix | each step in order |

A verb with no executor has nothing to re-derive at a merge; it is a
label with anchors, and a later verb can point at what it made.

**3. Server verbs go through a table, not code.** `rename` and `format`
are one LSP request on every server. A code action is found by its
`CodeActionKind`; LSP allows sub-kinds (`refactor.extract.function`) but
does not make servers report them. Where a server reports only the
parent kind, a per-server table maps our verb to that server's own way
of telling the actions apart (a sub-kind, a command id, a title as the
last resort). It is typed data under the same schema as the definitions.
`Intent::Action { kind, title }` of ADR-0017 becomes this: the title is
a fallback matcher in a table, no longer the identity of the intent.
Which servers report sub-kinds is not measured.

**4. A note is an event that carries the reason.**

    Op::Note { text: String, spans: Vec<Span> }

It links (I15) to the intents or events it is about, and its `spans` are
ranges of characters named as `xanadu/transclusion`'s `Span { first,
last }` names them, so they follow the text. Its causal parents are the
frontier it was written at, which is what that spike's `Cite` adds to a
span. "Extract this because these two passages do the same" is a note
with two spans, linked to the extract. Links today hold `EventId`s only
(`event.rs`), which is why the spans live in the op. A note needs no
grammar and no server: it hangs on prose as well as on code.

**5. An intent may link to its effect.** ADR-0017 has the effect link to
the intent, which only works when the intent is recorded first. An agent
that labels afterwards (recognises that these keystrokes were a rename)
appends an intent that links to the older events; I14 already allows a
younger event to link to older ones. So the effect of `I` is: the events
linking to `I`, and the events `I` links to. ADR-0017's guard exempts an
effect because it causally saw the intent; a labelled effect did not, so
the exemption must be by membership in the effect, not by causality.
That change needs its own red test.

**6. Nothing is required of a human.** Typing without declaring anything
is free text, as before. A bracket command (open an intent, type, close
it) is offered, never imposed. For an agent every tool call is one
intent, already delimited. A file with no grammar (an essay, a page of
Markdown) has no `v0_code` layer at all; notes still apply.

**7. Conflicts move up a level** (direction, not designed). Measured in
ADR-0017: two concurrent renames of one declaration give `barbaz`. With
verbs as data a definition can name what makes two intents the same
question (same verb, same first anchor); concurrent intents that agree
on that and differ in `args` are one conflict with the two answers as
its sides, instead of a character collision. This is where VISION §8's
per-type conflict function would attach.

**8. Cherry-pick is a re-run** (direction, not designed). An intent's
dependency is its anchors, not the lines its effect touched. Picking a
rename into another world runs the verb there, so references that exist
only there follow. ADR-0017's merge rule does this for unseen text under
the guard; a pick has no guard to lean on and is a separate case.

**What it costs.** The compiler no longer checks a verb's arguments; the
schema does, at the boundary, and an unknown verb must be refused with
its name, never guessed (strict readers). A replica that lacks a verb's
definition or its server still converges, because effects are ordinary
events; it cannot derive for that verb, which ADR-0017 already counts as
`refused`.

**Order to build it in.** (a) `Intent` as data and the definitions for
what exists today (`rename`, `format`, one code action), every ADR-0017
test passing unchanged; (b) the two-way link with its guard test; (c)
`Op::Note` with spans; (d) the `tree` executor; (e) a CLI and an MCP
front over the same events. Items 7 and 8 wait for a design of their own.
