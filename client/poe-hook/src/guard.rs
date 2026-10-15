//! Network guard (installed in `DllMain`, before the game runs: its crash-report helper
//! connects out within milliseconds): hooks `connect` / `WSAConnect` to
//!
//! - refuse non-loopback connections in private mode with `block_external`, which keeps a
//!   client pointed at our server away from official services (shop, telemetry, crash upload);
//! - set the **read-only latch** the first time an external connection goes through (official
//!   play), or from the start in official mode. Read-only means the core sends, alters and
//!   injects nothing; it only observes;
//! - record every connection (the core's packet capture and position sync use the table).

use std::{
    ffi::c_void,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use ilhook::x64::{CallbackOption, HookFlags, HookType, Hooker, Registers};
use poe_hook_shared::{abi::Connection, module::Module};
use windows_sys::Win32::{
    Networking::WinSock::{
        SOCKADDR, SOCKADDR_IN, SOCKET, SOCKET_ERROR, WSAECONNREFUSED, WSASetLastError,
    },
    System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW},
};

static READ_ONLY: AtomicBool = AtomicBool::new(false);
static BLOCK_EXTERNAL: AtomicBool = AtomicBool::new(true);
static SEQ: AtomicU32 = AtomicU32::new(0);
static CONNECTIONS: Mutex<Vec<Connection>> = Mutex::new(Vec::new());
static MAIN: std::sync::OnceLock<Module> = std::sync::OnceLock::new();

pub fn init(main: &Module, official: bool, block_external: bool) -> Result<(), String> {
    MAIN.set(*main).ok();
    if official {
        READ_ONLY.store(true, Ordering::SeqCst);
        log::warn!("official mode: read-only latch on");
    }
    BLOCK_EXTERNAL.store(block_external && !official, Ordering::SeqCst);
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
    for name in [&b"connect\0"[..], b"WSAConnect\0"] {
        let display = String::from_utf8_lossy(&name[..name.len() - 1]).into_owned();
        let addr = unsafe { GetProcAddress(ws2, name.as_ptr()) }
            .ok_or_else(|| format!("{display} not exported"))? as usize;
        let hooker = Hooker::new(
            addr,
            HookType::Retn(on_connect),
            CallbackOption::None,
            0,
            HookFlags::empty(),
        );
        let point = unsafe { hooker.hook() }.map_err(|e| format!("hook {display}: {e:?}"))?;
        // The loader is never unloaded: hooks live for the whole process.
        Box::leak(Box::new(point));
    }
    Ok(())
}

pub fn read_only() -> bool {
    READ_ONLY.load(Ordering::SeqCst)
}

pub extern "C" fn host_read_only() -> bool {
    read_only()
}

pub extern "C" fn host_connections(out: *mut Connection, max: usize) -> usize {
    let list = CONNECTIONS.lock().unwrap();
    let start = list.len().saturating_sub(max);
    for (i, c) in list[start..].iter().enumerate() {
        unsafe { out.add(i).write(*c) };
    }
    list.len() - start
}

fn backtrace(rsp: u64) -> String {
    let Some(main) = MAIN.get() else {
        return String::new();
    };
    poe_hook_shared::module::stack_refs(main, rsp as usize, 12)
        .iter()
        .map(|a| format!("{a:#x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// connect and WSAConnect share the first three arguments; WSAConnect has four more.
type WsaConnectFn = unsafe extern "system" fn(
    SOCKET,
    *const SOCKADDR,
    i32,
    *const c_void,
    *const c_void,
    *const c_void,
    *const c_void,
) -> i32;

unsafe extern "win64" fn on_connect(regs: *mut Registers, original: usize, _: usize) -> usize {
    let r = unsafe { &*regs };
    let (sock, name, len) = (r.rcx, r.rdx as *const SOCKADDR, r.r8 as i32);
    let mut ip = [0u8; 4];
    let mut port = 0;
    let ipv4 = !name.is_null() && unsafe { (*name).sa_family } == 2;
    if ipv4 {
        let a = unsafe { &*(name as *const SOCKADDR_IN) };
        ip = unsafe { a.sin_addr.S_un.S_addr }.to_ne_bytes();
        port = u16::from_be(a.sin_port);
    }
    // IPv6 and others count as external.
    let external = !name.is_null() && !(ipv4 && ip[0] == 127);
    let target = format!("{}.{}.{}.{}:{port}", ip[0], ip[1], ip[2], ip[3]);
    if external && BLOCK_EXTERNAL.load(Ordering::SeqCst) {
        log::warn!(
            "blocked connect sock={sock} to {target} stack=[{}]",
            backtrace(r.rsp)
        );
        unsafe { WSASetLastError(WSAECONNREFUSED) };
        return SOCKET_ERROR as u32 as usize;
    }
    if external && !READ_ONLY.swap(true, Ordering::SeqCst) {
        log::warn!("external connection to {target}: read-only latch on");
    }
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    log::info!(
        "connect #{seq} sock={sock} {target}{} stack=[{}]",
        if external { " (external)" } else { "" },
        backtrace(r.rsp)
    );
    if !name.is_null() {
        let mut list = CONNECTIONS.lock().unwrap();
        if list.len() >= 256 {
            list.remove(0);
        }
        list.push(Connection {
            socket: sock,
            ip,
            port,
            external,
            seq,
            unix_ms: poe_hook_shared::unix_ms(),
        });
    }
    // Forward all seven argument slots; plain connect ignores the extra ones.
    let (a4, a5, a6, a7) = unsafe {
        (
            r.r9 as *const c_void,
            r.get_stack(5) as *const c_void,
            r.get_stack(6) as *const c_void,
            r.get_stack(7) as *const c_void,
        )
    };
    let f: WsaConnectFn = unsafe { std::mem::transmute(original) };
    unsafe { f(sock as SOCKET, name, len, a4, a5, a6, a7) as u32 as usize }
}
