# ADR-0005: Free canvas layout and a tray that follows its widgets

- Status: accepted
- Date: 2026-11-02
- Type: corrective (amends the row/column flow of ADR-0002's tray and the fixed surface of ADR-0004)
- Decision: Giovanni, 2026-11-02: Edit tray must allow any size on both axes, the tray must grow with the widgets up to the whole desktop, and widgets must adapt their display to their size.

## Context

The first Edit tray kept the flow layout: a width of one or two columns, heights scaled so rows fill a fixed-size surface. That cannot express arbitrary width, free placement or a tray larger than its surface.

## Decision

1. Each widget instance may carry an explicit rectangle (`x`, `y`, `width`, `height`, pixels). `width == 0` keeps the old span flow, so old configurations load unchanged. Limits: width 120 to 8000, height 60 to 8000, position 0 to 20000.
2. Entering Edit tray freezes what is on screen into explicit rectangles, so nothing moves; Cancel discards them. Done normalizes the canvas to its corner and records the corner's desktop position in `geometry.tray`.
3. In the editor every border and corner resizes freely and the body moves freely (`layout::drag_rect`, portable and tested). Widget content is disabled while editing so it cannot take the pointer from the handles.
4. The tray reports `desired_size()` (canvas plus header). The layer-shell host docks the surface top-left at the saved position and resizes it to that size; while editing it covers the whole output above windows so widgets can be dragged anywhere (`host::layer::Surface`). Window hosts follow with an inner-size request.
5. Widgets learn their room through egui temporary data: charts take their height from it, tokens controls wrap, meter text wraps.

## Alternatives

Keeping the flow and only widening the span allowed would not give free placement. A fixed full-screen transparent surface at all times would capture pointer input over the whole desktop and was rejected. Per-monitor placement is not handled: the first output defines the screen.

## Consequences

The surface is recreated at a different size on every docked resize. Overlapping widgets are allowed. Content adaptation is heuristic (chart height is about 55 percent of the widget's room); other widget kinds only wrap. Editing is verified by simulated pointer events and by a real-compositor run of a docked free layout; a real-pointer drag in the full-screen editor on the live desktop is not yet observed.

## Update 2026-11-02: one gap, sticky borders, lock, new widgets

- **One gap:** `Config.gap` (0 to 64 pixels, default 10, Customize slider "Space between widgets") is the only space between blocks: the flow layout, the stacking of unsized widgets and the editor use it.
- **Sticky borders:** while dragging, the unsnapped rectangle follows the pointer and the displayed one is snapped (`layout::snap_rect`, distance 10 pixels), so a widget sticks and lets go again. A border settles exactly one gap from a neighbour's border, or flush with an aligned one; a width or height near another widget's becomes exactly that size. Gap and alignment win over size when both are near.
- **Lock:** the Edit tray button is disabled while the layout is locked.
- **New widgets:** in a free canvas a new widget gets an explicit rectangle at the top-left corner, so it is visible; a first-run layout taller than its surface is compressed to fit when editing starts.
- **Host:** the docked tray is kept on the screen (`clamp_to_screen`) and the exit flag is polled every 200 ms.
