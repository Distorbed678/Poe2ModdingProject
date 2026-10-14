//! `poe-hook.toml` next to the game executable. Missing or invalid files fall back to
//! defaults (the error is logged once the logger is up).

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub log: LogConfig,
    pub features: FeatureConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: log::LevelFilter,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: log::LevelFilter::Info,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct FeatureConfig {
    pub crash_log: bool,
    pub crypto_bypass: bool,
    pub net_trace: bool,
    pub net_trace_dump_bytes: usize,
    pub block_external: bool,
    pub trace_addresses: Vec<String>,
    pub trace_limit: u32,
    pub dump_object_types: bool,
    /// With `dump_object_types`: log the player's position every simulation frame (movement
    /// research; also enabled by the `POE_HOOK_MOVEMENT_LOG` environment variable).
    pub movement_log: bool,
    /// With `dump_object_types`: report the player's position and action to the local
    /// instance server every frame (movement dev bridge until M4, see position_sync.rs).
    pub position_sync: bool,
    /// The instance server's port: only connections to it become the report target (0: the
    /// last loopback connection). Set by `cargo xtask deploy-client` from `server.toml`.
    pub position_sync_port: u16,
    /// Keys posted to the game window for unattended tests: `[[ms_after_load, vk], ...]`
    /// (e.g. 0x0D Enter to log in and play, 0x2C PrintScreen for an in-game screenshot).
    pub auto_keys: Vec<(u64, u16)>,
}

impl Default for FeatureConfig {
    fn default() -> Self {
        Self {
            crash_log: true,
            crypto_bypass: true,
            net_trace: false,
            net_trace_dump_bytes: 256,
            block_external: true,
            trace_addresses: Vec::new(),
            trace_limit: 20,
            dump_object_types: false,
            movement_log: false,
            position_sync: true,
            position_sync_port: 0,
            auto_keys: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }
}
