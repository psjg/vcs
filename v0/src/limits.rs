//! A limit on everything that comes from outside (TigerStyle, Power of 10).
//!
//! [`crate::budget`] bounds the process; these bound the individual things a
//! peer or a disk can hand it, so a wasteful or hostile input shows up as a
//! named limit being hit, at the door, and not as a larger number in the
//! budget. Fixed, and deliberately generous: each is far above anything the
//! CLI itself produces, so hitting one means the input is wrong, not that
//! the limit needs tuning.
//!
//! An event over a limit is refused by [`crate::sync::integrate`] exactly as
//! one that breaks the Lamport order is (**I14**), and for the same reason it
//! may be: the judgement reads the event alone, so it is the same on every
//! replica and in any arrival order (**I13**).

use crate::event::Event;
use crate::op::Op;

/// Bytes of text one event may carry: an insert's run, or a rename's new
/// identifier. Above [`FILE_BYTES_MAX`], so a file recorded in one go -- one
/// run -- always fits.
pub const EVENT_TEXT_BYTES_MAX: usize = 1 << 20;
/// Bytes in a node's name: what POSIX filesystems allow a path component.
pub const NAME_BYTES_MAX: usize = 255;
/// Causal parents of one event. A frontier has at most one head per
/// concurrently writing replica.
pub const EVENT_PARENTS_MAX: usize = 4096;
/// Links of one event: a citation of this many things is a bulk import.
pub const EVENT_LINKS_MAX: usize = 4096;
/// Events in one exchange. [`crate::sync::missing`] sends at most this many,
/// oldest first, and [`crate::sync::integrate`] takes at most this many; a
/// sync of more is several exchanges.
pub const SYNC_EVENTS_MAX: usize = 1 << 16;
/// Bytes of a file the working copy tracks. A larger one is reported and
/// left alone ([`crate::store::scan`]), never recorded and never mistaken
/// for a removal.
pub const FILE_BYTES_MAX: usize = 400_000;

const _: () = assert!(FILE_BYTES_MAX <= EVENT_TEXT_BYTES_MAX);

/// A limit that was hit: which, by how much.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Over {
    /// The constant's name, as written in this module.
    pub limit: &'static str,
    pub got: usize,
    pub max: usize,
}

impl std::fmt::Display for Over {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is {}, {} at most", self.limit, self.got, self.max)
    }
}

fn over(limit: &'static str, got: usize, max: usize) -> Option<Over> {
    (got > max).then_some(Over { limit, got, max })
}

impl Event {
    /// The first limit this event exceeds, if any. Reads the event alone.
    pub fn over_limit(&self) -> Option<Over> {
        let text = |t: &str| over("EVENT_TEXT_BYTES_MAX", t.len(), EVENT_TEXT_BYTES_MAX);
        let name = |n: &str| over("NAME_BYTES_MAX", n.len(), NAME_BYTES_MAX);
        over("EVENT_PARENTS_MAX", self.parents.len(), EVENT_PARENTS_MAX)
            .or_else(|| over("EVENT_LINKS_MAX", self.links.len(), EVENT_LINKS_MAX))
            .or_else(|| match &self.op {
                Op::Insert { text: t, .. } | Op::Rename { to: t, .. } => text(t),
                Op::Create { name: n, .. } | Op::MoveNode { name: n, .. } => name(n),
                Op::Delete { .. } | Op::MoveRun { .. } | Op::Remove { .. } | Op::Restore { .. } | Op::SetMode { .. } => None,
            })
    }
}
