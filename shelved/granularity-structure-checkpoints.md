# Shelved: granularity, structure, checkpoints

**Status:** shelved 2026-09-21 — questions raised while fixing file-level
conflicts, not yet researched. Companion to
`history-presentation-redaction-provenance.md`.

## Is the line the right unit?

v0 is line-granular (ADR-0002) because it was cheap. Rob Pike's argument for
*sam* ("Structural Regular Expressions", 1987) is that lines are an arbitrary
structure imposed by `ed`/`sed`: text is a sequence of characters, and structure
— lines, paragraphs, fields, functions — should be defined by the operation, not
baked into the medium.

The cost of lines is already visible in v0:

- conflict survival misreads an inline edit as "this side lost" (ADR-0009);
- a move can only ever be a whole line;
- the diff adapter has to guess at line boundaries what was one edit.

**Fugue and eg-walker are character-level natively.** Going to characters is
mostly giving `Insert` a run instead of a line — lines become a *view*. It needs
run-length encoding to stay fast (Diamond Types, Loro and Yjs all merge
consecutive characters from one author into runs).

## Ropes, and undo that knows about peers

A rope is the *in-memory representation* of large text with cheap edits (Zed's
SumTree, Ropey, Xi). It is orthogonal to CRDT granularity: the CRDT decides
identity and order, the rope decides how the materialised text is held. Ropes do
not change the algorithm's complexity; they make the working copy cheap to edit.

A **peered undo tree** falls out of the existing algebra: undoing *my* last edit
while others keep typing is replaying the set without my events — the same
subtraction as `drop`, at event rather than change granularity. Selective undo
in a multi-writer setting is otherwise notoriously hard; here it is a subset.

## Layered structural ops: tree-sitter and LSP

Store intent, not just text, where a language gives it to us — layered, so
formats without tooling degrade to plain text:

1. **text op** — always present; what v0 has now.
2. **syntactic annotation** — tree-sitter node path of what was edited (this was
   a change to `fn main`'s body), available for any language with a grammar.
3. **semantic op** — an LSP action as a single op: *rename symbol* is one
   `WorkspaceEdit` across many files. Recorded as such, a rename and its call
   sites are one change with one intent, instead of N unrelated line edits.

Layer 3 is the textual approximation of what Unison gets from content
addressing (the "rename is not an edit" point in the wiki). It would make many
rename/call-site conflicts disappear rather than be resolved.

## Checkpoints as first-class objects

`import-git` is quadratic because every commit re-materialises the whole log.
Ropes do not fix that; **incremental materialisation from a checkpoint** does —
keep the materialised state and apply only new events. That is eg-walker's core
move: replay only the concurrent part since the last common version.

The idea to keep: a checkpoint is itself an **object in the model**, not a
cache:

```
Checkpoint { frontier: ChangeSet, tree: hash(M(frontier)), meta, signature }
```

- **verifiable by anyone** — replay the frontier and compare the hash;
- **signable** — once signing exists (see the other shelved note);
- **useful three ways**: fast start (replay from here), releases, provenance.

It is what a release tag is on a forge, but first-class: not a pointer that can
be moved, but a claim about content that can be checked.

## Resume here

1. ~~Character granularity + run-length encoding~~ — **done 2026-09-21,
   ADR-0012.** Two surprises on the way: character *diffs* make letter soup
   (diff by words), and words fuse across authors (cut tokens at run
   boundaries).
2. Checkpoint objects — fixes import cost and gives releases a real home.
3. tree-sitter annotations — cheap once characters exist.
4. LSP semantic ops — needs a live front-end, so after v1's capture decision.
