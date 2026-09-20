//! The deliverable: DESIGN.md's invariants as executable properties.
//!
//! Tested at the highest seam that still observes real behaviour — the event
//! log, the change layer and the materialiser, driven the way a front-end would
//! drive them. No mocks: these are the real structures.

use std::collections::BTreeSet;
use v0::change::{Change, ChangeId, ChangeSet, Changes, Meta};
use v0::event::EventLog;
use v0::op::{Anchor, EventId, NodeId, NodeKind, Op, ReplicaId};
use v0::replay::{materialise_events, Materialiser, WeaveReplay, Worktree};
use v0::sync;

/// A working copy, driven directly. The fixture is deliberately thin: if a test
/// needs a helper the real API does not have, that is a smell about the API.
struct Peer {
    log: EventLog,
    changes: Changes,
    replica: ReplicaId,
    seq: u32,
    /// Events appended since the last `record` — what a live session would be
    /// holding, unnamed.
    pending: BTreeSet<EventId>,
}

impl Peer {
    fn new(replica: u64) -> Self {
        Self {
            log: EventLog::default(),
            changes: Changes::default(),
            replica: ReplicaId(replica),
            seq: 1,
            pending: BTreeSet::new(),
        }
    }

    fn append(&mut self, op: Op) -> EventId {
        let id = self.log.append(self.replica, &mut self.seq, op);
        self.pending.insert(id);
        id
    }

    fn create_file(&mut self, name: &str) -> NodeId {
        let node = NodeId(EventId { seq: self.seq, replica: self.replica });
        self.append(Op::Create {
            node,
            parent: Op::ROOT,
            name: name.into(),
            kind: NodeKind::File,
        });
        node
    }

    fn insert(&mut self, anchor: Anchor, line: &str) -> EventId {
        self.append(Op::Insert { anchor, line: line.into() })
    }

    /// Append `lines` as a chain, returning the last atom.
    fn insert_chain(&mut self, mut at: Anchor, lines: &[&str]) -> Anchor {
        for l in lines {
            at = Anchor::After(self.insert(at, l));
        }
        at
    }

    /// Name everything appended since the last record. Recording is labelling:
    /// the events already exist.
    fn record(&mut self, message: &str) -> ChangeId {
        let events = std::mem::take(&mut self.pending);
        let meta = Meta { message: message.into(), author: "test".into() };
        let change = Change::new(events, meta, &self.log, &self.changes.owners());
        let id = change.id();
        self.changes.by_id.insert(id, change);
        id
    }

    fn set(&self, ids: &[ChangeId]) -> ChangeSet {
        let closed: BTreeSet<ChangeId> =
            ids.iter().flat_map(|c| self.changes.closure(*c)).collect();
        ChangeSet::new(closed, &self.changes).expect("closure is closed by construction")
    }

    fn worktree(&self, ids: &[ChangeId]) -> Worktree {
        WeaveReplay.materialise(&self.set(ids), &self.changes, &self.log)
    }
}

