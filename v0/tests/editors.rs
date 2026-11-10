//! The two plugins, in the two real editors, on one document: Neovim hosts,
//! Emacs joins, both headless. They type to each other live; then both go
//! offline, Alice renames `foo` from Neovim while Bob adds a call to `foo`
//! in Emacs, and when they come back his call reads `bar` in both buffers.
//!
//! Needs `nvim` and `emacs` on PATH, and fails without them: a demonstration
//! that is skipped demonstrates nothing. The scripts are `editors/test/`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_else(|e| format!("<{name}: {e}>"))
}

#[test]
fn neovim_and_emacs_share_a_document_and_a_rename() {
    // A Unix socket's path is short (104 bytes on macOS): not under target/.
    let dir = PathBuf::from(format!("/tmp/v0-editors-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for seat in ["alice", "bob"] {
        std::fs::create_dir_all(dir.join(seat)).unwrap();
    }
    let editors = Path::new(env!("CARGO_MANIFEST_DIR")).join("editors");
    let run = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .env("V0_BIN", env!("CARGO_BIN_EXE_v0"))
            .env("V0_TEST_DIR", &dir)
            .env("V0_EDITORS", &editors)
            .stdin(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("{program}: {e} (this test needs nvim and emacs)"))
    };
    let alice_lua = editors.join("test/alice.lua");
    let bob_el = editors.join("test/bob.el");
    let mut alice = run("nvim", &["--headless", "-u", "NONE", "-i", "NONE", "-n", "-l", alice_lua.to_str().unwrap()]);
    // Bob joins once Alice listens.
    let start = Instant::now();
    while !dir.join("s").exists() {
        assert!(start.elapsed() < Duration::from_secs(15), "no socket: {}", read(&dir, "alice.failed"));
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut bob = run("emacs", &["--batch", "-Q", "-l", bob_el.to_str().unwrap()]);
    let (bob, alice) = (bob.wait().unwrap(), alice.wait().unwrap());
    assert!(
        alice.success() && bob.success(),
        "alice: {alice}, bob: {bob}\n{}\n{}",
        read(&dir, "alice.failed"),
        read(&dir, "bob.failed")
    );

    // Vacuity: they were apart, each with only their own half.
    assert!(read(&dir, "alice.renamed").contains("fn bar() {}\nfn main() {\n    bar();\n}\n"));
    assert!(read(&dir, "bob.typed").contains("fn foo() {}\nfn main() {\n    foo();\n    foo();\n}\n"));
    let merged = "fn bar() {}\nfn main() {\n    bar();\n    bar();\n}\n// bob here\n// alice hère\n";
    assert_eq!(read(&dir, "alice.out"), merged, "Neovim");
    assert_eq!(read(&dir, "bob.out"), merged, "Emacs");
    std::fs::remove_dir_all(&dir).unwrap();
}
