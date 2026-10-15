//! The loader's services ([`HostApi`]) and the `log` facade that forwards to its log sink.

use std::{ffi::c_void, path::PathBuf, sync::OnceLock};

use poe_hook_shared::abi::{Connection, HostApi, LogRecord, Str, VulkanState};

static HOST: OnceLock<HostApi> = OnceLock::new();

pub fn set(api: HostApi) {
    HOST.set(api).ok();
    static FACADE: Facade = Facade;
    let _ = log::set_logger(&FACADE);
}

pub fn api() -> &'static HostApi {
    HOST.get().expect("host api set in poe_core_start")
}

pub fn read_only() -> bool {
    (api().read_only)()
}

/// Recorded connections, oldest first.
pub fn connections() -> Vec<Connection> {
    let mut out = vec![Connection::default(); 256];
    let n = (api().connections)(out.as_mut_ptr(), out.len());
    out.truncate(n);
    out
}

pub fn vulkan() -> Option<VulkanState> {
    let mut state = VulkanState::default();
    (api().vulkan)(&mut state).then_some(state)
}

pub fn queue_family(queue: u64) -> i32 {
    (api().queue_family)(queue)
}

pub fn session_dir() -> PathBuf {
    PathBuf::from(unsafe { api().session_dir.as_str() })
}

pub fn reload_count() -> u32 {
    api().reload_count
}

pub fn request_reload() {
    (api().request_reload)()
}

/// Visits the kept log records from `since` on; returns the next sequence number.
pub fn logs_since<F: FnMut(&LogRecord)>(since: u64, mut visit: F) -> u64 {
    extern "C" fn trampoline<F: FnMut(&LogRecord)>(ctx: *mut c_void, record: &LogRecord) {
        unsafe { (*(ctx as *mut F))(record) }
    }
    (api().logs_since)(
        since,
        &mut visit as *mut _ as *mut c_void,
        trampoline::<F>,
    )
}

struct Facade;

impl log::Log for Facade {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if let Some(api) = HOST.get()
            && self.enabled(record.metadata())
        {
            let message = record.args().to_string();
            (api.log)(
                record.level() as u32,
                Str::new(record.target()),
                Str::new(&message),
            );
        }
    }

    fn flush(&self) {}
}
