//! Game administration seen from outside the instance server (the web API).
//!
//! The instance server owns the command registry (`servers/instance-server/src/commands.rs`:
//! kick, mute, announce, instances, accounts, ...) and the sessions in game. [`AdminControl`]
//! is the small surface the web API needs from it, so the API runs the same commands through
//! the same registry (role check, audit log) instead of reimplementing them, and so tests can
//! substitute a mock.

/// A player in game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnlinePlayer {
    pub character: String,
    pub account: String,
    pub instance: Option<u32>,
    /// `WorldAreas.Id`.
    pub area: String,
}

/// What a command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminOutcome {
    pub ok: bool,
    pub text: String,
}

impl AdminOutcome {
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            ok: true,
            text: text.into(),
        }
    }

    pub fn err(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: text.into(),
        }
    }
}

/// A new instance: an area and an optional layout choice (`None`: the server's or the
/// client's choice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInstance {
    /// `WorldAreas.Id`.
    pub area: String,
    pub seed: Option<u32>,
    /// 1-based topology of the area.
    pub topology: Option<u32>,
    pub orientation: Option<u8>,
}

/// Where a transferred player goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferTarget {
    /// An existing instance.
    Instance(u32),
    /// A new instance, as the `transfer` command creates it.
    New(NewInstance),
}

pub trait AdminControl: Send + Sync {
    /// The players in game now.
    fn online(&self) -> Vec<OnlinePlayer>;
    /// Runs a registry command line (`kick Bob`, `announce hello`, `instance_new G1_1`, ...,
    /// without the chat prefix) as the account `actor`. The actor's admin role is checked now;
    /// commands that need the actor's own game session (`where`, `join`, `teleport`) are
    /// refused. Logged and audited like a chat command.
    fn run_command(&self, actor: &str, line: &str) -> AdminOutcome;
    /// Creates an instance without moving anyone (`instance_new`); returns its id, or why not
    /// (`ok` is false). Same role check and audit as [`run_command`](Self::run_command).
    fn create_instance(&self, actor: &str, new: &NewInstance) -> Result<u32, AdminOutcome>;
    /// Moves a player in game to another instance (the `transfer` command, but for someone
    /// else). Same role check and audit as [`run_command`](Self::run_command).
    fn transfer_player(&self, actor: &str, character: &str, target: TransferTarget)
    -> AdminOutcome;
    /// Logs an admin action that is not a registry command (a server stop or restart) and
    /// appends it to the audit file, like [`run_command`](Self::run_command) does for commands.
    /// The default does nothing (mocks).
    fn record(&self, _actor: &str, _action: &str, _result: &str) {}
}
