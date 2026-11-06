//! Times one collector "local" tick against the real transcript folders, read-only: discovery,
//! a full poll, a poll of a few changed files, token buckets, pruning and checkpoint
//! serialization. Prints timings and counts only, never transcript content.
//! Usage: cargo run --release -p sq-sources --example local_bench [path/to/local.json]
use sq_core::*;
use sq_sources::*;
use std::time::Instant;
fn main() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let config = SourceConfig::default();
    let mut state: LocalState = std::env::args()
        .nth(1)
        .map(|p| serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap())
        .unwrap_or_default();
    let time = |label: &str, runs: u32, f: &mut dyn FnMut()| {
        let start = Instant::now();
        for _ in 0..runs {
            f();
        }
        println!(
            "{label}: {:.2} ms",
            start.elapsed().as_secs_f64() * 1e3 / runs as f64
        );
    };
    time("discover (first)", 1, &mut || {
        state.discover(&config, now).unwrap();
    });
    time("discover (repeat)", 5, &mut || {
        state.discover(&config, now).unwrap();
    });
    let mut snapshot = Snapshot::default();
    // Catch up first so the timed polls measure the steady state.
    for _ in 0..1000 {
        state.poll(&mut snapshot, now);
        if state.cursors.values().all(|c| c.deferred == 0) {
            break;
        }
    }
    println!(
        "cursors={} records={} watch_dirs={}",
        state.cursors.len(),
        state.ledger.records.len(),
        state.watch_dirs.len()
    );
    time("poll (full)", 5, &mut || {
        state.poll(&mut snapshot, now);
    });
    let mut recent: Vec<_> = state
        .cursors
        .iter()
        .map(|(k, c)| (c.modified, k.clone()))
        .collect();
    recent.sort();
    let selected: std::collections::BTreeSet<String> = recent
        .iter()
        .rev()
        .take(3)
        .map(|(_, k)| k.clone())
        .collect();
    time("poll (3 changed files)", 20, &mut || {
        state.poll_selected(&mut snapshot, now, Some(&selected));
    });
    for provider in ["claude", "codex"] {
        time(&format!("buckets {provider}"), 20, &mut || {
            state
                .ledger
                .buckets(provider, now - 30 * 86400, now + 1, 3600)
                .ok();
        });
    }
    time("prune", 20, &mut || {
        state.ledger.prune(now - 30 * 86400);
    });
    time("serialize checkpoint", 5, &mut || {
        serde_json::to_vec(&state).unwrap();
    });
}
