//! The live weave against the batch replay (the sum tree has its own file).
//!
//! The oracle throughout is `replay`: whatever the weave says about a set of
//! events, materialising the set from scratch must agree.

use proptest::prelude::*;
use v0::capture;
use v0::event::EventLog;
use v0::op::{EventId, NodeId, NodeKind, Op, Pos, ReplicaId};
use v0::replay;
use v0::sumtree::{Bias, Summary};
use v0::weave::{Chars, Locator, Metrics, PointUtf16, Weave, MAX_FRAGMENT};

/// Save each text in turn on one replica, capturing the diff the way `record`
/// does. Returns the log with its file's node.
fn saves(log: &mut EventLog, node: NodeId, replica: u64, texts: &[String]) {
    let r = ReplicaId(replica);
    for text in texts {
        let all: Vec<EventId> = log.events.keys().copied().collect();
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
    log.events.extend(other.events);
    (log, node)
}

fn texts() -> impl Strategy<Value = Vec<String>> {
    let word = prop_oneof![
        Just("a"), Just("bb"), Just(" "), Just("\n"), Just("\r\n"), Just("\r"), Just("é"), Just("😀"), Just("zz")
    ];
    proptest::collection::vec(proptest::collection::vec(word, 0..12).prop_map(|w| w.concat()), 0..4)
}

fn built(log: &EventLog, node: NodeId) -> (Weave, Vec<(v0::replay::Atom, bool)>) {
    let all: Vec<EventId> = log.events.keys().copied().collect();
    let mut weaves = replay::weaves(&all, log);
    let walk = weaves.walks.remove(&node).unwrap_or_default();
    (Weave::from_walk(node, &walk, &weaves.anchors, log), walk)
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
        .events
        .values()
        .map(|e| (e.id, e.parents.iter().filter(|p| log.events.contains_key(p)).count()))
        .collect();
    let mut ready: Vec<EventId> = waiting.iter().filter(|(_, n)| **n == 0).map(|(id, _)| *id).collect();
    waiting.retain(|_, n| *n > 0);
    let mut order = Vec::new();
    let mut c = choices.iter().cycle();
    while !ready.is_empty() {
        let i = c.next().copied().unwrap_or(0) % ready.len();
        let e = ready.swap_remove(i);
        order.push(e);
        for child in log.events.values().filter(|x| x.parents.contains(&e)) {
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
        let new: Vec<char> = now[e.old.start..e.old.start + e.new_len].to_vec();
        prop_assert!(e.new_len == 0 || edits.len() == 1, "an insert is one edit");
        model.splice(e.old.clone(), new);
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
            apply_checked(&mut w, id, &log.events[&id].op)?;
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
        let all: Vec<EventId> = log.events.keys().copied().collect();
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
            let lo = if i == 0 { Locator::MIN } else { keys[i - 1].clone() };
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
    let mut lo = Locator::MIN;
    let hi = Locator::between(&Locator::MIN, &Locator::max());
    for _ in 0..2000 {
        let k = Locator::between(&lo, &hi);
        assert!(lo < k && k < hi);
        lo = k;
    }
    let mut hi2 = hi.clone();
    for _ in 0..2000 {
        let k = Locator::between(&Locator::MIN, &hi2);
        assert!(Locator::MIN < k && k < hi2);
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
    let run = *log.events.keys().find(|id| matches!(log.events[id].op, Op::Insert { .. })).unwrap();
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
