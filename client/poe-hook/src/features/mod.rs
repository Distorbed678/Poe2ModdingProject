//! Optional client features, each toggled in `poe-hook.toml`.
//!
//! New behaviour (encryption bypass, packet tracing, asset overrides for client mods...) is
//! added as another module here and registered in [`init_all`].

mod crash;
mod crypto_bypass;
mod dump_types;
mod exit_trace;
mod net_trace;
mod trace_calls;

use crate::{config::Config, module::Module};

pub fn init_all(config: &Config, main: &Module) {
    let f = &config.features;
    let net = || net_trace::Options {
        trace: f.net_trace,
        dump_bytes: f.net_trace_dump_bytes,
        block_external: f.block_external,
    };
    let trace = (f.trace_addresses.clone(), f.trace_limit);
    let dump_path = crate::exe_dir().join("object_types.txt");
    let features: [(&str, bool, Box<dyn Fn(&Module) -> Result<(), String>>); 6] = [
        (
            "dump_object_types",
            f.dump_object_types,
            Box::new(move |m| dump_types::init(m, dump_path.clone())),
        ),
        (
            "trace_calls",
            !f.trace_addresses.is_empty(),
            Box::new(move |m| trace_calls::init(m, &trace.0, trace.1)),
        ),
        ("crash_log", f.crash_log, Box::new(crash::init)),
        ("exit_trace", f.crash_log, Box::new(exit_trace::init)),
        (
            "crypto_bypass",
            f.crypto_bypass,
            Box::new(crypto_bypass::init),
        ),
        (
            "net_hooks",
            f.net_trace || f.block_external,
            Box::new(move |m| net_trace::init(m, net())),
        ),
    ];
    for (name, enabled, init) in features {
        if !enabled {
            log::info!("feature {name}: disabled");
            continue;
        }
        match init(main) {
            Ok(()) => log::info!("feature {name}: enabled"),
            Err(e) => log::error!("feature {name}: failed: {e}"),
        }
    }
}
