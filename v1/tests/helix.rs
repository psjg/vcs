//! Two Helix instances (25.07), each in a tmux pane of a server of its own,
//! each with `v1 lsp` as the language server of Rust and Python, on one
//! document. Typed by this test a character at a time, as
//! `v0/editors/demo.sh` and `v0/tests/editors.rs` do for Neovim and Emacs,
//! and read back with `:w`.
//!
//! The scene: both type a line the other sees; both go offline
//! (`:lsp-workspace-command v1.hold`); Alice renames `foo` from its call
//! (`space r`: Helix's rename-to prompt starts with the name under the
//! cursor, so `C-u` clears it first); Bob adds a call to `foo`; both come
//! back (`v1.release`), and Bob's call reads `bar` in both saved files.
//! Then the same in Python, and a `:format` in Rust.
//!
//! Needs `hx`, `tmux`, `rust-analyzer` and `basedpyright` on PATH, and
//! fails without them: a scene that is skipped shows nothing. Run it as
//! `nix shell nixpkgs#basedpyright -c pkg env rust -- cargo nextest run`.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long anything may take to show: rust-analyzer loads a project.
const SHOW: Duration = Duration::from_secs(90);

/// Run a program that must be there, and say which one is missing.
fn need(program: &str, arg: &str) {
    let ok = Command::new(program).arg(arg).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    assert!(ok.is_ok_and(|s| s.success()), "this test needs `{program}` on PATH");
}

/// Two editors on a stage under /tmp (a Unix socket's path is short).
struct Stage {
    dir: PathBuf,
    tmux: String,
}

#[derive(Clone, Copy)]
enum Seat {
    Alice,
    Bob,
}

impl Seat {
    fn name(self) -> &'static str {
        match self {
            Seat::Alice => "alice",
            Seat::Bob => "bob",
        }
    }
}

