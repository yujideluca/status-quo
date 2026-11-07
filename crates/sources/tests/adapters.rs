#[path = "../../scratch.rs"]
mod scratch;
use scratch::Scratch;
use serde_json::json;
use sq_sources::*;
#[test] // REQ SQ-5 SQ-6: primary is not necessarily the five-hour window.
fn account_windows_keep_durations_model_labels_and_unknowns() {
    let value = json!({"rateLimits":{"primary":{"usedPercent":23.0,"windowDurationMins":10080,"resetsAt":500},"secondary":null},"rateLimitsByLimitId":null});
    let usage = codex_usage(&value, 100).unwrap();
    assert!(usage.window(300).is_none());
    assert_eq!(usage.window(10080).unwrap().used, 23.0);
    let usage=claude_usage(&json!({"five_hour":{"utilization":40,"resets_at":"2026-11-03T00:00:00Z"},"seven_day":null,"seven_day_sonnet":{"utilization":5,"resets_at":null}}),100).unwrap();
    assert_eq!(usage.window(300).unwrap().used, 40.0);
    assert!(usage.windows.iter().any(|w| w.label == "seven_day_sonnet"));
    assert!(claude_usage(&json!({"five_hour":{"utilization":-1}}), 100).is_err());
}
#[test] // REQ SQ-4 SQ-7: normalize included Codex cache, deltas, duplicate and reset totals.
fn codex_cumulative_usage_counts_each_delta_once() {
    let mut parser = LogParser::new("codex", "session");
    let event = |input, cache, output| json!({"timestamp":"2026-11-02T00:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":cache,"output_tokens":output}}}});
    let first = parser.ingest(&event(100, 30, 10)).unwrap().unwrap();
    assert_eq!(
        (first.input, first.cache_read, first.output),
        (70, Some(30), 10)
    );
    assert!(parser.ingest(&event(100, 30, 10)).unwrap().is_none());
    let next = parser.ingest(&event(120, 35, 13)).unwrap().unwrap();
    assert_eq!((next.input, next.cache_read, next.output), (15, Some(5), 3));
    let reset = parser.ingest(&event(10, 0, 1)).unwrap().unwrap();
    assert_eq!(reset.input, 10);
    assert!(parser.ingest(&event(10, 11, 1)).is_err());
}
#[test] // REQ SQ-7 SQ-24: Claude message identity and missing cache stay distinct.
fn claude_stream_revisions_share_message_identity_without_persisting_content() {
    let mut parser = LogParser::new("claude", "session");
    let event = |out| json!({"timestamp":"2026-11-02T00:00:00Z","type":"assistant","message":{"id":"msg-1","content":[{"text":"private conversation"}],"usage":{"input_tokens":100,"output_tokens":out}}});
    let first = parser.ingest(&event(2)).unwrap().unwrap();
    let second = parser.ingest(&event(3)).unwrap().unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(second.cache_read, None);
    assert_eq!(second.input, 100);
    assert!(!serde_json::to_string(&second)
        .unwrap()
        .contains("private conversation"));
}
#[test] // REQ SQ-11: historic event feed never silently accepts malformed metadata.
fn memory_frontmatter_requires_identity_timestamp_and_known_event_kind() {
    let text="---\ntype: decision\ninstance: codex\nkey: tray\nts: 2026-11-02T00:00:00Z\n---\nIndependent widgets";
    let item = memory_event(text, "20261102T000000Z-codex-abc", "/events/item.md").unwrap();
    assert_eq!(item.kind, "decision");
    assert_eq!(item.instance, "codex");
    assert!(memory_event("broken", "id", "/events/bad.md").is_err());
    assert!(memory_event(&text.replace("decision", "madeup"), "id", "/events/bad.md").is_err());
}
#[test] // REQ SQ-7: a trailing partial line is deferred and read exactly once after completion.
fn incremental_reader_resumes_partial_lines_and_reports_bad_records() {
    use std::fs;
    let scratch = Scratch::new("sq-log");
    let path = scratch.join("log.jsonl");
    let line = r#"{"timestamp":"2026-11-02T00:00:00Z","type":"assistant","message":{"id":"one","usage":{"input_tokens":10,"output_tokens":2}}}"#;
    fs::write(&path, format!("bad\n{}", &line[..40])).unwrap();
    let mut cursor = Cursor::new("claude", "test");
    let mut ledger = sq_core::TokenLedger::default();
    let first = cursor.poll(&path, &mut ledger, 1024 * 1024).unwrap();
    assert_eq!(first.malformed, 1);
    assert_eq!(first.deferred, 1);
    assert!(ledger.records.is_empty());
    fs::write(&path, format!("bad\n{line}\n")).unwrap();
    cursor.poll(&path, &mut ledger, 1024 * 1024).unwrap();
    assert_eq!(ledger.records.len(), 1);
    cursor.poll(&path, &mut ledger, 1024 * 1024).unwrap();
    assert_eq!(ledger.records.len(), 1);
    fs::write(&path, format!("{line}\n")).unwrap();
    let rotated = cursor.poll(&path, &mut ledger, 1024 * 1024).unwrap();
    assert!(rotated.rotated);
    assert_eq!(ledger.records.len(), 1);
    fs::remove_file(path).unwrap();
}
#[test] // REQ SQ-10: endpoint records normalize with stable IDs and PRs aren't duplicated as issues.
fn github_records_keep_stable_identity_and_do_not_count_prs_twice() {
    let issue = json!({"id":42,"title":"Fix tray","updated_at":"2026-11-02T00:00:00Z","html_url":"https://github.com/owner/repo/issues/3"});
    let items = github_records("owner/repo", "issues", &json!([issue.clone()])).unwrap();
    assert_eq!(items.len(), 1);
    let mut pr = issue;
    pr["pull_request"] = json!({});
    assert!(github_records("owner/repo", "issues", &json!([pr]))
        .unwrap()
        .is_empty());
    assert!(github_records("bad;command/repo", "issues", &json!([])).is_err());
    assert!(github_records("owner/repo", "issues", &json!([{"title":"no identity"}])).is_err());
}
#[test] // REQ SQ-5: seven_day_breakdown is metadata, not a quota window.
fn claude_breakdown_metadata_does_not_hide_valid_account_limits() {
    let usage=claude_usage(&json!({"five_hour":{"utilization":33},"seven_day":{"utilization":66},"seven_day_breakdown":{"some_breakdown":[]}}),100).unwrap();
    assert_eq!(usage.windows.len(), 2);
    assert_eq!(usage.window(300).unwrap().used, 33.0);
}
#[test] // REQ SQ-8 SQ-9 SQ-24: hooks discard prompt/tool contents and Stop means waiting, not proven goal completion.
fn lifecycle_events_keep_only_labels_and_explicit_states() {
    let input = json!({"session_id":"main-1","cwd":"/tmp/project","prompt":"secret prompt must not persist","hook_event_name":"UserPromptSubmit"});
    let event = lifecycle_event("claude", "UserPromptSubmit", &input, 100, 1).unwrap();
    assert_eq!(event.state, sq_core::SessionState::Running);
    assert!(!serde_json::to_string(&event).unwrap().contains("secret"));
    let event = lifecycle_event("claude", "Stop", &input, 101, 2).unwrap();
    assert_eq!(event.state, sq_core::SessionState::Waiting);
    let child = lifecycle_event(
        "claude",
        "SubagentStop",
        &json!({"session_id":"main-1","agent_id":"child-1","cwd":"/tmp/project"}),
        102,
        3,
    )
    .unwrap();
    assert_eq!(child.parent, Some("main-1".into()));
    assert_eq!(child.state, sq_core::SessionState::Completed);
    assert!(lifecycle_event(
        "claude",
        "UserPromptSubmit",
        &json!({"session_id":"../../escape"}),
        1,
        1
    )
    .is_err());
}

