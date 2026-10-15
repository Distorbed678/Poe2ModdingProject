//! "Logging breakpoints": hooks arbitrary game addresses (Ghidra addresses from
//! `poe-hook.toml`, `trace_addresses`) and logs the first argument registers plus the game
//! call stack every time one runs (up to `trace_limit` hits per address). A cheap dynamic
//! analysis aid that does not need a debugger.

use std::sync::{
    OnceLock,
    atomic::{AtomicU32, Ordering},
};

use ilhook::x64::{HookType, Registers};

use poe_hook_shared::module::Module;

static MAIN: OnceLock<Module> = OnceLock::new();
static LIMIT: OnceLock<u32> = OnceLock::new();

struct Target {
    ghidra: usize,
    hits: AtomicU32,
}

pub fn init(main: &Module, addresses: &[String], limit: u32) -> Result<(), String> {
    MAIN.set(*main).ok();
    LIMIT.set(limit).ok();
    for text in addresses {
        let ghidra = usize::from_str_radix(text.trim_start_matches("0x"), 16)
            .map_err(|e| format!("bad address {text}: {e}"))?;
        let runtime = main.from_ghidra(ghidra);
        let target: &'static Target = Box::leak(Box::new(Target {
            ghidra,
            hits: AtomicU32::new(0),
        }));
        crate::hooks::install(
            runtime,
            HookType::JmpBack(on_hit),
            target as *const Target as usize,
        )
        .map_err(|e| format!("hook {text}: {e}"))?;
        log::info!("tracing {ghidra:#x}");
    }
    Ok(())
}

unsafe extern "win64" fn on_hit(regs: *mut Registers, user: usize) {
    let _guard = crate::hook_guard!("trace_calls::on_hit");
    let target = unsafe { &*(user as *const Target) };
    let n = target.hits.fetch_add(1, Ordering::Relaxed);
    if n >= *LIMIT.get().unwrap_or(&20) {
        return;
    }
    let r = unsafe { &*regs };
    let stack = MAIN
        .get()
        .map(|m| {
            poe_hook_shared::module::stack_refs(m, r.rsp as usize, 20)
                .iter()
                .map(|a| format!("{a:#x} {}", crate::symbols::fmt(*a)))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    // [rax] as a Ghidra address when it points into the game (useful at `call [rax]`).
    let rax_target = MAIN
        .get()
        .and_then(|m| {
            let p = r.rax as usize;
            (p > 0x10000 && m.contains(p))
                .then(|| unsafe { (p as *const usize).read_unaligned() })
                .filter(|&v| m.contains(v))
                .map(|v| format!(" [rax]={:#x}", m.to_ghidra(v)))
        })
        .unwrap_or_default();
    // [[rcx+8]] slot 0: for a packet factory object, the handler that receives its packets.
    let handler = MAIN
        .get()
        .and_then(|m| {
            let read =
                |p: usize| (p > 0x10000).then(|| unsafe { (p as *const usize).read_unaligned() });
            let obj = read(r.rcx as usize + 8)?;
            let vtable = read(obj)?;
            let slot0 = read(vtable)?;
            m.contains(slot0)
                .then(|| format!(" handler={obj:#x} slot0={:#x}", m.to_ghidra(slot0)))
        })
        .unwrap_or_default();
    log::info!(
        "hit {:#x} #{n} rax={:#x}{rax_target}{handler} rcx={:#x} rdx={:#x} r8={:#x} r9={:#x} stack=[{stack}]",
        target.ghidra,
        r.rax,
        r.rcx,
        r.rdx,
        r.r8,
        r.r9
    );
}
