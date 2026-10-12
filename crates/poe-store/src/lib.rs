//! Persistence behind the [`Store`] trait, so the backend can be swapped (SQLite now).

mod sqlite;

pub use sqlite::SqliteStore;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("creating {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: u32,
    pub login_key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    pub id: u32,
    pub account_id: u32,
    pub name: String,
    pub league: String,
    /// `Characters.IntegerId` of the class.
    pub class_id: u32,
    /// `CharacterStartStates.Id` chosen at creation.
    pub start_state: String,
    pub level: u32,
    pub experience: u64,
}

/// Fields supplied when creating a character.
#[derive(Debug, Clone)]
pub struct NewCharacter {
    pub account_id: u32,
    pub name: String,
    pub league: String,
    pub class_id: u32,
    pub start_state: String,
}

pub trait Store: Send + Sync {
    fn account_by_login_key(&self, login_key: &str) -> Result<Option<Account>>;
    /// Creates an account named `<name_prefix><id>`.
    fn create_account(&self, login_key: &str, name_prefix: &str) -> Result<Account>;
    /// Characters of an account, oldest first.
    fn characters(&self, account_id: u32) -> Result<Vec<Character>>;
    /// Creates a character; `Ok(None)` if the name is already taken.
    fn create_character(&self, new: &NewCharacter) -> Result<Option<Character>>;
}
