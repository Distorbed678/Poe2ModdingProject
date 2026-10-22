//! Request and response bodies (also the OpenAPI schemas).

use poe_accounts::AccountRecord;
use poe_cluster::{InstanceInfo, OnlinePlayer};
use poe_store::Character;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `GET /server`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ServerInfo {
    /// The gateway's display name.
    pub name: String,
    /// The game client version this server speaks.
    pub version: String,
    pub cluster_id: String,
    /// Players in game now.
    pub online: usize,
    pub open_registration: bool,
    /// Host the game client connects to.
    pub login_host: String,
    /// Port the client connects to for login (`[network.login] public_port`, else the bind port).
    pub login_port: u16,
    /// Port of the instance server the client is sent to (with `login_host`); the launcher
    /// trusts both addresses in the client hook.
    pub instance_port: u16,
    /// Realm region written into the client's gateway list.
    pub gateway_region: String,
    pub uptime_secs: u64,
    /// Message of the day (`[server] motd`, plain text, at most 280 characters); absent when
    /// there is none. The launcher's Home page shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motd: Option<String>,
    /// The client mods a player needs on this server (side `client` or `both`, enabled): the
    /// launcher compares them with its mods and downloads the missing ones. Absent when the
    /// server has none (old servers have no such field either). Server-only mods are not
    /// listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mods: Option<Vec<ModInfo>>,
}

/// A client mod this server requires (`GET /server`, field `mods`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    /// The exact version the server pins (semver).
    pub version: String,
    /// `client` or `both`.
    pub side: String,
    /// SHA-256 of the package zip, 64 lower-case hex digits.
    pub sha256: String,
    /// Size of the package in bytes.
    pub size: u64,
    /// The login check requires it (all listed mods are).
    pub required: bool,
    /// The client mod API the client half was built for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_api: Option<u32>,
    pub game_versions: Vec<String>,
    /// The client half contains a native DLL (code that runs in the game process).
    pub code: bool,
    pub authors: Vec<String>,
    pub description: String,
    /// Where to download the package: the server itself first, then `[mods] extra_urls`.
    pub sources: Vec<ModSource>,
}

/// One download address of a mod package.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModSource {
    /// `server` (a path on this server's web address) or `url` (an absolute address).
    #[serde(rename = "type")]
    pub kind: String,
    /// For `server`: `/api/v1/mods/{id}/{version}.zip`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// For `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// `GET /admin/mods`: every mod in the mods folder and what the check does.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModsAdminInfo {
    /// The mods folder has a `state.toml` (the mod system is on).
    pub active: bool,
    pub mods: Vec<ModStatusInfo>,
    /// `state.toml` or an installed mod changed since the server started: a restart applies it.
    pub restart_required: bool,
    /// `[mods] require_client_mods` and `allow_extra_client_mods`.
    pub require_client_mods: bool,
    pub allow_extra_client_mods: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModStatusInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    /// `client`, `server` or `both`.
    pub side: String,
    pub enabled: bool,
    /// `loaded`, `skipped`, `failed` or `installed` (not enabled).
    pub state: String,
    /// Why a mod is not loaded, or a note.
    pub detail: String,
    /// Chat commands the mod added.
    pub commands: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AccountInfo {
    pub id: u32,
    pub username: String,
    /// `player` or `admin`.
    pub role: String,
    /// Unix seconds.
    pub created_at: i64,
    pub last_login: Option<i64>,
    pub disabled: bool,
    /// `false` for dev machine-hash accounts: they cannot log in with a password.
    pub has_password: bool,
}

impl From<AccountRecord> for AccountInfo {
    fn from(a: AccountRecord) -> Self {
        Self {
            id: a.id,
            username: a.username,
            role: a.role.as_str().into(),
            created_at: a.created_at,
            last_login: a.last_login,
            disabled: a.disabled,
            has_password: a.has_password,
        }
    }
}

/// `POST /register`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterRequest {
    /// 3-64 characters from `A-Za-z0-9_.@+-`.
    pub username: String,
    /// Plain text, at least 8 characters. Use HTTPS outside the local machine.
    pub password: String,
}

/// `POST /login`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    /// `web` (default, 12 hours) or `launcher` (`[accounts] remember_days`; the game login
    /// also accepts it).
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LoginResponse {
    /// 64 hex characters: send as `Authorization: Bearer <token>`. Shown only once.
    pub token: String,
    /// `web`, `launcher` or (from `POST /game-token`) `game_remember`.
    pub kind: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub account: AccountInfo,
}

/// `POST /admin/logs/ws-ticket`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WsTicketResponse {
    /// 64 hex characters, single use: `GET /admin/logs/ws?ticket=<ticket>`. Never log it.
    pub ticket: String,
    /// Seconds from now until it expires (`[web] web_token_ttl_secs`).
    pub expires_in_secs: u64,
}

/// `POST /web-token`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WebTokenResponse {
    /// 64 hex characters. Single use: redeem it with `POST /web-token/redeem` (the web admin
    /// does this itself when opened through `login_path`). Shown only once; never log it.
    pub token: String,
    /// Seconds from now until it expires (`[web] web_token_ttl_secs`).
    pub expires_in_secs: u64,
    /// Where to open the web admin so it signs in with this token: append it to the server's
    /// web address, e.g. `http://host:8080` + `login_path`. The token is in the URL fragment,
    /// so it is not sent to the server, not logged by proxies and not sent as a referrer; the
    /// web admin removes it from the address bar and the history at once.
    pub login_path: String,
}

