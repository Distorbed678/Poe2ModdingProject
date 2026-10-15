//! Logs fatal exceptions with addresses translated to Ghidra's image base, so a crash in
//! the client can be looked up directly in the Ghidra project.
//!
//! C++ exceptions (code `0xE06D7363`) are logged too, with the thrown type's name and its
//! `what()` text when it is a `std::exception`: the client reports protocol errors that way
//! ("Unable to deserialise packet", "Protocol violation ...").

use std::sync::OnceLock;

use windows_sys::Win32::{
    Foundation::{
        EXCEPTION_ACCESS_VIOLATION, EXCEPTION_ILLEGAL_INSTRUCTION, EXCEPTION_INT_DIVIDE_BY_ZERO,
        EXCEPTION_PRIV_INSTRUCTION, EXCEPTION_STACK_OVERFLOW,
    },
    System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS},
};

use poe_hook_shared::{
    mem::{c_string, read},
    module::Module,
};

const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
/// MSVC C++ `throw`.
const CPP_EXCEPTION: i32 = 0xE06D7363u32 as i32;

static MAIN: OnceLock<Module> = OnceLock::new();

pub fn init(main: &Module) -> Result<(), String> {
    MAIN.set(*main).ok();
    let handle = unsafe { AddVectoredExceptionHandler(1, Some(handler)) };
    if handle.is_null() {
        return Err("AddVectoredExceptionHandler failed".into());
    }
    HANDLE.store(handle as usize, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

static HANDLE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Removes the handler (core stop).
pub fn stop() {
    let h = HANDLE.swap(0, std::sync::atomic::Ordering::SeqCst);
    if h != 0 {
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::RemoveVectoredExceptionHandler(
                h as *const _,
            )
        };
    }
}

unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    let _guard = crate::hook_guard!("crash::handler");
    let (record, context) = unsafe { (&*(*info).ExceptionRecord, &*(*info).ContextRecord) };
    let code = record.ExceptionCode;
    let fatal = [
        EXCEPTION_ACCESS_VIOLATION,
        EXCEPTION_ILLEGAL_INSTRUCTION,
        EXCEPTION_INT_DIVIDE_BY_ZERO,
        EXCEPTION_PRIV_INSTRUCTION,
        EXCEPTION_STACK_OVERFLOW,
    ];
    if code == CPP_EXCEPTION {
        log_cpp_exception(record, context.Rip as usize);
        return EXCEPTION_CONTINUE_SEARCH;
    }
    if !fatal.contains(&code) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let main = MAIN.get().copied();
    let fmt = |addr: usize| match main {
        Some(m) if m.contains(addr) => format!(
            "{addr:#x} (ghidra {:#x} {})",
            m.to_ghidra(addr),
            crate::symbols::fmt(m.to_ghidra(addr))
        ),
        _ => format!("{addr:#x}"),
    };
    let rip = context.Rip as usize;
    let mut msg = format!("exception {code:#x} at {}", fmt(rip));
    if code == EXCEPTION_ACCESS_VIOLATION && record.NumberParameters >= 2 {
        let op = match record.ExceptionInformation[0] {
            0 => "read",
            1 => "write",
            8 => "execute",
            _ => "access",
        };
        msg += &format!(" ({op} of {:#x})", record.ExceptionInformation[1]);
    }
    msg += &format!(
        "\n  rax={:#x} rbx={:#x} rcx={:#x} rdx={:#x} rsi={:#x} rdi={:#x} rsp={:#x} rbp={:#x}\n  r8={:#x} r9={:#x} r10={:#x} r11={:#x} r12={:#x} r13={:#x} r14={:#x} r15={:#x}",
        context.Rax,
        context.Rbx,
        context.Rcx,
        context.Rdx,
        context.Rsi,
        context.Rdi,
        context.Rsp,
        context.Rbp,
        context.R8,
        context.R9,
        context.R10,
        context.R11,
        context.R12,
        context.R13,
        context.R14,
        context.R15
    );
    // Game-code addresses on the stack: a cheap call-stack hint.
    if let Some(m) = main {
        let frames: Vec<String> = poe_hook_shared::module::stack_refs(&m, context.Rsp as usize, 16)
            .iter()
            .map(|a| format!("{a:#x} {}", crate::symbols::fmt(*a)))
            .collect();
        msg += &format!("\n  stack refs: {}", frames.join(" | "));
    }
    log::error!("{msg}");
    EXCEPTION_CONTINUE_SEARCH
}

/// `ExceptionInformation`: [magic, object, ThrowInfo*, image base].
fn log_cpp_exception(
    record: &windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_RECORD,
    rip: usize,
) {
    if record.NumberParameters < 4 {
        return;
    }
    let object = record.ExceptionInformation[1];
    let throw_info = record.ExceptionInformation[2];
    let image = record.ExceptionInformation[3];
    // ThrowInfo { attributes, pmfnUnwind, pForwardCompat, pCatchableTypeArray } (RVAs).
    let type_name = read::<u32>(throw_info + 0xc)
        .and_then(|cta| read::<u32>(image + cta as usize + 4))
        .and_then(|ct| read::<u32>(image + ct as usize + 4))
        .and_then(|td| c_string(image + td as usize + 0x10, 128))
        .unwrap_or_default();
    // std::exception { vtable, const char* what, bool owns }.
    let what = read::<usize>(object + 8)
        .and_then(|p| c_string(p, 300))
        .unwrap_or_default();
    let main = MAIN.get().copied();
    let mut msg = format!("C++ exception {type_name} what={what:?}");
    if let Some(m) = main {
        if m.contains(rip) {
            msg += &format!(" at ghidra {:#x}", m.to_ghidra(rip));
        }
        let frames: Vec<String> = poe_hook_shared::module::stack_refs(&m, unsafe { get_rsp() }, 16)
            .iter()
            .map(|a| format!("{a:#x} {}", crate::symbols::fmt(*a)))
            .collect();
        msg += &format!("\n  stack refs: {}", frames.join(" | "));
    }
    log::warn!("{msg}");
}

#[inline(always)]
unsafe fn get_rsp() -> usize {
    let rsp: usize;
    unsafe { std::arch::asm!("mov {}, rsp", out(reg) rsp) };
    rsp
}
