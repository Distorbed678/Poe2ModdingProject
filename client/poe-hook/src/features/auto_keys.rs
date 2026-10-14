//! Unattended tests: posts configured key presses to the game window on a timeline (relative
//! to DLL load), so a test run can log in and enter the game without anyone at the keyboard.
//!
//! Afterwards it polls `poe-hook.keys` next to the exe: each whitespace-separated entry is a
//! virtual key, optionally with a hold time in ms (`0x0d`, `87`, `0x57:800`), or a left click
//! at client coordinates (`click:1280,720`), or `close` (WM_CLOSE, like the window's X button).
//! Posted key messages reach the UI, but the game reads movement and skill keys through Raw
//! Input, so `in:<vk>[:ms]` posts `WM_INPUT` with a marker handle that the hooked
//! `GetRawInputData` turns into a keyboard record (and `GetKeyState` reports the key as held).
//! `inmove:x,y` / `inclick:x,y` override `GetCursorPos` with that client position and post the
//! mouse messages there. `indown:<vk>` / `inup:<vk>` press and release a Raw Input key
//! separately and `wait:<ms>` pauses, so input can overlap (e.g. Escape while W is held).
//! Nothing is sent to other windows and the real cursor never moves.
//! The file is deleted once processed, so a test
//! driver can send input at any time.

use std::time::{Duration, Instant};

use std::{
    collections::HashMap,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
    },
};

use ilhook::x64::{CallbackOption, HookFlags, HookType, Hooker, Registers};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT},
    Graphics::Gdi::ClientToScreen,
    System::{
        LibraryLoader::{GetModuleHandleW, GetProcAddress},
        Threading::GetCurrentProcessId,
    },
    UI::{
        Input::{
            KeyboardAndMouse::{MAPVK_VK_TO_VSC, MapVirtualKeyW},
            RAWINPUT, RAWINPUTHEADER, RID_HEADER, RID_INPUT, RIM_TYPEKEYBOARD,
        },
        WindowsAndMessaging::{
            EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, RI_KEY_BREAK,
            RI_KEY_MAKE, WM_CLOSE, WM_INPUT, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MOUSEMOVE,
        },
    },
};

pub fn init(keys: Vec<(u64, u16)>) -> Result<(), String> {
    hook_input()?;
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
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let _ = std::fs::remove_file(&file);
            for entry in text.split_whitespace() {
                if entry == "close" {
                    match game_window() {
                        Some(hwnd) => unsafe {
                            log::info!("posting WM_CLOSE");
                            PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        },
                        None => log::warn!("close: no game window"),
                    }
                    continue;
                }
                // Raw Input key held down / released separately (to overlap with other input).
                if let Some((kind, key)) = entry.split_once(':')
                    && (kind == "indown" || kind == "inup")
                {
                    match (parse_num(key), game_window()) {
                        (Some(vk), Some(hwnd)) => {
                            raw_key(hwnd, vk as u16, kind == "inup");
                            log::info!("raw input key {vk:#x} {kind}");
                        }
                        _ => log::warn!("poe-hook.keys: bad entry {entry:?}"),
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                if let Some(ms) = entry.strip_prefix("wait:") {
                    if let Some(ms) = parse_num(ms) {
                        std::thread::sleep(Duration::from_millis(ms));
                    }
                    continue;
                }
                if let Some(rest) = entry.strip_prefix("in:") {
                    let (key, hold) = rest.split_once(':').unwrap_or((rest, "60"));
                    match (parse_num(key), parse_num(hold)) {
                        (Some(vk), Some(ms)) => input_key(vk as u16, ms),
                        _ => log::warn!("poe-hook.keys: bad entry {entry:?}"),
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                if let Some((kind, pos)) = entry.split_once(':')
                    && (kind == "inmove" || kind == "inclick")
                {
                    match pos
                        .split_once(',')
                        .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))
                    {
                        Some((x, y)) => input_mouse(x, y, kind == "inclick"),
                        None => log::warn!("poe-hook.keys: bad entry {entry:?}"),
                    }
                    std::thread::sleep(Duration::from_millis(150));
                    continue;
                }
                if let Some(pos) = entry.strip_prefix("click:") {
                    match pos
                        .split_once(',')
                        .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))
                    {
                        Some((x, y)) => click(x, y),
                        None => log::warn!("poe-hook.keys: bad click {entry:?}"),
                    }
                    std::thread::sleep(Duration::from_millis(150));
                    continue;
                }
                let (key, hold) = entry.split_once(':').unwrap_or((entry, "60"));
                match (parse_num(key), parse_num(hold)) {
                    (Some(vk), Some(ms)) => press(vk as u16, ms),
                    _ => log::warn!("poe-hook.keys: bad entry {entry:?}"),
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    Ok(())
}

fn parse_num(v: &str) -> Option<u64> {
    match v.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => v.parse().ok(),
    }
}

/// Marker for synthetic `WM_INPUT` handles: `RAW_MARK | id`.
const RAW_MARK: usize = 0x5eed_0000;
static NEXT_RAW: AtomicU32 = AtomicU32::new(1);
static PENDING: OnceLock<Mutex<HashMap<usize, Raw>>> = OnceLock::new();

/// A queued record (its device handle is always null).
#[derive(Clone, Copy)]
struct Raw(RAWINPUT);
unsafe impl Send for Raw {}
/// Keys held by `in:` entries (reported by `GetKeyState`), 256 bits.
static HELD: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];
/// `GetCursorPos` override (screen coordinates) while set.
static CURSOR_SET: AtomicBool = AtomicBool::new(false);
static CURSOR_X: AtomicI32 = AtomicI32::new(0);
static CURSOR_Y: AtomicI32 = AtomicI32::new(0);

