use serde_json::json;
use sq_sources::*;

fn claude_line(at: &str, blocks: serde_json::Value) -> serde_json::Value {
    json!({"type":"assistant","timestamp":at,"cwd":"/work/app","sessionId":"s1",
        "message":{"id":"m","content":blocks}})
}
fn use_block(id: &str, name: &str, input: serde_json::Value) -> serde_json::Value {
    json!({"type":"tool_use","id":id,"name":name,"input":input})
}

// REQ SQ-38
#[test]
fn claude_task_list_gives_progress_and_the_current_step_from_the_transcript_alone() {
    let mut p = LogParser::new("claude", "s1");
    assert_eq!(p.plan.progress(), None);
    for (n, subject) in ["Core", "Tray", "Host"].iter().enumerate() {
        let line = claude_line(
            "2026-11-06T10:00:00Z",
            json!([use_block(
                &format!("c{n}"),
                "TaskCreate",
                json!({"subject": subject})
            )]),
        );
        p.ingest(&line).unwrap();
        // The same line written twice (streaming) counts once.
        p.ingest(&line).unwrap();
    }
    assert_eq!(p.plan.progress(), Some((0, 3)));
    let update = |p: &mut LogParser, id: &str, task: &str, status: &str| {
        p.ingest(&claude_line(
            "2026-11-06T10:05:00Z",
            json!([use_block(
                id,
                "TaskUpdate",
                json!({"taskId": task, "status": status})
            )]),
        ))
        .unwrap();
    };
    update(&mut p, "u1", "1", "completed");
    update(&mut p, "u2", "2", "in_progress");
    assert_eq!(p.plan.progress(), Some((1, 3)));
    assert_eq!(p.plan.current(), Some("Tray"));
    // An unknown id changes nothing; a deleted task leaves the list.
    update(&mut p, "u3", "99", "completed");
    assert_eq!(p.plan.progress(), Some((1, 3)));
    update(&mut p, "u4", "3", "deleted");
    assert_eq!(p.plan.progress(), Some((1, 2)));
    assert!(p.plan.at > 0);
}

// REQ SQ-38
#[test]
fn todo_write_and_codex_update_plan_replace_the_whole_list() {
    let mut p = LogParser::new("claude", "s1");
    p.ingest(&claude_line(
        "2026-11-06T10:00:00Z",
        json!([use_block(
            "t",
            "TodoWrite",
            json!({"todos":[
            {"content":"a","status":"completed","activeForm":"A"},
            {"content":"b","status":"in_progress","activeForm":"B"},
            {"content":"c","status":"pending","activeForm":"C"}]})
        )]),
    ))
    .unwrap();
    assert_eq!(p.plan.progress(), Some((1, 3)));
    assert_eq!(p.plan.current(), Some("b"));
    let mut c = LogParser::new("codex", "x1");
    let args = json!({"plan":[{"step":"Read","status":"completed"},{"step":"Write","status":"completed"},{"step":"Test","status":"in_progress"}]}).to_string();
    c.ingest(
        &json!({"type":"response_item","timestamp":"2026-11-06T10:00:00Z",
        "payload":{"type":"function_call","name":"update_plan","arguments":args}}),
    )
    .unwrap();
    assert_eq!(c.plan.progress(), Some((2, 3)));
    // A malformed call or another tool leaves the list as it was.
    c.ingest(&json!({"type":"response_item","payload":{"type":"function_call","name":"update_plan","arguments":"{broken"}})).unwrap();
    c.ingest(&json!({"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{}"}})).unwrap();
    assert_eq!(c.plan.progress(), Some((2, 3)));
}

// REQ SQ-38 SQ-12
#[test]
fn the_task_list_survives_a_saved_cursor_and_old_cursors_without_it_still_load() {
    let mut p = LogParser::new("claude", "s1");
    p.ingest(&claude_line(
        "2026-11-06T10:00:00Z",
        json!([use_block("c", "TaskCreate", json!({"subject":"x"}))]),
    ))
    .unwrap();
    let text = serde_json::to_string(&p).unwrap();
    let back: LogParser = serde_json::from_str(&text).unwrap();
    assert_eq!(back.plan.progress(), Some((0, 1)));
    let mut old: serde_json::Value = serde_json::from_str(&text).unwrap();
    old.as_object_mut().unwrap().remove("plan");
    let old: LogParser = serde_json::from_value(old).unwrap();
    assert_eq!(old.plan.progress(), None);
}

// REQ SQ-38 SQ-4
#[test]
fn old_saved_state_is_read_again_once() {
    let mut state = LocalState {
        version: 2,
        ..LocalState::default()
    };
    state.cursors.insert(
        "claude:x".into(),
        Cursor {
            offset: 10,
            inode: 1,
            parser: LogParser::new("claude", "x"),
            malformed: 0,
            deferred: 0,
            modified: 0,
            discarding: false,
            ..serde_json::from_str::<Cursor>(
                r#"{"offset":0,"inode":0,"parser":{"provider":"claude","session":"x","total":null,"sequence":0,"project":"","parent":null,"state":"Unknown","updated_at":0,"resets":0},"malformed":0,"deferred":0,"modified":0,"discarding":false}"#,
            )
            .unwrap()
        },
    );
    state.upgrade();
    assert!(state.cursors.is_empty());
    assert_eq!(state.version, 3);
    state.cursors.insert("keep".into(), state_cursor());
    state.upgrade();
    assert_eq!(state.cursors.len(), 1, "a current state is not reset again");
}
fn state_cursor() -> Cursor {
    serde_json::from_str(
        r#"{"offset":0,"inode":0,"parser":{"provider":"claude","session":"x","total":null,"sequence":0,"project":"","parent":null,"state":"Unknown","updated_at":0,"resets":0},"malformed":0,"deferred":0,"modified":0,"discarding":false}"#,
    )
    .unwrap()
}