// REQ SQ-10: a running check has null completion and a real start timestamp.
#[test]
fn running_github_checks_keep_their_start_timestamp() {
    let value = json!({"check_runs":[{"id":7,"name":"CI","completed_at":null,"started_at":"2026-11-02T00:00:00Z","html_url":"https://github.com/o/r/actions/runs/7"}]});
    let records = github_records("o/r", "checks", &value).unwrap();
    assert_eq!(records[0].at, timestamp("2026-11-02T00:00:00Z").unwrap());
}

// REQ SQ-11 SQ-24: bounded memory excerpts identify omitted text explicitly.
#[test]
fn memory_excerpt_marks_truncation() {
    let text = format!(
        "---\ntype: decision\nts: 2026-11-02T00:00:00Z\n---\n{}",
        "x".repeat(8001)
    );
    let item = memory_event(&text, "id", "/tmp/event.md").unwrap();
    assert!(item.body.contains("excerpt truncated"));
}

// REQ SQ-4 SQ-7: rotating a cumulative transcript keeps its baseline and stable observed totals.
#[test]
fn codex_rotation_does_not_replace_old_deltas_or_count_totals_again() {
    use std::fs;
    let scratch = Scratch::new("sq-rotation");
    let root = scratch.to_path_buf();
    let path = root.join("session.jsonl");
    let event = |sec, input| {
        format!(
            "{}\n",
            json!({"timestamp":format!("2026-11-02T00:00:{sec:02}Z"),"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":0,"output_tokens":0}}}})
        )
    };
    let mut cursor = Cursor::new("codex", "session");
    let mut ledger = sq_core::TokenLedger::default();
    fs::write(&path, format!("{}{}", event(1, 100), event(2, 120))).unwrap();
    cursor.poll(&path, &mut ledger, 10000).unwrap();
    fs::rename(&path, root.join("old.jsonl")).unwrap();
    fs::write(&path, format!("{}{}", event(2, 120), event(3, 150))).unwrap();
    cursor.poll(&path, &mut ledger, 10000).unwrap();
    assert_eq!(
        ledger
            .buckets("codex", 0, i64::MAX, 1)
            .unwrap()
            .iter()
            .map(|r| r.input)
            .sum::<u64>(),
        150
    );
    assert_eq!(ledger.records.len(), 3);
    fs::remove_dir_all(root).unwrap();
}

