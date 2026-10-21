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

pub mod versions;
pub use versions::{ClientCheck, VersionFile};

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
    /// `[game] version` names no supported build (`versions/<version>.toml`).
    #[error("{path}: {source}")]
    Version {
        path: PathBuf,
        source: versions::VersionError,
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
    /// Password attempt limits of the login server and the web API.
    #[serde(default)]
    pub security: SecurityConfig,
    /// The build `[game] version` selects (`versions/<version>.toml`). Filled by
    /// [`Config::load`]; `None` for a config made with [`Config::parse`] (tests), which turns
    /// the client version check off.
    #[serde(skip)]
    pub version_info: Option<VersionFile>,
}

/// Rate limiting of password logins (the game login `0x0003` and the web API `/login` and
/// `/password`; `crates/poe-accounts/src/ratelimit.rs`). Failed attempts are counted per
/// account name and per client IP over a sliding window; at the limit the key is locked for
/// `login_lockout_secs` and further attempts are refused **before** the password is checked.
/// A limit of 0 turns that limit off. Saved-token logins are not counted (a token cannot be
/// guessed), and a successful login clears the account's count.
#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    /// Failed password attempts per account name within the window before it is locked.
    #[serde(default = "default_max_failures_account")]
    pub login_max_failures_per_account: u32,
    /// Failed password attempts per client IP (IPv6: per /64) within the window.
    #[serde(default = "default_max_failures_ip")]
    pub login_max_failures_per_ip: u32,
    /// Window in which failures are counted (seconds).
    #[serde(default = "default_failure_window")]
    pub login_failure_window_secs: u64,
    /// How long a locked key stays locked (seconds).
    #[serde(default = "default_lockout")]
    pub login_lockout_secs: u64,
    /// Behind a reverse proxy: take the client IP of the web API from the first entry of
    /// `X-Forwarded-For` instead of the socket address. Only turn this on when a proxy you
    /// control sets the header (otherwise a client can pick its own IP).
    #[serde(default)]
    pub trust_forwarded_for: bool,
    /// Accept a client whose `client_hash` (a SHA-256 of its exe, `0x0003`/`0x0005`) is not a
    /// known value of `[game] version` (`versions/<version>.toml`, `[client] hashes`). Off by
    /// default: such a client is refused with the "you need to update" popup
    /// (`BackendErrors.InvalidProtocolVersion`), because a build the server does not know
    /// speaks messages it may misread. Turn it on to try an unlisted build.
    #[serde(default)]
    pub allow_unknown_client: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            login_max_failures_per_account: default_max_failures_account(),
            login_max_failures_per_ip: default_max_failures_ip(),
            login_failure_window_secs: default_failure_window(),
            login_lockout_secs: default_lockout(),
            trust_forwarded_for: false,
            allow_unknown_client: false,
        }
    }
}

fn default_max_failures_account() -> u32 {
    5
}

fn default_max_failures_ip() -> u32 {
    20
}

fn default_failure_window() -> u64 {
    300
}

fn default_lockout() -> u64 {
    300
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
    /// How long a single-use web sign-in token (`POST /api/v1/web-token`, the launcher's "Open
    /// web admin") can be redeemed, in seconds. Must be at least 1.
    #[serde(default = "default_web_token_ttl")]
    pub web_token_ttl_secs: u64,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: default_web_bind(),
            static_dir: PathBuf::new(),
            cors_origins: Vec::new(),
            log_buffer: default_log_buffer(),
            web_token_ttl_secs: default_web_token_ttl(),
        }
    }
}

fn default_web_bind() -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], 8080))
}

fn default_log_buffer() -> usize {
    2000
}

fn default_web_token_ttl() -> u64 {
    60
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
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
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
    pub upload: UploadConfig,
    /// Smallest distance in grid cells between a joining player's spawn cell and the players
    /// already in the instance; a joiner whose spawn is closer than this stands on the nearest
    /// free walkable cell instead (0 turns it off).
    pub player_spacing: i32,
}

