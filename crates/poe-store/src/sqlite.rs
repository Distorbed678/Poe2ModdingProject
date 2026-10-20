use std::{path::Path, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{Account, Character, Location, NewCharacter, Result, Store, StoreError};

/// Ordered schema migrations; `user_version` records how many were applied.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_accounts.sql"),
    include_str!("../migrations/0002_characters.sql"),
    include_str!("../migrations/0003_character_area.sql"),
    include_str!("../migrations/0004_character_intro.sql"),
    include_str!("../migrations/0005_character_location.sql"),
    include_str!("../migrations/0006_accounts_auth.sql"),
];

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

    /// Runs `f` with the connection. For sibling crates that own tables in this database
    /// (`poe-accounts`); the schema itself is migrated here.
    pub fn with_connection<R>(&self, f: impl FnOnce(&mut Connection) -> R) -> R {
        f(&mut self.conn.lock().unwrap())
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

fn character_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Character> {
    Ok(Character {
        id: r.get(0)?,
        account_id: r.get(1)?,
        name: r.get(2)?,
        league: r.get(3)?,
        class_id: r.get(4)?,
        start_state: r.get(5)?,
        level: r.get(6)?,
        experience: r.get(7)?,
        area: r.get(8)?,
        intro_done: r.get(9)?,
        location: match (r.get(10)?, r.get(11)?, r.get(12)?) {
            (Some(seed), Some(x), Some(y)) => Some(Location { seed, x, y }),
            _ => None,
        },
    })
}

const CHARACTER_COLUMNS: &str = "id, account_id, name, league, class_id, start_state, level, \
     experience, area, intro_done, seed, pos_x, pos_y";

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

    fn account_by_id(&self, id: u32) -> Result<Option<Account>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT id, login_key, name FROM accounts WHERE id = ?1",
                params![id],
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

    fn characters(&self, account_id: u32) -> Result<Vec<Character>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CHARACTER_COLUMNS} FROM characters WHERE account_id = ?1 ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![account_id], character_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn set_intro_done(&self, character_id: u32) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE characters SET intro_done = 1 WHERE id = ?1",
            params![character_id],
        )?;
        Ok(())
    }

    fn save_location(
        &self,
        character_id: u32,
        area: &str,
        location: Option<Location>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE characters SET area = ?2, seed = ?3, pos_x = ?4, pos_y = ?5 WHERE id = ?1",
            params![
                character_id,
                area,
                location.map(|l| l.seed),
                location.map(|l| l.x),
                location.map(|l| l.y)
            ],
        )?;
        Ok(())
    }

    fn create_character(&self, new: &NewCharacter) -> Result<Option<Character>> {
        let conn = self.conn.lock().unwrap();
        let inserted = conn.execute(
            "INSERT INTO characters (account_id, name, league, class_id, start_state, level, area)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(name) DO NOTHING",
            params![
                new.account_id,
                new.name,
                new.league,
                new.class_id,
                new.start_state,
                new.level,
                new.area
            ],
        )?;
        if inserted == 0 {
            return Ok(None);
        }
        let id = conn.last_insert_rowid();
        Ok(Some(conn.query_row(
            &format!("SELECT {CHARACTER_COLUMNS} FROM characters WHERE id = ?1"),
            params![id],
            character_from_row,
        )?))
    }

    fn delete_character(&self, account_id: u32, name: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM characters WHERE account_id = ?1 AND name = ?2",
            params![account_id, name],
        )?;
        Ok(deleted > 0)
    }
}

#[cfg(test)]
mod character_tests {
    use super::*;

    #[test]
    fn creates_and_lists_characters() {
        let store = SqliteStore::in_memory().unwrap();
        let a = store.create_account("k", "Exile").unwrap();
        let new = NewCharacter {
            account_id: a.id,
            name: "Tester".into(),
            league: "Standard".into(),
            class_id: 2,
            start_state: "DexFourDefault".into(),
            level: 1,
            area: "G1_1".into(),
        };
        let c = store.create_character(&new).unwrap().expect("created");
        assert_eq!((c.level, c.area.as_str()), (1, "G1_1"));
        // Names are unique, case-insensitively.
        let dup = NewCharacter {
            name: "tester".into(),
            ..new
        };
        assert!(store.create_character(&dup).unwrap().is_none());
        assert_eq!(store.characters(a.id).unwrap(), vec![c.clone()]);
        // Locations are saved with the area.
        assert_eq!(c.location, None);
        let at = Location {
            seed: 3357350429,
            x: 285,
            y: 1140,
        };
        store.save_location(c.id, "G1_1", Some(at)).unwrap();
        let saved = &store.characters(a.id).unwrap()[0];
        assert_eq!((saved.area.as_str(), saved.location), ("G1_1", Some(at)));
        // Deleting only touches the owner's character.
        assert!(!store.delete_character(a.id + 1, "Tester").unwrap());
        assert!(store.delete_character(a.id, "Tester").unwrap());
        assert!(store.characters(a.id).unwrap().is_empty());
        assert!(!store.delete_character(a.id, "Tester").unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0006 on a database created by 0001-0005: machine-hash accounts survive as dev accounts
    /// (null password, player role, `Exile1` becomes admin as `[admin] accounts` had it).
    #[test]
    fn migration_0006_keeps_existing_accounts() {
        let conn = Connection::open_in_memory().unwrap();
        for (i, sql) in MIGRATIONS.iter().take(5).enumerate() {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", i + 1).unwrap();
        }
        conn.execute("INSERT INTO accounts (login_key, name) VALUES ('aa', 'Exile1'), ('bb', 'Exile2')", []).unwrap();
        let store = SqliteStore::from_connection(conn).unwrap();
        let rows: Vec<(String, Option<String>, String, bool)> = store.with_connection(|c| {
            let mut st = c.prepare("SELECT name, password_hash, role, disabled FROM accounts ORDER BY id").unwrap();
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().map(|r| r.unwrap()).collect()
        });
        assert_eq!(rows[0], ("Exile1".into(), None, "admin".into(), false));
        assert_eq!(rows[1], ("Exile2".into(), None, "player".into(), false));
        assert_eq!(store.account_by_login_key("bb").unwrap().unwrap().name, "Exile2");
    }

    #[test]
    fn creates_and_finds_accounts() {
        let store = SqliteStore::in_memory().unwrap();
        assert!(store.account_by_login_key("abc").unwrap().is_none());
        let a = store.create_account("abc", "Exile").unwrap();
        assert_eq!(a.name, format!("Exile{}", a.id));
        assert_eq!(store.account_by_login_key("abc").unwrap(), Some(a));
    }
}
