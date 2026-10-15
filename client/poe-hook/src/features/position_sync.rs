//! Dev bridge for movement (until the server simulates movement exactly, M4): reports the
//! player's position and current action to the instance server every simulation frame.
//!
//! The client runs in lockstep, but the server's pathfinding and collision only approximate
//! the client's, so the two can drift apart near walls and the client snaps back when the
//! server next sends a position. With these reports the server knows where the client really
//! is and sends that, so nothing snaps.
//!
//! Transport: one UDP datagram per frame to the game's loopback connection to the instance
//! server (`position_sync_port`; the server listens for UDP on the same port), so it only ever
//! goes to a local server. Without a configured port, the last loopback connection is used,
//! which breaks when the game later connects to another local service.
//!
//! Payload (little-endian, 32 bytes): `b"PSY2", u16 our TCP port of that connection,
//! u16 flags, u32 frame counter, f32 grid x, f32 grid y, u16 current action type,
//! u16 second action type`, then the cursor: `i16 x, i16 y` in the game window's client area
//! and `u16 width, u16 height` of it (zero when unknown); the server turns the player toward
//! the cursor from it.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering},
};

use windows_sys::Win32::Networking::WinSock::{
    AF_INET, INVALID_SOCKET, IPPROTO_UDP, SOCK_DGRAM, SOCKADDR, SOCKADDR_IN, SOCKET, getsockname,
    sendto, socket,
};

pub static ENABLED: AtomicBool = AtomicBool::new(false);
/// The instance server's port (0: any loopback connection).
pub static PORT: AtomicU16 = AtomicU16::new(0);

/// Last loopback connection: destination, socket and our local port (0 until known).
static TARGET: Mutex<Option<(SOCKADDR_IN, SOCKET, u16)>> = Mutex::new(None);
static SOCK: AtomicUsize = AtomicUsize::new(INVALID_SOCKET);

/// Called by the connect hook for loopback connections, after the connect call.
pub fn record_connect(sock: SOCKET, dest: &SOCKADDR_IN) {
    let wanted = PORT.load(Ordering::Relaxed);
    if wanted != 0 && u16::from_be(dest.sin_port) != wanted {
        return;
    }
    // The game connects non-blocking: under Wine the local port can still be 0 here, so it
    // is looked up again when reporting until it is known.
    let port = local_port(sock);
    log::info!(
        "position sync: target port {} (local port {port}, socket {sock:#x})",
        u16::from_be(dest.sin_port)
    );
    *TARGET.lock().unwrap() = Some((*dest, sock, port));
}

fn local_port(sock: SOCKET) -> u16 {
    let mut local: SOCKADDR_IN = unsafe { std::mem::zeroed() };
    let mut len = size_of::<SOCKADDR_IN>() as i32;
    let ok = unsafe { getsockname(sock, &mut local as *mut _ as *mut SOCKADDR, &mut len) } == 0;
    if ok { u16::from_be(local.sin_port) } else { 0 }
}

/// One report for the player entity (from the per-frame entity tick).
pub fn report(entity: usize, frame: u32) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let (dest, local_port) = {
        let mut target = TARGET.lock().unwrap();
        let Some((dest, sock, port)) = target.as_mut() else {
            return;
        };
        if *port == 0 {
            *port = local_port(*sock);
            if *port != 0 {
                log::info!("position sync: local port {} (resolved late)", *port);
            }
        }
        (*dest, *port)
    };
    if local_port == 0 {
        return;
    }
    use crate::mem::read;
    let Some(comps) = read::<usize>(entity + 0x10) else {
        return;
    };
    let (Some(pos), Some(actor)) = (read::<usize>(comps), read::<usize>(comps + 9 * 8)) else {
        return;
    };
    const CELL: f32 = 250.0 / 23.0;
    let x = read::<f32>(pos + 0x490).unwrap_or(0.0) / CELL;
    let y = read::<f32>(pos + 0x494).unwrap_or(0.0) / CELL;
    let action = |off: usize| {
        read::<usize>(actor + off)
            .filter(|&a| a != 0)
            .and_then(|a| read::<u16>(a + 0x120))
            .unwrap_or(0)
    };
    // Actor +0x2a2 bit 5: sprint key held (set by the server's 0x2e7).
    let flags = read::<u8>(actor + 0x2a2).unwrap_or(0) as u16;
    let mut msg = Vec::with_capacity(32);
    msg.extend_from_slice(b"PSY2");
    msg.extend_from_slice(&local_port.to_le_bytes());
    msg.extend_from_slice(&flags.to_le_bytes());
    msg.extend_from_slice(&frame.to_le_bytes());
    msg.extend_from_slice(&x.to_le_bytes());
    msg.extend_from_slice(&y.to_le_bytes());
    msg.extend_from_slice(&action(0x238).to_le_bytes());
    msg.extend_from_slice(&action(0x240).to_le_bytes());
    let (cx, cy, w, h) = cursor().unwrap_or_default();
    msg.extend_from_slice(&cx.to_le_bytes());
    msg.extend_from_slice(&cy.to_le_bytes());
    msg.extend_from_slice(&w.to_le_bytes());
    msg.extend_from_slice(&h.to_le_bytes());
    let mut s = SOCK.load(Ordering::Relaxed);
    if s == INVALID_SOCKET {
        s = unsafe { socket(AF_INET as i32, SOCK_DGRAM, IPPROTO_UDP) };
        if s == INVALID_SOCKET {
            return;
        }
        SOCK.store(s, Ordering::Relaxed);
    }
    unsafe {
        sendto(
            s,
            msg.as_ptr(),
            msg.len() as i32,
            0,
            &dest as *const SOCKADDR_IN as *const SOCKADDR,
            size_of::<SOCKADDR_IN>() as i32,
        )
    };
}

/// The cursor in the game window's client area and the area's size: (x, y, width, height).
fn cursor() -> Option<(i16, i16, u16, u16)> {
    use windows_sys::Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::ScreenToClient,
        UI::WindowsAndMessaging::{GetClientRect, GetCursorPos},
    };
    static WINDOW: AtomicUsize = AtomicUsize::new(0);
    let mut hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd == 0 {
        hwnd = super::auto_keys::game_window()? as usize;
        WINDOW.store(hwnd, Ordering::Relaxed);
    }
    let hwnd = hwnd as HWND;
    let mut p = POINT { x: 0, y: 0 };
    let mut r: RECT = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetCursorPos(&mut p) != 0
            && ScreenToClient(hwnd, &mut p) != 0
            && GetClientRect(hwnd, &mut r) != 0
    };
    ok.then(|| {
        (
            p.x.clamp(-32768, 32767) as i16,
            p.y.clamp(-32768, 32767) as i16,
            (r.right - r.left).clamp(0, 65535) as u16,
            (r.bottom - r.top).clamp(0, 65535) as u16,
        )
    })
}