impl Default for SpawnConfig {
    fn default() -> Self {
        Self {
            layout_dir: String::new(),
            filler: Vec::new(),
            start_tiles: Default::default(),
            intro_areas: Vec::new(),
            upload: UploadConfig::default(),
            player_spacing: 8,
        }
    }
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
    /// The supported client build: names `versions/<version>.toml` at the repository root
    /// (`cargo xtask check-build`, docs/guides/new-game-version.md). `{version}` in other path
    /// settings of this file is replaced by it.
    pub version: String,
    pub game_dir: PathBuf,
    pub schema: PathBuf,
    /// Opcode tables. Default (empty): the version file's `[protocol] opcodes`.
    #[serde(default)]
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

impl Config {
    /// Settings that are fine on a developer machine and dangerous on a server others can
    /// reach (logged at startup; none of them stops the server). The committed
    /// `config/server.toml` triggers none of them.
    pub fn security_warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.accounts.dev_machine_hash_login.is_some() {
            out.push(
                "[accounts] dev_machine_hash_login is gone and ignored: the password-less Steam login \
                 (0x0005) was removed, every login is an account name and password or a saved \
                 login token (remove the key from this config)"
                    .into(),
            );
        }
        if !self.web.bind.ip().is_loopback() {
            out.push(format!(
                "[web] bind {} is not loopback: passwords and admin actions travel in the clear \
                 unless a TLS reverse proxy sits in front (docs/guides/hosting.md)",
                self.web.bind
            ));
        }
        if self.accounts.open_registration && !self.web.bind.ip().is_loopback() {
            out.push("[accounts] open_registration is on and the web API is reachable".into());
        }
        out
    }
}

impl NetworkConfig {
    /// Problems with the advertised address that would send remote clients to the wrong place
    /// (logged at startup; none of them stops the server).
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let host = self.public_host.trim();
        let loopback = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        let listens_beyond_loopback =
            !self.login.bind.ip().is_loopback() || !self.instance.bind.ip().is_loopback();
        if host.is_empty() {
            out.push("[network] public_host is empty: clients get no address".into());
        } else if loopback && listens_beyond_loopback {
            out.push(format!(
                "[network] public_host = {host:?} is a loopback address: only clients on this \
                 machine can connect (set the LAN or public address for other players, \
                 docs/guides/hosting.md)"
            ));
        }
        if host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_unspecified())
        {
            out.push(
                "[network] public_host = \"0.0.0.0\" is not an address clients can reach".into(),
            );
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
    /// Before a login: longest time (seconds) one message may take to arrive completely, the
    /// key exchange included. A slow or idle socket is closed. 0 = no limit.
    #[serde(default = "default_pre_auth_read_timeout_secs")]
    pub pre_auth_read_timeout_secs: u64,
    /// Before a login: longest time (seconds) from connecting to a successful login. 0 = no limit.
    #[serde(default = "default_pre_auth_login_timeout_secs")]
    pub pre_auth_login_timeout_secs: u64,
    /// Before a login: most bytes held for an incomplete message (the real messages are far
    /// smaller). 0 = no limit.
    #[serde(default = "default_pre_auth_max_buffer")]
    pub pre_auth_max_buffer: usize,
    /// Most connections that have not logged in yet, all addresses together. 0 = no limit.
    #[serde(default = "default_max_pre_auth_connections")]
    pub max_pre_auth_connections: usize,
    /// Most connections that have not logged in yet from one client IP (IPv6: per /64).
    /// 0 = no limit.
    #[serde(default = "default_max_pre_auth_per_ip")]
    pub max_pre_auth_per_ip: usize,
}

fn default_pre_auth_read_timeout_secs() -> u64 {
    15
}

fn default_pre_auth_login_timeout_secs() -> u64 {
    60
}

fn default_pre_auth_max_buffer() -> usize {
    64 * 1024
}

fn default_max_pre_auth_connections() -> usize {
    256
}

