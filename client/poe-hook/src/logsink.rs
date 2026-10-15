//! The log sink shared by both DLLs: every record goes to `poe-hook.log` next to the exe (as
//! before), to the session folder's `poe-hook.log`, and into a ring buffer that the core's
//! Log tab reads (it survives core reloads).

use std::{
    collections::VecDeque,
    ffi::c_void,
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::Mutex,
};

use poe_hook_shared::abi::{LogRecord, LogVisit, Str};

/// Records kept in memory.
const KEEP: usize = 50_000;

struct Owned {
    seq: u64,
    unix_ms: u64,
    level: u32,
    target: String,
    message: String,
}

struct Sink {
    files: Vec<File>,
    next: u64,
    records: VecDeque<Owned>,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);

struct Facade;

impl log::Log for Facade {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            write(
                record.level() as u32,
                record.target(),
                &record.args().to_string(),
            );
        }
    }

    fn flush(&self) {}
}

/// Opens the log files (appending: the client may restart itself, and each process loads the
/// DLL again; `run-client` clears the exe-side file per launch) and installs the `log` facade.
pub fn init(paths: &[&Path], level: log::LevelFilter) {
    let files = paths
        .iter()
        .filter_map(|p| OpenOptions::new().create(true).append(true).open(p).ok())
        .map(|mut f| {
            let _ = writeln!(f, "---- process {} ----", std::process::id());
            f
        })
        .collect();
    *SINK.lock().unwrap() = Some(Sink {
        files,
        next: 1,
        records: VecDeque::new(),
    });
    static FACADE: Facade = Facade;
    if log::set_logger(&FACADE).is_ok() {
        log::set_max_level(level);
    }
}

/// Writes one record (from the loader's facade or from the core through the host API).
pub fn write(level: u32, target: &str, message: &str) {
    if level > log::max_level() as u32 {
        return;
    }
    let unix_ms = poe_hook_shared::unix_ms();
    let name = match level {
        1 => "ERROR",
        2 => "WARN",
        3 => "INFO",
        4 => "DEBUG",
        _ => "TRACE",
    };
    let line = format!(
        "{}.{:03} {name:5} [{target}] {message}\n",
        unix_ms / 1000,
        unix_ms % 1000
    );
    let Ok(mut guard) = SINK.lock() else { return };
    let Some(sink) = guard.as_mut() else { return };
    for f in &mut sink.files {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
    let seq = sink.next;
    sink.next += 1;
    if sink.records.len() >= KEEP {
        sink.records.pop_front();
    }
    sink.records.push_back(Owned {
        seq,
        unix_ms,
        level,
        target: target.to_owned(),
        message: message.to_owned(),
    });
}

pub extern "C" fn host_log(level: u32, target: Str, message: Str) {
    write(level, unsafe { target.as_str() }, unsafe {
        message.as_str()
    });
}

pub extern "C" fn host_logs_since(since: u64, ctx: *mut c_void, visit: LogVisit) -> u64 {
    let Ok(guard) = SINK.lock() else { return since };
    let Some(sink) = guard.as_ref() else {
        return since;
    };
    let first = sink.records.front().map_or(sink.next, |r| r.seq);
    let skip = since.saturating_sub(first) as usize;
    for r in sink.records.iter().skip(skip) {
        visit(
            ctx,
            &LogRecord {
                seq: r.seq,
                unix_ms: r.unix_ms,
                level: r.level,
                target: Str::new(&r.target),
                message: Str::new(&r.message),
            },
        );
    }
    sink.next
}
