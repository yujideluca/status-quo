//! Windows: makes the panel part of the desktop and keeps it behind every other window (SQ-48).
//!
//! An ordinary window, even without a frame, is still an application window to Windows: it is
//! listed in Alt+Tab, "Show desktop" (Win+D) hides it, its edges resize it and dragging it to a
//! screen edge snaps it. `attach_to_desktop` removes all of that: the window styles become
//! those of a plain surface, and while the panel is kept behind the other windows the desktop
//! window is its owner (Windows keeps an owned window in front of its owner, so the panel
//! stays when "Show desktop" clears the screen).
//! Which window is "the desktop" changes on Windows 10: "Show desktop" moves the desktop icons
//! out of "Program Manager" into a separate full-screen window (class `WorkerW`) and puts that
//! one in front of every ordinary window, the panel included (seen on Windows 10 LTSC 2021,
//! 2026-11-19; Windows 11 24H2 keeps the icons where they are). So the panel's owner is always
//! the window that holds the icons right now (`icon_host`), looked up again whenever the
//! active window changes or a window is minimised or restored.
//! The panel is not the active window either, not even after a click on it: its buttons work
//! without that. It takes the keyboard focus only while a text field of the panel is being
//! typed in (`keyboard`), and gives it back to the desktop afterwards. Focus that Windows hands
//! it unasked goes back to the desktop at once. The reason is "Show desktop" (Win+D): with the
//! panel as the active window, before or after the first press, a second press no longer
//! brought the other windows back (seen on Windows 11, 2026-11-18). While a text field has
//! the focus, the panel lets go of it as soon as the Windows key is pressed or another window
//! is minimised ("Show desktop" does that to every window).
//!
//! Windows has no "stay at the bottom" window style. The window library moves the window to
//! the bottom once, and Windows raises it again at the next click or activation. So every
//! position change of the panel is answered here, before it happens, with "at the bottom":
//! the window procedure is wrapped and WM_WINDOWPOSCHANGING is edited on its way through.
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};

static BEHIND: AtomicBool = AtomicBool::new(false);
static ON_DESKTOP: AtomicBool = AtomicBool::new(false);
static TYPING: AtomicBool = AtomicBool::new(false);
/// When the panel was last clicked, in milliseconds since Windows started (0: never).
static CLICKED_AT: AtomicU64 = AtomicU64::new(0);
static EXPLORER_STARTED: AtomicU32 = AtomicU32::new(0);
/// The panel's owner is a raised icon window ("Show desktop" on Windows 10), not the bottom one.
static RAISED: AtomicBool = AtomicBool::new(false);
static PREVIOUS: AtomicIsize = AtomicIsize::new(0);
static PANEL: AtomicIsize = AtomicIsize::new(0);

#[repr(C)]
struct WindowPos {
    hwnd: isize,
    insert_after: isize,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    flags: u32,
}
#[repr(C)]
struct StyleChange {
    old: u32,
    new: u32,
}
const GWLP_WNDPROC: i32 = -4;
const GWLP_HWNDPARENT: i32 = -8;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const WM_STYLECHANGING: u32 = 0x007C;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_WINDOWEDGE: u32 = 0x0000_0100;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;
const WM_ACTIVATE: u32 = 0x0006;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_KEYDOWN: u32 = 0x0100;
const VK_LWIN: usize = 0x5B;
const VK_RWIN: usize = 0x5C;
const MA_NOACTIVATE: isize = 3;
const SWP_FRAMECHANGED: u32 = 0x0020;

