use sq_core::*;

// REQ SQ-1
#[test]
fn removing_and_replacing_a_widget_preserves_other_instances_and_survives_serialization() {
    let mut config = Config::default();
    assert_eq!(config.widgets.len(), 7);
    let survivor = config.widgets[1].clone();
    let removed = config.widgets[0].id.clone();
    config.apply(ConfigAction::Remove(removed)).unwrap();
    assert_eq!(config.widgets[0], survivor);
    let added = config.add_widget("clock", "custom");
    assert_ne!(added, survivor.id);
    let saved = serde_json::to_string(&config).unwrap();
    let restored: Config = serde_json::from_str(&saved).unwrap();
    assert_eq!(config, restored);
    while let Some(widget) = config.widgets.last() {
        config
            .apply(ConfigAction::Remove(widget.id.clone()))
            .unwrap();
    }
    assert!(config.widgets.is_empty());
    config.add_widget("quota", "claude");
    assert_eq!(config.widgets.len(), 1);
}

// REQ SQ-2
#[test]
fn duplicate_instances_keep_independent_options_and_move_without_losing_identity() {
    let mut config = Config::default();
    let first = config.widgets[0].id.clone();
    let second = config.add_widget("quota", "claude");
    config
        .widget_mut(&second)
        .unwrap()
        .options
        .insert("range".into(), "30d".into());
    config
        .apply(ConfigAction::Move {
            id: second.clone(),
            index: 0,
        })
        .unwrap();
    assert_eq!(config.widgets[0].id, second);
    assert!(!config.widget(&first).unwrap().options.contains_key("range"));
    assert!(config
        .apply(ConfigAction::Move {
            id: "missing".into(),
            index: 0
        })
        .is_err());
}

// REQ SQ-3
#[test]
fn invalid_imports_cannot_replace_valid_settings_and_navigation_is_independent() {
    let mut config = Config::default();
    let original = config.clone();
    let mut invalid = original.clone();
    invalid.theme.opacity = f32::NAN;
    assert!(config
        .apply(ConfigAction::Replace(Box::new(invalid)))
        .is_err());
    assert_eq!(config, original);
    let mut invalid = original.clone();
    invalid.version = 42;
    assert!(config
        .apply(ConfigAction::Replace(Box::new(invalid)))
        .is_err());
    config.navigation = false;
    assert!(!config.allows(&HostAction::OpenUrl(
        "https://github.com/example/repo".into()
    )));
    assert!(config.allows(&HostAction::LocalControl));
    config.theme.opacity = 0.0;
    config.validate().unwrap();
}

// REQ SQ-4
#[test]
fn token_buckets_are_deduplicated_bounded_and_keep_unknown_cache_distinct_from_zero() {
    let mut ledger = TokenLedger::default();
    let record = TokenRecord {
        id: "message-a".into(),
        provider: "claude".into(),
        at: 3_601,
        input: 100,
        output: 20,
        cache_read: Some(30),
        cache_write: None,
        cumulative: None,
    };
    ledger.insert(record.clone()).unwrap();
    ledger.insert(record).unwrap();
    ledger
        .insert(TokenRecord {
            id: "message-b".into(),
            provider: "claude".into(),
            at: 7_200,
            input: 40,
            output: 10,
            cache_read: None,
            cache_write: Some(0),
            cumulative: None,
        })
        .unwrap();
    let buckets = ledger.buckets("claude", 0, 10_000, 3_600).unwrap();
    assert_eq!(buckets.len(), 2);
    assert_eq!(buckets[0].at, 3_600);
    assert_eq!(buckets[0].input, 100);
    assert_eq!(buckets[0].cache_read, Some(30));
    assert_eq!(buckets[1].cache_read, None);
    assert_eq!(buckets[0].cache_write, None);
    assert!(ledger.buckets("claude", 0, 10_000, 0).is_err());
    ledger.prune(7_200);
    assert_eq!(ledger.records.len(), 1);
}

