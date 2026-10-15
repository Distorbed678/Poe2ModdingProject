//! poe-hook-core: the debug core, loaded (and hot-reloaded) by the `version.dll` loader.
//!
//! It holds every client feature (crypto bypass, packet capture, research hooks, position
//! sync, test input) and the in-game debug overlay. Features that change the game or talk to
//! our server are not started while the loader's read-only latch is on (official servers),
//! and check it again where they act, since the latch can switch on later.
//!
//! Everything installed here is undone in `stop` ([`hooks::remove_all`]).

mod actions;
mod auto_keys;
mod crash;
mod crypto_bypass;
mod devlink;
mod dump_types;
mod entities;
mod exit_trace;
mod gamedata;
mod gamelog;
mod hooks;
mod host;
mod inspect;
mod minimap;
mod net_trace;
mod overlay;
mod packets;
mod position_sync;
mod protocol;
mod scanner;
mod snapshot;
mod symbols;
mod timing;
mod trace_calls;
mod ui;
mod util;
mod watches;

use std::sync::{OnceLock, atomic::Ordering};

use poe_hook_shared::{
    abi::{ABI_VERSION, CoreApi, HostApi},
    config::Config,
    module::Module,
};

static CONFIG: OnceLock<Config> = OnceLock::new();

pub fn config() -> &'static Config {
    CONFIG.get().expect("config loaded in poe_core_start")
}

/// Entry point called by the loader after loading this DLL.
///
/// # Safety
/// `host` must point to a valid [`HostApi`] that outlives the core.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poe_core_start(host: *const HostApi) -> *const CoreApi {
    let host = unsafe { *host };
    if host.abi_version != ABI_VERSION {
        return std::ptr::null();
    }
    host::set(host);
    // Release builds abort on panic: log it first (otherwise the game just vanishes).
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        log::error!("PANIC: {info}\n{bt}");
    }));
    let loaded = Config::load(&poe_hook_shared::exe_dir().join("poe-hook.toml"));
    log::set_max_level(loaded.log.level);
    CONFIG.set(loaded).ok();
    log::info!(
        "debug core {} started (load #{}, read-only {})",
        env!("CARGO_PKG_VERSION"),
        host::reload_count(),
        host::read_only()
    );
    // Build the symbol table off the game's threads (the first stack print would otherwise).
    hooks::spawn(|| {
        symbols::fmt(0);
    });
    start_features(config(), &Module::main());
    overlay::init();
    static API: CoreApi = CoreApi {
        abi_version: ABI_VERSION,
        on_present,
        on_swapchain_destroy,
        on_window_message,
        stop,
    };
    &API
}

fn report(name: &str, enabled: bool, init: impl FnOnce() -> Result<(), String>) {
    if !enabled {
        log::info!("feature {name}: off");
        return;
    }
    match init() {
        Ok(()) => log::info!("feature {name}: on"),
        Err(e) => log::error!("feature {name}: failed: {e}"),
    }
}

fn start_features(config: &Config, main: &Module) {
    let f = &config.features;
    let read_only = host::read_only();
    let first_load = host::reload_count() == 0;
    // POE_HOOK_TRACE (comma-separated Ghidra addresses) adds to `trace_addresses` (RE tests).
    let mut addresses = f.trace_addresses.clone();
    if let Ok(extra) = std::env::var("POE_HOOK_TRACE") {
        addresses.extend(
            extra
                .split(',')
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty()),
        );
    }
    position_sync::ENABLED.store(f.position_sync && !read_only, Ordering::Relaxed);
    position_sync::PORT.store(f.position_sync_port, Ordering::Relaxed);
    dump_types::ENTITY_TRACE.store(f.entity_trace, Ordering::Relaxed);
    dump_types::MOVEMENT_LOG.store(
        f.movement_log || std::env::var_os("POE_HOOK_MOVEMENT_LOG").is_some(),
        Ordering::Relaxed,
    );
    report("crash_log", f.crash_log, || crash::init(main));
    report("exit_trace", f.crash_log, || exit_trace::init(main));
    // Official servers need the real handshake; the hook also passes through once the latch is
    // on.
    report("crypto_bypass", f.crypto_bypass && !read_only, || {
        crypto_bypass::init(main)
    });
    report("net_trace", f.net_trace, || {
        net_trace::init(
            main,
            net_trace::Options {
                trace: true,
                dump_bytes: f.net_trace_dump_bytes,
            },
        )
    });
    report("packets", config.packets.capture, || {
        packets::init(main)?;
        protocol::init(main)
    });
    let dump_path = poe_hook_shared::exe_dir().join("object_types.txt");
    report("dump_object_types", f.dump_object_types, || {
        dump_types::init(main, dump_path)
    });
    report("trace_calls", !addresses.is_empty(), || {
        trace_calls::init(main, &addresses, f.trace_limit.max(200))
    });
    // Test input is injection: never while read-only. The key timeline only runs once per
    // process (a reload must not press Enter again).
    report(
        "window_title",
        !read_only && std::env::var_os("POE_HOOK_WINDOW_TITLE").is_some(),
        auto_keys::init_window_title,
    );
    let keys = if first_load {
        f.auto_keys.clone()
    } else {
        Vec::new()
    };
    report("auto_keys", !read_only, || auto_keys::init(keys));
}

extern "C" fn on_present(queue: u64, info: *const std::ffi::c_void) -> u64 {
    timing::present();
    if !config().overlay.enabled {
        return 0;
    }
    overlay::present(queue, info)
}

extern "C" fn on_swapchain_destroy(swapchain: u64) {
    overlay::swapchain_destroy(swapchain);
}

extern "C" fn on_window_message(
    hwnd: isize,
    msg: u32,
    wparam: usize,
    lparam: isize,
    result: *mut isize,
) -> bool {
    match overlay::window_message(hwnd, msg, wparam, lparam) {
        Some(r) => {
            unsafe { result.write(r) };
            true
        }
        None => false,
    }
}

extern "C" fn stop() {
    log::info!("debug core stopping");
    overlay::stop();
    packets::stop();
    crash::stop();
    hooks::remove_all();
    position_sync::stop();
    log::info!("debug core stopped");
}
