//! Layer 1: labels over the log, and the dependency reduction.
//!
//! A change is **not** a snapshot and **not** a diff. It is a name for a set of
//! events that already exist. Labels can be unioned and subtracted while the
//! events underneath keep their identity — that is the whole trick, and the
//! reason `drop` does not rewrite anything.

use crate::event::EventLog;
use crate::op::EventId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Content address: `blake3` over the canonical encoding.
///
/// Because no change's bytes mention a change it does not depend on, dropping
/// one cannot perturb another's identity — invariant **I3** is structural, not
/// enforced by code that could forget.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ChangeId(pub [u8; 32]);

impl Serialize for ChangeId {
    /// Hex, so `cat .v0/head.json` is readable by a human.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ChangeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let hex = String::deserialize(d)?;
        parse_hex32(&hex).map(ChangeId).map_err(serde::de::Error::custom)
    }
}

/// Parse a 64-character hex string into 32 bytes. Shared by every content
/// address in the crate, so they all print and parse the same way.
pub(crate) fn parse_hex32(hex: &str) -> Result<[u8; 32], String> {
    if hex.len() != 64 || !hex.is_char_boundary(hex.len()) {
        return Err(format!("expected 64 hex characters, got {}", hex.len()));
    }
    let bytes: Result<Vec<u8>, _> =
        (0..64).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16)).collect();
    let bytes = bytes.map_err(|e| e.to_string())?;
    bytes.try_into().map_err(|_| "not 32 bytes".to_string())
}

impl std::fmt::Display for ChangeId {
    /// Hex, like every other content address a developer has to type.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl ChangeId {
    /// The first 8 hex characters — enough to name a change out loud.
    pub fn short(&self) -> String {
        self.to_string()[..8].to_owned()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Meta {
    pub message: String,
    pub author: String,
    /// Conflicts this change declares it resolves. Empty for ordinary changes
    /// and then **omitted from the encoding**, so adding the field did not
    /// change the content address of any change recorded before it existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolves: Vec<crate::conflict::ConflictId>,
}

impl Meta {
    pub fn new(message: impl Into<String>, author: impl Into<String>) -> Self {
        Self { message: message.into(), author: author.into(), resolves: Vec::new() }
    }
}

/// A named set of events plus the changes it depends on.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Change {
    pub events: BTreeSet<EventId>,
    pub deps: BTreeSet<ChangeId>,
    pub meta: Meta,
}

impl Change {
    /// Name a set of events, deriving `deps` from the ops themselves.
    ///
    /// The rule in full: for every event in the set, map each
    /// [`crate::op::Op::refs`] entry to the change owning that event; drop
    /// self-references. No antichain minimisation — `closure` is identical
    /// either way, so minimisation is normalisation, not semantics (TECHDEBT).
    ///
    /// `owners` is the only context needed, and notably *not* causal history.
    pub fn new(
        events: BTreeSet<EventId>,
        meta: Meta,
        log: &EventLog,
        owners: &BTreeMap<EventId, ChangeId>,
    ) -> Self {
        let mut deps = BTreeSet::new();
        for id in &events {
            let Some(ev) = log.events.get(id) else { continue };
            for r in ev.op.refs() {
                // A reference into our own event set is internal, not a
                // dependency. A reference to an event nobody has named yet
                // cannot be a dependency either -- it is a live edit.
                if events.contains(&r) {
                    continue;
                }
                if let Some(owner) = owners.get(&r) {
                    deps.insert(*owner);
                }
            }
        }
        Self { events, deps, meta }
    }

    pub fn id(&self) -> ChangeId {
        ChangeId(*blake3::hash(&self.canonical()).as_bytes())
    }

