use chrono::{DateTime, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sq_core::*;
use std::collections::BTreeMap;

pub fn timestamp(value: &str) -> Result<i64, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|t| t.timestamp())
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ").map(|t| t.and_utc().timestamp())
        })
        .map_err(|_| "Invalid timestamp".into())
}
pub fn claude_usage(value: &Value, now: i64) -> Result<Usage, String> {
    let object = value
        .as_object()
        .ok_or("Claude usage response is not an object")?;
    let mut windows = Vec::new();
    for (key, value) in object {
        if key == "seven_day_breakdown"
            || value.is_null()
            || !(key == "five_hour" || key.starts_with("seven_day"))
        {
            continue;
        }
        let used = value
            .get("utilization")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("Missing Claude utilization for {key}"))?;
        let resets_at = match value.get("resets_at").and_then(Value::as_str) {
            Some(s) => Some(timestamp(s)?),
            None => None,
        };
        let window = QuotaWindow {
            minutes: if key == "five_hour" { 300 } else { 10080 },
            used,
            resets_at,
            label: key.clone(),
        };
        window.validate()?;
        windows.push(window);
    }
    // Put account-wide limits before model-specific limits for duration lookup.
    windows.sort_by_key(|w| match w.label.as_str() {
        "five_hour" => 0,
        "seven_day" => 1,
        _ => 2,
    });
    if windows.is_empty() {
        return Err("Claude returned no supported usage windows".into());
    }
    Ok(Usage {
        windows,
        updated_at: Some(now),
    })
}
pub fn codex_usage(value: &Value, now: i64) -> Result<Usage, String> {
    let mut windows = Vec::new();
    fn append(bucket: &Value, label: &str, windows: &mut Vec<QuotaWindow>) -> Result<(), String> {
        for key in ["primary", "secondary"] {
            if let Some(value) = bucket.get(key).filter(|v| !v.is_null()) {
                let Some(minutes) = value.get("windowDurationMins").and_then(Value::as_u64) else {
                    continue;
                };
                let window = QuotaWindow {
                    minutes: u32::try_from(minutes).map_err(|_| "Quota duration overflow")?,
                    used: value
                        .get("usedPercent")
                        .and_then(Value::as_f64)
                        .ok_or("Missing Codex utilization")?,
                    resets_at: value.get("resetsAt").and_then(Value::as_i64),
                    label: label.into(),
                };
                window.validate()?;
                if !windows.iter().any(|w| w == &window) {
                    windows.push(window);
                }
            }
        }
        Ok(())
    }
    if let Some(bucket) = value.get("rateLimits") {
        append(bucket, "account", &mut windows)?;
    }
    if let Some(buckets) = value.get("rateLimitsByLimitId").and_then(Value::as_object) {
        for (label, bucket) in buckets {
            if label != "codex" || windows.is_empty() {
                append(bucket, label, &mut windows)?;
            }
        }
    }
    if windows.is_empty() {
        return Err("Codex returned no duration-bearing quota windows".into());
    }
    Ok(Usage {
        windows,
        updated_at: Some(now),
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogParser {
    pub provider: String,
    pub session: String,
    pub total: Option<[u64; 3]>,
    pub sequence: u64,
    pub project: String,
    pub parent: Option<String>,
    pub state: SessionState,
    pub updated_at: i64,
    pub resets: u64,
}
impl LogParser {
    pub fn new(provider: &str, session: &str) -> Self {
        Self {
            provider: provider.into(),
            session: session.into(),
            total: None,
            sequence: 0,
            project: String::new(),
            parent: None,
            state: SessionState::Unknown,
            updated_at: 0,
            resets: 0,
        }
    }
    pub fn ingest(&mut self, value: &Value) -> Result<Option<TokenRecord>, String> {
        if let Some(ts) = value.get("timestamp").and_then(Value::as_str) {
            self.updated_at = timestamp(ts)?;
        }
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        if self.provider == "claude" {
            if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
                self.project = cwd.into();
            }
            if let Some(id) = value.get("sessionId").and_then(Value::as_str) {
                self.session = id.into();
            }
            if kind == "assistant" {
                let Some(usage) = value.pointer("/message/usage") else {
                    return Ok(None);
                };
                let id = value
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .ok_or("Claude usage has no message identity")?;
                return Ok(Some(TokenRecord {
                    id: format!("claude:{}:{id}", self.session),
                    provider: "claude".into(),
                    at: self.updated_at,
                    input: counter(usage, "input_tokens")?,
                    output: counter(usage, "output_tokens")?,
                    cache_read: optional_counter(usage, "cache_read_input_tokens")?,
                    cache_write: optional_counter(usage, "cache_creation_input_tokens")?,
                    cumulative: None,
                }));
            }
        } else {
            let payload = value.get("payload").unwrap_or(&Value::Null);
            if kind == "session_meta" {
                if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                    self.project = cwd.into();
                }
                if let Some(id) = payload.get("id").and_then(Value::as_str) {
                    self.session = id.into();
                }
                self.parent = payload
                    .pointer("/source/subagent/thread_spawn/parent_thread_id")
                    .and_then(Value::as_str)
                    .map(String::from);
            }
            if kind == "event_msg" {
                match payload.get("type").and_then(Value::as_str).unwrap_or("") {
                    "task_started" | "turn_started" => self.state = SessionState::Running,
                    "task_complete" | "turn_complete" => self.state = SessionState::Completed,
                    "turn_aborted" => self.state = SessionState::Idle,
                    "token_count" => {
                        let Some(usage) = payload
                            .pointer("/info/total_token_usage")
                            .filter(|v| !v.is_null())
                        else {
                            return Ok(None);
                        };
                        let input = counter(usage, "input_tokens")?;
                        let cache = optional_counter(usage, "cached_input_tokens")?;
                        if cache.is_some_and(|c| c > input) {
                            return Err(
                                "Codex cached input exceeds included input; record rejected".into(),
                            );
                        }
                        let current = [input, cache.unwrap_or(0), counter(usage, "output_tokens")?];
                        let mut delta = current;
                        if let Some(previous) = self.total {
                            if current == previous {
                                return Ok(None);
                            }
                            if current.iter().zip(previous).any(|(n, p)| *n < p) {
                                if current.iter().zip(previous).any(|(n, p)| *n > p) {
                                    return Err("Ambiguous partial Codex cumulative regression; original observations retained".into());
                                }
                                self.resets += 1;
                            } else {
                                for i in 0..3 {
                                    delta[i] -= previous[i];
                                }
                            }
                        }
                        if delta[1] > delta[0] {
                            return Err("Codex cache delta exceeds input delta; record retained for diagnosis".into());
                        }
                        self.total = Some(current);
                        self.sequence = self
                            .sequence
                            .checked_add(1)
                            .ok_or("Token event sequence overflow")?;
                        return Ok(Some(TokenRecord {
                            id: format!(
                                "codex:{}:{}:{}:{}:{}",
                                self.session,
                                value
                                    .get("timestamp")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown"),
                                current[0],
                                current[1],
                                current[2]
                            ),
                            provider: "codex".into(),
                            at: self.updated_at,
                            input: delta[0] - delta[1],
                            output: delta[2],
                            cache_read: cache.map(|_| delta[1]),
                            cache_write: None,
                            cumulative: Some(Cumulative {
                                stream: self.session.clone(),
                                subsecond: value
                                    .get("timestamp")
                                    .and_then(Value::as_str)
                                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                                    .map(|t| t.timestamp_subsec_nanos())
                                    .unwrap_or(0),
                                input: current[0],
                                output: current[2],
                                cache_read: cache,
                            }),
                        }));
                    }
                    _ => {}
                }
            }
        }
        Ok(None)
    }
}
fn counter(value: &Value, key: &str) -> Result<u64, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("Invalid or missing {key}"))
}
fn optional_counter(value: &Value, key: &str) -> Result<Option<u64>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| format!("Invalid {key}")),
    }
}

