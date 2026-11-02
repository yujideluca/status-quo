use sq_core::WidgetInstance;
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutRow {
    pub ids: Vec<String>,
}
/// Host-independent packing. Wide instances occupy their own row; narrow hosts stack.
pub fn layout(widgets: &[WidgetInstance], width: f32) -> Vec<LayoutRow> {
    let columns = if width >= 620.0 { 2 } else { 1 };
    let mut rows = Vec::new();
    let mut pending = Vec::new();
    for widget in widgets.iter().filter(|w| !w.hidden && !w.detached) {
        if widget.span as usize >= columns {
            if !pending.is_empty() {
                rows.push(LayoutRow {
                    ids: std::mem::take(&mut pending),
                });
            }
            rows.push(LayoutRow {
                ids: vec![widget.id.clone()],
            });
        } else {
            pending.push(widget.id.clone());
            if pending.len() == columns {
                rows.push(LayoutRow {
                    ids: std::mem::take(&mut pending),
                });
            }
        }
    }
    if !pending.is_empty() {
        rows.push(LayoutRow { ids: pending });
    }
    rows
}

/// Scale applied to row heights so the rows plus gaps fill `space` exactly.
/// Never below 0.6: smaller rows would crush content, so the tray scrolls instead.
pub fn fit_scale(total_height: f32, space: f32) -> f32 {
    if total_height <= 0.0 {
        return 1.0;
    }
    (space / total_height).max(0.6)
}

/// Distance, in pixels, within which a border or a size sticks to another widget.
pub const SNAP: f32 = 10.0;
/// Smallest size a widget can be dragged to.
pub const MIN_SIZE: egui::Vec2 = egui::vec2(120.0, 80.0);
/// Width given to a widget that has no explicit size yet.
pub const DEFAULT_WIDTH: f32 = 420.0;

/// True once any visible widget carries an explicit size: the tray is then a free canvas.
pub fn is_free(widgets: &[WidgetInstance]) -> bool {
    widgets
        .iter()
        .any(|w| !w.hidden && !w.detached && w.width > 0.0)
}

/// Canvas rectangle of every visible, attached widget, relative to the canvas corner.
/// Sized widgets keep their rectangle; unsized ones stack below everything else.
pub fn free_rects(widgets: &[WidgetInstance], gap: f32) -> Vec<(String, egui::Rect)> {
    let mut rects = Vec::new();
    let mut bottom = 0.0_f32;
    for w in widgets.iter().filter(|w| !w.hidden && !w.detached) {
        if w.width > 0.0 {
            let rect =
                egui::Rect::from_min_size(egui::pos2(w.x, w.y), egui::vec2(w.width, w.height));
            bottom = bottom.max(rect.bottom());
            rects.push((w.id.clone(), rect));
        }
    }
    let mut stacked = Vec::new();
    for w in widgets.iter().filter(|w| !w.hidden && !w.detached) {
        if w.width <= 0.0 {
            let y = if bottom > 0.0 { bottom + gap } else { 0.0 };
            let rect =
                egui::Rect::from_min_size(egui::pos2(0.0, y), egui::vec2(DEFAULT_WIDTH, w.height));
            bottom = rect.bottom();
            stacked.push((w.id.clone(), rect));
        }
    }
    rects.extend(stacked);
    rects
}

/// Size of the smallest rectangle that holds every widget, from the canvas corner.
pub fn canvas_size(rects: &[(String, egui::Rect)]) -> egui::Vec2 {
    rects.iter().fold(egui::Vec2::ZERO, |size, (_, r)| {
        egui::vec2(size.x.max(r.right()), size.y.max(r.bottom()))
    })
}

/// Write rectangles into widgets, making every one explicitly sized.
pub fn freeze(widgets: &mut [WidgetInstance], rects: &[(String, egui::Rect)]) {
    for (id, rect) in rects {
        if let Some(w) = widgets.iter_mut().find(|w| &w.id == id) {
            w.x = rect.min.x.max(0.0);
            w.y = rect.min.y.max(0.0);
            w.width = rect.width().max(MIN_SIZE.x);
            w.height = rect.height().clamp(60.0, 8_000.0);
        }
    }
}