impl Stage {
    fn new(scene: &str) -> Stage {
        for (program, arg) in [("hx", "--version"), ("tmux", "-V"), ("rust-analyzer", "--version"), ("basedpyright", "--version")] {
            need(program, arg);
        }
        let dir = PathBuf::from(format!("/tmp/v1hx-{}-{scene}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tmux = format!("v1hx-{}-{scene}", std::process::id());
        let stage = Stage { dir, tmux };
        let _ = stage.tmux(&["kill-server"]);
        stage
    }

    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux").args(["-L", &self.tmux, "-f", "/dev/null"]).args(args).output().expect("tmux runs");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn socket(&self) -> PathBuf {
        self.dir.join("s")
    }

    fn file(&self, who: Seat, name: &str) -> PathBuf {
        self.dir.join(who.name()).join("proj").join(name)
    }

    /// A project for `who` holding `files`, and the Helix configuration
    /// that points Rust and Python at `v1 lsp`.
    fn project(&self, who: Seat, files: &[(&str, &str)]) {
        let home = self.dir.join(who.name());
        for (name, text) in files {
            let path = home.join("proj").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let v1 = env!("CARGO_BIN_EXE_v1");
        let socket = self.socket();
        let config = home.join("xdg/helix");
        std::fs::create_dir_all(&config).unwrap();
        let languages = format!(
            r#"[language-server.v1-rust]
command = "{v1}"
args = ["lsp", "--socket", "{socket}", "--", "rust-analyzer"]

[language-server.v1-python]
command = "{v1}"
args = ["lsp", "--socket", "{socket}", "--", "basedpyright-langserver", "--stdio"]

[[language]]
name = "rust"
language-servers = ["v1-rust"]

[[language]]
name = "python"
language-servers = ["v1-python"]
"#,
            socket = socket.display()
        );
        std::fs::write(config.join("languages.toml"), languages).unwrap();
        std::fs::write(config.join("config.toml"), "[editor]\nauto-pairs = false\nauto-completion = false\nauto-format = false\n").unwrap();
    }

    /// Start Helix on `file` for `who`, in a pane of its own.
    fn start(&self, who: Seat, file: &str) {
        let home = self.dir.join(who.name());
        let command = format!(
            "cd {proj} && exec env XDG_CONFIG_HOME={xdg} PATH={path} hx {file}",
            proj = home.join("proj").display(),
            xdg = home.join("xdg").display(),
            path = std::env::var("PATH").unwrap()
        );
        self.tmux(&["new-session", "-d", "-s", who.name(), "-x", "100", "-y", "30", &command]);
        self.tmux(&["set", "-s", "escape-time", "0"]);
    }

    /// Type like a person: a character at a time.
    fn type_(&self, who: Seat, text: &str) {
        for c in text.chars() {
            // A bare ";" ends a tmux command rather than being sent.
            let c = if c == ';' { "\\;".to_owned() } else { c.to_string() };
            self.tmux(&["send-keys", "-t", who.name(), "-l", &c]);
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn key(&self, who: Seat, key: &str) {
        self.tmux(&["send-keys", "-t", who.name(), key]);
        std::thread::sleep(Duration::from_millis(100));
    }

    /// An ex command, with Helix's own `:` prompt.
    fn ex(&self, who: Seat, command: &str) {
        self.type_(who, &format!(":{command}"));
        self.key(who, "Enter");
    }

    fn screen(&self, who: Seat) -> String {
        self.tmux(&["capture-pane", "-p", "-J", "-t", who.name()])
    }

    /// Wait until the screen satisfies `done`; else fail with the screens.
    fn wait(&self, who: Seat, what: &str, done: impl Fn(&str) -> bool) {
        let start = Instant::now();
        while !done(&self.screen(who)) {
            assert!(
                start.elapsed() < SHOW,
                "{}: waiting for {what}\n--- alice\n{}\n--- bob\n{}",
                who.name(),
                self.screen(Seat::Alice),
                self.screen(Seat::Bob)
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Save the buffer and read the file back.
    fn saved(&self, who: Seat, name: &str) -> String {
        self.ex(who, "w");
        let path = self.file(who, name);
        std::thread::sleep(Duration::from_millis(500));
        std::fs::read_to_string(&path).unwrap_or_else(|e| format!("<{}: {e}>", path.display()))
    }

    /// Alice hosts, once her language server listens; Bob joins and is
    /// sent the document.
    fn open(&self, alice: &str, bob: &str, document: &str) {
        self.start(Seat::Alice, alice);
        let start = Instant::now();
        while !self.socket().exists() {
            assert!(start.elapsed() < SHOW, "alice's v1 does not listen\n{}", self.screen(Seat::Alice));
            std::thread::sleep(Duration::from_millis(100));
        }
        self.start(Seat::Bob, bob);
        let first = document.lines().next().unwrap().to_owned();
        self.wait(Seat::Bob, "the document", |s| s.contains(&first));
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What differs between the languages in the rename scene.
struct Language {
    scene: &'static str,
    /// The project: (path, text) of Alice's, and the file she opens.
    files: &'static [(&'static str, &'static str)],
    file: &'static str,
    /// Where the call is, once the comment is on top of the file.
    call_line: usize,
    comment: &'static str,
    call: &'static str,
    /// The merged file.
    merged: &'static str,
}

/// Both type, both hold, Alice renames from the call, Bob adds a call,
/// both release.
fn rename_scene(l: &Language) {
    let s = Stage::new(l.scene);
    s.project(Seat::Alice, l.files);
    let bob: Vec<(&str, &str)> = l.files.iter().map(|(p, t)| (*p, if *p == l.file { "" } else { *t })).collect();
    s.project(Seat::Bob, &bob);
    let document = l.files.iter().find(|(p, _)| *p == l.file).unwrap().1;
    s.open(l.file, l.file, document);
    let (alice, bob) = (Seat::Alice, Seat::Bob);

    // Each types a line the other sees.
    s.type_(alice, &format!("ggO{} alice", l.comment));
    s.key(alice, "Escape");
    s.wait(bob, "alice's line", |t| t.contains(&format!("{} alice", l.comment)));
    s.type_(bob, &format!("Go{} bob", l.comment));
    s.key(bob, "Escape");
    s.wait(alice, "bob's line", |t| t.contains(&format!("{} bob", l.comment)));

    // Offline, both.
    s.ex(alice, "lsp-workspace-command v1.hold");
    s.ex(bob, "lsp-workspace-command v1.hold");
    std::thread::sleep(Duration::from_secs(1));

    // Alice renames foo from its call.
    s.type_(alice, &format!("{}Ggs", l.call_line));
    s.type_(alice, " r");
    s.wait(alice, "the rename prompt", |t| t.contains("rename-to:"));
    s.key(alice, "C-u");
    s.type_(alice, "bar");
    s.key(alice, "Enter");
    s.wait(alice, "the rename", |t| !t.contains("foo") && t.contains("bar"));
    s.wait(bob, "bob unaware of the rename", |t| t.contains("foo"));

    // Bob adds a call, not having seen it.
    s.type_(bob, &format!("{}Go{}", l.call_line, l.call));
    s.key(bob, "Escape");

    s.ex(alice, "lsp-workspace-command v1.release");
    s.ex(bob, "lsp-workspace-command v1.release");
    let call = l.call.replace("foo", "bar");
    let calls = |t: &str| t.lines().filter(|line| line.trim_end().ends_with(&call)).count() == 2 && !t.contains("foo");
    s.wait(alice, "the merge", calls);
    s.wait(bob, "the merge", calls);

    let (a, b) = (s.saved(alice, l.file), s.saved(bob, l.file));
    assert_eq!(a, l.merged, "alice's file");
    assert_eq!(b, l.merged, "bob's file");
}

#[test]
fn a_rename_reaches_a_call_added_apart_in_rust() {
    rename_scene(&Language {
        scene: "rust",
        files: &[
            ("Cargo.toml", "[package]\nname = \"scene\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
            ("src/main.rs", "fn foo() {}\nfn main() {\n    foo();\n}\n"),
        ],
        file: "src/main.rs",
        call_line: 4,
        comment: "//",
        call: "foo();",
        merged: "// alice\nfn bar() {}\nfn main() {\n    bar();\n    bar();\n}\n// bob\n",
    });
}

#[test]
fn a_rename_reaches_a_call_added_apart_in_python() {
    rename_scene(&Language {
        scene: "python",
        files: &[("main.py", "def foo():\n    pass\n\n\ndef main():\n    foo()\n")],
        file: "main.py",
        call_line: 7,
        comment: "#",
        call: "foo()",
        // Helix indents the line it opens after an indented one.
        merged: "# alice\ndef bar():\n    pass\n\n\ndef main():\n    bar()\n    bar()\n    # bob\n",
    });
}

/// `:format` in Rust: Helix asks for `textDocument/formatting`, is
/// answered with nothing, and both buffers get the formatted text pushed.
#[test]
fn a_format_reaches_both_buffers() {
    let s = Stage::new("format");
    let text = "fn main() {\nlet   a=1;\nlet b  =  2;\n}\n";
    let files = [
        ("Cargo.toml", "[package]\nname = \"scene\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/main.rs", text),
    ];
    s.project(Seat::Alice, &files);
    s.project(Seat::Bob, &[files[0], ("src/main.rs", "")]);
    s.open("src/main.rs", "src/main.rs", text);
    s.ex(Seat::Alice, "format");
    let done = |t: &str| t.contains("let a = 1;") && t.contains("let b = 2;");
    s.wait(Seat::Alice, "the format", done);
    s.wait(Seat::Bob, "the format", done);
    let want = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n";
    assert_eq!(s.saved(Seat::Alice, "src/main.rs"), want);
    assert_eq!(s.saved(Seat::Bob, "src/main.rs"), want);
}
