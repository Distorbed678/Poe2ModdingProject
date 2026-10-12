//! Logs fatal exceptions with addresses translated to Ghidra's image base, so a crash in
//! the client can be looked up directly in the Ghidra project.

use std::sync::OnceLock;

use windows_sys::Win32::{
    Foundation::{
        EXCEPTION_ACCESS_VIOLATION, EXCEPTION_ILLEGAL_INSTRUCTION, EXCEPTION_INT_DIVIDE_BY_ZERO,
        EXCEPTION_PRIV_INSTRUCTION, EXCEPTION_STACK_OVERFLOW,
    },
    System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS},
};

use crate::module::Module;

const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

static MAIN: OnceLock<Module> = OnceLock::new();

pub fn init(main: &Module) -> Result<(), String> {
    MAIN.set(*main).ok();
    let handle = unsafe { AddVectoredExceptionHandler(1, Some(handler)) };
    if handle.is_null() {
        return Err("AddVectoredExceptionHandler failed".into());
    }
    Ok(())
}

unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    let (record, context) = unsafe { (&*(*info).ExceptionRecord, &*(*info).ContextRecord) };
    let code = record.ExceptionCode;
    let fatal = [
        EXCEPTION_ACCESS_VIOLATION,
        EXCEPTION_ILLEGAL_INSTRUCTION,
        EXCEPTION_INT_DIVIDE_BY_ZERO,
        EXCEPTION_PRIV_INSTRUCTION,
        EXCEPTION_STACK_OVERFLOW,
    ];
    if !fatal.contains(&code) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let main = MAIN.get().copied();
    let fmt = |addr: usize| match main {
        Some(m) if m.contains(addr) => format!("{addr:#x} (ghidra {:#x})", m.to_ghidra(addr)),
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
        let frames: Vec<String> = crate::module::stack_refs(&m, context.Rsp as usize, 16)
            .iter()
            .map(|a| format!("{a:#x}"))
            .collect();
        msg += &format!("\n  stack refs: {}", frames.join(" "));
    }
    log::error!("{msg}");
    EXCEPTION_CONTINUE_SEARCH
}
