//! End-to-end through the real binary and a real filesystem — the seam where
//! the last two bugs lived, both invisible to the in-memory tests.

use std::path::{Path, PathBuf};
use std::process::Command;

fn v0(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_v0"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run v0");
    let text = String::from_utf8_lossy(&out.stdout).into_owned()
        + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

fn fresh(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("v0-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A removal that arrives from a peer must leave the peer's disk. It used to
/// linger, and the next `record` then restored it -- silently undoing the
/// removal on the other side.
#[test]
fn a_removal_synced_from_a_peer_leaves_the_disk_and_stays_removed() {
    let root = fresh("removal");
    let (alice, bob) = (root.join("alice"), root.join("bob"));
    std::fs::create_dir_all(&alice).unwrap();
    std::fs::create_dir_all(&bob).unwrap();

    v0(&alice, &["init"]);
    std::fs::write(alice.join("gone.txt"), "a\nb\n").unwrap();
    v0(&alice, &["record", "-m", "base"]);
    v0(&bob, &["init"]);
    v0(&bob, &["sync", alice.to_str().unwrap()]);
    assert!(bob.join("gone.txt").exists());

    std::fs::remove_file(alice.join("gone.txt")).unwrap();
    v0(&alice, &["record", "-m", "remove it"]);
    v0(&bob, &["sync", alice.to_str().unwrap()]);

    assert!(!bob.join("gone.txt").exists(), "the removal reached bob's disk");
    let (_, out) = v0(&bob, &["record", "-m", "noop"]);
    assert!(out.contains("nothing to record"), "and bob does not resurrect it: {out}");
}

/// The bug that motivated file-level conflicts, through the CLI: a rename on
/// one side and an edit on the other must surface as a conflict, not vanish.
#[test]
fn rename_versus_edit_surfaces_as_a_conflict() {
    let root = fresh("rename");
    let (alice, bob) = (root.join("alice"), root.join("bob"));
    std::fs::create_dir_all(&alice).unwrap();
    std::fs::create_dir_all(&bob).unwrap();

    v0(&alice, &["init"]);
    std::fs::write(alice.join("old.txt"), "one\ntwo\n").unwrap();
    v0(&alice, &["record", "-m", "base"]);
    v0(&bob, &["init"]);
    v0(&bob, &["sync", alice.to_str().unwrap()]);

    std::fs::rename(alice.join("old.txt"), alice.join("new.txt")).unwrap();
    v0(&alice, &["record", "-m", "rename"]);
    std::fs::write(bob.join("old.txt"), "one\nTWO by bob\n").unwrap();
    v0(&bob, &["record", "-m", "edit"]);
    v0(&alice, &["sync", bob.to_str().unwrap()]);

    let (code, out) = v0(&alice, &["conflicts"]);
    assert_eq!(code, 4, "unresolved conflicts exit 4: {out}");
    let shown = std::fs::read_to_string(alice.join("old.txt")).unwrap();
    assert!(shown.contains("TWO by bob"), "bob's edit is on disk to be decided on: {shown}");
}

/// TigerStyle: the budget is fixed at startup and exceeding it is a loud,
/// immediate failure with its own exit code -- never a machine grinding into
/// swap, which is what a runaway test did before the budget existed.
#[test]
fn exceeding_the_memory_budget_fails_loudly_with_exit_5() {
    let dir = fresh("budget");
    v0(&dir, &["init"]);
    let big: String = (0..200_000).map(|i| format!("line {i} with some padding text\n")).collect();
    std::fs::write(dir.join("big.txt"), big).unwrap();

    let (code, out) = v0(&dir, &["--memory", "4", "record", "-m", "too big"]);
    assert_eq!(code, 5, "exhausting the budget has its own exit code: {out}");
    assert!(out.contains("memory budget of 4 MB exhausted"), "and says why: {out}");

    let (code, _) = v0(&dir, &["record", "-m", "fits"]);
    assert_eq!(code, 0, "the same work fits in the default budget");
}
