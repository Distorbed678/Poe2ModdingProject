//! The game's own log (`logs/Client.txt` next to the exe), this launch's lines only: the file
//! is shared by every launch and Wine reuses process ids, so lines are kept from this
//! process's start time on (the session folder name, `YYYY-MM-DD_HH-MM-SS_<pid>`).

use std::{
    io::{Read, Seek, SeekFrom},
    sync::Mutex,
};

/// How far back from the end to start on the first read (the file grows across launches).
const START_WINDOW: u64 = 4 << 20;
const KEEP: usize = 20_000;

struct Tail {
    offset: u64,
    partial: String,
    lines: Vec<String>,
}

static TAIL: Mutex<Option<Tail>> = Mutex::new(None);

/// Reads what was appended since the last call; returns the kept lines.
pub fn lines() -> Vec<String> {
    let path = poe_hook_shared::exe_dir().join("logs").join("Client.txt");
    let mut guard = TAIL.lock().unwrap();
    let Ok(mut f) = std::fs::File::open(&path) else {
        return guard.as_ref().map(|t| t.lines.clone()).unwrap_or_default();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let tail = guard.get_or_insert_with(|| Tail {
        offset: len.saturating_sub(START_WINDOW),
        partial: String::new(),
        lines: Vec::new(),
    });
    if len < tail.offset {
        tail.offset = 0;
    }
    if len > tail.offset && f.seek(SeekFrom::Start(tail.offset)).is_ok() {
        let mut buf = Vec::new();
        let _ = (&mut f).take(len - tail.offset).read_to_end(&mut buf);
        tail.offset += buf.len() as u64;
        tail.partial.push_str(&String::from_utf8_lossy(&buf));
        let start = start_stamp();
        while let Some(nl) = tail.partial.find('\n') {
            let line: String = tail.partial.drain(..=nl).collect();
            let line = line.trim_end();
            // Lines start with `YYYY/MM/DD HH:MM:SS`, which sorts as text.
            if line.get(..19).is_some_and(|t| t >= start.as_str()) {
                tail.lines.push(line.to_owned());
            }
        }
        if tail.lines.len() > KEEP {
            let drop = tail.lines.len() - KEEP;
            tail.lines.drain(..drop);
        }
    }
    tail.lines.clone()
}

/// This process's start as `YYYY/MM/DD HH:MM:SS`, from the session folder name.
fn start_stamp() -> String {
    let dir = crate::host::session_dir();
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // 2026-10-14_21-56-48_336
    match (name.get(..10), name.get(11..19)) {
        (Some(date), Some(time)) => {
            format!("{} {}", date.replace('-', "/"), time.replace('-', ":"))
        }
        _ => String::new(),
    }
}