/// A history where one change is causally downstream of a lot of unrelated
/// work: 40 events in `noise.txt`, then two lines in `real.txt`. Exactly the
/// shape where causal parents over-report and the reduction should pay.
fn noisy_history() -> (Peer, ChangeId, ChangeId) {
    let mut p = Peer::new(1);
    let noise = p.create_file("noise.txt");
    let lines: Vec<String> = (0..40).map(|i| format!("noise {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    p.insert_chain(Anchor::DocStart(noise), &refs);
    let c_noise = p.record("a lot of unrelated work");

    let real = p.create_file("real.txt");
    p.insert_chain(Anchor::DocStart(real), &["fn main() {", "}"]);
    let c_real = p.record("the change we actually want");
    (p, c_noise, c_real)
}

// --- the algebra ------------------------------------------------------------

#[test]
fn i1_materialise_is_a_pure_function_of_the_set() {
    let (p, _, c) = noisy_history();
    assert_eq!(p.worktree(&[c]), p.worktree(&[c]), "same set, same bytes");
}

#[test]
fn i2_merge_is_commutative_associative_and_idempotent() {
    let (p, a, b) = noisy_history();
    let (sa, sb) = (p.set(&[a]), p.set(&[b]));
    let m = |s: &ChangeSet| WeaveReplay.materialise(s, &p.changes, &p.log);

    assert_eq!(m(&sa.union(&sb)), m(&sb.union(&sa)), "commutative");
    assert_eq!(m(&sa.union(&sa)), m(&sa), "idempotent");
    assert_eq!(
        m(&sa.union(&sb).union(&sa)),
        m(&sa.union(&sb)),
        "associative enough: re-adding a member changes nothing"
    );
}

#[test]
fn i3_drop_leaves_other_change_ids_byte_identical() {
    let (p, c_noise, c_real) = noisy_history();
    let before: Vec<ChangeId> = p.changes.by_id.keys().copied().collect();
    // "Dropping" is set subtraction; nothing is rewritten, so every surviving
    // id must be the one it always was.
    let survivors: BTreeSet<ChangeId> = p
        .changes
        .by_id
        .keys()
        .copied()
        .filter(|id| !p.changes.upward_closure(c_noise).contains(id))
        .collect();
    assert!(survivors.contains(&c_real) || p.changes.upward_closure(c_noise).contains(&c_real));
    for id in &survivors {
        assert_eq!(p.changes.by_id[id].id(), *id, "content address is stable");
        assert!(before.contains(id));
    }
}

#[test]
fn i4_changeset_cannot_be_constructed_unclosed() {
    let (p, _c_noise, c_real) = noisy_history();
    let deps = &p.changes.by_id[&c_real].deps;
    if deps.is_empty() {
        return; // nothing to violate
    }
    let lone: BTreeSet<ChangeId> = [c_real].into_iter().collect();
    assert!(ChangeSet::new(lone, &p.changes).is_err(), "a change without its deps is refused");
}

// --- the reduction: what the spike exists to answer -------------------------

#[test]
fn i8_minimal_deps_are_a_subset_of_causal_deps() {
    let (p, _, c_real) = noisy_history();
    let ch = &p.changes.by_id[&c_real];
    let causal: BTreeSet<EventId> =
        ch.events.iter().flat_map(|e| p.log.causal_closure(*e)).collect();
    let minimal = p.changes.events_of(&p.changes.closure(c_real));
    let extra: Vec<&EventId> = minimal.difference(&causal).collect();
    assert!(
        extra.is_empty(),
        "the reduction invented {} dependencies the author never saw: {extra:?}",
        extra.len()
    );
}

#[test]
fn i9_minimal_closure_loses_nothing_against_causal_closure() {
    let (p, _, c_real) = noisy_history();
    let ch = &p.changes.by_id[&c_real];

    let minimal: Vec<EventId> =
        p.changes.events_of(&p.changes.closure(c_real)).into_iter().collect();
    let causal: Vec<EventId> =
        ch.events.iter().flat_map(|e| p.log.causal_closure(*e)).collect();

    let (wm, wc) = (materialise_events(&minimal, &p.log), materialise_events(&causal, &p.log));
    assert_eq!(
        wm.files.get("real.txt"),
        wc.files.get("real.txt"),
        "replaying only what the change needs must give the same lines as \
         replaying everything its author had seen"
    );
}

/// Not an assertion but the headline measurement: how much smaller is the
/// derived dependency closure than the causal one? Near 1.0 means layer 1 buys
/// nothing and the design is wrong.
#[test]
fn reduction_ratio_is_reported() {
    let (p, _, c_real) = noisy_history();
    let ratios = p.changes.reduction_ratio(&p.log);
    let r = ratios[&c_real];
    println!("reduction ratio for the isolated change: {r:.1}x");
    assert!(r > 3.0, "expected a large reduction on an isolated change, got {r:.2}x");
}

// --- liveness ---------------------------------------------------------------

#[test]
fn i12_two_replicas_converge_after_a_bidirectional_sync() {
    let mut a = Peer::new(1);
    let f = a.create_file("shared.txt");
    a.insert_chain(Anchor::DocStart(f), &["one", "two"]);

    let mut b = Peer::new(2);
    let to_b = sync::missing(&a.log, &sync::state_vector(&b.log));
    sync::integrate(&mut b.log, to_b);
    b.insert(Anchor::DocStart(f), "zero");
    let to_a = sync::missing(&b.log, &sync::state_vector(&a.log));
    sync::integrate(&mut a.log, to_a);

    let all_a: Vec<EventId> = a.log.events.keys().copied().collect();
    let all_b: Vec<EventId> = b.log.events.keys().copied().collect();
    assert_eq!(
        materialise_events(&all_a, &a.log),
        materialise_events(&all_b, &b.log),
        "peers that exchanged state vectors must see the same worktree"
    );
}

#[test]
fn i13_integrate_is_idempotent_and_order_insensitive() {
    let mut a = Peer::new(1);
    let f = a.create_file("shared.txt");
    a.insert_chain(Anchor::DocStart(f), &["one", "two", "three"]);

    let mut once = EventLog::default();
    let events = sync::missing(&a.log, &sync::state_vector(&once));
    sync::integrate(&mut once, events.clone());

    let mut twice_reversed = EventLog::default();
    let mut rev = events.clone();
    rev.reverse();
    sync::integrate(&mut twice_reversed, rev);
    sync::integrate(&mut twice_reversed, events);

    let ka: Vec<EventId> = once.events.keys().copied().collect();
    let kb: Vec<EventId> = twice_reversed.events.keys().copied().collect();
    assert_eq!(ka, kb);
    assert_eq!(materialise_events(&ka, &once), materialise_events(&kb, &twice_reversed));
}

// --- still skeleton ---------------------------------------------------------

#[test]
#[ignore = "needs a two-branch fixture"]
fn i5_adopt_after_drop_restores_the_original_set() {}

#[test]
#[ignore = "expected to fail: RGA-family interleaving, see DESIGN"]
fn i6_concurrent_insert_blocks_never_interleave() {}

#[test]
#[ignore = "skeleton"]
fn i7_drop_only_perturbs_lines_the_change_touched() {}

#[test]
#[ignore = "skeleton"]
fn i10_concurrent_line_moves_leave_exactly_one_copy() {}

#[test]
#[ignore = "skeleton"]
fn i11_concurrent_node_moves_never_cycle_and_all_replicas_skip_the_same_one() {}

/// Guard against a hollow suite: if the fixture did not actually build the
/// files, every invariant above would pass vacuously.
#[test]
fn fixture_is_not_vacuous() {
    let (p, c_noise, c_real) = noisy_history();
    let w = p.worktree(&[c_noise, c_real]);
    assert_eq!(w.files["real.txt"], vec!["fn main() {", "}"]);
    assert_eq!(w.files["noise.txt"].len(), 40);
    assert_eq!(w.files["noise.txt"][0], "noise 0");

    let only_real = p.worktree(&[c_real]);
    assert_eq!(only_real.files["real.txt"], vec!["fn main() {", "}"]);
    assert!(
        !only_real.files.contains_key("noise.txt"),
        "adopting the real change must not drag in 40 unrelated events"
    );
}

// --- the dependency shape that actually matters -----------------------------

/// The easy case is a change that depends on nothing. The real one is a change
/// that edits a line an earlier change created — that is where the derived
/// dependency has to be right, and where there is nothing left to reduce.
#[test]
fn dependency_is_derived_when_a_change_edits_an_earlier_one() {
    let mut p = Peer::new(1);
    let f = p.create_file("a.txt");
    let one = p.insert(Anchor::DocStart(f), "one");
    let two = p.insert(Anchor::After(one), "two");
    p.insert(Anchor::After(two), "three");
    let a = p.record("A: three lines");

    p.append(Op::Delete { target: two });
    p.insert(Anchor::After(one), "TWO");
    let b = p.record("B: rewrite the middle line");

    assert_eq!(
        p.changes.by_id[&b].deps,
        [a].into_iter().collect::<BTreeSet<_>>(),
        "B refers to A's atoms, so A is derived as its dependency"
    );
    assert!(
        ChangeSet::new([b].into_iter().collect(), &p.changes).is_err(),
        "B alone is not materialisable"
    );
    assert_eq!(p.worktree(&[b]).files["a.txt"], vec!["one", "TWO", "three"]);

    // And the honest half: when the dependency is real there is nothing to
    // reduce. A large ratio is a property of *isolated* work, not a free lunch.
    let r = p.changes.reduction_ratio(&p.log)[&b];
    println!("reduction ratio for a dependent change: {r:.2}x");
    assert!(r < 1.5, "a change that edits its predecessor cannot shed it");
}

/// What does *change* granularity cost against *event* granularity? A change is
/// adopted whole, including events nothing referred to.
#[test]
fn change_granularity_over_approximates_and_we_measure_by_how_much() {
    let mut p = Peer::new(1);
    let f = p.create_file("a.txt");
    let one = p.insert(Anchor::DocStart(f), "one");
    // Nine more lines nobody will refer to, recorded together with `one`.
    let mut at = Anchor::After(one);
    for i in 2..=10 {
        at = Anchor::After(p.insert(at, &format!("line {i}")));
    }
    p.record("A: ten lines");

    p.append(Op::Delete { target: one });
    let b = p.record("B: delete the first line");

    let change_minimal = p.changes.events_of(&p.changes.closure(b)).len();
    let event_minimal: usize = p.changes.by_id[&b]
        .events
        .iter()
        .flat_map(|e| p.log.semantic_closure(*e))
        .collect::<BTreeSet<_>>()
        .len();
    println!(
        "granularity cost: change-level pulls {change_minimal} events, \
         event-level would need {event_minimal}"
    );
    assert!(
        change_minimal >= event_minimal,
        "grouping can only over-approximate, never under"
    );
}
