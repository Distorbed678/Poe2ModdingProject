//! Debug link with our instance server (private mode only; never while read-only).
//!
//! The server sends `PDB1` datagrams (`key\tvalue` lines: its simulation state, every second
//! tick, and `ack` lines for commands) to the UDP socket the position reports come from;
//! commands go back as `PDC1`, `u16 our TCP port of the session`, text: a server command
//! line (the server's registry, which it sends as `cmds` datagrams; `registry` asks for it
//! again). `instances` datagrams carry the server's instance list for the Instances tab.
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

/// One argument of a server command (`commands::wire` on the server).
#[derive(Clone, Debug)]
pub struct CmdArg {
    pub name: String,
    /// `int`, `text`, `rest`, `bool`, `choice=a|b`, `area`, `seed`, `topology`,
    /// `orientation`, `instance`, `player`, `minutes`.
    pub kind: String,
    pub optional: bool,
}

/// A server command, from the server's registry.
#[derive(Clone, Debug)]
pub struct Cmd {
    pub name: String,
    pub group: String,
    pub admin: bool,
    pub args: Vec<CmdArg>,
    pub help: String,
}

/// Who we are on the server.
#[derive(Clone, Debug, Default)]
pub struct Me {
    pub account: String,
    pub admin: bool,
    /// Chat command prefix.
    pub prefix: String,
}

/// An instance on the server.
#[derive(Clone, Debug)]
pub struct Inst {
    pub id: u32,
    pub area: String,
    pub seed: Option<u32>,
    pub topology: Option<u32>,
    pub orientation: Option<u8>,
    pub persistent: bool,
    /// Seconds it has been empty.
    pub empty_secs: Option<u64>,
    /// Ours.
    pub current: bool,
    pub players: Vec<String>,
}

static REGISTRY: Mutex<(Vec<Cmd>, Option<Me>)> = Mutex::new((Vec::new(), None));
static INSTANCES: Mutex<Option<(Instant, Vec<Inst>, u64)>> = Mutex::new(None);
static STAGING: Mutex<Vec<Inst>> = Mutex::new(Vec::new());
/// Command results (`ack`), newest last.
static RESULTS: Mutex<VecDeque<(u64, String, String)>> = Mutex::new(VecDeque::new());

fn opt<T: std::str::FromStr>(s: &str) -> Option<T> {
    if s.is_empty() { None } else { s.parse().ok() }
}

fn handle_registry(text: &str) {
    let mut reg = REGISTRY.lock().unwrap();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first().copied() {
            Some("cmds") if f.get(1) == Some(&"begin") => reg.0.clear(),
            Some("cmd") if f.len() >= 6 => {
                let args = f[4]
                    .split(',')
                    .filter(|a| !a.is_empty())
                    .filter_map(|a| {
                        let (name, kind) = a.split_once(':')?;
                        let optional = kind.ends_with('?');
                        Some(CmdArg { name: name.to_owned(), kind: kind.trim_end_matches('?').to_owned(), optional })
                    })
                    .collect();
                reg.0.push(Cmd { name: f[1].to_owned(), group: f[2].to_owned(), admin: f[3] == "1", args, help: f[5].to_owned() });
            }
            Some("you") if f.len() >= 4 => {
                reg.1 = Some(Me { account: f[1].to_owned(), admin: f[2] == "1", prefix: f[3].to_owned() });
            }
            _ => {}
        }
    }
}

fn handle_instances(text: &str) {
    let mut staging = STAGING.lock().unwrap();
    let mut idle = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first().copied() {
            Some("instances") if f.get(1) == Some(&"begin") => staging.clear(),
            Some("inst") if f.len() >= 10 => {
                if let Ok(id) = f[1].parse() {
                    staging.push(Inst {
                        id,
                        area: f[2].to_owned(),
                        seed: opt(f[3]),
                        topology: opt(f[4]),
                        orientation: opt(f[5]),
                        persistent: f[6] == "1",
                        empty_secs: opt(f[7]),
                        current: f[8] == "1",
                        players: f[9].split(',').filter(|p| !p.is_empty()).map(str::to_owned).collect(),
                    });
                }
            }
            Some("idle") => idle = f.get(1).and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    // The last datagram of a list ends with its `idle` line.
    if let Some(idle) = idle {
        *INSTANCES.lock().unwrap() = Some((Instant::now(), std::mem::take(&mut *staging), idle));
    }
}

/// The server's command registry and who we are (empty until the server sent it).
pub fn registry() -> (Vec<Cmd>, Option<Me>) {
    REGISTRY.lock().unwrap().clone()
}

/// The server's instances (how old the list is, the list, idle minutes before an empty
/// instance closes).
pub fn instances() -> Option<(Duration, Vec<Inst>, u64)> {
    INSTANCES.lock().unwrap().as_ref().map(|(at, l, idle)| (at.elapsed(), l.clone(), *idle))
}

/// Recent command results (unix ms, command, result), oldest first.
pub fn results() -> Vec<(u64, String, String)> {
    RESULTS.lock().unwrap().iter().cloned().collect()
}

fn handle(text: &str) {
    if text.starts_with("cmds\t") {
        handle_registry(text);
        return;
    }
    if text.starts_with("instances\t") {
        handle_instances(text);
        return;
    }
    let values: Vec<(String, String)> = text
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('\t')?;
            Some((k.to_owned(), v.to_owned()))
        })
        .collect();
    if let Some((_, rest)) = values.iter().find(|(k, _)| k == "ack") {
        crate::ui::notice(format!("Server: {}", rest.replace('\t', " -> ")));
        let (cmd, result) = rest.split_once('\t').unwrap_or((rest.as_str(), ""));
        let mut r = RESULTS.lock().unwrap();
        if r.len() >= 200 {
            r.pop_front();
        }
        r.push_back((poe_hook_shared::unix_ms(), cmd.to_owned(), result.to_owned()));
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