fn default_max_pre_auth_per_ip() -> usize {
    16
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
    /// Before the ticket is checked: longest time (seconds) for the client's first message to
    /// arrive completely. 0 = no limit.
    #[serde(default = "default_pre_auth_read_timeout_secs")]
    pub pre_auth_read_timeout_secs: u64,
    /// Before the ticket is checked: most bytes held for an incomplete message. 0 = no limit.
    #[serde(default = "default_pre_auth_max_buffer")]
    pub pre_auth_max_buffer: usize,
    /// Most connections that have not shown a valid ticket yet, all addresses together.
    /// 0 = no limit.
    #[serde(default = "default_max_pre_auth_connections")]
    pub max_pre_auth_connections: usize,
    /// Most such connections from one client IP (IPv6: per /64). 0 = no limit.
    #[serde(default = "default_max_pre_auth_per_ip")]
    pub max_pre_auth_per_ip: usize,
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
    /// After a sprint ends the client's walk is over and it coasts to a stop along its last
    /// direction. First step as a multiple of the walking step of a client frame (default 1.0614:
    /// 1.309 cells against 1.233; measured on the official 2026-10-20 sprint ends and six sessions
    /// of ours, docs/protocol/movement.md "Coasting after a sprint").
    #[serde(default = "default_sprint_coast_start_scale")]
    pub sprint_coast_start_scale: f32,
    /// Frames over which the coasting step falls linearly to zero (default 29.5: 30 steps, about
    /// 20 cells; same source).
    #[serde(default = "default_sprint_coast_frames")]
    pub sprint_coast_frames: f32,
    /// Frames between the sprint-off batch and the first coasting step (default 1).
    #[serde(default = "default_sprint_coast_delay_frames")]
    pub sprint_coast_delay_frames: u32,
    /// Frames between the sprint-off batch and the first step of a walk the unscaled direction
    /// ack restarts, when a key is still held (default 2: new Move action 2 frames after the
    /// batch, first step one later; official `f2394` -> Move at `f2395`, step `f2396`).
    #[serde(default = "default_sprint_restart_delay_frames")]
    pub sprint_restart_delay_frames: u32,
    /// A zero direction this many frames after the sprint-off batch, after a sprint that
    /// followed the cursor (Space only), means the keys were already up when the restart reached
    /// the client, which coasts (default 6: the client sends the zero 1 to 2 frames after the
    /// Space release with a free-angle cursor, official `f4584` -> `f4586`, and 4 to 5 with an
    /// exactly diagonal one). In click-to-move mode (no keys) the sprint coasts at once and this
    /// is how long cursor vectors still in flight are ignored.
    #[serde(default = "default_sprint_release_window_frames")]
    pub sprint_release_window_frames: u32,
    /// A key-shaped direction (axis or exact diagonal) after a sprint counts as the cursor's when
    /// the roll key's cursor offset points within this many degrees of it (default 10; the
    /// client's cursor vector and the roll target differ by up to about 6 degrees). An exactly
    /// diagonal cursor sends the same vector as W.
    #[serde(default = "default_sprint_cursor_match_degrees")]
    pub sprint_cursor_match_degrees: f32,
}

impl Default for MovementConfig {
    fn default() -> Self {
        Self {
            sprint_buff_rows: default_sprint_buff_rows(),
            sprint_coast_start_scale: default_sprint_coast_start_scale(),
            sprint_coast_frames: default_sprint_coast_frames(),
            sprint_coast_delay_frames: default_sprint_coast_delay_frames(),
            sprint_restart_delay_frames: default_sprint_restart_delay_frames(),
            sprint_release_window_frames: default_sprint_release_window_frames(),
            sprint_cursor_match_degrees: default_sprint_cursor_match_degrees(),
        }
    }
}

fn default_sprint_buff_rows() -> [u16; 3] {
    [2758, 2737, 2853]
}

fn default_sprint_coast_start_scale() -> f32 {
    1.0614
}

fn default_sprint_coast_frames() -> f32 {
    29.5
}

fn default_sprint_coast_delay_frames() -> u32 {
    1
}

fn default_sprint_restart_delay_frames() -> u32 {
    2
}

fn default_sprint_release_window_frames() -> u32 {
    6
}

