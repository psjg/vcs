//! The live weave against the batch replay (the sum tree has its own file).
//!
//! The oracle throughout is `replay`: whatever the weave says about a set of
//! events, materialising the set from scratch must agree.

use proptest::prelude::*;
use v0::capture;
use v0::event::EventLog;
use v0::op::{Anchor, EventId, NodeId, NodeKind, Op, Pos, ReplicaId, Side};
use v0::replay;
use v0::sumtree::{Bias, Summary};
use v0::weave::{Chars, Locator, Metrics, PointUtf16, Weave, MAX_FRAGMENT};

/// Save each text in turn on one replica, capturing the diff the way `record`
/// does. Returns the log with its file's node.
fn saves(log: &mut EventLog, node: NodeId, replica: u64, texts: &[String]) {
    let r = ReplicaId(replica);
    for text in texts {
        let all: Vec<EventId> = log.events().keys().copied().collect();
        let before = replay::materialise_atoms(&all, log).remove("f").unwrap_or_default();
        let mut seq = log.lamport_next();
        let cap = capture::from_save(&before, node, text, r, seq, log);
        log.append_batch(r, &mut seq, cap.ops);
    }
}

/// A shared start, then two replicas editing apart, then their union: runs
/// nested in runs, tombstones inside and between them.
fn history(shared: &[String], a: &[String], b: &[String]) -> (EventLog, NodeId) {
    let mut log = EventLog::default();
    let mut seq = 1;
    let node = NodeId(EventId { seq, replica: ReplicaId(1) });
    log.append(ReplicaId(1), &mut seq, Op::Create { node, parent: Op::ROOT, name: "f".into(), kind: NodeKind::File });
    saves(&mut log, node, 1, shared);
    let mut other = log.clone();
    saves(&mut log, node, 1, a);
    saves(&mut other, node, 2, b);
    log.extend(other.events().values().cloned());
    (log, node)
}

fn texts() -> impl Strategy<Value = Vec<String>> {
    let word = prop_oneof![
        Just("a"), Just("bb"), Just(" "), Just("\n"), Just("\r\n"), Just("\r"), Just("é"), Just("😀"), Just("zz")
    ];
    proptest::collection::vec(proptest::collection::vec(word, 0..12).prop_map(|w| w.concat()), 0..4)
}

fn built(log: &EventLog, node: NodeId) -> (Weave, Vec<(v0::replay::Atom, bool)>) {
    let all: Vec<EventId> = log.events().keys().copied().collect();
    built_from(&all, log, node)
}

/// A weave opened on the subset `events` of the log.
fn built_from(events: &[EventId], log: &EventLog, node: NodeId) -> (Weave, Vec<(v0::replay::Atom, bool)>) {
    let weaves = replay::weaves(events, log);
    let walk = weaves.walks.get(&node).cloned().unwrap_or_default();
    (Weave::from_walk(node, &weaves, log), walk)
}

/// Move runs of the document about. Each pick names a run and an anchor
/// among *all* of them -- visible or hidden -- plus the document start on
/// either side, and positions one past a run's end. So picks hide runs (the
/// start's left, past an end, into a hidden run), bring hidden runs back, and
/// land runs inside their own subtree, which replay skips.
fn moves(log: &mut EventLog, docs: &[NodeId], replica: u64, picks: &[(usize, usize, bool)]) {
    for (t, a, left) in picks {
        let runs: Vec<(EventId, u32)> = log
            .events()
            .iter()
            .filter_map(|(id, e)| match &e.op {
                Op::Insert { text, .. } => Some((*id, text.chars().count() as u32)),
                _ => None,
            })
            .collect();
        if runs.is_empty() {
            continue;
        }
        let target = runs[t % runs.len()].0;
        let side = if *left { Side::Left } else { Side::Right };
        let anchors: Vec<Anchor> = docs
            .iter()
            .map(|n| Anchor::DocStart(*n))
            .chain(runs.iter().flat_map(|(r, n)| (0..=*n).map(|offset| Anchor::At(Pos { event: *r, offset }))))
            .collect();
        let parent = anchors[a % anchors.len()];
        let mut seq = log.lamport_next();
        log.append(ReplicaId(replica), &mut seq, Op::MoveRun { target, parent, side });
    }
}

/// One replica's part: saves, then moves, then more saves on top.
#[derive(Clone, Debug)]
struct Script {
    before: Vec<String>,
    picks: Vec<(usize, usize, bool)>,
    after: Vec<String>,
}