pub fn memory_event(text: &str, id: &str, path: &str) -> Result<MemoryItem, String> {
    let normalized = text.replace("\r\n", "\n");
    let body = normalized
        .strip_prefix("---\n")
        .ok_or("Missing memory frontmatter")?;
    let (head, body) = body
        .split_once("\n---\n")
        .ok_or("Incomplete memory frontmatter")?;
    let mut fields = BTreeMap::new();
    for line in head
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
    {
        let (k, v) = line.split_once(':').ok_or("Invalid memory metadata")?;
        fields.insert(k.trim().to_lowercase(), v.trim().to_string());
    }
    let kind = fields.get("type").ok_or("Missing memory event type")?;
    if ![
        "pref", "fact", "decision", "task", "note", "digest", "feedback", "claim", "done",
        "forget", "use", "archive", "reopen", "rescope",
    ]
    .contains(&kind.as_str())
    {
        return Err("Unknown memory event type".into());
    }
    let at = timestamp(
        fields
            .get("ts")
            .map(String::as_str)
            .unwrap_or_else(|| id.split('-').next().unwrap_or("")),
    )?;
    let instance = fields
        .get("instance")
        .cloned()
        .unwrap_or_else(|| id.split('-').nth(1).unwrap_or("unknown").into());
    Ok(MemoryItem {
        id: id.into(),
        instance,
        kind: kind.clone(),
        key: fields.get("key").cloned().unwrap_or_default(),
        body: {
            let text = body.trim();
            let mut excerpt: String = text.chars().take(8000).collect();
            if text.chars().count() > 8000 {
                excerpt.push_str("\n[excerpt truncated; open source event for full text]");
            }
            excerpt
        },
        at,
        target: fields.get("target").cloned(),
        path: path.into(),
    })
}
