//! Packet capture: the plaintext of every login and instance message, read inside the client
//! (no cipher is broken; the same code runs on our and on official servers).
//!
//! - **Received** (`READ_MESSAGE` `0x1421c5950(registry, stream)`): the game's message reader.
//!   It reads the opcode from the stream's receive ring (decrypted on arrival), lets the
//!   registered handlers deserialize the body, then advances the ring's read offset (+0x268,
//!   ring at +0x298..+0x2a0) by exactly the message's size. The bytes between the old and new
//!   offset are the message. The caller tells the channel: `0x1420e6bc0` login,
//!   `0x142120b70` instance.
//! - **Sent** (`FLUSH_PLAINTEXT` `0x142198037`, inside the stream flush `0x142197fc0`, after
//!   its lock and before the cipher runs in place): the pending plaintext is
//!   `[+0x280] + [+0x260 .. +0x258]` of the stream (rbx). Message boundaries come from the game
//!   session send (`SESSION_SEND` `0x14211f880(session, packet)`, stream at session +0x20f8,
//!   opcode at packet +8), which records where each message starts and ends in the send
//!   buffer; bytes outside recorded messages (login sends) are kept as one unframed chunk.
//!
//! Messages go to the Packets tab (the last `[packets] keep`), to `packets.jsonl` and
//! `packets.txt` in the session folder, and into the per-opcode coverage
//! (`logs/poe-hook/coverage.tsv`, merged across sessions, split by official/private).
//! With `[packets] redact`, bodies that carry secrets are dropped before they are stored.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io::Write,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    time::{Duration, Instant},
};

use ilhook::x64::{HookType, Registers};
use poe_hook_shared::{mem::read, module::Module};
use poe_proto::{Direction, OpcodeTable};

const READ_MESSAGE: usize = 0x1421c5950;
const FLUSH_PLAINTEXT: usize = 0x142198037;
const SESSION_SEND: usize = 0x14211f880;
/// Callers of `READ_MESSAGE` (function ranges): login and instance sessions.
const LOGIN_READER: std::ops::Range<usize> = 0x1420e6bc0..0x1420e7136;
const INSTANCE_READER: std::ops::Range<usize> = 0x142120b70..0x1421215b5;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Channel {
    Login,
    Instance,
    Other,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Login => "login",
            Channel::Instance => "instance",
            Channel::Other => "other",
        }
    }
}

/// `In`: server → client. `Out`: client → server.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Dir {
    In,
    Out,
}

impl Dir {
    pub fn name(self) -> &'static str {
        match self {
            Dir::In => "in",
            Dir::Out => "out",
        }
    }

    fn sender(self) -> Direction {
        match self {
            Dir::In => Direction::Server,
            Dir::Out => Direction::Client,
        }
    }
}

pub struct Msg {
    pub seq: u64,
    pub unix_ms: u64,
    pub dir: Dir,
    pub channel: Channel,
    pub opcode: u16,
    /// Opcode width in bytes (the login reader can use 1-byte opcodes).
    pub opcode_len: usize,
    /// The message (opcode included); only the opcode when redacted.
    pub data: Vec<u8>,
    /// Size on the wire.
    pub len: usize,
    /// Whether the message boundaries are known (received, or sent through the session).
    pub framed: bool,
    pub redacted: bool,
    /// Captured while the read-only latch was on (official servers).
    pub official: bool,
}

impl Msg {
    pub fn name(&self) -> Option<&'static str> {
        name(self.channel, self.dir, self.opcode)
    }

    pub fn body(&self) -> &[u8] {
        self.data.get(self.opcode_len..).unwrap_or(&[])
    }
}

static MAIN: OnceLock<Module> = OnceLock::new();
static SEQ: AtomicU64 = AtomicU64::new(0);
static STORE: Mutex<VecDeque<Arc<Msg>>> = Mutex::new(VecDeque::new());
static STREAMS: Mutex<Option<HashMap<usize, Channel>>> = Mutex::new(None);
/// Per stream: messages written to the send buffer since its last flush (start, end, opcode;
/// `end` is `None` while the send is still serializing).
static SENDS: Mutex<Option<HashMap<usize, Vec<(usize, Option<usize>, u16)>>>> = Mutex::new(None);
static WRITER: Mutex<Option<Sender<Arc<Msg>>>> = Mutex::new(None);
static COVERAGE: Mutex<Option<Coverage>> = Mutex::new(None);

