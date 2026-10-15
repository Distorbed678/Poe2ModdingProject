//! Window-procedure subclass of the game window (installed once the Vulkan swapchain names the
//! window): every message goes to the core first, which consumes overlay input and hotkeys.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{CallWindowProcW, GWLP_WNDPROC, SetWindowLongPtrW, WNDPROC},
};

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static ORIGINAL: AtomicIsize = AtomicIsize::new(0);

pub fn install(hwnd: isize) {
    if WINDOW.load(Ordering::SeqCst) == hwnd {
        return;
    }
    let previous = unsafe { SetWindowLongPtrW(hwnd as HWND, GWLP_WNDPROC, proc as *const () as isize) };
    if previous == 0 {
        log::error!("window {hwnd:#x}: subclass failed");
        return;
    }
    ORIGINAL.store(previous, Ordering::SeqCst);
    WINDOW.store(hwnd, Ordering::SeqCst);
    log::info!("window {hwnd:#x}: subclassed for the overlay");
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(result) = crate::core::window_message(hwnd as isize, msg, wparam, lparam) {
        return result;
    }
    let original: WNDPROC = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::SeqCst)) };
    unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) }
}
