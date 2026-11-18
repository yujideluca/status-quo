# ADR-0004: Host compatibility layer and widget-environment surfaces

- Status: accepted
- Date: 2026-11-02
- Type: corrective (amends ADR-0002 consequence "standalone desktop panel")
- Decision: Giovanni, 2026-11-02: Status Quo must be a desktop widget environment (small always-visible panels on the desktop background or edges, stacked behind or above windows) and stay portable across Linux versions; environment differences belong in a compatibility layer.

## Context

ADR-0002 delivered a framed window. A framed window is not a widget environment, and Wayland compositors differ: KDE Plasma, Hyprland and sway offer `wlr-layer-shell`; GNOME on Wayland does not; X11 sessions use window-manager hints.

## Decision

Add `crates/desktop/src/host/` as the only place that knows how a surface is created, anchored and stacked. It selects at run time, with no build-time desktop dependency:

1. **layer-shell**: a Wayland layer-shell surface drawn by egui on wgpu, below windows by default, anchored top-right (`STATUS_QUO_ANCHOR`), pointer input, frames only on demand.
2. **x11-desktop**: a frameless window plus `_NET_WM_STATE` below, skip-taskbar and skip-pager hints (sticky requested, best effort), verified by reading the property back.
3. **window**: the ordinary framed window; used when neither is available and for `--screenshot` and `--customize` (the layer host has no keyboard input yet).

`STATUS_QUO_HOST=layer-shell|x11|window` forces a backend when it can work; otherwise the layer falls back and the note says so. Widgets, tray, contract, core and collector are unchanged; the boundary checker still keeps window code out of them.

## Alternatives

A native Plasma plasmoid would tie the project to KDE (rejected in ADR-0002). Forcing X11 hints everywhere would be wrong on Wayland. Software shared-memory rendering would avoid the GPU stack but needs its own rasterizer for egui.

## Consequences

Memory grows by the GPU driver libraries (panel about 155 MB resident, mostly shared library pages). Layer-shell has no keyboard input yet. Other distributions and desktops stay unverified until checked on a real host; the selection logic is covered by tests that need no desktop.

## Amendment, 2026-11-18 (SQ-48)

The plain window stops being the fallback people meet. Giovanni reported that the tray opened as a floating window on Windows; the decision is that the desktop background is the default everywhere and the floating window is a user setting.

* `host::select` takes the `floating` setting of the configuration. When it is on, the host is the plain window on every system.
* Windows has its own host, `windows-desktop`: a window without frame and without a taskbar button, kept below the others by the window level.
* Wayland without layer-shell (GNOME) uses the X11 host through XWayland instead of the plain window. Mutter accepts the same hints as an X11 session.
* The host is fixed when the window is created, so a change of the setting restarts the panel. On Linux the process replaces itself (`exec`), which keeps the process the login service watches. On Windows a new panel starts, waits for the panel lock, and the old one closes.
* Rejected: changing decorations and stacking of the live window (the layer-shell surface and the framed window are created by different event loops, and X11 hints are applied once after mapping); making the floating window a per-platform default (the report was exactly that default).
* Known limit: on Windows the *Show desktop* shortcut (Win+D) hides the panel like any window until a window is activated again. Drawing behind the desktop icons would avoid it and is not done.