fn script() -> impl Strategy<Value = Script> {
    (texts(), proptest::collection::vec((0usize..1000, 0usize..1000, any::<bool>()), 0..4), texts())
        .prop_map(|(before, picks, after)| Script { before, picks, after })
}

/// Like [`history`], plus a second document, `g`, and each replica moving
/// runs concurrently -- within `f`, between `f` and `g`, into hiding and back.
/// Returns both documents.
fn history_moving(shared: &[String], a: &Script, b: &Script) -> (EventLog, [NodeId; 2]) {
    let (mut log, f) = history(shared, &[], &[]);
    let mut seq = log.lamport_next();
    let g = NodeId(EventId { seq, replica: ReplicaId(1) });
    log.append(ReplicaId(1), &mut seq, Op::Create { node: g, parent: Op::ROOT, name: "g".into(), kind: NodeKind::File });
    log.append(ReplicaId(1), &mut seq, Op::Insert { parent: Anchor::DocStart(g), side: Side::Right, text: "gee\n".into() });
    let mut other = log.clone();
    for (log, replica, s) in [(&mut log, 1, a), (&mut other, 2, b)] {
        saves(log, f, replica, &s.before);
        moves(log, &[f, g], replica, &s.picks);
        saves(log, f, replica, &s.after);
    }
    log.extend(other.events().values().cloned());
    (log, [f, g])
}

/// LSP's reading of a text: lines broken by `\r\n`, `\r` or `\n`, columns in
/// UTF-16. Written the obvious way, as the reference for `Metrics`.
fn reference(text: &str) -> (u32, u32) {
    let (mut lines, mut col) = (0u32, 0u32);
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\r' => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                lines += 1;
                col = 0;
            }
            '\n' => {
                lines += 1;
                col = 0;
            }
            _ => col += c.len_utf16() as u32,
        }
    }
    (lines, col)
}

fn text() -> impl Strategy<Value = String> {
    proptest::collection::vec(prop_oneof![Just('a'), Just('\r'), Just('\n'), Just('é'), Just('😀')], 0..40)
        .prop_map(|v| v.into_iter().collect())
}

proptest! {
    /// Metrics are a monoid homomorphism: the metrics of a concatenation are
    /// the sum of the parts' -- wherever the cut falls, a `\r\n` torn in two
    /// included -- and they read the text the way LSP does.
    #[test]
    fn metrics_of_a_concatenation_is_the_sum(t in text(), cut in 0usize..41) {
        let cut = t.char_indices().map(|(i, _)| i).chain([t.len()]).nth(cut).unwrap_or(t.len());
        let (a, b) = t.split_at(cut);
        let mut sum = Metrics::of(a);
        sum.add(&Metrics::of(b));
        let whole = Metrics::of(&t);
        prop_assert_eq!(sum, whole);
        prop_assert_eq!((whole.lines, whole.last_utf16), reference(&t));
        let tail = t.rsplit(['\r', '\n']).next().unwrap_or("");
        prop_assert_eq!(whole.last_chars as usize, tail.chars().count());
        prop_assert_eq!(whole.bytes, t.len());
        prop_assert_eq!(whole.chars, t.chars().count());
        prop_assert_eq!(whole.utf16, t.encode_utf16().count());
    }
}

/// A causal order of the log, picked by `choices`: Kahn's algorithm over
/// each event's causal parents, taking the ready event the next choice
/// points at. Every causal order is reachable.
fn causal_order(log: &EventLog, choices: &[usize]) -> Vec<EventId> {
    let mut waiting: std::collections::BTreeMap<EventId, usize> = log
        .events()
        .values()
        .map(|e| (e.id, e.parents.iter().filter(|p| log.events().contains_key(p)).count()))
        .collect();
    let mut ready: Vec<EventId> = waiting.iter().filter(|(_, n)| **n == 0).map(|(id, _)| *id).collect();
    waiting.retain(|_, n| *n > 0);
    let mut order = Vec::new();
    let mut c = choices.iter().cycle();
    while !ready.is_empty() {
        let i = c.next().copied().unwrap_or(0) % ready.len();
        let e = ready.swap_remove(i);
        order.push(e);
        for child in log.events().values().filter(|x| x.parents.contains(&e)) {
            if let Some(n) = waiting.get_mut(&child.id) {
                *n -= 1;
                if *n == 0 {
                    waiting.remove(&child.id);
                    ready.push(child.id);
                }
            }
        }
    }
    assert!(waiting.is_empty(), "the log's causal graph is acyclic and closed");
    order
}

