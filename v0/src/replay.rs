//! `M(S)` — materialisation as replay.
//!
//! Walk the events of a change set in a deterministic order, building a
//! throwaway structure (a weave of atoms plus a [`crate::tree::Tree`]), and read
//! the worktree off it. This is eg-walker's move: keep the log, build CRDT state
//! transiently at replay, discard it.
//!
//! Because `M` reads a **set**, `merge` needs no algorithm: union the sets and
//! the same walk yields the merged result. Commutativity is not implemented, it
//! is a consequence — which is why this module has no `merge` function.

use crate::change::{ChangeSet, Changes};
use crate::event::EventLog;
use crate::op::EventId;
use std::collections::BTreeMap;

/// A line that has ever existed, alive or tombstoned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Atom {
    /// The insert that created it — also its permanent position identity.
    pub id: EventId,
    pub line: String,
}

/// What lands on disk: every file's lines, keyed by path.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Worktree {
    pub files: BTreeMap<String, Vec<String>>,
}

/// Swappable materialisers, so a Loro-backed implementation can be benchmarked
/// against ours once the semantics settle (ADR-0004). The trait is deliberately
/// one method wide: a materialiser is a pure function of a set.
pub trait Materialiser {
    fn materialise(&self, set: &ChangeSet, changes: &Changes, log: &EventLog) -> Worktree;
}

/// The reference implementation: order siblings at an anchor by [`EventId`]
/// descending, then walk each subtree.
///
/// The order must be total, deterministic and **permanent**. If two people
/// resolve the same conflict independently and the order is not fixed by the
/// structure, one gets `AXYB` and the other `AYXB`, and merging those has no
/// good answer.
#[derive(Clone, Copy, Default, Debug)]
pub struct WeaveReplay;

impl Materialiser for WeaveReplay {
    fn materialise(&self, set: &ChangeSet, changes: &Changes, log: &EventLog) -> Worktree {
        todo!()
    }
}

/// Replay a raw event set, ignoring change labels.
///
/// This is what a **live session** materialises: events that no change names yet
/// still have to render in the editor. Same function, different granularity —
/// the unification the project is built on.
pub fn materialise_events(events: &[EventId], log: &EventLog) -> Worktree {
    todo!()
}