pub fn init(main: &Module) -> Result<(), String> {
    MAIN.set(*main).ok();
    tables();
    *COVERAGE.lock().unwrap() = Some(Coverage::load());
    restore();
    let (tx, rx) = channel();
    *WRITER.lock().unwrap() = Some(tx);
    crate::hooks::spawn(move || writer(rx));
    crate::hooks::install(
        main.from_ghidra(READ_MESSAGE),
        HookType::Retn(on_read_message),
        0,
    )?;
    crate::hooks::install(
        main.from_ghidra(SESSION_SEND),
        HookType::Retn(on_session_send),
        0,
    )?;
    crate::hooks::install(
        main.from_ghidra(FLUSH_PLAINTEXT),
        HookType::JmpBack(on_flush),
        0,
    )?;
    Ok(())
}

fn tables() -> &'static [(Channel, OpcodeTable); 2] {
    static TABLES: OnceLock<[(Channel, OpcodeTable); 2]> = OnceLock::new();
    TABLES.get_or_init(|| {
        let parse = |channel: &str, text: &str| {
            OpcodeTable::parse(channel, text).unwrap_or_else(|e| {
                log::error!("packets: {channel}.toml: {e}");
                OpcodeTable::parse(channel, "").expect("empty table")
            })
        };
        [
            (
                Channel::Login,
                parse("login", include_str!("../../../protocol/0.5.5e/login.toml")),
            ),
            (
                Channel::Instance,
                parse(
                    "instance",
                    include_str!("../../../protocol/0.5.5e/instance.toml"),
                ),
            ),
        ]
    })
}

/// The message's name in our protocol tables.
pub fn name(channel: Channel, dir: Dir, opcode: u16) -> Option<&'static str> {
    tables()
        .iter()
        .find(|(c, _)| *c == channel)
        .and_then(|(_, t)| t.name(dir.sender(), opcode))
}

/// Decoded fields of a known message.
pub fn describe(msg: &Msg) -> Option<String> {
    if msg.redacted {
        return Some(format!("<redacted: {} bytes>", msg.len - msg.opcode_len));
    }
    let d = poe_proto::describe::describe(msg.name()?, msg.dir.sender(), msg.body())?;
    Some(match d.text {
        Ok(text) if d.trailing == 0 => text,
        Ok(text) => format!("{text}\n({} bytes not decoded)", d.trailing),
        Err(e) => format!("decode error: {e} ({} bytes left)", d.trailing),
    })
}

/// Bodies dropped with `[packets] redact`: Steam ticket and machine hashes, session tokens,
/// account names, instance tickets and key material. The instance ticket travels in client
/// `0x000e`, sent right after `0x000f` Hello (not named in the protocol table yet).
fn secret(channel: Channel, dir: Dir, opcode: u16, name: Option<&str>) -> bool {
    (channel, dir, opcode) == (Channel::Instance, Dir::Out, 0x000e)
        || matches!(
            (channel, dir, name),
            (
                Channel::Login,
                Dir::Out,
                Some("Authenticate" | "ResumeSession")
            ) | (
                Channel::Login,
                Dir::In,
                Some("AuthenticationResult" | "InstanceDetails")
            ) | (Channel::Instance, Dir::Out, Some("Hello"))
        )
}

fn channel_of(stream: usize) -> Option<Channel> {
    if let Some(c) = STREAMS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&stream).copied())
    {
        return Some(c);
    }
    // Not seen yet: the stream's socket (+0) and its connection's port. The login servers are
    // the ones in gateway_list.txt; every other game connection is an instance.
    let socket = read::<u64>(stream)?;
    let conn = crate::host::connections()
        .into_iter()
        .rev()
        .find(|c| c.socket == socket)?;
    let c = if gateway_ports().contains(&conn.port) {
        Channel::Login
    } else {
        Channel::Instance
    };
    set_channel(stream, c);
    Some(c)
}