/// Check `w` against the oracle: the replay's text, and every character of
/// the replay's walk -- tombstones too -- at the offset the walk puts it.
fn agrees_with_replay(w: &Weave, log: &EventLog, node: NodeId) -> Result<(), TestCaseError> {
    let (bulk, walk) = built(log, node);
    prop_assert_eq!(w.text(), bulk.text());
    // The cached summaries, not just the items: what line/column answers
    // are read from after every cut, split and append.
    prop_assert_eq!(w.metrics(), Metrics::of(&w.text()));
    let mut visible = 0usize;
    for (atom, alive) in &walk {
        prop_assert_eq!(w.offset_of(atom.id), Some((Chars(visible), *alive)), "at {:?}", atom);
        visible += usize::from(*alive);
    }
    for f in w.fragments() {
        prop_assert!((1..=MAX_FRAGMENT).contains(&(f.str().chars().count() as u32)));
    }
    Ok(())
}

/// Apply `op` and check what `apply` reported: its edits, played in order on
/// the old text, give the new one; and applying it again changes nothing
/// (sync re-delivers).
fn apply_checked(w: &mut Weave, id: EventId, op: &Op) -> Result<(), TestCaseError> {
    let mut model: Vec<char> = w.text().chars().collect();
    let edits = w.apply(id, op);
    let now: Vec<char> = w.text().chars().collect();
    for e in &edits {
        model.splice(e.old.clone(), e.text.chars());
    }
    prop_assert_eq!(&model, &now, "edits {:?} replay to the new text", edits);
    prop_assert_eq!(w.apply(id, op), Vec::new(), "a repeat is a no-op");
    prop_assert_eq!(w.text().chars().collect::<Vec<_>>(), now);
    Ok(())
}

proptest! {
    /// Convergence: folding `Weave::apply` over *any* causal order of the
    /// events gives what `replay` materialises -- and, character by
    /// character, what `from_walk` builds.
    #[test]
    fn applying_events_in_any_causal_order_matches_replay(
        shared in texts(), a in texts(), b in texts(),
        choices in proptest::collection::vec(0usize..64, 1..64),
    ) {
        let (log, node) = history(&shared, &a, &b);
        let mut w = Weave::new(node);
        for id in causal_order(&log, &choices) {
            apply_checked(&mut w, id, &log.events()[&id].op)?;
        }
        agrees_with_replay(&w, &log, node)?;
    }

    /// Live editing: `insert_op` and `delete_ops` at random places, each op
    /// appended to the log and applied. The weave matches a plain string
    /// edited the same way, and replaying the log agrees.
    #[test]
    fn live_ops_reproduce_what_a_save_would(
        shared in texts(),
        edits in proptest::collection::vec((0usize..64, 0usize..4, "[ab\n😀]{0,3}"), 1..24),
    ) {
        let (mut log, node) = history(&shared, &[], &[]);
        let (mut w, _) = built(&log, node);
        let mut model: Vec<char> = w.text().chars().collect();
        let r = ReplicaId(3);
        for (at, del, ins) in edits {
            let at = at % (model.len() + 1);
            let del = del.min(model.len() - at);
            let mut ops = w.delete_ops(Chars(at)..Chars(at + del));
            for pair in ops.windows(2) {
                if let (Op::Delete { target: t, range: r }, Op::Delete { target: u, range: q }) = (&pair[0], &pair[1]) {
                    prop_assert!(!(t == u && r.1 == q.0), "{:?} could have been one delete", pair);
                }
            }
            model.drain(at..at + del);
            for op in ops.drain(..) {
                let mut seq = log.lamport_next();
                let id = log.append(r, &mut seq, op.clone());
                apply_checked(&mut w, id, &op)?;
            }
            if !ins.is_empty() {
                let op = w.insert_op(Chars(at), &ins);
                let mut seq = log.lamport_next();
                let id = log.append(r, &mut seq, op.clone());
                apply_checked(&mut w, id, &op)?;
                model.splice(at..at, ins.chars());
            }
            prop_assert_eq!(w.text(), model.iter().collect::<String>());
        }
        agrees_with_replay(&w, &log, node)?;
    }
}

