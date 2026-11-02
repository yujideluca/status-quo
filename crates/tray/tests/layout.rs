use sq_core::*;
use sq_tray::*;
// REQ SQ-1 SQ-16
#[test]
fn layout_keeps_identity_when_removed_resized_or_detached() {
    let mut config = Config::default();
    config.widgets[0].hidden = true;
    config.widgets[1].detached = true;
    let wide = layout(&config.widgets, 800.0);
    assert_eq!(wide[0].ids, vec!["widget-3", "widget-4"]);
    let narrow = layout(&config.widgets, 300.0);
    assert_eq!(narrow.len(), 5);
    assert!(narrow.iter().all(|row| row.ids.len() == 1));
    config.widgets.clear();
    assert!(layout(&config.widgets, 800.0).is_empty());
}
// REQ SQ-3 SQ-16 SQ-21
#[test]
fn customization_cancel_and_invalid_import_preserve_active_configuration() {
    let mut tray = Tray::new(Config::default());
    let original = tray.config.clone();
    tray.begin_customization();
    tray.draft.as_mut().unwrap().widgets.clear();
    tray.cancel_customization();
    assert_eq!(tray.config, original);
    tray.begin_customization();
    assert!(tray.import_draft("{broken").is_err());
    assert_eq!(tray.draft.as_ref().unwrap(), &original);
    tray.draft.as_mut().unwrap().theme.opacity = 0.0;
    let pending = tray.save_customization().unwrap();
    assert!(matches!(pending, ConfigAction::Replace(_)));
    assert_eq!(tray.config, original);
}

// REQ SQ-3
#[test]
fn system_theme_resolves_defaults_and_preserves_custom_colors() {
    let mut theme = Theme {
        mode: Mode::System,
        ..Theme::default()
    };
    assert_eq!(
        sq_tray::terminal(&theme, false).override_text_color,
        Some(egui::Color32::from_rgb(25, 44, 32))
    );
    theme.palette.text = [1, 2, 3];
    assert_eq!(
        sq_tray::terminal(&theme, false).override_text_color,
        Some(egui::Color32::from_rgb(1, 2, 3))
    );
}

struct Harness {
    ctx: egui::Context,
    now: f64,
}
impl Harness {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            now: 0.0,
        }
    }
    fn frame(
        &mut self,
        tray: &mut Tray,
        events: Vec<egui::Event>,
    ) -> (Vec<Intent>, egui::FullOutput) {
        self.now += 0.02;
        let mut intents = vec![];
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 1000.0),
                )),
                time: Some(self.now),
                events,
                ..Default::default()
            },
            |ctx| {
                intents = tray.show(ctx, 0);
            },
        );
        (intents, output)
    }
    fn click(&mut self, tray: &mut Tray, text: &str) -> Vec<Intent> {
        self.frame(tray, vec![]);
        let (_, output) = self.frame(tray, vec![]);
        fn find(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.pos + t.galley.rect.center().to_vec2())
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        let pos = output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, text))
            .unwrap_or_else(|| panic!("Control {text} not rendered"));
        self.frame(
            tray,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        self.frame(
            tray,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
        .0
    }
}
// REQ SQ-3 SQ-16
#[test]
fn real_pointer_events_cancel_and_save_a_draft_without_prematurely_changing_active_state() {
    let mut tray = Tray::new(Config::default());
    let original = tray.config.clone();
    let mut ui = Harness::new();
    ui.click(&mut tray, "Customize");
    assert!(tray.draft.is_some());
    tray.draft.as_mut().unwrap().theme.opacity = 0.25;
    ui.click(&mut tray, "Cancel");
    assert!(tray.draft.is_none());
    assert_eq!(tray.config, original);
    ui.click(&mut tray, "Customize");
    let draft = tray.draft.as_mut().unwrap();
    let first = draft.widgets[0].id.clone();
    draft
        .apply(ConfigAction::Move {
            id: first.clone(),
            index: 3,
        })
        .unwrap();
    let mut detached = draft.widget(&first).unwrap().clone();
    detached.detached = true;
    draft
        .apply(ConfigAction::Widget(Box::new(detached)))
        .unwrap();
    let removed = draft.widgets[0].id.clone();
    draft.apply(ConfigAction::Remove(removed.clone())).unwrap();
    let added = draft.add_widget("tokens", "claude");
    let mut intents = ui.click(&mut tray, "Save changes");
    assert_eq!(intents.len(), 1);
    assert_eq!(tray.config, original);
    let Intent::Configure(action) = intents.remove(0) else {
        panic!("Expected configuration submission")
    };
    let mut accepted = original;
    accepted.apply(action).unwrap();
    tray.accept(accepted);
    assert!(tray.draft.is_none());
    assert!(tray.config.widget(&first).unwrap().detached);
    assert!(tray.config.widget(&removed).is_none());
    assert!(tray.config.widget(&added).is_some());
    assert!(!layout(&tray.config.widgets, 1000.0)
        .iter()
        .any(|r| r.ids.contains(&first)));
}
// REQ SQ-1 SQ-3 SQ-16
#[test]
fn empty_tray_add_control_still_works_with_navigation_disabled() {
    let mut config = Config::default();
    config.widgets.clear();
    config.navigation = false;
    let mut tray = Tray::new(config);
    let mut ui = Harness::new();
    ui.click(&mut tray, "+ Add widget");
    let mut intents = ui.click(&mut tray, "claude / Account usage");
    assert_eq!(intents.len(), 1);
    let Intent::Configure(action) = intents.remove(0) else {
        panic!("Expected add action")
    };
    tray.config.apply(action).unwrap();
    assert_eq!(tray.config.widgets.len(), 1);
    assert_eq!(tray.config.widgets[0].provider, "claude");
    assert!(!tray.config.navigation);
}
// REQ SQ-3 SQ-16
#[test]
fn rendered_widget_uses_its_own_font_and_does_not_change_sibling_font() {
    let mut config = Config::default();
    config.widgets.truncate(2);
    config.widgets[0].theme = Some(Theme {
        font_size: 23.0,
        compact: true,
        ..Theme::default()
    });
    let mut tray = Tray::new(config);
    let mut ui = Harness::new();
    ui.frame(&mut tray, vec![]);
    let (_, output) = ui.frame(&mut tray, vec![]);
    let font_size = |label: &str| {
        output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(t.galley.job.sections[0].format.font_id.size)
            }
            _ => None,
        })
    };
    assert_eq!(font_size("claude / Account usage"), Some(23.0));
    assert_eq!(font_size("codex / Account usage"), Some(14.0));
}