/// Ports of the login servers in `gateway_list.txt` (UTF-16, `region;name;host:port;...`).
fn gateway_ports() -> &'static Vec<u16> {
    static PORTS: OnceLock<Vec<u16>> = OnceLock::new();
    PORTS.get_or_init(|| {
        let bytes =
            std::fs::read(poe_hook_shared::exe_dir().join("gateway_list.txt")).unwrap_or_default();
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
            .lines()
            .filter_map(|l| l.split(';').nth(2)?.rsplit(':').next()?.trim().parse().ok())
            .collect()
    })
}

fn set_channel(stream: usize, channel: Channel) {
    STREAMS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(stream, channel);
}

fn record(dir: Dir, channel: Channel, data: Vec<u8>, opcode_len: usize, framed: bool) {
    if data.len() < opcode_len.max(1) {
        return;
    }
    let opcode = if opcode_len == 1 {
        data[0] as u16
    } else {
        u16::from_be_bytes([data[0], data[1]])
    };
    let len = data.len();
    let redacted =
        crate::config().packets.redact && secret(channel, dir, opcode, name(channel, dir, opcode));
    let data = if redacted {
        data[..opcode_len].to_vec()
    } else {
        data
    };
    let msg = Arc::new(Msg {
        seq: SEQ.fetch_add(1, Ordering::Relaxed) + 1,
        unix_ms: poe_hook_shared::unix_ms(),
        dir,
        channel,
        opcode,
        opcode_len,
        data,
        len,
        framed,
        redacted,
        official: crate::host::read_only(),
    });
    note_player(&msg);
    crate::timing::record(
        match dir {
            Dir::In => crate::timing::Series::PacketIn,
            Dir::Out => crate::timing::Series::PacketOut,
        },
        len as f32,
    );
    if let Some(c) = COVERAGE.lock().unwrap().as_mut() {
        c.add(&msg);
    }
    {
        let mut store = STORE.lock().unwrap();
        let keep = crate::config().packets.keep.max(100);
        while store.len() >= keep {
            store.pop_front();
        }
        store.push_back(msg.clone());
    }
    if let Some(tx) = WRITER.lock().unwrap().as_ref() {
        let _ = tx.send(msg);
    }
}

type ReadFn = unsafe extern "win64" fn(usize, usize) -> usize;

unsafe extern "win64" fn on_read_message(regs: *mut Registers, original: usize, _: usize) -> usize {
    let _guard = crate::hooks::enter();
    let r = unsafe { &*regs };
    let (registry, stream) = (r.rcx as usize, r.rdx as usize);
    let before = read::<usize>(stream + 0x268);
    let caller = read::<usize>(r.rsp as usize).unwrap_or(0);
    let f: ReadFn = unsafe { std::mem::transmute(original) };
    let ret = unsafe { f(registry, stream) };
    if ret & 0xff == 1
        && let Some(before) = before
    {
        capture_received(registry, stream, before, caller);
    }
    ret
}

fn capture_received(registry: usize, stream: usize, before: usize, caller: usize) {
    let (Some(base), Some(end), Some(after)) = (
        read::<usize>(stream + 0x298),
        read::<usize>(stream + 0x2a0),
        read::<usize>(stream + 0x268),
    ) else {
        return;
    };
    let size = end.wrapping_sub(base);
    if size == 0 || size > 0x1000_0000 {
        return;
    }
    let len = (after + size - before % size) % size;
    if len == 0 || !poe_hook_shared::mem::readable(base, size) {
        return;
    }
    let ring = unsafe { std::slice::from_raw_parts(base as *const u8, size) };
    let start = before % size;
    let mut data = Vec::with_capacity(len);
    if start + len <= size {
        data.extend_from_slice(&ring[start..start + len]);
    } else {
        data.extend_from_slice(&ring[start..]);
        data.extend_from_slice(&ring[..len - (size - start)]);
    }
    let channel = match MAIN.get().map(|m| m.to_ghidra(caller)) {
        Some(a) if LOGIN_READER.contains(&a) => Channel::Login,
        Some(a) if INSTANCE_READER.contains(&a) => Channel::Instance,
        _ => channel_of(stream).unwrap_or(Channel::Other),
    };
    set_channel(stream, channel);
    // Registry byte 0 set: 1-byte opcodes.
    let opcode_len = if read::<u8>(registry).unwrap_or(0) != 0 {
        1
    } else {
        2
    };
    record(Dir::In, channel, data, opcode_len, true);
}

type SendFn = unsafe extern "win64" fn(usize, usize, usize, usize) -> usize;

