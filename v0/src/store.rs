//! The imperative shell: the only module that touches the filesystem.
//!
//! ```text
//! .v0/events.json    the append-only log
//! .v0/changes.json   labels over it
//! .v0/head.json      which changes are in effect
//! .v0/replica        this working copy's id
//! .v0/hooks/         local programs v0 runs, never synced (see `hook`)
//! ```
//!
//! Boring and inspectable on purpose: a spike's state should be readable with
//! `cat` while you are debugging the algebra. No format stability (TECHDEBT).

use crate::change::{ChangeId, ChangeSet, Changes};
use crate::event::EventLog;
use crate::op::ReplicaId;
use crate::repo::Repo;
use crate::replay::Worktree;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

const DIR: &str = ".v0";

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::Other, msg.into())
}

/// Create `.v0/` and an empty repository.
pub fn init(root: &Path, replica_seed: u64) -> io::Result<Repo> {
    let dir = root.join(DIR);
    if dir.exists() {
        return Err(err(format!("{} already exists", dir.display())));
    }
    // Empty, so a hook is one file away (see `hook`); none is installed.
    fs::create_dir_all(dir.join("hooks"))?;
    let repo = Repo {
        log: EventLog::default(),
        changes: Changes::default(),
        head: ChangeSet::new(BTreeSet::new(), &Changes::default()).expect("empty is closed"),
        replica: ReplicaId(replica_seed),
        next_seq: 1,
    };
    save(root, &repo)?;
    Ok(repo)
}

/// Walk up from `cwd` until a `.v0/` turns up.
pub fn load(cwd: &Path) -> io::Result<(Repo, PathBuf)> {
    let root = cwd
        .ancestors()
        .find(|d| d.join(DIR).is_dir())
        .ok_or_else(|| err("not a v0 repository (no .v0 found in this directory or above)"))?
        .to_path_buf();
    let dir = root.join(DIR);

    let log: EventLog = read_json(&dir.join("events.json"))?;
    let changes: Changes = read_json(&dir.join("changes.json"))?;
    let head_ids: BTreeSet<ChangeId> = read_json(&dir.join("head.json"))?;
    let replica: u64 = fs::read_to_string(dir.join("replica"))?
        .trim()
        .parse()
        .map_err(|_| err("corrupt .v0/replica"))?;

    let head = ChangeSet::new(head_ids, &changes)
        .map_err(|e| err(format!("stored head is not dependency-closed: {e:?}")))?;
    let next_seq = log.lamport_next();

    Ok((Repo { log, changes, head, replica: ReplicaId(replica), next_seq }, root))
}

/// Persist everything. Write-to-temp-and-rename, so an interrupted save cannot
/// leave a half-written log — the same atomic-replace trick that makes
/// filesystem-level op capture hard, used here deliberately.
pub fn save(root: &Path, repo: &Repo) -> io::Result<()> {
    let dir = root.join(DIR);
    write_json(&dir.join("events.json"), &repo.log)?;
    write_json(&dir.join("changes.json"), &repo.changes)?;
    write_json(&dir.join("head.json"), repo.head.ids())?;
    write_atomic(&dir.join("replica"), repo.replica.0.to_string().as_bytes())
}

/// Marker prefixes. Deliberately v0-specific, so a stray `<<<<<<<` from some
/// other tool is never mistaken for one of ours.
pub const OPEN_MARK: &str = "<<<<<<< v0 conflict";
pub const CLOSE_MARK: &str = ">>>>>>> v0 conflict";

/// Does this text still contain conflict markers?
pub fn has_markers(text: &str) -> bool {
    text.lines().any(|l| l.starts_with(OPEN_MARK) || l.starts_with(CLOSE_MARK))
}

