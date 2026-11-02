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