unsafe extern "win64" fn on_session_send(regs: *mut Registers, original: usize, _: usize) -> usize {
    let _guard = crate::hooks::enter();
    let r = unsafe { &*regs };
    let (session, packet) = (r.rcx as usize, r.rdx as usize);
    let stream = read::<usize>(session + 0x20f8).unwrap_or(0);
    let opcode = read::<u16>(packet + 8).unwrap_or(0);
    let start = read::<usize>(stream + 0x258);
    if let Some(start) = start {
        SENDS
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .entry(stream)
            .or_default()
            .push((start, None, opcode));
    }
    let f: SendFn = unsafe { std::mem::transmute(original) };
    let ret = unsafe { f(r.rcx as usize, r.rdx as usize, r.r8 as usize, r.r9 as usize) };
    if let (Some(start), Some(end)) = (start, read::<usize>(stream + 0x258))
        && let Some(list) = SENDS
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|m| m.get_mut(&stream))
        && let Some(open) = list
            .iter_mut()
            .rev()
            .find(|b| b.0 == start && b.1.is_none())
    {
        // A flush inside the send (buffer full) resets the offsets: unframed then.
        open.1 = (end > start).then_some(end);
    }
    ret
}

unsafe extern "win64" fn on_flush(regs: *mut Registers, _: usize) {
    let _guard = crate::hooks::enter();
    let stream = unsafe { (*regs).rbx } as usize;
    let (Some(base), Some(sent), Some(len)) = (
        read::<usize>(stream + 0x280),
        read::<usize>(stream + 0x260),
        read::<usize>(stream + 0x258),
    ) else {
        return;
    };
    if len <= sent
        || len - sent > 0x100_0000
        || !poe_hook_shared::mem::readable(base + sent, len - sent)
    {
        return;
    }
    let buf = unsafe { std::slice::from_raw_parts(base as *const u8, len) };
    let bounds = SENDS
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|m| m.remove(&stream))
        .unwrap_or_default();
    let mut channel = channel_of(stream);
    let mut at = sent;
    for (start, end, opcode) in bounds {
        // A send still serializing ends at the current length.
        let end = end.unwrap_or(len).min(len);
        if start < at || end <= start {
            continue;
        }
        if start > at {
            emit_out(stream, &mut channel, &buf[at..start], false);
        }
        let data = &buf[start..end];
        if data.len() >= 2 && u16::from_be_bytes([data[0], data[1]]) != opcode {
            log::debug!("packets: send boundary opcode {opcode:#06x} != buffer");
        }
        emit_out(stream, &mut channel, data, true);
        at = end;
    }
    if at < len {
        emit_out(stream, &mut channel, &buf[at..len], false);
    }
}

fn emit_out(stream: usize, channel: &mut Option<Channel>, data: &[u8], framed: bool) {
    if data.len() < 2 {
        return;
    }
    let c = *channel.get_or_insert_with(|| channel_of(stream).unwrap_or(Channel::Other));
    if framed {
        record(Dir::Out, c, data.to_vec(), 2, true);
        return;
    }
    // Sent outside the session (frame reports, login): split where the layout is known.
    let mut rest = data;
    while rest.len() >= 2 {
        let n = known_len(c, rest)
            .unwrap_or(rest.len())
            .clamp(2, rest.len());
        record(Dir::Out, c, rest[..n].to_vec(), 2, false);
        rest = &rest[n..];
    }
}

/// Length of the client message at the start of `data`, from the fixed body sizes in the
/// protocol table or from a decoder.
fn known_len(channel: Channel, data: &[u8]) -> Option<usize> {
    let opcode = u16::from_be_bytes([data[0], data[1]]);
    let table = &tables().iter().find(|(c, _)| *c == channel)?.1;
    if let Some(size) = table.client_body_size(opcode) {
        return Some(2 + size);
    }
    let name = table.name(Direction::Client, opcode)?;
    let d = poe_proto::describe::describe(name, Direction::Client, &data[2..])?;
    d.text.ok()?;
    Some(data.len() - d.trailing)
}

/// Messages since `since_seq` (Packets tab), oldest first.
pub fn since(since_seq: u64) -> Vec<Arc<Msg>> {
    let store = STORE.lock().unwrap();
    let start = store.partition_point(|m| m.seq <= since_seq);
    store.range(start..).cloned().collect()
}

