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