fn default_sprint_cursor_match_degrees() -> f32 {
    10.0
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
    /// Remote player blob experiments (the model other players see: `docs/protocol/entities.md`,
    /// "Remote player look"). Parts of the official remote layout the server generates that can
    /// be left out one by one to find which one a client reacts to: `"stat_sets"` (the weapon
    /// set stat sets), `"derived_stats"` (full life/mana conditions, poise, recharge delay),
    /// `"life_state"` (the pools as the official server sends them: nothing reserved, flags 1),
    /// `"player_flags"` (Player flags bit 3). Empty: everything is sent.
    #[serde(default)]
    pub remote_blob_omit: Vec<String>,
    /// Also send the class's animated object path in the remote blob (as the player's own blob
    /// does; official blobs send none).
    #[serde(default)]
    pub remote_animated_path: bool,
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
            remote_blob_omit: Vec::new(),
            remote_animated_path: false,
        }
    }
}

fn default_sweep_results() -> PathBuf {
    "dumps/{version}/terrain/vectors/sweep-results.tsv".into()
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
    /// The account flag bytes of the authentication result (`docs/protocol/login.md`). Default
    /// `[0x23, 0x03]`: no terms-of-use, warning or update windows, no strict-unlock banner.
    #[serde(default = "default_account_flags")]
    pub flags: [u8; 2],
    /// Full game access (InitialState CharacterState flags byte 1 bit 6); without it the
    /// in-game escape menu shows a "Get Full Access" button.
    #[serde(default = "yes")]
    pub full_access: bool,
    /// Stash tab names an account gets (sent in InitialState; official: "1".."4").
    #[serde(default = "default_stash_tabs")]
    pub stash_tabs: Vec<String>,
    /// Removed: the password-less Steam login (`0x0005`) no longer exists (a `0x0005` is always
    /// refused). The key is still read so an old config loads; any value only gives a startup
    /// warning ([`Config::security_warnings`]).
    #[serde(default)]
    pub dev_machine_hash_login: Option<bool>,
    /// Whether the web API may register accounts (WP4.1); server commands always can.
    #[serde(default)]
    pub open_registration: bool,
    /// How long a "Remember Me" login token stays valid (days).
    #[serde(default = "default_remember_days")]
    pub remember_days: u32,
}