proptest! {
    /// A weave built from the walk reads as the replay does.
    #[test]
    fn from_walk_reads_like_replay(shared in texts(), a in texts(), b in texts()) {
        let (log, node) = history(&shared, &a, &b);
        let all: Vec<EventId> = log.events().keys().copied().collect();
        let expect: String = replay::materialise_atoms(&all, &log)
            .remove("f").unwrap_or_default().iter().map(|a| a.ch).collect();
        let (w, _) = built(&log, node);
        prop_assert_eq!(w.metrics(), Metrics::of(&expect));
        prop_assert_eq!(w.text(), expect);

        // Fragments are maximal and bounded: two neighbours could not have
        // been one -- another run, another visibility, a gap in the run, or
        // the first already full.
        let frags: Vec<_> = w.fragments().collect();
        for f in &frags {
            let n = f.str().chars().count() as u32;
            prop_assert!((1..=MAX_FRAGMENT).contains(&n));
        }
        for pair in frags.windows(2) {
            let (f, g) = (pair[0], pair[1]);
            let f_len = f.str().chars().count() as u32;
            let mergeable = f.run == g.run && f.visible == g.visible && f.start + f_len == g.start && f_len < MAX_FRAGMENT;
            prop_assert!(!mergeable, "{:?} and {:?} should be one fragment", f, g);
        }
    }

    /// Every character, alive or dead, resolves to the visible offset the walk
    /// puts it at; every visible offset resolves back to its character; and
    /// every offset's LSP point converts back to the offset -- except inside a
    /// `\r\n`, which LSP does not address and which lands after the `\n`.
    #[test]
    fn coordinates_round_trip(shared in texts(), a in texts(), b in texts()) {
        let (log, node) = history(&shared, &a, &b);
        let (w, walk) = built(&log, node);
        let mut visible = 0usize;
        for (atom, alive) in &walk {
            prop_assert_eq!(w.offset_of(atom.id), Some((Chars(visible), *alive)));
            if *alive {
                prop_assert_eq!(w.pos_at(Chars(visible), Bias::Right), Some(atom.id));
                prop_assert_eq!(w.pos_at(Chars(visible + 1), Bias::Left), Some(atom.id));
                visible += 1;
            }
        }
        prop_assert_eq!(w.pos_at(Chars(visible), Bias::Right), None);
        prop_assert_eq!(w.pos_at(Chars(0), Bias::Left), None);
        // What the weave never saw: an unknown event, or past a run's end.
        let unknown = EventId { seq: u32::MAX, replica: ReplicaId(9) };
        prop_assert_eq!(w.offset_of(Pos { event: unknown, offset: 0 }), None);
        if let Some((atom, _)) = walk.iter().find(|(a, _)| a.id.offset == 0) {
            let len = walk.iter().filter(|(b, _)| b.id.event == atom.id.event).count() as u32;
            prop_assert_eq!(w.offset_of(Pos { event: atom.id.event, offset: len }), None);
        }

        let text = w.text();
        let chars: Vec<char> = text.chars().collect();
        for k in 0..=chars.len() {
            let prefix: String = chars[..k].iter().collect();
            let m = Metrics::of(&prefix);
            let point = w.point_of_offset(Chars(k));
            prop_assert_eq!(point, PointUtf16 { line: m.lines, col: m.last_utf16 });
            let mid_crlf = k > 0 && chars[k - 1] == '\r' && chars.get(k) == Some(&'\n');
            let back = if mid_crlf { k + 1 } else { k };
            prop_assert_eq!(w.offset_of_point(point), Chars(back));
        }
    }

    /// A column past the end of its line clamps to the line's end; a line past
    /// the end of the text clamps to the end.
    #[test]
    fn points_past_the_end_clamp(shared in texts()) {
        let (log, node) = history(&shared, &[], &[]);
        let (w, _) = built(&log, node);
        let text = w.text();
        let m = w.metrics();
        prop_assert_eq!(w.offset_of_point(PointUtf16 { line: m.lines + 1, col: 0 }), Chars(m.chars));
        let first_break = text.chars().position(|c| c == '\r' || c == '\n').unwrap_or(m.chars);
        prop_assert_eq!(w.offset_of_point(PointUtf16 { line: 0, col: u32::MAX }), Chars(first_break));
    }

    /// Locators are dense: inserting anywhere, any number of times, keeps them
    /// strictly ordered, and never reaches the sentinels.
    #[test]
    fn locators_are_dense(slots in proptest::collection::vec(0usize..1000, 1..300)) {
        let mut keys: Vec<Locator> = Vec::new();
        for s in slots {
            let i = s % (keys.len() + 1);
            let lo = if i == 0 { Locator::min() } else { keys[i - 1].clone() };
            let hi = keys.get(i).cloned().unwrap_or_else(Locator::max);
            let k = Locator::between(&lo, &hi);
            prop_assert!(lo < k && k < hi, "{:?} < {:?} < {:?}", lo, k, hi);
            keys.insert(i, k);
        }
    }
}