/// The window style of a panel that is part of the desktop: no title bar, no resizing edge
/// (which also ends snapping to screen edges and the rounded, shadowed frame of Windows 11),
/// no minimize or maximize.
pub fn desktop_style(style: u32) -> u32 {
    (style & !(WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX))
        | WS_POPUP
}
/// The extended style of such a panel: a tool window is listed neither in Alt+Tab nor on the
/// taskbar.
pub fn desktop_ex_style(style: u32) -> u32 {
    (style & !(WS_EX_APPWINDOW | WS_EX_WINDOWEDGE)) | WS_EX_TOOLWINDOW
}
/// Whether the panel gives the keyboard focus it has just received back to the desktop: yes,
/// unless a text field of the panel is being typed in.
pub fn returns_focus(typing: bool) -> bool {
    !typing
}
/// Whether a click brings the panel in front of the other windows: in every layer but Behind
/// (Above is in front already; the call is harmless there).
/// The window that shows the desktop icons right now, which is the one the panel must be owned
/// by: "Program Manager" (`progman`) while it holds the icons or when nobody does, otherwise
/// the first of the desktop's extra windows (`layers`: window and "holds the icons, visible").
pub fn icon_host(progman: isize, progman_has_icons: bool, layers: &[(isize, bool)]) -> isize {
    if progman_has_icons {
        return progman;
    }
    layers
        .iter()
        .find(|(_, has_icons)| *has_icons)
        .map_or(progman, |(window, _)| *window)
}
pub fn raises_on_click(behind: bool) -> bool {
    !behind
}
/// What the panel does about the keyboard in one frame.
#[derive(Debug, PartialEq)]
pub enum Keyboard {
    /// Nothing changes.
    Stay,
    /// Become the active window: a text field was just clicked.
    Take,
    /// Hand the keyboard back to the desktop: no text field is being typed in any more.
    GiveBack,
    /// Take the text cursor out of the field: the user went to another window a while ago.
    LeaveField,
}
/// How long after a click on the panel a text field may still claim the keyboard, in
/// milliseconds: the field takes the text cursor when the button is released, not pressed.
pub const CLICK_GRACE_MS: u64 = 1500;
/// `active`: the panel is the active window; `wants_keyboard`: a text field of the panel
/// holds the text cursor; `since_click`: milliseconds since the last click on the panel
/// (`None`: never clicked). The panel never takes the keyboard by itself, only for a text
/// field the user has just clicked.
pub fn keyboard_action(active: bool, wants_keyboard: bool, since_click: Option<u64>) -> Keyboard {
    let just_clicked = since_click.is_some_and(|ms| ms <= CLICK_GRACE_MS);
    match (active, wants_keyboard) {
        (false, true) if just_clicked => Keyboard::Take,
        (false, true) => Keyboard::LeaveField,
        (true, false) => Keyboard::GiveBack,
        _ => Keyboard::Stay,
    }
}

const WM_WINDOWPOSCHANGING: u32 = 0x0046;
/// Events Windows reports to `desktop_changed`.
const FOREGROUND: u32 = 0x0003;
const MINIMIZE_START: u32 = 0x0016;
const MINIMIZE_END: u32 = 0x0017;
const HWND_BOTTOM: isize = 1;
const HWND_TOP: isize = 0;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;

