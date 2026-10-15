//! Logs who terminates the process: hooks `ExitProcess` and `TerminateProcess` and records
//! the exit code with the game-code addresses on the caller's stack (Ghidra addresses).
//! Useful when the client quits on purpose (failed protocol checks) without an exception.

use std::sync::OnceLock;

use ilhook::x64::{HookType, Registers};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

use poe_hook_shared::module::Module;

static MAIN: OnceLock<Module> = OnceLock::new();

pub fn init(main: &Module) -> Result<(), String> {
    MAIN.set(*main).ok();
    let kernel32: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
    let module = unsafe { GetModuleHandleW(kernel32.as_ptr()) };
    if module.is_null() {
        return Err("kernel32 not loaded".into());
    }
    for (name, routine) in [
        (
            &b"ExitProcess\0"[..],
            on_exit_process as ilhook::x64::JmpBackRoutine,
        ),
        (&b"TerminateProcess\0"[..], on_terminate_process),
    ] {
        let display = String::from_utf8_lossy(&name[..name.len() - 1]).into_owned();
        let addr = unsafe { GetProcAddress(module, name.as_ptr()) }
            .ok_or_else(|| format!("{display} not exported"))? as usize;
        crate::hooks::install(addr, HookType::JmpBack(routine), 0)
            .map_err(|e| format!("hook {display}: {e}"))?;
    }
    Ok(())
}

fn stack(rsp: u64) -> String {
    MAIN.get()
        .map(|m| {
            poe_hook_shared::module::stack_refs(m, rsp as usize, 24)
                .iter()
                .map(|a| format!("{a:#x} {}", crate::symbols::fmt(*a)))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

unsafe extern "win64" fn on_exit_process(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("exit_trace::on_exit_process");
    let r = unsafe { &*regs };
    log::error!("ExitProcess({}) stack=[{}]", r.rcx as u32, stack(r.rsp));
}

unsafe extern "win64" fn on_terminate_process(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("exit_trace::on_terminate_process");
    let r = unsafe { &*regs };
    log::error!(
        "TerminateProcess(handle={:#x}, code={}) stack=[{}]",
        r.rcx,
        r.rdx as u32,
        stack(r.rsp)
    );
}
