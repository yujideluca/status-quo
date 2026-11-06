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
        account: "github".into(),
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
#[test] // SQ-30: the system widget shows the latest values and keeps unreadable ones unavailable.
fn system_widget_graphs_cpu_and_temperature_and_never_invents_zero() {
    let registry = sq_widgets::registry();
    let mut config = Config::default();
    config.add_widget("system", "local");
    let widget = config.widgets.last().unwrap();
    let empty = registry.view(widget, &Snapshot::default(), 100);
    assert!(empty
        .nodes
        .iter()
        .any(|n| matches!(n, Node::Text(t) if t.contains("Waiting"))));
    let snapshot = Snapshot {
        system: vec![
            SystemSample {
                at: 90,
                cpu: Some(12.0),
                temp: None,
                ram: None,
            },
            SystemSample {
                at: 95,
                cpu: Some(37.0),
                temp: None,
                ram: None,
            },
        ],
        ..Default::default()
    };
    let view = registry.view(widget, &snapshot, 100);
    let texts: Vec<_> = view
        .nodes
        .iter()
        .filter_map(|n| {
            if let Node::Text(t) = n {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect();
    assert!(texts.contains(&"CPU 37%"));
    assert!(texts.contains(&"Temperature Unavailable"));
    let charts: Vec<_> = view
        .nodes
        .iter()
        .filter_map(|n| {
            if let Node::Chart { series, .. } = n {
                Some(series)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(charts.len(), 2);
    assert!(charts[1][0].points.iter().all(|p| p.value.is_none()));
}
#[test] // SQ-32: RAM shows used/total and swap; asleep GPU shows no invented numbers.
fn ram_and_gpu_widgets_show_values_and_never_invent_them() {
    let registry = sq_widgets::registry();
    let mut config = Config::default();
    config.add_widget("ram", "local");
    config.add_widget("gpu", "local");
    let (ram, gpu) = (
        &config.widgets[config.widgets.len() - 2],
        &config.widgets[config.widgets.len() - 1],
    );
    let snapshot = Snapshot {
        system: vec![SystemSample {
            at: 90,
            cpu: None,
            temp: None,
            ram: Some(RamSample {
                used: 12_288,
                total: 24_576,
                swap_used: 1_024,
                swap_total: 2_048,
            }),
        }],
        gpu: vec![
            GpuSample {
                id: "card2".into(),
                name: "NVIDIA nvidia (card2)".into(),
                at: 90,
                state: GpuState::Reading,
                util: Some(40.0),
                vram_used: Some(512.0),
                vram_total: Some(6144.0),
                temp: None,
                power: None,
                clock: None,
                awake_share: false,
            },
            GpuSample {
                id: "card2".into(),
                name: "NVIDIA nvidia (card2)".into(),
                at: 95,
                state: GpuState::Asleep,
                util: None,
                vram_used: None,
                vram_total: None,
                temp: None,
                power: None,
                clock: None,
                awake_share: false,
            },
            GpuSample {
                id: "card1".into(),
                name: "Intel i915 (card1)".into(),
                at: 95,
                state: GpuState::Reading,
                util: Some(12.0),
                vram_used: None,
                vram_total: None,
                temp: None,
                power: None,
                clock: Some(300.0),
                awake_share: true,
            },
        ],
        ..Default::default()
    };
    let texts = |view: View| -> Vec<String> {
        view.nodes
            .iter()
            .filter_map(|n| match n {
                Node::Text(t) | Node::Notice { text: t, .. } => Some(t.clone()),
                _ => None,
            })
            .collect()
    };
    let r = texts(registry.view(ram, &snapshot, 100));
    assert!(r.contains(&"RAM 12.0 of 24.0 GiB used (50%)".to_string()));
    assert!(r.contains(&"Swap 1.0 of 2.0 GiB used".to_string()));
    let g = texts(registry.view(gpu, &snapshot, 100));
    assert!(g.contains(&"NVIDIA nvidia (card2)".to_string()));
    assert!(g.iter().any(|t| t.contains("Powered down")));
    assert!(g.contains(&"Intel i915 (card1)".to_string()));
    assert!(g.contains(&"Awake 12% · 300 MHz".to_string()));
    let view = registry.view(gpu, &snapshot, 100);
    let Node::Chart { series, .. } = &view.nodes[2] else {
        panic!("chart")
    };
    assert_eq!(series[0].points[1].value, None);
}

fn instance(kind: &str, provider: &str) -> WidgetInstance {
    let mut config = Config::default();
    config.widgets.clear();
    let id = config.add_widget(kind, provider);
    config.widget(&id).unwrap().clone()
}
fn usage(used: f64) -> Usage {
    Usage {
        windows: vec![QuotaWindow {
            minutes: 300,
            used,
            resets_at: Some(500),
            label: "5 hour".into(),
        }],
        updated_at: Some(1),
    }
}
fn meters(view: &View) -> Vec<Option<f64>> {
    view.nodes
        .iter()
        .filter_map(|n| match n {
            Node::Meter { label, used, .. } if label == "5 hour" => Some(*used),
            _ => None,
        })
        .collect()
}

// REQ SQ-35: each widget shows the account it is set to, and only that one.
#[test]
fn quota_and_token_widgets_follow_their_own_account() {
    let registry = sq_widgets::registry();
    let mut snapshot = Snapshot::default();
    snapshot.usage.insert("claude".into(), usage(11.0));
    snapshot.usage.insert("claude:work".into(), usage(77.0));
    let personal = registry.view(&instance("quota", "claude"), &snapshot, 100);
    let work = registry.view(&instance("quota", "claude:work"), &snapshot, 100);
    let unknown = registry.view(&instance("quota", "claude:gone"), &snapshot, 100);
    assert_eq!(meters(&personal), [Some(11.0)]);
    assert_eq!(meters(&work), [Some(77.0)]);
    assert_eq!(
        meters(&unknown),
        [None],
        "an unknown account is unknown, never another's value"
    );
    assert_eq!(work.title, "claude:work / Account usage");
    let bucket = |input| Bucket {
        at: 0,
        input,
        output: 0,
        cache_read: None,
        cache_write: None,
    };
    snapshot.tokens.insert("claude".into(), vec![bucket(10)]);
    snapshot
        .tokens
        .insert("claude:work".into(), vec![bucket(999)]);
    let peak = |provider: &str| {
        let view = registry.view(&instance("tokens", provider), &snapshot, 3_600);
        view.nodes.iter().find_map(|n| match n {
            Node::Chart { series, .. } => series[0]
                .points
                .iter()
                .filter_map(|p| p.value)
                .reduce(f64::max),
            _ => None,
        })
    };
    assert_eq!(peak("claude"), Some(10.0));
    assert_eq!(peak("claude:work"), Some(999.0));
}

// REQ SQ-35: GitHub activity and sessions are filtered by account too; old records are the default login's.
#[test]
fn activity_and_session_widgets_filter_by_account() {
    let registry = sq_widgets::registry();
    let mut snapshot = Snapshot::default();
    let item = |account: &str, id: &str| Activity {
        account: account.into(),
        id: id.into(),
        repository: "o/r".into(),
        kind: "commits".into(),
        title: format!("title-{id}"),
        at: 100,
        url: "https://github.com/o/r/commit/1".into(),
    };
    snapshot.activity = vec![
        item("github", "a"),
        item("github:org", "b"),
        item("", "legacy"),
    ];
    let ids = |provider: &str| -> Vec<String> {
        registry
            .view(&instance("activity", provider), &snapshot, 200)
            .nodes
            .iter()
            .filter_map(|n| match n {
                Node::Row { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(ids("github"), ["a", "legacy"]);
    assert_eq!(ids("github:org"), ["b"]);
    assert_eq!(ids("both").len(), 3);
    let session = |assistant: &str, id: &str| Session {
        id: id.into(),
        assistant: assistant.into(),
        project: format!("app-{id}"),
        project_path: format!("/p/{id}"),
        task: "t".into(),
        state: SessionState::Running,
        updated_at: 100,
        parent: None,
        progress: None,
        children_supported: false,
    };
    snapshot.sessions = vec![session("claude", "one"), session("claude:work", "two")];
    let names = |provider: &str| -> usize {
        registry
            .view(&instance("sessions", provider), &snapshot, 110)
            .nodes
            .iter()
            .filter(|n| matches!(n, Node::Row { .. }))
            .count()
    };
    assert_eq!(names("claude"), 1);
    assert_eq!(names("claude:work"), 1);
    assert_eq!(names("both"), 2);
}

// REQ SQ-32: the GPU widget is named for any GPU, not for one vendor.
#[test]
fn the_gpu_widget_is_not_named_after_a_vendor() {
    let registry = sq_widgets::registry();
    let descriptor = registry
        .descriptors()
        .into_iter()
        .find(|d| d.kind == "gpu")
        .unwrap();
    assert_eq!(descriptor.title, "GPU");
    let view = registry.view(&instance("gpu", "local"), &Snapshot::default(), 0);
    assert_eq!(view.title, "System / GPU");
}
