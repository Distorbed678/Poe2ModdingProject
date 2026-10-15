//! Guarded reads of game memory (for diagnostics on structures whose layout is not known
//! for sure): every read is checked against committed memory first.

use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEMORY_BASIC_INFORMATION, VirtualQuery};

/// Whether `len` bytes at `addr` are committed memory (cheap guard before raw reads).
pub fn readable(addr: usize, len: usize) -> bool {
    if addr < 0x10000 {
        return false;
    }
    let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let n = unsafe {
        VirtualQuery(
            addr as *const _,
            &mut mbi,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    n != 0 && mbi.State == MEM_COMMIT && addr + len <= mbi.BaseAddress as usize + mbi.RegionSize
}

pub fn read<T: Copy>(addr: usize) -> Option<T> {
    readable(addr, size_of::<T>()).then(|| unsafe { (addr as *const T).read_unaligned() })
}

pub fn c_string(addr: usize, max: usize) -> Option<String> {
    if !readable(addr, 1) {
        return None;
    }
    let mut out = Vec::new();
    for i in 0..max {
        let b: u8 = read(addr + i)?;
        if b == 0 {
            break;
        }
        out.push(b);
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// NUL-terminated UTF-16 string at `addr` (at most `max` units).
pub fn wide_string(addr: usize, max: usize) -> Option<String> {
    let mut out = Vec::new();
    for i in 0..max {
        let u: u16 = read(addr + i * 2)?;
        if u == 0 {
            break;
        }
        out.push(u);
    }
    Some(String::from_utf16_lossy(&out))
}
