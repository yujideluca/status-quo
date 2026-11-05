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

impl Harness {
    fn drag(&mut self, tray: &mut Tray, from: egui::Pos2, to: egui::Pos2) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(tray, vec![egui::Event::PointerMoved(from)]);
        self.frame(tray, vec![button(from, true)]);
        let mid = from.lerp(to, 0.5);
        self.frame(tray, vec![egui::Event::PointerMoved(mid)]);
        self.frame(tray, vec![egui::Event::PointerMoved(to)]);
        self.frame(tray, vec![egui::Event::PointerMoved(to)]);
        self.frame(tray, vec![button(to, false)]);
        self.frame(tray, vec![]);
    }
    fn cell(&self, tray: &Tray, id: &str) -> egui::Rect {
        tray.edit_cells
            .iter()
            .find(|(i, _)| i == id)
            .unwrap_or_else(|| panic!("{id} not drawn in edit mode"))
            .1
    }
}

fn unlocked() -> Config {
    Config {
        locked: false,
        ..Config::default()
    }
}

fn editing_tray(ui: &mut Harness) -> Tray {
    let mut tray = Tray::new(unlocked());
    ui.click(&mut tray, "Edit tray");
    assert!(tray.editing && tray.draft.is_some());
    ui.frame(&mut tray, vec![]);
    tray
}

// REQ SQ-16 SQ-29
#[test]
fn entering_edit_mode_freezes_the_current_layout_without_moving_anything() {
    let mut ui = Harness::new();
    let mut tray = Tray::new(unlocked());
    ui.frame(&mut tray, vec![]);
    ui.frame(&mut tray, vec![]);
    let before: Vec<_> = tray.flow_rects();
    ui.click(&mut tray, "Edit tray");
    let draft = tray.draft.clone().unwrap();
    assert!(draft.widgets.iter().all(|w| w.width > 0.0));
    ui.frame(&mut tray, vec![]);
    for (id, rect) in &before {
        let w = draft.widget(id).unwrap();
        assert!((w.width - rect.width()).abs() < 1.0, "{id} width");
        assert!((w.height - rect.height()).abs() < 1.0, "{id} height");
    }
    assert_eq!(tray.config, unlocked());
}

// REQ SQ-16 SQ-29
#[test]
fn a_corner_resizes_both_axes_freely_and_cancel_discards() {
    let mut ui = Harness::new();
    let mut tray = editing_tray(&mut ui);
    let id = tray.config.widgets[0].id.clone();
    let rect = ui.cell(&tray, &id);
    let from = rect.right_bottom() - egui::vec2(4.0, 4.0);
    let w0 = tray.draft.as_ref().unwrap().widget(&id).unwrap().clone();
    ui.drag(&mut tray, from, from + egui::vec2(700.0, 300.0));
    let w1 = tray.draft.as_ref().unwrap().widget(&id).unwrap();
    assert!(
        w1.width > w0.width + 600.0,
        "width {} -> {}",
        w0.width,
        w1.width
    );
    assert!(
        w1.height > w0.height + 250.0,
        "height {} -> {}",
        w0.height,
        w1.height
    );
    assert_eq!(tray.config.widgets[0].width, 0.0, "active config untouched");
    ui.click(&mut tray, "Cancel");
    assert!(!tray.editing && tray.draft.is_none());
    assert_eq!(tray.config.widgets[0].width, 0.0);
}

// REQ SQ-16 SQ-29
#[test]
fn left_and_top_borders_resize_and_shift_the_widget() {
    let mut ui = Harness::new();
    let mut tray = editing_tray(&mut ui);
    let id = tray.config.widgets[1].id.clone();
    let w0 = tray.draft.as_ref().unwrap().widget(&id).unwrap().clone();
    let rect = ui.cell(&tray, &id);
    let from = egui::pos2(rect.left() + 3.0, rect.center().y);
    ui.drag(&mut tray, from, from - egui::vec2(120.0, 0.0));
    let w1 = tray.draft.as_ref().unwrap().widget(&id).unwrap().clone();
    assert!(
        w1.x < w0.x - 90.0 && w1.width > w0.width + 90.0,
        "left edge"
    );
    let rect = ui.cell(&tray, &id);
    let from = egui::pos2(rect.center().x, rect.top() + 3.0);
    ui.drag(&mut tray, from, from + egui::vec2(0.0, 40.0));
    let w2 = tray.draft.as_ref().unwrap().widget(&id).unwrap();
    assert!(
        w2.y > w1.y + 20.0 && w2.height < w1.height - 20.0,
        "top edge"
    );
}

