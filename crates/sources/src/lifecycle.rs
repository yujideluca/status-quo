use serde::{Deserialize, Serialize};
use serde_json::Value;
use sq_core::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub version: u32,
    pub id: String,
    pub assistant: String,
    pub project_path: String,
    pub at: i64,
    pub sequence: u64,
    pub state: SessionState,
    pub label: String,
    pub parent: Option<String>,
}
pub fn lifecycle_event(
    assistant: &str,
    event: &str,
    input: &Value,
    at: i64,
    sequence: u64,
) -> Result<LifecycleEvent, String> {
    if !["claude", "codex"].contains(&assistant) {
        return Err("Unsupported lifecycle provider".into());
    }
    let session = input
        .get("session_id")
        .or_else(|| input.get("thread_id"))
        .and_then(Value::as_str)
        .ok_or("Missing hook session identity")?;
    let child = event == "SubagentStart" || event == "SubagentStop";
    let id = if child {
        input
            .get("agent_id")
            .or_else(|| input.get("subagent_id"))
            .and_then(Value::as_str)
            .ok_or("Missing subagent identity")?
    } else {
        session
    };
    for id in [id, session] {
        if id.is_empty()
            || id.len() > 200
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err("Invalid hook identity".into());
        }
    }
    let state = match event {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "SubagentStart" => {
            SessionState::Running
        }
        "PermissionRequest" | "Stop" | "Notification" => SessionState::Waiting,
        "SubagentStop" => SessionState::Completed,
        "SessionEnd" | "Interrupt" => SessionState::Idle,
        "SessionStart" => SessionState::Idle,
        _ => return Err("Unsupported lifecycle event".into()),
    };
    let label = if child {
        input
            .get("agent_type")
            .and_then(Value::as_str)
            .map(|s| format!("Subagent: {s}"))
            .unwrap_or_else(|| "Subagent task".into())
    } else if matches!(event, "PreToolUse" | "PostToolUse") {
        input
            .get("tool_name")
            .and_then(Value::as_str)
            .map(|s| format!("Using {s}"))
            .unwrap_or_else(|| "Session active".into())
    } else {
        match state {
            SessionState::Waiting => "Waiting for input",
            SessionState::Running => "Session active",
            _ => "Session idle",
        }
        .into()
    };
    Ok(LifecycleEvent {
        version: 1,
        id: id.into(),
        assistant: assistant.into(),
        project_path: input
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        at,
        sequence,
        state,
        label: label
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect(),
        parent: if child { Some(session.into()) } else { None },
    })
}
