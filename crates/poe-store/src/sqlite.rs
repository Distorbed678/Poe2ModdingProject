use std::{path::Path, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{Account, Result, Store, StoreError};

/// Ordered schema migrations; `user_version` records how many were applied.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_accounts.sql")];

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|source| StoreError::Io {
                path: dir.display().to_string(),
                source,
            })?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let applied: usize = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i + 1)?;
            tx.commit()?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

fn account_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        login_key: r.get(1)?,
        name: r.get(2)?,
    })
}

impl Store for SqliteStore {
    fn account_by_login_key(&self, login_key: &str) -> Result<Option<Account>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT id, login_key, name FROM accounts WHERE login_key = ?1",
                params![login_key],
                account_from_row,
            )
            .optional()?)
    }

    fn create_account(&self, login_key: &str, name_prefix: &str) -> Result<Account> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        // Insert with a placeholder name, then name it after the assigned id.
        tx.execute(
            "INSERT INTO accounts (login_key, name) VALUES (?1, ?1)",
            params![login_key],
        )?;
        let id = tx.last_insert_rowid();
        let name = format!("{name_prefix}{id}");
        tx.execute(
            "UPDATE accounts SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        tx.commit()?;
        Ok(Account {
            id: id as u32,
            login_key: login_key.to_owned(),
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_finds_accounts() {
        let store = SqliteStore::in_memory().unwrap();
        assert!(store.account_by_login_key("abc").unwrap().is_none());
        let a = store.create_account("abc", "Exile").unwrap();
        assert_eq!(a.name, format!("Exile{}", a.id));
        assert_eq!(store.account_by_login_key("abc").unwrap(), Some(a));
    }
}
