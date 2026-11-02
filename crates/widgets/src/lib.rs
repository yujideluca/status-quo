//! Built-in widgets produce portable views; no window system or credential access.
use sq_contract::*;
use sq_core::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn registry() -> Registry {
    let mut registry = Registry::default();
    for kind in ["quota", "tokens", "sessions", "activity", "memory"] {
        registry
            .register(Box::new(Builtin(kind)))
            .expect("distinct built-in kinds");
    }
    registry
}
struct Builtin(&'static str);
impl Widget for Builtin {
    fn descriptor(&self) -> Descriptor {
        let (title, providers) = match self.0 {
            "quota" => ("Account usage", vec!["claude", "codex"]),
            "tokens" => ("Token history", vec!["claude", "codex"]),
            "sessions" => ("Sessions", vec!["both", "claude", "codex"]),
            "activity" => ("Recent GitHub activity", vec!["github"]),
            _ => ("Workflow memory", vec!["both", "claude", "codex"]),
        };
        Descriptor {
            kind: self.0.into(),
            title: title.into(),
            providers: providers.into_iter().map(String::from).collect(),
        }
    }
    fn view(&self, instance: &WidgetInstance, snapshot: &Snapshot, now: i64) -> View {
        let mut nodes = match self.0 {
            "quota" => quota(instance, snapshot),
            "tokens" => tokens(instance, snapshot, now),
            "sessions" => sessions(instance, snapshot, now),
            "activity" => snapshot
                .activity
                .iter()
                .filter(|a| {
                    instance
                        .options
                        .get("repository")
                        .is_none_or(|r| r == &a.repository)
                })
                .take(100)
                .map(|a| Node::Row {
                    id: a.id.clone(),
                    label: format!("{} · {}", a.repository, a.kind),
                    detail: format!("{}\n{}", a.title, format_utc(a.at)),
                    action: Some(HostAction::OpenUrl(a.url.clone())),
                    children: vec![],
                })
                .collect(),
            _ => snapshot
                .memory
                .iter()
                .filter(|m| {
                    (instance.provider == "both" || m.instance == instance.provider)
                        && instance.options.get("type").is_none_or(|t| t == &m.kind)
                })
                .take(100)
                .map(|m| Node::Row {
                    id: m.id.clone(),
                    label: format!("{} · {} · {}", m.instance, m.kind, m.key),
                    detail: format!(
                        "{}\nHistorical event at {}{}",
                        m.body,
                        format_utc(m.at),
                        m.target
                            .as_ref()
                            .map(|t| format!(" · target {t}"))
                            .unwrap_or_default()
                    ),
                    action: Some(HostAction::OpenMemory(m.path.clone())),
                    children: vec![],
                })
                .collect(),
        };
        let health_key = match self.0 {
            "tokens" => format!("{}-logs", instance.provider),
            "sessions" => "sessions".into(),
            "activity" => "github".into(),
            "memory" => "memory".into(),
            _ => instance.provider.clone(),
        };
        if let Some(health) = snapshot.health.get(&health_key) {
            if health.state != SourceState::Fresh || health.malformed > 0 || health.deferred > 0 {
                nodes.push(Node::Notice {
                    tone: if matches!(health.state, SourceState::Error | SourceState::Stale) {
                        Tone::Warning
                    } else {
                        Tone::Muted
                    },
                    text: format!(
                        "{:?}: {} · malformed {} · deferred {}",
                        health.state, health.message, health.malformed, health.deferred
                    ),
                });
            }
        } else {
            nodes.push(Node::Notice {
                tone: Tone::Muted,
                text: "Waiting for source status".into(),
            });
        }
        if nodes.is_empty() {
            nodes.push(Node::Text("No matching entries".into()));
        }
        View {
            title: format!("{} / {}", instance.provider, self.descriptor().title),
            nodes,
        }
    }
}
fn quota(instance: &WidgetInstance, snapshot: &Snapshot) -> Vec<Node> {
    let usage = snapshot.usage.get(&instance.provider);
    let mut result: Vec<_> = [(300, "5 hour"), (10080, "Weekly")]
        .into_iter()
        .map(|(duration, label)| {
            let window = usage.and_then(|u| {
                u.windows
                    .iter()
                    .find(|w| w.minutes == duration && account_label(&w.label, duration))
            });
            Node::Meter {
                label: label.into(),
                used: window.map(|w| w.used),
                resets_at: window.and_then(|w| w.resets_at),
            }
        })
        .collect();
    if let Some(usage) = usage {
        for (index, window) in usage.windows.iter().enumerate() {
            let selected = [300, 10080]
                .into_iter()
                .filter_map(|duration| {
                    usage
                        .windows
                        .iter()
                        .position(|w| w.minutes == duration && account_label(&w.label, duration))
                })
                .any(|i| i == index);
            if !selected {
                result.push(Node::Meter {
                    label: window.label.clone(),
                    used: Some(window.used),
                    resets_at: window.resets_at,
                });
            }
        }
    }
    result
}
fn tokens(instance: &WidgetInstance, snapshot: &Snapshot, now: i64) -> Vec<Node> {
    let days = match instance.options.get("range").map(String::as_str) {
        Some("7d") => 7,
        Some("30d") => 30,
        _ => 1,
    };
    let step = if instance.options.get("bucket").is_some_and(|b| b == "daily") {
        86400
    } else {
        3600
    };
    let start = (now - days * 86400).div_euclid(step) * step;
    let cumulative = instance
        .options
        .get("cumulative")
        .is_some_and(|s| s == "true");
    let buckets = snapshot
        .tokens
        .get(&instance.provider)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let filtered: Vec<_> = buckets
        .iter()
        .filter(|b| b.at >= start && b.at <= now)
        .cloned()
        .collect();
    let buckets = match regroup(&filtered, start, step, cumulative) {
        Ok(buckets) => buckets,
        Err(error) => {
            return vec![Node::Notice {
                tone: Tone::Error,
                text: error,
            }]
        }
    };
    if buckets.is_empty() {
        return vec![Node::Text(
            "No token observations in this range; missing data is unknown.".into(),
        )];
    }
    let series = ["Input", "Output", "Cache read", "Cache write"]
        .into_iter()
        .enumerate()
        .map(|(index, label)| Series {
            label: label.into(),
            points: buckets
                .iter()
                .map(|b| Point {
                    at: b.at,
                    value: match index {
                        0 => Some(b.input as f64),
                        1 => Some(b.output as f64),
                        2 => b.cache_read.map(|v| v as f64),
                        _ => b.cache_write.map(|v| v as f64),
                    },
                })
                .collect(),
        })
        .collect();
    vec![Node::Chart { series }]
}
fn account_label(label: &str, minutes: u32) -> bool {
    let label = label.to_ascii_lowercase();
    matches!(label.as_str(), "account" | "codex" | "claude")
        || (minutes == 300 && matches!(label.as_str(), "five_hour" | "5 hour" | "5h"))
        || (minutes == 10080 && matches!(label.as_str(), "seven_day" | "weekly" | "7d"))
}

/// Deterministic UTC display without IO, GUI or a platform clock dependency.
pub fn format_utc(at: i64) -> String {
    if !(-62_135_596_800..=253_402_300_799).contains(&at) {
        return "Timestamp unavailable".into();
    }
    let days = at.div_euclid(86400);
    let seconds = at.rem_euclid(86400);
    // Gregorian civil date from days since 1970-01-01, using 400-year eras.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let day_of_era = z - era * 146097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3600,
        seconds % 3600 / 60
    )
}
fn visible(session: &Session, history: bool, now: i64) -> bool {
    history
        || match session.state {
            SessionState::Running | SessionState::Waiting | SessionState::Blocked => true,
            SessionState::Completed => now.saturating_sub(session.updated_at) <= 3600,
            _ => now.saturating_sub(session.updated_at) <= 86400,
        }
}
fn sessions(instance: &WidgetInstance, snapshot: &Snapshot, now: i64) -> Vec<Node> {
    let history = instance.options.get("history").is_some_and(|s| s == "true");
    let available: Vec<_> = snapshot
        .sessions
        .iter()
        .filter(|s| instance.provider == "both" || instance.provider == s.assistant)
        .collect();
    // Preserve retained ancestors of visible descendants, including nested agents.
    let by_identity: BTreeMap<_, _> = available
        .iter()
        .map(|s| ((s.assistant.as_str(), s.id.as_str()), *s))
        .collect();
    let mut context = BTreeSet::new();
    for session in available.iter().filter(|s| visible(s, history, now)) {
        let mut current = *session;
        let mut seen = BTreeSet::new();
        loop {
            let identity = (current.assistant.as_str(), current.id.as_str());
            if !seen.insert(identity) {
                break;
            }
            context.insert(identity);
            let Some(parent) = current
                .parent
                .as_deref()
                .and_then(|id| by_identity.get(&(current.assistant.as_str(), id)))
            else {
                break;
            };
            current = parent;
        }
    }
    let roots: Vec<_> = available
        .iter()
        .copied()
        .filter(|s| s.parent.is_none() && context.contains(&(s.assistant.as_str(), s.id.as_str())))
        .collect();
    let mut projects: BTreeMap<String, (String, String, Vec<Node>)> = BTreeMap::new();
    let mut displayed = BTreeSet::new();
    for session in &roots {
        let row = session_tree(session, &available, &mut displayed, 0);
        add_project_row(&mut projects, session, row);
    }
    for session in available
        .iter()
        .filter(|s| s.parent.is_some() && visible(s, history, now))
    {
        if !displayed.contains(&(session.assistant.clone(), session.id.clone())) {
            let mut row = session_tree(session, &available, &mut displayed, 0);
            if let Node::Row { detail, .. } = &mut row {
                let parent_exists = available.iter().any(|p| {
                    p.assistant == session.assistant && session.parent.as_deref() == Some(&p.id)
                });
                let message = if parent_exists {
                    "Parent context unavailable or cyclic"
                } else {
                    "Parent session unavailable"
                };
                detail.push_str(&format!(
                    " · {message} ({})",
                    session.parent.as_deref().unwrap_or("unknown")
                ));
            }
            add_project_row(&mut projects, session, row);
        }
    }
    projects
        .into_iter()
        .map(|(identity, (label, detail, children))| Node::Row {
            id: format!("project:{identity}"),
            label,
            detail,
            action: None,
            children,
        })
        .collect()
}
fn session_tree(
    session: &Session,
    available: &[&Session],
    displayed: &mut BTreeSet<(String, String)>,
    depth: usize,
) -> Node {
    displayed.insert((session.assistant.clone(), session.id.clone()));
    let mut row = session_row(session);
    if let Node::Row { children, .. } = &mut row {
        for child in available.iter().filter(|s| {
            s.assistant == session.assistant && s.parent.as_deref() == Some(&session.id)
        }) {
            if displayed.contains(&(child.assistant.clone(), child.id.clone())) {
                continue;
            }
            if depth >= 64 {
                children.push(Node::Notice {
                    tone: Tone::Warning,
                    text: "Subagent nesting limit reached; remaining agents are shown separately"
                        .into(),
                });
                break;
            }
            children.push(session_tree(child, available, displayed, depth + 1));
        }
    }
    row
}
fn add_project_row(
    projects: &mut BTreeMap<String, (String, String, Vec<Node>)>,
    session: &Session,
    row: Node,
) {
    let identity = if session.project_path.is_empty() {
        format!("unlocated:{}", session.project)
    } else {
        session.project_path.clone()
    };
    projects
        .entry(identity)
        .or_insert_with(|| {
            (
                session.project.clone(),
                session.project_path.clone(),
                vec![],
            )
        })
        .2
        .push(row);
}
fn session_row(session: &Session) -> Node {
    Node::Row {
        id: format!("{}:{}", session.assistant, session.id),
        label: format!(
            "{} · {:?} · {}",
            session.assistant, session.state, session.task
        ),
        detail: format!(
            "Updated {}{}{}",
            format_utc(session.updated_at),
            session
                .progress
                .map(|(d, t)| format!(" · {d}/{t}"))
                .unwrap_or_default(),
            if session.children_supported {
                ""
            } else {
                " · subagent reporting unavailable"
            }
        ),
        action: Some(HostAction::OpenSession {
            assistant: session.assistant.clone(),
            id: session.id.clone(),
            project: session.project_path.clone(),
        }),
        children: vec![],
    }
}