pub fn text_line(m: &Msg) -> String {
    format!(
        "{} {:3} {:8} {:#06x} {:28} {:5} B{}{} {}",
        crate::util::clock(m.unix_ms),
        m.dir.name(),
        m.channel.name(),
        m.opcode,
        m.name().unwrap_or("?"),
        m.len,
        if m.framed { "" } else { " (unframed)" },
        if m.redacted { " (redacted)" } else { "" },
        crate::util::hex(&m.data, 48)
    )
}

/// PlayerEntity (`u8, u8 local index, u32 entity id`) names the controlled character.
fn note_player(m: &Msg) {
    if m.dir == Dir::In && m.channel == Channel::Instance && m.name() == Some("PlayerEntity") {
        if let Some(id) = m.body().get(2..6) {
            crate::dump_types::set_player_id(u32::from_be_bytes([id[0], id[1], id[2], id[3]]));
        }
    }
}

/// After a core reload: the messages of this session so far, from `packets.jsonl`.
fn restore() {
    let Ok(text) = std::fs::read_to_string(crate::host::session_dir().join("packets.jsonl")) else {
        return;
    };
    let field = |line: &str, key: &str| -> Option<String> {
        let at = line.find(&format!("\"{key}\":"))? + key.len() + 3;
        let rest = &line[at..];
        let rest = rest.strip_prefix('"').unwrap_or(rest);
        let end = rest.find(['"', ',', '}']).unwrap_or(rest.len());
        Some(rest[..end].to_owned())
    };
    let keep = crate::config().packets.keep.max(100);
    let lines: Vec<&str> = text.lines().collect();
    let mut store = STORE.lock().unwrap();
    for line in &lines[lines.len().saturating_sub(keep)..] {
        let parse = || -> Option<Msg> {
            let hex = field(line, "hex")?;
            let data: Vec<u8> = (0..hex.len() / 2)
                .filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
                .collect();
            let channel = match field(line, "chan")?.as_str() {
                "login" => Channel::Login,
                "instance" => Channel::Instance,
                _ => Channel::Other,
            };
            let opcode =
                u16::from_str_radix(field(line, "op")?.trim_start_matches("0x"), 16).ok()?;
            Some(Msg {
                seq: field(line, "seq")?.parse().ok()?,
                unix_ms: field(line, "t")?.parse().ok()?,
                dir: if field(line, "dir")? == "in" {
                    Dir::In
                } else {
                    Dir::Out
                },
                channel,
                opcode,
                opcode_len: if data.len() >= 2 && u16::from_be_bytes([data[0], data[1]]) == opcode {
                    2
                } else {
                    1
                },
                len: field(line, "len")?.parse().ok()?,
                framed: field(line, "framed")? == "true",
                redacted: field(line, "redacted")? == "true",
                official: field(line, "official")? == "true",
                data,
            })
        };
        if let Some(m) = parse() {
            note_player(&m);
            SEQ.fetch_max(m.seq, Ordering::Relaxed);
            store.push_back(Arc::new(m));
        }
    }
    if !store.is_empty() {
        log::info!(
            "packets: {} messages restored from this session",
            store.len()
        );
    }
}

fn json_line(m: &Msg) -> String {
    let hex: String = m.data.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{{\"seq\":{},\"t\":{},\"dir\":\"{}\",\"chan\":\"{}\",\"op\":\"{:#06x}\",\"name\":\"{}\",\"len\":{},\"framed\":{},\"redacted\":{},\"official\":{},\"hex\":\"{hex}\"}}",
        m.seq,
        m.unix_ms,
        m.dir.name(),
        m.channel.name(),
        m.opcode,
        m.name().unwrap_or(""),
        m.len,
        m.framed,
        m.redacted,
        m.official
    )
}

pub fn snapshot_text(since: u64) -> String {
    let store = STORE.lock().unwrap();
    let mut out = String::new();
    for m in store.iter().filter(|m| m.unix_ms >= since) {
        out.push_str(&text_line(m));
        out.push('\n');
    }
    out
}

/// Writes the session's packet files and, every few seconds, the coverage file.
fn writer(rx: Receiver<Arc<Msg>>) {
    let dir = crate::host::session_dir();
    let open = |name: &str| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(name))
            .ok()
            .map(std::io::BufWriter::new)
    };
    let (mut jsonl, mut text) = (open("packets.jsonl"), open("packets.txt"));
    let mut last_save = Instant::now();
    loop {
        let running = crate::hooks::running();
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(m) => {
                if let Some(f) = jsonl.as_mut() {
                    let _ = writeln!(f, "{}", json_line(&m));
                }
                if let Some(f) = text.as_mut() {
                    let _ = writeln!(f, "{}", text_line(&m));
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if running => {}
            Err(_) => break,
        }
        for f in [jsonl.as_mut(), text.as_mut()].into_iter().flatten() {
            let _ = f.flush();
        }
        if last_save.elapsed() > Duration::from_secs(10) {
            last_save = Instant::now();
            save_coverage();
        }
    }
    for f in [jsonl.as_mut(), text.as_mut()].into_iter().flatten() {
        let _ = f.flush();
    }
    save_coverage();
}