fn pending() -> &'static Mutex<HashMap<usize, Raw>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hook_input() -> Result<(), String> {
    let user32 = unsafe {
        let name: Vec<u16> = "user32.dll\0".encode_utf16().collect();
        GetModuleHandleW(name.as_ptr())
    };
    if user32.is_null() {
        return Err("user32.dll not loaded".into());
    }
    let hooks: [(&[u8], ilhook::x64::RetnRoutine); 3] = [
        (b"GetRawInputData\0", on_get_raw_input_data),
        (b"GetKeyState\0", on_get_key_state),
        (b"GetCursorPos\0", on_get_cursor_pos),
    ];
    for (name, routine) in hooks {
        let display = String::from_utf8_lossy(&name[..name.len() - 1]).into_owned();
        let addr = unsafe { GetProcAddress(user32, name.as_ptr()) }
            .ok_or_else(|| format!("{display} not exported"))? as usize;
        let hooker = Hooker::new(
            addr,
            HookType::Retn(routine),
            CallbackOption::None,
            0,
            HookFlags::empty(),
        );
        let point = unsafe { hooker.hook() }.map_err(|e| format!("hook {display}: {e:?}"))?;
        Box::leak(Box::new(point));
    }
    Ok(())
}

type GetRawInputDataFn = unsafe extern "system" fn(usize, u32, *mut u8, *mut u32, u32) -> u32;

unsafe extern "win64" fn on_get_raw_input_data(
    regs: *mut Registers,
    original: usize,
    _: usize,
) -> usize {
    let r = unsafe { &*regs };
    let (handle, command, data, size, header_size) = (
        r.rcx as usize,
        r.rdx as u32,
        r.r8 as *mut u8,
        r.r9 as *mut u32,
        unsafe { r.get_stack(5) } as u32,
    );
    if handle & !0xffff == RAW_MARK {
        let Some(Raw(input)) = pending().lock().unwrap().get(&handle).copied() else {
            return u32::MAX as usize;
        };
        let needed = if command == RID_HEADER {
            size_of::<RAWINPUTHEADER>()
        } else {
            size_of::<RAWINPUT>()
        } as u32;
        unsafe {
            if data.is_null() {
                *size = needed;
                return 0;
            }
            if *size < needed {
                return u32::MAX as usize;
            }
            std::ptr::copy_nonoverlapping(
                &input as *const RAWINPUT as *const u8,
                data,
                needed as usize,
            );
        }
        if command == RID_INPUT {
            pending().lock().unwrap().remove(&handle);
        }
        return needed as usize;
    }
    let f: GetRawInputDataFn = unsafe { std::mem::transmute(original) };
    unsafe { f(handle, command, data, size, header_size) as usize }
}

type GetKeyStateFn = unsafe extern "system" fn(i32) -> i16;

