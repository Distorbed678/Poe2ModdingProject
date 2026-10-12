//! Optional client features, each toggled in `poe-hook.toml`.
//!
//! New behaviour (encryption bypass, packet tracing, asset overrides for client mods...) is
//! added as another module here and registered in [`init_all`].

mod crash;
mod crypto_bypass;
mod net_trace;

use crate::{config::Config, module::Module};

pub fn init_all(config: &Config, main: &Module) {
    let f = &config.features;
    let net = || net_trace::Options {
        trace: f.net_trace,
        dump_bytes: f.net_trace_dump_bytes,
        block_external: f.block_external,
    };
    let features: [(&str, bool, Box<dyn Fn(&Module) -> Result<(), String>>); 3] = [
        ("crash_log", f.crash_log, Box::new(crash::init)),
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
