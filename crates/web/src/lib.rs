//! Browser host: the same Rust tray with synthetic data, no local data transport.
use sq_contract::{Descriptor, Node, View, Widget};
use sq_core::*;
use sq_tray::{Intent, Tray};

pub const SAMPLE_NOW: i64 = 1_793_577_600;
struct BuildStatus;
impl Widget for BuildStatus {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            kind: "sample-build".into(),
            title: "Example build status".into(),
            providers: vec!["demo".into()],
        }
    }
    fn view(&self, _: &WidgetInstance, _: &Snapshot, _: i64) -> View {
        View {
            title: "EXAMPLE / BUILD STATUS".into(),
            nodes: vec![
                Node::Text("An unrelated widget registered by the browser host.".into()),
                Node::Text("Build #42 · passed · synthetic demonstration".into()),
            ],
        }
    }
}

pub fn demo_tray() -> Tray {
    let mut config = Config {
        navigation: false,
        theme: Theme {
            opacity: 1.0,
            ..Theme::default()
        },
        ..Config::default()
    };
    config.add_widget("sample-build", "demo");
    let mut tray = Tray::new(config);
    tray.registry
        .register(Box::new(BuildStatus))
        .expect("unique demo widget");
    tray.connected = true;
    tray.capabilities = "Browser sample: synthetic data only. Desktop stacking, startup and local source paths are not applied. Settings last for this page session.".into();
    tray.snapshot = sample_snapshot();
    tray
}

pub fn sample_snapshot() -> Snapshot {
    let mut snapshot = Snapshot {
        revision: 1,
        ..Snapshot::default()
    };
    for (provider, five, week) in [("claude", 24.0, 61.0), ("codex", 39.0, 73.0)] {
        snapshot.usage.insert(
            provider.into(),
            Usage {
                windows: vec![
                    QuotaWindow {
                        minutes: 300,
                        used: five,
                        resets_at: Some(SAMPLE_NOW + 7200),
                        label: "Five hour".into(),
                    },
                    QuotaWindow {
                        minutes: 10080,
                        used: week,
                        resets_at: Some(SAMPLE_NOW + 172800),
                        label: "Weekly".into(),
                    },
                ],
                updated_at: Some(SAMPLE_NOW),
            },
        );
        snapshot.tokens.insert(
            provider.into(),
            (0..24)
                .map(|hour| Bucket {
                    at: SAMPLE_NOW - (23 - hour) * 3600,
                    input: (1000 + hour * 130) as u64,
                    output: (300 + hour % 5 * 190) as u64,
                    cache_read: if hour == 8 {
                        None
                    } else {
                        Some((hour * 70) as u64)
                    },
                    cache_write: Some((hour * 40) as u64),
                })
                .collect(),
        );
        for source in [provider.to_string(), format!("{provider}-logs")] {
            snapshot.health.insert(
                source,
                Health {
                    state: SourceState::Fresh,
                    last_success: Some(SAMPLE_NOW),
                    message: "Synthetic browser demonstration".into(),
                    ..Health::default()
                },
            );
        }
    }
    snapshot.sessions = vec![
        Session {
            id: "sample-main".into(),
            assistant: "claude".into(),
            project: "Status Quo demo".into(),
            task: "Demonstrate reusable widgets".into(),
            state: SessionState::Running,
            updated_at: SAMPLE_NOW,
            children_supported: true,
            ..Session::default()
        },
        Session {
            id: "sample-child".into(),
            assistant: "claude".into(),
            project: "Status Quo demo".into(),
            task: "Validate browser host".into(),
            state: SessionState::Waiting,
            parent: Some("sample-main".into()),
            updated_at: SAMPLE_NOW,
            ..Session::default()
        },
        Session {
            id: "sample-codex".into(),
            assistant: "codex".into(),
            project: "Example project".into(),
            task: "Waiting for review".into(),
            state: SessionState::Waiting,
            updated_at: SAMPLE_NOW,
            ..Session::default()
        },
    ];
    snapshot.activity.push(Activity {
        account: "github".into(),
        id: "sample-commit".into(),
        repository: "example/status-quo-demo".into(),
        kind: "commit".into(),
        title: "Example: add a reusable widget".into(),
        at: SAMPLE_NOW - 300,
        url: String::new(),
    });
    snapshot.memory.push(MemoryItem {
        id: "sample-memory".into(),
        instance: "codex".into(),
        kind: "decision".into(),
        key: "demo.modularity".into(),
        body: "Synthetic example: widgets are interchangeable between hosts.".into(),
        at: SAMPLE_NOW - 600,
        ..MemoryItem::default()
    });
    for source in ["sessions", "github", "memory"] {
        snapshot.health.insert(
            source.into(),
            Health {
                state: SourceState::Fresh,
                last_success: Some(SAMPLE_NOW),
                message: "Synthetic browser demonstration".into(),
                ..Health::default()
            },
        );
    }
    snapshot
}

