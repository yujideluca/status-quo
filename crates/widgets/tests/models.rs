use sq_contract::*;
use sq_core::*;
#[test] // SQ-5 SQ-18: unavailable quota is unknown, not an invented zero.
fn missing_usage_and_preserved_stale_values_are_distinct() {
    let registry = sq_widgets::registry();
    let config = Config::default();
    let widget = &config.widgets[0];
    let mut snapshot = Snapshot::default();
    let missing = registry.view(widget, &snapshot, 100);
    assert!(missing
        .nodes
        .iter()
        .any(|n| matches!(n, Node::Meter { used: None, .. })));
    snapshot.usage.insert(
        "claude".into(),
        Usage {
            windows: vec![QuotaWindow {
                minutes: 300,
                used: 42.0,
                resets_at: Some(200),
                label: "5 hour".into(),
            }],
            updated_at: Some(99),
        },
    );
    snapshot.health.insert(
        "claude".into(),
        Health {
            state: SourceState::Stale,
            message: "Offline; last successful value retained".into(),
            ..Health::default()
        },
    );
    let stale = registry.view(widget, &snapshot, 100);
    assert!(stale
        .nodes
        .iter()
        .any(|n| matches!(n, Node::Meter { used: Some(v), .. } if *v == 42.0)));
    assert!(stale
        .nodes
        .iter()
        .any(|n| matches!(n, Node::Notice { text, .. } if text.contains("Offline"))));
}
#[test] // SQ-8: children belong to their main session, not a separate inferred task.
fn sessions_group_by_project_and_keep_explicit_states() {
    let mut snapshot = Snapshot::default();
    for (id, parent, state) in [
        ("main", None, SessionState::Running),
        ("child", Some("main".to_string()), SessionState::Blocked),
    ] {
        snapshot.sessions.push(Session {
            id: id.into(),
            parent,
            state,
            assistant: "codex".into(),
            project: "Project".into(),
            project_path: "/tmp/project".into(),
            task: id.into(),
            updated_at: 1,
            progress: None,
            children_supported: true,
        });
    }
    let config = Config::default();
    let view = sq_widgets::registry().view(&config.widgets[4], &snapshot, 100);
    let text = serde_json::to_string(&view).unwrap();
    assert!(text.contains("Blocked"));
    if let Node::Row { children, .. } = &view.nodes[0] {
        if let Node::Row { children, .. } = &children[0] {
            assert_eq!(children.len(), 1);
        } else {
            panic!("missing main session");
        }
    } else {
        panic!("missing project");
    }
}

// REQ SQ-5 SQ-6: selected duration windows appear once, with model limits retained.
#[test]
fn account_labels_do_not_duplicate_primary_quota_meters() {
    let config = Config::default();
    let mut snapshot = Snapshot::default();
    snapshot.usage.insert(
        "codex".into(),
        Usage {
            windows: vec![
                QuotaWindow {
                    minutes: 300,
                    used: 20.0,
                    resets_at: None,
                    label: "account".into(),
                },
                QuotaWindow {
                    minutes: 10080,
                    used: 40.0,
                    resets_at: None,
                    label: "account".into(),
                },
                QuotaWindow {
                    minutes: 300,
                    used: 3.0,
                    resets_at: None,
                    label: "other-model".into(),
                },
            ],
            updated_at: None,
        },
    );
    let view = sq_widgets::registry().view(&config.widgets[1], &snapshot, 100);
    assert_eq!(
        view.nodes
            .iter()
            .filter(|n| matches!(n, Node::Meter { .. }))
            .count(),
        3
    );
}

