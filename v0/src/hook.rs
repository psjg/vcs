//! Hooks: local programs v0 runs at fixed points, and obeys.
//!
//! ```text
//! .v0/hooks/pre-record    before `record` and `resolve` capture the working
//!                         copy; a nonzero exit refuses the record (exit 6)
//! ```
//!
//! A hook runs with the repository root as its working directory, stdin closed,
//! its output passed through, and this environment:
//!
//! | variable     | value                                  |
//! |--------------|----------------------------------------|
//! | `V0_ROOT`    | the repository root, canonical         |
//! | `V0_ACTION`  | the command: `record` or `resolve`     |
//! | `V0_MESSAGE` | the `-m` text                          |
//!
//! v0 has no staging area -- the working copy *is* what gets recorded -- so a
//! hook testing the working copy tests exactly the change about to be made.
//! It runs before capture, so a refusal leaves nothing half-minted.
//!
//! Hooks are local and never travel. They live under `.v0/`, which is neither
//! scanned nor synced, so no peer can make you run their code. (ADR-0015.)
//!
//! A hook file that exists but is not executable is refused, not skipped: git
//! skips it silently, and a guard you believe in that never runs is worse than
//! none.
//!
//! This is the only module that starts processes.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

/// Why a hook stopped the command.
#[derive(Debug)]
pub enum Refusal {
    /// The hook ran and said no; its output has already been shown.
    Failed { name: String, code: Option<i32> },
    /// The hook is there but cannot run.
    Broken { name: String, why: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Failed { name, code: Some(c) } => write!(f, "hook {name} refused (exit {c})"),
            Refusal::Failed { name, code: None } => write!(f, "hook {name} refused (killed by a signal)"),
            Refusal::Broken { name, why } => write!(f, "hook {name} cannot run: {why}"),
        }
    }
}

/// Run `.v0/hooks/<name>` if it exists. No hook is a yes.
pub fn run(root: &Path, name: &str, env: &[(&str, &str)]) -> Result<(), Refusal> {
    let path = root.join(".v0/hooks").join(name);
    let Ok(meta) = std::fs::metadata(&path) else { return Ok(()) };
    let broken = |why: String| Refusal::Broken { name: name.to_owned(), why };
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(broken(format!("{} is not executable (chmod +x it, or remove it)", path.display())));
    }
    let root = root.canonicalize().map_err(|e| broken(e.to_string()))?;
    let status = Command::new(&path)
        .current_dir(&root)
        .env("V0_ROOT", &root)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .status()
        .map_err(|e| broken(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(Refusal::Failed { name: name.to_owned(), code: status.code() })
    }
}
