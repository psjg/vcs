# Xanadu on v0's atoms — a spike

Ted Nelson's Xanadu wanted three things the web never got: an address
for a *piece* of a document that survives every edit; links that run both
ways; and transclusion — one document showing another's text by reference,
live, not by copy. The W3C Web Annotation stack (`TextQuoteSelector`,
`TextPositionSelector`, `RangeSelector`, fuzzy re-anchoring, a whole
working group 2014–2017) exists because the web has no such address: it
addresses documents, and documents change under the link. Every selector
is a heuristic for finding afterwards what you meant.

v0 has the address. Every character is an atom with a permanent identity,
`Pos { event, offset }`, and the weave turns identity into a position in
O(log n) whatever has happened since. This crate shows what follows from
that, in a few hundred lines that change nothing in v0, one test per claim.

    cd v0/xanadu/transclusion && cargo test

## The claims, each a test

| # | claim | test | what it does *not* show |
|---|---|---|---|
| 1 | An address is not a position: insert before it, delete after it, it still names the same characters | `an_address_survives_edits_around_it` | — |
| 2 | An address follows its **atoms**, not its document: move the run to another file and the citation resolves there | `an_address_follows_its_atoms_to_another_document` | — (this is the one no selector can do) |
| 3 | A deleted atom in a span leaves a hole that knows who deleted it; nothing disappears | `a_deleted_atom_leaves_a_hole_with_provenance` | provenance is a scan of the log here; a store would index it |
| 4 | Links run both ways: "who cites this character" is a lookup, not a second structure | `links_run_both_ways` | the lookup is linear in the citations; an index is routine |
| 5 | A quote is live both ways: edit the source and the quote shows it; type inside the quote and the source changes | `a_quote_is_live_in_both_directions` | whether that is what you *want* — open question 1 |
| 6 | Typing at a quote's edge stays in the citing document | `typing_at_a_quotes_edge_stays_in_the_citing_document` | — |
| 7 | A citation knows what it said *then*: it carries its frontier, and staleness is a computation | `a_citation_knows_what_it_said_then` | the cost: "then" is a full replay per frontier |
| 8 | A view is a function of the events: reopen from the log, same text; replicas agree | `a_view_is_a_function_of_the_events` | only what v0's convergence already guarantees |
| 9 | A span whose ends are moved past each other is reported torn, with a reason | `a_span_whose_ends_cross_is_torn` | what it *should* mean — open question 2 |

Claims 1–4 and 7–8 are Xanadu's **addressing**, and they cost nothing:
the identity was already there (v0 ADR-0012; `theory/FORMAL.md` 3.7,
monotone on atoms, and 3.3a, order is world-independent except under
moves). Claims 5, 6 and 9 are Xanadu's **document model**, and the spike
takes a position on each so that the position can be argued with.

## The three decisions the spike makes

**A quote is an atom of the citing document.** Not a marker between two
characters: a real character (`U+FFFC`, the object replacement character)
with an identity, whose *content* is a span elsewhere. That is what makes
"before the quote" and "after the quote" different places (claim 6),
lets a quote be deleted like any character, and makes the citing document
a plain v0 document — the table from quote-atom to span is the only thing
outside the log, and it could be an op (`VISION.md`: `Cite` as the first
link-only op).

**An edit inside a quote is an edit to the atoms.** So it happens in every
document that shows them (claim 5). This is Xanadu's answer and it is
coherent, but it means a citing document is mutable through its quotes.
The alternative — copy-on-write, where typing inside a quote forks the
span into the citing document — is the other coherent answer. Which one a
document wants is a per-quote choice, and the spike does not make it for
you. **Open question 1.**

**A span is its two ends.** Cheap, and enough while the ends stay in one
document in order. When a move puts the last before the first, or the
ends in different documents, the spike says *torn* and why (claim 9)
rather than guessing. Whether a span should instead be the set of its
atoms (surviving any move, at the cost of size) is **open question 2**.

## After review (rubric, the FML author, 2026-09-28)

Ran on a clean machine, nine of nine. Three refinements taken, recorded
here so the code is read with them:

- **The quote atom is FML's island.** `⟨$… ⟩` is one Dyck token in the
  text with its own identity, so before and after the quote are outside
  the pair: claim 6 is a consequence of the brackets. `U+FFFC` here is the
  placeholder until that spelling exists.
- **Open question 1 is decided by authority, not by a flag.** Typing
  inside a quote is a write on the source's atoms; it goes through iff the
  writer holds the write capability on that mount, and forks the span into
  the citing document otherwise (copy-on-write as the consequence of
  refusal). Claim 5 is the case where the capability is held; its twin,
  the refused case, waits for capabilities in v0.
- **Open question 2: torn is a hole is a task.** The reason string is the
  hole's reason; the reader sees it and someone resolves it. A span as the
  set of its atoms would hide the tear.

Spelling agreed: an atom is `$⟨event⟩/⟨offset⟩`, a span
`{ from $e/3 to $f/7 kind … at ⟨frontier heads⟩ }`. The gate for standoff
records in FML is `Cite` as a link-only op in v0's log; that is the next
step here.

## What this is not

- **Not universal.** It holds for what is in the poset. The open web's
  problem is not solved; its assumption is dropped. To cite an external
  page you import it, and then it has atoms.
- **Not the W3C model, deliberately.** No selectors, no re-anchoring, no
  fuzzy matching: the target has identity, so there is nothing to find.
- **Not micropayments or transcopyright.** Those are separate from the
  mechanics and the mechanics do not need them.
- **Not stored.** Spans, citations and views live in memory here. Putting
  them in the log is a design step (deps versus links: a citation must not
  drag two hundred lines into a closure), not a mechanical one.
- **Not fast for "then".** A citation's frontier is replayed from scratch.
  v1's `goto` (`theory/THEORY.org`, Worlds) is what makes that incremental.
