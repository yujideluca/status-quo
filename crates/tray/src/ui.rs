use crate::layout;
use egui::{Color32, RichText, Vec2};
use sq_contract::*;
use sq_core::*;
use std::collections::BTreeMap;

pub enum Intent {
    Configure(ConfigAction),
    Navigate(HostAction),
}
pub type StyleFactory = fn(&Theme, bool) -> egui::Visuals;
pub struct Tray {
    pub config: Config,
    pub draft: Option<Config>,
    pub snapshot: Snapshot,
    pub error: Option<String>,
    pub capabilities: String,
    pub connected: bool,
    /// Desktop-widget look: no tray background or title, only the widgets and two buttons.
    pub bare: bool,
    pub registry: Registry,
    /// Edit-tray mode: the draft layout is edited by dragging borders and widgets.
    pub editing: bool,
    /// Screen rectangle of each widget as drawn in the last edit-mode frame.
    pub edit_cells: Vec<(String, egui::Rect)>,
    /// Flow-layout rectangles of the last normal frame, relative to the content corner.
    flow_cells: Vec<(String, egui::Rect)>,
    /// The host shows the editor over the whole desktop, so widgets can be dragged anywhere.
    pub fullscreen_edit: bool,
    /// Screen position of the tray surface's top-left corner, set by a host that knows it.
    pub origin: egui::Vec2,
    /// The draft has been moved into desktop coordinates for this editing session.
    origin_applied: bool,
    /// Unsnapped rectangle of the border or widget being dragged, so a snap can be left again.
    drag_raw: Option<(String, layout::Edges, egui::Rect)>,
    /// Height available to the flow layout in the last normal frame.
    flow_space: f32,
    /// Height of the header strip in the last frame.
    pub header_height: f32,
    /// Size of the free canvas in the last frame.
    canvas: Vec2,
    styles: BTreeMap<String, StyleFactory>,
    applied: Option<(Theme, bool)>,
    import: String,
    preset_name: String,
}
impl Tray {
    pub fn new(config: Config) -> Self {
        let mut styles = BTreeMap::new();
        styles.insert("terminal".into(), terminal as StyleFactory);
        Self {
            config,
            draft: None,
            snapshot: Snapshot::default(),
            error: None,
            capabilities: String::new(),
            connected: false,
            bare: false,
            registry: sq_widgets::registry(),
            editing: false,
            edit_cells: Vec::new(),
            flow_cells: Vec::new(),
            fullscreen_edit: false,
            origin_applied: false,
            origin: Vec2::ZERO,
            drag_raw: None,
            flow_space: 0.0,
            header_height: 34.0,
            canvas: Vec2::ZERO,
            styles,
            applied: None,
            import: String::new(),
            preset_name: String::new(),
        }
    }
    pub fn register_style(&mut self, name: &str, factory: StyleFactory) -> Result<(), String> {
        if name.is_empty() || self.styles.contains_key(name) {
            return Err("Style name empty or already registered".into());
        }
        self.styles.insert(name.into(), factory);
        Ok(())
    }
    pub fn begin_customization(&mut self) {
        self.draft = Some(self.config.clone());
        self.editing = false;
    }
    /// Start the direct-manipulation editor on a copy of the active layout.
    pub fn begin_edit(&mut self) {
        let mut draft = self.config.clone();
        if !layout::is_free(&draft.widgets) {
            // Freeze what is on screen into explicit rectangles, so nothing jumps.
            let rects = if self.flow_cells.is_empty() {
                layout::free_rects(&draft.widgets, draft.gap)
            } else {
                self.flow_cells.clone()
            };
            let rects = fit_to_space(rects, self.flow_space);
            layout::freeze(&mut draft.widgets, &rects);
        }
        self.origin_applied = false;
        self.draft = Some(draft);
        self.editing = true;
    }
    /// The host reports where the docked tray sits on the desktop, just before its surface
    /// covers the whole output for editing (after `begin_edit`). Widget positions of the draft
    /// then become desktop positions, so a widget can go left of or above the docked tray.
    pub fn set_origin(&mut self, origin: Vec2) {
        self.origin = origin;
        if !self.editing || !self.fullscreen_edit || self.origin_applied {
            return;
        }
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        let rects: Vec<_> = layout::free_rects(&draft.widgets, draft.gap)
            .into_iter()
            .map(|(id, r)| (id, r.translate(origin)))
            .collect();
        layout::freeze(&mut draft.widgets, &rects);
        self.origin_applied = true;
    }
    /// Size the tray wants its surface to have, once it is a free canvas.
    pub fn desired_size(&self) -> Option<Vec2> {
        if self.editing || !layout::is_free(&self.config.widgets) {
            return None;
        }
        let rects = layout::free_rects(&self.config.widgets, self.config.gap);
        let canvas = layout::canvas_size(&rects);
        Some(Vec2::new(
            canvas.x.max(330.0),
            self.header_height + canvas.y,
        ))
    }
    /// Rectangles of the last flow-layout frame, relative to the content corner.
    pub fn flow_rects(&self) -> Vec<(String, egui::Rect)> {
        self.flow_cells.clone()
    }
    /// True when the tray is a free canvas the host may dock at a saved position.
    pub fn is_free(&self) -> bool {
        layout::is_free(&self.config.widgets)
    }
    pub fn cancel_customization(&mut self) {
        self.draft = None;
        self.editing = false;
        self.error = None;
    }
    pub fn import_draft(&mut self, text: &str) -> Result<(), String> {
        let config = parse_import(text)?;
        self.draft = Some(config);
        Ok(())
    }
    pub fn save_customization(&self) -> Result<ConfigAction, String> {
        let draft = self.draft.as_ref().ok_or("No settings draft")?;
        let mut draft = draft.clone();
        if self.editing {
            self.normalize_canvas(&mut draft);
        }
        draft.validate()?;
        if !self.styles.contains_key(&draft.theme.style) {
            return Err("This host has no compatible style renderer".into());
        }
        Ok(ConfigAction::Replace(Box::new(draft)))
    }
    /// Move the widgets so the canvas starts at zero and remember where its corner sits on
    /// the desktop, so the docked tray appears exactly where the widgets were left.
    fn normalize_canvas(&self, draft: &mut Config) {
        let rects = layout::free_rects(&draft.widgets, draft.gap);
        if rects.is_empty() {
            return;
        }
        let min = rects
            .iter()
            .fold(egui::vec2(f32::MAX, f32::MAX), |m, (_, r)| {
                egui::vec2(m.x.min(r.min.x), m.y.min(r.min.y))
            });
        let moved: Vec<_> = rects
            .iter()
            .map(|(id, r)| (id.clone(), r.translate(-min)))
            .collect();
        layout::freeze(&mut draft.widgets, &moved);
        if self.fullscreen_edit {
            let size = layout::canvas_size(&moved);
            let monitor = draft
                .geometry
                .get("tray")
                .map(|g| g.monitor.clone())
                .unwrap_or_default();
            draft.geometry.insert(
                "tray".into(),
                Geometry {
                    // Canvas positions are desktop positions while the editor covers the desktop.
                    x: min.x.max(0.0),
                    y: min.y.max(0.0),
                    width: size.x.max(330.0),
                    height: self.header_height + size.y.max(60.0),
                    monitor,
                },
            );
        }
    }
    pub fn accept(&mut self, config: Config) {
        self.config = config;
        self.draft = None;
        self.editing = false;
        self.error = None;
    }
    pub fn show(&mut self, ctx: &egui::Context, now: i64) -> Vec<Intent> {
        let mut intents = Vec::new();
        let system_dark = ctx.input(|i| i.raw.system_theme != Some(egui::Theme::Light));
        let theme = resolved_theme(
            self.draft
                .as_ref()
                .map(|d| &d.theme)
                .unwrap_or(&self.config.theme),
            system_dark,
        );
        if self.applied.as_ref() != Some(&(theme.clone(), system_dark)) {
            let factory = self.styles.get(&theme.style).copied().unwrap_or(terminal);
            ctx.set_visuals(factory(&theme, system_dark));
            ctx.style_mut(|s| {
                apply_typography(s, &theme);
            });
            self.applied = Some((theme.clone(), system_dark));
        }
        let bare = self.bare && (self.draft.is_none() || self.editing);
        let floating_toolbar = self.editing && self.fullscreen_edit;
        if floating_toolbar {
            egui::Area::new(egui::Id::new("edit-toolbar"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, 12.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Edit tray: drag borders, corners or widgets");
                            self.header_buttons(ui, &theme, true, &mut intents);
                        });
                        if let Some(error) = &self.error {
                            ui.colored_label(color(theme.palette.error), error);
                        }
                    });
                });
        } else {
            let header = egui::TopBottomPanel::top("header")
                .frame(if bare {
                    egui::Frame::NONE.inner_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: 0,
                        bottom: 8,
                    })
                } else {
                    egui::Frame::NONE
                        .fill(background(&theme))
                        .inner_margin(12.0)
                })
                .show_separator_line(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if !bare {
                            ui.label(
                                RichText::new("◉ STATUS QUO")
                                    .strong()
                                    .color(color(theme.palette.accent)),
                            );
                        }
                        if !bare || !self.connected {
                            ui.label(
                                RichText::new(if self.connected {
                                    "● connected"
                                } else {
                                    "○ reconnecting"
                                })
                                .small()
                                .color(color(theme.palette.muted)),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.header_buttons(ui, &theme, bare, &mut intents);
                        });
                    });
                    if let Some(error) = &self.error {
                        ui.colored_label(color(theme.palette.error), error);
                    }
                });
            self.header_height = header.response.rect.height();
        }
        egui::CentralPanel::default()
            .frame(if bare || floating_toolbar {
                egui::Frame::NONE
            } else {
                egui::Frame::NONE
                    .fill(background(&theme))
                    .inner_margin(12.0)
            })
            .show(ctx, |ui| {
                if self.draft.is_some() && !self.editing {
                    self.settings(ui, &mut intents);
                    return;
                }
                let edit_config = if self.editing {
                    self.draft.clone()
                } else {
                    None
                };
                let config = edit_config.as_ref().unwrap_or(&self.config);
                if self.editing || layout::is_free(&config.widgets) {
                    let offset = if floating_toolbar {
                        egui::vec2(0.0, self.header_height)
                    } else {
                        Vec2::ZERO
                    };
                    let corner = ui.max_rect().min + offset;
                    let rects = layout::free_rects(&config.widgets, config.gap);
                    self.canvas = layout::canvas_size(&rects);
                    if rects.is_empty() {
                        ui.add_space(40.0);
                        ui.heading("Your tray is empty");
                        ui.label("Use + Add widget to put a widget here.");
                    }
                    let mut ops: Vec<EditOp> = Vec::new();
                    let mut cells: Vec<(String, egui::Rect)> = Vec::new();
                    for (id, rect) in &rects {
                        let Some(instance) = config.widget(id) else {
                            continue;
                        };
                        let screen = rect.translate(corner.to_vec2());
                        // One layer per widget, stacked in configuration order: the widget drawn on
                        // top also gets the pointer where widgets overlap.
                        let editing = edit_config.is_some();
                        let area = egui::Area::new(egui::Id::new(("sq-widget", id)))
                            .order(egui::Order::Middle)
                            .fixed_pos(screen.min)
                            .constrain(false)
                            .sense(egui::Sense::hover())
                            .show(ctx, |ui| {
                                ui.set_clip_rect(screen);
                                ui.set_min_size(screen.size());
                                ui.set_max_size(screen.size());
                                let inner = ui
                                    .scope(|ui| {
                                        if editing {
                                            // Content must not compete with the edit handles.
                                            ui.disable();
                                        }
                                        self.widget(ui, instance, now, Some(screen.height()))
                                    })
                                    .inner;
                                if editing {
                                    edit_handles(ui, instance, screen, &mut ops, &mut cells);
                                }
                                inner
                            });
                        intents.extend(area.inner);
                        ctx.move_to_top(area.response.layer_id);
                    }
                    if edit_config.is_some() {
                        self.apply_edits(ops);
                        self.edit_cells = cells;
                    }
                    return;
                }
                let origin = ui.max_rect().min;
                self.flow_space = ui.available_height();
                let mut flow = Vec::new();
                egui::ScrollArea::vertical()
                    .id_salt("tray-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let top = ui.max_rect().min;
                        // One space between widgets, both ways, from the configuration.
                        ui.spacing_mut().item_spacing = Vec2::splat(config.gap);
                        let rows = layout(&config.widgets, ui.available_width());
                        if rows.is_empty() {
                            ui.add_space(40.0);
                            ui.heading("Your tray is empty");
                            ui.label("Use + Add widget to put a widget here.");
                        }
                        // Bare tray: row heights are scaled so the rows fill the space exactly.
                        let gap = ui.spacing().item_spacing.y;
                        let row_heights: Vec<f32> = rows
                            .iter()
                            .map(|row| {
                                row.ids
                                    .iter()
                                    .filter_map(|id| config.widget(id))
                                    .map(|w| w.height)
                                    .fold(80.0, f32::max)
                            })
                            .collect();
                        let total: f32 = row_heights.iter().sum();
                        let space =
                            ui.available_height() - gap * rows.len().saturating_sub(1) as f32;
                        let scale = if bare {
                            layout::fit_scale(total, space)
                        } else {
                            1.0
                        };
                        for (row, row_height) in rows.into_iter().zip(row_heights) {
                            let width = (ui.available_width()
                                - config.gap * (row.ids.len() - 1) as f32)
                                / row.ids.len() as f32;
                            let cell = row_height * scale;
                            ui.horizontal_top(|ui| {
                                for id in row.ids {
                                    if let Some(instance) = config.widget(&id) {
                                        let height = if bare { cell } else { instance.height };
                                        let placed = ui.allocate_ui_with_layout(
                                            Vec2::new(width, height),
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                                intents.extend(self.widget(
                                                    ui,
                                                    instance,
                                                    now,
                                                    bare.then_some(cell),
                                                ));
                                            },
                                        );
                                        let r = placed.response.rect;
                                        flow.push((
                                            id.clone(),
                                            egui::Rect::from_min_size(
                                                egui::pos2(r.min.x - top.x, r.min.y - top.y),
                                                r.size(),
                                            ),
                                        ));
                                    }
                                }
                            });
                        }
                    });
                let _ = origin;
                self.flow_cells = flow;
            });
        intents
    }
    fn header_buttons(
        &mut self,
        ui: &mut egui::Ui,
        theme: &Theme,
        bare: bool,
        intents: &mut Vec<Intent>,
    ) {
        let chip_fill = background(theme);
        let chip_stroke = egui::Stroke::new(1.0, color(theme.palette.muted).gamma_multiply(0.35));
        let chip = |ui: &mut egui::Ui, label: &str, selected: bool| {
            if bare {
                ui.add(egui::Button::new(label).fill(chip_fill).stroke(chip_stroke))
            } else {
                ui.selectable_label(selected, label)
            }
        };
        if self.editing {
            if chip(ui, "Done", false).clicked() {
                match self.save_customization() {
                    Ok(action) => intents.push(Intent::Configure(action)),
                    Err(error) => self.error = Some(error),
                }
            }
            if chip(ui, "Cancel", false).clicked() {
                self.cancel_customization();
            }
        } else {
            if chip(ui, "Customize", self.draft.is_some()).clicked() {
                if self.draft.is_some() {
                    self.cancel_customization();
                } else {
                    self.begin_customization();
                }
            }
            if self.draft.is_none() {
                let locked = self.config.locked;
                let response = ui
                    .add_enabled_ui(!locked, |ui| chip(ui, "Edit tray", false))
                    .inner;
                if locked {
                    response.on_hover_text(
                        "The widget layout is locked: unlock it in Customize to edit the tray",
                    );
                } else if response.clicked() {
                    self.begin_edit();
                }
            }
        }
        let add_button = if bare {
            egui::Button::new("+ Add widget")
                .fill(chip_fill)
                .stroke(chip_stroke)
        } else {
            egui::Button::new("+ Add widget")
        };
        egui::menu::menu_custom_button(ui, add_button, |ui| {
            for descriptor in self.registry.descriptors() {
                for provider in descriptor.providers {
                    if ui
                        .button(format!("{provider} / {}", descriptor.title))
                        .clicked()
                    {
                        if let Some(draft) = &mut self.draft {
                            let id = draft.add_widget(&descriptor.kind, &provider);
                            if self.editing {
                                // New widgets land at a free spot below the others.
                                let rects = layout::free_rects(&draft.widgets, draft.gap);
                                layout::freeze(&mut draft.widgets, &rects);
                                let _ = id;
                            }
                        } else {
                            intents.push(Intent::Configure(ConfigAction::Add {
                                kind: descriptor.kind.clone(),
                                provider,
                            }));
                        }
                        ui.close_menu();
                    }
                }
            }
        });
    }
    fn apply_edits(&mut self, ops: Vec<EditOp>) {
        let Some(draft) = self.draft.as_mut().filter(|_| self.editing) else {
            return;
        };
        if ops.is_empty() {
            // The drag ended: forget the unsnapped rectangle.
            self.drag_raw = None;
            return;
        }
        let gap = draft.gap;
        for op in ops {
            let (id, edges, delta) = match op {
                EditOp::Remove(id) => {
                    // Removing only edits the draft; Done saves it and Cancel brings it back.
                    draft.widgets.retain(|w| w.id != id);
                    self.drag_raw = None;
                    continue;
                }
                EditOp::Drag(id, edges, delta) => (id, edges, delta),
            };
            let others: Vec<egui::Rect> = layout::free_rects(&draft.widgets, gap)
                .into_iter()
                .filter(|(other, _)| other != &id)
                .map(|(_, r)| r)
                .collect();
            let Some(w) = draft.widget_mut(&id) else {
                continue;
            };
            // Follow the pointer with the unsnapped rectangle and show the snapped one, so a
            // widget sticks near another widget and lets go again when dragged further.
            let base = match &self.drag_raw {
                Some((raw_id, raw_edges, raw)) if raw_id == &id && *raw_edges == edges => *raw,
                _ => egui::Rect::from_min_size(
                    egui::pos2(w.x, w.y),
                    egui::vec2(w.width.max(layout::MIN_SIZE.x), w.height),
                ),
            };
            let raw = layout::drag_rect(base, edges, delta);
            self.drag_raw = Some((id.clone(), edges, raw));
            let next = layout::snap_rect(raw, edges, &others, gap);
            w.x = next.min.x;
            w.y = next.min.y;
            w.width = next.width().min(8_000.0);
            w.height = next.height().clamp(60.0, 8_000.0);
        }
    }
    pub fn detached_windows(&self, ctx: &egui::Context, now: i64) -> Vec<Intent> {
        let mut intents = Vec::new();
        for instance in self
            .config
            .widgets
            .iter()
            .filter(|w| w.detached && !w.hidden)
        {
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(&instance.id),
                egui::ViewportBuilder::default()
                    .with_title(format!("Status Quo / {}", instance.kind))
                    .with_inner_size([400.0, instance.height.max(200.0)]),
                |ctx, class| {
                    if class == egui::ViewportClass::Embedded {
                        egui::Window::new(format!("{} / {}", instance.provider, instance.kind))
                            .id(egui::Id::new(&instance.id))
                            .show(ctx, |ui| {
                                intents.extend(self.widget(ui, instance, now, None))
                            });
                    } else {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            intents.extend(self.widget(ui, instance, now, None))
                        });
                        if ctx.input(|i| i.viewport().close_requested()) {
                            let mut widget = instance.clone();
                            widget.detached = false;
                            intents.push(Intent::Configure(ConfigAction::Widget(Box::new(widget))));
                        }
                    }
                },
            );
        }
        intents
    }
    fn widget(
        &self,
        ui: &mut egui::Ui,
        instance: &WidgetInstance,
        now: i64,
        fit: Option<f32>,
    ) -> Vec<Intent> {
        let mut intents = Vec::new();
        let view = self.registry.view(instance, &self.snapshot, now);
        let resolved = resolved_theme(
            instance.theme.as_ref().unwrap_or(&self.config.theme),
            ui.ctx()
                .input(|i| i.raw.system_theme != Some(egui::Theme::Light)),
        );
        let theme = &resolved;
        ui.push_id(&instance.id, |ui| {
            let old = ui.style().clone();
            let factory = self.styles.get(&theme.style).copied().unwrap_or(terminal);
            let mut style = (*old).clone();
            style.visuals = factory(theme, ui.visuals().dark_mode);
            apply_typography(&mut style, theme);
            ui.set_style(style);
            egui::Frame::NONE
                .fill(background(theme))
                .stroke(egui::Stroke::new(
                    1.0,
                    color(theme.palette.muted).gamma_multiply(0.35),
                ))
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().max(80.0));
                    if let Some(height) = fit {
                        ui.set_min_height((height - 26.0).max(0.0));
                    }
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&view.title)
                                .strong()
                                .color(color(theme.palette.accent)),
                        );
                        if !self.config.locked {
                            ui.menu_button("⋮", |ui| {
                                for (label, action) in [
                                    ("Remove", ConfigAction::Remove(instance.id.clone())),
                                    ("Detach / reattach", {
                                        let mut w = instance.clone();
                                        w.detached = !w.detached;
                                        ConfigAction::Widget(Box::new(w))
                                    }),
                                ] {
                                    if ui.button(label).clicked() {
                                        intents.push(Intent::Configure(action));
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                    });
                    if instance.kind == "tokens" {
                        let mut widget = instance.clone();
                        let mut changed = false;
                        ui.horizontal_wrapped(|ui| {
                            let range =
                                widget.options.entry("range".into()).or_insert("24h".into());
                            for value in ["24h", "7d", "30d"] {
                                changed |=
                                    ui.selectable_value(range, value.into(), value).changed();
                            }
                            let bucket = widget
                                .options
                                .entry("bucket".into())
                                .or_insert("hourly".into());
                            egui::ComboBox::from_id_salt("bucket")
                                .selected_text(bucket.as_str())
                                .show_ui(ui, |ui| {
                                    for value in ["hourly", "daily"] {
                                        changed |= ui
                                            .selectable_value(bucket, value.into(), value)
                                            .changed();
                                    }
                                });
                        });
                        let mut cumulative = widget
                            .options
                            .get("cumulative")
                            .is_some_and(|v| v == "true");
                        if ui.checkbox(&mut cumulative, "Cumulative").changed() {
                            widget
                                .options
                                .insert("cumulative".into(), cumulative.to_string());
                            changed = true;
                        }
                        if changed {
                            intents.push(Intent::Configure(ConfigAction::Widget(Box::new(widget))));
                        }
                    }
                    let available = (fit.unwrap_or(instance.height)
                        - if instance.kind == "tokens" {
                            100.0
                        } else {
                            55.0
                        })
                    .max(30.0);
                    // Tell the content how much room it has, so charts and meters adapt.
                    let room = Vec2::new(ui.available_width(), available);
                    ui.data_mut(|d| d.insert_temp(egui::Id::new("sq-room"), room));
                    egui::ScrollArea::vertical()
                        .id_salt("contents")
                        .auto_shrink([false, fit.is_none()])
                        .max_height(available)
                        .show(ui, |ui| {
                            for node in &view.nodes {
                                render_node(
                                    ui,
                                    node,
                                    theme,
                                    self.config.navigation,
                                    now,
                                    &mut intents,
                                );
                            }
                        });
                });
            ui.set_style(old);
        });
        intents
    }
    fn settings(&mut self, ui: &mut egui::Ui, intents: &mut Vec<Intent>) {
        ui.horizontal(|ui| {
            if ui.button("Save changes").clicked() {
                match self.save_customization() {
                    Ok(action) => intents.push(Intent::Configure(action)),
                    Err(error) => self.error = Some(error),
                }
            }
            if ui.button("Cancel").clicked() {
                self.cancel_customization();
            }
        });
        let Some(draft) = &mut self.draft else {
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("customization")
            .show(ui, |ui| {
                ui.heading("Appearance");
                theme_controls(ui, &mut draft.theme, &self.styles);
                ui.add(egui::Slider::new(&mut draft.gap, 0.0..=40.0).text("Space between widgets"));
                ui.separator();
                ui.heading("Desktop behavior");
                ui.horizontal(|ui| {
                    ui.label("Window layer");
                    for layer in [Layer::Behind, Layer::Normal, Layer::Above] {
                        ui.selectable_value(&mut draft.layer, layer, format!("{layer:?}"));
                    }
                });
                ui.label(&self.capabilities);
                ui.checkbox(&mut draft.navigation, "Enable links and project navigation");
                ui.checkbox(&mut draft.autostart, "Start at login");
                ui.checkbox(&mut draft.locked, "Lock widget layout");
                ui.separator();
                ui.heading("Widgets");
                ui.label(
                    "Each entry is independent. Move, replace, resize, hide or detach it here.",
                );
                let mut remove = None;
                let mut movement = None;
                let len = draft.widgets.len();
                for (index, widget) in draft.widgets.iter_mut().enumerate() {
                    egui::CollapsingHeader::new(format!("{} / {}", widget.provider, widget.kind))
                        .id_salt(&widget.id)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(index > 0, egui::Button::new("↑ Move up"))
                                    .clicked()
                                {
                                    movement = Some((index, index - 1));
                                }
                                if ui
                                    .add_enabled(index + 1 < len, egui::Button::new("↓ Move down"))
                                    .clicked()
                                {
                                    movement = Some((index, index + 1));
                                }
                                if ui.button("Remove").clicked() {
                                    remove = Some(index);
                                }
                            });
                            egui::ComboBox::from_id_salt(("kind", &widget.id))
                                .selected_text(&widget.kind)
                                .show_ui(ui, |ui| {
                                    for d in self.registry.descriptors() {
                                        ui.selectable_value(
                                            &mut widget.kind,
                                            d.kind.clone(),
                                            d.title,
                                        );
                                    }
                                });
                            let providers = self
                                .registry
                                .descriptors()
                                .into_iter()
                                .find(|d| d.kind == widget.kind)
                                .map(|d| d.providers)
                                .unwrap_or_default();
                            egui::ComboBox::from_id_salt(("provider", &widget.id))
                                .selected_text(&widget.provider)
                                .show_ui(ui, |ui| {
                                    for p in providers {
                                        ui.selectable_value(&mut widget.provider, p.clone(), p);
                                    }
                                });
                            ui.add(
                                egui::Slider::new(&mut widget.height, 100.0..=1400.0)
                                    .text("Height"),
                            );
                            ui.add(egui::Slider::new(&mut widget.span, 1..=2).text("Columns"));
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut widget.hidden, "Hidden");
                                ui.checkbox(&mut widget.detached, "Separate window");
                            });
                            if widget.kind == "sessions" {
                                let mut history =
                                    widget.options.get("history").is_some_and(|v| v == "true");
                                if ui
                                    .checkbox(&mut history, "Include retained session history")
                                    .changed()
                                {
                                    widget.options.insert("history".into(), history.to_string());
                                }
                            }
                            if widget.kind == "activity" || widget.kind == "memory" {
                                let key = if widget.kind == "activity" {
                                    "repository"
                                } else {
                                    "type"
                                };
                                let value = widget.options.entry(key.into()).or_default();
                                ui.horizontal(|ui| {
                                    ui.label(format!("Filter {key} (empty = all)"));
                                    ui.text_edit_singleline(value);
                                });
                                if value.is_empty() {
                                    widget.options.remove(key);
                                }
                            }
                            let mut custom = widget.theme.is_some();
                            if ui
                                .checkbox(&mut custom, "Override widget appearance")
                                .changed()
                            {
                                widget.theme = if custom {
                                    Some(draft.theme.clone())
                                } else {
                                    None
                                };
                            }
                            if let Some(theme) = &mut widget.theme {
                                theme_controls(ui, theme, &self.styles);
                            }
                        });
                }
                if let Some(index) = remove {
                    draft.widgets.remove(index);
                } else if let Some((a, b)) = movement {
                    draft.widgets.swap(a, b);
                }
                ui.separator();
                ui.heading("Sources");
                ui.label("Paths are read only. Credentials remain with the provider CLI.");
                for (label, path) in [
                    ("Claude directory", &mut draft.sources.claude),
                    ("Codex directory", &mut draft.sources.codex),
                    ("Workflow memory", &mut draft.sources.memory),
                    ("Task progress", &mut draft.sources.progress),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.text_edit_singleline(path);
                    });
                }
                ui.label("Local project roots (one absolute path or ~/ path per line)");
                let mut roots = draft.sources.project_roots.join("\n");
                if ui.text_edit_multiline(&mut roots).changed() {
                    draft.sources.project_roots = roots
                        .lines()
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(String::from)
                        .collect();
                }
                ui.add(
                    egui::Slider::new(&mut draft.sources.poll_seconds, 300..=3600)
                        .text("Account polling interval (seconds)"),
                );
                ui.label("Selected GitHub repositories (owner/name, one per line)");
                let mut repos = draft.repositories.join("\n");
                if ui.text_edit_multiline(&mut repos).changed() {
                    draft.repositories = repos
                        .lines()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect();
                }
                ui.separator();
                ui.heading("Presets and portable settings");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.preset_name);
                    if ui.button("Save preset").clicked() && !self.preset_name.trim().is_empty() {
                        draft.presets.insert(
                            self.preset_name.trim().into(),
                            Preset {
                                theme: draft.theme.clone(),
                                widgets: draft.widgets.clone(),
                                layer: draft.layer,
                            },
                        );
                    }
                });
                let names: Vec<_> = draft.presets.keys().cloned().collect();
                for name in names {
                    ui.horizontal(|ui| {
                        ui.label(&name);
                        if ui.button("Apply").clicked() {
                            if let Some(preset) = draft.presets.get(&name) {
                                draft.theme = preset.theme.clone();
                                draft.widgets = preset.widgets.clone();
                                draft.layer = preset.layer;
                            }
                        }
                        if ui.button("Delete").clicked() {
                            draft.presets.remove(&name);
                        }
                    });
                }
                if ui.button("Export settings to text").clicked() {
                    match serde_json::to_string_pretty(draft) {
                        Ok(text) => self.import = text,
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                ui.add(
                    egui::TextEdit::multiline(&mut self.import)
                        .desired_rows(5)
                        .hint_text("Paste exported settings here"),
                );
                if ui.button("Import into draft").clicked() {
                    match parse_import(&self.import) {
                        Ok(config) => *draft = config,
                        Err(error) => self.error = Some(error),
                    }
                }
            });
    }
}
/// Compress frozen rectangles vertically so the whole layout fits the space the tray had; a
/// tray whose rows scrolled would otherwise keep widgets below the screen edge.
fn fit_to_space(rects: Vec<(String, egui::Rect)>, space: f32) -> Vec<(String, egui::Rect)> {
    let bottom = layout::canvas_size(&rects).y;
    if space <= 0.0 || bottom <= space {
        return rects;
    }
    let scale = space / bottom;
    rects
        .into_iter()
        .map(|(id, r)| {
            let top = r.min.y * scale;
            let height = (r.height() * scale).max(60.0);
            (
                id,
                egui::Rect::from_min_size(egui::pos2(r.min.x, top), egui::vec2(r.width(), height)),
            )
        })
        .collect()
}
enum EditOp {
    Drag(String, layout::Edges, Vec2),
    Remove(String),
}
/// Edit-mode overlay for one widget on the free canvas: the body moves it, every border and
/// corner resizes it. Added after the content so it takes the pointer.
fn edit_handles(
    ui: &mut egui::Ui,
    instance: &WidgetInstance,
    rect: egui::Rect,
    ops: &mut Vec<EditOp>,
    cells: &mut Vec<(String, egui::Rect)>,
) {
    use egui::{CursorIcon, Id, Sense};
    const GRIP: f32 = 8.0;
    const CORNER: f32 = 16.0;
    cells.push((instance.id.clone(), rect));
    let accent = ui.visuals().selection.stroke.color;
    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, accent.gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    let mut handle = |ui: &mut egui::Ui,
                      name: &str,
                      area: egui::Rect,
                      edges: layout::Edges,
                      cursor: CursorIcon,
                      show: bool| {
        let response = ui.interact(area, Id::new((name, &instance.id)), Sense::drag());
        if response.hovered() || response.dragged() {
            ui.output_mut(|o| o.cursor_icon = cursor);
            if show {
                ui.painter()
                    .rect_filled(area, 0.0, accent.gamma_multiply(0.6));
            }
        }
        if response.dragged() {
            ops.push(EditOp::Drag(
                instance.id.clone(),
                edges,
                response.drag_delta(),
            ));
        }
    };
    let e = |left, right, top, bottom| layout::Edges {
        left,
        right,
        top,
        bottom,
    };
    let body_cursor = CursorIcon::Grab;
    handle(
        ui,
        "edit-body",
        rect,
        layout::Edges::BODY,
        body_cursor,
        false,
    );
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let strip = |x0, y0, x1, y1| egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1));
    handle(
        ui,
        "edit-left",
        strip(l, t + CORNER, l + GRIP, b - CORNER),
        e(true, false, false, false),
        CursorIcon::ResizeHorizontal,
        true,
    );
    handle(
        ui,
        "edit-right",
        strip(r - GRIP, t + CORNER, r, b - CORNER),
        e(false, true, false, false),
        CursorIcon::ResizeHorizontal,
        true,
    );
    handle(
        ui,
        "edit-top",
        strip(l + CORNER, t, r - CORNER, t + GRIP),
        e(false, false, true, false),
        CursorIcon::ResizeVertical,
        true,
    );
    handle(
        ui,
        "edit-bottom",
        strip(l + CORNER, b - GRIP, r - CORNER, b),
        e(false, false, false, true),
        CursorIcon::ResizeVertical,
        true,
    );
    handle(
        ui,
        "edit-tl",
        strip(l, t, l + CORNER, t + CORNER),
        e(true, false, true, false),
        CursorIcon::ResizeNwSe,
        true,
    );
    handle(
        ui,
        "edit-tr",
        strip(r - CORNER, t, r, t + CORNER),
        e(false, true, true, false),
        CursorIcon::ResizeNeSw,
        true,
    );
    handle(
        ui,
        "edit-bl",
        strip(l, b - CORNER, l + CORNER, b),
        e(true, false, false, true),
        CursorIcon::ResizeNeSw,
        true,
    );
    handle(
        ui,
        "edit-br",
        strip(r - CORNER, b - CORNER, r, b),
        e(false, true, false, true),
        CursorIcon::ResizeNwSe,
        true,
    );
    // Remove sits in the centre of the widget and is added last, so it takes the pointer
    // before the body that moves the widget.
    let remove = egui::Rect::from_center_size(rect.center(), egui::vec2(84.0, 28.0));
    if ui
        .put(remove, egui::Button::new("Remove"))
        .on_hover_text("Remove this widget from the tray")
        .clicked()
    {
        ops.push(EditOp::Remove(instance.id.clone()));
    }
}
fn parse_import(text: &str) -> Result<Config, String> {
    if text.len() > 1024 * 1024 {
        return Err("Settings exceed import size limit".into());
    }
    let config: Config =
        serde_json::from_str(text).map_err(|e| format!("Invalid settings: {e}"))?;
    config.validate()?;
    Ok(config)
}
fn color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}
fn background(theme: &Theme) -> Color32 {
    let [r, g, b] = theme.palette.background;
    Color32::from_rgba_unmultiplied(r, g, b, (theme.opacity * 255.0) as u8)
}
fn resolved_theme(theme: &Theme, system_dark: bool) -> Theme {
    let mut resolved = theme.clone();
    if theme.mode == Mode::System
        && (theme.palette == Palette::default() || theme.palette == Palette::light())
    {
        resolved.palette = if system_dark {
            Palette::default()
        } else {
            Palette::light()
        };
    }
    resolved
}
fn apply_typography(style: &mut egui::Style, theme: &Theme) {
    for font in style.text_styles.values_mut() {
        font.family = egui::FontFamily::Monospace;
        font.size = theme.font_size;
    }
    style.spacing.item_spacing = Vec2::new(10.0, if theme.compact { 4.0 } else { 8.0 });
}
pub fn terminal(theme: &Theme, system_dark: bool) -> egui::Visuals {
    let resolved = resolved_theme(theme, system_dark);
    let theme = &resolved;
    let dark = match theme.mode {
        Mode::Dark => true,
        Mode::Light => false,
        Mode::System => system_dark,
    };
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(color(theme.palette.text));
    visuals.panel_fill = background(theme);
    visuals.window_fill = background(theme);
    visuals.selection.bg_fill = color(theme.palette.accent).gamma_multiply(0.3);
    visuals.selection.stroke.color = color(theme.palette.accent);
    visuals.hyperlink_color = color(theme.palette.accent);
    visuals
}
fn theme_controls(ui: &mut egui::Ui, theme: &mut Theme, styles: &BTreeMap<String, StyleFactory>) {
    ui.horizontal(|ui| {
        ui.label("Mode");
        for mode in [Mode::Dark, Mode::Light, Mode::System] {
            if ui
                .selectable_value(&mut theme.mode, mode, format!("{mode:?}"))
                .changed()
            {
                theme.palette = if mode == Mode::Light {
                    Palette::light()
                } else {
                    Palette::default()
                };
            }
        }
    });
    egui::ComboBox::from_id_salt("visual-style")
        .selected_text(&theme.style)
        .show_ui(ui, |ui| {
            for name in styles.keys() {
                ui.selectable_value(&mut theme.style, name.clone(), name);
            }
        });
    egui::Grid::new("palette").num_columns(2).show(ui, |ui| {
        for (label, rgb) in [
            ("Background", &mut theme.palette.background),
            ("Text", &mut theme.palette.text),
            ("Accent", &mut theme.palette.accent),
            ("Muted", &mut theme.palette.muted),
            ("Warning", &mut theme.palette.warning),
            ("Error", &mut theme.palette.error),
        ] {
            ui.label(label);
            ui.color_edit_button_srgb(rgb);
            ui.end_row();
        }
    });
    ui.add(egui::Slider::new(&mut theme.opacity, 0.0..=1.0).text("Background opacity"));
    ui.add(egui::Slider::new(&mut theme.font_size, 10.0..=32.0).text("Font size"));
    ui.checkbox(&mut theme.compact, "Compact spacing");
}
fn render_node(
    ui: &mut egui::Ui,
    node: &Node,
    theme: &Theme,
    navigation: bool,
    now: i64,
    intents: &mut Vec<Intent>,
) {
    match node {
        Node::Text(text) => {
            ui.label(text);
        }
        Node::Notice { tone, text } => {
            ui.label(RichText::new(text).small().color(color(match tone {
                Tone::Error => theme.palette.error,
                Tone::Warning => theme.palette.warning,
                Tone::Accent => theme.palette.accent,
                Tone::Muted => theme.palette.muted,
            })));
        }
        Node::Meter {
            label,
            used,
            resets_at,
        } => {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        used.map(|v| format!("{v:.0}% used"))
                            .unwrap_or_else(|| "Unavailable".into()),
                    );
                });
            });
            if let Some(used) = used {
                ui.add(
                    egui::ProgressBar::new((*used as f32 / 100.0).clamp(0.0, 1.0))
                        .fill(color(theme.palette.accent))
                        .desired_height(6.0),
                );
            }
            if let Some(reset) = resets_at {
                ui.add(
                    egui::Label::new(
                        RichText::new(reset_label(*reset, now))
                            .small()
                            .color(color(theme.palette.muted)),
                    )
                    .wrap(),
                );
            }
        }
        Node::Row {
            id,
            label,
            detail,
            action,
            children,
        } => {
            egui::CollapsingHeader::new(label)
                .id_salt(id)
                .show(ui, |ui| {
                    ui.label(detail);
                    if let Some(action) = action {
                        if ui
                            .add_enabled(navigation, egui::Button::new("Open source"))
                            .clicked()
                        {
                            intents.push(Intent::Navigate(action.clone()));
                        }
                    }
                    for child in children {
                        render_node(ui, child, theme, navigation, now, intents);
                    }
                });
        }
        Node::Chart { series } => chart(ui, series, theme),
    }
}
fn chart(ui: &mut egui::Ui, series: &[Series], theme: &Theme) {
    // The chart takes about half of the widget's room and all of its width.
    let room: Vec2 = ui
        .data(|d| d.get_temp(egui::Id::new("sq-room")))
        .unwrap_or(Vec2::new(400.0, 190.0));
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(
            ui.available_width().max(60.0),
            (room.y * 0.55).clamp(48.0, 1_200.0),
        ),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let colors = [
        color(theme.palette.accent),
        Color32::from_rgb(100, 175, 245),
        color(theme.palette.warning),
        Color32::from_rgb(198, 136, 228),
    ];
    let min = series
        .iter()
        .flat_map(|s| s.points.iter())
        .map(|p| p.at)
        .min()
        .unwrap_or(0);
    let max = series
        .iter()
        .flat_map(|s| s.points.iter())
        .map(|p| p.at)
        .max()
        .unwrap_or(min);
    let time_span = (max as f64 - min as f64).max(1.0);
    let peak = series
        .iter()
        .flat_map(|s| s.points.iter())
        .filter_map(|p| p.value)
        .fold(1.0f64, f64::max);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, color(theme.palette.muted)),
    );
    for (index, s) in series.iter().enumerate() {
        let mut previous = None;
        for point in &s.points {
            if let Some(value) = point.value {
                let position = egui::pos2(
                    rect.left()
                        + ((point.at as f64 - min as f64) / time_span) as f32 * rect.width(),
                    rect.bottom() - (value / peak) as f32 * (rect.height() - 10.0),
                );
                if let Some(old) = previous {
                    painter
                        .line_segment([old, position], egui::Stroke::new(1.5, colors[index % 4]));
                } else {
                    painter.circle_filled(position, 2.0, colors[index % 4]);
                }
                previous = Some(position);
            } else {
                previous = None;
            }
        }
    }
    painter.text(
        rect.right_top(),
        egui::Align2::RIGHT_TOP,
        format!("{peak:.0} tokens"),
        egui::FontId::monospace(10.0),
        color(theme.palette.muted),
    );
    let hover_time = response
        .hover_pos()
        .map(|p| {
            min as f64 + ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * time_span
        })
        .unwrap_or(max as f64);
    response.on_hover_ui(|ui| {
        ui.label(format!(
            "Peak {peak:.0} tokens per bucket; gaps indicate unknown counters"
        ));
        for s in series {
            if let Some(point) = s.points.iter().min_by(|a, b| {
                ((a.at as f64 - hover_time).abs()).total_cmp(&(b.at as f64 - hover_time).abs())
            }) {
                ui.label(format!(
                    "{} · {}: {}",
                    timestamp_utc(point.at),
                    s.label,
                    point
                        .value
                        .map(|v| format!("{v:.0}"))
                        .unwrap_or_else(|| "unknown".into())
                ));
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(timestamp_utc(min))
                .small()
                .color(color(theme.palette.muted)),
        );
        ui.label(
            RichText::new(format!("→ {}", timestamp_utc(max)))
                .small()
                .color(color(theme.palette.muted)),
        );
    });
    ui.horizontal_wrapped(|ui| {
        for (index, s) in series.iter().enumerate() {
            ui.label(RichText::new(&s.label).small().color(colors[index % 4]));
        }
    });
}

