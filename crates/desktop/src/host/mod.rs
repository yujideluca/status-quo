//! Compatibility layer: picks how the tray is placed on this desktop (Linux and Windows).
//!
//! Widgets, tray and collector never see this module. A backend only decides how
//! one drawing surface is created, anchored, stacked and reported.
#[cfg(target_os = "linux")]
pub mod layer;
#[cfg(windows)]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod x11;

use sq_core::Layer;

/// How the tray is shown on the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Wayland layer-shell surface: borderless, anchored to an edge, below or above windows.
    LayerShell,
    /// X11 window without frame, kept below other windows, on every workspace, off the taskbar.
    X11Desktop,
    /// Windows: window without frame or taskbar button, kept below other windows.
    WindowsDesktop,
    /// Ordinary framed window. Works everywhere. Used when the user asks for a floating
    /// window, and where nothing else is possible.
    Window,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::LayerShell => "layer-shell",
            Backend::X11Desktop => "x11-desktop",
            Backend::WindowsDesktop => "windows-desktop",
            Backend::Window => "window",
        }
    }
    pub fn parse(text: &str) -> Option<Backend> {
        match text.trim().to_ascii_lowercase().as_str() {
            "layer" | "layer-shell" | "wayland" => Some(Backend::LayerShell),
            "x11" | "x11-desktop" => Some(Backend::X11Desktop),
            "windows-desktop" | "windows" => Some(Backend::WindowsDesktop),
            "window" | "plain" | "floating" => Some(Backend::Window),
            _ => None,
        }
    }
}

/// What the running session offers. Filled from the real session, or by tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    /// A Wayland compositor answered and advertises `zwlr_layer_shell_v1`.
    pub layer_shell: bool,
    /// An X display can be opened.
    pub x11_display: bool,
    /// The session itself is X11 (not Wayland with XWayland).
    pub x11_session: bool,
    /// This is Windows: a frameless window can be kept below the others.
    pub windows: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub backend: Backend,
    /// Plain-language statement of what was chosen and what is not available.
    pub note: String,
}

/// Choose a backend. `forced` comes from `STATUS_QUO_HOST` and wins when it can work.
/// The tray sits on the desktop background wherever the system allows it; `floating` is the
/// user's setting that asks for an ordinary window instead (SQ-48).
pub fn select(forced: Option<Backend>, probe: &Probe, floating: bool) -> Selection {
    let possible = |backend: Backend| match backend {
        Backend::LayerShell => probe.layer_shell,
        Backend::X11Desktop => probe.x11_display,
        Backend::WindowsDesktop => probe.windows,
        Backend::Window => true,
    };
    let mut prefix = String::new();
    if let Some(backend) = forced {
        if possible(backend) {
            return Selection {
                backend,
                note: format!("{}: chosen by STATUS_QUO_HOST.", backend.name()),
            };
        }
        prefix = format!("STATUS_QUO_HOST={} is not available here; ", backend.name());
    }
    let (backend, why) = if floating {
        (
            Backend::Window,
            "the Floating window setting is on (Customize, Desktop behavior).",
        )
    } else if probe.layer_shell {
        (
            Backend::LayerShell,
            "the compositor offers layer-shell, so widgets sit on the desktop layer.",
        )
    } else if probe.x11_display && probe.x11_session {
        (
            Backend::X11Desktop,
            "X11 session: frameless window kept below others, on every workspace.",
        )
    } else if probe.x11_display {
        (
            Backend::X11Desktop,
            "Wayland without layer-shell (such as GNOME): frameless window kept below others, through XWayland.",
        )
    } else if probe.windows {
        (
            Backend::WindowsDesktop,
            "Windows: frameless panel on the desktop, below other windows, not in the taskbar or Alt+Tab.",
        )
    } else {
        (
            Backend::Window,
            "no layer-shell and no X display: ordinary window; it cannot be placed on the desktop background here.",
        )
    };
    Selection {
        backend,
        note: format!("{prefix}{}: {why}", backend.name()),
    }
}

/// Read the real session. Never fails: a missing piece only turns a capability off.
#[cfg(not(target_os = "linux"))]
pub fn probe_session() -> Probe {
    // Windows has neither layer-shell nor X11; its own desktop backend is used.
    Probe {
        windows: cfg!(windows),
        ..Probe::default()
    }
}
#[cfg(target_os = "linux")]
pub fn probe_session() -> Probe {
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    Probe {
        layer_shell: std::env::var_os("WAYLAND_DISPLAY").is_some() && layer::available(),
        x11_display: std::env::var_os("DISPLAY").is_some() && x11rb::connect(None).is_ok(),
        x11_session: session.eq_ignore_ascii_case("x11"),
        windows: false,
    }
}

pub fn forced_backend() -> Option<Backend> {
    std::env::var("STATUS_QUO_HOST")
        .ok()
        .and_then(|v| Backend::parse(&v))
}

/// Edge the layer-shell surface attaches to; two edges make a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Anchor {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

