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
    /// Listen addresses and advertised host/port of the login and instance servers.
    pub network: NetworkConfig,
    /// Instances of areas and the simulation frame clock.
    #[serde(default)]
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
    #[serde(default)]
    pub terrain: TerrainConfig,
    /// The cluster these servers belong to (tokens are not valid across clusters).
    #[serde(default)]
    pub cluster: ClusterConfig,
    #[serde(default)]
    pub commands: CommandsConfig,
    /// Movement simulation tuning.
    #[serde(default)]
    pub movement: MovementConfig,
    /// Development and research tools (debug link, position sync, traces, seed sweep).
    #[serde(default)]
    pub dev: DevConfig,
    /// HTTP API and web admin (`crates/poe-web-api`).
    #[serde(default)]
    pub web: WebConfig,
}

/// The web API (REST and WebSocket) and the optional static web admin.
#[derive(Debug, Clone, Deserialize)]
pub struct WebConfig {
    /// Start the web API with poe-server.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Address to listen on. Loopback by default: put a TLS reverse proxy in front before
    /// exposing it (passwords travel in the request body).
    #[serde(default = "default_web_bind")]
    pub bind: std::net::SocketAddr,
    /// Directory with the built web admin (SPA), served at `/` with an `index.html` fallback
    /// for client-side routes. Empty = no static files.
    #[serde(default)]
    pub static_dir: PathBuf,
    /// Origins allowed to call the API from a browser (CORS), e.g. a Vite dev server.
    #[serde(default)]
    pub cors_origins: Vec<String>,
    /// How many recent log events the live log viewer keeps.
    #[serde(default = "default_log_buffer")]
    pub log_buffer: usize,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: default_web_bind(),
            static_dir: PathBuf::new(),
            cors_origins: Vec::new(),
            log_buffer: default_log_buffer(),
        }
    }
}

fn default_web_bind() -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], 8080))
}

fn default_log_buffer() -> usize {
    2000
}

/// Server-side terrain generation (`poe-world`). The client always builds the terrain itself
/// from the area and seed; the server's copy is for collision, pathing and spawns later.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TerrainConfig {
    /// Generate each new instance's terrain on the server and compare its tile hash with the
    /// client's. Off by default until the generator covers every area.
    #[serde(default)]
    pub generate: bool,
}

/// Cluster identity. Login tokens carry the id of the cluster that issued them, so a token
/// (or a database copied from another cluster) does not log in here.
#[derive(Debug, Clone, Deserialize)]
pub struct ClusterConfig {
    #[serde(default = "default_cluster_id")]
    pub id: String,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            id: default_cluster_id(),
        }
    }
}

fn default_cluster_id() -> String {
    "local".into()
}

/// Chat commands (`!help` lists them).
#[derive(Debug, Clone, Deserialize)]
pub struct CommandsConfig {
    /// Prefix of server commands typed in chat. Kept apart from `/`, which the client uses
    /// for its own commands.
    #[serde(default = "default_command_prefix")]
    pub prefix: String,
    /// Append every admin command (account, command, result) to this file; empty = off.
    #[serde(default)]
    pub audit_log: PathBuf,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            prefix: default_command_prefix(),
            audit_log: PathBuf::new(),
        }
    }
}

fn default_command_prefix() -> String {
    "!".into()
}

/// Player characters.
#[derive(Debug, Clone, Deserialize)]
pub struct PlayerConfig {
    /// Granted effects (`GrantedEffects.Id`) every character has, e.g. `DodgeRollPlayer`.
    #[serde(default)]
    pub innate_skills: Vec<String>,
    /// Area (`WorldAreas.Id`) for new characters whose start state names no hub area.
    #[serde(default)]
    pub start_area: Option<String>,
    /// The three animation set names (the Actor component's weapon-pose strings) other players'
    /// clients get for a character, in the official order: main hand set, off hand set,
    /// default. The official values depend on the equipped weapons (`BowTown`,
    /// `WandSpiritShieldTown`, ...), `UnarmedTown`, `default`; characters have no equipment
    /// yet, so everyone is unarmed.
    #[serde(default = "default_animation_sets")]
    pub animation_sets: [String; 3],
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            innate_skills: Vec::new(),
            start_area: None,
            animation_sets: default_animation_sets(),
        }
    }
}

fn default_animation_sets() -> [String; 3] {
    ["UnarmedTown".into(), "UnarmedTown".into(), "default".into()]
}

/// Chat channels: join requests get the requested channel number (this one for "any"); the
/// instance server routes chat lines by channel (`instance-server::chat`).
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

