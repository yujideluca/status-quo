use sq_core::*;

fn observation(
    id: &str,
    stream: &str,
    at: i64,
    input: u64,
    output: u64,
    cache: Option<u64>,
) -> TokenRecord {
    TokenRecord {
        id: id.into(),
        provider: "codex".into(),
        at,
        // A restarted parser may emit the full cumulative amount as its local delta.
        // The shared ledger must reconcile using the global stream metadata instead.
        input: input - cache.unwrap_or(0),
        output,
        cache_read: cache,
        cache_write: None,
        cumulative: Some(Cumulative {
            stream: stream.into(),
            subsecond: 0,
            input,
            output,
            cache_read: cache,
        }),
    }
}
fn totals(ledger: &TokenLedger, start: i64, end: i64) -> (u64, u64, Option<u64>) {
    let buckets = ledger.buckets("codex", start, end, 1000).unwrap();
    assert_eq!(buckets.len(), 1);
    (buckets[0].input, buckets[0].output, buckets[0].cache_read)
}

// REQ SQ-4, SQ-7: independently read copies share one cumulative stream.
#[test]
fn overlapping_transcripts_and_restart_reconcile_to_one_global_total() {
    let records = vec![
        observation("copy1-a", "session", 10, 100, 10, Some(20)),
        observation("copy2-a", "session", 10, 100, 10, Some(20)),
        observation("copy1-b", "session", 20, 160, 16, Some(30)),
        observation("copy2-b", "session", 20, 160, 16, Some(30)),
    ];
    let mut ledger = TokenLedger::default();
    for record in records.iter().rev() {
        ledger.insert(record.clone()).unwrap();
    }
    assert_eq!(totals(&ledger, 0, 30), (130, 16, Some(30)));
    let saved = serde_json::to_string(&ledger).unwrap();
    let mut restored: TokenLedger = serde_json::from_str(&saved).unwrap();
    for record in records {
        assert!(!restored.insert(record).unwrap());
    }
    assert_eq!(totals(&restored, 0, 30), (130, 16, Some(30)));
}

// REQ SQ-4, SQ-7: lexical transcript IDs do not define cumulative event order.
#[test]
fn same_timestamp_counter_observations_use_monotonic_order_not_record_id() {
    let mut ledger = TokenLedger::default();
    ledger
        .insert(observation("a-later", "session", 10, 150, 15, Some(30)))
        .unwrap();
    ledger
        .insert(observation("z-earlier", "session", 10, 100, 10, Some(20)))
        .unwrap();
    assert_eq!(totals(&ledger, 0, 20), (120, 15, Some(30)));
}

// REQ SQ-4, SQ-7: a partial counter decrease is rejected or explicitly unavailable.
#[test]
fn partial_regression_cannot_be_counted_as_a_whole_stream_reset() {
    let mut ledger = TokenLedger::default();
    ledger
        .insert(observation("first", "session", 10, 100, 20, Some(10)))
        .unwrap();
    ledger
        .insert(observation(
            "partial-regression",
            "session",
            20,
            120,
            19,
            Some(12),
        ))
        .unwrap();
    assert!(
        ledger.buckets("codex", 0, 30, 1000).is_err(),
        "a regressive observation must not silently add its full totals"
    );
    assert_eq!(
        ledger.records.len(),
        2,
        "unavailable graph must preserve observations for diagnosis"
    );
}

// REQ SQ-4: explicitly distinct reset epochs begin a new count without merging streams.
#[test]
fn explicit_reset_epoch_and_independent_sessions_count_separately() {
    let mut ledger = TokenLedger::default();
    for record in [
        observation("before", "session:epoch-0", 10, 100, 10, Some(20)),
        observation("after", "session:epoch-1", 20, 20, 2, Some(5)),
        observation("other", "other-session", 20, 30, 3, Some(10)),
    ] {
        ledger.insert(record).unwrap();
    }
    assert_eq!(totals(&ledger, 0, 30), (115, 15, Some(35)));
}

// REQ SQ-4: retain only the predecessor needed to subtract old cumulative usage.
#[test]
fn retention_keeps_the_latest_predecessor_without_recounting_old_usage() {
    let mut ledger = TokenLedger::default();
    for record in [
        observation("old", "session", 0, 100, 10, Some(20)),
        observation("baseline", "session", 10, 200, 20, Some(40)),
        observation("new", "session", 20, 260, 26, Some(60)),
    ] {
        ledger.insert(record).unwrap();
    }
    assert_eq!(ledger.prune(15), 1);
    assert!(!ledger.records.contains_key("old"));
    assert!(ledger.records.contains_key("baseline"));
    assert_eq!(totals(&ledger, 15, 30), (40, 6, Some(20)));
}

