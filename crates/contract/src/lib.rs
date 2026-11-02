//! Environment-neutral widget models and bounded, versioned collector messages.
use serde::{Deserialize, Serialize};
use sq_core::{Config, ConfigAction, HostAction, Snapshot, WidgetInstance};
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Descriptor {
    pub kind: String,
    pub title: String,
    pub providers: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Tone {
    Muted,
    Accent,
    Warning,
    Error,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub at: i64,
    pub value: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Series {
    pub label: String,
    pub points: Vec<Point>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Node {
    Text(String),
    Notice {
        tone: Tone,
        text: String,
    },
    Meter {
        label: String,
        used: Option<f64>,
        resets_at: Option<i64>,
    },
    Chart {
        series: Vec<Series>,
    },
    Row {
        id: String,
        label: String,
        detail: String,
        action: Option<HostAction>,
        children: Vec<Node>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct View {
    pub title: String,
    pub nodes: Vec<Node>,
}

pub trait Widget: Send + Sync {
    fn descriptor(&self) -> Descriptor;
    fn view(&self, instance: &WidgetInstance, snapshot: &Snapshot, now: i64) -> View;
}
#[derive(Default)]
pub struct Registry {
    widgets: BTreeMap<String, Box<dyn Widget>>,
}
impl Registry {
    pub fn register(&mut self, widget: Box<dyn Widget>) -> Result<(), String> {
        let kind = widget.descriptor().kind;
        if kind.is_empty() || self.widgets.contains_key(&kind) {
            return Err("Widget kind is empty or already registered".into());
        }
        self.widgets.insert(kind, widget);
        Ok(())
    }
    pub fn descriptors(&self) -> Vec<Descriptor> {
        self.widgets.values().map(|w| w.descriptor()).collect()
    }
    pub fn view(&self, instance: &WidgetInstance, snapshot: &Snapshot, now: i64) -> View {
        match self.widgets.get(&instance.kind) {
            Some(widget) => widget.view(instance, snapshot, now),
            None => View { title: instance.kind.clone(), nodes: vec![Node::Notice { tone: Tone::Warning,
                text: "This host has no compatible renderer for this widget. Its settings are retained.".into() }] },
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub version: u32,
    pub body: Command,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Command {
    Snapshot { after: Option<u64> },
    Configuration,
    Configure { revision: u64, action: ConfigAction },
    Ping,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub version: u32,
    pub body: Reply,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Reply {
    Snapshot(Box<Snapshot>),
    Unchanged { revision: u64 },
    Configuration { revision: u64, config: Box<Config> },
    Pong,
    Error(String),
}
pub fn decode_request(bytes: &[u8]) -> Result<Request, String> {
    if bytes.len() > MAX_FRAME {
        return Err("Collector frame exceeds the size limit".into());
    }
    let request: Request =
        serde_json::from_slice(bytes).map_err(|_| "Malformed collector request")?;
    if request.version != PROTOCOL_VERSION {
        return Err("Incompatible collector protocol version".into());
    }
    Ok(request)
}
