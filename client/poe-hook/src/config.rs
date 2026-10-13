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
