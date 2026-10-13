//! Unattended tests: posts configured key presses to the game window on a timeline (relative
//! to DLL load), so a test run can log in and enter the game without anyone at the keyboard.
//!
//! Afterwards it polls `poe-hook.keys` next to the exe: each whitespace-separated entry is a
//! virtual key, optionally with a hold time in ms (`0x0d`, `87`, `0x57:800`). The file is
//! deleted once processed, so a test driver can send keys at any time.

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
    let file = crate::exe_dir().join("poe-hook.keys");
    std::thread::spawn(move || {
        for (at_ms, vk) in keys {
            let at = Duration::from_millis(at_ms);
            if let Some(wait) = at.checked_sub(start.elapsed()) {
                std::thread::sleep(wait);
            }
            press(vk, 60);
        }
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let _ = std::fs::remove_file(&file);
            for entry in text.split_whitespace() {
                let (key, hold) = entry.split_once(':').unwrap_or((entry, "60"));
                let parse = |v: &str| match v.strip_prefix("0x") {
                    Some(hex) => u64::from_str_radix(hex, 16).ok(),
                    None => v.parse().ok(),
                };
                match (parse(key), parse(hold)) {
                    (Some(vk), Some(ms)) => press(vk as u16, ms),
                    _ => log::warn!("poe-hook.keys: bad entry {entry:?}"),
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    Ok(())
}

fn press(vk: u16, hold_ms: u64) {
    let Some(hwnd) = game_window() else {
        log::warn!("key {vk:#x}: no game window");
        return;
    };
    unsafe {
        PostMessageW(hwnd, WM_KEYDOWN, vk as usize, 1);
        std::thread::sleep(Duration::from_millis(hold_ms));
        PostMessageW(hwnd, WM_KEYUP, vk as usize, 0xc000_0001u32 as LPARAM);
    }
    log::info!("key {vk:#x} pressed for {hold_ms} ms");
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