/// `POST /web-token/redeem`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct WebTokenRedeemRequest {
    /// The `token` from `POST /web-token`.
    pub token: String,
}

/// `POST /password`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PasswordRequest {
    pub old_password: String,
    /// Plain text, at least 8 characters.
    pub new_password: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OnlinePlayerInfo {
    pub character: String,
    pub account: String,
    pub account_id: u32,
    /// Level of the character, when it is in the database.
    pub level: Option<u32>,
    /// `Characters.IntegerId` of the character's class.
    pub class_id: Option<u32>,
    pub instance: Option<u32>,
    /// `WorldAreas.Id`.
    pub area: String,
    /// When the session started (unix seconds).
    pub connected_at: i64,
    /// The client's address (admin API only), when known.
    pub ip: Option<String>,
    /// The character cannot chat.
    pub muted: bool,
}

impl From<OnlinePlayer> for OnlinePlayerInfo {
    fn from(o: OnlinePlayer) -> Self {
        Self {
            character: o.character,
            account: o.account,
            account_id: o.account_id,
            level: None,
            class_id: None,
            instance: o.instance,
            area: o.area,
            connected_at: o.connected_at,
            ip: o.ip,
            muted: o.muted,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InstanceView {
    pub id: u32,
    pub area: String,
    pub seed: Option<u32>,
    /// Forced topology (1-based); absent: the client's choice.
    pub topology: Option<u32>,
    pub orientation: Option<u8>,
    /// Character names.
    pub players: Vec<String>,
    pub created_unix_ms: u64,
    /// Towns and hideouts: kept when empty.
    pub persistent: bool,
    /// Seconds since the instance went empty; absent while players are in it.
    pub empty_for_secs: Option<u64>,
}

impl From<InstanceInfo> for InstanceView {
    fn from(i: InstanceInfo) -> Self {
        Self {
            id: i.id,
            area: i.spec.area,
            seed: i.spec.seed,
            topology: i.spec.topology,
            orientation: i.spec.orientation,
            players: i.players,
            created_unix_ms: i.created_unix_ms,
            persistent: i.persistent,
            empty_for_secs: i.empty_since.map(|t| t.elapsed().as_secs()),
        }
    }
}

/// `POST /admin/instances`, and the `New` form of a transfer.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct NewInstanceRequest {
    /// `WorldAreas.Id`, e.g. `G1_1`.
    pub area: String,
    /// Terrain seed; absent: the server picks one.
    pub seed: Option<u32>,
    /// Forced topology, 1-based in the area's list; absent: the client's choice.
    pub topology: Option<u32>,
    /// Forced orientation 0..8; absent: the client's choice.
    pub orientation: Option<u8>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreatedInstance {
    pub id: u32,
}

/// `POST /admin/players/{name}/transfer`: give `instance`, or `area` (with the optional
/// layout fields) for a new one.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TransferRequest {
    pub instance: Option<u32>,
    pub area: Option<String>,
    pub seed: Option<u32>,
    pub topology: Option<u32>,
    pub orientation: Option<u8>,
}

/// `POST /admin/players/{name}/mute`.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct MuteRequest {
    /// Absent: until unmuted.
    pub minutes: Option<u32>,
}

/// `POST /admin/announce`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnnounceRequest {
    pub message: String,
}

/// `POST /admin/server/stop` and `/admin/server/restart` (the body is optional).
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ShutdownRequest {
    /// Countdown in seconds (0..=3600, default 0): announced to the players in game, then the
    /// server shuts down.
    pub delay_secs: Option<u32>,
    /// Added to every announcement (at most 200 characters).
    pub message: Option<String>,
}

/// `GET /admin/server/shutdown`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ShutdownStatus {
    /// The requested shutdown that has not happened yet, if any.
    pub pending: Option<crate::lifecycle::PendingShutdown>,
}

/// `POST /admin/accounts/{name}/role`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RoleRequest {
    /// `player` or `admin`.
    pub role: String,
}

/// What a command printed.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CommandResult {
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LocationInfo {
    pub seed: u32,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CharacterInfo {
    pub id: u32,
    pub account_id: u32,
    /// The owning account's name, when it is known.
    pub account: Option<String>,
    pub name: String,
    pub league: String,
    /// `Characters.IntegerId` of the class.
    pub class_id: u32,
    pub start_state: String,
    pub level: u32,
    pub experience: u64,
    /// `WorldAreas.Id` of the current or return area.
    pub area: String,
    pub intro_done: bool,
    pub location: Option<LocationInfo>,
    pub online: bool,
}

impl CharacterInfo {
    pub fn new(c: Character, account: Option<String>, online: bool) -> Self {
        Self {
            id: c.id,
            account_id: c.account_id,
            account,
            name: c.name,
            league: c.league,
            class_id: c.class_id,
            start_state: c.start_state,
            level: c.level,
            experience: c.experience,
            area: c.area,
            intro_done: c.intro_done,
            location: c.location.map(|l| LocationInfo {
                seed: l.seed,
                x: l.x,
                y: l.y,
            }),
            online,
        }
    }
}
