//! Socket-level trace of the process's network calls (connects are logged by the loader's
//! network guard).
//!
//! The ws2_32 exports (send / WSASend / recv / WSARecv) are hooked
//! inline. The game does not route all of its socket calls through its import table, which is
//! why inline hooks are used. Each call is logged with its length, a hexdump prefix and the
//! game-code return addresses on the stack (as Ghidra addresses), which pinpoints the code
//! that builds or parses a message. The data is whatever is on the wire, so it is plaintext
//! only while encryption is disabled or not yet active (e.g. the login key exchange).

use std::{ffi::c_void, sync::OnceLock};

use ilhook::x64::{HookType, Registers};
use windows_sys::Win32::{
    Networking::WinSock::{SOCKET, WSABUF},
    System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW},
};

use poe_hook_shared::module::Module;

pub struct Options {
    pub trace: bool,
    pub dump_bytes: usize,
}

static OPTIONS: OnceLock<Options> = OnceLock::new();
static MAIN: OnceLock<Module> = OnceLock::new();

pub fn init(main: &Module, options: Options) -> Result<(), String> {
    MAIN.set(*main).ok();
    OPTIONS.set(options).ok();
    let ws2 = unsafe {
        let name: Vec<u16> = "ws2_32.dll\0".encode_utf16().collect();
        let mut h = GetModuleHandleW(name.as_ptr());
        if h.is_null() {
            h = LoadLibraryW(name.as_ptr());
        }
        h
    };
    if ws2.is_null() {
        return Err("ws2_32.dll not loaded".into());
    }
    let hooks: [(&[u8], ilhook::x64::RetnRoutine); 4] = [
        (b"send\0", on_send),
        (b"WSASend\0", on_wsa_send),
        (b"recv\0", on_recv),
        (b"WSARecv\0", on_wsa_recv),
    ];
    for (name, routine) in hooks {
        let display = String::from_utf8_lossy(&name[..name.len() - 1]).into_owned();
        let addr = unsafe { GetProcAddress(ws2, name.as_ptr()) }
            .ok_or_else(|| format!("{display} not exported"))? as usize;
        crate::hooks::install(addr, HookType::Retn(routine), 0)
            .map_err(|e| format!("hook {display}: {e}"))?;
    }
    Ok(())
}

fn opts() -> &'static Options {
    OPTIONS.get().expect("net_trace initialised")
}

