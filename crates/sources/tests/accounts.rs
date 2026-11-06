use sq_core::{Account, SourceConfig};
use sq_sources::*;
use std::{fs, path::Path};

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
fn claude_line(session: &str, message: &str, output: u64) -> String {
    let at = chrono::DateTime::from_timestamp(now(), 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ");
    format!(
        "{{\"timestamp\":\"{at}\",\"type\":\"assistant\",\"sessionId\":\"{session}\",\"cwd\":\"/work/app\",\"message\":{{\"id\":\"{message}\",\"usage\":{{\"input_tokens\":100,\"output_tokens\":{output}}}}}}}\n"
    )
}
fn write_claude(home: &Path, project: &str, session: &str, output: u64) {
    let dir = home.join("projects").join(project);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{session}.jsonl")),
        claude_line(session, &format!("m-{session}"), output),
    )
    .unwrap();
}
fn temp(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sq-accounts-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
fn total(buckets: &[sq_core::Bucket]) -> u64 {
    buckets.iter().map(|b| b.input + b.output).sum()
}

// REQ SQ-35: every Claude account keeps its own tokens, sessions and health.
#[test]
fn two_claude_accounts_never_mix_tokens_sessions_or_health() {
    let root = temp("claude");
    write_claude(&root.join("personal"), "-p-app", "sess-personal", 50);
    write_claude(&root.join("work"), "-w-app", "sess-work", 900);
    let config = SourceConfig {
        claude: root.join("personal").to_string_lossy().into(),
        codex: root.join("codex").to_string_lossy().into(),
        accounts: vec![Account {
            id: "work".into(),
            kind: "claude".into(),
            path: root.join("work").to_string_lossy().into(),
            ..Account::default()
        }],
        ..Default::default()
    };
    let mut state = LocalState::default();
    assert_eq!(state.discover(&config, now()).unwrap(), 2);
    let mut snapshot = sq_core::Snapshot::default();
    state.poll(&mut snapshot, now());
    assert_eq!(total(&snapshot.tokens["claude"]), 150);
    assert_eq!(total(&snapshot.tokens["claude:work"]), 1000);
    let mut sessions: Vec<_> = snapshot
        .sessions
        .iter()
        .map(|s| (s.assistant.clone(), s.id.clone()))
        .collect();
    sessions.sort();
    assert_eq!(
        sessions,
        vec![
            ("claude".to_string(), "sess-personal".to_string()),
            ("claude:work".to_string(), "sess-work".to_string())
        ]
    );
    assert!(snapshot.health.contains_key("claude-logs"));
    assert!(snapshot.health.contains_key("claude:work-logs"));
    assert!(snapshot.health.contains_key("codex-logs"));
    // Removing the account stops reading its folder but the default account is untouched.
    let alone = SourceConfig {
        accounts: vec![],
        ..config
    };
    state.discover(&alone, now()).unwrap();
    assert!(state.cursors.keys().all(|k| k.contains("personal")));
    let _ = fs::remove_dir_all(root);
}

// REQ SQ-35: an extra Codex account reads the sessions folder of its own home.
#[test]
fn an_extra_codex_account_reads_its_own_sessions_folder() {
    let root = temp("codex");
    let day = root.join("team/sessions/2026/11/05");
    fs::create_dir_all(&day).unwrap();
    let at = chrono::DateTime::from_timestamp(now(), 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ");
    fs::write(
        day.join("rollout-abc.jsonl"),
        format!("{{\"timestamp\":\"{at}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":100,\"cached_input_tokens\":0,\"output_tokens\":10}}}}}}}}\n"),
    )
    .unwrap();
    let config = SourceConfig {
        claude: root.join("none").to_string_lossy().into(),
        codex: root.join("none-codex").to_string_lossy().into(),
        accounts: vec![Account {
            id: "team".into(),
            kind: "codex".into(),
            path: root.join("team").to_string_lossy().into(),
            ..Account::default()
        }],
        ..Default::default()
    };
    let mut state = LocalState::default();
    assert_eq!(state.discover(&config, now()).unwrap(), 1);
    let mut snapshot = sq_core::Snapshot::default();
    state.poll(&mut snapshot, now());
    assert_eq!(total(&snapshot.tokens["codex:team"]), 110);
    assert_eq!(total(&snapshot.tokens["codex"]), 0);
    assert_eq!(snapshot.sessions[0].assistant, "codex:team");
    assert!(snapshot.sessions[0].children_supported);
    let _ = fs::remove_dir_all(root);
}

// REQ SQ-35: the extra environment reaches the program, and only that program.
#[cfg(unix)]
#[test]
fn a_provider_command_gets_the_account_folder_in_its_environment() {
    let value = command_json_env(
        "sh",
        &["-c", "printf '{\"folder\":\"%s\"}' \"$GH_CONFIG_DIR\""],
        &[(
            "GH_CONFIG_DIR",
            std::ffi::OsStr::new("/home/x/.config/gh-work"),
        )],
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(value["folder"], "/home/x/.config/gh-work");
    assert!(std::env::var_os("GH_CONFIG_DIR").is_none());
}

// REQ SQ-35: a GitHub account reads with its own gh login and stamps its records.
#[cfg(unix)]
#[test]
fn github_accounts_use_their_own_gh_folder_and_stamp_their_records() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("gh");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    // A fake gh: answers every call with one commit whose title names the login folder.
    let gh = bin.join("gh");
    fs::write(
        &gh,
        r#"#!/bin/sh
case "$2" in
  *check-runs*) printf '{"check_runs":[]}' ;;
  *commits?*) printf '[{"sha":"abc","html_url":"https://github.com/o/r/commit/abc","commit":{"message":"from %s","committer":{"date":"2026-11-05T00:00:00Z"}}}]' "${GH_CONFIG_DIR:-default}" ;;
  *) printf '[]' ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    // Only this test starts gh, so changing PATH here cannot disturb another test of this file.
    std::env::set_var("PATH", path);
    let repos = vec!["o/r".to_string()];
    let default = fetch_github_as("github", None, &repos);
    assert!(default.errors.is_empty(), "{:?}", default.errors);
    assert_eq!(default.items.len(), 1);
    assert_eq!(default.items[0].account, "github");
    assert!(default.items[0].title.contains("from default"));
    let work = fetch_github_as("github:work", Some(Path::new("/cfg/work")), &repos);
    assert_eq!(work.items[0].account, "github:work");
    assert!(work.items[0].title.contains("from /cfg/work"));
    let _ = fs::remove_dir_all(root);
}

// REQ SQ-36: a tool is found on PATH as .exe, .cmd or .bat, in PATH order.
#[test]
fn programs_are_found_with_windows_extensions_in_path_order() {
    let root = temp("path");
    let (first, second) = (root.join("first"), root.join("second"));
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("codex.cmd"), "").unwrap();
    fs::write(second.join("codex.exe"), "").unwrap();
    fs::write(second.join("gh.exe"), "").unwrap();
    let path = std::env::join_paths([&first, &second]).unwrap();
    assert_eq!(find_program("codex", &path), Some(first.join("codex.cmd")));
    assert_eq!(find_program("gh", &path), Some(second.join("gh.exe")));
    assert_eq!(find_program("absent", &path), None);
    // An explicit path is never searched.
    assert_eq!(
        program("/usr/bin/env"),
        std::path::PathBuf::from("/usr/bin/env")
    );
    let _ = fs::remove_dir_all(root);
}