/// The worktree as it should appear on disk: the materialised head, with every
/// unresolved conflict drawn in place.
///
/// A conflict is a stretch of characters, but markers belong on whole lines, so
/// each is widened to the lines around it. What goes between the markers comes
/// from the set algebra rather than from a merge algorithm: each side's section
/// is the head **with the other sides taken out**, and the `before` section is
/// the head with every side taken out. Diff3 for free, because `M` reads sets.
///
/// Markers exist only on disk — the model never contains them — which is why
/// `record` refuses text that still has them.
pub fn render(repo: &Repo) -> Worktree {
    use crate::conflict::{conflicts, side_chars, Conflict, Kind};
    use crate::op::{NodeId, Pos};
    use std::collections::BTreeMap;

    let events: BTreeSet<crate::op::EventId> = repo.changes.events_of(repo.head.ids());
    let open: Vec<Conflict> = conflicts(&repo.head, &repo.changes, &repo.log)
        .into_iter()
        .filter(|c| c.status.is_open())
        .collect();

    // A file whose removal is disputed stays on disk, so whoever resolves it can
    // read what the other side did to it.
    let disputed: BTreeMap<NodeId, &Conflict> = open
        .iter()
        .filter(|c| c.kind == Kind::Removal)
        .map(|c| (NodeId(c.atom), c))
        .collect();
    let revive: BTreeSet<NodeId> = disputed.keys().copied().collect();
    // What an author had in front of them right after their change: every event
    // their change had seen, and the change itself.
    let seen_by = |side: &crate::change::ChangeId| -> BTreeSet<crate::op::EventId> {
        repo.changes.by_id[side].events.iter().flat_map(|e| repo.log.causal_closure(*e)).collect()
    };
    let materialise_events = |events: &BTreeSet<crate::op::EventId>| {
        let evs: Vec<_> = events.iter().copied().collect();
        crate::replay::materialise_with(&evs, &repo.log, &revive)
    };
    let _ = &events;
    // The head as everyone sees it: agreed text once.
    let merged = crate::replay::materialise_head(&repo.head, &repo.changes, &repo.log, &revive);

    let mut files = BTreeMap::new();
    for (path, atoms) in &merged.files {
        let index: BTreeMap<Pos, usize> = atoms.iter().enumerate().map(|(i, a)| (a.id, i)).collect();

        // Every text conflict with inserted characters in this file, widened to
        // whole lines. Overlapping blocks are drawn as one.
        let mut blocks: Vec<(usize, usize, BTreeSet<crate::change::ChangeId>, Vec<String>)> = Vec::new();
        for c in open.iter().filter(|c| c.kind == Kind::Text) {
            let hits: Vec<usize> = side_chars(c, &repo.changes, &repo.log)
                .values()
                .flatten()
                .filter_map(|p| index.get(p).copied())
                .collect();
            let (Some(&lo), Some(&hi)) = (hits.iter().min(), hits.iter().max()) else { continue };
            let start = atoms[..lo].iter().rposition(|a| a.ch == '\n').map_or(0, |i| i + 1);
            let end = atoms[hi..].iter().position(|a| a.ch == '\n').map_or(atoms.len(), |i| hi + i + 1);
            blocks.push((start, end, c.sides.clone(), vec![c.id.short()]));
        }
        blocks.sort_by_key(|b| b.0);
        let mut merged_blocks: Vec<(usize, usize, BTreeSet<_>, Vec<String>)> = Vec::new();
        for b in blocks {
            match merged_blocks.last_mut() {
                Some(last) if b.0 < last.1 => {
                    last.1 = last.1.max(b.1);
                    last.2.extend(b.2);
                    last.3.extend(b.3);
                }
                _ => merged_blocks.push(b),
            }
        }

        let mut out = String::new();
        let mut cursor = 0;
        for (start, end, sides, ids) in merged_blocks {
            out.extend(atoms[cursor..start].iter().map(|a| a.ch));
            // Diff3 by author. Each section is the text as that author had it
            // right after their change; `before` is what all of them had in
            // common. Cut each variant at the nearest characters outside the
            // block that the variant also contains -- an author who never saw a
            // line cannot be cut at it.
            let cut = |variant: &crate::replay::Materialised| -> String {
                let Some(v) = variant.files.get(path) else { return String::new() };
                let at: BTreeMap<Pos, usize> = v.iter().enumerate().map(|(i, a)| (a.id, i)).collect();
                let from = atoms[..start].iter().rev().find_map(|a| at.get(&a.id)).map_or(0, |i| i + 1);
                let to = atoms[end..].iter().find_map(|a| at.get(&a.id)).copied().unwrap_or(v.len());
                let mut t: String = v[from..to.max(from)].iter().map(|a| a.ch).collect();
                if !t.is_empty() && !t.ends_with('\n') {
                    t.push('\n');
                }
                t
            };
            let pasts: Vec<BTreeSet<crate::op::EventId>> = sides.iter().map(&seen_by).collect();
            let common: BTreeSet<crate::op::EventId> = pasts
                .iter()
                .skip(1)
                .fold(pasts[0].clone(), |acc, p| acc.intersection(p).copied().collect());
            out.push_str(&format!("{OPEN_MARK} {}\n", ids.join(" ")));
            out.push_str("||||||| before\n");
            out.push_str(&cut(&materialise_events(&common)));
            for (side, past) in sides.iter().zip(&pasts) {
                let msg = &repo.changes.by_id[side].meta.message;
                out.push_str(&format!("======= {} {msg}\n", side.short()));
                out.push_str(&cut(&materialise_events(past)));
            }
            out.push_str(&format!("{CLOSE_MARK} {}\n", ids.join(" ")));
            cursor = end;
        }
        out.extend(atoms[cursor..].iter().map(|a| a.ch));

        if let Some(c) = merged.nodes.get(path).and_then(|n| disputed.get(n)) {
            let who = |remover: bool| -> String {
                c.sides
                    .iter()
                    .filter(|s| removes(&repo.changes.by_id[s], c.atom, &repo.log) == remover)
                    .map(|s| format!("{} ({})", s.short(), repo.changes.by_id[s].meta.message))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let mut framed = format!(
                "{OPEN_MARK} {}: removed by {} while edited by {}\n",
                c.id.short(),
                who(true),
                who(false)
            );
            framed.push_str(&out);
            if !framed.ends_with('\n') {
                framed.push('\n');
            }
            framed.push_str(&format!("{CLOSE_MARK} {}\n", c.id.short()));
            out = framed;
        }
        files.insert(path.clone(), out);
    }
    Worktree { files }
}

/// Does this change remove the node created by `node_event`?
fn removes(change: &crate::change::Change, node_event: crate::op::EventId, log: &EventLog) -> bool {
    change.events.iter().any(|e| {
        matches!(log.events.get(e).map(|ev| &ev.op), Some(crate::op::Op::Remove { node }) if node.0 == node_event)
    })
}

/// Project the rendered worktree onto disk, removing files the head no longer
/// contains. Only paths v0 knows about are touched: an untracked file is none of
/// our business.
pub fn checkout(root: &Path, repo: &Repo) -> io::Result<()> {
    let want = render(repo);
    // Every path v0 has ever placed a file at -- *including removed files*. The
    // first version asked the materialiser, which leaves removed files out by
    // design, so a removal arriving from a peer never left the disk; worse, the
    // next `record` then saw the stray file and restored it, silently undoing
    // the peer's removal.
    let all: Vec<_> = repo.log.events.keys().copied().collect();
    let tree = crate::replay::tree_of(&all, &repo.log);
    let ever: BTreeSet<String> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.kind == crate::op::NodeKind::File)
        .filter_map(|(id, _)| tree.path_ignoring_removal(*id))
        .collect();

    for (path, text) in &want.files {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        // Byte-exact now that files are text rather than lines: a file without
        // a trailing newline keeps not having one.
        write_atomic(&full, text.as_bytes())?;
    }
    for path in &ever {
        if !want.files.contains_key(path) {
            let full = root.join(path);
            if full.exists() {
                fs::remove_file(full)?;
            }
        }
    }
    Ok(())
}

/// Every file in the working copy, excluding `.v0`, VCS metadata, and anything
/// a line-oriented system has no business storing.
pub fn scan(root: &Path) -> io::Result<Worktree> {
    let mut files = std::collections::BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(text) = fs::read_to_string(&path) {
                if text.len() < 400_000 {
                    let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy();
                    files.insert(rel.into_owned(), text);
                }
            }
        }
    }
    Ok(Worktree { files })
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| err(format!("{}: {e}", path.display())))
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| err(e.to_string()))?;
    write_atomic(path, &text)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)
}
