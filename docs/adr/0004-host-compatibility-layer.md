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
* Windows brings a window to the front at every click, and the window level alone is applied only when it changes. The Windows host therefore wraps the window procedure and answers every reordering request with "bottom" while the layer is Behind (`host/windows.rs`); the hold is lifted before another layer is applied.
* A floating window is an ordinary window: with it the Behind layer works like Normal, on every system (`native::level_for`). Held behind, a titled window could never be brought to the front.
* Windows: the first position and every restored position stay inside the free part of the screen, above the taskbar (`host::windows::work_area`, `native::fitted`).
* Windows without usable OpenGL (virtual machines, remote sessions) draws through DirectX, which cannot make the window see-through: the background is then drawn solid and Customize says that Background opacity has no effect there.
* Windows, part of the desktop (2026-11-19, replaces the earlier known limit that *Show desktop* hid the panel): the frameless panel is a tool window (not in Alt+Tab) and, in the Behind layer, is owned by the desktop window, so *Show desktop* (Win+D) leaves it in place. "The desktop window" is the one that holds the desktop icons at that moment: `Progman` normally and on Windows 11; on Windows 10 *Show desktop* moves the icons into a full-screen `WorkerW` window placed in front of every ordinary window, so the owner is looked up again whenever the active window changes or a window is minimised or restored (`icon_host`, seen on Windows 10 LTSC 2021 on 2026-11-19), and while that raised window owns the panel every restacking request is answered with the place directly in front of it instead of the bottom (`stack_place`), so a window opened during *Show desktop* still comes in front of the panel; after a restart of Windows Explorer it is given to the new desktop window (`TaskbarCreated` message). Windows 11 does not restore the other windows at the second Win+D when the panel is the active window, so the panel never becomes active by a click (`WM_MOUSEACTIVATE` answered with 'do not activate'); it takes the keyboard only when a text field was clicked within the last 1.5 seconds, and gives it back to the desktop when the field is left, when the Windows key goes down and when another window is minimised. In the Normal layer a click raises it without activating it. Rejected: drawing behind the desktop icons (the `WorkerW` trick), which is undocumented and breaks with Explorer updates.
* Windows: the panel attaches to the console of the terminal only for commands that print (SQ-49). Attached always, as before, it died when the terminal window that started it was closed.
* The saved place is the outer position of the window and the size of its content (SQ-50, `native::saved_place`); saving the content position moved a titled window by its title bar at every start.

