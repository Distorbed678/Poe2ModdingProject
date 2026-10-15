//! The C ABI between the loader (`version.dll`, never unloaded) and the debug core
//! (`poe_hook_core.dll`, reloadable). Only `#[repr(C)]` data and `extern "C"` functions cross
//! it: the two DLLs have separate Rust runtimes and allocators.
//!
//! The loader calls the core's `poe_core_start(&HostApi) -> *const CoreApi` after loading it,
//! forwards Vulkan presents and window messages to it, and calls `CoreApi::stop` before
//! unloading it. The core must undo everything it installed in `stop`.

use std::ffi::c_void;

/// Bumped whenever a struct or function below changes.
pub const ABI_VERSION: u32 = 2;
/// Name of the core's entry point.
pub const CORE_START: &[u8] = b"poe_core_start\0";
pub type CoreStart = unsafe extern "C" fn(host: *const HostApi) -> *const CoreApi;

/// Borrowed UTF-8 string.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Str {
    pub ptr: *const u8,
    pub len: usize,
}

// Only ever points at data that outlives its use (leaked strings, or a callback's arguments).
unsafe impl Send for Str {}
unsafe impl Sync for Str {}

impl Str {
    pub fn new(s: &str) -> Self {
        Self {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }

    /// # Safety
    /// The pointer must be valid for `len` bytes of UTF-8 for `'a`.
    pub unsafe fn as_str<'a>(&self) -> &'a str {
        if self.len == 0 {
            return "";
        }
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.ptr, self.len)) }
    }
}

/// One log record kept by the loader (log sink shared by both DLLs).
#[repr(C)]
pub struct LogRecord {
    pub seq: u64,
    pub unix_ms: u64,
    /// `log::Level as u32` (1 = error .. 5 = trace).
    pub level: u32,
    pub target: Str,
    pub message: Str,
}

/// A TCP connection the game opened (recorded by the loader's connect hook).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Connection {
    pub socket: u64,
    pub ip: [u8; 4],
    pub port: u16,
    /// Not loopback: an official (or other external) server.
    pub external: bool,
    /// Order of the connect calls (1, 2, ...).
    pub seq: u32,
    pub unix_ms: u64,
}

/// What the loader learned from the game's Vulkan calls. Handles are raw `u64`s.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VulkanState {
    /// Changes whenever the swapchain is (re)created.
    pub generation: u64,
    /// The real `vkGetInstanceProcAddr` (unhooked), for loading functions.
    pub get_instance_proc_addr: usize,
    pub instance: u64,
    pub physical_device: u64,
    pub device: u64,
    pub swapchain: u64,
    pub format: i32,
    pub width: u32,
    pub height: u32,
    /// `VkImageUsageFlags` the swapchain was created with (after the loader's additions).
    pub usage: u32,
    pub image_count: u32,
    pub images: [u64; 16],
    pub hwnd: isize,
    /// A queue family the game found able to present (-1: unknown).
    pub present_family: i32,
}

impl Default for VulkanState {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

pub type LogVisit = extern "C" fn(ctx: *mut c_void, record: &LogRecord);

/// Services the loader offers the core.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostApi {
    pub abi_version: u32,
    /// How many times a core was loaded before this one (0 = first).
    pub reload_count: u32,
    pub exe_dir: Str,
    /// This process's folder under `logs/poe-hook/` (log files, packets, snapshots).
    pub session_dir: Str,
    pub log: extern "C" fn(level: u32, target: Str, message: Str),
    /// Visits the kept log records with `seq >= since`, oldest first; returns the next `seq`.
    pub logs_since: extern "C" fn(since: u64, ctx: *mut c_void, visit: LogVisit) -> u64,
    /// The read-only latch: set in official mode or once an external server was connected.
    pub read_only: extern "C" fn() -> bool,
    /// Copies up to `max` recorded connections (newest last); returns how many were copied.
    pub connections: extern "C" fn(out: *mut Connection, max: usize) -> usize,
    pub vulkan: extern "C" fn(out: *mut VulkanState) -> bool,
    /// Queue family of a queue the game got (-1 if unknown).
    pub queue_family: extern "C" fn(queue: u64) -> i32,
    /// Asks the loader to reload the core (from the core's own UI; done on another thread).
    pub request_reload: extern "C" fn(),
}

/// Callbacks the core offers the loader.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CoreApi {
    pub abi_version: u32,
    /// Called inside `vkQueuePresentKHR` with the game's `VkPresentInfoKHR`. Returns a
    /// semaphore the present must wait on instead of the game's (0: present unchanged).
    pub on_present: extern "C" fn(queue: u64, info: *const c_void) -> u64,
    /// Called before the game destroys a swapchain.
    pub on_swapchain_destroy: extern "C" fn(swapchain: u64),
    /// A message for the game window. Returns true when the core consumed it (`*result`).
    pub on_window_message: extern "C" fn(
        hwnd: isize,
        msg: u32,
        wparam: usize,
        lparam: isize,
        result: *mut isize,
    ) -> bool,
    /// Undo everything (hooks, threads, Vulkan objects) before the loader unloads the core.
    pub stop: extern "C" fn(),
}