// REQ SQ-4
#[test]
fn overflow_and_old_partial_updates_never_silently_destroy_real_counters() {
    let mut ledger = TokenLedger::default();
    let a = TokenRecord {
        id: "a".into(),
        provider: "codex".into(),
        at: 1,
        input: u64::MAX,
        output: 0,
        cache_read: Some(0),
        cache_write: Some(0),
        cumulative: None,
    };
    ledger.insert(a.clone()).unwrap();
    let mut partial = a.clone();
    partial.input = 0;
    assert!(ledger.insert(partial).is_err());
    assert_eq!(ledger.records["a"].input, u64::MAX);
    let mut b = a;
    b.id = "b".into();
    b.input = 1;
    ledger.insert(b).unwrap();
    assert!(ledger.buckets("codex", 0, 10, 10).is_err());
}

// REQ SQ-5
#[test]
fn percent_windows_are_selected_by_duration_and_unknown_is_not_zero() {
    let usage = Usage {
        windows: vec![QuotaWindow {
            minutes: 10_080,
            used: 54.0,
            resets_at: None,
            label: "weekly".into(),
        }],
        ..Default::default()
    };
    assert!(usage.window(300).is_none());
    assert_eq!(usage.window(10_080).unwrap().used, 54.0);
    assert!(QuotaWindow {
        used: -1.0,
        ..usage.windows[0].clone()
    }
    .validate()
    .is_err());
    assert!(QuotaWindow {
        used: 103.0,
        ..usage.windows[0].clone()
    }
    .validate()
    .is_ok());
}
#[test]
// REQ SQ-12 SQ-21: invalid nested presets cannot bypass top-level import validation.
fn imported_presets_and_paths_receive_the_same_validation_as_active_settings() {
    let mut config = Config::default();
    let mut bad = Config::default().widgets;
    bad[0].span = 0;
    config.presets.insert(
        "bad".into(),
        Preset {
            theme: Theme::default(),
            widgets: bad,
            layer: Layer::Behind,
        },
    );
    assert!(config.validate().is_err());
    config.presets.clear();
    config.sources.project_roots = vec!["relative/not/declared".into()];
    assert!(config.validate().is_err());
    assert!(validate_repository("../repo").is_err());
}

// REQ SQ-2 SQ-29
#[test]
fn free_form_widget_rectangles_are_validated_and_old_configs_still_load() {
    let mut config = Config::default();
    let mut json: serde_json::Value = serde_json::to_value(&config).unwrap();
    for w in json["widgets"].as_array_mut().unwrap() {
        let o = w.as_object_mut().unwrap();
        o.remove("x");
        o.remove("y");
        o.remove("width");
    }
    let old: Config = serde_json::from_value(json).unwrap();
    assert!(old.widgets.iter().all(|w| w.width == 0.0));
    old.validate().unwrap();
    config.widgets[0].width = 3_000.0;
    config.widgets[0].height = 2_000.0;
    config.widgets[0].x = 1_500.0;
    config.widgets[0].y = 900.0;
    config.validate().unwrap();
    for bad in [
        |w: &mut WidgetInstance| w.width = 50.0,
        |w: &mut WidgetInstance| w.width = f32::NAN,
        |w: &mut WidgetInstance| w.x = -1.0,
        |w: &mut WidgetInstance| w.y = f32::INFINITY,
        |w: &mut WidgetInstance| w.height = 10.0,
    ] {
        let mut candidate = config.clone();
        bad(&mut candidate.widgets[1]);
        assert!(candidate.validate().is_err());
    }
}

// REQ SQ-2 SQ-29
#[test]
fn a_widget_added_to_a_free_canvas_lands_visible_with_its_own_rectangle() {
    let mut config = Config::default();
    let before = config.add_widget("quota", "claude");
    assert_eq!(
        config.widget(&before).unwrap().width,
        0.0,
        "flow layouts stay flow layouts"
    );
    config.widgets[0].width = 500.0;
    let id = config.add_widget("quota", "claude");
    let added = config.widget(&id).unwrap();
    assert!(added.width > 0.0 && added.x == 0.0 && added.y == 0.0);
    config.validate().unwrap();
}

fn account(id: &str, kind: &str, path: &str) -> Account {
    Account {
        id: id.into(),
        kind: kind.into(),
        path: path.into(),
        ..Account::default()
    }
}