// REQ SQ-16 SQ-29
#[test]
fn dragging_the_body_moves_anywhere_and_done_saves_a_normalized_canvas() {
    let mut ui = Harness::new();
    let mut tray = editing_tray(&mut ui);
    let original = tray.config.clone();
    let id = original.widgets[0].id.clone();
    // Off-centre: the centre of a widget holds its Remove button.
    let cell = ui.cell(&tray, &id);
    let from = cell.min + cell.size() * egui::vec2(0.25, 0.2);
    ui.drag(&mut tray, from, from + egui::vec2(400.0, 250.0));
    let moved = tray.draft.as_ref().unwrap().widget(&id).unwrap().clone();
    assert!(
        moved.x > 350.0 && moved.y > 200.0,
        "moved to {},{}",
        moved.x,
        moved.y
    );
    assert_eq!(tray.config, original);
    let mut intents = ui.click(&mut tray, "Done");
    let Intent::Configure(action) = intents.remove(0) else {
        panic!("Expected configuration submission")
    };
    let mut accepted = original;
    accepted.apply(action).unwrap();
    let min_x = accepted
        .widgets
        .iter()
        .map(|w| w.x)
        .fold(f32::MAX, f32::min);
    let min_y = accepted
        .widgets
        .iter()
        .map(|w| w.y)
        .fold(f32::MAX, f32::min);
    assert_eq!((min_x, min_y), (0.0, 0.0), "canvas starts at its corner");
    tray.accept(accepted);
    assert!(!tray.editing);
    let size = tray.desired_size().expect("a free canvas wants a size");
    let rects = sq_tray::free_rects(&tray.config.widgets, tray.config.gap);
    let canvas = sq_tray::canvas_size(&rects);
    assert_eq!(size.x, canvas.x.max(330.0));
    assert!(size.y > canvas.y, "header is added to the canvas height");
}

// REQ SQ-29
#[test]
fn the_tray_size_follows_the_widgets_up_to_the_whole_desktop() {
    let mut config = Config::default();
    config.widgets.truncate(1);
    config.widgets[0].width = 400.0;
    config.widgets[0].height = 200.0;
    let mut tray = Tray::new(config);
    let small = tray.desired_size().unwrap();
    tray.config.widgets[0].width = 3_800.0;
    tray.config.widgets[0].height = 2_000.0;
    tray.config.validate().unwrap();
    let large = tray.desired_size().unwrap();
    assert!(large.x >= 3_800.0 && large.y >= 2_000.0);
    assert!(small.x < 1_000.0);
}

// REQ SQ-16 SQ-29
#[test]
fn a_button_on_the_top_widget_still_works_where_widgets_overlap() {
    let mut config = Config::default();
    config.widgets.truncate(3);
    // widget-1 and widget-2 are account meters; widget-3 is tokens. Put the tokens widget on
    // top of the first meter so its range buttons lie inside the meter's rectangle.
    for (i, (x, y, w, h)) in [
        (0.0, 0.0, 700.0, 400.0),
        (720.0, 0.0, 250.0, 200.0),
        (60.0, 40.0, 500.0, 300.0),
    ]
    .into_iter()
    .enumerate()
    {
        let widget = &mut config.widgets[i];
        widget.x = x;
        widget.y = y;
        widget.width = w;
        widget.height = h;
    }
    assert_eq!(config.widgets[2].kind, "tokens");
    let mut tray = Tray::new(config);
    let mut ui = Harness::new();
    let mut intents = ui.click(&mut tray, "7d");
    assert_eq!(
        intents.len(),
        1,
        "the tokens widget on top must receive the click"
    );
    let Intent::Configure(ConfigAction::Widget(widget)) = intents.remove(0) else {
        panic!("Expected a widget change")
    };
    assert_eq!(widget.id, "widget-3");
    assert_eq!(widget.options.get("range").map(String::as_str), Some("7d"));
}

// REQ SQ-16 SQ-29
#[test]
fn a_locked_layout_cannot_be_edited_and_unlocking_allows_it() {
    let config = Config {
        locked: true,
        ..Config::default()
    };
    let mut tray = Tray::new(config);
    let mut ui = Harness::new();
    ui.frame(&mut tray, vec![]);
    ui.click(&mut tray, "Edit tray");
    assert!(
        !tray.editing && tray.draft.is_none(),
        "a locked layout must not enter edit mode"
    );
    tray.config.locked = false;
    ui.click(&mut tray, "Edit tray");
    assert!(tray.editing);
}