/// Parse text such as `top-right`, `left` or `bottom-left`. Unknown words are rejected.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_anchor(text: &str) -> Option<Anchor> {
    let mut anchor = Anchor::default();
    let mut any = false;
    for word in text.split(['-', ' ', ',']).filter(|w| !w.is_empty()) {
        match word.to_ascii_lowercase().as_str() {
            "top" => anchor.top = true,
            "bottom" => anchor.bottom = true,
            "left" => anchor.left = true,
            "right" => anchor.right = true,
            _ => return None,
        }
        any = true;
    }
    if !any || (anchor.top && anchor.bottom && !anchor.left && !anchor.right) {
        return None;
    }
    Some(anchor)
}

/// Layer-shell has Background, Bottom, Top and Overlay. "Behind" and "Normal" both map to
/// Bottom (above wallpaper, below windows); "Above" maps to Top.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn layer_for(layer: Layer) -> LayerPlacement {
    match layer {
        Layer::Behind | Layer::Normal => LayerPlacement::Bottom,
        Layer::Above => LayerPlacement::Top,
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerPlacement {
    Bottom,
    Top,
}

/// Size the surface asks for, from saved geometry, kept to a sane range.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn surface_size(saved: Option<(f32, f32)>) -> (u32, u32) {
    let (w, h) = saved.unwrap_or((440.0, 900.0));
    (w.clamp(240.0, 1600.0) as u32, h.clamp(160.0, 2400.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(layer_shell: bool, x11_display: bool, x11_session: bool, windows: bool) -> Probe {
        Probe {
            layer_shell,
            x11_display,
            x11_session,
            windows,
        }
    }

    // REQ SQ-26 SQ-22: backend choice follows the real session and reports fallbacks.
    #[test]
    fn layer_shell_is_preferred_when_the_compositor_offers_it() {
        let chosen = select(None, &probe(true, true, false, false), false);
        assert_eq!(chosen.backend, Backend::LayerShell);
        assert!(chosen.note.contains("layer-shell"));
    }

    // REQ SQ-48 SQ-26: without the Floating window setting, every system that can place the
    // tray on the desktop background does so; an ordinary window is only the last resort.
    #[test]
    fn the_desktop_background_is_the_default_on_every_system() {
        let x11 = probe(false, true, true, false);
        assert_eq!(select(None, &x11, false).backend, Backend::X11Desktop);
        let gnome_wayland = probe(false, true, false, false);
        let chosen = select(None, &gnome_wayland, false);
        assert_eq!(chosen.backend, Backend::X11Desktop);
        assert!(chosen.note.contains("XWayland"));
        let windows = probe(false, false, false, true);
        let chosen = select(None, &windows, false);
        assert_eq!(chosen.backend, Backend::WindowsDesktop);
        assert!(chosen.note.contains("not in the taskbar or Alt+Tab"));
        let nothing = select(None, &Probe::default(), false);
        assert_eq!(nothing.backend, Backend::Window);
        assert!(nothing
            .note
            .contains("cannot be placed on the desktop background"));
    }

    // REQ SQ-48: the Floating window setting gives an ordinary window on every system.
    #[test]
    fn the_floating_setting_gives_an_ordinary_window_everywhere() {
        for session in [
            probe(true, true, false, false),
            probe(false, true, true, false),
            probe(false, true, false, false),
            probe(false, false, false, true),
            Probe::default(),
        ] {
            let chosen = select(None, &session, true);
            assert_eq!(chosen.backend, Backend::Window, "{session:?}");
            assert!(chosen.note.contains("Floating window setting"));
        }
    }

    // REQ SQ-26: a forced choice that cannot work falls back and says so.
    #[test]
    fn forced_backend_is_honored_only_when_possible() {
        let session = probe(false, true, false, false);
        assert_eq!(
            select(Some(Backend::X11Desktop), &session, false).backend,
            Backend::X11Desktop
        );
        assert_eq!(
            select(Some(Backend::Window), &session, false).backend,
            Backend::Window
        );
        let fallback = select(Some(Backend::LayerShell), &session, false);
        assert_eq!(fallback.backend, Backend::X11Desktop);
        assert!(fallback.note.contains("is not available here"));
        let fallback = select(Some(Backend::WindowsDesktop), &session, false);
        assert_eq!(fallback.backend, Backend::X11Desktop);
        assert_eq!(Backend::parse("Layer-Shell"), Some(Backend::LayerShell));
        assert_eq!(
            Backend::parse("windows-desktop"),
            Some(Backend::WindowsDesktop)
        );
        assert_eq!(Backend::parse("floating"), Some(Backend::Window));
        assert_eq!(Backend::parse("nonsense"), None);
    }

    // REQ SQ-27: anchors, layers and sizes are validated before they reach the compositor.
    #[test]
    fn anchor_layer_and_size_are_validated() {
        let corner = parse_anchor("top-right").unwrap();
        assert!(corner.top && corner.right && !corner.left && !corner.bottom);
        assert!(parse_anchor("middle").is_none());
        assert!(parse_anchor("").is_none());
        assert!(parse_anchor("top-bottom").is_none());
        assert_eq!(layer_for(Layer::Behind), LayerPlacement::Bottom);
        assert_eq!(layer_for(Layer::Above), LayerPlacement::Top);
        assert_eq!(surface_size(Some((10.0, 99999.0))), (240, 2400));
        assert_eq!(surface_size(None), (440, 900));
    }
}
