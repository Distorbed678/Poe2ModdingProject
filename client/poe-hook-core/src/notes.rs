//! The user's notes on packets: on an opcode ("what this message does") or on one captured
//! message. Kept across sessions in `logs/poe-hook/packet_notes.tsv` next to the exe (read by
//! Claude when working on the protocol), one note per line:
//! `kind  channel  dir  opcode  name  session  seq  unix_ms  note  hex`
//! (`kind` = `opcode` or `message`; `session`/`seq`/`hex` only for message notes, so a note
//! stays meaningful after the session folder is gone). Tabs and newlines in the text are
//! stored as spaces and `\n`.

use std::sync::Mutex;

use crate::packets::{Channel, Dir, Msg};

#[derive(Clone)]
pub struct Note {
    pub message: bool,
    pub channel: Channel,
    pub dir: Dir,
    pub opcode: u16,
    pub session: String,
    pub seq: u64,
    pub unix_ms: u64,
    pub text: String,
    pub hex: String,
}

static NOTES: Mutex<Option<Vec<Note>>> = Mutex::new(None);

fn path() -> std::path::PathBuf {
    poe_hook_shared::exe_dir()
        .join("logs")
        .join("poe-hook")
        .join("packet_notes.tsv")
}

fn escape(s: &str) -> String {
    s.replace('\t', " ").replace('\r', "").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n")
}

fn channel(s: &str) -> Channel {
    match s {
        "login" => Channel::Login,
        "instance" => Channel::Instance,
        _ => Channel::Other,
    }
}

fn with<T>(f: impl FnOnce(&mut Vec<Note>) -> T) -> T {
    let mut guard = NOTES.lock().unwrap();
    let notes = guard.get_or_insert_with(load);
    f(notes)
}

fn load() -> Vec<Note> {
    std::fs::read_to_string(path())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 9 {
                return None;
            }
            Some(Note {
                message: f[0] == "message",
                channel: channel(f[1]),
                dir: if f[2] == "in" { Dir::In } else { Dir::Out },
                opcode: u16::from_str_radix(f[3].trim_start_matches("0x"), 16).ok()?,
                session: f[5].to_owned(),
                seq: f[6].parse().unwrap_or(0),
                unix_ms: f[7].parse().unwrap_or(0),
                text: unescape(f[8]),
                hex: f.get(9).copied().unwrap_or("").to_owned(),
            })
        })
        .collect()
}

fn save(notes: &[Note]) {
    let mut out = String::from(
        "# kind\tchannel\tdir\topcode\tname\tsession\tseq\tunix_ms\tnote\thex (packet notes from the overlay)\n",
    );
    for n in notes {
        out.push_str(&format!(
            "{}\t{}\t{}\t{:#06x}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            if n.message { "message" } else { "opcode" },
            n.channel.name(),
            n.dir.name(),
            n.opcode,
            crate::packets::name(n.channel, n.dir, n.opcode).unwrap_or(""),
            n.session,
            n.seq,
            n.unix_ms,
            escape(&n.text),
            n.hex
        ));
    }
    if let Err(e) = std::fs::write(path(), out) {
        log::error!("packet notes: {e}");
    }
}

fn session_name() -> String {
    crate::host::session_dir()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The note on an opcode ("" if none).
pub fn opcode_note(channel: Channel, dir: Dir, opcode: u16) -> String {
    with(|notes| {
        notes
            .iter()
            .find(|n| !n.message && (n.channel, n.dir, n.opcode) == (channel, dir, opcode))
            .map(|n| n.text.clone())
            .unwrap_or_default()
    })
}

/// The note on one captured message of this session ("" if none).
pub fn message_note(m: &Msg) -> String {
    let session = session_name();
    with(|notes| {
        notes
            .iter()
            .find(|n| n.message && n.seq == m.seq && n.session == session)
            .map(|n| n.text.clone())
            .unwrap_or_default()
    })
}

pub fn has_opcode_note(channel: Channel, dir: Dir, opcode: u16) -> bool {
    !opcode_note(channel, dir, opcode).is_empty()
}

pub fn has_message_note(m: &Msg) -> bool {
    !message_note(m).is_empty()
}

/// Sets (or, with empty text, removes) the note on an opcode.
pub fn set_opcode_note(channel: Channel, dir: Dir, opcode: u16, text: &str) {
    with(|notes| {
        notes.retain(|n| n.message || (n.channel, n.dir, n.opcode) != (channel, dir, opcode));
        if !text.trim().is_empty() {
            notes.push(Note {
                message: false,
                channel,
                dir,
                opcode,
                session: String::new(),
                seq: 0,
                unix_ms: poe_hook_shared::unix_ms(),
                text: text.trim().to_owned(),
                hex: String::new(),
            });
        }
        save(notes);
    });
    log::info!(
        "packet note on {} {} {opcode:#06x}: {}",
        channel.name(),
        dir.name(),
        escape(text.trim())
    );
}

/// Sets (or removes) the note on one captured message.
pub fn set_message_note(m: &Msg, text: &str) {
    let session = session_name();
    with(|notes| {
        notes.retain(|n| !(n.message && n.seq == m.seq && n.session == session));
        if !text.trim().is_empty() {
            notes.push(Note {
                message: true,
                channel: m.channel,
                dir: m.dir,
                opcode: m.opcode,
                session: session.clone(),
                seq: m.seq,
                unix_ms: m.unix_ms,
                text: text.trim().to_owned(),
                hex: if m.redacted {
                    String::new()
                } else {
                    m.data.iter().map(|b| format!("{b:02x}")).collect()
                },
            });
        }
        save(notes);
    });
    log::info!(
        "packet note on message #{} ({} {} {:#06x}): {}",
        m.seq,
        m.channel.name(),
        m.dir.name(),
        m.opcode,
        escape(text.trim())
    );
}

/// All notes, opcode notes first.
pub fn all() -> Vec<Note> {
    let mut v = with(|notes| notes.clone());
    v.sort_by_key(|n| (n.message, n.channel, n.dir, n.opcode, n.seq));
    v
}
