//! `poe-hook.toml` next to the game executable. Missing or invalid files fall back to
//! defaults (the error is logged once the logger is up). Both DLLs read it: the loader at
//! process start, the core on every (re)load.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `private` (our server) or `official` (read-only: see [`Mode`]).
    pub mode: Mode,
    pub log: LogConfig,
    pub features: FeatureConfig,
    pub overlay: OverlayConfig,
    pub packets: PacketConfig,
    pub reload: ReloadConfig,
}

/// How the client is used. The read-only latch also switches on by itself the first time the
/// game connects to a non-loopback address, whatever the mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Our server: crypto bypass on, external connections refused (`block_external`).
    #[default]
    Private,
    /// Official servers, read-only: nothing is sent, altered or injected; the DLL only
    /// observes and logs. External connections are allowed.
    Official,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FeatureConfig {
    pub crash_log: bool,
    pub crypto_bypass: bool,
    pub net_trace: bool,
    pub net_trace_dump_bytes: usize,
    pub block_external: bool,
    pub trace_addresses: Vec<String>,
    pub trace_limit: u32,
    /// Research hooks on entities and frames (object-type dump, entity components, terrain
    /// layout export, player frame tick). Position sync and the player inspector need them.
    pub dump_object_types: bool,
    /// With `dump_object_types`: per-entity tracing (each entity's components, the blob span
    /// every component reader consumes, terrain dumps on the Positioned reader). Slow: it
    /// froze the game for ~2 s per spawned entity on official servers.
    pub entity_trace: bool,
    /// Log the player's position every simulation frame (movement research; also enabled by
    /// the `POE_HOOK_MOVEMENT_LOG` environment variable).
    pub movement_log: bool,
    /// Report the player's position and action to the local instance server every frame
    /// (movement dev bridge until M5, see position_sync.rs). Never in read-only mode.
    pub position_sync: bool,
    /// The instance server's port: only connections to it become the report target (0: the
    /// last loopback connection). Set by `cargo xtask deploy-client` from `server.toml`.
    pub position_sync_port: u16,
    /// Keys posted to the game window for unattended tests: `[[ms_after_load, vk], ...]`
    /// (e.g. 0x0D Enter to log in and play). Also `POE_HOOK_AUTO_KEYS="20000:13,..."`.
    /// Never in read-only mode.
    pub auto_keys: Vec<(u64, u16)>,
    /// Terrain research (M5): export each generated area (layout, grids, generator objects,
    /// walkability probe) to `<session>/areas/`. Read-only, so also on official servers.
    pub area_export: bool,
    /// Log every terrain RNG draw and generator phase (`generator_trace.tsv` in the area
    /// export). Large; also enabled by the `POE_HOOK_GEN_TRACE` environment variable.
    pub generator_trace: bool,
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
            dump_object_types: true,
            entity_trace: false,
            movement_log: false,
            position_sync: true,
            position_sync_port: 0,
            auto_keys: Vec::new(),
            area_export: true,
            generator_trace: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    pub enabled: bool,
    /// Virtual key that shows/hides the overlay (default Insert).
    pub toggle_key: u16,
    /// Virtual key that saves a bug-report snapshot (default Home).
    pub snapshot_key: u16,
    /// How much history a snapshot keeps.
    pub snapshot_seconds: u32,
    pub visible_at_start: bool,
    /// Global UI scale (font size multiplier).
    pub scale: f32,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            toggle_key: 0x2d,
            snapshot_key: 0x24,
            snapshot_seconds: 30,
            visible_at_start: false,
            scale: 1.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PacketConfig {
    pub capture: bool,
    /// Replace secrets (Steam ticket, session tokens, key material) in exported files.
    pub redact: bool,
    /// Messages kept in memory for the Packets tab (older ones stay in the files).
    pub keep: usize,
}

impl Default for PacketConfig {
    fn default() -> Self {
        Self {
            capture: true,
            redact: true,
            keep: 50_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ReloadConfig {
    /// Reload the core when `poe_hook_core.dll` next to the exe changes.
    pub auto: bool,
}

impl Default for ReloadConfig {
    fn default() -> Self {
        Self { auto: true }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        let mut config: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default();
        if let Ok(keys) = std::env::var("POE_HOOK_AUTO_KEYS") {
            config
                .features
                .auto_keys
                .extend(keys.split(',').filter_map(|entry| {
                    let (at, vk) = entry.trim().split_once(':')?;
                    let num = |v: &str| match v.strip_prefix("0x") {
                        Some(hex) => u64::from_str_radix(hex, 16).ok(),
                        None => v.parse().ok(),
                    };
                    Some((num(at)?, num(vk)? as u16))
                }));
        }
        config
    }

    /// Official mode: everything that could change the game is off from the start.
    pub fn official(&self) -> bool {
        self.mode == Mode::Official
    }
}
