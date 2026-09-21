//! Per-keystroke cost of the live weave, at one build's `B` and fragment cap.
//!
//! ```text
//! weave_bench <text-file> <bytes> <warmup-keystrokes> <measured-keystrokes>
//! ```
//!
//! The document is `<text-file>` repeated to `<bytes>`. A warm-up of typing
//! sessions -- a random spot, then a burst of keystrokes and backspaces, each
//! keystroke its own event as an editor sends them -- fragments it the way a
//! day of editing would. Then every measured keystroke is timed in the steps
//! a live front-end takes:
//!
//! | step    | what                                                   |
//! |---------|--------------------------------------------------------|
//! | seek    | LSP position -> offset (`offset_of_point`)             |
//! | make    | offset -> op (`insert_op`, `delete_ops`)               |
//! | apply   | op -> weave (`apply`)                                  |
//! | report  | offset -> LSP position, for the cursor (`point_of_offset`) |
//!
//! Also: opening the document from the log (replay + `from_walk`), a move
//! (a full relayout), and `EventLog::append` at the log's final size -- the
//! path a keystroke takes into history.
//!
//! Prints one CSV line; `bench/weave.sh` sweeps builds and collects them.

use std::time::Instant;
use v0::event::{Event, EventLog};
use v0::op::{Anchor, EventId, NodeId, NodeKind, Op, Pos, ReplicaId, Side};
use v0::replay;
use v0::sumtree::{Bias, DEFAULT_B};
use v0::weave::{Chars, Weave, MAX_FRAGMENT};

/// xorshift64: deterministic, dependency-free, good enough to pick spots.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Mints events straight into the log: `EventLog::append` computes the
/// frontier in O(events) per call, which is measured separately and must not
/// drown the weave's own cost.
struct Session {
    log: EventLog,
    w: Weave,
    seq: u32,
    rng: Rng,
    cursor: usize,
}

const REPLICA: ReplicaId = ReplicaId(7);

impl Session {
    fn mint(&mut self, op: Op) -> EventId {
        let id = EventId { seq: self.seq, replica: REPLICA };
        self.seq += 1;
        self.log.events.insert(id, Event { id, parents: Vec::new(), op });
        id
    }

    /// One keystroke of a burst; timings go to `t` when given.
    fn keystroke(&mut self, t: Option<&mut Timings>) {
        let len = self.w.metrics().chars;
        let backspace = self.cursor > 0 && self.rng.below(100) < 15;
        let point = self.w.point_of_offset(Chars(self.cursor));
        let t0 = Instant::now();
        let at = self.w.offset_of_point(point);
        let t1 = Instant::now();
        debug_assert_eq!(at.0, self.cursor.min(len));
        if backspace {
            let ops = self.w.delete_ops(Chars(at.0 - 1)..at);
            let t2 = Instant::now();
            let ids: Vec<EventId> = ops.iter().map(|op| self.mint(op.clone())).collect();
            let t3 = Instant::now();
            for (id, op) in ids.iter().zip(&ops) {
                self.w.apply(*id, op);
            }
            let t4 = Instant::now();
            self.cursor -= 1;
            let _ = self.w.point_of_offset(Chars(self.cursor));
            let t5 = Instant::now();
            if let Some(t) = t {
                t.back.push((t4 - t3 + (t2 - t0)).as_nanos() as u64);
                t.report.push((t5 - t4).as_nanos() as u64);
            }
        } else {
            let ch = b"etaoin shrdlu\n"[self.rng.below(14)] as char;
            let op = self.w.insert_op(at, ch.encode_utf8(&mut [0; 4]));
            let t2 = Instant::now();
            let id = self.mint(op.clone());
            let t3 = Instant::now();
            self.w.apply(id, &op);
            let t4 = Instant::now();
            self.cursor += 1;
            let _ = self.w.point_of_offset(Chars(self.cursor));
            let t5 = Instant::now();
            if let Some(t) = t {
                t.seek.push((t1 - t0).as_nanos() as u64);
                t.make.push((t2 - t1).as_nanos() as u64);
                t.apply.push((t4 - t3).as_nanos() as u64);
                t.typed.push((t4 - t3 + (t2 - t0)).as_nanos() as u64);
                t.report.push((t5 - t4).as_nanos() as u64);
            }
        }
    }

    /// A typing session: a random spot, then a burst.
    fn session(&mut self, keystrokes: &mut usize, mut t: Option<&mut Timings>) {
        self.cursor = self.rng.below(self.w.metrics().chars + 1);
        let burst = 1 + self.rng.below(30);
        for _ in 0..burst.min(*keystrokes) {
            self.keystroke(t.as_deref_mut());
            *keystrokes -= 1;
        }
    }
}