/// Which borders a drag handle moves. A corner moves two of them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}
impl Edges {
    pub const BODY: Edges = Edges {
        left: true,
        right: true,
        top: true,
        bottom: true,
    };
}

/// Rectangle after dragging `edges` by `delta`. All four borders move freely; the result
/// never gets smaller than `MIN_SIZE` and never crosses the canvas corner. Dragging all four
/// borders together moves the rectangle without changing its size.
pub fn drag_rect(rect: egui::Rect, edges: Edges, delta: egui::Vec2) -> egui::Rect {
    let moving = edges == Edges::BODY;
    let mut r = rect;
    if moving {
        let shift = egui::vec2(
            (rect.min.x + delta.x).max(0.0) - rect.min.x,
            (rect.min.y + delta.y).max(0.0) - rect.min.y,
        );
        return rect.translate(shift);
    }
    if edges.left {
        r.min.x = (r.min.x + delta.x).clamp(0.0, r.max.x - MIN_SIZE.x);
    }
    if edges.right {
        r.max.x = (r.max.x + delta.x).max(r.min.x + MIN_SIZE.x);
    }
    if edges.top {
        r.min.y = (r.min.y + delta.y).clamp(0.0, r.max.y - MIN_SIZE.y);
    }
    if edges.bottom {
        r.max.y = (r.max.y + delta.y).max(r.min.y + MIN_SIZE.y);
    }
    r
}

fn near(value: f32, candidates: impl IntoIterator<Item = f32>) -> Option<f32> {
    candidates
        .into_iter()
        .filter(|c| (c - value).abs() <= SNAP)
        .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
}
/// True when two ranges overlap once each is widened by `slack`.
fn overlaps(a: (f32, f32), b: (f32, f32), slack: f32) -> bool {
    a.0 < b.1 + slack && b.0 < a.1 + slack
}

