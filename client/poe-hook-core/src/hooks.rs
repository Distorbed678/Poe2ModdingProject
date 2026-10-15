//! Hooks and threads owned by the core, so that `stop` can undo all of them before the loader
//! unloads the DLL.
//!
//! - [`install`] saves the target's first bytes before hooking; [`remove_all`] writes them back
//!   and leaks the hook's trampoline (a thread may still be inside it).
//! - Every hook callback starts with [`enter`]; [`remove_all`] waits until no callback is
//!   running before the DLL may go.
//! - Long-lived threads are started with [`spawn`], loop while [`running`] and are joined.

use std::{
    mem::ManuallyDrop,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use ilhook::x64::{CallbackOption, HookFlags, HookPoint, HookType, Hooker};
use windows_sys::Win32::System::{
    Diagnostics::Debug::FlushInstructionCache,
    Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect},
    Threading::GetCurrentProcess,
};

/// Bytes saved per hook (ilhook's jump is at most 14 bytes).
const SAVED: usize = 16;

struct Installed {
    addr: usize,
    saved: [u8; SAVED],
    _point: ManuallyDrop<HookPoint>,
}

// Only touched under the mutex; the hook point is never dropped (see module docs).
unsafe impl Send for Installed {}

static HOOKS: Mutex<Vec<Installed>> = Mutex::new(Vec::new());
static THREADS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());
static RUNNING: AtomicBool = AtomicBool::new(true);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Hooks `addr` (a runtime address). The hook stays until [`remove_all`].
pub fn install(addr: usize, kind: HookType, user: usize) -> Result<(), String> {
    let mut saved = [0u8; SAVED];
    unsafe { std::ptr::copy_nonoverlapping(addr as *const u8, saved.as_mut_ptr(), SAVED) };
    let point =
        unsafe { Hooker::new(addr, kind, CallbackOption::None, user, HookFlags::empty()).hook() }
            .map_err(|e| format!("{e:?}"))?;
    HOOKS.lock().unwrap().push(Installed {
        addr,
        saved,
        _point: ManuallyDrop::new(point),
    });
    Ok(())
}

/// Marks a hook callback as running (held for the whole callback) and times it for the hook
/// profiler ([`stats`]).
pub struct Guard {
    slot: usize,
    start: Instant,
    /// Time spent in the game's original function (Retn hooks), not ours.
    excluded: Duration,
}

impl Guard {
    /// Runs the hooked game function: its time is not counted as the hook's.
    pub fn original<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let s = Instant::now();
        let r = f();
        self.excluded += s.elapsed();
        r
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let ns = self
            .start
            .elapsed()
            .saturating_sub(self.excluded)
            .as_nanos() as u64;
        let s = &SLOTS[self.slot];
        s.calls.fetch_add(1, Ordering::Relaxed);
        s.total_ns.fetch_add(ns, Ordering::Relaxed);
        s.max_ns.fetch_max(ns, Ordering::Relaxed);
        if ns > SLOW_NS {
            slow_call(self.slot, ns);
        }
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Starts a named, timed callback; use [`crate::hook_guard!`].
pub fn enter(slot: usize) -> Guard {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    Guard {
        slot,
        start: Instant::now(),
        excluded: Duration::ZERO,
    }
}

/// `let _guard = hook_guard!("module::callback");` at the top of every hook callback.
#[macro_export]
macro_rules! hook_guard {
    ($name:literal) => {{
        static SLOT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        $crate::hooks::enter(*SLOT.get_or_init(|| $crate::hooks::register($name)))
    }};
}

// ---- profiler ----------------------------------------------------------------------------

/// A callback slower than this is reported (a game thread is blocked meanwhile).
const SLOW_NS: u64 = 50_000_000;
const MAX_SLOTS: usize = 128;

pub struct Slot {
    pub calls: std::sync::atomic::AtomicU64,
    pub total_ns: std::sync::atomic::AtomicU64,
    pub max_ns: std::sync::atomic::AtomicU64,
}

static SLOTS: [Slot; MAX_SLOTS] = [const {
    Slot {
        calls: std::sync::atomic::AtomicU64::new(0),
        total_ns: std::sync::atomic::AtomicU64::new(0),
        max_ns: std::sync::atomic::AtomicU64::new(0),
    }
}; MAX_SLOTS];
static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub fn register(name: &'static str) -> usize {
    let mut names = NAMES.lock().unwrap();
    if let Some(i) = names.iter().position(|n| *n == name) {
        return i;
    }
    names.push(name);
    (names.len() - 1).min(MAX_SLOTS - 1)
}

fn slow_call(slot: usize, ns: u64) {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let name = NAMES.lock().unwrap().get(slot).copied().unwrap_or("?");
    log::warn!("slow hook: {name} took {:.1} ms", ns as f64 / 1e6);
    let mut last = LAST.lock().unwrap();
    if last.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
        *last = Some(Instant::now());
        crate::ui::notice(format!(
            "Slow hook: {name} took {:.0} ms (Hooks tab)",
            ns as f64 / 1e6
        ));
    }
}

/// Per callback: name, calls, total and maximum time (ns).
pub fn stats() -> Vec<(&'static str, u64, u64, u64)> {
    let names = NAMES.lock().unwrap().clone();
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let s = &SLOTS[i.min(MAX_SLOTS - 1)];
            (
                *n,
                s.calls.load(Ordering::Relaxed),
                s.total_ns.load(Ordering::Relaxed),
                s.max_ns.load(Ordering::Relaxed),
            )
        })
        .collect()
}

pub fn reset_stats() {
    for s in &SLOTS {
        s.calls.store(0, Ordering::Relaxed);
        s.total_ns.store(0, Ordering::Relaxed);
        s.max_ns.store(0, Ordering::Relaxed);
    }
}

pub fn running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// Starts a thread that is joined on stop (it must return soon after [`running`] is false).
pub fn spawn(f: impl FnOnce() + Send + 'static) {
    let handle = std::thread::spawn(f);
    THREADS.lock().unwrap().push(handle);
}

/// Sleeps up to `total`, returning early (false) when the core is stopping.
pub fn sleep(total: Duration) -> bool {
    let end = Instant::now() + total;
    while running() {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
    false
}

/// Removes every hook, waits for running callbacks and joins the threads.
pub fn remove_all() {
    RUNNING.store(false, Ordering::SeqCst);
    let hooks = std::mem::take(&mut *HOOKS.lock().unwrap());
    // Newest first, in case two hooks share a function.
    for h in hooks.into_iter().rev() {
        unsafe {
            let mut old = 0;
            VirtualProtect(h.addr as *const _, SAVED, PAGE_EXECUTE_READWRITE, &mut old);
            std::ptr::copy_nonoverlapping(h.saved.as_ptr(), h.addr as *mut u8, SAVED);
            VirtualProtect(h.addr as *const _, SAVED, old, &mut old);
            FlushInstructionCache(GetCurrentProcess(), h.addr as *const _, SAVED);
        }
        // The trampoline stays allocated: `h._point` is never dropped.
        std::mem::forget(h);
    }
    let start = Instant::now();
    while IN_FLIGHT.load(Ordering::SeqCst) != 0 {
        if start.elapsed() > Duration::from_secs(60) {
            log::error!("hook callbacks still running after 60 s; unloading anyway");
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // A thread can be between a hook's jump and `enter`.
    std::thread::sleep(Duration::from_millis(100));
    for t in std::mem::take(&mut *THREADS.lock().unwrap()) {
        let _ = t.join();
    }
}