// REQ SQ-16 SQ-29
#[test]
fn editing_a_tall_scrolling_layout_freezes_it_inside_the_space_the_tray_had() {
    let mut config = unlocked();
    for w in &mut config.widgets {
        w.height = 700.0;
    }
    let mut tray = Tray::new(config);
    let mut ui = Harness::new();
    ui.frame(&mut tray, vec![]);
    ui.frame(&mut tray, vec![]);
    ui.click(&mut tray, "Edit tray");
    let draft = tray.draft.as_ref().unwrap();
    let rects = free_rects(&draft.widgets, draft.gap);
    let bottom = canvas_size(&rects).y;
    assert!(
        bottom <= 1000.0,
        "frozen canvas is {bottom} px tall in a 1000 px tray"
    );
    assert!(draft.widgets.iter().all(|w| w.height >= 60.0));
}

fn two_widgets(gap: f32) -> Config {
    let mut config = unlocked();
    config.widgets.truncate(2);
    config.gap = gap;
    for (i, x) in [0.0, 500.0].into_iter().enumerate() {
        let w = &mut config.widgets[i];
        w.x = x;
        w.y = 0.0;
        w.width = 300.0;
        w.height = 200.0;
    }
    config
}

// REQ SQ-29
#[test]
fn a_dragged_border_sticks_at_the_configured_gap_and_lets_go_when_pulled_on() {
    let mut ui = Harness::new();
    let mut tray = Tray::new(two_widgets(12.0));
    ui.click(&mut tray, "Edit tray");
    ui.frame(&mut tray, vec![]);
    let (a, b) = ("widget-1", "widget-2");
    let rect = ui.cell(&tray, a);
    let from = egui::pos2(rect.right() - 3.0, rect.center().y);
    // Pointer ends 6 px short of the neighbour's left border minus the gap.
    let target = ui.cell(&tray, b).left() - 12.0 - 6.0 + 3.0;
    ui.drag(&mut tray, from, egui::pos2(target, from.y));
    let draft = tray.draft.clone().unwrap();
    let (wa, wb) = (draft.widget(a).unwrap(), draft.widget(b).unwrap());
    assert_eq!(wb.x - (wa.x + wa.width), 12.0, "gap between the two blocks");
    let wb = wb.clone();
    // Pulled well past the snap distance the widget lets go.
    let rect = ui.cell(&tray, a);
    let from = egui::pos2(rect.right() - 3.0, rect.center().y);
    ui.drag(&mut tray, from, egui::pos2(from.x - 60.0, from.y));
    let wa = tray.draft.as_ref().unwrap().widget(a).unwrap();
    assert!(wb.x - (wa.x + wa.width) > 40.0, "no longer stuck");
}

// REQ SQ-29
#[test]
fn a_dragged_size_sticks_to_another_widgets_size() {
    let mut ui = Harness::new();
    let mut config = two_widgets(10.0);
    config.widgets[1].width = 360.0;
    config.widgets[1].height = 260.0;
    let mut tray = Tray::new(config);
    ui.click(&mut tray, "Edit tray");
    ui.frame(&mut tray, vec![]);
    let rect = ui.cell(&tray, "widget-1");
    // Bottom-right corner dragged so the size lands 7 px short of widget-2's 360 x 260.
    let from = rect.right_bottom() - egui::vec2(4.0, 4.0);
    ui.drag(&mut tray, from, from + egui::vec2(60.0 - 7.0, 60.0 - 7.0));
    let w = tray.draft.as_ref().unwrap().widget("widget-1").unwrap();
    assert_eq!((w.width, w.height), (360.0, 260.0));
}