#[link(name = "user32")]
extern "system" {
    fn SetWindowLongPtrW(window: isize, index: i32, value: isize) -> isize;
    fn GetWindowLongPtrW(window: isize, index: i32) -> isize;
    fn FindWindowW(class: *const u16, title: *const u16) -> isize;
    fn FindWindowExW(parent: isize, after: isize, class: *const u16, title: *const u16) -> isize;
    fn IsWindowVisible(window: isize) -> i32;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn SetForegroundWindow(window: isize) -> i32;
    fn GetShellWindow() -> isize;
    fn GetForegroundWindow() -> isize;
    fn SetWinEventHook(
        first: u32,
        last: u32,
        module: isize,
        callback: unsafe extern "system" fn(isize, u32, isize, i32, i32, u32, u32),
        process: u32,
        thread: u32,
        flags: u32,
    ) -> isize;
    fn CallWindowProcW(
        previous: isize,
        window: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize;
    fn DefWindowProcW(window: isize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn SystemParametersInfoW(action: u32, param: u32, value: *mut [i32; 4], update: u32) -> i32;
    fn SetWindowPos(
        window: isize,
        insert_after: isize,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetTickCount64() -> u64;
}
#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(window: isize, attribute: u32, value: *const u32, size: u32) -> i32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn wrap(window: isize) {
    if PREVIOUS.load(Ordering::Acquire) == 0 {
        let previous = SetWindowLongPtrW(window, GWLP_WNDPROC, procedure as usize as isize);
        PREVIOUS.store(previous, Ordering::Release);
    }
}

/// The window that holds the desktop icons now (see `icon_host`), or 0 without a desktop.
unsafe fn desktop_now() -> isize {
    let progman = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
    if progman == 0 {
        return 0;
    }
    let (icons, layer) = (wide("SHELLDLL_DefView"), wide("WorkerW"));
    let holds_icons = |window| FindWindowExW(window, 0, icons.as_ptr(), std::ptr::null()) != 0;
    let mut layers = Vec::new();
    let mut window = 0;
    for _ in 0..64 {
        window = FindWindowExW(0, window, layer.as_ptr(), std::ptr::null());
        if window == 0 {
            break;
        }
        layers.push((window, IsWindowVisible(window) != 0 && holds_icons(window)));
    }
    icon_host(progman, holds_icons(progman), &layers)
}

/// The desktop window (the one with the icons, owned by Windows Explorer) becomes the panel's
/// owner while the panel is kept behind the other windows. In the other layers it has no
/// owner: an owned window is stacked with its owner, and those layers stack the panel by
/// themselves. Called again whenever the desktop may have changed; does nothing when it has
/// not.
unsafe fn own_by_desktop(window: isize) {
    if !ON_DESKTOP.load(Ordering::Acquire) {
        return;
    }
    if !BEHIND.load(Ordering::Acquire) {
        RAISED.store(false, Ordering::Release);
        if GetWindowLongPtrW(window, GWLP_HWNDPARENT) != 0 {
            SetWindowLongPtrW(window, GWLP_HWNDPARENT, 0);
        }
        return;
    }
    let desktop = desktop_now();
    if desktop == 0 || GetWindowLongPtrW(window, GWLP_HWNDPARENT) == desktop {
        return;
    }
    let raised = desktop != FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
    RAISED.store(raised, Ordering::Release);
    SetWindowLongPtrW(window, GWLP_HWNDPARENT, desktop);
    // Windows sorts an owned window in front of its owner only when the stacking changes.
    let place = if raised { HWND_TOP } else { HWND_BOTTOM };
    SetWindowPos(
        window,
        place,
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
    );
}

/// Give the keyboard back to the desktop now; the text field may not claim it again until
/// the panel is clicked once more.
unsafe fn stop_typing() {
    TYPING.store(false, Ordering::Release);
    CLICKED_AT.store(0, Ordering::Release);
    SetForegroundWindow(GetShellWindow());
}

/// Called by Windows, on the panel's own thread, when another program's window becomes the
/// active one or is minimised or restored: the moments at which "Show desktop" starts or ends.
unsafe extern "system" fn desktop_changed(
    _hook: isize,
    event: u32,
    window: isize,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let panel = PANEL.load(Ordering::Acquire);
    // Object 0 is the window itself (not a part of it such as a scroll bar).
    if panel == 0 {
        return;
    }
    if event == MINIMIZE_START && object == 0 && window != panel && GetForegroundWindow() == panel {
        stop_typing();
    }
    own_by_desktop(panel);
}

/// Make the window `window` (a HWND) part of the desktop: see the module text. Must be called
/// on the thread that owns the window. The styles are held from then on: the window library
/// writes its own again when it changes a window property, and that is edited on its way in.
pub fn attach_to_desktop(window: isize) {
    unsafe {
        wrap(window);
        EXPLORER_STARTED.store(
            RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            Ordering::Release,
        );
        ON_DESKTOP.store(true, Ordering::Release);
        if PANEL.swap(window, Ordering::AcqRel) == 0 {
            // From "a window became the active one" to "a window was restored", from other
            // programs only, delivered through this thread's message loop.
            const OUT_OF_CONTEXT_SKIP_OWN_PROCESS: u32 = 0x0002;
            SetWinEventHook(
                FOREGROUND,
                MINIMIZE_END,
                0,
                desktop_changed,
                0,
                0,
                OUT_OF_CONTEXT_SKIP_OWN_PROCESS,
            );
        }
        // Writing the styles back sends them through `procedure`, which edits them.
        SetWindowLongPtrW(window, GWL_STYLE, GetWindowLongPtrW(window, GWL_STYLE));
        SetWindowLongPtrW(window, GWL_EXSTYLE, GetWindowLongPtrW(window, GWL_EXSTYLE));
        // The panel was given the focus when it appeared; nobody asked for that either.
        if GetForegroundWindow() == window {
            SetForegroundWindow(GetShellWindow());
        }
        // Windows 11 draws round corners and a thin border around every window; neither
        // setting exists on Windows 10, where the calls fail and nothing is lost.
        const CORNER_PREFERENCE: u32 = 33;
        const BORDER_COLOR: u32 = 34;
        let (square, no_border) = (1u32, 0xFFFF_FFFEu32);
        DwmSetWindowAttribute(window, CORNER_PREFERENCE, &square, 4);
        DwmSetWindowAttribute(window, BORDER_COLOR, &no_border, 4);
        SetWindowPos(
            window,
            0,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

unsafe extern "system" fn procedure(
    window: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if message == WM_WINDOWPOSCHANGING && lparam != 0 && BEHIND.load(Ordering::Acquire) {
        // Only a request that changes the stacking is answered. A move or a repaint is left
        // alone: restacking the panel on those made Windows forget which windows "Show
        // desktop" had put away.
        let position = &mut *(lparam as *mut WindowPos);
        // While "Show desktop" has raised the desktop the panel stays where its owner is.
        if position.flags & SWP_NOZORDER == 0 && !RAISED.load(Ordering::Acquire) {
            position.insert_after = HWND_BOTTOM;
        }
    }
    if ON_DESKTOP.load(Ordering::Acquire) {
        if message == WM_STYLECHANGING && lparam != 0 {
            let change = &mut *(lparam as *mut StyleChange);
            if wparam as i32 == GWL_STYLE {
                change.new = desktop_style(change.new);
            } else if wparam as i32 == GWL_EXSTYLE {
                change.new = desktop_ex_style(change.new);
            }
        }
        // A click does not make the panel the active window.
        if message == WM_MOUSEACTIVATE && !TYPING.load(Ordering::Acquire) {
            return MA_NOACTIVATE;
        }
        // The Windows key starts a desktop shortcut (Win+D and others): the desktop must be
        // the active window before the second key arrives.
        if message == WM_KEYDOWN && (wparam == VK_LWIN || wparam == VK_RWIN) {
            stop_typing();
        }
        if message == WM_LBUTTONDOWN {
            // In the Normal layer a click brings the panel in front of the other windows, as
            // it does for any window, still without making it the active one.
            if raises_on_click(BEHIND.load(Ordering::Acquire)) {
                SetWindowPos(
                    window,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            CLICKED_AT.store(GetTickCount64().max(1), Ordering::Release);
        }
        if message == WM_ACTIVATE {
            if wparam & 0xFFFF == 0 {
                TYPING.store(false, Ordering::Release);
            } else if returns_focus(TYPING.load(Ordering::Acquire)) {
                SetForegroundWindow(GetShellWindow());
            }
        }
        // Windows Explorer was started again (after a crash or an update): the desktop is a
        // new window, and the panel lost its owner with the old one.
        if message != 0 && message == EXPLORER_STARTED.load(Ordering::Acquire) {
            own_by_desktop(window);
        }
    }
    match PREVIOUS.load(Ordering::Acquire) {
        0 => DefWindowProcW(window, message, wparam, lparam),
        previous => CallWindowProcW(previous, window, message, wparam, lparam),
    }
}

/// Called once per frame with whether a text field of the panel holds the text cursor.
/// Takes the keyboard for a text field that was just clicked and gives it back to the desktop
/// when no text field wants it any more. Returns true when the caller must take the text
/// cursor out of the field (the user left for another window). Does nothing for a floating
/// window. Must be called on the thread that owns the window.
pub fn keyboard(wants_keyboard: bool) -> bool {
    let panel = PANEL.load(Ordering::Acquire);
    if panel == 0 {
        return false;
    }
    // A safety net for a desktop change that no event announced.
    unsafe { own_by_desktop(panel) };
    unsafe {
        let since_click = match CLICKED_AT.load(Ordering::Acquire) {
            0 => None,
            at => Some(GetTickCount64().saturating_sub(at)),
        };
        match keyboard_action(GetForegroundWindow() == panel, wants_keyboard, since_click) {
            Keyboard::Take => {
                TYPING.store(true, Ordering::Release);
                SetForegroundWindow(panel);
            }
            Keyboard::GiveBack => {
                TYPING.store(false, Ordering::Release);
                SetForegroundWindow(GetShellWindow());
            }
            Keyboard::LeaveField => return true,
            Keyboard::Stay => {}
        }
    }
    false
}

/// Keep the window `window` (a HWND) behind all others, or stop doing so. Must be called on
/// the thread that owns the window.
pub fn keep_behind(window: isize, behind: bool) {
    BEHIND.store(behind, Ordering::Release);
    unsafe {
        wrap(window);
        own_by_desktop(window);
        if behind && !RAISED.load(Ordering::Acquire) {
            SetWindowPos(
                window,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

/// The part of the main screen not covered by the taskbar: left, top, right, bottom in pixels.
pub fn work_area() -> Option<[i32; 4]> {
    const SPI_GETWORKAREA: u32 = 0x0030;
    let mut area = [0i32; 4];
    let found = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut area, 0) } != 0;
    (found && area[2] > area[0] && area[3] > area[1]).then_some(area)
}

#[cfg(test)]
mod tests {
    use super::*;
    // REQ SQ-48: on Windows the panel on the desktop background is no application window: no
    // title bar, no resizing edge, no minimize or maximize, not listed in Alt+Tab or on the
    // taskbar, and it holds the keyboard focus only while a text field is typed in; the rest of the
    // style is kept.
    #[test]
    fn a_desktop_panel_has_no_application_window_styles() {
        // What the window library gives a frameless window (read on Windows 11, 2026-11-18).
        let (style, ex_style) = (0x16CF_0000u32, 0x0004_0110u32);
        let desktop = desktop_style(style);
        assert_eq!(desktop, 0x9600_0000, "visible popup, clipping kept");
        for gone in [
            WS_CAPTION,
            WS_SYSMENU,
            WS_THICKFRAME,
            WS_MINIMIZEBOX,
            WS_MAXIMIZEBOX,
        ] {
            assert_eq!(desktop & gone, 0);
        }
        let desktop_ex = desktop_ex_style(ex_style);
        assert_eq!(desktop_ex & WS_EX_APPWINDOW, 0);
        assert_ne!(desktop_ex & WS_EX_TOOLWINDOW, 0);
        assert_eq!(desktop_ex & 0x10, 0x10, "other bits are kept");
        // Applying the rule twice changes nothing.
        assert_eq!(desktop_style(desktop), desktop);
        assert_eq!(desktop_ex_style(desktop_ex), desktop_ex);
        // Focus is kept only while a text field is typed in, and taken only for a text field
        // the user has just clicked; a field left behind gives up the text cursor.
        assert!(returns_focus(false));
        assert!(!returns_focus(true));
        // The owner is the window with the desktop icons: "Program Manager", or the extra
        // window "Show desktop" moves them to on Windows 10.
        assert_eq!(icon_host(7, true, &[(8, false), (9, true)]), 7);
        assert_eq!(icon_host(7, false, &[(8, false), (9, true)]), 9);
        assert_eq!(icon_host(7, false, &[(8, false)]), 7);
        assert_eq!(icon_host(7, false, &[]), 7);
        assert!(raises_on_click(false));
        assert!(!raises_on_click(true));
        use Keyboard::*;
        assert_eq!(keyboard_action(false, true, Some(200)), Take);
        assert_eq!(
            keyboard_action(false, true, Some(CLICK_GRACE_MS + 1)),
            LeaveField
        );
        assert_eq!(keyboard_action(false, true, None), LeaveField);
        assert_eq!(keyboard_action(true, false, Some(10)), GiveBack);
        assert_eq!(keyboard_action(true, true, None), Stay);
        assert_eq!(keyboard_action(false, false, Some(10)), Stay);
    }
}