    /// Canonical encoding. Must stay stable across versions or every id in an
    /// existing repository silently changes meaning.
    fn canonical(&self) -> Vec<u8> {
        // Deterministic because every collection here is ordered (`BTreeSet`)
        // and serde emits struct fields in declaration order.
        serde_json::to_vec(self).expect("a Change is always serialisable")
    }
}

/// Every change a repository knows.
///
/// Stored as a list, and the ids are **recomputed on load**. That is not just a
/// workaround for JSON keys: because an id is a content address, rebuilding the
/// map re-verifies every hash each time a repository is opened.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(from = "Vec<Change>", into = "Vec<Change>")]
pub struct Changes {
    pub by_id: BTreeMap<ChangeId, Change>,
}

impl From<Vec<Change>> for Changes {
    fn from(list: Vec<Change>) -> Self {
        Self { by_id: list.into_iter().map(|c| (c.id(), c)).collect() }
    }
}

impl From<Changes> for Vec<Change> {
    fn from(c: Changes) -> Self {
        c.by_id.into_values().collect()
    }
}

impl Changes {
    /// Which change named each event. Events named by no change are live edits
    /// that nobody has recorded yet — ordinary, not an error.
    pub fn owners(&self) -> BTreeMap<EventId, ChangeId> {
        let mut out = BTreeMap::new();
        for (id, c) in &self.by_id {
            for e in &c.events {
                out.insert(*e, *id);
            }
        }
        out
    }

    /// `c` and everything it transitively depends on: the smallest legal set
    /// containing `c`.
    pub fn closure(&self, c: ChangeId) -> BTreeSet<ChangeId> {
        self.reach(c, |ch| ch.deps.iter().copied().collect())
    }

    /// `c` and everything that transitively depends on it: what must go when
    /// `c` goes.
    pub fn upward_closure(&self, c: ChangeId) -> BTreeSet<ChangeId> {
        let mut dependents: BTreeMap<ChangeId, Vec<ChangeId>> = BTreeMap::new();
        for (id, ch) in &self.by_id {
            for d in &ch.deps {
                dependents.entry(*d).or_default().push(*id);
            }
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![c];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            stack.extend(dependents.get(&id).into_iter().flatten().copied());
        }
        seen
    }

    /// Transitive reachability from `c` through `step`.
    fn reach(&self, c: ChangeId, step: impl Fn(&Change) -> Vec<ChangeId>) -> BTreeSet<ChangeId> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![c];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(ch) = self.by_id.get(&id) {
                stack.extend(step(ch));
            }
        }
        seen
    }

    /// Every event named by a set of changes.
    pub fn events_of(&self, ids: &BTreeSet<ChangeId>) -> BTreeSet<EventId> {
        ids.iter()
            .filter_map(|id| self.by_id.get(id))
            .flat_map(|c| c.events.iter().copied())
            .collect()
    }

    /// Resolve a hex prefix, the way a developer types it. Ambiguity is an
    /// error rather than a guess.
    pub fn resolve(&self, prefix: &str) -> Result<ChangeId, &'static str> {
        let hits: Vec<ChangeId> = self
            .by_id
            .keys()
            .filter(|id| id.to_string().starts_with(prefix))
            .copied()
            .collect();
        match hits.len() {
            1 => Ok(hits[0]),
            0 => Err("no such change"),
            _ => Err("ambiguous prefix"),
        }
    }

    /// How much smaller the derived dependency closure is than the causal one,
    /// per change. **The headline measurement of the spike** — near 1.0 means
    /// layer 1 buys nothing and the design is wrong.
    pub fn reduction_ratio(&self, log: &EventLog) -> BTreeMap<ChangeId, f64> {
        self.by_id
            .iter()
            .map(|(id, ch)| {
                // Both sides counted in events, so the ratio is like for like.
                let causal: BTreeSet<EventId> =
                    ch.events.iter().flat_map(|e| log.causal_closure(*e)).collect();
                let minimal = self.events_of(&self.closure(*id));
                let ratio = if minimal.is_empty() {
                    1.0
                } else {
                    causal.len() as f64 / minimal.len() as f64
                };
                (*id, ratio)
            })
            .collect()
    }
}