// REQ SQ-4: equal timestamps cannot retain the smaller predecessor by lexical accident.
#[test]
fn retention_uses_the_highest_same_timestamp_predecessor() {
    let mut ledger = TokenLedger::default();
    for record in [
        observation("a-low", "session", 10, 100, 10, Some(20)),
        observation("z-high", "session", 10, 200, 20, Some(40)),
        observation("new", "session", 20, 260, 26, Some(60)),
    ] {
        ledger.insert(record).unwrap();
    }
    ledger.prune(15);
    assert_eq!(totals(&ledger, 15, 30), (40, 6, Some(20)));
}

// REQ SQ-4: an unsupported cache counter remains unknown across later observations.
#[test]
fn unknown_cache_is_not_invented_from_a_later_known_cumulative_value() {
    let mut ledger = TokenLedger::default();
    ledger
        .insert(observation("first", "session", 10, 100, 10, None))
        .unwrap();
    ledger
        .insert(observation("next", "session", 20, 150, 15, Some(30)))
        .unwrap();
    assert_eq!(totals(&ledger, 0, 30), (150, 15, None));
    assert_eq!(totals(&ledger, 15, 30), (50, 5, None));
}

// REQ SQ-4, SQ-12: overflow and rejected updates cannot overwrite original observations.
#[test]
fn cumulative_aggregation_overflow_is_explicit_and_preserves_original_data() {
    let mut ledger = TokenLedger::default();
    let original = observation("first", "one", 10, u64::MAX, 0, Some(0));
    ledger.insert(original.clone()).unwrap();
    ledger
        .insert(observation("second", "two", 10, 1, 0, Some(0)))
        .unwrap();
    assert!(ledger.buckets("codex", 0, 20, 1000).is_err());
    assert_eq!(ledger.records["first"], original);
    let mut regression = original.clone();
    regression.input = 0;
    regression.cumulative.as_mut().unwrap().input = 0;
    assert!(ledger.insert(regression).is_err());
    assert_eq!(ledger.records["first"], original);
}

// REQ SQ-4, SQ-7: all counters dropping at a later observation is an explicit reset.
#[test]
fn full_counter_reset_on_the_same_stream_counts_new_usage_once() {
    let mut ledger = TokenLedger::default();
    for record in [
        observation("before", "session", 10, 100, 10, Some(20)),
        observation("after", "session", 20, 20, 2, Some(5)),
        observation("after-copy", "session", 20, 20, 2, Some(5)),
    ] {
        ledger.insert(record).unwrap();
    }
    assert_eq!(totals(&ledger, 0, 30), (95, 12, Some(25)));
}
// REQ SQ-4, SQ-7: subsecond event order takes priority over magnitude and lexical IDs.
#[test]
fn subsecond_order_preserves_a_real_reset_within_one_second() {
    let mut ledger = TokenLedger::default();
    let mut before = observation("z-before", "session", 10, 100, 10, Some(20));
    before.cumulative.as_mut().unwrap().subsecond = 100;
    let mut after = observation("a-after", "session", 10, 20, 2, Some(5));
    after.cumulative.as_mut().unwrap().subsecond = 900;
    ledger.insert(after).unwrap();
    ledger.insert(before).unwrap();
    assert_eq!(totals(&ledger, 0, 20), (95, 12, Some(25)));
}
// REQ SQ-4, SQ-7: conflicting dimensions at one exact instant have no safe inferred order.
#[test]
fn same_instant_mixed_counter_directions_are_explicitly_ambiguous() {
    let mut ledger = TokenLedger::default();
    ledger
        .insert(observation("one", "session", 10, 100, 20, Some(20)))
        .unwrap();
    ledger
        .insert(observation("two", "session", 10, 120, 19, Some(22)))
        .unwrap();
    assert!(ledger.buckets("codex", 0, 20, 1000).is_err());
}
// REQ SQ-4: retention baseline honors subsecond reset order rather than maximum count.
#[test]
fn predecessor_after_a_subsecond_reset_is_the_retained_baseline() {
    let mut ledger = TokenLedger::default();
    let mut before = observation("a-before", "session", 10, 100, 10, Some(20));
    before.cumulative.as_mut().unwrap().subsecond = 100;
    let mut after = observation("z-after", "session", 10, 20, 2, Some(5));
    after.cumulative.as_mut().unwrap().subsecond = 900;
    for record in [
        before,
        after,
        observation("next", "session", 20, 30, 3, Some(8)),
    ] {
        ledger.insert(record).unwrap();
    }
    ledger.prune(15);
    assert_eq!(totals(&ledger, 15, 30), (7, 1, Some(3)));
}
// REQ SQ-12: saved pre-cumulative records remain readable after schema extension.
#[test]
fn missing_cumulative_metadata_and_subsecond_fields_default_safely() {
    let record:TokenRecord=serde_json::from_value(serde_json::json!({"id":"legacy","provider":"codex","at":0,"input":1,"output":0,"cache_read":null,"cache_write":null})).unwrap();
    assert_eq!(record.cumulative, None);
    let cumulative: Cumulative = serde_json::from_value(
        serde_json::json!({"stream":"session","input":100,"output":10,"cache_read":20}),
    )
    .unwrap();
    assert_eq!(cumulative.subsecond, 0);
}