pub struct BrowserApp {
    pub tray: Tray,
}
impl Default for BrowserApp {
    fn default() -> Self {
        Self { tray: demo_tray() }
    }
}
impl BrowserApp {
    pub fn apply(&mut self, intents: Vec<Intent>) {
        for intent in intents {
            match intent {
                Intent::Configure(action) => {
                    let saved = matches!(action, ConfigAction::Replace(_));
                    let mut config = self.tray.config.clone();
                    match config.apply(action) {
                        Ok(()) => {
                            if saved {
                                self.tray.accept(config);
                            } else {
                                self.tray.config = config;
                            }
                        }
                        Err(error) => self.tray.error = Some(error),
                    }
                }
                Intent::Navigate(_) => {
                    self.tray.error = Some(
                        "Source navigation is unavailable in this synthetic browser sample.".into(),
                    )
                }
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
impl eframe::App for BrowserApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _: &mut eframe::Frame) {
        eframe::egui::TopBottomPanel::bottom("sample-notice").show(ctx, |ui| {
            ui.label("SYNTHETIC DEMO · no desktop data · settings reset on reload");
        });
        let intents = self.tray.show(ctx, SAMPLE_NOW);
        self.apply(intents);
        let intents = self.tray.detached_windows(ctx, SAMPLE_NOW);
        self.apply(intents);
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;
    let document = web_sys::window()
        .ok_or("No browser window")?
        .document()
        .ok_or("No document")?;
    let canvas = document
        .get_element_by_id("status-quo")
        .ok_or("Missing Status Quo canvas")?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_| Ok(Box::<BrowserApp>::default())),
            )
            .await
        {
            if let Some(element) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("loading"))
            {
                element.set_text_content(Some(&format!(
                    "Could not start the browser sample: {error:?}"
                )));
            }
        } else if let Some(element) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("loading"))
        {
            element.remove();
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // REQ SQ-20, SQ-3, SQ-16
    #[test]
    fn browser_host_keeps_draft_local_until_save_and_rejects_invalid_settings() {
        let mut app = BrowserApp::default();
        let original = app.tray.config.clone();
        app.tray.begin_customization();
        app.tray.draft.as_mut().unwrap().theme.font_size = 22.0;
        assert_eq!(app.tray.config, original);
        app.tray.cancel_customization();
        assert_eq!(app.tray.config, original);
        app.tray.begin_customization();
        app.tray.draft.as_mut().unwrap().theme.font_size = 20.0;
        let save = app.tray.save_customization().unwrap();
        app.apply(vec![Intent::Configure(save)]);
        assert_eq!(app.tray.config.theme.font_size, 20.0);
        assert!(app.tray.draft.is_none());
        let saved = app.tray.config.clone();
        let mut invalid = saved.clone();
        invalid.theme.font_size = 200.0;
        app.apply(vec![Intent::Configure(ConfigAction::Replace(Box::new(
            invalid,
        )))]);
        assert_eq!(app.tray.config, saved);
        assert!(app.tray.error.is_some());
    }
    // REQ SQ-20, SQ-24, SQ-21
    #[test]
    fn browser_never_dispatches_source_navigation() {
        let mut app = BrowserApp::default();
        app.tray.config.navigation = true;
        app.apply(vec![Intent::Navigate(HostAction::OpenMemory(
            "/private/fixture".into(),
        ))]);
        assert!(app.tray.error.as_ref().unwrap().contains("unavailable"));
        assert!(app.tray.snapshot.extensions.is_empty());
        assert!(app
            .tray
            .snapshot
            .memory
            .iter()
            .all(|item| item.path.is_empty()));
    }
    // REQ SQ-20, SQ-14, SQ-1
    #[test]
    fn browser_registers_unrelated_widget_and_uses_shared_tray_actions() {
        let mut app = BrowserApp::default();
        let extra = app.tray.config.widgets.last().unwrap().clone();
        assert_eq!(
            app.tray
                .registry
                .view(&extra, &app.tray.snapshot, SAMPLE_NOW)
                .title,
            "EXAMPLE / BUILD STATUS"
        );
        app.apply(vec![Intent::Configure(ConfigAction::Remove(
            extra.id.clone(),
        ))]);
        assert!(app.tray.config.widget(&extra.id).is_none());
        assert_eq!(
            app.tray.snapshot.usage["codex"].window(300).unwrap().used,
            39.0
        );
        assert!(!app.tray.config.navigation);
    }
}
