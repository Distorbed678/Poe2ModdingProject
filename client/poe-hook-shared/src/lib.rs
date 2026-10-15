//! Code shared by the loader (`version.dll`) and the debug core (`poe_hook_core.dll`): the
//! configuration file, the C ABI between the two DLLs and guarded memory helpers.

pub mod abi;
pub mod config;
pub mod mem;
pub mod module;

use std::path::PathBuf;

/// Directory of the host executable (the game folder).
pub fn exe_dir() -> PathBuf {
    use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
    let mut buf = [0u16; 1024];
    let len =
        unsafe { GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    let exe = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
    exe.parent().map(PathBuf::from).unwrap_or_default()
}

/// Local time as `YYYY-MM-DD_HH-MM-SS` (session folder names).
pub fn local_timestamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// Milliseconds since the Unix epoch.
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