/// Player start position (see `instance-server::spawn`). The layout comes from the client DLL
/// (exported or uploaded); the server's own generator (`poe-world`) does not feed spawns yet.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SpawnConfig {
    /// Terrain dump directory (`dumps/<version>/terrain`): the client DLL writes
    /// `layouts/terrain_layout_<tile hash>.tsv` and `grids/terrain_grid_<tile hash>.bin`
    /// there (poe-hook.toml `dump_dir`); empty disables layout-based spawns.
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
    /// Terrain uploads from client DLLs (`layout_dir` is their cache).
    #[serde(default)]
    pub upload: UploadConfig,
}

/// Terrain upload (WP3.4): a client DLL on a trusted server sends the layout and cell grid it
/// generated, authenticated with the session's side-channel keys (docs/protocol/side-channel.md),
/// over the instance TCP port. The server validates them and caches them per tile hash in
/// `[spawn] layout_dir`, which is also where local clients write them.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UploadConfig {
    /// Accept uploads. Needs `[spawn] layout_dir`.
    pub enabled: bool,
    /// Largest upload as sent (zlib-compressed layout and grid), in bytes.
    pub max_bytes: usize,
    /// Largest layout after decompression.
    pub max_layout_bytes: usize,
    /// Largest grid after decompression.
    pub max_grid_bytes: usize,
    /// How long a spawn waits for the layout when no upload is announced (a local DLL writes
    /// the file within a moment).
    pub wait_ms: u64,
    /// How long a spawn waits for the layout once its client announced an upload.
    pub wait_upload_secs: u64,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_bytes: 32 << 20,
            max_layout_bytes: 16 << 20,
            max_grid_bytes: 128 << 20,
            wait_ms: 3000,
            wait_upload_secs: 60,
        }
    }
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

/// `[network]`: where the servers listen and what the client is told to connect to.
#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    /// Host written into `gateway_list.txt` (login server) and sent to the client in
    /// InstanceDetails (instance server). One value for both.
    pub public_host: String,
    pub login: NetworkLoginConfig,
    pub instance: NetworkInstanceConfig,
}

impl NetworkConfig {
    /// Problems with the advertised address that would send remote clients to the wrong place
    /// (logged at startup; none of them stops the server).
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let host = self.public_host.trim();
        let loopback = host == "localhost"
            || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
        let listens_beyond_loopback = !self.login.bind.ip().is_loopback()
            || !self.instance.bind.ip().is_loopback();
        if host.is_empty() {
            out.push("[network] public_host is empty: clients get no address".into());
        } else if loopback && listens_beyond_loopback {
            out.push(format!(
                "[network] public_host = {host:?} is a loopback address: only clients on this \
                 machine can connect (set the LAN or public address for other players, \
                 docs/guides/hosting.md)"
            ));
        }
        if host.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_unspecified()) {
            out.push("[network] public_host = \"0.0.0.0\" is not an address clients can reach".into());
        }
        out
    }
}

/// `[network.login]`.
#[derive(Debug, Clone, Deserialize)]
pub struct NetworkLoginConfig {
    pub bind: SocketAddr,
    /// Send `0x0017 0x0014` after InstanceDetails as official servers do: the client then closes
    /// its login connection, so in-game transfers (which go through it) stop working.
    #[serde(default)]
    pub close_after_handoff: bool,
    /// Port written into the client's gateway entry and trusted list (`cargo xtask
    /// deploy-client`); differs from `bind` behind port forwarding. Default: the bind port.
    #[serde(default)]
    pub public_port: Option<u16>,
}

impl NetworkLoginConfig {
    /// The login port clients connect to.
    pub fn public_port(&self) -> u16 {
        self.public_port.unwrap_or(self.bind.port())
    }
}

/// `[network.instance]`.
#[derive(Debug, Clone, Deserialize)]
pub struct NetworkInstanceConfig {
    pub bind: SocketAddr,
    /// Port the login server tells the client to connect to (and where the DLL sends position
    /// reports); differs from `bind` behind port forwarding.
    pub public_port: u16,
}

/// `[instance]`: instances of areas and their simulation clock.
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    /// Close instances that have been empty this long (towns and hideouts stay); 0 = never.
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u64,
    /// Simulation frame interval: a tick marker (0x2f4) is sent this often.
    #[serde(default = "default_tick_ms")]
    pub tick_ms: u64,
    /// First entity id an instance hands out (one counter per instance, shared by all
    /// players): official 0.5.5e instances give players 12000 and up (areas' own objects have
    /// lower ids).
    #[serde(default = "default_first_entity_id")]
    pub first_entity_id: u32,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            idle_minutes: default_idle_minutes(),
            tick_ms: default_tick_ms(),
            first_entity_id: default_first_entity_id(),
        }
    }
}

