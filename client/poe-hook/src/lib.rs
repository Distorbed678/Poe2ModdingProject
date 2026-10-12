//! poe-hook: client-side companion of the private server.
//!
//! Loaded by the PoE2 client as `version.dll` (see [`proxy`]). On attach it starts a thread
//! that reads `poe-hook.toml`, opens `poe-hook.log` and enables the configured
//! [`features`]. Game code is located through [`module`] (main-module info and byte-pattern
//! scanning), so features avoid hardcoded addresses where practical.

mod config;
mod features;
mod logger;
mod module;
mod proxy;

use std::{ffi::c_void, path::PathBuf};

use windows_sys::Win32::{
    Foundation::HINSTANCE,
    System::{LibraryLoader::GetModuleFileNameW, SystemServices::DLL_PROCESS_ATTACH},
};

#[unsafe(no_mangle)]
pub extern "system" fn DllMain(_module: HINSTANCE, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        // Never do real work under the loader lock.
        std::thread::spawn(init);
    }
    1
}

fn init() {
    let dir = exe_dir();
    let config = config::Config::load(&dir.join("poe-hook.toml"));
    logger::init(&dir.join("poe-hook.log"), config.log.level);
    log::info!("poe-hook {} loaded", env!("CARGO_PKG_VERSION"));
    let main = module::Module::main();
    log::info!("main module: base={:#x} size={:#x}", main.base, main.size);
    features::init_all(&config, &main);
}

/// Directory of the host executable (the game folder).
fn exe_dir() -> PathBuf {
    let mut buf = [0u16; 1024];
    let len =
        unsafe { GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    let exe = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
    exe.parent().map(PathBuf::from).unwrap_or_default()
}
