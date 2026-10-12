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
}

impl Default for FeatureConfig {
    fn default() -> Self {
        Self { crash_log: true }
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