fn default_first_entity_id() -> u32 {
    12000
}

fn default_idle_minutes() -> u64 {
    15
}

fn default_tick_ms() -> u64 {
    33
}

/// `[movement]`: tuning of the server-side movement simulation (`poe-game`'s `Locomotion`).
/// Everything else (skill ids, walking speed, roll distance) comes from game data; the defaults
/// here are the values of the official 0.5.5e captures.
#[derive(Debug, Clone, Deserialize)]
pub struct MovementConfig {
    /// `BuffDefinitions` rows of the three buffs a sprint adds (official captures: 2758, 2737,
    /// 2853).
    #[serde(default = "default_sprint_buff_rows")]
    pub sprint_buff_rows: [u16; 3],
}

impl Default for MovementConfig {
    fn default() -> Self {
        Self {
            sprint_buff_rows: default_sprint_buff_rows(),
        }
    }
}

fn default_sprint_buff_rows() -> [u16; 3] {
    [2758, 2737, 2853]
}

/// `[dev]`: development and research tools. None of these are needed for normal play.
#[derive(Debug, Clone, Deserialize)]
pub struct DevConfig {
    /// Optional TSV of terrain seeds known to generate (`area, level, seed, ...`); the server
    /// picks one for areas listed there, since some random seeds fail to generate.
    #[serde(default)]
    pub known_seeds: PathBuf,
    /// Listen for the client DLL's UDP datagrams on the instance port: per-frame position
    /// reports (only read by the drift checker, `drift_check`), and the debug link's overlay
    /// commands. Reports never move the player. Datagrams carry a per-session HMAC and a
    /// counter (docs/protocol/side-channel.md); unauthenticated ones are dropped.
    #[serde(default = "default_true")]
    pub position_sync: bool,
    /// Drift checker: log the server's player position against the client's reported one
    /// (distance in cells) every `drift_interval_ms`. Needs `position_sync` and the DLL; the
    /// reports are compared only, never fed into the simulation.
    #[serde(default)]
    pub drift_check: bool,
    /// Interval of the drift checker's log lines.
    #[serde(default = "default_drift_interval_ms")]
    pub drift_interval_ms: u64,
    /// Movement trace: every client position report, movement input and movement reply
    /// is appended to this TSV file, for analysing a play session. Empty: off.
    #[serde(default)]
    pub movement_trace: PathBuf,
    /// Debug link for the client DLL's overlay: send the player's movement state
    /// (from the instance's simulation) to the DLL (UDP, where its position reports come from), the command registry and instance
    /// list, and accept server commands from its overlay (`commands.rs`). Authenticated per
    /// session, so remote clients work too.
    #[serde(default = "default_true")]
    pub debug_link: bool,
    /// Terrain seed sweep (research, for the `poe-world` generator): a file of
    /// `<WorldAreas.Id> <seed> [count]` lines.
    /// Each instance session generates the next listed area/seed instead of the character's
    /// area, records the client's hashes in `sweep_results` and ends (the client then logs in
    /// again; `scripts/re/sweep_drive.py`). A second AreaInfo on one connection crashes the
    /// client, hence one layout per session. The client DLL's area export records each layout.
    /// `POE_SWEEP` overrides it. Empty: off.
    #[serde(default)]
    pub sweep: PathBuf,
    /// Where the sweep appends `area, seed, tile hash, doodad hash` lines.
    #[serde(default = "default_sweep_results")]
    pub sweep_results: PathBuf,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            known_seeds: PathBuf::new(),
            position_sync: true,
            drift_check: false,
            drift_interval_ms: default_drift_interval_ms(),
            movement_trace: PathBuf::new(),
            debug_link: true,
            sweep: PathBuf::new(),
            sweep_results: default_sweep_results(),
        }
    }
}

fn default_sweep_results() -> PathBuf {
    "dumps/0.5.5e/terrain/vectors/sweep-results.tsv".into()
}

fn default_drift_interval_ms() -> u64 {
    1000
}

fn default_true() -> bool {
    true
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
    /// Stash tab names an account gets (sent in InitialState; official: "1".."4").
    #[serde(default = "default_stash_tabs")]
    pub stash_tabs: Vec<String>,
    /// Dev only: the Steam login (`0x0005`) auto-creates and logs into an account keyed by the
    /// client's machine hash, with no password. Off: only username/password and token logins
    /// (`0x0003`) work.
    #[serde(default)]
    pub dev_machine_hash_login: bool,
    /// Whether the web API may register accounts (WP4.1); server commands always can.
    #[serde(default)]
    pub open_registration: bool,
    /// How long a "Remember Me" login token stays valid (days).
    #[serde(default = "default_remember_days")]
    pub remember_days: u32,
}

