//! Information about loaded modules and byte-pattern scanning over their images.

use windows_sys::Win32::System::{
    LibraryLoader::GetModuleHandleW,
    ProcessStatus::{GetModuleInformation, MODULEINFO},
    Threading::GetCurrentProcess,
};

#[derive(Debug, Clone, Copy)]
pub struct Module {
    pub base: usize,
    pub size: usize,
}

impl Module {
    /// The game executable.
    pub fn main() -> Self {
        unsafe {
            let handle = GetModuleHandleW(std::ptr::null());
            let mut info: MODULEINFO = std::mem::zeroed();
            GetModuleInformation(
                GetCurrentProcess(),
                handle,
                &mut info,
                size_of::<MODULEINFO>() as u32,
            );
            Self {
                base: handle as usize,
                size: info.SizeOfImage as usize,
            }
        }
    }

    pub fn contains(&self, addr: usize) -> bool {
        (self.base..self.base + self.size).contains(&addr)
    }

    /// Converts a runtime address to the Ghidra address (image base 0x140000000).
    pub fn to_ghidra(&self, addr: usize) -> usize {
        addr - self.base + 0x1_4000_0000
    }

    /// Converts a Ghidra address to a runtime address.
    #[allow(dead_code)]
    pub fn from_ghidra(&self, addr: usize) -> usize {
        addr - 0x1_4000_0000 + self.base
    }

    /// Finds all matches of an IDA-style pattern (`"48 8b ?? 05"`) in the image.
    pub fn scan(&self, pattern: &str) -> Vec<usize> {
        let pat = parse_pattern(pattern);
        if pat.is_empty() {
            return Vec::new();
        }
        let image = unsafe { std::slice::from_raw_parts(self.base as *const u8, self.size) };
        image
            .windows(pat.len())
            .enumerate()
            .filter(|(_, w)| w.iter().zip(&pat).all(|(b, p)| p.is_none_or(|p| p == *b)))
            .map(|(i, _)| self.base + i)
            .collect()
    }
}

fn parse_pattern(pattern: &str) -> Vec<Option<u8>> {
    pattern
        .split_whitespace()
        .map(|t| u8::from_str_radix(t, 16).ok())
        .collect()
}

/// Pointer-sized values on the current thread's stack from `rsp` upwards that point into
/// `module`, converted to Ghidra addresses. Bounded by the thread's stack limits.
pub fn stack_refs(module: &Module, rsp: usize, max: usize) -> Vec<usize> {
    let (mut low, mut high) = (0usize, 0usize);
    unsafe {
        windows_sys::Win32::System::Threading::GetCurrentThreadStackLimits(&mut low, &mut high)
    };
    if rsp < low || rsp >= high {
        return Vec::new();
    }
    (rsp..high)
        .step_by(8)
        .map(|slot| unsafe { (slot as *const usize).read() })
        .filter(|&v| module.contains(v))
        .take(max)
        .map(|v| module.to_ghidra(v))
        .collect()
}
