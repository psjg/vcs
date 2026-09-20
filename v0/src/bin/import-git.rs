//! Import a git history as events, then measure what layer 1 buys.
//!
//! ```text
//! cargo run --release --bin import-git -- <repo> [--max N]
//! ```
//!
//! Each commit becomes one change: its file diffs are handed to the degraded
//! capture adapter, which mints ops, and the resulting events are labelled with
//! the commit subject. The point is not fidelity to git — it is to run the
//! dependency derivation over a **real** history instead of a fixture built to
//! flatter it.
//!
//! Three numbers come out:
//!
//! - **reduction** `|causal closure| / |change closure|` — what adopting a
//!   change costs if you believe causal parents versus derived dependencies.
//! - **granularity cost** `|change closure| / |event closure|` — what taking a
//!   change whole costs against per-event dependencies.
//! - **independence** — how many changes depend on nothing at all.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::time::Instant;
use v0::capture;
use v0::change::{Change, ChangeId, Changes, Meta};
use v0::event::EventLog;
use v0::op::{EventId, NodeId, NodeKind, Op, ReplicaId};
use v0::replay::materialise_atoms;

fn git(repo: &str, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Skip what a line-oriented VCS has no business importing.
fn importable(content: &str) -> bool {
    !content.is_empty() && content.len() < 400_000 && !content.contains('\0')
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(repo) = args.get(1) else {
        eprintln!("usage: import-git <repo> [--max N]");
        std::process::exit(2);
    };
    let max: usize = args
        .iter()
        .position(|a| a == "--max")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(usize::MAX);

    let started = Instant::now();
    let replica = ReplicaId(1);
    let mut seq = 1u32;
    let mut log = EventLog::default();
    let mut changes = Changes::default();
    let mut nodes: BTreeMap<String, NodeId> = BTreeMap::new();
    let mut order: Vec<ChangeId> = Vec::new();

    let commits: Vec<String> = git(repo, &["rev-list", "--reverse", "--first-parent", "HEAD"])
        .lines()
        .take(max)
        .map(str::to_owned)
        .collect();
    eprintln!("importing {} commits from {repo}", commits.len());

    for (n, sha) in commits.iter().enumerate() {
        let subject = git(repo, &["log", "-1", "--format=%s", sha]).trim().to_owned();
        let status = git(repo, &["show", "--pretty=format:", "--name-status", sha]);

        // One materialisation per commit, not per file: the working state is
        // everything appended so far, since an import is linear.
        let all: Vec<EventId> = log.events.keys().copied().collect();
        let mut atoms = materialise_atoms(&all, &log);
        let mut minted: BTreeSet<EventId> = BTreeSet::new();

        for line in status.lines() {
            let mut cols = line.split('\t');
            let (Some(kind), Some(path)) = (cols.next(), cols.next()) else { continue };

            // A rename git *detected* with a similarity heuristic becomes the
            // move op it always was. Importing a guess, but a useful one.
            if kind.starts_with('R') {
                let Some(dest) = cols.next() else { continue };
                if let Some(node) = nodes.remove(path) {
                    let name = dest.rsplit('/').next().unwrap_or(dest).to_owned();
                    let id = log.append(replica, &mut seq, Op::MoveNode { node, parent: Op::ROOT, name });
                    minted.insert(id);
                    nodes.insert(dest.to_owned(), node);
                }
                continue;
            }
            if kind.starts_with('D') {
                if let Some(node) = nodes.remove(path) {
                    minted.insert(log.append(replica, &mut seq, Op::Remove { node }));
                }
                continue;
            }

            let content = git(repo, &["show", &format!("{sha}:{path}")]);
            if !importable(&content) {
                continue;
            }
            let node = *nodes.entry(path.to_owned()).or_insert_with(|| {
                let node = NodeId(EventId { seq, replica });
                let name = path.rsplit('/').next().unwrap_or(path).to_owned();
                minted.insert(log.append(
                    replica,
                    &mut seq,
                    Op::Create { node, parent: Op::ROOT, name, kind: NodeKind::File },
                ));
                node
            });

            let before = atoms.get(path).cloned().unwrap_or_default();
            let ops = capture::from_save(&before, node, &content, replica, seq);
            minted.extend(log.append_batch(replica, &mut seq, ops));
            atoms.remove(path); // stale now; refreshed next commit
        }

        if minted.is_empty() {
            continue;
        }
        let meta = Meta { message: subject, author: "import".into() };
        let change = Change::new(minted, meta, &log, &changes.owners());
        let id = change.id();
        changes.by_id.insert(id, change);
        order.push(id);
        if n % 25 == 0 {
            eprintln!("  {n} commits, {} events", log.events.len());
        }
    }

    // --- the report ---------------------------------------------------------
    let mut reductions = Vec::new();
    let mut granularity = Vec::new();
    let mut dep_counts = Vec::new();
    for id in &order {
        let ch = &changes.by_id[id];
        let causal: BTreeSet<EventId> =
            ch.events.iter().flat_map(|e| log.causal_closure(*e)).collect();
        let change_min = changes.events_of(&changes.closure(*id));
        let event_min: BTreeSet<EventId> =
            ch.events.iter().flat_map(|e| log.semantic_closure(*e)).collect();
        if !change_min.is_empty() {
            reductions.push(causal.len() as f64 / change_min.len() as f64);
            granularity.push(change_min.len() as f64 / event_min.len().max(1) as f64);
        }
        dep_counts.push(ch.deps.len());
    }

    let stat = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        (mean, v[v.len() / 2], v[v.len() * 9 / 10], *v.last().unwrap())
    };
    let (rm, rmed, rp90, rmax) = stat(reductions.clone());
    let (gm, gmed, gp90, gmax) = stat(granularity.clone());
    let independent = dep_counts.iter().filter(|d| **d == 0).count();

    println!("\n=== {repo} ===");
    println!("commits imported : {}", order.len());
    println!("events           : {}", log.events.len());
    println!("files            : {}", nodes.len());
    println!();
    println!("reduction  causal/change-minimal   mean {rm:6.1}x  median {rmed:6.1}x  p90 {rp90:6.1}x  max {rmax:7.1}x");
    println!("granularity change/event-minimal   mean {gm:6.2}x  median {gmed:6.2}x  p90 {gp90:6.2}x  max {gmax:7.2}x");
    println!(
        "dependencies per change            mean {:6.2}   max {:4}   independent {independent}/{} ({:.0}%)",
        dep_counts.iter().sum::<usize>() as f64 / dep_counts.len() as f64,
        dep_counts.iter().max().copied().unwrap_or(0),
        dep_counts.len(),
        100.0 * independent as f64 / dep_counts.len() as f64
    );
    // Does the cost of adopting a change grow with the length of history?
    // If the causal side grows and the derived side does not, that is the
    // whole argument for layer 1 stated as a measurement.
    let sizes: Vec<(usize, usize, usize)> = order
        .iter()
        .map(|id| {
            let ch = &changes.by_id[id];
            let causal: BTreeSet<EventId> =
                ch.events.iter().flat_map(|e| log.causal_closure(*e)).collect();
            let event_min: BTreeSet<EventId> =
                ch.events.iter().flat_map(|e| log.semantic_closure(*e)).collect();
            (causal.len(), changes.events_of(&changes.closure(*id)).len(), event_min.len())
        })
        .collect();
    let third = sizes.len() / 3;
    let avg = |s: &[(usize, usize, usize)], f: fn(&(usize, usize, usize)) -> usize| {
        s.iter().map(f).sum::<usize>() as f64 / s.len().max(1) as f64
    };
    println!();
    println!("                              first third      last third");
    println!(
        "causal closure  (events)   {:12.0}    {:12.0}",
        avg(&sizes[..third], |p| p.0),
        avg(&sizes[sizes.len() - third..], |p| p.0)
    );
    println!(
        "change-level    (events)   {:12.0}    {:12.0}",
        avg(&sizes[..third], |p| p.1),
        avg(&sizes[sizes.len() - third..], |p| p.1)
    );
    println!(
        "event-level     (events)   {:12.0}    {:12.0}",
        avg(&sizes[..third], |p| p.2),
        avg(&sizes[sizes.len() - third..], |p| p.2)
    );

    println!("\nimported in {:.1}s", started.elapsed().as_secs_f64());
}
