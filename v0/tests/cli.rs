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
#[cfg(not(feature = "system-alloc"))] // a profiling build has no budget by design
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

/// Resolve first: while a conflict is open, nothing that could add another one
/// runs. The escape hatch works, and says it is not the way.
#[test]
fn nothing_new_while_a_conflict_is_open() {
    let root = fresh("gate");
    let (alice, bob) = (root.join("alice"), root.join("bob"));
    std::fs::create_dir_all(&alice).unwrap();
    std::fs::create_dir_all(&bob).unwrap();
    v0(&alice, &["init"]);
    std::fs::write(alice.join("g.txt"), "hallo mooie wereld\n").unwrap();
    v0(&alice, &["record", "-m", "base"]);
    v0(&bob, &["init"]);
    v0(&bob, &["sync", alice.to_str().unwrap()]);

    // The same word, changed differently, without syncing in between.
    std::fs::write(alice.join("g.txt"), "hallo prachtige wereld\n").unwrap();
    v0(&alice, &["record", "-m", "alice"]);
    std::fs::write(bob.join("g.txt"), "hallo vrolijke wereld\n").unwrap();
    v0(&bob, &["record", "-m", "bob"]);
    v0(&alice, &["sync", bob.to_str().unwrap()]);
    assert_eq!(v0(&alice, &["conflicts"]).0, 4, "there is an open conflict");

    std::fs::write(alice.join("other.txt"), "unrelated\n").unwrap();
    let (code, out) = v0(&alice, &["record", "-m", "more"]);
    assert_eq!(code, 4, "record refuses: {out}");
    assert!(out.contains("Resolve first"), "and says what to do: {out}");
    assert_eq!(v0(&alice, &["sync", bob.to_str().unwrap()]).0, 4, "sync refuses too");

    let (code, out) = v0(&alice, &["record", "-m", "more", "--despite-conflicts"]);
    assert_eq!(code, 0, "the escape hatch works: {out}");
    assert!(out.contains("not the intended workflow"), "and says it is not the way: {out}");
}

/// Two peers rewriting two words of one line differently: two conflicts drawn
/// in one block, which one edit and one `resolve --file` must settle together.
fn two_conflicts_on_one_line(name: &str) -> PathBuf {
    let root = fresh(name);
    let (alice, bob) = (root.join("alice"), root.join("bob"));
    std::fs::create_dir_all(&alice).unwrap();
    std::fs::create_dir_all(&bob).unwrap();
    v0(&alice, &["init"]);
    std::fs::write(alice.join("g.txt"), "een mooie zin\n").unwrap();
    v0(&alice, &["record", "-m", "base"]);
    v0(&bob, &["init"]);
    v0(&bob, &["sync", alice.to_str().unwrap()]);
    std::fs::write(alice.join("g.txt"), "twee mooie regels\n").unwrap();
    v0(&alice, &["record", "-m", "alice"]);
    std::fs::write(bob.join("g.txt"), "drie mooie woorden\n").unwrap();
    v0(&bob, &["record", "-m", "bob"]);
    v0(&alice, &["sync", bob.to_str().unwrap()]);
    let (_, out) = v0(&alice, &["conflicts"]);
    assert!(out.contains("2 unresolved"), "fixture makes two conflicts: {out}");
    alice
}

#[test]
fn one_resolve_settles_every_conflict_in_a_file() {
    let alice = two_conflicts_on_one_line("resolve-file");
    std::fs::write(alice.join("g.txt"), "twee mooie woorden\n").unwrap();
    let (code, out) = v0(&alice, &["resolve", "--file", "g.txt", "-m", "alice's number, bob's noun"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(v0(&alice, &["conflicts"]).0, 0, "both closed by one resolution");
    assert_eq!(std::fs::read_to_string(alice.join("g.txt")).unwrap(), "twee mooie woorden\n");
}

#[test]
fn resolve_takes_several_ids_or_none_for_all() {
    let alice = two_conflicts_on_one_line("resolve-all");
    std::fs::write(alice.join("g.txt"), "drie mooie regels\n").unwrap();
    let (code, out) = v0(&alice, &["resolve", "-m", "everything at once"]);
    assert_eq!(code, 0, "{out}");
    let named = out.split_whitespace().skip(1).take_while(|w| *w != "with").count();
    assert_eq!(named, 2, "one resolution names both conflicts: {out}");
    assert_eq!(v0(&alice, &["conflicts"]).0, 0);
}

/// Three people change the same word three ways. Every section of the rendered
/// conflict must be exactly what that person saved, and `before` what they all
/// started from -- the playground once rendered "hoihallo vrolijke
/// aardewereld", a line nobody had typed.
#[test]
fn a_three_way_conflict_shows_what_each_author_wrote() {
    let root = fresh("threeway");
    let peers: Vec<PathBuf> = ["alice", "bob", "carol"].iter().map(|n| root.join(n)).collect();
    for p in &peers {
        std::fs::create_dir_all(p).unwrap();
    }
    let (alice, bob, carol) = (&peers[0], &peers[1], &peers[2]);
    v0(alice, &["init"]);
    std::fs::write(alice.join("g.txt"), "een mooie zin\n").unwrap();
    v0(alice, &["record", "-m", "base"]);
    for p in [bob, carol] {
        v0(p, &["init"]);
        v0(p, &["sync", alice.to_str().unwrap()]);
    }

    let wrote = [
        (alice, "alice", "een prachtige zin\n"),
        (bob, "bob", "een vrolijke zin\n"),
        (carol, "carol", "een lelijke zin\n"),
    ];
    for (dir, who, text) in wrote {
        std::fs::write(dir.join("g.txt"), text).unwrap();
        v0(dir, &["record", "-m", who]);
    }
    // bob gathers carol, alice gathers bob: one sync brings all three together.
    v0(bob, &["sync", carol.to_str().unwrap()]);
    v0(alice, &["sync", bob.to_str().unwrap()]);

    let shown = std::fs::read_to_string(alice.join("g.txt")).unwrap();
    let mut sections = std::collections::BTreeMap::new();
    let mut current: Option<String> = None;
    for line in shown.lines() {
        if line.starts_with("||||||| before") {
            current = Some("before".into());
        } else if let Some(rest) = line.strip_prefix("======= ") {
            current = rest.split_whitespace().nth(1).map(str::to_owned);
        } else if line.starts_with("<<<<<<<") || line.starts_with(">>>>>>>") {
            current = None;
        } else if let Some(who) = &current {
            sections.entry(who.clone()).or_insert_with(String::new).push_str(&format!("{line}\n"));
        }
    }
    assert_eq!(sections.get("before").map(String::as_str), Some("een mooie zin\n"), "{shown}");
    for (_, who, text) in wrote {
        assert_eq!(sections.get(who).map(String::as_str), Some(text), "{who}'s section:\n{shown}");
    }
}