/// UTC date without platform time-zone or browser dependencies.
pub fn timestamp_utc(at: i64) -> String {
    let days = at.div_euclid(86400);
    let seconds = at.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3600,
        (seconds % 3600) / 60
    )
}
pub fn reset_label(reset: i64, now: i64) -> String {
    let remaining = reset.saturating_sub(now);
    let duration = if remaining <= 0 {
        "reset pending".into()
    } else if remaining < 3600 {
        format!("in {}m", (remaining + 59) / 60)
    } else if remaining < 86400 {
        format!("in {}h {}m", remaining / 3600, (remaining % 3600) / 60)
    } else {
        format!("in {}d {}h", remaining / 86400, (remaining % 86400) / 3600)
    };
    format!("Resets {duration} · {}", timestamp_utc(reset))
}
#[cfg(test)]
mod tests {
    use super::*;
    // REQ SQ-3
    #[test]
    fn per_widget_style_applies_size_spacing_and_palette() {
        let theme = Theme {
            font_size: 23.0,
            compact: true,
            ..Theme::default()
        };
        let mut style = egui::Style::default();
        apply_typography(&mut style, &theme);
        assert!(style
            .text_styles
            .values()
            .all(|f| f.size == 23.0 && f.family == egui::FontFamily::Monospace));
        assert_eq!(style.spacing.item_spacing.y, 4.0);
    }
    // REQ SQ-5
    #[test]
    fn reset_dates_are_human_readable_across_epoch_leap_day_and_expiry() {
        assert_eq!(timestamp_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(timestamp_utc(-1), "1969-12-31 23:59 UTC");
        assert_eq!(timestamp_utc(1709164800), "2024-02-29 00:00 UTC");
        assert!(reset_label(3600, 0).starts_with("Resets in 1h 0m"));
        assert!(reset_label(0, 1).contains("reset pending"));
    }
}
