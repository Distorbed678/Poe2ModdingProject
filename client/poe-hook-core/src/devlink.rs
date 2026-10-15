//! Debug link with our instance server (private mode only; never while read-only).
//!
//! The server sends `PDB1` datagrams (`key\tvalue` lines: its simulation state, every second
//! tick, and `ack` lines for commands) to the UDP socket the position reports come from;
//! commands go back as `PDC1`, `u16 our TCP port of the session`, text (`teleport x y`,
//! `turn_cell natural|own|heading`, `turn_timing on|off`, `face_cursor on|off`).
//! Each state is paired with the client's own position at that moment for the Desync tab.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

use windows_sys::Win32::Networking::WinSock::{
    SO_RCVTIMEO, SOCKADDR, SOCKADDR_IN, SOCKET, SOL_SOCKET, recvfrom, sendto, setsockopt,
};

pub struct State {
    pub at: Instant,
    pub values: Vec<(String, String)>,
}

#[derive(Clone, Copy)]
pub struct Sample {
    pub unix_ms: u64,
    pub client: (f32, f32),
    pub server: Option<(f32, f32)>,
    pub predicted: Option<(f32, f32)>,
    pub report: Option<(f32, f32)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static SAMPLES: Mutex<VecDeque<Sample>> = Mutex::new(VecDeque::new());
const KEEP: usize = 20_000;
/// A prediction further than this from the client (cells) counts as a desync.
pub const DESYNC_CELLS: f32 = 1.5;

pub fn start() {
    crate::hooks::spawn(receiver);
}

fn receiver() {
    let mut timeout_set: Option<SOCKET> = None;
    let mut buf = [0u8; 4096];
    while crate::hooks::running() {
        let Some((sock, _, _)) = crate::position_sync::link() else {
            crate::hooks::sleep(Duration::from_millis(250));
            continue;
        };
        if timeout_set != Some(sock) {
            let ms: u32 = 200;
            unsafe {
                setsockopt(
                    sock,
                    SOL_SOCKET,
                    SO_RCVTIMEO,
                    &ms as *const u32 as *const u8,
                    4,
                )
            };
            timeout_set = Some(sock);
        }
        let mut from: SOCKADDR_IN = unsafe { std::mem::zeroed() };
        let mut len = size_of::<SOCKADDR_IN>() as i32;
        let n = unsafe {
            recvfrom(
                sock,
                buf.as_mut_ptr(),
                buf.len() as i32,
                0,
                &mut from as *mut _ as *mut SOCKADDR,
                &mut len,
            )
        };
        if n <= 4 || &buf[..4] != b"PDB1" {
            continue;
        }
        let text = String::from_utf8_lossy(&buf[4..n as usize]).into_owned();
        handle(&text);
    }
}

fn handle(text: &str) {
    let values: Vec<(String, String)> = text
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('\t')?;
            Some((k.to_owned(), v.to_owned()))
        })
        .collect();
    if let Some((_, rest)) = values.iter().find(|(k, _)| k == "ack") {
        crate::ui::notice(format!("Server: {}", rest.replace('\t', " -> ")));
        return;
    }
    let map: HashMap<&str, &str> = values
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let pair = |k: &str| -> Option<(f32, f32)> {
        let mut it = map.get(k)?.split_whitespace();
        Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
    };
    if let Some(p) = crate::inspect::player() {
        let cell = 250.0 / 23.0;
        let sample = Sample {
            unix_ms: poe_hook_shared::unix_ms(),
            client: (p.world.0 / cell, p.world.1 / cell),
            server: pair("server_pos"),
            predicted: pair("predicted_pos"),
            report: pair("report_pos"),
        };
        if let Some(pred) = sample.predicted {
            let d = (pred.0 - sample.client.0).hypot(pred.1 - sample.client.1);
            if d > DESYNC_CELLS * 4.0 {
                static LAST: Mutex<Option<Instant>> = Mutex::new(None);
                let mut last = LAST.lock().unwrap();
                if last.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
                    *last = Some(Instant::now());
                    log::warn!("desync: server prediction {d:.1} cells from the client");
                }
            }
        }
        let mut s = SAMPLES.lock().unwrap();
        if s.len() >= KEEP {
            s.pop_front();
        }
        s.push_back(sample);
    }
    *STATE.lock().unwrap() = Some(State {
        at: Instant::now(),
        values,
    });
}

/// Sends a dev command to our server (not while read-only).
pub fn command(text: &str) -> bool {
    if crate::host::read_only() {
        return false;
    }
    let Some((sock, dest, port)) = crate::position_sync::link() else {
        return false;
    };
    let mut msg = b"PDC1".to_vec();
    msg.extend_from_slice(&port.to_le_bytes());
    msg.extend_from_slice(text.as_bytes());
    log::info!("dev command: {text}");
    unsafe {
        sendto(
            sock,
            msg.as_ptr(),
            msg.len() as i32,
            0,
            &dest as *const SOCKADDR_IN as *const SOCKADDR,
            size_of::<SOCKADDR_IN>() as i32,
        ) > 0
    }
}

/// The latest state (and how old it is).
pub fn state() -> Option<(Duration, Vec<(String, String)>)> {
    STATE
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| (s.at.elapsed(), s.values.clone()))
}

pub fn samples(since_ms: u64) -> Vec<Sample> {
    let s = SAMPLES.lock().unwrap();
    let start = s.partition_point(|x| x.unix_ms < since_ms);
    s.range(start..).copied().collect()
}

/// The server's idea of the player's cell position (its prediction), when fresh.
pub fn server_cell() -> Option<(f32, f32)> {
    let state = STATE.lock().unwrap();
    let s = state
        .as_ref()
        .filter(|s| s.at.elapsed() < Duration::from_secs(1))?;
    let v = s
        .values
        .iter()
        .find(|(k, _)| k == "predicted_pos")
        .map(|(_, v)| v)?;
    let mut it = v.split_whitespace();
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}
