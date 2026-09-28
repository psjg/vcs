//! The event log's incremental frontier and clock against the definitions
//! they replace: recomputed from every event, which is what `append` used to
//! do on every call (O(n) per keystroke; FINDINGS).

use proptest::prelude::*;
use std::collections::BTreeSet;
use v0::event::{Event, EventLog};
use v0::op::{EventId, NodeId, Op, ReplicaId};

/// The frontier by definition: events no event names as a parent.
fn frontier_by_definition(log: &EventLog) -> Vec<EventId> {
    let claimed: BTreeSet<EventId> = log.events().values().flat_map(|e| e.parents.iter().copied()).collect();
    log.events().keys().copied().filter(|id| !claimed.contains(id)).collect()
}

/// The next Lamport time by definition: one past the highest seq seen.
fn next_by_definition(log: &EventLog) -> u32 {
    log.events().keys().map(|id| id.seq + 1).max().unwrap_or(1)
}

fn op() -> Op {
    Op::Remove { node: NodeId(EventId { seq: 0, replica: ReplicaId(0) }) }
}

proptest! {
    /// Replicas append, exchange events in any order -- children before their
    /// parents included -- and re-deliver some; after every step the kept
    /// frontier and clock equal the recomputed ones, and so does a log
    /// rebuilt from the stored list (how the store loads it).
    #[test]
    fn the_kept_frontier_and_clock_are_the_defined_ones(
        steps in proptest::collection::vec((0usize..3, 0usize..3, any::<bool>(), 0usize..1000), 1..60),
    ) {
        let mut logs = [EventLog::default(), EventLog::default(), EventLog::default()];
        let mut seqs = [1u32; 3];
        for (from, to, deliver, pick) in steps {
            if deliver {
                // Hand over a random subset of `from`'s events, in a shuffled
                // order: holes and orphans on the receiving side.
                let mut es: Vec<Event> = logs[from].events().values().cloned().collect();
                let n = es.len();
                if n > 0 {
                    es.rotate_left(pick % n);
                    es.truncate(1 + pick % n);
                    es.reverse();
                }
                logs[to].extend(es);
            } else {
                logs[from].append(ReplicaId(from as u64), &mut seqs[from], op());
            }
            for log in &logs {
                prop_assert_eq!(log.frontier(), frontier_by_definition(log));
                prop_assert_eq!(log.lamport_next(), next_by_definition(log));
                let holes: BTreeSet<EventId> = log.events().values()
                    .flat_map(|e| e.parents.iter().copied())
                    .filter(|p| !log.events().contains_key(p))
                    .collect();
                prop_assert_eq!(log.holes(), holes.len());
                let reloaded = EventLog::from(log.events().values().cloned().collect::<Vec<_>>());
                prop_assert_eq!(reloaded.frontier(), log.frontier());
                prop_assert_eq!(reloaded.lamport_next(), log.lamport_next());
            }
        }
    }
}

/// A link is the reference kind that never enters a closure. An event that
/// links to another -- a citation -- is recorded with the link, the link
/// survives sync, and adopting the event brings nothing it links to along:
/// neither into its semantic closure (what layer 1 derives dependencies
/// from) nor, through that, into a world. What the linked event's own
/// history does is its own business (I14 still holds for the citer).
#[test]
fn a_link_is_replicated_and_never_closed_over() {
    use v0::op::{Anchor, NodeKind, Side};
    use v0::sync;
    let (mut log, mut seq) = (EventLog::default(), 1);
    let r = ReplicaId(1);
    let f = NodeId(EventId { seq, replica: r });
    log.append(r, &mut seq, Op::Create { node: f, parent: Op::ROOT, name: "f".into(), kind: NodeKind::File });
    let quoted = log.append(r, &mut seq, Op::Insert { parent: Anchor::DocStart(f), side: Side::Right, text: "two hundred lines".into() });
    // A second document, and a note in it that cites the first: its op refers
    // to nothing in `f`; only its links do.
    let g = NodeId(EventId { seq, replica: r });
    log.append(r, &mut seq, Op::Create { node: g, parent: Op::ROOT, name: "g".into(), kind: NodeKind::File });
    let note = log.append_linked(r, &mut seq, Op::Insert { parent: Anchor::DocStart(g), side: Side::Right, text: "see".into() }, vec![quoted]);
    assert_eq!(log.events()[&note].links, vec![quoted], "recorded");
    assert!(!log.semantic_closure(note).contains(&quoted), "never closed over");
    assert!(log.causal_closure(note).contains(&quoted), "vacuity: causally it did see it, and that is not what closure derives from");

    // Replicated: a peer that pulls the note gets the link with it.
    let mut peer = EventLog::default();
    let wanted = sync::missing(&log, &sync::state_vector(&peer));
    let refused = sync::integrate(&mut peer, wanted);
    assert!(refused.is_empty());
    assert_eq!(peer.events()[&note].links, vec![quoted]);
    // And saving and loading keeps it (the store's JSON goes through Vec<Event>).
    let reloaded = EventLog::from(peer.events().values().cloned().collect::<Vec<_>>());
    assert_eq!(reloaded.events()[&note].links, vec![quoted]);

    // Lamport holds for links as for parents and refs (I14): a citation of
    // something younger than itself cannot have seen it, and is refused.
    let forged = v0::event::Event {
        id: EventId { seq: quoted.seq, replica: ReplicaId(9) },
        parents: vec![],
        op: Op::Create { node: NodeId(EventId { seq: quoted.seq, replica: ReplicaId(9) }), parent: Op::ROOT, name: "h".into(), kind: NodeKind::File },
        links: vec![quoted],
    };
    assert_eq!(forged.lamport_violation(), Some(quoted));
    let refused = sync::integrate(&mut peer, vec![forged]);
    assert_eq!(refused.len(), 1, "integrate refuses a link to something it could not have seen");
}
