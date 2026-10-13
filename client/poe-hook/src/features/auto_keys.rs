//! Unattended tests: posts configured key presses to the game window on a timeline (relative
//! to DLL load), so a test run can log in, enter the game and take an in-game screenshot
//! without anyone at the keyboard.

use std::time::{Duration, Instant};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    System::Threading::GetCurrentProcessId,
    UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_KEYDOWN,
        WM_KEYUP,
    },
};

pub fn init(keys: Vec<(u64, u16)>) -> Result<(), String> {
    let start = Instant::now();
    std::thread::spawn(move || {
        for (at_ms, vk) in keys {
            let at = Duration::from_millis(at_ms);
            if let Some(wait) = at.checked_sub(start.elapsed()) {
                std::thread::sleep(wait);
            }
            match game_window() {
                Some(hwnd) => {
                    unsafe {
                        PostMessageW(hwnd, WM_KEYDOWN, vk as usize, 1);
                        std::thread::sleep(Duration::from_millis(60));
                        PostMessageW(hwnd, WM_KEYUP, vk as usize, (0xc000_0001u32) as LPARAM);
                    }
                    log::info!("auto key {vk:#x} posted at {at_ms} ms");
                }
                None => log::warn!("auto key {vk:#x}: no game window"),
            }
        }
    });
    Ok(())
}

/// First visible top-level window of this process.
fn game_window() -> Option<HWND> {
    unsafe extern "system" fn visit(hwnd: HWND, out: LPARAM) -> i32 {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == unsafe { GetCurrentProcessId() } && unsafe { IsWindowVisible(hwnd) } != 0 {
            unsafe { *(out as *mut HWND) = hwnd };
            return 0;
        }
        1
    }
    let mut found: HWND = std::ptr::null_mut();
    unsafe { EnumWindows(Some(visit), &mut found as *mut HWND as LPARAM) };
    (!found.is_null()).then_some(found)
}
