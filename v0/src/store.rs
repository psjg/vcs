//! The imperative shell: the only module that touches the filesystem.
//!
//! ```text
//! .v0/events.json    the append-only log
//! .v0/changes.json   labels over it
//! .v0/head.json      which changes are in effect
//! .v0/replica        this working copy's replica id
//! ```
//!
//! Boring and inspectable on purpose: a spike's state should be readable with
//! `cat` while you are debugging the algebra. No format stability (TECHDEBT).

use crate::repo::Repo;
use std::io;
use std::path::{Path, PathBuf};

pub fn init(root: &Path, replica_seed: u64) -> io::Result<Repo> {
    todo!()
}

/// Walk up from `cwd` to find `.v0/`.
pub fn load(cwd: &Path) -> io::Result<(Repo, PathBuf)> {
    todo!()
}

/// Write-to-temp-and-rename, so an interrupted save cannot leave a half-written
/// log. The same atomic-replace trick that makes filesystem-level op capture
/// hard, used here deliberately.
pub fn save(root: &Path, repo: &Repo) -> io::Result<()> {
    todo!()
}

/// Project the materialised worktree onto disk.
pub fn checkout(root: &Path, repo: &Repo) -> io::Result<()> {
    todo!()
}