/// The adversarial spot for dense keys: always inserting right after the
/// same key, and right before it, many times over.
#[test]
fn locators_stay_dense_at_one_spot() {
    let mut lo = Locator::min();
    let hi = Locator::between(&Locator::min(), &Locator::max());
    for _ in 0..2000 {
        let k = Locator::between(&lo, &hi);
        assert!(lo < k && k < hi);
        lo = k;
    }
    let mut hi2 = hi.clone();
    for _ in 0..2000 {
        let k = Locator::between(&Locator::min(), &hi2);
        assert!(Locator::min() < k && k < hi2);
        hi2 = k;
    }
}

/// A long paste is one run cut into full fragments, not one fragment per
/// character and not one unbounded fragment.
#[test]
fn a_long_run_is_cut_at_the_fragment_cap() {
    let text = "x".repeat(1000);
    let (log, node) = history(&[text], &[], &[]);
    let (w, _) = built(&log, node);
    let sizes: Vec<usize> = w.fragments().map(|f| f.str().chars().count()).collect();
    assert_eq!(sizes.len(), 1000usize.div_ceil(MAX_FRAGMENT as usize), "{sizes:?}");
    assert!(sizes[..sizes.len() - 1].iter().all(|n| *n == MAX_FRAGMENT as usize));
}


/// What replay would never walk, `apply` never places: an insert into another
/// document, on the left of a document's start, or anchored past its run's
/// end. And a paste longer than a fragment lands whole.
#[test]
fn apply_ignores_what_replay_ignores_and_places_long_pastes() {
    let (mut log, node) = history(&["ab".into()], &[], &[]);
    let (mut w, _) = built(&log, node);
    let run = *log.events().keys().find(|id| matches!(log.events()[id].op, Op::Insert { .. })).unwrap();
    let other = NodeId(EventId { seq: 99, replica: ReplicaId(9) });
    let ignored = [
        Op::Insert { parent: v0::op::Anchor::DocStart(other), side: v0::op::Side::Right, text: "x".into() },
        Op::Insert { parent: v0::op::Anchor::DocStart(node), side: v0::op::Side::Left, text: "x".into() },
        Op::Insert { parent: v0::op::Anchor::At(Pos { event: run, offset: 2 }), side: v0::op::Side::Right, text: "x".into() },
    ];
    let r = ReplicaId(3);
    for op in ignored {
        let mut seq = log.lamport_next();
        let id = log.append(r, &mut seq, op.clone());
        assert_eq!(w.apply(id, &op), Vec::new(), "{op:?}");
    }
    let paste = "0123456789".repeat(40);
    let op = w.insert_op(Chars(1), &paste);
    let mut seq = log.lamport_next();
    let id = log.append(r, &mut seq, op.clone());
    assert_eq!(w.apply(id, &op).len(), 1);
    assert_eq!(w.text(), format!("a{paste}b"));
    agrees_with_replay(&w, &log, node).unwrap();
}

proptest! {
    /// Moves converge: concurrent moves on two replicas -- cyclic ones
    /// included, which replay skips in EventId order -- folded in any causal
    /// order give what replay gives. A smaller-id move arriving after a larger
    /// one is exactly the case Kleppmann's undo/redo exists for.
    #[test]
    fn moves_in_any_causal_order_match_replay(
        shared in texts(), a in script(), b in script(),
        choices in proptest::collection::vec(0usize..64, 1..64),
    ) {
        let (log, docs) = history_moving(&shared, &a, &b);
        let mut ws = docs.map(Weave::new);
        for id in causal_order(&log, &choices) {
            for w in &mut ws {
                apply_checked(w, id, &log.events()[&id].op)?;
            }
        }
        for (w, node) in ws.iter().zip(docs) {
            agrees_with_replay(w, &log, node)?;
        }
    }

    /// A weave opened on part of the history -- moves and all -- and fed the
    /// rest live ends where replay of the whole does: `from_walk` keeps
    /// everything a later move needs.
    #[test]
    fn opening_midway_then_applying_the_rest_matches_replay(
        shared in texts(), a in script(), b in script(),
        choices in proptest::collection::vec(0usize..64, 1..64),
        cut in 0usize..200,
    ) {
        let (log, docs) = history_moving(&shared, &a, &b);
        let order = causal_order(&log, &choices);
        let cut = cut % (order.len() + 1);
        for node in docs {
            let (mut w, _) = built_from(&order[..cut], &log, node);
            for id in &order[cut..] {
                apply_checked(&mut w, *id, &log.events()[id].op)?;
            }
            agrees_with_replay(&w, &log, node)?;
        }
    }
}

