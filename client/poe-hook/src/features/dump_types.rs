//! Dumps the client's object-type table the first time it is used.
//!
//! `0x1416936c0(registry, out, key)` binary-searches a sorted array of 0x30-byte entries
//! `{u32 key, .., wchar* path at +8}` to resolve the object type key carried by the
//! server's add-object message (0x2f7). The dump (`object_types.txt` next to the exe, one
//! `key<TAB>path` per line) gives the key for each metadata path.

use std::{
    io::Write,
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use ilhook::x64::{CallbackOption, HookFlags, HookType, Hooker, Registers};

use crate::module::Module;

/// Ghidra address of the object-type lookup (0.5.5e).
const LOOKUP: usize = 0x1416936c0;
const ENTRY_SIZE: usize = 0x30;

static OUT: OnceLock<PathBuf> = OnceLock::new();
static DONE: AtomicBool = AtomicBool::new(false);

pub fn init(main: &Module, out: PathBuf) -> Result<(), String> {
    OUT.set(out).ok();
    let hooker = Hooker::new(
        main.from_ghidra(LOOKUP),
        HookType::JmpBack(on_lookup),
        CallbackOption::None,
        0,
        HookFlags::empty(),
    );
    let point = unsafe { hooker.hook() }.map_err(|e| format!("hook: {e:?}"))?;
    Box::leak(Box::new(point));
    Ok(())
}

unsafe fn wide(ptr: usize) -> String {
    if ptr < 0x10000 {
        return String::new();
    }
    let p = ptr as *const u16;
    let mut len = 0;
    while len < 512 && unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

unsafe extern "win64" fn on_lookup(regs: *mut Registers, _: usize) {
    let r = unsafe { &*regs };
    log::info!("object type lookup key={:#010x}", r.r8 as u32);
    if DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let registry = r.rcx as *const usize;
        let vec = *registry as *const usize;
        let (begin, end) = (*vec, *vec.add(1));
        let count = (end - begin) / ENTRY_SIZE;
        let Some(path) = OUT.get() else { return };
        let Ok(mut f) = std::fs::File::create(path) else {
            return;
        };
        for i in 0..count {
            let e = begin + i * ENTRY_SIZE;
            let key = *(e as *const u32);
            let name = wide(*((e + 8) as *const usize));
            let _ = writeln!(f, "{key:#010x}\t{name}");
        }
        log::info!("dumped {count} object types to {}", path.display());
    }
}
