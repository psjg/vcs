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
//! v0 conflicts [--all]    open conflicts; --all adds resolved ones and why
//! v0 resolve [<id>...] [--file PATH] -m WHY
//!                         record your edit as the resolution of those conflicts
//!                         (default: every open conflict)
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
//! | 4 | unresolved conflicts — a state, like pijul's, not a crash |
//! | 5 | memory budget exhausted (see `--memory`) |
//! | 6 | a hook refused (see [`v0::hook`]; `--despite-hooks` pushes through) |
//!
//! `--memory MB`, anywhere on the command line, fixes the heap budget for the
//! run (default 512, or `V0_MEMORY_MB`). It is reserved at startup and never
//! exceeded; see [`v0::budget`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use v0::change::{ChangeId, ChangeSet, Meta};
use v0::event::EventLog;
use v0::op::{EventId, NodeId, NodeKind, Op};
use v0::repo::Repo;
use v0::{capture, conflict, hook, replay, store, sync};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();

    // TigerStyle: the memory budget is fixed before any real work starts.
    let budget = match take_memory_flag(&mut args) {
        Ok(mb) => mb,
        Err(msg) => {
            eprintln!("usage: {msg}");
            return ExitCode::from(2);
        }
    };
    if let Err(msg) = v0::budget::reserve_mb(budget) {
        eprintln!("error: {msg}");
        return ExitCode::FAILURE;
    }

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
        Err(Fail::Conflicts(msg)) => {
            eprintln!("{msg}");
            ExitCode::from(4)
        }
        Err(Fail::Hook(msg)) => {
            eprintln!("{msg}");
            ExitCode::from(6)
        }
        Err(Fail::Other(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// `--memory MB` if given (and removed from `args`), else `V0_MEMORY_MB`, else
/// the default.
fn take_memory_flag(args: &mut Vec<String>) -> Result<usize, String> {
    if let Some(i) = args.iter().position(|a| a == "--memory") {
        let value = args.get(i + 1).cloned().ok_or("--memory needs a size in MB")?;
        args.drain(i..=i + 1);
        return value.parse().map_err(|_| format!("--memory: not a number of MB: {value}"));
    }
    Ok(std::env::var("V0_MEMORY_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(v0::budget::DEFAULT_MB))
}

enum Fail {
    Usage(String),
    NotClosed(String),
    Conflicts(String),
    Hook(String),
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
        "adopt" => pick(arg(args, 0, "v0 adopt <id>")?, true, args),
        "drop" => pick(arg(args, 0, "v0 drop <id>")?, false, args),
        "checkout" => checkout(),
        "sync" => sync_cmd(Path::new(arg(args, 0, "v0 sync <path>")?), args),
        "reduction" => reduction(),
        "conflicts" => conflicts_cmd(args.iter().any(|a| a == "--all")),
        "resolve" => resolve_cmd(args),
        _ => Err(Fail::Usage(
            "v0 <init|status|record|log|show|deps|adopt|drop|checkout|sync|conflicts|resolve|reduction>"
                .into(),
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
    let current = store::render(&repo);
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
    match v0::budget::limit_bytes() {
        0 => println!("memory    system allocator (profiling build, NO budget)"),
        b => println!("memory    budget {} MB, reserved at startup", b >> 20),
    }
    let open = open_conflicts(&repo);
    if !open.is_empty() {
        println!("conflicts {} unresolved -- see `v0 conflicts`", open.len());
    }
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
    let mut args = args.to_vec();
    let message = message_arg(&args, "v0 record -m MESSAGE")?;
    let (mut repo, root) = here()?;
    let despite = gate_unresolved(&repo, &mut args, "record")?;
    let skip_hook = take_flag(&mut args, "--despite-hooks");
    gate_hook(&root, skip_hook, "record", &message)?;
    let (minted, hints) = capture_disk(&mut repo, &root, despite)?;
    if minted.is_empty() {
        println!("nothing to record");
        return Ok(());
    }
    let meta = Meta::new(message, whoami());
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

fn message_arg(args: &[String], usage: &str) -> Result<String, Fail> {
    match args.iter().position(|a| a == "-m") {
        Some(i) => Ok(args.get(i + 1).cloned().unwrap_or_default()),
        None => Err(Fail::Usage(usage.into())),
    }
}

/// Turn the working copy into events: diff every file against the materialised
/// head, mint nodes for new files, removes for vanished ones. Returns what was
/// minted and what the capture adapter saw belonged together.
///
/// Refuses while conflict markers are on disk: they are a drawing of a state,
/// not text anyone wrote, and recording them would put the drawing into history.
fn capture_disk(
    repo: &mut Repo,
    root: &Path,
    skip_marked: bool,
) -> Result<(BTreeSet<EventId>, Vec<BTreeSet<EventId>>), Fail> {
    let mut disk = store::scan(root)?;
    let marked: Vec<String> =
        disk.files.iter().filter(|(_, l)| store::has_markers(l)).map(|(p, _)| p.clone()).collect();
    // Pushing through open conflicts records everything *else*: a file still
    // showing markers is a drawing of the conflict, not text anyone wrote.
    if skip_marked {
        for p in &marked {
            eprintln!("warning: skipping {p}: it still shows an open conflict");
        }
    }
    let skipped: BTreeSet<String> = if skip_marked { marked.iter().cloned().collect() } else { BTreeSet::new() };
    disk.files.retain(|p, _| !skipped.contains(p));
    if !skip_marked && !marked.is_empty() {
        return Err(Fail::Conflicts(format!(
            "conflict markers still in: {} -- edit them away, then `v0 resolve <id> -m WHY`",
            marked.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        )));
    }

    let all: Vec<EventId> = repo.changes.events_of(repo.head.ids()).into_iter().collect();
    // The same view of the head that render wrote to disk -- agreed text once --
    // or the next record would "see" the hidden copy deleted and write that
    // down as a choice.
    let current = replay::materialise_head(&repo.head, &repo.changes, &repo.log, &BTreeSet::new());
    let mut minted: BTreeSet<EventId> = BTreeSet::new();
    let mut hints: Vec<BTreeSet<EventId>> = Vec::new();

    // Removed files, by the path they had. A removed file that reappears on disk
    // is *restored* -- same node, same lines, same history -- rather than being
    // recorded as a brand-new file that happens to have the same name.
    let tree = replay::tree_of(&all, &repo.log);
    let removed_at: std::collections::BTreeMap<String, NodeId> = tree
        .removed
        .iter()
        .filter_map(|n| tree.path_ignoring_removal(*n).map(|p| (p, *n)))
        .collect();

    for (path, text) in &disk.files {
        let mut before = current.files.get(path).cloned().unwrap_or_default();
        let node = match current.nodes.get(path) {
            Some(n) => *n,
            None if removed_at.contains_key(path) => {
                let node = removed_at[path];
                let op = Op::Restore { node };
                minted.insert(repo.log.append(repo.replica, &mut repo.next_seq, op));
                let revived =
                    replay::materialise_head(&repo.head, &repo.changes, &repo.log, &[node].into_iter().collect());
                before = revived.files.get(path).cloned().unwrap_or_default();
                node
            }
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
        let cap = capture::from_save(&before, node, text, repo.replica, repo.next_seq, &repo.log);
        let ids = repo.log.append_batch(repo.replica, &mut repo.next_seq, cap.ops);
        hints.extend(cap.hunks.iter().map(|h| h.iter().map(|i| ids[*i]).collect::<BTreeSet<_>>()));
        minted.extend(ids);
    }
    for (path, node) in &current.nodes {
        if !disk.files.contains_key(path) && !skipped.contains(path) {
            let op = Op::Remove { node: *node };
            minted.insert(repo.log.append(repo.replica, &mut repo.next_seq, op));
        }
    }
    Ok((minted, hints))
}

/// Resolve first. Nothing that could add a conflict -- recording, syncing,
/// adopting -- runs while one is open, the way git refuses a merge over
/// unmerged paths. Stacking conflicts on conflicts is how a working copy turns
/// into a tangle nobody can read (measured: five conflicts, one line, text no
/// one typed).
///
/// `--despite-conflicts` pushes through anyway. It exists because a hard stop
/// with no way past is worse than a loud one; it says so every time.
fn gate_unresolved(repo: &Repo, args: &mut Vec<String>, action: &str) -> Result<bool, Fail> {
    let despite = match args.iter().position(|a| a == "--despite-conflicts") {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
    let open = open_conflicts(repo);
    if open.is_empty() {
        return Ok(false);
    }
    let ids: Vec<String> = open.iter().map(|c| c.id.short()).collect();
    if despite {
        eprintln!(
            "warning: {action} with {} unresolved conflict(s) ({}). They stay open and will keep \
             being reported. This is not the intended workflow -- resolve first.",
            open.len(),
            ids.join(" ")
        );
        return Ok(true);
    }
    Err(Fail::Conflicts(format!(
        "refusing to {action}: {} unresolved conflict(s) ({}). Resolve first with `v0 resolve`, \
         or pass --despite-conflicts to push through anyway (not recommended).",
        open.len(),
        ids.join(" ")
    )))
}

/// Ask `.v0/hooks/pre-record` before capturing anything. `--despite-hooks`
/// skips it -- the same loud emergency exit as `--despite-conflicts`, for the
/// same reason: a hard stop with no way past is worse than a loud one.
fn gate_hook(root: &Path, skip: bool, action: &str, message: &str) -> Result<(), Fail> {
    if skip {
        eprintln!(
            "warning: {action} without running the pre-record hook. What it guards is \
             unchecked in this change. This is not the intended workflow."
        );
        return Ok(());
    }
    hook::run(root, "pre-record", &[("V0_ACTION", action), ("V0_MESSAGE", message)]).map_err(|r| {
        Fail::Hook(format!(
            "refusing to {action}: {r}. Fix what it reports, or pass --despite-hooks to push \
             through anyway (not recommended)."
        ))
    })
}

/// Remove `flag` from `args`; whether it was there.
fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let at = args.iter().position(|a| a == flag);
    at.map(|i| args.remove(i)).is_some()
}

fn open_conflicts(repo: &Repo) -> Vec<conflict::Conflict> {
    conflict::conflicts(&repo.head, &repo.changes, &repo.log)
        .into_iter()
        .filter(|c| c.status.is_open())
        .collect()
}

fn conflicts_cmd(all: bool) -> Result<(), Fail> {
    let (repo, _) = here()?;
    let every = conflict::conflicts(&repo.head, &repo.changes, &repo.log);
    let alive: BTreeSet<v0::op::Pos> = replay::materialise_head(&repo.head, &repo.changes, &repo.log, &BTreeSet::new())
        .files
        .values()
        .flatten()
        .map(|a| a.id)
        .collect();

    let mut open = 0;
    for c in &every {
        if !c.status.is_open() && !all {
            continue;
        }
        // The contested text as it was before either side touched it.
        let was: String = match repo.log.events.get(&c.atom).map(|e| &e.op) {
            Some(Op::Insert { text, .. }) => {
                text.chars().skip(c.range.0 as usize).take((c.range.1 - c.range.0) as usize).collect()
            }
            _ => "?".into(),
        };
        let state = match &c.status {
            conflict::Status::Open => {
                open += 1;
                "OPEN".to_string()
            }
            conflict::Status::Contested(rs) => {
                open += 1;
                format!("CONTESTED by {} concurrent resolutions", rs.len())
            }
            conflict::Status::Resolved(r) => format!("resolved by {}", r.short()),
            conflict::Status::Agreed => "agreed -- every side made the same change".to_string(),
            conflict::Status::Moot => "moot -- none of the contested text survives".to_string(),
        };
        println!("{}  {state}", c.id.short());
        match c.kind {
            conflict::Kind::Text => println!("    both replaced {was:?}"),
            conflict::Kind::Removal => println!("    a file was removed while someone edited it"),
            conflict::Kind::Rename => println!("    a file was renamed to different names"),
        }
        // Which side survived is *derived* from the text, never taken on trust.
        let sides = conflict::side_chars(c, &repo.changes, &repo.log);
        for (side, chars) in &sides {
            let kept = chars.iter().filter(|p| alive.contains(p)).count();
            let atoms = chars;
            println!(
                "    {}  {:<32} {kept}/{} of its characters survive",
                side.short(),
                repo.changes.by_id[side].meta.message,
                atoms.len()
            );
        }
        match &c.status {
            conflict::Status::Resolved(r) => {
                let m = &repo.changes.by_id[r].meta;
                println!("    resolver {}: {:?}", m.author, m.message);
            }
            conflict::Status::Contested(rs) => {
                for r in rs {
                    let m = &repo.changes.by_id[r].meta;
                    println!("    competing {} by {}: {:?}", r.short(), m.author, m.message);
                }
            }
            conflict::Status::Open | conflict::Status::Agreed | conflict::Status::Moot => {}
        }
    }
    if every.is_empty() || (open == 0 && !all) {
        println!("no open conflicts");
    }
    if open > 0 {
        return Err(Fail::Conflicts(format!("{open} unresolved conflict(s)")));
    }
    Ok(())
}

/// Resolve one conflict, several, every one in a file, or all of them -- with
/// one edit and one reason. A single edit routinely settles several conflicts
/// drawn in one block, and resolving them one id at a time used to redraw the
/// markers of the rest over the edit that had just settled them.
fn resolve_cmd(args: &[String]) -> Result<(), Fail> {
    const USAGE: &str = "v0 resolve [<id>...] [--file PATH] -m WHY";
    let mut args = args.to_vec();
    let why = message_arg(&args, USAGE)?;
    let (mut repo, root) = here()?;
    let skip_hook = take_flag(&mut args, "--despite-hooks");
    let open = open_conflicts(&repo);
    if open.is_empty() {
        return Err(Fail::Other("no unresolved conflicts".into()));
    }

    let (mut prefixes, mut file) = (Vec::new(), None);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-m" => i += 2,
            "--file" => {
                file = Some(args.get(i + 1).cloned().ok_or_else(|| Fail::Usage(USAGE.into()))?);
                i += 2;
            }
            p => {
                prefixes.push(p.to_owned());
                i += 1;
            }
        }
    }

    let mut chosen: Vec<conflict::Conflict> = Vec::new();
    for p in &prefixes {
        let hits: Vec<&conflict::Conflict> = open.iter().filter(|c| c.id.to_string().starts_with(p.as_str())).collect();
        match hits.as_slice() {
            [one] => chosen.push((*one).clone()),
            [] => return Err(Fail::Other(format!("no unresolved conflict {p}"))),
            _ => return Err(Fail::Other(format!("ambiguous conflict prefix {p}"))),
        }
    }
    if let Some(f) = &file {
        chosen.extend(open.iter().filter(|c| conflict_path(&repo, c).as_deref() == Some(f.as_str())).cloned());
    }
    if prefixes.is_empty() && file.is_none() {
        chosen = open;
    }
    chosen.sort_by_key(|c| c.id);
    chosen.dedup_by_key(|c| c.id);
    if chosen.is_empty() {
        return Err(Fail::Other(format!("no unresolved conflict in {}", file.unwrap_or_default())));
    }

    gate_hook(&root, skip_hook, "resolve", &why)?;
    let (minted, _hints) = capture_disk(&mut repo, &root, false)?;
    let fix = repo.resolve(minted, &chosen, Meta::new(why, whoami()));
    store::save(&root, &repo)?;
    store::checkout(&root, &repo)?;
    let ids: Vec<String> = chosen.iter().map(|c| c.id.short()).collect();
    println!(
        "resolved {} with {} ({} events)",
        ids.join(" "),
        fix.short(),
        repo.changes.by_id[&fix].events.len()
    );
    Ok(())
}

/// The file a conflict is about: the document holding the contested text, or
/// the contested node itself.
fn conflict_path(repo: &Repo, c: &conflict::Conflict) -> Option<String> {
    let all: Vec<EventId> = repo.log.events.keys().copied().collect();
    let tree = replay::tree_of(&all, &repo.log);
    let node = match c.kind {
        conflict::Kind::Text => replay::document_of(c.atom, &repo.log)?,
        _ => NodeId(c.atom),
    };
    tree.path_ignoring_removal(node)
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

fn pick(prefix: &str, adopt: bool, args: &[String]) -> Result<(), Fail> {
    let (mut repo, root) = here()?;
    // Dropping can only remove conflicts; adopting can bring new ones in.
    if adopt {
        gate_unresolved(&repo, &mut args.to_vec(), "adopt")?;
    }
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
fn sync_cmd(other: &Path, args: &[String]) -> Result<(), Fail> {
    let (mut repo, root) = here()?;
    gate_unresolved(&repo, &mut args.to_vec(), "sync")?;
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