/// The undo/redo case, pinned: two replicas each move one of two sibling runs
/// under the other. Together that is a cycle; replay applies the smaller move
/// id and skips the larger. Applied either way round, the weave must agree --
/// which means undoing the larger move when the smaller one arrives second.
/// (The runs must be siblings: a run already under the other makes the
/// smaller move the cyclic one in both orders, and nothing is ever undone.)
#[test]
fn crossing_moves_resolve_like_replay_in_either_order() {
    let (mut log, node) = history(&["one\ntwo\n".into()], &[], &[]);
    let x = built(&log, node).1.first().expect("the saved text is a run").0.id.event;
    let mut seq = log.lamport_next();
    let y = log.append(ReplicaId(1), &mut seq, Op::Insert {
        parent: Anchor::DocStart(node), side: Side::Right, text: "zero\n".into(),
    });
    let mut other = log.clone();
    let mut seq = log.lamport_next();
    let under_y = log.append(ReplicaId(1), &mut seq, Op::MoveRun {
        target: x, parent: Anchor::At(Pos { event: y, offset: 0 }), side: Side::Right,
    });
    let mut seq = other.lamport_next();
    let under_x = other.append(ReplicaId(2), &mut seq, Op::MoveRun {
        target: y, parent: Anchor::At(Pos { event: x, offset: 0 }), side: Side::Right,
    });
    log.extend(other.events().values().cloned());
    let base: Vec<EventId> = log.events().keys().copied().filter(|e| *e != under_y && *e != under_x).collect();

    for (first, second) in [(under_y, under_x), (under_x, under_y)] {
        let (mut w, _) = built_from(&base, &log, node);
        w.apply(first, &log.events()[&first].op);
        w.apply(second, &log.events()[&second].op);
        agrees_with_replay(&w, &log, node).unwrap();
        // Larger id second: cyclic, skipped, nothing moves. Smaller second:
        // it wins and undoes the larger -- two anchors change, the one case
        // that lays the document out afresh.
        assert_eq!(w.relaid, u32::from(second < first), "{first:?} then {second:?}");
    }
}

/// The move tests are only as good as what they generate. Sample the same
/// strategies and require each hard case to turn up: moves that change the
/// layout, moves replay skips as cyclic, and moves that arrive after a larger
/// one in the causal order (the undo/redo case).
#[test]
fn move_histories_reach_the_hard_cases() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut effective, mut cyclic, mut late, mut hid, mut revived) = (0, 0, 0, 0, 0);
    let (mut relocated, mut relaid) = (0, 0);
    for _ in 0..300 {
        let (shared, a, b, choices) = (texts(), script(), script(), proptest::collection::vec(0usize..64, 1..64))
            .new_tree(&mut runner)
            .unwrap()
            .current();
        let (log, [node, _]) = history_moving(&shared, &a, &b);
        let mut w = Weave::new(node);
        let mut max_move: Option<EventId> = None;
        for id in causal_order(&log, &choices) {
            let op = &log.events()[&id].op;
            if let Op::MoveRun { target, parent, .. } = op {
                if max_move.is_some_and(|m| m > id) {
                    late += 1;
                }
                max_move = max_move.max(Some(id));
                let before = w.text();
                let shown = |w: &Weave| w.offset_of(Pos { event: *target, offset: 0 }).is_some();
                let was = shown(&w);
                let edits = w.apply(id, op);
                if !edits.is_empty() && w.text() != before {
                    effective += 1;
                }
                match (was, shown(&w)) {
                    (true, false) => hid += 1,
                    (false, true) => revived += 1,
                    _ => {}
                }
                if matches!(parent, Anchor::At(p) if p.event == *target) {
                    cyclic += 1;
                }
            } else {
                w.apply(id, op);
            }
        }
        relocated += w.relocated;
        relaid += w.relaid;
    }
    let counts = format!(
        "effective {effective}, cyclic {cyclic}, late {late}, hid {hid}, revived {revived}; \
         relocated {relocated}, relaid {relaid}"
    );
    eprintln!("hard cases in 300 histories: {counts}");
    assert!(effective >= 30 && cyclic >= 10 && late >= 10 && hid >= 10 && revived >= 10, "{counts}");
    // Both of apply's paths for a move are exercised: one block carried, and
    // the fallback for a late move that changed more than one anchor. The
    // fallback is genuinely rare here (2 in 300 histories);
    // `crossing_moves_resolve_like_replay_in_either_order` pins it.
    assert!(relocated >= 30 && relaid >= 1, "{counts}");
}

