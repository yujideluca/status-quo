//! X11 desktop backend: standard window-manager hints, set after the window is shown.
//!
//! The window type cannot be changed through eframe, so this uses the state hints that
//! Conky-style tools use: below other windows, sticky (all workspaces), off the taskbar.
use x11rb::{
    connection::Connection,
    protocol::xproto::{
        Atom, ClientMessageData, ClientMessageEvent, ConnectionExt as _, EventMask,
    },
};

/// Hints that keep the tray on the desktop. Order matters only for readability.
pub const STATE_HINTS: [&str; 4] = [
    "_NET_WM_STATE_BELOW",
    "_NET_WM_STATE_STICKY",
    "_NET_WM_STATE_SKIP_TASKBAR",
    "_NET_WM_STATE_SKIP_PAGER",
];

/// Hints the window manager must report for the backend to count as working.
/// Sticky (all workspaces) is requested but best-effort: KWin on XWayland ignores it.
pub const REQUIRED_HINTS: [&str; 3] = [
    "_NET_WM_STATE_BELOW",
    "_NET_WM_STATE_SKIP_TASKBAR",
    "_NET_WM_STATE_SKIP_PAGER",
];

/// `_NET_WM_STATE` client message: action 1 = add, up to two properties per message.
pub fn state_message_data(first: Atom, second: Atom) -> [u32; 5] {
    [1, first, second, 1, 0]
}

/// True when the window manager reports every desktop hint on `window`.
pub fn hints_active(window: u32) -> bool {
    let Ok((conn, _)) = x11rb::connect(None) else {
        return false;
    };
    let atom = |name: &str| {
        conn.intern_atom(false, name.as_bytes())
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom)
    };
    let Some(state) = atom("_NET_WM_STATE") else {
        return false;
    };
    let Some(reply) = conn
        .get_property(
            false,
            window,
            state,
            x11rb::protocol::xproto::AtomEnum::ATOM,
            0,
            64,
        )
        .ok()
        .and_then(|c| c.reply().ok())
    else {
        return false;
    };
    let present: Vec<u32> = reply.value32().map(|v| v.collect()).unwrap_or_default();
    REQUIRED_HINTS
        .iter()
        .all(|name| atom(name).is_some_and(|a| present.contains(&a)))
}

/// `_NET_WM_DESKTOP` value that means "every workspace".
pub const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;

/// Ask the window manager to apply the desktop hints to `window`. Returns what was sent.
pub fn apply_desktop_hints(window: u32) -> Result<String, String> {
    let (conn, screen) = x11rb::connect(None).map_err(|e| format!("X display: {e}"))?;
    let root = conn.setup().roots[screen].root;
    let atom = |name: &str| -> Result<Atom, String> {
        conn.intern_atom(false, name.as_bytes())
            .map_err(|e| e.to_string())?
            .reply()
            .map(|r| r.atom)
            .map_err(|e| e.to_string())
    };
    let state = atom("_NET_WM_STATE")?;
    let atoms: Vec<Atom> = STATE_HINTS
        .iter()
        .map(|n| atom(n))
        .collect::<Result<_, _>>()?;
    for pair in atoms.chunks(2) {
        let data = state_message_data(pair[0], *pair.get(1).unwrap_or(&0));
        let event = ClientMessageEvent {
            response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window,
            type_: state,
            data: ClientMessageData::from(data),
        };
        conn.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .map_err(|e| e.to_string())?;
    }
    // Some window managers (KWin) take "all workspaces" only as a desktop change.
    let desktop = atom("_NET_WM_DESKTOP")?;
    let event = ClientMessageEvent {
        response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window,
        type_: desktop,
        data: ClientMessageData::from([ALL_DESKTOPS, 1, 0, 0, 0]),
    };
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    )
    .map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    Ok(STATE_HINTS.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ SQ-26 SQ-27: the X11 backend requests below, sticky and off-taskbar state.
    #[test]
    fn desktop_hints_add_below_sticky_and_skip_taskbar() {
        assert!(STATE_HINTS.contains(&"_NET_WM_STATE_BELOW"));
        assert!(STATE_HINTS.contains(&"_NET_WM_STATE_STICKY"));
        assert!(STATE_HINTS.contains(&"_NET_WM_STATE_SKIP_TASKBAR"));
        assert!(!REQUIRED_HINTS.contains(&"_NET_WM_STATE_STICKY"));
        assert!(REQUIRED_HINTS.iter().all(|h| STATE_HINTS.contains(h)));
        assert_eq!(state_message_data(7, 9), [1, 7, 9, 1, 0]);
    }
}
