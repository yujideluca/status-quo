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

fn near(value: f32, allowed: (f32, f32), candidates: impl IntoIterator<Item = f32>) -> Option<f32> {
    candidates
        .into_iter()
        .filter(|c| (c - value).abs() <= SNAP && *c >= allowed.0 && *c <= allowed.1)
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
        let shift = |lefts: Vec<f32>, rights: Vec<f32>, centers: Vec<f32>, min: f32, max: f32| {
            lefts
                .into_iter()
                .map(|c| c - min)
                .chain(rights.into_iter().map(|c| c - max))
                .chain(centers.into_iter().map(|c| c - (min + max) / 2.0))
                .filter(|s| s.abs() <= SNAP && min + s >= 0.0)
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
            others.iter().map(|o| o.center().x).collect(),
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
            others.iter().map(|o| o.center().y).collect(),
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
        let centers = others.iter().map(|o| 2.0 * o.center().x - rect.max.x);
        if let Some(x) = near(rect.min.x, (0.0, rect.max.x - MIN_SIZE.x), edges_at)
            .or_else(|| near(rect.min.x, (0.0, rect.max.x - MIN_SIZE.x), centers))
            .or_else(|| near(rect.min.x, (0.0, rect.max.x - MIN_SIZE.x), sizes))
        {
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
        let centers = others.iter().map(|o| 2.0 * o.center().x - rect.min.x);
        if let Some(x) = near(rect.max.x, (rect.min.x + MIN_SIZE.x, f32::MAX), edges_at)
            .or_else(|| near(rect.max.x, (rect.min.x + MIN_SIZE.x, f32::MAX), centers))
            .or_else(|| near(rect.max.x, (rect.min.x + MIN_SIZE.x, f32::MAX), sizes))
        {
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
        let centers = others.iter().map(|o| 2.0 * o.center().y - rect.max.y);
        if let Some(y) = near(rect.min.y, (0.0, rect.max.y - MIN_SIZE.y), edges_at)
            .or_else(|| near(rect.min.y, (0.0, rect.max.y - MIN_SIZE.y), centers))
            .or_else(|| near(rect.min.y, (0.0, rect.max.y - MIN_SIZE.y), sizes))
        {
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
        let centers = others.iter().map(|o| 2.0 * o.center().y - rect.min.y);
        if let Some(y) = near(rect.max.y, (rect.min.y + MIN_SIZE.y, f32::MAX), edges_at)
            .or_else(|| near(rect.max.y, (rect.min.y + MIN_SIZE.y, f32::MAX), centers))
            .or_else(|| near(rect.max.y, (rect.min.y + MIN_SIZE.y, f32::MAX), sizes))
        {
            r.max.y = y.max(r.min.y + MIN_SIZE.y);
        }
    }
    r
}

/// What a highlighted line says about the dragged widget and one other widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideKind {
    /// Two borders sit on one line, or one gap apart.
    Edge,
    /// The two widgets share a centre axis.
    Center,
    /// The two widgets have the same width.
    Width,
    /// The two widgets have the same height.
    Height,
}
/// A highlighted line segment in canvas coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    pub kind: GuideKind,
    pub from: egui::Pos2,
    pub to: egui::Pos2,
}

/// Lines to highlight for a settled rectangle: every border or centre axis of the dragged
/// widget that lines up exactly with another widget, and equal widths or heights. Only what
/// the drag moves is reported (a move reports alignment, not size), so the lines are exactly
/// the points `snap_rect` makes sticky.
pub fn guides(rect: egui::Rect, edges: Edges, others: &[egui::Rect], gap: f32) -> Vec<Guide> {
    const TOLERANCE: f32 = 0.5;
    let same = |a: f32, b: f32| (a - b).abs() < TOLERANCE;
    let moving = edges == Edges::BODY;
    let horizontal = edges.left || edges.right;
    let vertical = edges.top || edges.bottom;
    let mut out = Vec::new();
    let vline = |out: &mut Vec<Guide>, kind, x: f32, o: &egui::Rect| {
        out.push(Guide {
            kind,
            from: egui::pos2(x, rect.min.y.min(o.min.y)),
            to: egui::pos2(x, rect.max.y.max(o.max.y)),
        });
    };
    let hline = |out: &mut Vec<Guide>, kind, y: f32, o: &egui::Rect| {
        out.push(Guide {
            kind,
            from: egui::pos2(rect.min.x.min(o.min.x), y),
            to: egui::pos2(rect.max.x.max(o.max.x), y),
        });
    };
    for o in others {
        if edges.left
            && (same(rect.min.x, o.min.x)
                || same(rect.min.x, o.max.x + gap)
                || same(rect.min.x, o.max.x))
        {
            vline(&mut out, GuideKind::Edge, rect.min.x, o);
        }
        if edges.right
            && (same(rect.max.x, o.max.x)
                || same(rect.max.x, o.min.x - gap)
                || same(rect.max.x, o.min.x))
        {
            vline(&mut out, GuideKind::Edge, rect.max.x, o);
        }
        if edges.top
            && (same(rect.min.y, o.min.y)
                || same(rect.min.y, o.max.y + gap)
                || same(rect.min.y, o.max.y))
        {
            hline(&mut out, GuideKind::Edge, rect.min.y, o);
        }
        if edges.bottom
            && (same(rect.max.y, o.max.y)
                || same(rect.max.y, o.min.y - gap)
                || same(rect.max.y, o.min.y))
        {
            hline(&mut out, GuideKind::Edge, rect.max.y, o);
        }
        if (moving || horizontal) && same(rect.center().x, o.center().x) {
            vline(&mut out, GuideKind::Center, rect.center().x, o);
        }
        if (moving || vertical) && same(rect.center().y, o.center().y) {
            hline(&mut out, GuideKind::Center, rect.center().y, o);
        }
        if !moving && horizontal && same(rect.width(), o.width()) {
            for r in [rect, *o] {
                out.push(Guide {
                    kind: GuideKind::Width,
                    from: egui::pos2(r.min.x, r.max.y),
                    to: egui::pos2(r.max.x, r.max.y),
                });
            }
        }
        if !moving && vertical && same(rect.height(), o.height()) {
            for r in [rect, *o] {
                out.push(Guide {
                    kind: GuideKind::Height,
                    from: egui::pos2(r.max.x, r.min.y),
                    to: egui::pos2(r.max.x, r.max.y),
                });
            }
        }
    }
    out
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

    fn kinds(g: &[Guide]) -> Vec<GuideKind> {
        g.iter().map(|g| g.kind).collect()
    }
    fn left() -> Edges {
        Edges {
            left: true,
            ..Default::default()
        }
    }
    fn right() -> Edges {
        Edges {
            right: true,
            ..Default::default()
        }
    }
    fn top() -> Edges {
        Edges {
            top: true,
            ..Default::default()
        }
    }
    fn bottom() -> Edges {
        Edges {
            bottom: true,
            ..Default::default()
        }
    }

    // REQ SQ-34: moving a widget sticks its centre to another widget's centre axis, both ways.
    #[test]
    fn a_moved_widget_sticks_to_the_centre_axes_of_others() {
        let other = Rect::from_min_size(pos2(400.0, 400.0), vec2(300.0, 200.0));
        // Horizontal centre: other centre x is 550; ours is 544 -> shifts 6 right.
        let r = Rect::from_min_size(pos2(444.0, 100.0), vec2(200.0, 100.0));
        let moved = snap_rect(r, Edges::BODY, &[other], 12.0);
        assert_eq!(moved.center().x, 550.0);
        assert_eq!(moved.min.y, 100.0, "y untouched");
        assert!(kinds(&guides(moved, Edges::BODY, &[other], 12.0)).contains(&GuideKind::Center));
        // Vertical centre: other centre y is 500; ours is 493.
        let r = Rect::from_min_size(pos2(100.0, 443.0), vec2(100.0, 100.0));
        let moved = snap_rect(r, Edges::BODY, &[other], 12.0);
        assert_eq!(moved.center().y, 500.0);
        assert_eq!(moved.min.x, 100.0, "x untouched");
        // Beyond the snap distance nothing happens.
        let r = Rect::from_min_size(pos2(444.0 - 11.0, 100.0), vec2(200.0, 100.0));
        assert_eq!(snap_rect(r, Edges::BODY, &[other], 12.0), r);
    }

    // REQ SQ-34: resizing a border can put the widget's centre on another widget's axis.
    #[test]
    fn a_resized_border_sticks_to_make_the_centres_line_up() {
        let other = Rect::from_min_size(pos2(0.0, 300.0), vec2(400.0, 200.0));
        // Our box is 0..300 wide; the right border at 396 would centre it at 198; need 400.
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(396.0, 100.0));
        // Right border near 400 is also this widget's equal-width point; both agree.
        assert_eq!(snap_rect(r, right(), &[other], 12.0).max.x, 400.0);
        // Centre only: ours starts at 100, other's centre x is 200 -> right border to 300.
        let r = Rect::from_min_size(pos2(100.0, 0.0), vec2(205.0, 100.0));
        let s = snap_rect(r, right(), &[other], 12.0);
        assert_eq!(s.center().x, 200.0);
        assert_eq!(s.max.x, 300.0);
        // Left border: right at 300, other centre 200 -> left to 100.
        let r = Rect::from_min_size(pos2(105.0, 0.0), vec2(195.0, 100.0));
        let s = snap_rect(r, left(), &[other], 12.0);
        assert_eq!(s.center().x, 200.0);
        // Vertical: bottom border and top border.
        let other = Rect::from_min_size(pos2(500.0, 0.0), vec2(100.0, 200.0));
        let r = Rect::from_min_size(pos2(0.0, 50.0), vec2(100.0, 105.0));
        let s = snap_rect(r, bottom(), &[other], 12.0);
        assert_eq!(s.center().y, 100.0);
        let r = Rect::from_min_size(pos2(0.0, 40.0), vec2(100.0, 115.0));
        let s = snap_rect(r, top(), &[other], 12.0);
        assert_eq!(s.center().y, 100.0);
    }

    // REQ SQ-34: every kind of line is reported for the border or axis that causes it.
    #[test]
    fn guides_name_the_edge_centre_and_size_matches() {
        let other = Rect::from_min_size(pos2(500.0, 100.0), vec2(300.0, 200.0));
        // Edge flush with the other's left border.
        let r = Rect::from_min_size(pos2(500.0, 400.0), vec2(300.0, 100.0));
        let g = guides(r, Edges::BODY, &[other], 12.0);
        assert!(g
            .iter()
            .any(|g| g.kind == GuideKind::Edge && g.from.x == 500.0));
        assert!(g
            .iter()
            .any(|g| g.kind == GuideKind::Edge && g.from.x == 800.0));
        assert!(g
            .iter()
            .any(|g| g.kind == GuideKind::Center && g.from.x == 650.0));
        // A move never reports sizes.
        assert!(!kinds(&g).contains(&GuideKind::Width));
        // Bottom border one gap above the other's top: edge line at that border.
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 88.0));
        let g = guides(r, bottom(), &[other], 12.0);
        assert!(g
            .iter()
            .any(|g| g.kind == GuideKind::Edge && g.from.y == 88.0));
        // Same width: two bars, one under each widget.
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 50.0));
        let g = guides(r, right(), &[other], 12.0);
        let bars: Vec<_> = g.iter().filter(|g| g.kind == GuideKind::Width).collect();
        assert_eq!(bars.len(), 2);
        assert!(bars.iter().all(|b| b.to.x - b.from.x == 300.0));
        // Same height.
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(80.0, 200.0));
        let g = guides(r, bottom(), &[other], 12.0);
        assert_eq!(g.iter().filter(|g| g.kind == GuideKind::Height).count(), 2);
        // A border that is not being dragged is not reported even if it lines up.
        let r = Rect::from_min_size(pos2(500.0, 0.0), vec2(300.0, 50.0));
        let g = guides(r, bottom(), &[other], 12.0);
        assert!(!g
            .iter()
            .any(|g| g.kind == GuideKind::Edge && g.from.x == 500.0));
        // Far from everything: no lines.
        let r = Rect::from_min_size(pos2(1000.0, 1000.0), vec2(77.0, 55.0));
        assert!(guides(r, Edges::BODY, &[other], 12.0).is_empty());
        assert!(guides(r, right(), &[], 12.0).is_empty());
    }

    // REQ SQ-34: guides join widgets that overlap on the other axis with a line spanning both.
    #[test]
    fn a_guide_spans_both_widgets() {
        let other = Rect::from_min_size(pos2(0.0, 300.0), vec2(100.0, 100.0));
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
        let g = guides(r, left(), &[other], 12.0);
        let line = g.iter().find(|g| g.kind == GuideKind::Edge).unwrap();
        assert_eq!((line.from.y, line.to.y), (0.0, 400.0));
    }

    // REQ SQ-34: sticky points are consistent, over thousands of random drags.
    #[test]
    fn snapping_is_deterministic_bounded_and_always_explained_by_a_guide() {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move |max: f32| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % 10_000) as f32 / 10_000.0 * max
        };
        let all = [
            Edges::BODY,
            left(),
            right(),
            top(),
            bottom(),
            Edges {
                right: true,
                bottom: true,
                ..Default::default()
            },
            Edges {
                left: true,
                top: true,
                ..Default::default()
            },
        ];
        let mut stuck = 0;
        for round in 0..6_000 {
            let others: Vec<Rect> = (0..1 + round % 4)
                .map(|_| {
                    Rect::from_min_size(
                        pos2(next(900.0).round(), next(900.0).round()),
                        vec2(
                            MIN_SIZE.x + next(500.0).round(),
                            MIN_SIZE.y + next(400.0).round(),
                        ),
                    )
                })
                .collect();
            let edges = all[round % all.len()];
            let gap = [0.0, 8.0, 12.0, 40.0][round % 4];
            let r = Rect::from_min_size(
                pos2(next(900.0).round(), next(900.0).round()),
                vec2(
                    MIN_SIZE.x + next(500.0).round(),
                    MIN_SIZE.y + next(400.0).round(),
                ),
            );
            let s = snap_rect(r, edges, &others, gap);
            assert!(s.min.x >= 0.0 && s.min.y >= 0.0, "left the canvas: {s:?}");
            assert!(
                s.width() >= MIN_SIZE.x - 0.01 && s.height() >= MIN_SIZE.y - 0.01,
                "smaller than the minimum: {s:?} from {r:?}"
            );
            if edges == Edges::BODY {
                assert!((s.width() - r.width()).abs() < 0.01, "a move resized");
                assert!((s.height() - r.height()).abs() < 0.01, "a move resized");
            }
            // Nothing moves further than the snap distance (plus the canvas clamp).
            let travel = |a: f32, b: f32| (a - b).abs();
            for (a, b) in [
                (s.min.x, r.min.x),
                (s.max.x, r.max.x),
                (s.min.y, r.min.y),
                (s.max.y, r.max.y),
            ] {
                if r.min.x >= SNAP && r.min.y >= SNAP {
                    assert!(travel(a, b) <= SNAP * 2.0 + 0.01, "jumped {a} from {b}");
                }
            }
            // The same drag always lands on the same point.
            assert_eq!(s, snap_rect(r, edges, &others, gap));
            // Whenever the rectangle moved, a line explains why.
            if (s.min - r.min).length() > 0.01 || (s.max - r.max).length() > 0.01 {
                stuck += 1;
                assert!(
                    !guides(s, edges, &others, gap).is_empty(),
                    "stuck without a line: {r:?} -> {s:?} ({edges:?}, gap {gap}, {others:?})"
                );
            }
        }
        assert!(
            stuck > 300,
            "the random drags barely exercised snapping: {stuck}"
        );
    }
}
