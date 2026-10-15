//! poe-hook: the PoE2 client's companion DLL, loaded as `version.dll` (see [`proxy`]).
//!
//! This loader stays small and is never unloaded. In `DllMain` it reads `poe-hook.toml`, opens
//! the logs (next to the exe and in this process's session folder `logs/poe-hook/<time>_<pid>/`)
//! and installs the network guard (the client's crash-report helper connects out within
//! milliseconds of starting). A thread then hooks Vulkan for the overlay and loads the debug
//! core (`poe_hook_core.dll`), which holds the features and the overlay and can be hot-reloaded
//! ([`core`]).

mod core;
mod guard;
mod logsink;
mod proxy;
mod vulkan;
mod window;

use std::ffi::c_void;

use poe_hook_shared::{config::Config, module::Module};
use windows_sys::Win32::{Foundation::HINSTANCE, System::SystemServices::DLL_PROCESS_ATTACH};

#[unsafe(no_mangle)]
pub extern "system" fn DllMain(_module: HINSTANCE, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        // Under the loader lock: only file I/O and code patching of already-mapped modules
        // (ws2_32 is a static import of the game). Everything else runs on a thread.
        let dir = poe_hook_shared::exe_dir();
        let config = Config::load(&dir.join("poe-hook.toml"));
        let session = dir.join("logs").join("poe-hook").join(format!(
            "{}_{}",
            poe_hook_shared::local_timestamp(),
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&session);
        logsink::init(
            &[&dir.join("poe-hook.log"), &session.join("poe-hook.log")],
            config.log.level,
        );
        log::info!(
            "poe-hook {} loaded, mode {:?}, session {}",
            env!("CARGO_PKG_VERSION"),
            config.mode,
            session.display()
        );
        let main = Module::main();
        log::info!("main module: base={:#x} size={:#x}", main.base, main.size);
        match guard::init(&main, config.official(), config.features.block_external) {
            Ok(()) => log::info!("network guard: enabled"),
            Err(e) => log::error!("network guard: failed: {e}"),
        }
        std::thread::spawn(move || {
            logsink::flusher();
            if config.overlay.enabled {
                match vulkan::init() {
                    Ok(()) => log::info!("vulkan hooks: enabled"),
                    Err(e) => log::warn!("vulkan hooks: {e}"),
                }
            }
            core::start(dir, session, config.reload.auto);
        });
    }
    1
}
