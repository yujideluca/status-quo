//! Windows: keeps the panel behind every other window (SQ-48).
//!
//! Windows has no "stay at the bottom" window style. The window library moves the window to
//! the bottom once, and Windows raises it again at the next click or activation. So every
//! position change of the panel is answered here, before it happens, with "at the bottom":
//! the window procedure is wrapped and WM_WINDOWPOSCHANGING is edited on its way through.
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

static BEHIND: AtomicBool = AtomicBool::new(false);
static PREVIOUS: AtomicIsize = AtomicIsize::new(0);

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
const GWLP_WNDPROC: i32 = -4;
const WM_WINDOWPOSCHANGING: u32 = 0x0046;
const HWND_BOTTOM: isize = 1;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;

#[link(name = "user32")]
extern "system" {
    fn SetWindowLongPtrW(window: isize, index: i32, value: isize) -> isize;
    fn CallWindowProcW(
        previous: isize,
        window: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize;
    fn DefWindowProcW(window: isize, message: u32, wparam: usize, lparam: isize) -> isize;
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

unsafe extern "system" fn procedure(
    window: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if message == WM_WINDOWPOSCHANGING && lparam != 0 && BEHIND.load(Ordering::Acquire) {
        let position = &mut *(lparam as *mut WindowPos);
        position.insert_after = HWND_BOTTOM;
        position.flags &= !SWP_NOZORDER;
    }
    match PREVIOUS.load(Ordering::Acquire) {
        0 => DefWindowProcW(window, message, wparam, lparam),
        previous => CallWindowProcW(previous, window, message, wparam, lparam),
    }
}

/// Keep the window `window` (a HWND) behind all others, or stop doing so. Must be called on
/// the thread that owns the window.
pub fn keep_behind(window: isize, behind: bool) {
    BEHIND.store(behind, Ordering::Release);
    unsafe {
        if PREVIOUS.load(Ordering::Acquire) == 0 {
            let previous = SetWindowLongPtrW(window, GWLP_WNDPROC, procedure as usize as isize);
            PREVIOUS.store(previous, Ordering::Release);
        }
        if behind {
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