/// Split events into the connected components of their semantic-reference
/// graph: events that refer to each other belong together, events that do not
/// are separate work.
///
/// This is ADR-0007, and it is what stops one commit touching five unrelated
/// files from welding their histories together for good. Measured effect
/// (docs/FINDINGS.md): adoption cost across a real history goes from 222 → 1663
/// events under git's own commit boundaries to 110 → 123 under this rule.
///
/// Two events belong together when one refers to the other **or when both
/// refer to the same atom**. The second half is not optional: replacing a line
/// is a `Delete` and an `Insert` that both name the old atom and never name
/// each other, so without it every edited line splits into two changes — which
/// is exactly what the first CLI run produced.
///
/// Shared *node* references are excluded, or every line ever inserted at the
/// top of a file would weld into one component and this would collapse into
/// per-file grouping.
///
/// It remains a heuristic about *reference*, not *intent*: a fix and the test
/// that covers it, touching nothing in common, land in separate components. An
/// explicit "these are one change" override belongs above this function, and
/// should be recorded as an override rather than hidden inside it.
pub fn components(
    events: &BTreeSet<EventId>,
    log: &EventLog,
    hints: &[BTreeSet<EventId>],
) -> Vec<BTreeSet<EventId>> {
    let ids: Vec<EventId> = events.iter().copied().collect();
    let index: BTreeMap<EventId, usize> = ids.iter().enumerate().map(|(i, e)| (*e, i)).collect();
    let mut parent: Vec<usize> = (0..ids.len()).collect();

    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }

    // One event referring to another in the same set: they are one piece of
    // work (a run typed onto the end of a run typed a moment earlier).
    for (i, e) in ids.iter().enumerate() {
        let Some(ev) = log.events.get(e) else { continue };
        for r in ev.op.refs() {
            if let Some(j) = index.get(&r) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, *j));
                parent[a] = b;
            }
        }
    }
    // Two events touching the same *character* are working on the same place.
    // Deliberately characters, not events: a file created in one go is a single
    // run, so sharing a referenced event would weld every later edit anywhere
    // in that file into one change.
    let mut sharers: BTreeMap<crate::op::Pos, usize> = BTreeMap::new();
    for (i, e) in ids.iter().enumerate() {
        let Some(ev) = log.events.get(e) else { continue };
        let run_len = |t: EventId| match log.events.get(&t).map(|e| &e.op) {
            Some(crate::op::Op::Insert { text, .. }) => text.chars().count() as u32,
            _ => 0,
        };
        for p in ev.op.positions(&run_len) {
            match sharers.get(&p) {
                Some(j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, *j));
                    parent[a] = b;
                }
                None => {
                    sharers.insert(p, i);
                }
            }
        }
    }

    // What the capture adapter *knows* belongs together, which structure cannot
    // recover. Replacing a line is a Delete naming the old atom and an Insert
    // that Fugue placed against the *next* line: they share no referent and
    // never name each other, yet they are plainly one edit. The adapter saw
    // that; the graph cannot.
    for hint in hints {
        let mut members = hint.iter().filter_map(|e| index.get(e).copied());
        if let Some(first) = members.next() {
            for m in members {
                let (a, b) = (find(&mut parent, first), find(&mut parent, m));
                parent[a] = b;
            }
        }
    }

    let mut groups: BTreeMap<usize, BTreeSet<EventId>> = BTreeMap::new();
    for (i, e) in ids.iter().enumerate() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().insert(*e);
    }
    groups.into_values().collect()
}

/// A set of changes closed under `deps` — the only thing that can be replayed.
/// The constructor *is* the invariant (**I4**).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangeSet(BTreeSet<ChangeId>);

impl ChangeSet {
    pub fn new(ids: BTreeSet<ChangeId>, changes: &Changes) -> Result<Self, NotClosed> {
        for id in &ids {
            let Some(ch) = changes.by_id.get(id) else { continue };
            for d in &ch.deps {
                if !ids.contains(d) {
                    return Err(NotClosed { change: *id, missing_dep: *d });
                }
            }
        }
        Ok(Self(ids))
    }

    pub fn ids(&self) -> &BTreeSet<ChangeId> {
        &self.0
    }

    /// Union of two closed sets is closed, so this cannot fail. `merge` is this
    /// function and nothing else.
    pub fn union(&self, other: &ChangeSet) -> ChangeSet {
        ChangeSet(self.0.union(&other.0).copied().collect())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotClosed {
    pub change: ChangeId,
    pub missing_dep: ChangeId,
}
