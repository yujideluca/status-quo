use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct QuotaWindow {
    pub minutes: u32,
    pub used: f64,
    pub resets_at: Option<i64>,
    pub label: String,
}
impl QuotaWindow {
    pub fn validate(&self) -> Result<(), String> {
        if !self.used.is_finite() || self.used < 0.0 || self.minutes == 0 {
            return Err("Invalid quota percentage or window duration".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub windows: Vec<QuotaWindow>,
    pub updated_at: Option<i64>,
}
impl Usage {
    pub fn window(&self, minutes: u32) -> Option<&QuotaWindow> {
        self.windows.iter().find(|w| w.minutes == minutes)
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SourceState {
    #[default]
    Loading,
    Fresh,
    Empty,
    Unavailable,
    Stale,
    Error,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Health {
    pub state: SourceState,
    pub last_success: Option<i64>,
    pub retry_at: Option<i64>,
    pub message: String,
    pub malformed: u64,
    pub deferred: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Waiting,
    Blocked,
    Completed,
    Idle,
    #[default]
    Unknown,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub id: String,
    pub assistant: String,
    pub project: String,
    pub project_path: String,
    pub task: String,
    pub state: SessionState,
    pub updated_at: i64,
    pub parent: Option<String>,
    pub progress: Option<(u32, u32)>,
    pub children_supported: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    pub id: String,
    pub repository: String,
    pub kind: String,
    pub title: String,
    pub at: i64,
    pub url: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MemoryItem {
    pub id: String,
    pub instance: String,
    pub kind: String,
    pub key: String,
    pub body: String,
    pub at: i64,
    pub target: Option<String>,
    pub path: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub revision: u64,
    pub usage: BTreeMap<String, Usage>,
    pub tokens: BTreeMap<String, Vec<crate::Bucket>>,
    pub sessions: Vec<Session>,
    pub activity: Vec<Activity>,
    pub memory: Vec<MemoryItem>,
    pub health: BTreeMap<String, Health>,
    pub extensions: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum HostAction {
    OpenUrl(String),
    OpenProject(String),
    OpenSession {
        assistant: String,
        id: String,
        project: String,
    },
    OpenMemory(String),
    LocalControl,
}
