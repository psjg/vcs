//! The imperative shell: the only module that touches the filesystem.
//!
//! Layout of a repository, deliberately boring and inspectable:
//!
//! ```text
//! .v0/log.json     every change, serde JSON
//! .v0/head.json    the change ids currently in effect
//! .v0/replica      this working copy's replica id
//! ```
//!
//! No database, no format stability (TECHDEBT.md). A spike's on-disk state
//! should be readable with `cat` while debugging the algebra.

use crate::repo::Repo;
use std::io;
use std::path::Path;

/// Create `.v0/` and an empty log. Fails if one already exists.
pub fn init(root: &Path, replica_seed: u64) -> io::Result<Repo> {
    todo!()
}

/// Load a repository by walking up from `cwd` to find `.v0/`.
pub fn load(cwd: &Path) -> io::Result<(Repo, std::path::PathBuf)> {
    todo!()
}

/// Persist log and head. Write-to-temp-and-rename, so an interrupted save
/// cannot leave a half-written log — the same atomic-replace trick that makes
/// filesystem-level op capture hard, used here on purpose.
pub fn save(root: &Path, repo: &Repo) -> io::Result<()> {
    todo!()
}

/// Write the materialised document into the working copy.
pub fn checkout(root: &Path, repo: &Repo) -> io::Result<()> {
    todo!()
}
