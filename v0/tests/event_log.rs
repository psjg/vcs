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