// REQ SQ-35
#[test]
fn extra_accounts_have_keys_kinds_and_a_strict_identity() {
    assert_eq!(
        account("work", "claude", "~/.claude-work").key(),
        "claude:work"
    );
    assert_eq!(account_kind("claude:work"), "claude");
    assert_eq!(account_kind("github"), "github");
    let mut config = Config::default();
    config.sources.accounts = vec![
        account("work", "claude", "~/.claude-work"),
        account("team_2", "codex", "/srv/codex-team"),
        Account {
            repositories: vec!["o/r".into()],
            ..account("org", "github", "")
        },
    ];
    config.validate().unwrap();
    let all = config.sources.assistant_accounts();
    let keys: Vec<_> = all.iter().map(|(k, ..)| k.as_str()).collect();
    assert_eq!(keys, ["claude", "codex", "claude:work", "codex:team_2"]);
    assert_eq!(
        config.sources.account_keys("claude"),
        ["claude", "claude:work"]
    );
    assert_eq!(
        config.sources.account_keys("github"),
        ["github", "github:org"]
    );
    for (id, kind, path) in [
        ("", "claude", "~/x"),
        ("Work", "claude", "~/x"),
        ("a b", "claude", "~/x"),
        ("-x", "claude", "~/x"),
        ("work", "gitlab", "~/x"),
        ("work", "claude", "relative/path"),
        ("work", "claude", ""),
        (&"a".repeat(33), "claude", "~/x"),
    ] {
        let mut bad = Config::default();
        bad.sources.accounts = vec![account(id, kind, path)];
        assert!(bad.validate().is_err(), "accepted {id:?} {kind:?} {path:?}");
    }
    // The same kind and name twice, repositories on a Claude account, a bad repository.
    let mut bad = Config::default();
    bad.sources.accounts = vec![
        account("work", "claude", "~/a"),
        account("work", "claude", "~/b"),
    ];
    assert!(bad.validate().is_err());
    bad.sources.accounts = vec![Account {
        repositories: vec!["o/r".into()],
        ..account("work", "claude", "~/a")
    }];
    assert!(bad.validate().is_err());
    bad.sources.accounts = vec![Account {
        repositories: vec!["not a repository".into()],
        ..account("org", "github", "")
    }];
    assert!(bad.validate().is_err());
    // The same name under another kind is another account.
    let mut ok = Config::default();
    ok.sources.accounts = vec![account("x", "claude", "~/a"), account("x", "codex", "~/b")];
    ok.validate().unwrap();
    // At most 16 extra accounts.
    let mut many = Config::default();
    many.sources.accounts = (0..17)
        .map(|n| account(&format!("a{n}"), "claude", "~/a"))
        .collect();
    assert!(many.validate().is_err());
}

// REQ SQ-35: configurations written before accounts existed still load, and accounts round-trip.
#[test]
fn old_configurations_load_without_accounts_and_accounts_survive_serialization() {
    let mut value = serde_json::to_value(Config::default()).unwrap();
    value["sources"].as_object_mut().unwrap().remove("accounts");
    let old: Config = serde_json::from_value(value).unwrap();
    assert!(old.sources.accounts.is_empty());
    old.validate().unwrap();
    let mut config = Config::default();
    config.sources.accounts = vec![account("work", "claude", "~/.claude-work")];
    let again: Config = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(again, config);
}

// REQ SQ-35 SQ-36: absolute paths of every platform are accepted, relative ones are not.
#[test]
fn source_paths_accept_unix_home_windows_drive_and_unc_forms() {
    for ok in [
        "/home/me/.claude",
        "~/.claude",
        "~\\.claude",
        "C:\\Users\\me\\.claude",
        "c:/Users/me/.claude",
        "\\\\server\\share\\claude",
    ] {
        let mut config = Config::default();
        config.sources.claude = ok.into();
        config.validate().unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    for bad in ["claude", "./claude", "C:", "C:claude", "..\\x", "\\rooted"] {
        let mut config = Config::default();
        config.sources.claude = bad.into();
        assert!(config.validate().is_err(), "accepted {bad}");
    }
}
