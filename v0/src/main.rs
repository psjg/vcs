//! argv in, exit code out. No logic lives here — every command is a few lines
//! of glue over the pure core.
//!
//! ```text
//! v0 init                 start a repository here
//! v0 status               what is tracked, what changed, how big the head is
//! v0 record -m MESSAGE    name the edits — one change per dependency component
//! v0 log                  changes, newest first
//! v0 show <id>            one change: its events, its dependencies
//! v0 deps <id>            what must come along if you take it
//! v0 adopt <id>           cherry-pick: the change and what it needs
//! v0 drop <id>            revert: the change and whatever depends on it
//! v0 checkout             rewrite the working copy from the head
//! v0 sync <path>          exchange with another v0 repository, no server
//! v0 reduction            measure derived dependencies against causal ones
//! ```
//!
//! Exit codes follow the lesson from the pijul harness — a dependency violation
//! is a *state* to handle, not a crash:
//!
//! | code | meaning |
//! |------|---------|
//! | 0 | success |
//! | 1 | failure |
//! | 2 | usage |
//! | 3 | change set not dependency-closed |

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use v0::change::{ChangeId, ChangeSet, Meta};
use v0::event::EventLog;
use v0::op::{EventId, NodeId, NodeKind, Op};
use v0::repo::Repo;
use v0::{capture, replay, store, sync};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];

    match run(cmd, rest) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail::Usage(msg)) => {
            eprintln!("usage: {msg}");
            ExitCode::from(2)
        }
        Err(Fail::NotClosed(msg)) => {
            eprintln!("dependency violation: {msg}");
            ExitCode::from(3)
        }
        Err(Fail::Other(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

enum Fail {
    Usage(String),
    NotClosed(String),
    Other(String),
}

impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Self {
        Fail::Other(e.to_string())
    }
}

fn run(cmd: &str, args: &[String]) -> Result<(), Fail> {
    match cmd {
        "init" => init(),
        "status" => status(),
        "record" => record(args),
        "log" => log_cmd(),
        "show" => show(arg(args, 0, "v0 show <id>")?),
        "deps" => deps(arg(args, 0, "v0 deps <id>")?),
        "adopt" => pick(arg(args, 0, "v0 adopt <id>")?, true),
        "drop" => pick(arg(args, 0, "v0 drop <id>")?, false),
        "checkout" => checkout(),
        "sync" => sync_cmd(Path::new(arg(args, 0, "v0 sync <path>")?)),
        "reduction" => reduction(),
        _ => Err(Fail::Usage(
            "v0 <init|status|record|log|show|deps|adopt|drop|checkout|sync|reduction>".into(),
        )),
    }
}

fn arg<'a>(args: &'a [String], n: usize, help: &str) -> Result<&'a str, Fail> {
    args.get(n).map(String::as_str).ok_or_else(|| Fail::Usage(help.into()))
}

fn here() -> Result<(Repo, PathBuf), Fail> {
    Ok(store::load(&std::env::current_dir()?)?)
}

fn init() -> Result<(), Fail> {
    let root = std::env::current_dir()?;
    // A replica id per working copy, not per human: two clones by the same
    // person must not mint colliding event ids.
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    store::init(&root, seed)?;
    println!("initialised v0 repository in {}", root.join(".v0").display());
    Ok(())
}

fn status() -> Result<(), Fail> {
    let (repo, root) = here()?;
    let current = repo.worktree();
    let disk = store::scan(&root)?;

    let mut changed: Vec<&String> = disk
        .files
        .iter()
        .filter(|(p, lines)| current.files.get(*p).map(|c| c != *lines).unwrap_or(true))
        .map(|(p, _)| p)
        .collect();
    let gone: Vec<&String> = current.files.keys().filter(|p| !disk.files.contains_key(*p)).collect();
    changed.sort();

    println!("head      {} changes, {} events", repo.head.ids().len(), repo.log.events.len());
    println!("tracked   {} files", current.files.len());
    let unnamed = repo.unnamed().len();
    if unnamed > 0 {
        println!("unnamed   {unnamed} events not in any change");
    }
    if changed.is_empty() && gone.is_empty() {
        println!("clean");
    } else {
        for p in changed {
            println!("  modified  {p}");
        }
        for p in gone {
            println!("  deleted   {p}");
        }
    }
    Ok(())
}

