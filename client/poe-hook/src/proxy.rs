//! `version.dll` proxy: every export forwards to the real system `version.dll`.
//!
//! The client imports `GetFileVersionInfoW`, `GetFileVersionInfoSizeW` and `VerQueryValueW`.
//! The rest are exported too, so any other module that resolves version.dll through us still
//! works.

use std::{ffi::c_void, sync::OnceLock};

use windows_sys::Win32::{
    Foundation::HMODULE,
    System::{
        LibraryLoader::{GetProcAddress, LoadLibraryW},
        SystemInformation::GetSystemDirectoryW,
    },
};

type Bool = i32;

fn real_module() -> HMODULE {
    static MODULE: OnceLock<usize> = OnceLock::new();
    *MODULE.get_or_init(|| unsafe {
        let mut buf = [0u16; 260];
        let len = GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) as usize;
        let mut path: Vec<u16> = buf[..len].to_vec();
        path.extend("\\version.dll\0".encode_utf16());
        LoadLibraryW(path.as_ptr()) as usize
    }) as HMODULE
}

fn real_proc(name: &'static [u8]) -> usize {
    let module = real_module();
    let addr = unsafe { GetProcAddress(module, name.as_ptr()) };
    match addr {
        Some(f) => f as usize,
        None => {
            log::error!(
                "real version.dll lacks {}",
                String::from_utf8_lossy(&name[..name.len() - 1])
            );
            0
        }
    }
}

/// Defines an exported function that forwards to the same-named export of the real DLL.
macro_rules! forward {
    ($( fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty; )*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn $name($($arg: $ty),*) -> $ret {
            static ADDR: OnceLock<usize> = OnceLock::new();
            let addr = *ADDR.get_or_init(|| real_proc(concat!(stringify!($name), "\0").as_bytes()));
            if addr == 0 {
                return Default::default();
            }
            let f: unsafe extern "system" fn($($ty),*) -> $ret = unsafe { std::mem::transmute(addr) };
            unsafe { f($($arg),*) }
        }
    )*};
}

forward! {
    fn GetFileVersionInfoA(file: *const u8, handle: u32, len: u32, data: *mut c_void) -> Bool;
    fn GetFileVersionInfoW(file: *const u16, handle: u32, len: u32, data: *mut c_void) -> Bool;
    fn GetFileVersionInfoExA(flags: u32, file: *const u8, handle: u32, len: u32, data: *mut c_void) -> Bool;
    fn GetFileVersionInfoExW(flags: u32, file: *const u16, handle: u32, len: u32, data: *mut c_void) -> Bool;
    fn GetFileVersionInfoSizeA(file: *const u8, handle: *mut u32) -> u32;
    fn GetFileVersionInfoSizeW(file: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoSizeExA(flags: u32, file: *const u8, handle: *mut u32) -> u32;
    fn GetFileVersionInfoSizeExW(flags: u32, file: *const u16, handle: *mut u32) -> u32;
    fn VerQueryValueA(block: *const c_void, sub: *const u8, buf: *mut *mut c_void, len: *mut u32) -> Bool;
    fn VerQueryValueW(block: *const c_void, sub: *const u16, buf: *mut *mut c_void, len: *mut u32) -> Bool;
    fn VerFindFileA(flags: u32, file: *const u8, win_dir: *const u8, app_dir: *const u8, cur_dir: *mut u8, cur_len: *mut u32, dest_dir: *mut u8, dest_len: *mut u32) -> u32;
    fn VerFindFileW(flags: u32, file: *const u16, win_dir: *const u16, app_dir: *const u16, cur_dir: *mut u16, cur_len: *mut u32, dest_dir: *mut u16, dest_len: *mut u32) -> u32;
    fn VerInstallFileA(flags: u32, src: *const u8, dest: *const u8, src_dir: *const u8, dest_dir: *const u8, cur_dir: *const u8, tmp: *mut u8, tmp_len: *mut u32) -> u32;
    fn VerInstallFileW(flags: u32, src: *const u16, dest: *const u16, src_dir: *const u16, dest_dir: *const u16, cur_dir: *const u16, tmp: *mut u16, tmp_len: *mut u32) -> u32;
    fn VerLanguageNameA(lang: u32, buf: *mut u8, len: u32) -> u32;
    fn VerLanguageNameW(lang: u32, buf: *mut u16, len: u32) -> u32;
}
