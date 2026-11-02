//! Compatibility layer: picks how the tray is placed on this Linux desktop.
//!
//! Widgets, tray and collector never see this module. A backend only decides how
//! one drawing surface is created, anchored, stacked and reported.
pub mod layer;
pub mod x11;

use sq_core::Layer;

/// How the tray is shown on the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Wayland layer-shell surface: borderless, anchored to an edge, below or above windows.
    LayerShell,
    /// X11 window without frame, kept below other windows, on every workspace, off the taskbar.
    X11Desktop,
    /// Ordinary framed window. Works everywhere; cannot sit behind other windows reliably.
    Window,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::LayerShell => "layer-shell",
            Backend::X11Desktop => "x11-desktop",
            Backend::Window => "window",
        }
    }
    pub fn parse(text: &str) -> Option<Backend> {
        match text.trim().to_ascii_lowercase().as_str() {
            "layer" | "layer-shell" | "wayland" => Some(Backend::LayerShell),
            "x11" | "x11-desktop" => Some(Backend::X11Desktop),
            "window" | "plain" => Some(Backend::Window),
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub backend: Backend,
    /// Plain-language statement of what was chosen and what is not available.
    pub note: String,
}

/// Choose a backend. `forced` comes from `STATUS_QUO_HOST` and wins when it can work.
pub fn select(forced: Option<Backend>, probe: &Probe) -> Selection {
    let possible = |backend: Backend| match backend {
        Backend::LayerShell => probe.layer_shell,
        Backend::X11Desktop => probe.x11_display,
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
    let (backend, why) = if probe.layer_shell {
        (
            Backend::LayerShell,
            "the compositor offers layer-shell, so widgets sit on the desktop layer.",
        )
    } else if probe.x11_session && probe.x11_display {
        (
            Backend::X11Desktop,
            "X11 session: frameless window kept below others, on every workspace.",
        )
    } else {
        (
            Backend::Window,
            "no layer-shell and no X11 session: ordinary window; it cannot stay behind other windows.",
        )
    };
    Selection {
        backend,
        note: format!("{prefix}{}: {why}", backend.name()),
    }
}

/// Read the real session. Never fails: a missing piece only turns a capability off.
pub fn probe_session() -> Probe {
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    Probe {
        layer_shell: std::env::var_os("WAYLAND_DISPLAY").is_some() && layer::available(),
        x11_display: std::env::var_os("DISPLAY").is_some() && x11rb::connect(None).is_ok(),
        x11_session: session.eq_ignore_ascii_case("x11"),
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
pub fn layer_for(layer: Layer) -> LayerPlacement {
    match layer {
        Layer::Behind | Layer::Normal => LayerPlacement::Bottom,
        Layer::Above => LayerPlacement::Top,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerPlacement {
    Bottom,
    Top,
}

/// Size the surface asks for, from saved geometry, kept to a sane range.
pub fn surface_size(saved: Option<(f32, f32)>) -> (u32, u32) {
    let (w, h) = saved.unwrap_or((440.0, 900.0));
    (w.clamp(240.0, 1600.0) as u32, h.clamp(160.0, 2400.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ SQ-26 SQ-22: backend choice follows the real session and reports fallbacks.
    #[test]
    fn layer_shell_is_preferred_when_the_compositor_offers_it() {
        let probe = Probe {
            layer_shell: true,
            x11_display: true,
            x11_session: false,
        };
        let chosen = select(None, &probe);
        assert_eq!(chosen.backend, Backend::LayerShell);
        assert!(chosen.note.contains("layer-shell"));
    }

    // REQ SQ-26 SQ-22
    #[test]
    fn x11_session_uses_the_x11_backend_and_wayland_without_layer_shell_uses_a_window() {
        let x11 = Probe {
            layer_shell: false,
            x11_display: true,
            x11_session: true,
        };
        assert_eq!(select(None, &x11).backend, Backend::X11Desktop);
        let gnome_wayland = Probe {
            layer_shell: false,
            x11_display: true,
            x11_session: false,
        };
        let chosen = select(None, &gnome_wayland);
        assert_eq!(chosen.backend, Backend::Window);
        assert!(chosen.note.contains("cannot stay behind"));
        assert_eq!(select(None, &Probe::default()).backend, Backend::Window);
    }

    // REQ SQ-26: a forced choice that cannot work falls back and says so.
    #[test]
    fn forced_backend_is_honored_only_when_possible() {
        let probe = Probe {
            layer_shell: false,
            x11_display: true,
            x11_session: false,
        };
        assert_eq!(
            select(Some(Backend::X11Desktop), &probe).backend,
            Backend::X11Desktop
        );
        let fallback = select(Some(Backend::LayerShell), &probe);
        assert_eq!(fallback.backend, Backend::Window);
        assert!(fallback.note.contains("is not available here"));
        assert_eq!(Backend::parse("Layer-Shell"), Some(Backend::LayerShell));
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