// REQ SQ-3 SQ-29
#[test]
fn the_space_between_widgets_is_customizable_and_validated() {
    let mut config = Config {
        gap: 24.0,
        ..unlocked()
    };
    config.validate().unwrap();
    config.gap = 200.0;
    assert!(config.validate().is_err());
    config.gap = f32::NAN;
    assert!(config.validate().is_err());
    let mut tray = Tray::new(Config {
        gap: 24.0,
        ..unlocked()
    });
    let mut ui = Harness::new();
    ui.click(&mut tray, "Customize");
    ui.frame(&mut tray, vec![]);
    let (_, output) = ui.frame(&mut tray, vec![]);
    fn has(shape: &egui::Shape, text: &str) -> bool {
        match shape {
            egui::Shape::Text(t) => t.galley.text().contains(text),
            egui::Shape::Vec(v) => v.iter().any(|s| has(s, text)),
            _ => false,
        }
    }
    assert!(
        output
            .shapes
            .iter()
            .any(|s| has(&s.shape, "Space between widgets")),
        "the customization page offers the setting"
    );
    // Flow layout and stacking use the configured gap.
    let mut widgets = tray.config.widgets.clone();
    widgets.truncate(2);
    let rects = free_rects(&widgets, 24.0);
    assert_eq!(rects[1].1.top() - rects[0].1.bottom(), 24.0);
}

// REQ SQ-16 SQ-29
#[test]
fn a_remove_button_in_the_widget_centre_removes_it_from_the_draft_and_cancel_restores_it() {
    let mut ui = Harness::new();
    let mut tray = editing_tray(&mut ui);
    let before = tray.draft.as_ref().unwrap().widgets.len();
    assert!(before >= 2, "the default tray has several widgets");
    let id = tray.config.widgets[0].id.clone();
    let cell = ui.cell(&tray, &id);
    let pos = cell.center();
    // Pointer on the centre: the button, not the body drag, must receive the click.
    ui.frame(&mut tray, vec![egui::Event::PointerMoved(pos)]);
    let click = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    ui.frame(&mut tray, vec![click(true)]);
    ui.frame(&mut tray, vec![click(false)]);
    ui.frame(&mut tray, vec![]);
    let draft = tray.draft.as_ref().unwrap();
    assert_eq!(draft.widgets.len(), before - 1);
    assert!(draft.widget(&id).is_none(), "the clicked widget is gone");
    assert_eq!(tray.config.widgets.len(), before, "active config untouched");
    ui.click(&mut tray, "Cancel");
    assert_eq!(tray.config.widgets.len(), before);
    assert!(tray.config.widget(&id).is_some());
}

// REQ SQ-16 SQ-29
#[test]
fn removing_a_widget_and_pressing_done_saves_the_smaller_tray() {
    let mut ui = Harness::new();
    let mut tray = editing_tray(&mut ui);
    let before = tray.config.widgets.len();
    let id = tray.config.widgets[0].id.clone();
    let pos = ui.cell(&tray, &id).center();
    ui.frame(&mut tray, vec![egui::Event::PointerMoved(pos)]);
    let click = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    ui.frame(&mut tray, vec![click(true)]);
    ui.frame(&mut tray, vec![click(false)]);
    let mut intents = ui.click(&mut tray, "Done");
    let Intent::Configure(action) = intents.remove(0) else {
        panic!("Expected configuration submission")
    };
    let mut accepted = tray.config.clone();
    accepted.apply(action).unwrap();
    assert_eq!(accepted.widgets.len(), before - 1);
    assert!(accepted.widget(&id).is_none());
}

// REQ SQ-16 SQ-29
#[test]
fn a_full_desktop_editor_places_widgets_left_of_and_above_the_docked_tray() {
    let mut ui = Harness::new();
    let mut tray = Tray::new(unlocked());
    tray.fullscreen_edit = true;
    ui.click(&mut tray, "Edit tray");
    // The host reports the docked corner only after the editor has started.
    tray.set_origin(egui::vec2(300.0, 200.0));
    ui.frame(&mut tray, vec![]);
    let id = tray.config.widgets[0].id.clone();
    let cell = ui.cell(&tray, &id);
    let from = cell.min + cell.size() * egui::vec2(0.25, 0.2);
    ui.drag(&mut tray, from, from - egui::vec2(250.0, 100.0));
    let moved = tray.draft.as_ref().unwrap().widget(&id).unwrap().clone();
    assert!(
        moved.x < 300.0 && moved.x >= 0.0,
        "x {} is left of the docked tray but on the desktop",
        moved.x
    );
    let mut intents = ui.click(&mut tray, "Done");
    let Intent::Configure(action) = intents.remove(0) else {
        panic!("Expected configuration submission")
    };
    let mut accepted = tray.config.clone();
    accepted.apply(action).unwrap();
    let g = &accepted.geometry["tray"];
    assert!(
        g.x < 300.0,
        "the tray is saved where the widget was left: {}",
        g.x
    );
}
