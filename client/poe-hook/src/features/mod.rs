//! Optional client features, each toggled in `poe-hook.toml`.
//!
//! New behaviour (encryption bypass, packet tracing, asset overrides for client mods...) is
//! added as another module here and registered in [`init_all`].

mod crash;

use crate::{config::Config, module::Module};

pub fn init_all(config: &Config, main: &Module) {
    let features: [(&str, bool, fn(&Module) -> Result<(), String>); 1] =
        [("crash_log", config.features.crash_log, crash::init)];
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