fn session(id: &str, project: &str, path: &str, parent: Option<&str>, at: i64) -> Session {
    Session {
        id: id.into(),
        assistant: "codex".into(),
        project: project.into(),
        project_path: path.into(),
        parent: parent.map(String::from),
        updated_at: at,
        task: id.into(),
        state: SessionState::Running,
        progress: None,
        children_supported: true,
    }
}
// REQ SQ-8: distinct directories sharing a basename are independent projects.
#[test]
fn project_identity_uses_path_and_orphan_children_remain_visible() {
    let snapshot = Snapshot {
        sessions: vec![
            session("a", "same", "/first/same", None, 0),
            session("b", "same", "/second/same", None, 0),
            session("orphan", "same", "/second/same", Some("expired-parent"), 0),
        ],
        ..Snapshot::default()
    };
    let view = sq_widgets::registry().view(&Config::default().widgets[4], &snapshot, 0);
    let projects: Vec<_> = view
        .nodes
        .iter()
        .filter_map(|n| {
            if let Node::Row {
                id,
                detail,
                children,
                ..
            } = n
            {
                Some((id, detail, children))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        projects.len(),
        2,
        "equal basenames must not merge directories"
    );
    assert_ne!(projects[0].0, projects[1].0);
    assert_eq!(projects[0].1, "/first/same");
    let second = serde_json::to_string(&projects[1].2).unwrap();
    assert!(
        second.contains("orphan"),
        "missing parent must not hide child"
    );
    assert!(second.contains("Parent session unavailable"));
}
// REQ SQ-8: an active child preserves the context of an older inactive parent.
#[test]
fn active_child_keeps_older_parent_context_visible() {
    let mut snapshot = Snapshot::default();
    let mut main = session("main", "project", "/project", None, 0);
    main.state = SessionState::Completed;
    snapshot.sessions = vec![
        main,
        session("child", "project", "/project", Some("main"), 200000),
    ];
    let view = sq_widgets::registry().view(&Config::default().widgets[4], &snapshot, 200000);
    let text = serde_json::to_string(&view).unwrap();
    assert!(text.contains("main"));
    assert!(text.contains("child"));
}
// REQ SQ-8, SQ-10, SQ-11: observed dates are readable UTC, including leap days.
#[test]
fn session_activity_and_memory_dates_are_human_readable() {
    let mut snapshot = Snapshot::default();
    snapshot
        .sessions
        .push(session("main", "project", "/project", None, 1709164800));
    snapshot.activity.push(Activity {
        id: "a".into(),
        repository: "owner/repo".into(),
        kind: "commit".into(),
        title: "Change".into(),
        at: 1709164800,
        url: "https://github.com/owner/repo".into(),
    });
    snapshot.memory.push(MemoryItem {
        id: "m".into(),
        instance: "codex".into(),
        kind: "pref".into(),
        key: "key".into(),
        body: "body".into(),
        at: 1709164800,
        target: None,
        path: "/memory/event".into(),
    });
    let config = Config::default();
    for index in [4, 5, 6] {
        let text = serde_json::to_string(&sq_widgets::registry().view(
            &config.widgets[index],
            &snapshot,
            1709164800,
        ))
        .unwrap();
        assert!(text.contains("2024-02-29 00:00 UTC"), "{text}");
        assert!(
            !text.contains("1709164800"),
            "raw epoch remains in a display row"
        );
    }
}
// REQ SQ-5: model-scoped limits never masquerade as missing account limits.
#[test]
fn model_only_limits_leave_account_percentages_unavailable() {
    let config = Config::default();
    let mut snapshot = Snapshot::default();
    for provider in ["claude", "codex"] {
        snapshot.usage.insert(
            provider.into(),
            Usage {
                windows: vec![QuotaWindow {
                    minutes: 10080,
                    used: 44.0,
                    resets_at: Some(0),
                    label: if provider == "claude" {
                        "seven_day_sonnet"
                    } else {
                        "special-model"
                    }
                    .into(),
                }],
                updated_at: None,
            },
        );
    }
    for index in [0, 1] {
        let view = sq_widgets::registry().view(&config.widgets[index], &snapshot, 0);
        assert!(matches!(&view.nodes[0], Node::Meter { used: None, .. }));
        assert!(matches!(&view.nodes[1], Node::Meter { used: None, .. }));
        assert!(matches!(
            &view.nodes[2],
            Node::Meter {
                used: Some(44.0),
                ..
            }
        ));
    }
}
// REQ SQ-8: nested agents retain their actual parent instead of a false orphan label.
#[test]
fn nested_agents_are_expandable_under_their_parent() {
    let snapshot = Snapshot {
        sessions: vec![
            session("main", "project", "/project", None, 0),
            session("child", "project", "/project", Some("main"), 0),
            session("grandchild", "project", "/project", Some("child"), 0),
        ],
        ..Snapshot::default()
    };
    let view = sq_widgets::registry().view(&Config::default().widgets[4], &snapshot, 0);
    let Node::Row {
        children: projects, ..
    } = &view.nodes[0]
    else {
        panic!("project missing")
    };
    let Node::Row {
        children: agents, ..
    } = &projects[0]
    else {
        panic!("main missing")
    };
    let Node::Row {
        children: descendants,
        ..
    } = &agents[0]
    else {
        panic!("child missing")
    };
    assert_eq!(descendants.len(), 1);
    assert!(!serde_json::to_string(&view)
        .unwrap()
        .contains("Parent session unavailable"));
}
// REQ SQ-8, SQ-10, SQ-11: date boundaries never overflow or imply a local timezone.
#[test]
fn utc_display_handles_negative_epoch_century_leap_year_and_extreme_values() {
    assert_eq!(sq_widgets::format_utc(-1), "1969-12-31 23:59 UTC");
    assert_eq!(sq_widgets::format_utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(sq_widgets::format_utc(951782400), "2000-02-29 00:00 UTC");
    assert_eq!(sq_widgets::format_utc(i64::MAX), "Timestamp unavailable");
    assert_eq!(sq_widgets::format_utc(i64::MIN), "Timestamp unavailable");
}
// REQ SQ-8: deep active descendants preserve every retained ancestor context.
#[test]
fn active_grandchild_keeps_inactive_main_and_child_in_the_tree() {
    let mut main = session("main", "project", "/project", None, 0);
    main.state = SessionState::Completed;
    let mut child = session("child", "project", "/project", Some("main"), 0);
    child.state = SessionState::Idle;
    let snapshot = Snapshot {
        sessions: vec![
            main,
            child,
            session("active", "project", "/project", Some("child"), 200000),
        ],
        ..Snapshot::default()
    };
    let view = sq_widgets::registry().view(&Config::default().widgets[4], &snapshot, 200000);
    let text = serde_json::to_string(&view).unwrap();
    assert!(
        text.contains("codex:main"),
        "inactive main context disappeared"
    );
    assert!(
        text.contains("codex:child"),
        "inactive intermediate agent context disappeared"
    );
    assert!(!text.contains("Parent context unavailable"));
}
