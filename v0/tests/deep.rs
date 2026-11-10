//! A long typing session is a deep Fugue tree: with live capture every
//! keystroke is its own run, hanging off the one before. Nothing that reads
//! the tree may be as deep on the machine stack as the session was long.

use std::collections::BTreeSet;
use v0::event::EventLog;
use v0::op::{Anchor, EventId, NodeId, NodeKind, Op, Pos, ReplicaId, Side};
use v0::replay;
use v0::weave::Weave;

/// `n` single-character runs, each a child of the one before on `side`:
/// typing forwards (`Right`) or always at the caret's left (`Left`).
fn session(n: u32, side: Side) -> (EventLog, NodeId, Vec<EventId>) {
    let (mut log, mut seq, r) = (EventLog::default(), 1, ReplicaId(1));
    let f = NodeId(EventId { seq, replica: r });
    let mut ids = vec![log.append(r, &mut seq, Op::Create { node: f, parent: Op::ROOT, name: "f".into(), kind: NodeKind::File })];
    let mut at = Anchor::DocStart(f);
    for i in 0..n {
        let ch = char::from(b'a' + (i % 26) as u8);
        // The first run hangs right of the document start either way.
        let s = if i == 0 { Side::Right } else { side };
        let id = log.append(r, &mut seq, Op::Insert { parent: at, side: s, text: ch.to_string() });
        at = Anchor::At(Pos { event: id, offset: 0 });
        ids.push(id);
    }
    (log, f, ids)
}

/// Deep enough to overflow a 2 MB test-thread stack many times over when
/// the walk recursed per run (it did, until 2026-11-10).
const KEYSTROKES: u32 = 100_000;

fn expected(n: u32, side: Side) -> String {
    let typed = (0..n).map(|i| char::from(b'a' + (i % 26) as u8));
    match side {
        Side::Right => typed.collect(),
        // The first run is the root's right child and each next one a left
        // child of the last, so reading order is typing order reversed.
        Side::Left => typed.collect::<Vec<_>>().into_iter().rev().collect(),
    }
}

#[test]
fn a_long_session_typed_forwards_replays_and_opens() {
    let (log, f, ids) = session(KEYSTROKES, Side::Right);
    let text = &replay::materialise_with(&ids, &log, &BTreeSet::new()).files["f"];
    assert_eq!(text.iter().map(|a| a.ch).collect::<String>(), expected(KEYSTROKES, Side::Right));
    let weave = Weave::from_walk(f, &replay::weaves(&ids, &log), &log);
    assert_eq!(weave.text(), expected(KEYSTROKES, Side::Right));
}

#[test]
fn a_long_session_typed_backwards_replays_and_opens() {
    let (log, f, ids) = session(KEYSTROKES, Side::Left);
    let text = &replay::materialise_with(&ids, &log, &BTreeSet::new()).files["f"];
    assert_eq!(text.iter().map(|a| a.ch).collect::<String>(), expected(KEYSTROKES, Side::Left));
    let weave = Weave::from_walk(f, &replay::weaves(&ids, &log), &log);
    assert_eq!(weave.text(), expected(KEYSTROKES, Side::Left));
}
