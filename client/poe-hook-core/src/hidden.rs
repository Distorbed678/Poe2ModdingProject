//! Opcodes hidden from the Packets tab (identified ones, so new opcodes stand out). Kept across
//! sessions in `logs/poe-hook/hidden_opcodes.tsv` next to the exe, one `channel  dir  opcode`
//! per line. Hiding only filters the view: every message is still captured and written to the
//! session's packet files.

use std::{
    collections::BTreeSet,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::packets::{Channel, Dir};

pub type Key = (u8, u8, u16);

static HIDDEN: Mutex<Option<BTreeSet<Key>>> = Mutex::new(None);
/// Bumped on every change, so views know to filter again.
static VERSION: AtomicU64 = AtomicU64::new(1);

pub fn key(channel: Channel, dir: Dir, opcode: u16) -> Key {
    let c = match channel {
        Channel::Login => 0,
        Channel::Instance => 1,
        Channel::Other => 2,
    };
    let d = match dir {
        Dir::In => 0,
        Dir::Out => 1,
    };
    (c, d, opcode)
}

fn path() -> std::path::PathBuf {
    poe_hook_shared::exe_dir()
        .join("logs")
        .join("poe-hook")
        .join("hidden_opcodes.tsv")
}

fn load() -> BTreeSet<Key> {
    std::fs::read_to_string(path())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let c = match *f.first()? {
                "login" => 0,
                "instance" => 1,
                _ => 2,
            };
            let d = if *f.get(1)? == "in" { 0 } else { 1 };
            let op = u16::from_str_radix(f.get(2)?.trim_start_matches("0x"), 16).ok()?;
            Some((c, d, op))
        })
        .collect()
}

fn save(set: &BTreeSet<Key>) {
    let mut out = String::from("# channel\tdir\topcode (hidden in the overlay's Packets tab)\n");
    for &(c, d, op) in set {
        let c = ["login", "instance", "other"][c as usize];
        let d = ["in", "out"][d as usize];
        out.push_str(&format!("{c}\t{d}\t{op:#06x}\n"));
    }
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    if let Err(e) = std::fs::write(&p, out) {
        log::warn!("hidden opcodes: writing {}: {e}", p.display());
    }
}

fn with<T>(f: impl FnOnce(&mut BTreeSet<Key>) -> T) -> T {
    let mut guard = HIDDEN.lock().unwrap();
    f(guard.get_or_insert_with(load))
}

pub fn is_hidden(k: Key) -> bool {
    with(|s| s.contains(&k))
}

pub fn set_hidden(k: Key, hidden: bool) {
    with(|s| {
        let changed = if hidden { s.insert(k) } else { s.remove(&k) };
        if changed {
            save(s);
            VERSION.fetch_add(1, Ordering::Relaxed);
        }
    });
}

pub fn clear() {
    with(|s| {
        s.clear();
        save(s);
    });
    VERSION.fetch_add(1, Ordering::Relaxed);
}

pub fn count() -> usize {
    with(|s| s.len())
}

pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}
