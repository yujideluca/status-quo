#[path = "../../scratch.rs"]
mod scratch;
use scratch::Scratch;
use serde_json::json;
use sq_sources::lifecycle_event;
// REQ SQ-8 SQ-9 SQ-24: concurrent hooks retain newest state, no prompts and one bounded file per session.
#[test]
fn concurrent_lifecycle_writers_preserve_newest_observation() {
    let scratch = Scratch::new("sq-hooks");
    let root = scratch.to_path_buf();
    let mut threads = vec![];
    for n in 1..=20 {
        let root = root.clone();
        threads.push(std::thread::spawn(move || {
            let event = lifecycle_event(
                "claude",
                if n == 20 { "Stop" } else { "PreToolUse" },
                &json!({"session_id":"main","cwd":"/tmp/project","prompt":"private"}),
                100,
                n,
            )
            .unwrap();
            sq_collector::record_lifecycle(&root, &event).unwrap();
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    let paths: Vec<_> = std::fs::read_dir(root.join("events")).unwrap().collect();
    assert_eq!(paths.len(), 1);
    let text = std::fs::read_to_string(root.join("events/claude-main.json")).unwrap();
    assert!(!text.contains("private"));
    let event: sq_sources::LifecycleEvent = serde_json::from_str(&text).unwrap();
    assert_eq!(event.sequence, 20);
    assert_eq!(event.state, sq_core::SessionState::Waiting);
    std::fs::remove_dir_all(root).unwrap();
}
