//! Each test is one claim from the README, in the order the README makes
//! them. What a test asserts is what the spike shows; what it does not
//! assert, the README says is open.

use v0::event::EventLog;
use v0::op::{Anchor, EventId, NodeId, Op, Pos, ReplicaId, Side};
use v0::sumtree::Bias;
use v0::weave::Chars;
use xanadu_transclusion::{cited_by, Cite, Docs, Hole, Resolved, Span, View, QUOTE};

const R: ReplicaId = ReplicaId(1);

/// A file with one typed run of text. Returns the node and the run.
fn file(docs: &mut Docs, seq: &mut u32, name: &str, text: &str) -> (NodeId, EventId) {
    let node = docs.create(R, seq, name);
    let run = docs.append(R, seq, Op::Insert { parent: Anchor::DocStart(node), side: Side::Right, text: text.into() });
    (node, run)
}

fn edit(docs: &mut Docs, seq: &mut u32, ops: Vec<Op>) {
    for op in ops {
        docs.append(R, seq, op);
    }
}

/// "one two three", with "two" as the address under test.
fn one_two_three(docs: &mut Docs, seq: &mut u32) -> (NodeId, EventId, Span) {
    let (a, run) = file(docs, seq, "a.txt", "one two three");
    let span = Span::of(docs, a, Chars(4)..Chars(7));
    assert_eq!(span.text(docs).unwrap(), "two", "vacuity: the address names the word");
    (a, run, span)
}

/// 1. An address is not a position. Insert before it, delete after it: the
/// characters it names are the same characters, and it still says "two".
#[test]
fn an_address_survives_edits_around_it() {
    let (mut docs, mut seq) = (Docs::open(EventLog::default()), 1);
    let (a, _, span) = one_two_three(&mut docs, &mut seq);
    let op = docs.weave(a).insert_op(Chars(0), "zero ");
    edit(&mut docs, &mut seq, vec![op]);
    let ops = docs.weave(a).delete_ops(Chars(12)..Chars(18));
    edit(&mut docs, &mut seq, ops);
    assert_eq!(docs.text(a), "zero one two");
    assert_eq!(span.text(&docs).unwrap(), "two");
}

/// 2. An address follows its atoms, not its document. Move the whole run to
/// another file: the citation resolves there, unchanged. No text anchor and
/// no selector can do this; only identity can.
#[test]
fn an_address_follows_its_atoms_to_another_document() {
    let (mut docs, mut seq) = (Docs::open(EventLog::default()), 1);
    let (a, run, span) = one_two_three(&mut docs, &mut seq);
    let c = docs.create(R, &mut seq, "c.txt");
    edit(&mut docs, &mut seq, vec![Op::MoveRun { target: run, parent: Anchor::DocStart(c), side: Side::Right }]);
    assert_eq!(docs.text(a), "");
    assert_eq!(docs.text(c), "one two three");
    match span.resolve(&docs) {
        Resolved::Text { node, text, .. } => {
            assert_eq!(node, c);
            assert_eq!(text, "two");
        }
        torn => panic!("{torn:?}"),
    }
}

/// 3. Deleting inside a span leaves a hole that knows who made it. The
/// address still resolves; nothing disappears, it dies with provenance.
#[test]
fn a_deleted_atom_leaves_a_hole_with_provenance() {
    let (mut docs, mut seq) = (Docs::open(EventLog::default()), 1);
    let (a, _, span) = one_two_three(&mut docs, &mut seq);
    let ops = docs.weave(a).delete_ops(Chars(4)..Chars(5)); // the "t"
    let del = docs.append(R, &mut seq, ops.into_iter().next().unwrap());
    assert_eq!(docs.text(a), "one wo three");
    match span.resolve(&docs) {
        Resolved::Text { text, holes, .. } => {
            assert_eq!(text, "wo");
            assert_eq!(holes, vec![Hole { at: span.first, by: Some(del) }]);
        }
        torn => panic!("{torn:?}"),
    }
}

/// 4. Links run both ways. "What cites this character" is a lookup over the
/// citations, after any edits; Xanadu's two-way links are not a second
/// structure.
#[test]
fn links_run_both_ways() {
    let (mut docs, mut seq) = (Docs::open(EventLog::default()), 1);
    let (a, _, span) = one_two_three(&mut docs, &mut seq);
    let cites = vec![Cite::now(&docs, span)];
    let op = docs.weave(a).insert_op(Chars(0), "zero ");
    edit(&mut docs, &mut seq, vec![op]);
    let w = docs.weave(a).pos_at(Chars(10), Bias::Right).unwrap(); // the "w" of two
    let o = docs.weave(a).pos_at(Chars(5), Bias::Right).unwrap(); // the "o" of one
    assert_eq!(cited_by(&cites, &docs, w).len(), 1);
    assert_eq!(cited_by(&cites, &docs, o).len(), 0);
}

