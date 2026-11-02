use crate::HostAction;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CONFIG_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Dark,
    Light,
    System,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    #[default]
    Behind,
    Normal,
    Above,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Palette {
    pub background: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
    pub muted: [u8; 3],
    pub warning: [u8; 3],
    pub error: [u8; 3],
}
impl Default for Palette {
    fn default() -> Self {
        Self {
            background: [17, 24, 22],
            text: [224, 232, 226],
            accent: [125, 222, 138],
            muted: [145, 167, 155],
            warning: [230, 197, 117],
            error: [235, 124, 124],
        }
    }
}
impl Palette {
    pub fn light() -> Self {
        Self {
            background: [240, 244, 239],
            text: [25, 44, 32],
            accent: [28, 120, 65],
            muted: [81, 102, 90],
            warning: [141, 93, 15],
            error: [160, 40, 40],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Theme {
    pub style: String,
    pub mode: Mode,
    pub palette: Palette,
    pub opacity: f32,
    pub font_size: f32,
    pub compact: bool,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            style: "terminal".into(),
            mode: Mode::Dark,
            palette: Palette::default(),
            opacity: 0.85,
            font_size: 14.0,
            compact: false,
        }
    }
}
impl Theme {
    pub fn validate(&self) -> Result<(), String> {
        if self.style.is_empty()
            || self.style.len() > 100
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
            || !self.font_size.is_finite()
            || !(10.0..=32.0).contains(&self.font_size)
        {
            return Err("Theme needs a style, opacity 0–1 and font size 10–32".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WidgetInstance {
    pub id: String,
    pub kind: String,
    pub provider: String,
    pub options: BTreeMap<String, String>,
    pub height: f32,
    pub span: u8,
    /// Free-form placement in pixels inside the tray canvas. `width == 0` means the
    /// instance has no explicit size yet and follows the `span` flow.
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub width: f32,
    pub hidden: bool,
    pub detached: bool,
    pub theme: Option<Theme>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SourceConfig {
    pub claude: String,
    pub codex: String,
    pub memory: String,
    pub progress: String,
    pub project_roots: Vec<String>,
    pub poll_seconds: u64,
}
impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            claude: "~/.claude".into(),
            codex: "~/.codex".into(),
            memory: "~/dev/workflow-memory".into(),
            progress: "~/.local/share/codex-pbar/tasks".into(),
            project_roots: vec!["~/dev".into()],
            poll_seconds: 300,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Geometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub monitor: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Preset {
    pub theme: Theme,
    pub widgets: Vec<WidgetInstance>,
    pub layer: Layer,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub next_id: u64,
    pub widgets: Vec<WidgetInstance>,
    pub theme: Theme,
    pub layer: Layer,
    pub navigation: bool,
    pub autostart: bool,
    pub locked: bool,
    /// Space between widgets, in pixels; the flow layout, the stacking of new widgets and the
    /// snapping of the editor all use this one value.
    pub gap: f32,
    pub sources: SourceConfig,
    pub repositories: Vec<String>,
    pub geometry: BTreeMap<String, Geometry>,
    pub presets: BTreeMap<String, Preset>,
}
impl Default for Config {
    fn default() -> Self {
        let mut result = Self {
            version: CONFIG_VERSION,
            next_id: 1,
            widgets: vec![],
            theme: Theme::default(),
            layer: Layer::Behind,
            navigation: true,
            autostart: true,
            locked: true,
            gap: 10.0,
            sources: SourceConfig::default(),
            repositories: vec![],
            geometry: BTreeMap::new(),
            presets: BTreeMap::new(),
        };
        for (kind, provider) in [
            ("quota", "claude"),
            ("quota", "codex"),
            ("tokens", "claude"),
            ("tokens", "codex"),
            ("sessions", "both"),
            ("activity", "github"),
            ("memory", "both"),
        ] {
            result.add_widget(kind, provider);
        }
        result
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ConfigAction {
    Add { kind: String, provider: String },
    Remove(String),
    Move { id: String, index: usize },
    Widget(Box<WidgetInstance>),
    Replace(Box<Config>),
    Geometry { key: String, value: Geometry },
}

impl Config {
    pub fn widget(&self, id: &str) -> Option<&WidgetInstance> {
        self.widgets.iter().find(|w| w.id == id)
    }
    pub fn widget_mut(&mut self, id: &str) -> Option<&mut WidgetInstance> {
        self.widgets.iter_mut().find(|w| w.id == id)
    }
    pub fn add_widget(&mut self, kind: &str, provider: &str) -> String {
        let mut id = format!("widget-{}", self.next_id);
        while self.widget(&id).is_some() {
            self.next_id = self.next_id.checked_add(1).unwrap_or(1);
            id = format!("widget-{}", self.next_id);
        }
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        let wide = matches!(kind, "sessions" | "activity" | "memory");
        let free = self
            .widgets
            .iter()
            .any(|w| !w.hidden && !w.detached && w.width > 0.0);
        self.widgets.push(WidgetInstance {
            id: id.clone(),
            kind: kind.into(),
            provider: provider.into(),
            options: BTreeMap::new(),
            height: if kind == "quota" { 190.0 } else { 260.0 },
            span: if wide { 2 } else { 1 },
            x: 0.0,
            y: 0.0,
            // In a free canvas the new widget gets an explicit rectangle at the top-left corner,
            // where it is visible and can be dragged, instead of stacking below the screen.
            width: if free { 420.0 } else { 0.0 },
            hidden: false,
            detached: false,
            theme: None,
        });
        id
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != CONFIG_VERSION {
            return Err("Unsupported configuration version; original retained".into());
        }
        self.theme.validate()?;
        if !self.gap.is_finite() || !(0.0..=64.0).contains(&self.gap) {
            return Err("Space between widgets must be 0 to 64 pixels".into());
        }
        if self.widgets.len() > 128 || self.presets.len() > 32 || self.repositories.len() > 100 {
            return Err("Configuration exceeds widget/preset/repository bounds".into());
        }
        let mut ids = BTreeSet::new();
        for w in &self.widgets {
            if w.id.is_empty()
                || w.id.len() > 100
                || !ids.insert(&w.id)
                || w.kind.is_empty()
                || w.kind.len() > 100
                || w.provider.len() > 100
                || !(1..=2).contains(&w.span)
                || !w.height.is_finite()
                || !(60.0..=8_000.0).contains(&w.height)
                || !(w.width == 0.0 || (120.0..=8_000.0).contains(&w.width))
                || !w.x.is_finite()
                || !w.y.is_finite()
                || !(0.0..=20_000.0).contains(&w.x)
                || !(0.0..=20_000.0).contains(&w.y)
                || w.options.len() > 32
                || w.options
                    .iter()
                    .any(|(k, v)| k.len() > 100 || v.len() > 4_000)
            {
                return Err("Invalid or duplicate widget identity, size or options".into());
            }
            if let Some(theme) = &w.theme {
                theme.validate()?;
            }
        }
        for g in self.geometry.values() {
            if ![g.x, g.y, g.width, g.height].iter().all(|v| v.is_finite())
                || g.width < 200.0
                || g.height < 100.0
                || g.width > 20000.0
                || g.height > 20000.0
            {
                return Err("Invalid window geometry".into());
            }
        }
        if self.sources.poll_seconds < 300
            || self.sources.poll_seconds > 3_600
            || [
                &self.sources.claude,
                &self.sources.codex,
                &self.sources.memory,
                &self.sources.progress,
            ]
            .iter()
            .any(|p| p.is_empty())
        {
            return Err(
                "Sources need explicit paths and a safe 300–3600 second account polling interval"
                    .into(),
            );
        }
        for repo in &self.repositories {
            validate_repository(repo)?;
        }
        let valid_path = |p: &str| {
            p.len() <= 4096
                && (p.starts_with('/') || p == "~" || p.starts_with("~/"))
                && !p.contains('\0')
        };
        if self.sources.project_roots.len() > 100
            || self.sources.project_roots.iter().any(|p| !valid_path(p))
            || [
                &self.sources.claude,
                &self.sources.codex,
                &self.sources.memory,
                &self.sources.progress,
            ]
            .iter()
            .any(|p| !valid_path(p))
        {
            return Err(
                "Source paths must be absolute or start with ~/; at most 100 project roots".into(),
            );
        }
        for (name, preset) in &self.presets {
            if name.trim().is_empty() || name.len() > 100 {
                return Err("Invalid preset name".into());
            }
            let mut candidate = self.clone();
            candidate.presets.clear();
            candidate.theme = preset.theme.clone();
            candidate.widgets = preset.widgets.clone();
            candidate.layer = preset.layer;
            candidate.validate()?;
        }
        Ok(())
    }
    pub fn apply(&mut self, action: ConfigAction) -> Result<(), String> {
        let mut next = self.clone();
        match action {
            ConfigAction::Replace(config) => next = *config,
            ConfigAction::Add { kind, provider } => {
                next.add_widget(&kind, &provider);
            }
            ConfigAction::Remove(id) => {
                let i = next
                    .widgets
                    .iter()
                    .position(|w| w.id == id)
                    .ok_or("Widget not found")?;
                next.widgets.remove(i);
            }
            ConfigAction::Move { id, index } => {
                if index >= next.widgets.len() {
                    return Err("Invalid widget destination".into());
                }
                let old = next
                    .widgets
                    .iter()
                    .position(|w| w.id == id)
                    .ok_or("Widget not found")?;
                let widget = next.widgets.remove(old);
                next.widgets.insert(index, widget);
            }
            ConfigAction::Geometry { key, value } => {
                if key.is_empty() || key.len() > 100 {
                    return Err("Invalid geometry key".into());
                }
                next.geometry.insert(key, value);
            }
            ConfigAction::Widget(widget) => {
                let id = widget.id.clone();
                *next.widget_mut(&id).ok_or("Widget not found")? = *widget;
            }
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn allows(&self, action: &HostAction) -> bool {
        matches!(action, HostAction::LocalControl) || self.navigation
    }
}
pub fn validate_repository(repo: &str) -> Result<(), String> {
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || p.len() > 100
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    {
        return Err("Repository must be owner/name, without a command or URL".into());
    }
    Ok(())
}