// REQ SQ-7 SQ-18: a changed configured root stops reading old paths but keeps retained token observations.
#[test]
fn changing_source_root_excludes_old_cursors() {
    let mut state = LocalState::default();
    state.cursors.insert(
        "/old/claude/projects/session.jsonl".into(),
        Cursor::new("claude", "session"),
    );
    let config = sq_core::SourceConfig {
        claude: "/nonexistent/status-quo-new".into(),
        codex: "/nonexistent/status-quo-other".into(),
        ..Default::default()
    };
    state.discover(&config, 100).unwrap();
    assert!(state.cursors.is_empty());
}

// REQ SQ-6 SQ-19: closing stdout cannot bypass a provider command deadline.
#[test]
fn provider_process_exit_is_bounded_after_stdout_closes() {
    let start = std::time::Instant::now();
    let result = command_json(
        "sh",
        &["-c", "printf '{}'; exec 1>&-; exec sleep 2"],
        std::time::Duration::from_millis(60),
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    assert!(result.is_err());
}
// REQ SQ-31: many empty project folders must not trip the file bound, nor be watched. The bound is
// lowered to 10 files so 1,100 folders prove what 101,000 did, on file systems where creating that
// many folders takes minutes (Windows).
#[test]
fn discovery_ignores_empty_project_folders_and_watches_only_live_ones() {
    let scratch = Scratch::new("sq-discover");
    let root = scratch.to_path_buf();
    let projects = root.join(".claude/projects");
    for n in 0..1_100 {
        std::fs::create_dir_all(projects.join(format!("-tmp-tmp{n}"))).unwrap();
    }
    let live = projects.join("-home-me-app");
    std::fs::create_dir_all(&live).unwrap();
    std::fs::write(live.join("s1.jsonl"), "").unwrap();
    let config = sq_core::SourceConfig {
        claude: root.join(".claude").to_string_lossy().into(),
        codex: root.join(".codex").to_string_lossy().into(),
        ..Default::default()
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut state = LocalState::default();
    // 1,101 folders against a bound of 10 files: folders are counted apart and never fatal.
    assert_eq!(state.discover_bounded(&config, now, 10, 5_000), Ok(1));
    assert_eq!(state.watch_dirs.len(), 1);
    assert!(state.watch_dirs.contains(&live));
    // The folder bound itself still protects against runaway trees.
    let mut state = LocalState::default();
    assert!(state.discover_bounded(&config, now, 10, 1_000).is_err());
    // Files are bounded too: eleven transcripts against a bound of ten.
    for n in 0..11 {
        std::fs::write(live.join(format!("extra{n}.jsonl")), "").unwrap();
    }
    let mut state = LocalState::default();
    assert!(state.discover_bounded(&config, now, 10, 5_000).is_err());
    let _ = std::fs::remove_dir_all(&root);
}
// REQ SQ-31: discovery reuses the listing of a folder that has not changed since it was read, and
// finds exactly what a fresh walk finds: a transcript added to a remembered folder, a removed
// folder, a newly too-old transcript and nesting below the depth limit.
#[test]
fn repeated_discovery_matches_a_fresh_walk() {
    use std::fs;
    use std::time::{Duration, SystemTime};
    let scratch = Scratch::new("sq-rediscover");
    let root = scratch.to_path_buf();
    let projects = root.join(".claude").join("projects");
    let dirs = [
        "-tmp-a/memory",
        "-home-me-app/sess/subagents",
        "-home-me-old",
        "-home-me-deep/a/b/c/d",
    ];
    for dir in dirs {
        fs::create_dir_all(projects.join(dir)).unwrap();
    }
    for file in [
        "-home-me-app/s1.jsonl",
        "-home-me-app/sess/subagents/agent-1.jsonl",
        "-home-me-old/gone.jsonl",
        "-home-me-deep/a/b/c/four.jsonl",
        "-home-me-deep/a/b/c/d/too-deep.jsonl",
        "-tmp-a/memory/note.md",
    ] {
        fs::write(projects.join(file), "").unwrap();
    }
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let old = SystemTime::now() - Duration::from_secs(40 * 86400);
    // Folders untouched for a minute: their listings may be remembered.
    let settle = |path: &std::path::Path| {
        for entry in walkdir_like(path) {
            set_modified(&entry, SystemTime::now() - Duration::from_secs(60));
        }
    };
    fn walkdir_like(path: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = vec![path.to_path_buf()];
        for entry in fs::read_dir(path).unwrap().flatten() {
            if entry.file_type().unwrap().is_dir() {
                out.extend(walkdir_like(&entry.path()));
            }
        }
        out
    }
    let config = sq_core::SourceConfig {
        claude: root.join(".claude").to_string_lossy().into(),
        codex: root.join(".codex").to_string_lossy().into(),
        ..Default::default()
    };
    let fresh = || {
        let mut state = LocalState::default();
        let found = state.discover(&config, now);
        (found, state)
    };
    let same = |a: &LocalState, b: &LocalState| {
        assert_eq!(
            a.cursors.keys().collect::<Vec<_>>(),
            b.cursors.keys().collect::<Vec<_>>()
        );
        assert_eq!(a.watch_dirs, b.watch_dirs);
        for (key, cursor) in &a.cursors {
            assert_eq!(cursor.parser.parent, b.cursors[key].parser.parent);
        }
    };
    settle(&projects);
    let mut state = LocalState::default();
    assert_eq!(state.discover(&config, now), Ok(4));
    let (_, first) = fresh();
    same(&state, &first);
    assert_eq!(
        first.cursors[&["-home-me-app", "sess", "subagents", "agent-1.jsonl"]
            .iter()
            .fold(projects.clone(), |path, part| path.join(part))
            .to_string_lossy()
            .to_string()]
            .parser
            .parent
            .as_deref(),
        Some("sess")
    );
    // Unchanged tree: the remembered listings give the same result.
    assert_eq!(state.discover(&config, now), Ok(0));
    same(&state, &first);
    // A transcript added to a remembered folder is found.
    fs::write(projects.join("-tmp-a/memory/late.jsonl"), "").unwrap();
    assert_eq!(state.discover(&config, now), Ok(1));
    same(&state, &fresh().1);
    // A transcript turning old no longer marks its folder as watched; a removed folder is dropped.
    set_modified(&projects.join("-home-me-old/gone.jsonl"), old);
    settle(&projects);
    let mut again = LocalState::default();
    again.discover(&config, now).unwrap();
    again.discover(&config, now).unwrap();
    assert!(!again.watch_dirs.contains(&projects.join("-home-me-old")));
    same(&again, &fresh().1);
    fs::remove_dir_all(projects.join("-home-me-deep")).unwrap();
    again.discover(&config, now).unwrap();
    assert!(!again.watch_dirs.contains(&projects.join("-home-me-deep")));
    assert_eq!(again.watch_dirs, fresh().1.watch_dirs);
    let _ = fs::remove_dir_all(&root);
}

/// Sets the modification time of a file or a folder on every supported system. Windows opens a
/// folder only with FILE_FLAG_BACKUP_SEMANTICS and changes its time only with
/// FILE_WRITE_ATTRIBUTES; a plain `File::open` there fails with "Access is denied" (SQ-36).
fn set_modified(path: &std::path::Path, time: std::time::SystemTime) {
    let mut options = std::fs::OpenOptions::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        options
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    }
    #[cfg(not(windows))]
    options.read(true);
    options
        .open(path)
        .and_then(|file| file.set_modified(time))
        .unwrap_or_else(|e| panic!("set modified time of {}: {e}", path.display()));
}
