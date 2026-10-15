//! Loads, unloads and hot-reloads the debug core (`poe_hook_core.dll` next to the exe).
//!
//! The core is loaded from a copy in the temp folder, so the file next to the exe can be
//! replaced while the game runs (`cargo xtask deploy-core`). A watcher thread reloads it when
//! that file changes (`[reload] auto`) or when the core asks for it (overlay button).
//!
//! Unloading: the core's `stop` undoes its hooks and threads and releases its Vulkan objects
//! (on the next present), then the core pointer is cleared under a write lock, which waits
//! for every loader → core call in flight, and the DLL is freed.

use std::{
    path::{Path, PathBuf},
    sync::{
        OnceLock, RwLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, SystemTime},
};

use poe_hook_shared::abi::{ABI_VERSION, CORE_START, CoreApi, CoreStart, HostApi, Str};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    System::LibraryLoader::{GetProcAddress, LoadLibraryW},
};

pub const CORE_FILE: &str = "poe_hook_core.dll";

struct Loaded {
    module: usize,
    api: *const CoreApi,
    copy: PathBuf,
}

// The API table lives in the loaded core; it is only used while the core is loaded.
unsafe impl Send for Loaded {}
unsafe impl Sync for Loaded {}

static CORE: RwLock<Option<Loaded>> = RwLock::new(None);
static RELOAD: AtomicBool = AtomicBool::new(false);
static LOADS: AtomicU32 = AtomicU32::new(0);
static HOST: OnceLock<HostApi> = OnceLock::new();

pub fn present(queue: u64, info: *const std::ffi::c_void) -> u64 {
    match &*CORE.read().unwrap() {
        Some(c) => (unsafe { &*c.api }.on_present)(queue, info),
        None => 0,
    }
}

pub fn swapchain_destroy(swapchain: u64) {
    if let Some(c) = &*CORE.read().unwrap() {
        (unsafe { &*c.api }.on_swapchain_destroy)(swapchain);
    }
}

pub fn window_message(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> Option<isize> {
    let core = CORE.read().unwrap();
    let c = core.as_ref()?;
    let mut result = 0;
    (unsafe { &*c.api }.on_window_message)(hwnd, msg, wparam, lparam, &mut result).then_some(result)
}

extern "C" fn host_request_reload() {
    RELOAD.store(true, Ordering::SeqCst);
}

/// Loads the core and starts the watcher thread (on the loader's thread).
pub fn start(exe_dir: PathBuf, session_dir: PathBuf, auto_reload: bool) {
    let leak = |p: &Path| -> Str { Str::new(Box::leak(p.display().to_string().into_boxed_str())) };
    HOST.set(HostApi {
        abi_version: ABI_VERSION,
        reload_count: 0,
        exe_dir: leak(&exe_dir),
        session_dir: leak(&session_dir),
        log: crate::logsink::host_log,
        logs_since: crate::logsink::host_logs_since,
        read_only: crate::guard::host_read_only,
        connections: crate::guard::host_connections,
        vulkan: crate::vulkan::host_vulkan,
        queue_family: crate::vulkan::host_queue_family,
        request_reload: host_request_reload,
    })
    .ok();
    let file = exe_dir.join(CORE_FILE);
    let mut stamp = modified(&file);
    load(&file);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = modified(&file);
            let changed = auto_reload && now.is_some() && now != stamp;
            if changed {
                // Let the writer finish.
                std::thread::sleep(Duration::from_millis(700));
                if modified(&file) != now {
                    continue;
                }
            }
            if changed || RELOAD.swap(false, Ordering::SeqCst) {
                stamp = modified(&file);
                log::info!("reloading the debug core");
                unload();
                load(&file);
            }
        }
    });
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn wide(s: &Path) -> Vec<u16> {
    s.as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain([0])
        .collect()
}

fn load(file: &Path) {
    if !file.exists() {
        log::warn!("{} missing: no debug core", file.display());
        return;
    }
    let n = LOADS.fetch_add(1, Ordering::SeqCst);
    let copy = std::env::temp_dir().join(format!("poe_hook_core-{}-{n}.dll", std::process::id()));
    if let Err(e) = std::fs::copy(file, &copy) {
        log::error!("copy {} -> {}: {e}", file.display(), copy.display());
        return;
    }
    let module = unsafe { LoadLibraryW(wide(&copy).as_ptr()) };
    if module.is_null() {
        log::error!("LoadLibrary {} failed", copy.display());
        return;
    }
    let Some(start) = (unsafe { GetProcAddress(module, CORE_START.as_ptr()) }) else {
        log::error!("{} has no poe_core_start", file.display());
        unsafe { FreeLibrary(module) };
        return;
    };
    let start: CoreStart = unsafe { std::mem::transmute(start) };
    let host = HOST.get().expect("host api set");
    // `reload_count` changes per load; the rest is fixed.
    let host: &'static HostApi = Box::leak(Box::new(HostApi {
        reload_count: n,
        ..*host
    }));
    let api = unsafe { start(host) };
    if api.is_null() || unsafe { (*api).abi_version } != ABI_VERSION {
        log::error!("debug core refused to start or has another ABI version");
        unsafe { FreeLibrary(module) };
        return;
    }
    *CORE.write().unwrap() = Some(Loaded {
        module: module as usize,
        api,
        copy,
    });
    log::info!("debug core loaded (load #{n})");
}

fn unload() {
    let api = match &*CORE.read().unwrap() {
        Some(c) => c.api,
        None => return,
    };
    (unsafe { &*api }.stop)();
    let Some(old) = CORE.write().unwrap().take() else {
        return;
    };
    unsafe { FreeLibrary(old.module as HMODULE) };
    let _ = std::fs::remove_file(&old.copy);
    log::info!("debug core unloaded");
}