/// Game-code addresses on the hooked call's stack (mostly return addresses), as Ghidra
/// addresses. The stack is scanned raw because unwinding through the hook trampoline is not
/// possible.
fn backtrace(rsp: u64) -> String {
    let Some(main) = MAIN.get() else {
        return String::new();
    };
    poe_hook_shared::module::stack_refs(main, rsp as usize, 16)
        .iter()
        .map(|a| format!("{a:#x} {}", crate::symbols::fmt(*a)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn dump(data: &[u8]) -> String {
    let limit = opts().dump_bytes;
    let shown = &data[..data.len().min(limit)];
    let mut s: String = shown
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if data.len() > limit {
        s += " ...";
    }
    s
}

fn log_data(what: &str, rsp: u64, sock: u64, data: &[u8]) {
    if opts().trace && !data.is_empty() {
        log::info!(
            "{what} sock={sock} len={} stack=[{}]\n  {}",
            data.len(),
            backtrace(rsp),
            dump(data)
        );
    }
}

/// Gathers the first `len` bytes spread over a WSABUF array.
unsafe fn gather(bufs: *const WSABUF, count: u32, mut len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    for i in 0..count as usize {
        if len == 0 {
            break;
        }
        let b = unsafe { &*bufs.add(i) };
        let take = len.min(b.len as usize);
        out.extend_from_slice(unsafe { std::slice::from_raw_parts(b.buf, take) });
        len -= take;
    }
    out
}

type SendFn = unsafe extern "system" fn(SOCKET, *const u8, i32, i32) -> i32;
type RecvFn = unsafe extern "system" fn(SOCKET, *mut u8, i32, i32) -> i32;
type WsaIoFn = unsafe extern "system" fn(
    SOCKET,
    *const WSABUF,
    u32,
    *mut u32,
    u32,
    *mut c_void,
    *const c_void,
) -> i32;
type WsaRecvFn = unsafe extern "system" fn(
    SOCKET,
    *const WSABUF,
    u32,
    *mut u32,
    *mut u32,
    *mut c_void,
    *const c_void,
) -> i32;
unsafe extern "win64" fn on_send(regs: *mut Registers, original: usize, _: usize) -> usize {
    let _guard = crate::hook_guard!("net_trace::on_send");
    let r = unsafe { &*regs };
    let (sock, buf, len, flags) = (r.rcx, r.rdx as *const u8, r.r8 as i32, r.r9 as i32);
    if len > 0 {
        log_data("send", r.rsp, sock, unsafe {
            std::slice::from_raw_parts(buf, len as usize)
        });
    }
    let f: SendFn = unsafe { std::mem::transmute(original) };
    unsafe { f(sock as SOCKET, buf, len, flags) as usize }
}

unsafe extern "win64" fn on_wsa_send(regs: *mut Registers, original: usize, _: usize) -> usize {
    let _guard = crate::hook_guard!("net_trace::on_wsa_send");
    let r = unsafe { &*regs };
    let (sock, bufs, count, sent) = (r.rcx, r.rdx as *const WSABUF, r.r8 as u32, r.r9 as *mut u32);
    let (flags, ov, cb) = unsafe {
        (
            r.get_stack(5) as u32,
            r.get_stack(6) as *mut c_void,
            r.get_stack(7) as *const c_void,
        )
    };
    let total: usize = (0..count as usize)
        .map(|i| unsafe { (*bufs.add(i)).len as usize })
        .sum();
    log_data("WSASend", r.rsp, sock, &unsafe {
        gather(bufs, count, total)
    });
    let f: WsaIoFn = unsafe { std::mem::transmute(original) };
    unsafe { f(sock as SOCKET, bufs, count, sent, flags, ov, cb) as u32 as usize }
}

unsafe extern "win64" fn on_recv(regs: *mut Registers, original: usize, _: usize) -> usize {
    let _guard = crate::hook_guard!("net_trace::on_recv");
    let r = unsafe { &*regs };
    let (sock, buf, len, flags) = (r.rcx, r.rdx as *mut u8, r.r8 as i32, r.r9 as i32);
    let f: RecvFn = unsafe { std::mem::transmute(original) };
    let n = unsafe { f(sock as SOCKET, buf, len, flags) };
    if n > 0 {
        log_data("recv", r.rsp, sock, unsafe {
            std::slice::from_raw_parts(buf, n as usize)
        });
    }
    n as u32 as usize
}

unsafe extern "win64" fn on_wsa_recv(regs: *mut Registers, original: usize, _: usize) -> usize {
    let _guard = crate::hook_guard!("net_trace::on_wsa_recv");
    let r = unsafe { &*regs };
    let (sock, bufs, count, received) =
        (r.rcx, r.rdx as *const WSABUF, r.r8 as u32, r.r9 as *mut u32);
    let (flags, ov, cb) = unsafe {
        (
            r.get_stack(5) as *mut u32,
            r.get_stack(6) as *mut c_void,
            r.get_stack(7) as *const c_void,
        )
    };
    let f: WsaRecvFn = unsafe { std::mem::transmute(original) };
    let result = unsafe { f(sock as SOCKET, bufs, count, received, flags, ov, cb) };
    if result == 0 && !received.is_null() {
        // Completed immediately (pending overlapped completions are not visible here).
        let n = unsafe { *received } as usize;
        log_data("WSARecv", r.rsp, sock, &unsafe { gather(bufs, count, n) });
    }
    result as u32 as usize
}