#[derive(Default)]
struct Timings {
    seek: Vec<u64>,
    make: Vec<u64>,
    apply: Vec<u64>,
    typed: Vec<u64>,
    back: Vec<u64>,
    report: Vec<u64>,
}

fn pct(v: &mut [u64], p: f64) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, file, bytes, warmup, measured] = args.as_slice() else {
        eprintln!("usage: weave_bench <text-file> <bytes> <warmup-keystrokes> <measured-keystrokes>");
        std::process::exit(2);
    };
    let (bytes, warmup, measured): (usize, usize, usize) =
        (bytes.parse().unwrap(), warmup.parse().unwrap(), measured.parse().unwrap());
    let seed = std::fs::read_to_string(file).expect("read the text file");
    let mut text = String::new();
    while text.len() < bytes {
        text.push_str(&seed);
    }
    let cut = text.char_indices().map(|(i, _)| i).take_while(|i| *i <= bytes).last().unwrap_or(0);
    text.truncate(cut);

    // The document: one file, its text one insert.
    let mut log = EventLog::default();
    let mut seq = 1;
    let node = NodeId(EventId { seq, replica: REPLICA });
    log.append(REPLICA, &mut seq, Op::Create { node, parent: Op::ROOT, name: "doc".into(), kind: NodeKind::File });
    log.append(REPLICA, &mut seq, Op::Insert { parent: Anchor::DocStart(node), side: Side::Right, text });
    let all: Vec<EventId> = log.events.keys().copied().collect();
    let w = Weave::from_walk(node, &replay::weaves(&all, &log), &log);
    let mut s = Session { log, w, seq, rng: Rng(0x9E37_79B9_7F4A_7C15), cursor: 0 };

    let mut left = warmup;
    while left > 0 {
        s.session(&mut left, None);
    }
    let mut t = Timings::default();
    let mut left = measured;
    while left > 0 {
        s.session(&mut left, Some(&mut t));
    }
    let fragments = s.w.fragments().count();
    let events = s.log.events.len();

    // Opening the edited document from its log.
    let all: Vec<EventId> = s.log.events.keys().copied().collect();
    let t0 = Instant::now();
    let weaves = replay::weaves(&all, &s.log);
    let t1 = Instant::now();
    let reopened = Weave::from_walk(node, &weaves, &s.log);
    let t2 = Instant::now();
    assert_eq!(reopened.text(), s.w.text(), "the live weave and a fresh replay agree");
    let (replay_ms, from_walk_ms) = ((t1 - t0).as_secs_f64() * 1e3, (t2 - t1).as_secs_f64() * 1e3);

    // Moves: a typed run hung somewhere else, each a full relayout.
    let runs: Vec<EventId> = s.log.events.keys().copied().filter(|e| e.replica == REPLICA && e.seq > 2).collect();
    let mut moves = Vec::new();
    for _ in 0..10 {
        let target = runs[s.rng.below(runs.len())];
        let Some(to) = s.w.pos_at(Chars(s.rng.below(s.w.metrics().chars)), Bias::Right) else { continue };
        let op = Op::MoveRun { target, parent: Anchor::At(to), side: Side::Right };
        let id = s.mint(op.clone());
        let t0 = Instant::now();
        s.w.apply(id, &op);
        moves.push((t0.elapsed().as_micros()) as u64);
    }

    // The history path: EventLog::append at this log's size.
    let mut appends = Vec::new();
    let mut seq = s.seq;
    for i in 0..50 {
        let op = Op::Insert { parent: Anchor::At(Pos { event: runs[i % runs.len()], offset: 0 }), side: Side::Right, text: "x".into() };
        let t0 = Instant::now();
        s.log.append(REPLICA, &mut seq, op);
        appends.push(t0.elapsed().as_micros() as u64);
    }

    println!(
        "{B},{F},{bytes},{events},{fragments},{seek},{make},{apply},{typed50},{typed99},{typedmax},{back50},{back99},{report},{replay_ms:.1},{from_walk_ms:.1},{move_us},{append_us}",
        B = DEFAULT_B,
        F = MAX_FRAGMENT,
        seek = pct(&mut t.seek, 0.5),
        make = pct(&mut t.make, 0.5),
        apply = pct(&mut t.apply, 0.5),
        typed50 = pct(&mut t.typed, 0.5),
        typed99 = pct(&mut t.typed, 0.99),
        typedmax = pct(&mut t.typed, 1.0),
        back50 = pct(&mut t.back, 0.5),
        back99 = pct(&mut t.back, 0.99),
        report = pct(&mut t.report, 0.5),
        move_us = pct(&mut moves, 0.5),
        append_us = pct(&mut appends, 0.5),
    );
}