unsafe extern "win64" fn on_get_key_state(
    regs: *mut Registers,
    original: usize,
    _: usize,
) -> usize {
    let vk = unsafe { &*regs }.rcx as i32;
    if (0..256).contains(&vk) && HELD[vk as usize].load(Ordering::Relaxed) {
        return 0x8000u16 as i16 as isize as usize;
    }
    let f: GetKeyStateFn = unsafe { std::mem::transmute(original) };
    unsafe { f(vk) as isize as usize }
}

type GetCursorPosFn = unsafe extern "system" fn(*mut POINT) -> i32;

unsafe extern "win64" fn on_get_cursor_pos(
    regs: *mut Registers,
    original: usize,
    _: usize,
) -> usize {
    let p = unsafe { &*regs }.rcx as *mut POINT;
    if CURSOR_SET.load(Ordering::Relaxed) && !p.is_null() {
        unsafe {
            (*p).x = CURSOR_X.load(Ordering::Relaxed);
            (*p).y = CURSOR_Y.load(Ordering::Relaxed);
        }
        return 1;
    }
    let f: GetCursorPosFn = unsafe { std::mem::transmute(original) };
    unsafe { f(p) as usize }
}

/// Queues one synthetic raw keyboard event and posts its `WM_INPUT` to the game window.
fn raw_key(hwnd: HWND, vk: u16, up: bool) {
    let mut input: RAWINPUT = unsafe { std::mem::zeroed() };
    input.header.dwType = RIM_TYPEKEYBOARD;
    input.header.dwSize = size_of::<RAWINPUT>() as u32;
    unsafe {
        let kb = &mut input.data.keyboard;
        kb.MakeCode = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16;
        kb.Flags = if up { RI_KEY_BREAK } else { RI_KEY_MAKE } as u16;
        kb.VKey = vk;
        kb.Message = if up { WM_KEYUP } else { WM_KEYDOWN };
    }
    let handle = RAW_MARK | (NEXT_RAW.fetch_add(1, Ordering::Relaxed) & 0xffff) as usize;
    pending().lock().unwrap().insert(handle, Raw(input));
    HELD[vk as usize & 0xff].store(!up, Ordering::Relaxed);
    unsafe {
        PostMessageW(hwnd, WM_INPUT, 0 /* RIM_INPUT */, handle as LPARAM)
    };
}

fn input_key(vk: u16, hold_ms: u64) {
    let Some(hwnd) = game_window() else {
        log::warn!("in:{vk:#x}: no game window");
        return;
    };
    raw_key(hwnd, vk, false);
    std::thread::sleep(Duration::from_millis(hold_ms));
    raw_key(hwnd, vk, true);
    log::info!("raw input key {vk:#x} held for {hold_ms} ms");
}

/// Points the game's cursor at client coordinates and optionally left-clicks there.
fn input_mouse(x: i32, y: i32, click: bool) {
    let Some(hwnd) = game_window() else {
        log::warn!("inmouse: no game window");
        return;
    };
    let mut p = POINT { x, y };
    unsafe { ClientToScreen(hwnd, &mut p) };
    CURSOR_X.store(p.x, Ordering::Relaxed);
    CURSOR_Y.store(p.y, Ordering::Relaxed);
    CURSOR_SET.store(true, Ordering::Relaxed);
    let pos = ((y as u32 & 0xffff) << 16 | x as u32 & 0xffff) as LPARAM;
    unsafe {
        PostMessageW(hwnd, WM_MOUSEMOVE, 0, pos);
        if click {
            std::thread::sleep(Duration::from_millis(80));
            PostMessageW(hwnd, WM_LBUTTONDOWN, 0x0001, pos);
            std::thread::sleep(Duration::from_millis(80));
            PostMessageW(hwnd, WM_LBUTTONUP, 0, pos);
        }
    }
    log::info!("cursor at {x},{y} (screen {},{}) click={click}", p.x, p.y);
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

fn click(x: u16, y: u16) {
    let Some(hwnd) = game_window() else {
        log::warn!("click: no game window");
        return;
    };
    let pos = ((y as u32) << 16 | x as u32) as LPARAM;
    unsafe {
        PostMessageW(hwnd, WM_MOUSEMOVE, 0, pos);
        std::thread::sleep(Duration::from_millis(80));
        PostMessageW(hwnd, WM_LBUTTONDOWN, 0x0001 /* MK_LBUTTON */, pos);
        std::thread::sleep(Duration::from_millis(60));
        PostMessageW(hwnd, WM_LBUTTONUP, 0, pos);
    }
    log::info!("click at {x},{y}");
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
