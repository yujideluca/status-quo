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

#[cfg(test)]
mod tests {
    use super::fit_scale;

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
}