/// Make a dragged rectangle stick to the other widgets. A border within `SNAP` pixels of a
/// neighbour's border settles exactly `gap` pixels away from it (the one space used everywhere),
/// or flush with an aligned border, and a width or height within `SNAP` of another widget's
/// becomes exactly that size. `edges` says what the drag moves; all four means a move.
pub fn snap_rect(rect: egui::Rect, edges: Edges, others: &[egui::Rect], gap: f32) -> egui::Rect {
    let reach = gap + SNAP;
    let rows = (rect.min.y, rect.max.y);
    let columns = (rect.min.x, rect.max.x);
    let beside = |o: &egui::Rect| overlaps(rows, (o.min.y, o.max.y), reach);
    let above_or_below = |o: &egui::Rect| overlaps(columns, (o.min.x, o.max.x), reach);
    if edges == Edges::BODY {
        let shift = |lefts: Vec<f32>, rights: Vec<f32>, min: f32, max: f32| {
            lefts
                .into_iter()
                .map(|c| c - min)
                .chain(rights.into_iter().map(|c| c - max))
                .filter(|s| s.abs() <= SNAP)
                .min_by(|a, b| a.abs().total_cmp(&b.abs()))
                .unwrap_or(0.0)
        };
        let dx = shift(
            others
                .iter()
                .flat_map(|o| {
                    let mut v = vec![o.min.x];
                    if beside(o) {
                        v.push(o.max.x + gap);
                    }
                    v
                })
                .collect(),
            others
                .iter()
                .flat_map(|o| {
                    let mut v = vec![o.max.x];
                    if beside(o) {
                        v.push(o.min.x - gap);
                    }
                    v
                })
                .collect(),
            rect.min.x,
            rect.max.x,
        );
        let dy = shift(
            others
                .iter()
                .flat_map(|o| {
                    let mut v = vec![o.min.y];
                    if above_or_below(o) {
                        v.push(o.max.y + gap);
                    }
                    v
                })
                .collect(),
            others
                .iter()
                .flat_map(|o| {
                    let mut v = vec![o.max.y];
                    if above_or_below(o) {
                        v.push(o.min.y - gap);
                    }
                    v
                })
                .collect(),
            rect.min.y,
            rect.max.y,
        );
        let moved = rect.translate(egui::vec2(dx, dy));
        return moved.translate(egui::vec2((-moved.min.x).max(0.0), (-moved.min.y).max(0.0)));
    }
    let mut r = rect;
    if edges.left {
        let edges_at = others.iter().flat_map(|o| {
            let mut v = vec![o.min.x];
            if beside(o) {
                v.push(o.max.x + gap);
            }
            v
        });
        let sizes = others.iter().map(|o| rect.max.x - o.width());
        if let Some(x) = near(rect.min.x, edges_at).or_else(|| near(rect.min.x, sizes)) {
            r.min.x = x.clamp(0.0, r.max.x - MIN_SIZE.x);
        }
    }
    if edges.right {
        let edges_at = others.iter().flat_map(|o| {
            let mut v = vec![o.max.x];
            if beside(o) {
                v.push(o.min.x - gap);
            }
            v
        });
        let sizes = others.iter().map(|o| rect.min.x + o.width());
        if let Some(x) = near(rect.max.x, edges_at).or_else(|| near(rect.max.x, sizes)) {
            r.max.x = x.max(r.min.x + MIN_SIZE.x);
        }
    }
    if edges.top {
        let edges_at = others.iter().flat_map(|o| {
            let mut v = vec![o.min.y];
            if above_or_below(o) {
                v.push(o.max.y + gap);
            }
            v
        });
        let sizes = others.iter().map(|o| rect.max.y - o.height());
        if let Some(y) = near(rect.min.y, edges_at).or_else(|| near(rect.min.y, sizes)) {
            r.min.y = y.clamp(0.0, r.max.y - MIN_SIZE.y);
        }
    }
    if edges.bottom {
        let edges_at = others.iter().flat_map(|o| {
            let mut v = vec![o.max.y];
            if above_or_below(o) {
                v.push(o.min.y - gap);
            }
            v
        });
        let sizes = others.iter().map(|o| rect.min.y + o.height());
        if let Some(y) = near(rect.max.y, edges_at).or_else(|| near(rect.max.y, sizes)) {
            r.max.y = y.max(r.min.y + MIN_SIZE.y);
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2, Rect};

    // REQ SQ-28: rows grow or shrink so the widgets fill the tray space exactly.
    #[test]
    fn rows_fill_the_space_exactly_until_the_minimum_scale() {
        assert_eq!(fit_scale(500.0, 1000.0), 2.0);
        assert_eq!(fit_scale(1000.0, 500.0), 0.6);
        assert_eq!(fit_scale(0.0, 500.0), 1.0);
        let heights = [130.0_f32, 200.0, 200.0];
        let total: f32 = heights.iter().sum();
        let scale = fit_scale(total, 900.0);
        let filled: f32 = heights.iter().map(|h| h * scale).sum();
        assert!((filled - 900.0).abs() < 0.01);
    }

    // REQ SQ-29: any border drags freely on both axes, with a minimum size and no negative corner.
    #[test]
    fn any_border_resizes_on_both_axes_and_the_body_moves() {
        let r = Rect::from_min_size(pos2(100.0, 100.0), vec2(300.0, 200.0));
        let right = drag_rect(
            r,
            Edges {
                right: true,
                ..Default::default()
            },
            vec2(500.0, 99.0),
        );
        assert_eq!(
            (right.width(), right.height(), right.min),
            (800.0, 200.0, r.min)
        );
        let left = drag_rect(
            r,
            Edges {
                left: true,
                ..Default::default()
            },
            vec2(-60.0, 0.0),
        );
        assert_eq!((left.min.x, left.max.x), (40.0, 400.0));
        let top = drag_rect(
            r,
            Edges {
                top: true,
                ..Default::default()
            },
            vec2(0.0, -500.0),
        );
        assert_eq!((top.min.y, top.max.y), (0.0, 300.0));
        let corner = drag_rect(
            r,
            Edges {
                right: true,
                bottom: true,
                ..Default::default()
            },
            vec2(10.0, 400.0),
        );
        assert_eq!((corner.width(), corner.height()), (310.0, 600.0));
        let tiny = drag_rect(
            r,
            Edges {
                right: true,
                bottom: true,
                ..Default::default()
            },
            vec2(-900.0, -900.0),
        );
        assert_eq!(tiny.size(), MIN_SIZE);
        let moved = drag_rect(r, Edges::BODY, vec2(-500.0, 40.0));
        assert_eq!((moved.min, moved.size()), (pos2(0.0, 140.0), r.size()));
    }

    // REQ SQ-29: the canvas holds sized widgets where they are and stacks unsized ones below.
    #[test]
    fn canvas_places_sized_widgets_and_reports_its_total_size() {
        let mut widgets = sq_core::Config::default().widgets;
        widgets.truncate(3);
        assert!(!is_free(&widgets));
        let stacked = free_rects(&widgets, 10.0);
        assert_eq!(stacked.len(), 3);
        assert!(stacked[1].1.min.y > stacked[0].1.bottom());
        widgets[0].width = 700.0;
        widgets[0].x = 1_000.0;
        widgets[0].y = 50.0;
        widgets[0].height = 300.0;
        assert!(is_free(&widgets));
        let rects = free_rects(&widgets, 10.0);
        assert_eq!(
            rects[0].1,
            Rect::from_min_size(pos2(1_000.0, 50.0), vec2(700.0, 300.0))
        );
        assert!(rects[1].1.min.y >= 350.0);
        assert_eq!(canvas_size(&rects).x, 1_700.0);
        freeze(&mut widgets, &rects);
        assert!(widgets.iter().all(|w| w.width > 0.0));
        assert_eq!(free_rects(&widgets, 10.0), rects);
    }

    // REQ SQ-29: borders stick at the one configured gap, flush edges and equal sizes.
    #[test]
    fn borders_and_sizes_stick_to_other_widgets() {
        let other = Rect::from_min_size(pos2(500.0, 0.0), vec2(300.0, 200.0));
        let others = [other];
        let right = Edges {
            right: true,
            ..Default::default()
        };
        // Right border 6 px short of the neighbour's left border minus the gap: settles on it.
        let r = Rect::from_min_size(pos2(0.0, 20.0), vec2(484.0, 100.0));
        assert_eq!(snap_rect(r, right, &others, 12.0).max.x, 488.0);
        // Far from everything: untouched.
        let r = Rect::from_min_size(pos2(0.0, 20.0), vec2(200.0, 100.0));
        assert_eq!(snap_rect(r, right, &others, 12.0), r);
        // Width within 10 px of the neighbour's width becomes exactly that width.
        let r = Rect::from_min_size(pos2(0.0, 300.0), vec2(293.0, 100.0));
        assert_eq!(snap_rect(r, right, &others, 12.0).width(), 300.0);
        // Height likewise.
        let bottom = Edges {
            bottom: true,
            ..Default::default()
        };
        let r = Rect::from_min_size(pos2(0.0, 300.0), vec2(100.0, 194.0));
        assert_eq!(snap_rect(r, bottom, &others, 12.0).height(), 200.0);
        // Left border settles one gap after the neighbour's right border.
        let left = Edges {
            left: true,
            ..Default::default()
        };
        let r = Rect::from_min_size(pos2(805.0, 20.0), vec2(300.0, 100.0));
        assert_eq!(snap_rect(r, left, &others, 12.0).min.x, 812.0);
        // Moving: the left side settles one gap after it, and the top aligns with it.
        let r = Rect::from_min_size(pos2(806.0, 4.0), vec2(300.0, 100.0));
        let moved = snap_rect(r, Edges::BODY, &others, 12.0);
        assert_eq!((moved.min.x, moved.min.y), (812.0, 0.0));
        assert_eq!(moved.size(), r.size());
    }
}
