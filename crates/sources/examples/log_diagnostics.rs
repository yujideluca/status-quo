//! Read declared transcript sources, report only bounded parser rejection categories.
use sq_core::*;
use sq_sources::*;
fn main() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut state = LocalState::default();
    state.discover(&SourceConfig::default(), now).unwrap();
    let mut snapshot = Snapshot::default();
    for _ in 0..1000 {
        state.poll(&mut snapshot, now);
        if state.cursors.values().all(|c| c.deferred == 0) {
            break;
        }
    }
    for provider in ["claude", "codex"] {
        let mut reasons = std::collections::BTreeMap::<String, u64>::new();
        let mut malformed = 0;
        let mut deferred = 0;
        for cursor in state
            .cursors
            .values()
            .filter(|c| c.parser.provider == provider)
        {
            malformed += cursor.malformed;
            deferred += cursor.deferred;
            for (reason, count) in &cursor.rejection_reasons {
                *reasons.entry(reason.clone()).or_default() += count;
            }
        }
        println!("{provider}: malformed={malformed}, deferred={deferred}, reasons={reasons:?}");
    }
}