fn default_remember_days() -> u32 {
    30
}

fn yes() -> bool {
    true
}

fn default_stash_tabs() -> Vec<String> {
    ["1", "2", "3", "4"].map(String::from).to_vec()
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

    #[test]
    fn example_config_parses() {
        let text = include_str!("../../../config/server.example.toml");
        let cfg = Config::parse(text).expect("config/server.example.toml must parse");
        assert!(!cfg.leagues.is_empty());
    }

    #[test]
    fn advertised_host_warnings() {
        let mut cfg = Config::parse(include_str!("../../../config/server.toml")).unwrap();
        // The shipped config listens on 0.0.0.0 and advertises loopback: fine alone, a warning
        // for remote players.
        assert_eq!(cfg.network.warnings().len(), 1);
        cfg.network.public_host = "192.168.1.20".into();
        assert!(cfg.network.warnings().is_empty());
        cfg.network.public_host = "play.example.org".into();
        assert!(cfg.network.warnings().is_empty());
        cfg.network.public_host = "0.0.0.0".into();
        assert_eq!(cfg.network.warnings().len(), 1);
        // Loopback binds make a loopback host correct.
        cfg.network.public_host = "127.0.0.1".into();
        cfg.network.login.bind = "127.0.0.1:20481".parse().unwrap();
        cfg.network.instance.bind = "127.0.0.1:6112".parse().unwrap();
        assert!(cfg.network.warnings().is_empty());
        assert_eq!(cfg.network.login.public_port(), 20481);
        cfg.network.login.public_port = Some(30481);
        assert_eq!(cfg.network.login.public_port(), 30481);
    }

    #[test]
    fn spawn_upload_defaults_match_the_shipped_config() {
        let cfg = Config::parse(include_str!("../../../config/server.toml")).unwrap();
        let d = UploadConfig::default();
        let u = &cfg.spawn.upload;
        assert_eq!(
            (u.enabled, u.max_bytes, u.max_layout_bytes, u.max_grid_bytes, u.wait_ms, u.wait_upload_secs),
            (d.enabled, d.max_bytes, d.max_layout_bytes, d.max_grid_bytes, d.wait_ms, d.wait_upload_secs)
        );
    }

    #[test]
    fn web_section() {
        let cfg = Config::parse(include_str!("../../../config/server.toml")).unwrap();
        assert!(cfg.web.enabled);
        assert_eq!(cfg.web.bind.to_string(), "127.0.0.1:8080");
        assert_eq!(cfg.web.static_dir, std::path::Path::new("modules/web-admin/dist"));
        assert!(cfg.web.cors_origins.is_empty());
        assert_eq!(cfg.web.log_buffer, 2000);
    }

    /// The example documents the defaults: a config with only the required keys must equal it
    /// for every defaulted section.
    #[test]
    fn example_matches_defaults() {
        let example = Config::parse(include_str!("../../../config/server.example.toml")).unwrap();
        let minimal = Config::parse(
            r#"
            [game]
            version = "x"
            game_dir = "g"
            schema = "s"
            protocol_dir = "p"
            [network]
            public_host = "h"
            [network.login]
            bind = "0.0.0.0:1"
            [network.instance]
            bind = "0.0.0.0:2"
            public_port = 2
            [database]
            path = "d"
            [accounts]
            name_prefix = "Exile"
            max_characters = 1
            [gateway]
            region = "r"
            display_name = "n"
            "#,
        )
        .unwrap();
        assert_eq!(
            format!("{:?}", example.instance),
            format!("{:?}", minimal.instance)
        );
        assert_eq!(
            format!("{:?}", example.movement),
            format!("{:?}", minimal.movement)
        );
        assert_eq!(format!("{:?}", example.chat), format!("{:?}", minimal.chat));
        assert_eq!(
            format!("{:?}", example.cluster),
            format!("{:?}", minimal.cluster)
        );
        assert_eq!(format!("{:?}", example.web), format!("{:?}", minimal.web));
        // Accounts: the example documents the defaults of the optional keys.
        let (e, m) = (&example.accounts, &minimal.accounts);
        assert_eq!(
            (
                e.dev_machine_hash_login,
                e.open_registration,
                e.remember_days
            ),
            (
                m.dev_machine_hash_login,
                m.open_registration,
                m.remember_days
            )
        );
    }
}
