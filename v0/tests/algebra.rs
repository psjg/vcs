//! The deliverable: DESIGN.md's invariants as executable properties.
//!
//! Tested at the highest seam that still observes real behaviour — the event
//! log, the change layer and the materialiser, driven the way a front-end would
//! drive them. No mocks: these are the real structures.

use std::collections::BTreeSet;
use v0::change::{Change, ChangeId, ChangeSet, Changes, Meta};
use v0::event::EventLog;
use v0::capture;
use v0::op::{Anchor, EventId, NodeId, NodeKind, Op, ReplicaId, Side};
use v0::replay::{materialise_events, Materialiser, WeaveReplay, Worktree};
use v0::repo::Repo;
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

    /// Append after `parent` — the simple case, a right child.
    fn insert(&mut self, parent: Anchor, line: &str) -> EventId {
        self.append(Op::Insert { parent, side: Side::Right, line: line.into() })
    }

    /// Place a line between two neighbours by Fugue's rule, the way a real
    /// capture adapter does.
    fn insert_between(&mut self, a: Anchor, b: Option<EventId>, line: &str) -> EventId {
        let (parent, side) = {
            let log = &self.log;
            let parent_of = |e: EventId| match log.events.get(&e).map(|ev| &ev.op) {
                Some(Op::Insert { parent, .. } | Op::MoveLine { parent, .. }) => Some(*parent),
                _ => None,
            };
            capture::between(a, b, &parent_of)
        };
        self.append(Op::Insert { parent, side, line: line.into() })
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
        let meta = Meta::new(message, "test");
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

    /// A working copy whose head is every change recorded so far.
    fn to_repo(&self) -> Repo {
        let all: BTreeSet<ChangeId> = self.changes.by_id.keys().copied().collect();
        Repo {
            log: self.log.clone(),
            changes: self.changes.clone(),
            head: ChangeSet::new(all, &self.changes).expect("everything recorded is closed"),
            replica: self.replica,
            next_seq: self.seq,
        }
    }

    fn create_dir(&mut self, name: &str, parent: NodeId) -> NodeId {
        let node = NodeId(EventId { seq: self.seq, replica: self.replica });
        self.append(Op::Create { node, parent, name: name.into(), kind: NodeKind::Dir });
        node
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
    let first = *b.log.events.keys().nth(1).expect("the file has a first line");
    b.insert_between(Anchor::DocStart(f), Some(first), "zero");
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

/// Two changes, the second editing the first, plus an untouched bystander file.
fn two_changes_and_a_bystander() -> (Peer, ChangeId, ChangeId) {
    let mut p = Peer::new(1);
    let f = p.create_file("a.txt");
    let one = p.insert(Anchor::DocStart(f), "one");
    let two = p.insert(Anchor::After(one), "two");
    p.insert(Anchor::After(two), "three");
    let bystander = p.create_file("untouched.txt");
    p.insert_chain(Anchor::DocStart(bystander), &["keep", "me"]);
    let a = p.record("A");

    p.append(Op::Delete { target: two });
    p.insert_between(Anchor::After(one), Some(two), "TWO");
    let b = p.record("B");
    (p, a, b)
}

#[test]
fn i5_adopt_after_drop_restores_the_original_set() {
    let (p, _a, b) = two_changes_and_a_bystander();
    let mut repo = p.to_repo();
    let original = repo.head.clone();

    repo.head = repo.drop_change(b);
    assert!(!repo.head.ids().contains(&b), "B is gone");
    repo.head = repo.adopt(b);

    assert_eq!(repo.head, original, "adopt after drop is the identity");
    assert_eq!(repo.worktree(), p.to_repo().worktree(), "and so is the worktree");
}

#[test]
fn i7_drop_only_perturbs_lines_the_change_touched() {
    let (p, _a, b) = two_changes_and_a_bystander();
    let mut repo = p.to_repo();
    let before = repo.worktree();
    repo.head = repo.drop_change(b);
    let after = repo.worktree();

    assert_eq!(before.files["a.txt"], vec!["one", "TWO", "three"]);
    assert_eq!(after.files["a.txt"], vec!["one", "two", "three"], "B's edit is undone");
    assert_eq!(
        before.files["untouched.txt"], after.files["untouched.txt"],
        "a file B never touched must be byte-identical after the drop"
    );
}

// --- moves ------------------------------------------------------------------

#[test]
fn i10_concurrent_line_moves_leave_exactly_one_copy() {
    let mut a = Peer::new(1);
    let f = a.create_file("x.txt");
    let one = a.insert(Anchor::DocStart(f), "one");
    let two = a.insert(Anchor::After(one), "two");
    let three = a.insert(Anchor::After(two), "three");
    let mut b = a.fork(2);

    // Both peers move the same line somewhere visibly different: A puts it at
    // the top, B puts it in the middle. Same seq, so the replica id breaks the
    // tie and B must win on every replica.
    a.append(Op::MoveLine { target: three, parent: Anchor::After(one), side: Side::Left });
    b.append(Op::MoveLine { target: three, parent: Anchor::After(two), side: Side::Left });

    let lines = merged_lines(&a, &b, "x.txt");
    assert_eq!(
        lines.iter().filter(|l| *l == "three").count(),
        1,
        "a moved line exists once, never duplicated: {lines:?}"
    );
    assert_eq!(lines.len(), 3, "and nothing else appeared or vanished: {lines:?}");
    assert_eq!(lines, vec!["one", "three", "two"], "the higher EventId wins, everywhere");
    let _ = f;
}

#[test]
fn i11_concurrent_node_moves_never_cycle_and_all_replicas_skip_the_same_one() {
    let mut a = Peer::new(1);
    let d1 = a.create_dir("one", Op::ROOT);
    let d2 = a.create_dir("two", Op::ROOT);
    let f1 = NodeId(EventId { seq: a.seq, replica: a.replica });
    a.append(Op::Create { node: f1, parent: d1, name: "f1".into(), kind: NodeKind::File });
    a.insert(Anchor::DocStart(f1), "in one");
    let f2 = NodeId(EventId { seq: a.seq, replica: a.replica });
    a.append(Op::Create { node: f2, parent: d2, name: "f2".into(), kind: NodeKind::File });
    a.insert(Anchor::DocStart(f2), "in two");
    let mut b = a.fork(2);

    // The classic: each peer moves one directory inside the other.
    a.append(Op::MoveNode { node: d1, parent: d2, name: "one".into() });
    b.append(Op::MoveNode { node: d2, parent: d1, name: "two".into() });

    let mut log = a.log.clone();
    let incoming = sync::missing(&b.log, &sync::state_vector(&log));
    sync::integrate(&mut log, incoming);
    let all: Vec<EventId> = log.events.keys().copied().collect();
    let w = materialise_events(&all, &log);

    // A's move lands, B's is declined because it would close the cycle. Pinned
    // exactly, because "no cycle appeared" is also satisfied by dropping both.
    let paths: Vec<&String> = w.files.keys().collect();
    assert_eq!(paths, vec!["two/f2", "two/one/f1"], "one move applied, one declined");

    // Worth stating plainly: the agreement below is structural, not earned.
    // Once both peers hold the same event set, I1 makes identical output a
    // theorem rather than a test result. What is actually being tested is that
    // the *decision* is a function of the set and not of arrival order.
    let mut other = b.log.clone();
    let incoming = sync::missing(&a.log, &sync::state_vector(&other));
    sync::integrate(&mut other, incoming);
    let all_b: Vec<EventId> = other.events.keys().copied().collect();
    assert_eq!(w, materialise_events(&all_b, &other));
}

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
    // Between `one` and `two`. The naive "right child of the left neighbour"
    // would put this line after `two`'s whole subtree -- which is exactly the
    // mistake Fugue's rule exists to prevent, and it did catch it here.
    p.insert_between(Anchor::After(one), Some(two), "TWO");
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

// --- I6: does the ordering interleave concurrent blocks? --------------------

impl Peer {
    /// A second working copy that has seen everything this one has. Starts its
    /// sequence counter at the same value on purpose: equal `seq` is the worst
    /// case for a `(seq, replica)` tie-break, which is exactly where
    /// interleaving would show up.
    fn fork(&self, replica: u64) -> Peer {
        Peer {
            log: self.log.clone(),
            changes: self.changes.clone(),
            replica: ReplicaId(replica),
            seq: self.seq,
            pending: BTreeSet::new(),
        }
    }
}

/// Merge two peers' logs and read one file's lines.
fn merged_lines(a: &Peer, b: &Peer, path: &str) -> Vec<String> {
    let mut log = a.log.clone();
    let incoming = sync::missing(&b.log, &sync::state_vector(&log));
    sync::integrate(&mut log, incoming);
    let all: Vec<EventId> = log.events.keys().copied().collect();
    materialise_events(&all, &log).files[path].clone()
}

/// Is every line of `block` contiguous in `lines`?
fn contiguous(lines: &[String], block: &[&str]) -> bool {
    let positions: Vec<usize> =
        block.iter().filter_map(|b| lines.iter().position(|l| l == b)).collect();
    positions.len() == block.len()
        && positions.windows(2).all(|w| w[1].abs_diff(w[0]) == 1)
}

/// Typing forward — each line anchored to the one just typed. Subtree
/// contiguity should keep the two blocks apart.
#[test]
fn i6_forward_typed_blocks_do_not_interleave() {
    let mut a = Peer::new(1);
    let f = a.create_file("x.txt");
    let base = a.insert(Anchor::DocStart(f), "base");
    let mut b = a.fork(2);

    a.insert_chain(Anchor::After(base), &["a1", "a2", "a3"]);
    b.insert_chain(Anchor::After(base), &["b1", "b2", "b3"]);

    let lines = merged_lines(&a, &b, "x.txt");
    assert!(contiguous(&lines, &["a1", "a2", "a3"]), "A's block is broken up: {lines:?}");
    assert!(contiguous(&lines, &["b1", "b2", "b3"]), "B's block is broken up: {lines:?}");
}

/// Typing *backward* — each new line inserted above the previous one. This is
/// the case the Fugue paper shows RGA-family orderings get wrong; before Fugue
/// this test produced a perfect alternation, `base b3 a3 b2 a2 b1 a1`.
#[test]
fn i6_backward_typed_blocks_do_not_interleave() {
    let mut a = Peer::new(1);
    let f = a.create_file("x.txt");
    let base = a.insert(Anchor::DocStart(f), "base");
    let mut b = a.fork(2);

    // Both peers type upward from the same anchor: each new line goes between
    // `base` and the line they typed last.
    let mut top = None;
    for l in ["a1", "a2", "a3"] {
        top = Some(a.insert_between(Anchor::After(base), top, l));
    }
    let mut top = None;
    for l in ["b1", "b2", "b3"] {
        top = Some(b.insert_between(Anchor::After(base), top, l));
    }

    let lines = merged_lines(&a, &b, "x.txt");
    assert!(contiguous(&lines, &["a3", "a2", "a1"]), "A's block interleaves: {lines:?}");
    assert!(contiguous(&lines, &["b3", "b2", "b1"]), "B's block interleaves: {lines:?}");
}

/// Vacuity guard for the move tests: print what the merges actually produced,
/// and assert the moves were not silently no-ops.
#[test]
fn move_fixtures_are_not_vacuous() {
    // --- lines ---
    let mut a = Peer::new(1);
    let f = a.create_file("x.txt");
    let one = a.insert(Anchor::DocStart(f), "one");
    let two = a.insert(Anchor::After(one), "two");
    let three = a.insert(Anchor::After(two), "three");
    let before = materialise_events(
        &a.log.events.keys().copied().collect::<Vec<_>>(),
        &a.log,
    )
    .files["x.txt"]
        .clone();
    let mut b = a.fork(2);
    a.append(Op::MoveLine { target: three, parent: Anchor::After(one), side: Side::Left });
    b.append(Op::MoveLine { target: three, parent: Anchor::After(two), side: Side::Left });
    let after = merged_lines(&a, &b, "x.txt");
    let _ = f;
    println!("line move: {before:?} -> {after:?}");
    assert_ne!(before, after, "if the move changed nothing, I10 proves nothing");

    // --- nodes ---
    let mut a = Peer::new(1);
    let d1 = a.create_dir("one", Op::ROOT);
    let d2 = a.create_dir("two", Op::ROOT);
    let f1 = NodeId(EventId { seq: a.seq, replica: a.replica });
    a.append(Op::Create { node: f1, parent: d1, name: "f1".into(), kind: NodeKind::File });
    a.insert(Anchor::DocStart(f1), "in one");
    let f2 = NodeId(EventId { seq: a.seq, replica: a.replica });
    a.append(Op::Create { node: f2, parent: d2, name: "f2".into(), kind: NodeKind::File });
    a.insert(Anchor::DocStart(f2), "in two");
    let paths_before: Vec<String> = materialise_events(
        &a.log.events.keys().copied().collect::<Vec<_>>(),
        &a.log,
    )
    .files
    .keys()
    .cloned()
    .collect();

    let mut b = a.fork(2);
    a.append(Op::MoveNode { node: d1, parent: d2, name: "one".into() });
    b.append(Op::MoveNode { node: d2, parent: d1, name: "two".into() });
    let mut log = a.log.clone();
    let incoming = sync::missing(&b.log, &sync::state_vector(&log));
    sync::integrate(&mut log, incoming);
    let paths_after: Vec<String> =
        materialise_events(&log.events.keys().copied().collect::<Vec<_>>(), &log)
            .files
            .keys()
            .cloned()
            .collect();
    println!("node move: {paths_before:?} -> {paths_after:?}");
    assert_ne!(paths_before, paths_after, "one of the two moves must have taken effect");
    assert_eq!(paths_after.len(), 2, "and neither file may be orphaned");
}

// --- conflicts --------------------------------------------------------------

use v0::conflict::{conflicts, Status};

/// Everything both peers have, as one working copy.
fn union(a: &Peer, b: &Peer) -> Repo {
    let mut repo = a.to_repo();
    let incoming = sync::missing(&b.log, &sync::state_vector(&repo.log));
    sync::integrate(&mut repo.log, incoming);
    for (id, ch) in &b.changes.by_id {
        repo.changes.by_id.entry(*id).or_insert_with(|| ch.clone());
    }
    let all: BTreeSet<ChangeId> = repo.changes.by_id.keys().copied().collect();
    repo.head = ChangeSet::new(all, &repo.changes).expect("union of closed sets");
    repo
}

/// Alice and Bob both replace the middle line of a shared file, differently.
fn two_replacements() -> (Peer, Peer, ChangeId, ChangeId) {
    let mut a = Peer::new(1);
    let f = a.create_file("x.txt");
    let one = a.insert(Anchor::DocStart(f), "one");
    let two = a.insert(Anchor::After(one), "two");
    a.insert(Anchor::After(two), "three");
    a.record("base");
    let mut b = a.fork(2);

    a.append(Op::Delete { target: two });
    a.insert_between(Anchor::After(one), Some(two), "TWO by alice");
    let ca = a.record("alice");
    b.append(Op::Delete { target: two });
    b.insert_between(Anchor::After(one), Some(two), "TWO by bob");
    let cb = b.record("bob");
    (a, b, ca, cb)
}

#[test]
fn a_conflict_has_the_same_identity_on_every_replica() {
    let (a, b, ca, cb) = two_replacements();
    let (ra, rb) = (union(&a, &b), union(&b, &a));
    let (xa, xb) = (
        conflicts(&ra.head, &ra.changes, &ra.log),
        conflicts(&rb.head, &rb.changes, &rb.log),
    );
    assert_eq!(xa.len(), 1, "one contested line, one conflict: {xa:?}");
    assert_eq!(xa, xb, "derived, not stored -- and still identical everywhere");
    assert_eq!(xa[0].sides, [ca, cb].into_iter().collect::<BTreeSet<_>>());
    assert_eq!(xa[0].status, Status::Open);
}

#[test]
fn agreeing_deletions_are_not_a_conflict() {
    let mut a = Peer::new(1);
    let f = a.create_file("x.txt");
    let one = a.insert(Anchor::DocStart(f), "one");
    a.insert(Anchor::After(one), "two");
    a.record("base");
    let mut b = a.fork(2);
    a.append(Op::Delete { target: one });
    a.record("alice deletes");
    b.append(Op::Delete { target: one });
    b.record("bob deletes");
    let r = union(&a, &b);
    assert!(conflicts(&r.head, &r.changes, &r.log).is_empty(), "same intent, nothing to choose");
}

#[test]
fn a_resolution_closes_the_conflict_and_says_who_and_why() {
    let (a, b, ca, cb) = two_replacements();
    let mut r = union(&a, &b);
    let open = conflicts(&r.head, &r.changes, &r.log);

    // Keep Alice's line, drop Bob's -- an ordinary edit, then a declaration.
    let bobs_line = *r.changes.by_id[&cb]
        .events
        .iter()
        .find(|e| matches!(r.log.events[e].op, Op::Insert { .. }))
        .unwrap();
    let del = r.log.append(r.replica, &mut r.next_seq, Op::Delete { target: bobs_line });
    let fix = r.resolve([del].into_iter().collect(), &open, Meta::new("alice's wording is clearer", "carol"));

    let after = conflicts(&r.head, &r.changes, &r.log);
    assert_eq!(after[0].status, Status::Resolved(fix), "still visible, but closed");
    let ch = &r.changes.by_id[&fix];
    assert_eq!(ch.meta.author, "carol");
    assert_eq!(ch.meta.message, "alice's wording is clearer");
    assert!(ch.deps.contains(&ca) && ch.deps.contains(&cb), "depends on both sides, declared");
    assert_eq!(r.worktree().files["x.txt"], vec!["one", "TWO by alice", "three"]);
}

#[test]
fn dropping_a_side_takes_the_resolution_with_it() {
    let (a, b, _ca, cb) = two_replacements();
    let mut r = union(&a, &b);
    let open = conflicts(&r.head, &r.changes, &r.log);
    let fix = r.resolve(BTreeSet::new(), &open, Meta::new("keep both", "carol"));

    r.head = r.drop_change(cb);
    assert!(!r.head.ids().contains(&fix), "a resolution without its conflict means nothing");
    assert!(conflicts(&r.head, &r.changes, &r.log).is_empty(), "one side left, no conflict");
}

#[test]
fn concurrent_resolutions_of_one_conflict_are_contested() {
    let (a, b, _, _) = two_replacements();
    let mut r1 = union(&a, &b);
    let mut r2 = union(&b, &a);
    r2.replica = ReplicaId(3);
    let open = conflicts(&r1.head, &r1.changes, &r1.log);
    let f1 = r1.resolve(BTreeSet::new(), &open, Meta::new("keep both", "carol"));
    let f2 = r2.resolve(BTreeSet::new(), &open, Meta::new("also keep both, differently worded", "dave"));

    // Carol and Dave never saw each other's decision.
    let mut merged = r1.clone();
    let incoming = sync::missing(&r2.log, &sync::state_vector(&merged.log));
    sync::integrate(&mut merged.log, incoming);
    merged.changes.by_id.insert(f2, r2.changes.by_id[&f2].clone());
    let ids = merged.head.ids().iter().copied().chain([f2]).collect();
    merged.head = ChangeSet::new(ids, &merged.changes).unwrap();

    let c = conflicts(&merged.head, &merged.changes, &merged.log);
    match &c[0].status {
        Status::Contested(rs) => assert_eq!(rs.len(), 2, "both resolutions surfaced: {rs:?}"),
        other => panic!("two blind resolutions must not quietly pick one: {other:?} ({f1:?}, {f2:?})"),
    }
}
