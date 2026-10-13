//! Minimal file logger behind the `log` facade.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

struct FileLogger {
    file: Mutex<File>,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let line = format!(
            "{}.{:03} {:5} [{}] {}\n",
            t.as_secs(),
            t.subsec_millis(),
            record.level(),
            record.target(),
            record.args()
        );
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }

    fn flush(&self) {}
}

/// Appends to `path`: the client may restart itself (each process loads the DLL again), and
/// truncating would lose the previous process's log. `run-client` clears the file per launch.
pub fn init(path: &Path, level: log::LevelFilter) {
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "---- process {} ----", std::process::id());
    let logger = Box::leak(Box::new(FileLogger {
        file: Mutex::new(file),
    }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}
