//! The imperative shell: the only module that touches the filesystem.
//!
//! ```text
//! .v0/events.json    the append-only log
//! .v0/changes.json   labels over it
//! .v0/head.json      which changes are in effect
//! .v0/replica        this working copy's id
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
    fs::create_dir_all(&dir)?;
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
    let next_seq = log
        .events
        .keys()
        .filter(|id| id.replica == ReplicaId(replica))
        .map(|id| id.seq + 1)
        .max()
        .unwrap_or(1);

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

/// Project the materialised worktree onto disk, removing files the head no
/// longer contains. Only paths v0 knows about are touched: an untracked file is
/// none of our business.
pub fn checkout(root: &Path, repo: &Repo) -> io::Result<()> {
    let want = repo.worktree();
    let all: Vec<_> = repo.log.events.keys().copied().collect();
    let ever = crate::replay::materialise(&all, &repo.log).files;

    for (path, lines) in &want.files {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut body = lines.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        write_atomic(&full, body.as_bytes())?;
    }
    for path in ever.keys() {
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
                    files.insert(rel.into_owned(), text.lines().map(str::to_owned).collect());
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