fn record(args: &[String]) -> Result<(), Fail> {
    let message = match args.iter().position(|a| a == "-m") {
        Some(i) => args.get(i + 1).cloned().unwrap_or_default(),
        None => return Err(Fail::Usage("v0 record -m MESSAGE".into())),
    };
    let (mut repo, root) = here()?;

    let all: Vec<EventId> = repo.changes.events_of(repo.head.ids()).into_iter().collect();
    let current = replay::materialise(&all, &repo.log);
    let disk = store::scan(&root)?;

    let mut minted: BTreeSet<EventId> = BTreeSet::new();
    // What capture observed belongs together, passed to `record` alongside what
    // the graph can derive.
    let mut hints: Vec<BTreeSet<EventId>> = Vec::new();
    for (path, lines) in &disk.files {
        let text = {
            let mut t = lines.join("\n");
            if !t.is_empty() {
                t.push('\n');
            }
            t
        };
        let node = match current.nodes.get(path) {
            Some(n) => *n,
            None => {
                // A file v0 has not seen: mint its node first, so the lines
                // that follow have something to anchor to.
                let node = NodeId(EventId { seq: repo.next_seq, replica: repo.replica });
                let name = path.rsplit('/').next().unwrap_or(path).to_owned();
                let op = Op::Create { node, parent: Op::ROOT, name, kind: NodeKind::File };
                minted.insert(repo.log.append(repo.replica, &mut repo.next_seq, op));
                node
            }
        };
        let before = current.files.get(path).cloned().unwrap_or_default();
        let cap = capture::from_save(&before, node, &text, repo.replica, repo.next_seq, &repo.log);
        let ids = repo.log.append_batch(repo.replica, &mut repo.next_seq, cap.ops);
        hints.extend(
            cap.hunks.iter().map(|h| h.iter().map(|i| ids[*i]).collect::<BTreeSet<EventId>>()),
        );
        minted.extend(ids);
    }
    for (path, node) in &current.nodes {
        if !disk.files.contains_key(path) {
            let op = Op::Remove { node: *node };
            minted.insert(repo.log.append(repo.replica, &mut repo.next_seq, op));
        }
    }

    if minted.is_empty() {
        println!("nothing to record");
        return Ok(());
    }
    let meta = Meta { message, author: whoami() };
    let made = repo.record(minted, &hints, meta);
    store::save(&root, &repo)?;

    // One commit's worth of edits routinely becomes several changes: that is
    // ADR-0007 doing its job, not an accident.
    println!("recorded {} change(s):", made.len());
    for id in &made {
        let ch = &repo.changes.by_id[id];
        println!("  {}  {} events, {} deps", id.short(), ch.events.len(), ch.deps.len());
    }
    Ok(())
}

fn log_cmd() -> Result<(), Fail> {
    let (repo, _) = here()?;
    let mut ids: Vec<&ChangeId> = repo.head.ids().iter().collect();
    // Newest first, by the highest event each change names.
    ids.sort_by_key(|id| std::cmp::Reverse(repo.changes.by_id[id].events.iter().max().copied()));
    for id in ids {
        let ch = &repo.changes.by_id[id];
        println!("{}  {:<40}  {} events, {} deps", id.short(), ch.meta.message, ch.events.len(), ch.deps.len());
    }
    Ok(())
}

