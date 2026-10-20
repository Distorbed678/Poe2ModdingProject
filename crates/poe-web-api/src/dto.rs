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
    pub login_port: u16,
    pub uptime_secs: u64,
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
    /// `web` or `launcher`.
    pub kind: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub account: AccountInfo,
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
    pub instance: Option<u32>,
    /// `WorldAreas.Id`.
    pub area: String,
}

impl From<OnlinePlayer> for OnlinePlayerInfo {
    fn from(o: OnlinePlayer) -> Self {
        Self {
            character: o.character,
            account: o.account,
            instance: o.instance,
            area: o.area,
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