fn save_coverage() {
    if let Some(c) = COVERAGE.lock().unwrap().as_mut() {
        c.save();
    }
}

/// Core stop: no more messages are accepted (the writer drains and exits on its own).
pub fn stop() {
    WRITER.lock().unwrap().take();
}

// ---- coverage ----------------------------------------------------------------------------

/// Per opcode: how often each side sent it, on official and on our servers, across sessions.
#[derive(Clone, Default)]
pub struct Seen {
    pub official: u64,
    pub private: u64,
    pub session: u64,
    pub min_len: usize,
    pub max_len: usize,
}

pub type Key = (Channel, Dir, u16);

pub struct Coverage {
    pub entries: BTreeMap<Key, Seen>,
    dirty: bool,
}

fn coverage_path() -> std::path::PathBuf {
    poe_hook_shared::exe_dir()
        .join("logs")
        .join("poe-hook")
        .join("coverage.tsv")
}

impl Coverage {
    fn load() -> Self {
        let mut entries = BTreeMap::new();
        let text = std::fs::read_to_string(coverage_path()).unwrap_or_default();
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 8 {
                continue;
            }
            let channel = match f[0] {
                "login" => Channel::Login,
                "instance" => Channel::Instance,
                _ => Channel::Other,
            };
            let dir = if f[1] == "in" { Dir::In } else { Dir::Out };
            let Ok(op) = u16::from_str_radix(f[2].trim_start_matches("0x"), 16) else {
                continue;
            };
            entries.insert(
                (channel, dir, op),
                Seen {
                    official: f[4].parse().unwrap_or(0),
                    private: f[5].parse().unwrap_or(0),
                    session: 0,
                    min_len: f[6].parse().unwrap_or(0),
                    max_len: f[7].parse().unwrap_or(0),
                },
            );
        }
        Self {
            entries,
            dirty: false,
        }
    }

    fn add(&mut self, m: &Msg) {
        if !m.framed {
            return;
        }
        let e = self
            .entries
            .entry((m.channel, m.dir, m.opcode))
            .or_default();
        if m.official {
            e.official += 1;
        } else {
            e.private += 1;
        }
        e.min_len = if e.official + e.private == 1 || e.min_len == 0 {
            m.len
        } else {
            e.min_len.min(m.len)
        };
        e.max_len = e.max_len.max(m.len);
        e.session += 1;
        self.dirty = true;
    }

    fn save(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut out =
            String::from("channel\tdir\topcode\tname\tofficial\tprivate\tmin_len\tmax_len\n");
        for ((c, d, op), s) in &self.entries {
            out.push_str(&format!(
                "{}\t{}\t{op:#06x}\t{}\t{}\t{}\t{}\t{}\n",
                c.name(),
                d.name(),
                name(*c, *d, *op).unwrap_or(""),
                s.official,
                s.private,
                s.min_len,
                s.max_len
            ));
        }
        if let Err(e) = std::fs::write(coverage_path(), out) {
            log::warn!("packets: coverage: {e}");
        }
    }
}

/// A copy of the coverage table (Coverage tab).
pub fn coverage() -> Vec<(Key, Seen)> {
    COVERAGE
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.entries.iter().map(|(k, v)| (*k, v.clone())).collect())
        .unwrap_or_default()
}