fn default_account_flags() -> [u8; 2] {
    [0x23, 0x03]
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
        Self::load_in(path, Path::new("."))
    }

    /// [`Config::load`] with the repository root that `versions/` is read from.
    pub fn load_in(path: &Path, root: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
        let mut config = Self::parse(&text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        // The version file is read relative to the working directory like every other path of
        // the config (the repository root).
        let info = VersionFile::load(root, &config.game.version).map_err(|source| {
            ConfigError::Version {
                path: path.to_owned(),
                source,
            }
        })?;
        if config.game.protocol_dir.as_os_str().is_empty() {
            config.game.protocol_dir = info.opcodes_dir(root);
        }
        config.version_info = Some(info);
        Ok(config)
    }

    /// Parses the text (no files are read: see [`Config::load`] for the version file).
    /// `{version}` in the path settings that name a build's data is replaced by
    /// `[game] version`.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut config: Self = toml::from_str(text)?;
        let version = config.game.version.clone();
        let expand = |s: &str| s.replace("{version}", &version);
        config.game.protocol_dir = expand(&config.game.protocol_dir.to_string_lossy()).into();
        config.dev.known_seeds = expand(&config.dev.known_seeds.to_string_lossy()).into();
        config.dev.sweep_results = expand(&config.dev.sweep_results.to_string_lossy()).into();
        config.spawn.layout_dir = expand(&config.spawn.layout_dir);
        Ok(config)
    }

    /// Judges a client's `client_hash` against the version's known values
    /// (`versions/<version>.toml`): `Known`, `Unverified` (the version cannot judge yet) and
    /// `UnknownAllowed` (`[security] allow_unknown_client`) are accepted, `Unknown` is refused. A config without a version file
    /// (`Config::parse`, tests) accepts every hash.
    pub fn client_check(&self, hash: &[u8; 32]) -> ClientCheck {
        match &self.version_info {
            None => ClientCheck::Known,
            Some(v) => match v.check_client_hash(hash) {
                ClientCheck::Unknown if self.security.allow_unknown_client => {
                    ClientCheck::UnknownAllowed
                }
                other => other,
            },
        }
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
    fn load_selects_the_version_file_and_refuses_an_unknown_version() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cfg = Config::load_in(&root.join("config/server.toml"), &root).unwrap();
        let v = cfg.version_info.as_ref().expect("version file attached");
        assert_eq!(v.version, cfg.game.version);
        // protocol_dir is not in the shipped config: it comes from the version file.
        assert_eq!(cfg.game.protocol_dir, root.join(&v.protocol.opcodes));
        // `{version}` is replaced in the path settings.
        // `{version}` is replaced in the path settings (known_seeds is empty in the safe
        // committed config; the dev config sets a path with the placeholder).
        let dev = Config::parse(&std::fs::read_to_string(root.join("config/server.toml")).unwrap().replace(
            "known_seeds = \"\"",
            "known_seeds = \"dumps/{version}/terrain/seeds.tsv\"",
        ))
        .unwrap();
        assert_eq!(
            dev.dev.known_seeds,
            Path::new(&format!("dumps/{}/terrain/seeds.tsv", cfg.game.version))
        );
        assert!(!cfg.spawn.layout_dir.contains('{'));
        assert!(!cfg.dev.sweep_results.to_string_lossy().contains('{'));

        let dir = std::env::temp_dir().join(format!("poe-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = std::fs::read_to_string(root.join("config/server.toml"))
            .unwrap()
            .replacen("version = \"0.5.5e\"", "version = \"9.9.9z\"", 1);
        let bad = dir.join("server.toml");
        std::fs::write(&bad, text).unwrap();
        let e = Config::load_in(&bad, &root).unwrap_err().to_string();
        assert!(
            e.contains("9.9.9z") && e.contains("not a supported build") && e.contains("0.5.5e"),
            "{e}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn advertised_host_warnings() {
        let mut cfg = Config::parse(include_str!("../../../config/server.toml")).unwrap();
        // The committed config listens on loopback and advertises loopback: correct.
        assert!(cfg.network.warnings().is_empty());
        // Listening beyond loopback and advertising loopback: a warning for remote players.
        cfg.network.login.bind = "0.0.0.0:20481".parse().unwrap();
        cfg.network.instance.bind = "0.0.0.0:6112".parse().unwrap();
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

    /// S-01 of the public release audit (docs/plans/public-release-audit.md): the committed
    /// config is safe to run on a checkout. Development setups use runtime/server.dev.toml
    /// (scripts/dev-config.py).
    #[test]
    fn committed_config_is_safe() {
        let cfg = Config::parse(include_str!("../../../config/server.toml")).unwrap();
        assert!(cfg.accounts.dev_machine_hash_login.is_none(), "removed key");
        assert!(cfg.network.login.bind.ip().is_loopback());
        assert!(cfg.network.instance.bind.ip().is_loopback());
        assert!(cfg.web.bind.ip().is_loopback());
        assert!(!cfg.accounts.open_registration);
        assert!(!cfg.security.trust_forwarded_for);
        assert!(!cfg.security.allow_unknown_client);
        assert!(!cfg.dev.debug_link && !cfg.dev.position_sync && !cfg.dev.drift_check);
        assert!(cfg.dev.movement_trace.as_os_str().is_empty());
        assert!(cfg.dev.sweep.as_os_str().is_empty());
        assert!(!cfg.terrain.generate);
        // The pre-login limits are on.
        let l = &cfg.network.login;
        assert!(l.pre_auth_read_timeout_secs > 0 && l.pre_auth_login_timeout_secs > 0);
        assert!(l.pre_auth_max_buffer > 0 && l.max_pre_auth_connections > 0 && l.max_pre_auth_per_ip > 0);
        let i = &cfg.network.instance;
        assert!(i.pre_auth_read_timeout_secs > 0 && i.pre_auth_max_buffer > 0);
        assert!(i.max_pre_auth_connections > 0 && i.max_pre_auth_per_ip > 0);
        // Same for the example (a hosting template: it may listen beyond loopback, but nothing
        // password-less).
        let ex = Config::parse(include_str!("../../../config/server.example.toml")).unwrap();
        assert!(ex.accounts.dev_machine_hash_login.is_none());
        assert!(!ex.accounts.open_registration);
        assert!(ex.web.bind.ip().is_loopback());
    }

    #[test]
    fn security_warnings_name_the_dangerous_combinations() {
        let mut cfg = Config::parse(include_str!("../../../config/server.toml")).unwrap();
        assert!(cfg.security_warnings().is_empty());
        // An old config that still has the removed key loads and gets a warning.
        cfg.accounts.dev_machine_hash_login = Some(true);
        let w = cfg.security_warnings();
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("is gone and ignored"), "{w:?}");
        cfg.accounts.dev_machine_hash_login = None;
        cfg.network.login.bind = "0.0.0.0:20481".parse().unwrap();
        assert!(cfg.security_warnings().is_empty());
        cfg.web.bind = "0.0.0.0:8080".parse().unwrap();
        cfg.accounts.open_registration = true;
        assert_eq!(cfg.security_warnings().len(), 2);
    }

    #[test]
    fn pre_login_limit_defaults_match_the_example() {
        let ex = Config::parse(include_str!("../../../config/server.example.toml")).unwrap();
        let min = Config::parse(
            "[game]\nversion=\"x\"\ngame_dir=\"g\"\nschema=\"s\"\n[network]\npublic_host=\"h\"\n\
             [network.login]\nbind=\"0.0.0.0:1\"\n[network.instance]\nbind=\"0.0.0.0:2\"\npublic_port=2\n\
             [database]\npath=\"d\"\n[accounts]\nname_prefix=\"E\"\nmax_characters=1\n\
             [gateway]\nregion=\"r\"\ndisplay_name=\"n\"\n",
        )
        .unwrap();
        let t = |c: &NetworkLoginConfig| {
            (
                c.pre_auth_read_timeout_secs,
                c.pre_auth_login_timeout_secs,
                c.pre_auth_max_buffer,
                c.max_pre_auth_connections,
                c.max_pre_auth_per_ip,
            )
        };
        assert_eq!(t(&ex.network.login), t(&min.network.login));
        let i = |c: &NetworkInstanceConfig| {
            (
                c.pre_auth_read_timeout_secs,
                c.pre_auth_max_buffer,
                c.max_pre_auth_connections,
                c.max_pre_auth_per_ip,
            )
        };
        assert_eq!(i(&ex.network.instance), i(&min.network.instance));
    }

    #[test]
    fn spawn_upload_defaults_match_the_shipped_config() {
        let cfg = Config::parse(include_str!("../../../config/server.toml")).unwrap();
        let d = UploadConfig::default();
        let u = &cfg.spawn.upload;
        assert_eq!(
            (
                u.enabled,
                u.max_bytes,
                u.max_layout_bytes,
                u.max_grid_bytes,
                u.wait_ms,
                u.wait_upload_secs
            ),
            (
                d.enabled,
                d.max_bytes,
                d.max_layout_bytes,
                d.max_grid_bytes,
                d.wait_ms,
                d.wait_upload_secs
            )
        );
    }

    #[test]
    fn web_section() {
        let cfg = Config::parse(include_str!("../../../config/server.toml")).unwrap();
        assert!(cfg.web.enabled);
        assert_eq!(cfg.web.bind.to_string(), "127.0.0.1:8080");
        assert_eq!(
            cfg.web.static_dir,
            std::path::Path::new("modules/web-admin/dist")
        );
        assert!(cfg.web.cors_origins.is_empty());
        assert_eq!(cfg.web.log_buffer, 2000);
        assert_eq!(cfg.web.web_token_ttl_secs, 60);
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
        assert_eq!(
            format!("{:?}", example.security),
            format!("{:?}", minimal.security)
        );
        assert_eq!(example.accounts.flags, minimal.accounts.flags);
        assert_eq!(minimal.accounts.flags, [0x23, 0x03]);
        // Accounts: the example documents the defaults of the optional keys.
        let (e, m) = (&example.accounts, &minimal.accounts);
        assert_eq!(
            (e.open_registration, e.remember_days),
            (m.open_registration, m.remember_days)
        );
    }
}