/// 5. A quote is live, both ways. Edit the source: the quote shows it. Type
/// inside the quote: the source changes, and both documents show it. This
/// is transclusion, and it is also open question 1 -- the citing document
/// is mutable through its quotes, by design.
#[test]
fn a_quote_is_live_in_both_directions() {
    let (mut docs, mut seq) = (Docs::open(EventLog::default()), 1);
    let (a, _, span) = one_two_three(&mut docs, &mut seq);
    let (b, _) = file(&mut docs, &mut seq, "b.txt", "Quote: .");
    let mut view = View::new(b);
    view.quote(&mut docs, R, &mut seq, Chars(7), span);
    assert_eq!(docs.text(b), format!("Quote: {QUOTE}."), "the quote is an atom of b");
    assert_eq!(view.text(&docs), "Quote: two.");
    let op = docs.weave(a).insert_op(Chars(5), "-");
    edit(&mut docs, &mut seq, vec![op]);
    assert_eq!(view.text(&docs), "Quote: t-wo.", "an edit in the source shows in the quote");
    view.insert(&mut docs, R, &mut seq, Chars(8), "X"); // between t and -
    assert_eq!(docs.text(a), "one tX-wo three", "an edit in the quote lands in the source");
    assert_eq!(view.text(&docs), "Quote: tX-wo.");
}

/// 6. Typing at a quote's edge stays in the citing document. Because the
/// quote is an atom of b, before and after it are different places, and
/// neither is inside the source.
#[test]
fn typing_at_a_quotes_edge_stays_in_the_citing_document() {
    let (mut docs, mut seq) = (Docs::open(EventLog::default()), 1);
    let (a, _, span) = one_two_three(&mut docs, &mut seq);
    let (b, _) = file(&mut docs, &mut seq, "b.txt", "Quote: .");
    let mut view = View::new(b);
    view.quote(&mut docs, R, &mut seq, Chars(7), span);
    view.insert(&mut docs, R, &mut seq, Chars(7), "(");
    view.insert(&mut docs, R, &mut seq, Chars(11), ")");
    assert_eq!(view.text(&docs), "Quote: (two).");
    assert_eq!(docs.text(b), format!("Quote: ({QUOTE})."));
    assert_eq!(docs.text(a), "one two three", "the source is untouched");
}

/// 7. A citation knows what it said then. It carries the frontier it was made
/// at; "then" is that world replayed. The difference to "now" is the
/// staleness of a comment, computed, not guessed.
#[test]
fn a_citation_knows_what_it_said_then() {
    let (mut docs, mut seq) = (Docs::open(EventLog::default()), 1);
    let (a, _, span) = one_two_three(&mut docs, &mut seq);
    let cite = Cite::now(&docs, span);
    assert_eq!(cite.stale(&docs), None);
    let op = docs.weave(a).insert_op(Chars(5), "-");
    edit(&mut docs, &mut seq, vec![op]);
    assert_eq!(cite.stale(&docs), Some(("two".into(), "t-wo".into())));
}

/// 8. A view is a function of the events. Reopen everything from the log:
/// same addresses, same quotes, same text. Two replicas with the same events
/// show the same view -- v0's convergence carries over unchanged.
#[test]
fn a_view_is_a_function_of_the_events() {
    let (mut docs, mut seq) = (Docs::open(EventLog::default()), 1);
    let (a, _, span) = one_two_three(&mut docs, &mut seq);
    let (b, _) = file(&mut docs, &mut seq, "b.txt", "Quote: .");
    let mut view = View::new(b);
    view.quote(&mut docs, R, &mut seq, Chars(7), span);
    let op = docs.weave(a).insert_op(Chars(5), "-");
    edit(&mut docs, &mut seq, vec![op]);
    let again = Docs::open(docs.log.clone());
    assert_eq!(view.text(&again), view.text(&docs));
    assert_eq!(view.text(&again), "Quote: t-wo.");
}

/// 9. Open question 2, pinned as it is: a span whose ends are moved past each
/// other no longer names a stretch. The spike reports it as torn, with the
/// reason, rather than pretending.
#[test]
fn a_span_whose_ends_cross_is_torn() {
    let (mut docs, mut seq) = (Docs::open(EventLog::default()), 1);
    let (a, r1) = file(&mut docs, &mut seq, "a.txt", "alpha ");
    let op = docs.weave(a).insert_op(Chars(6), "beta");
    let r2 = docs.append(R, &mut seq, op);
    let span = Span { first: Pos { event: r1, offset: 0 }, last: Pos { event: r2, offset: 3 } };
    assert_eq!(span.text(&docs).unwrap(), "alpha beta");
    edit(&mut docs, &mut seq, vec![Op::MoveRun {
        target: r2, parent: Anchor::At(Pos { event: r1, offset: 0 }), side: Side::Left,
    }]);
    assert_eq!(docs.text(a), "betaalpha ");
    assert_eq!(span.text(&docs), Err("the ends have moved past each other"));
}