/// A move can hang an old run in the middle of a younger one, and then it is
/// read *before* the rest of that run, not after. Placing an insert after the
/// younger run's subtree must know that.
#[test]
fn an_old_run_moved_into_a_young_one_is_read_before_its_rest() {
    let (mut log, node) = history(&[], &[], &[]);
    let r = ReplicaId(1);
    let mut seq = log.lamport_next();
    let old = log.append(r, &mut seq, Op::Insert { parent: Anchor::DocStart(node), side: Side::Right, text: "xyz".into() });
    let young = log.append(r, &mut seq, Op::Insert { parent: Anchor::DocStart(node), side: Side::Right, text: "abc".into() });
    log.append(r, &mut seq, Op::MoveRun { target: old, parent: Anchor::At(Pos { event: young, offset: 0 }), side: Side::Right });
    let (mut w, _) = built(&log, node);
    assert_eq!(w.text(), "axyzbc", "vacuity: the old run sits inside the young one");
    let op = Op::Insert { parent: Anchor::DocStart(node), side: Side::Right, text: "!".into() };
    let id = log.append(r, &mut seq, op.clone());
    w.apply(id, &op);
    assert_eq!(w.text(), "axyzbc!");
    agrees_with_replay(&w, &log, node).unwrap();
}

/// A removed file's weave still holds its text: a weave is about content, and
/// a restore brings the file back.
#[test]
fn a_removed_files_weave_keeps_its_text() {
    let (mut log, node) = history(&["kept\n".into()], &[], &[]);
    let mut seq = log.lamport_next();
    log.append(ReplicaId(1), &mut seq, Op::Remove { node });
    assert_eq!(built(&log, node).0.text(), "kept\n");
}

/// Opened while a half-deleted run is hidden, a weave can still bring it back
/// -- with its deleted half still deleted -- and into the other document.
#[test]
fn a_hidden_run_comes_back_after_opening_with_its_deletes() {
    let (mut log, f) = history(&["keep\n".into()], &[], &[]);
    let r = ReplicaId(1);
    let mut seq = log.lamport_next();
    let g = NodeId(EventId { seq, replica: r });
    log.append(r, &mut seq, Op::Create { node: g, parent: Op::ROOT, name: "g".into(), kind: NodeKind::File });
    let run = log.append(r, &mut seq, Op::Insert { parent: Anchor::DocStart(f), side: Side::Right, text: "abcdef".into() });
    log.append(r, &mut seq, Op::Delete { target: run, range: (1, 3) });
    log.append(r, &mut seq, Op::MoveRun { target: run, parent: Anchor::DocStart(f), side: Side::Left });
    let (mut wf, _) = built(&log, f);
    let (mut wg, _) = built(&log, g);
    assert_eq!(wf.text(), "keep\n", "vacuity: the run is hidden");
    let back = Op::MoveRun { target: run, parent: Anchor::DocStart(g), side: Side::Right };
    let id = log.append(r, &mut seq, back.clone());
    wf.apply(id, &back);
    wg.apply(id, &back);
    assert_eq!(wg.text(), "adef", "back, in the other document, deletes kept");
    agrees_with_replay(&wf, &log, f).unwrap();
    agrees_with_replay(&wg, &log, g).unwrap();
}

