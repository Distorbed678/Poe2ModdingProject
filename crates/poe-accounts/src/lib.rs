//! Accounts, passwords and tokens (WP3.2). See `docs/architecture/accounts.md`.
//!
//! The client never sends a password: the standalone login sends `SHA-256(UTF-16LE(password))`
//! ([`password_digest`]). The stored secret is argon2id over that 32-byte digest, so the web
//! API and the server commands must hash the same way before they call [`AccountStore`].
//!
//! Tokens (the client's Remember Me file, the launcher, the web) are 32 random bytes. Only
//! their SHA-256 is stored, with an expiry, a revoked flag, the account and the `[cluster] id`
//! of the cluster that issued them.

mod commands;
mod ratelimit;
mod sqlite;

use sha2::{Digest, Sha256};

pub use commands::{CommandOutput, redact, run_account_command};
pub use ratelimit::{LoginLimiter, Locked};
pub use sqlite::SqliteAccounts;
/// Argon2 cost parameters (see [`SqliteAccounts::with_params`]).
pub use argon2::Params;

/// What the client puts on the wire as `password_hash` (`0x1421bbd70`): SHA-256 over the
/// UTF-16LE bytes of the password, no salt.
pub fn password_digest(password: &str) -> [u8; 32] {
    let bytes: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    Sha256::digest(bytes).into()
}

/// What is stored for a token: SHA-256 of its 32 bytes.
pub fn token_hash(token: &[u8; 32]) -> [u8; 32] {
    Sha256::digest(token).into()
}

/// Shortest password the server commands accept (the client itself has no limit).
pub const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Player,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Player => "player",
            Role::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "player" => Some(Role::Player),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// The client's "Remember Me" credentials file (`0x0003` with `token_login = 1`).
    GameRemember,
    /// Issued to the launcher (WP4/launcher).
    Launcher,
    /// Issued to the web API.
    Web,
}

impl TokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::GameRemember => "game_remember",
            TokenKind::Launcher => "launcher",
            TokenKind::Web => "web",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "game_remember" => Some(TokenKind::GameRemember),
            "launcher" => Some(TokenKind::Launcher),
            "web" => Some(TokenKind::Web),
            _ => None,
        }
    }
}

/// An account as the rest of the server sees it (never the password hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRecord {
    /// The same id as `poe_store::Account::id` (one table).
    pub id: u32,
    pub username: String,
    pub role: Role,
    /// Unix seconds.
    pub created_at: i64,
    pub last_login: Option<i64>,
    pub disabled: bool,
    /// `false` for the machine-hash dev accounts: they cannot log in with a password.
    pub has_password: bool,
}

/// A freshly issued token: the only time the raw bytes exist server-side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedToken {
    pub token: [u8; 32],
    /// Unix seconds.
    pub expires_at: i64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AccountError {
    #[error("no such account")]
    NoAccount,
    #[error("wrong password")]
    WrongPassword,
    #[error("account is disabled")]
    Disabled,
    #[error("unknown token")]
    TokenUnknown,
    #[error("token expired")]
    TokenExpired,
    #[error("token revoked")]
    TokenRevoked,
    #[error("token was issued by another cluster")]
    TokenWrongCluster,
    #[error("that username is taken")]
    NameTaken,
    #[error("invalid username: {0}")]
    InvalidName(&'static str),
    #[error("password must be at least {MIN_PASSWORD_LEN} characters")]
    WeakPassword,
    #[error("database: {0}")]
    Db(String),
}

pub type Result<T> = std::result::Result<T, AccountError>;

/// Checks a username: 3-64 characters from `A-Za-z0-9_.@+-` (email addresses work).
pub fn validate_username(name: &str) -> Result<()> {
    let len = name.chars().count();
    if !(3..=64).contains(&len) {
        return Err(AccountError::InvalidName("3 to 64 characters"));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || "_.@+-".contains(c)) {
        return Err(AccountError::InvalidName("letters, digits and _ . @ + - only"));
    }
    Ok(())
}

/// Accounts, passwords and tokens behind a trait, so the SQLite implementation can be swapped.
/// Passwords are passed as the client's [`password_digest`], never as plain text.
pub trait AccountStore: Send + Sync {
    /// `[cluster] id`: tokens issued under another id never validate.
    fn cluster_id(&self) -> &str;
    /// Creates an account with a password; the username must be free (case-insensitively).
    fn create_account(&self, username: &str, password_digest: &[u8; 32], role: Role) -> Result<AccountRecord>;
    fn find(&self, username: &str) -> Result<Option<AccountRecord>>;
    fn find_by_id(&self, id: u32) -> Result<Option<AccountRecord>>;
    /// All accounts by id.
    fn list(&self) -> Result<Vec<AccountRecord>>;
    /// Username plus password: `NoAccount`, `WrongPassword` (also for accounts without a
    /// password) or `Disabled` (checked after the password). Records the login.
    fn authenticate(&self, username: &str, password_digest: &[u8; 32]) -> Result<AccountRecord>;
    /// Sets a new password and revokes the account's tokens.
    fn set_password(&self, username: &str, password_digest: &[u8; 32]) -> Result<AccountRecord>;
    fn set_role(&self, username: &str, role: Role) -> Result<AccountRecord>;
    /// Disabling also revokes the account's tokens.
    fn set_disabled(&self, username: &str, disabled: bool) -> Result<AccountRecord>;
    /// Records a login that did not go through [`authenticate`](Self::authenticate) (the dev
    /// machine-hash login).
    fn record_login(&self, id: u32) -> Result<()>;
    /// Issues a random token valid for `ttl_secs`.
    fn issue_token(&self, account_id: u32, kind: TokenKind, ttl_secs: i64) -> Result<IssuedToken>;
    /// Validates a token of one of `kinds`: `TokenUnknown`, `TokenWrongCluster`,
    /// `TokenRevoked`, `TokenExpired` or `Disabled`. Records the login.
    fn authenticate_token(&self, token: &[u8; 32], kinds: &[TokenKind]) -> Result<AccountRecord>;
    /// Revokes one token; `false` if it is not known.
    fn revoke_token(&self, token: &[u8; 32]) -> Result<bool>;
    /// Revokes an account's tokens (of one kind, or all); returns how many were still valid.
    fn revoke_tokens(&self, account_id: u32, kind: Option<TokenKind>) -> Result<usize>;
}
