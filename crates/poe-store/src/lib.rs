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

pub trait Store: Send + Sync {
    fn account_by_login_key(&self, login_key: &str) -> Result<Option<Account>>;
    /// Creates an account named `<name_prefix><id>`.
    fn create_account(&self, login_key: &str, name_prefix: &str) -> Result<Account>;
}