/// The mirror case: an old run hung on the *last* character of a young one is
/// read right after it -- at a run's end, every right child is, smaller or
/// not -- so an insert after the young run's subtree lands after the old run.
#[test]
fn an_old_run_moved_to_a_young_ones_end_is_read_after_it() {
    let (mut log, node) = history(&[], &[], &[]);
    let r = ReplicaId(1);
    let mut seq = log.lamport_next();
    let old = log.append(r, &mut seq, Op::Insert { parent: Anchor::DocStart(node), side: Side::Right, text: "xyz".into() });
    let young = log.append(r, &mut seq, Op::Insert { parent: Anchor::DocStart(node), side: Side::Right, text: "abc".into() });
    log.append(r, &mut seq, Op::MoveRun { target: old, parent: Anchor::At(Pos { event: young, offset: 2 }), side: Side::Right });
    let (mut w, _) = built(&log, node);
    assert_eq!(w.text(), "abcxyz", "vacuity: the old run hangs on the young one's end");
    let op = Op::Insert { parent: Anchor::DocStart(node), side: Side::Right, text: "!".into() };
    let id = log.append(r, &mut seq, op.clone());
    w.apply(id, &op);
    assert_eq!(w.text(), "abcxyz!");
    agrees_with_replay(&w, &log, node).unwrap();
}

/// Siblings of both ages on one character of a young run, arriving through
/// `apply` in either order. At `(r,k)` the right children older than `r` read
/// before the rest of the run and the younger ones after it, each group
/// sorted; at the run's last character all of them read after it, sorted.
/// Each move must find its place among the siblings already there. And a
/// move that shows nothing -- into the run's own subtree, or of a run wholly
/// deleted -- reports nothing.
#[test]
fn old_and_young_siblings_on_one_character_group_like_replay() {
    let (mut log, node) = history(&[], &[], &[]);
    let r = ReplicaId(1);
    let mut seq = log.lamport_next();
    let mut insert = |log: &mut EventLog, parent, text: &str| {
        log.append(r, &mut seq, Op::Insert { parent, side: Side::Right, text: text.into() })
    };
    let olds: Vec<EventId> = ["1", "2", "3", "4"].iter().map(|t| insert(&mut log, Anchor::DocStart(node), t)).collect();
    let young = insert(&mut log, Anchor::DocStart(node), "abcd");
    let mid = Anchor::At(Pos { event: young, offset: 1 });
    let end = Anchor::At(Pos { event: young, offset: 3 });
    insert(&mut log, mid, "Y");
    insert(&mut log, end, "Z");
    let base: Vec<EventId> = log.events().keys().copied().collect();
    let mut seq = log.lamport_next();
    let moves: Vec<EventId> = [(0, mid), (1, mid), (2, end), (3, end)]
        .into_iter()
        .map(|(i, parent)| log.append(r, &mut seq, Op::MoveRun { target: olds[i], parent, side: Side::Right }))
        .collect();
    let cyclic = log.append(r, &mut seq, Op::MoveRun {
        target: young, parent: Anchor::At(Pos { event: young, offset: 2 }), side: Side::Right,
    });
    let mut w = Weave::new(node);
    for order in [vec![0, 1, 2, 3], vec![1, 0, 3, 2]] {
        w = built_from(&base, &log, node).0;
        assert_eq!(w.text(), "1234abcdZY", "vacuity: the old runs start as the young one's elders");
        for i in order {
            w.apply(moves[i], &log.events()[&moves[i]].op);
        }
        assert_eq!(w.text(), "ab12cd34ZY");
        assert_eq!(w.apply(cyclic, &log.events()[&cyclic].op), Vec::new(), "a cyclic move shows nothing");
        assert_eq!(w.relaid, 0, "every move here changes one anchor");
        agrees_with_replay(&w, &log, node).unwrap();
    }
    let gone = log.append(r, &mut seq, Op::Delete { target: olds[2], range: (0, 1) });
    let away = log.append(r, &mut seq, Op::MoveRun { target: olds[2], parent: Anchor::DocStart(node), side: Side::Right });
    w.apply(gone, &log.events()[&gone].op);
    assert_eq!(w.apply(away, &log.events()[&away].op), Vec::new(), "moving a tombstoned run shows nothing");
    agrees_with_replay(&w, &log, node).unwrap();
    // Out of causal order -- a move naming a run this weave has not seen --
    // is ignored, not recorded (see `Weave::apply`).
    let unknown = EventId { seq: u32::MAX, replica: ReplicaId(9) };
    let stray = Op::MoveRun { target: unknown, parent: Anchor::DocStart(node), side: Side::Right };
    assert_eq!(w.apply(EventId { seq: seq + 1, replica: r }, &stray), Vec::new());
    assert_eq!(w.text(), "ab12cd4ZY");
}
