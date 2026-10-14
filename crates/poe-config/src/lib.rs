//! Typed view of `config/server.toml`.
//!
//! Every tunable the servers and tooling need lives here so nothing deployment-specific is
//! hardcoded. The file location defaults to [`DEFAULT_PATH`] and can be overridden with the
//! `POE_CONFIG` environment variable.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::Deserialize;

pub const DEFAULT_PATH: &str = "config/server.toml";
pub const ENV_PATH: &str = "POE_CONFIG";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub game: GameConfig,
    pub login: LoginConfig,
    pub instance: InstanceConfig,
    pub database: DatabaseConfig,
    pub accounts: AccountsConfig,
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub leagues: Vec<LeagueConfig>,
    #[serde(default)]
    pub inventories: InventoriesConfig,
    #[serde(default)]
    pub spawn: SpawnConfig,
    #[serde(default)]
    pub chat: ChatConfig,
    #[serde(default)]
    pub player: PlayerConfig,
}

/// Player characters.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlayerConfig {
    /// Granted effects (`GrantedEffects.Id`) every character has, e.g. `DodgeRollPlayer`.
    #[serde(default)]
    pub innate_skills: Vec<String>,
    /// Area (`WorldAreas.Id`) for new characters whose start state names no hub area.
    #[serde(default)]
    pub start_area: Option<String>,
}

/// Chat channels. There is no chat server yet: join requests are answered so the client does
/// not report a timeout.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatConfig {
    /// Channel number given when the client asks for any channel.
    #[serde(default = "default_chat_channel")]
    pub default_channel: u16,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            default_channel: default_chat_channel(),
        }
    }
}

fn default_chat_channel() -> u16 {
    1
}

/// Player start position (see `instance-server::spawn`). Until the server generates terrain
/// (M4) it relies on the layout exported by the client DLL.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SpawnConfig {
    /// Directory where the DLL writes `terrain_layout_<tile hash>.tsv`; empty disables the
    /// bridge.
    #[serde(default)]
    pub layout_dir: String,
    /// Substrings of tile paths that are never playable (map filler).
    #[serde(default)]
    pub filler: Vec<String>,
    /// Per `WorldAreas.Id`: the tile the player starts on.
    #[serde(default)]
    pub start_tiles: std::collections::BTreeMap<String, StartTile>,
    /// Areas whose first visit plays the "washed up" opening (Player flag); without it the
    /// intro area stays covered in grey fog.
    #[serde(default)]
    pub intro_areas: Vec<String>,
}

/// Start tile of an area: a substring of the tile's path, and optionally the start point in the
/// tile's own (unrotated) orientation, in grid cells from its corner (23 cells per sub-tile).
/// The server finds how the tile was rotated/mirrored in the generated layout and maps the
/// point; without it the player starts at the tile's centre.
#[derive(Debug, Clone, Deserialize)]
pub struct StartTile {
    pub tile: String,
    #[serde(default)]
    pub point: Option<[i32; 2]>,
}

/// Player inventories created on instance entry. Slots come from `Inventories.dat`; grid
/// sizes are not in the game tables, so they are configured here by `Inventories.Id`.
#[derive(Debug, Clone, Deserialize)]
pub struct InventoriesConfig {
    /// Grid size for slots not listed in `sizes`.
    #[serde(default = "default_inventory_size")]
    pub default_size: [u8; 2],
    #[serde(default)]
    pub sizes: std::collections::BTreeMap<String, [u8; 2]>,
    /// `Inventories.Id` values never sent (account-wide or not per-character).
    #[serde(default)]
    pub skip: Vec<String>,
}

fn default_inventory_size() -> [u8; 2] {
    [1, 1]
}

impl Default for InventoriesConfig {
    fn default() -> Self {
        Self {
            default_size: default_inventory_size(),
            sizes: Default::default(),
            skip: Vec::new(),
        }
    }
}

impl InventoriesConfig {
    /// (width, height) of an inventory slot.
    pub fn size(&self, id: &str) -> [u8; 2] {
        self.sizes.get(id).copied().unwrap_or(self.default_size)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameConfig {
    pub version: String,
    pub game_dir: PathBuf,
    pub schema: PathBuf,
    pub protocol_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginConfig {
    pub bind: SocketAddr,
    pub public_host: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    pub bind: SocketAddr,
    pub public_host: String,
    pub public_port: u16,
    /// Simulation frame interval: a tick marker (0x2f4) is sent this often.
    #[serde(default = "default_tick_ms")]
    pub tick_ms: u64,
    /// Optional TSV of terrain seeds known to generate (`area, level, seed, ...`); the server
    /// picks one for areas listed there, since some random seeds fail to generate.
    #[serde(default)]
    pub known_seeds: PathBuf,
    /// Accept the client DLL's per-frame position reports (UDP on the instance port) and
    /// follow them (movement dev bridge until the server simulates movement exactly, M4).
    #[serde(default = "default_true")]
    pub position_sync: bool,
    /// Movement trace (dev): every client position report, movement input and movement reply
    /// is appended to this TSV file, for analysing a play session. Empty: off.
    #[serde(default)]
    pub movement_trace: PathBuf,
    /// Which cell a turn (`0x64`/`0x1c8`) is sent with: "own" (the client's cell when the
    /// turn applies: the client only re-aims its walk, as with a server that simulates it
    /// exactly), "heading" (a cell one step along the new heading: the client is put on its
    /// centre) or "natural" (the cell the client would reach in that frame).
    #[serde(default = "default_turn_cell")]
    pub turn_cell: String,
}

fn default_turn_cell() -> String {
    "natural".into()
}

fn default_true() -> bool {
    true
}

fn default_tick_ms() -> u64 {
    33
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountsConfig {
    pub name_prefix: String,
    pub max_characters: u8,
    #[serde(default)]
    pub flags: [u8; 2],
    /// Full game access (InitialState CharacterState flags byte 1 bit 6); without it the
    /// in-game escape menu shows a "Get Full Access" button.
    #[serde(default = "yes")]
    pub full_access: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub region: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub filter: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            filter: "info".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LeagueConfig {
    pub id: String,
    #[serde(default)]
    pub hardcore: bool,
    /// Permanent league (Standard): has a banner at character creation and is not offered in
    /// the character select "Join" list.
    #[serde(default)]
    pub permanent: bool,
    /// Makes this league a variant (e.g. Hardcore) of another league: at character creation
    /// it is an option on that league's banner instead of a banner of its own.
    #[serde(default)]
    pub variant_of: Option<String>,
    /// `LeagueFlag.Id` for the league's banner and icon (e.g. "StandardFlag").
    #[serde(default)]
    pub flag: Option<String>,
}

impl Config {
    /// Loads the config from `POE_CONFIG` or [`DEFAULT_PATH`].
    pub fn load_default() -> Result<Self, ConfigError> {
        let path = std::env::var_os(ENV_PATH)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PATH));
        Self::load(&path)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })
    }

    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_config_parses() {
        let text = include_str!("../../../config/server.toml");
        let cfg = Config::parse(text).expect("config/server.toml must parse");
        assert!(!cfg.leagues.is_empty());
    }
}