fn show(prefix: &str) -> Result<(), Fail> {
    let (repo, _) = here()?;
    let id = repo.changes.resolve(prefix).map_err(|e| Fail::Other(e.into()))?;
    let ch = &repo.changes.by_id[&id];
    println!("change   {id}");
    println!("message  {}", ch.meta.message);
    println!("author   {}", ch.meta.author);
    println!("events   {}", ch.events.len());
    for e in ch.events.iter().take(20) {
        if let Some(ev) = repo.log.events.get(e) {
            println!("  {:?}", ev.op);
        }
    }
    if ch.events.len() > 20 {
        println!("  ... {} more", ch.events.len() - 20);
    }
    println!("deps     {}", ch.deps.len());
    for d in &ch.deps {
        println!("  {}  {}", d.short(), repo.changes.by_id[d].meta.message);
    }
    Ok(())
}

fn deps(prefix: &str) -> Result<(), Fail> {
    let (repo, _) = here()?;
    let id = repo.changes.resolve(prefix).map_err(|e| Fail::Other(e.into()))?;
    let closure = repo.changes.closure(id);
    let causal: BTreeSet<EventId> = repo.changes.by_id[&id]
        .events
        .iter()
        .flat_map(|e| repo.log.causal_closure(*e))
        .collect();
    println!("taking {} brings {} change(s):", id.short(), closure.len());
    for c in &closure {
        println!("  {}  {}", c.short(), repo.changes.by_id[c].meta.message);
    }
    println!(
        "\n{} events derived, against {} the author had seen",
        repo.changes.events_of(&closure).len(),
        causal.len()
    );
    Ok(())
}

fn pick(prefix: &str, adopt: bool) -> Result<(), Fail> {
    let (mut repo, root) = here()?;
    let id = repo.changes.resolve(prefix).map_err(|e| Fail::Other(e.into()))?;
    let before = repo.head.ids().len();
    repo.head = if adopt { repo.adopt(id) } else { repo.drop_change(id) };
    store::save(&root, &repo)?;
    store::checkout(&root, &repo)?;
    let after = repo.head.ids().len();
    println!(
        "{} {}: head {before} -> {after} changes",
        if adopt { "adopted" } else { "dropped" },
        id.short()
    );
    Ok(())
}

fn checkout() -> Result<(), Fail> {
    let (repo, root) = here()?;
    store::checkout(&root, &repo)?;
    println!("wrote {} file(s)", repo.worktree().files.len());
    Ok(())
}

/// Exchange with another repository on disk. No server, no wire protocol —
/// state vectors and a set union are the whole of it.
fn sync_cmd(other: &Path) -> Result<(), Fail> {
    let (mut repo, root) = here()?;
    let (theirs, _) = store::load(other)?;

    let before = repo.log.events.len();
    let incoming = sync::missing(&theirs.log, &sync::state_vector(&repo.log));
    let count = incoming.len();
    sync::integrate(&mut repo.log, incoming);
    for (id, ch) in theirs.changes.by_id {
        repo.changes.by_id.entry(id).or_insert(ch);
    }
    let union: BTreeSet<ChangeId> =
        repo.head.ids().union(theirs.head.ids()).copied().collect();
    repo.head = ChangeSet::new(union, &repo.changes)
        .map_err(|e| Fail::NotClosed(format!("{e:?}")))?;

    store::save(&root, &repo)?;
    store::checkout(&root, &repo)?;
    println!(
        "pulled {count} events ({before} -> {}), head is {} changes",
        repo.log.events.len(),
        repo.head.ids().len()
    );
    Ok(())
}

fn reduction() -> Result<(), Fail> {
    let (repo, _) = here()?;
    let ratios = repo.changes.reduction_ratio(&repo.log);
    if ratios.is_empty() {
        println!("nothing recorded yet");
        return Ok(());
    }
    let mut v: Vec<f64> = ratios.values().copied().collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "reduction over {} changes:  median {:.1}x   mean {:.1}x   max {:.1}x",
        v.len(),
        v[v.len() / 2],
        v.iter().sum::<f64>() / v.len() as f64,
        v.last().unwrap()
    );
    Ok(())
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".into())
}

/// Keeps `EventLog` in scope for the type checker's benefit in `sync_cmd`.
const _: fn() -> EventLog = EventLog::default;
