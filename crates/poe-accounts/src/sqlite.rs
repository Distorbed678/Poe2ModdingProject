//! [`AccountStore`] over the `poe-store` SQLite database (tables from migration 0006).

use std::sync::Arc;

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use poe_store::SqliteStore;
use rand::RngCore;
use rusqlite::{OptionalExtension, Row, params};

use crate::{
    AccountError, AccountRecord, AccountStore, IssuedToken, Result, Role, TokenKind, token_hash,
    validate_username,
};

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub struct SqliteAccounts {
    store: Arc<SqliteStore>,
    cluster_id: String,
    argon: Argon2<'static>,
    clock: Clock,
}

impl From<rusqlite::Error> for AccountError {
    fn from(e: rusqlite::Error) -> Self {
        match &e {
            rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => AccountError::NameTaken,
            _ => AccountError::Db(e.to_string()),
        }
    }
}

const COLUMNS: &str = "id, name, role, created_at, last_login, disabled, password_hash IS NOT NULL";

fn record(r: &Row<'_>) -> rusqlite::Result<AccountRecord> {
    let role: String = r.get(2)?;
    Ok(AccountRecord {
        id: r.get(0)?,
        username: r.get(1)?,
        role: Role::parse(&role).unwrap_or(Role::Player),
        created_at: r.get(3)?,
        last_login: r.get(4)?,
        disabled: r.get(5)?,
        has_password: r.get(6)?,
    })
}

impl SqliteAccounts {
    /// Argon2id with the crate defaults (19 MiB, 2 passes), the system clock.
    pub fn new(store: Arc<SqliteStore>, cluster_id: impl Into<String>) -> Self {
        Self {
            store,
            cluster_id: cluster_id.into(),
            argon: Argon2::default(),
            clock: Arc::new(|| {
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
            }),
        }
    }

    /// Different argon2 cost (tests use a tiny one). Existing hashes still verify: they carry
    /// their own parameters.
    pub fn with_params(mut self, params: argon2::Params) -> Self {
        self.argon = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        self
    }

    /// Replaces the clock (unix seconds), for tests of token expiry.
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn hash(&self, digest: &[u8; 32]) -> Result<String> {
        let mut raw = [0u8; 16];
        rand::rng().fill_bytes(&mut raw);
        let salt = SaltString::encode_b64(&raw).map_err(|e| AccountError::Db(format!("salt: {e}")))?;
        self.argon
            .hash_password(digest, &salt)
            .map(|h| h.to_string())
            .map_err(|e| AccountError::Db(format!("argon2: {e}")))
    }

    fn by_name(&self, name: &str) -> Result<AccountRecord> {
        self.find(name)?.ok_or(AccountError::NoAccount)
    }
}

impl AccountStore for SqliteAccounts {
    fn cluster_id(&self) -> &str {
        &self.cluster_id
    }

    fn create_account(&self, username: &str, password_digest: &[u8; 32], role: Role) -> Result<AccountRecord> {
        validate_username(username)?;
        let hash = self.hash(password_digest)?;
        let mut key = [0u8; 8];
        rand::rng().fill_bytes(&mut key);
        let login_key = format!("pw:{}", hex::encode(key));
        let now = self.now();
        let id = self.store.with_connection(|c| -> Result<i64> {
            c.execute(
                "INSERT INTO accounts (login_key, name, password_hash, role, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![login_key, username, hash, role.as_str(), now],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        self.find_by_id(id as u32)?.ok_or(AccountError::NoAccount)
    }

    fn find(&self, username: &str) -> Result<Option<AccountRecord>> {
        Ok(self.store.with_connection(|c| {
            c.query_row(
                &format!("SELECT {COLUMNS} FROM accounts WHERE name = ?1 COLLATE NOCASE"),
                params![username],
                record,
            )
            .optional()
        })?)
    }

    fn find_by_id(&self, id: u32) -> Result<Option<AccountRecord>> {
        Ok(self.store.with_connection(|c| {
            c.query_row(&format!("SELECT {COLUMNS} FROM accounts WHERE id = ?1"), params![id], record).optional()
        })?)
    }

    fn list(&self) -> Result<Vec<AccountRecord>> {
        Ok(self.store.with_connection(|c| -> rusqlite::Result<_> {
            let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM accounts ORDER BY id"))?;
            st.query_map([], record)?.collect()
        })?)
    }

    fn authenticate(&self, username: &str, password_digest: &[u8; 32]) -> Result<AccountRecord> {
        let stored: Option<(u32, Option<String>)> = self.store.with_connection(|c| {
            c.query_row(
                "SELECT id, password_hash FROM accounts WHERE name = ?1 COLLATE NOCASE",
                params![username],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })?;
        let (id, hash) = stored.ok_or(AccountError::NoAccount)?;
        let hash = hash.ok_or(AccountError::WrongPassword)?;
        let parsed = PasswordHash::new(&hash).map_err(|e| AccountError::Db(format!("stored hash: {e}")))?;
        if self.argon.verify_password(password_digest, &parsed).is_err() {
            return Err(AccountError::WrongPassword);
        }
        let account = self.find_by_id(id)?.ok_or(AccountError::NoAccount)?;
        if account.disabled {
            return Err(AccountError::Disabled);
        }
        self.record_login(id)?;
        Ok(AccountRecord { last_login: Some(self.now()), ..account })
    }

    fn set_password(&self, username: &str, password_digest: &[u8; 32]) -> Result<AccountRecord> {
        let account = self.by_name(username)?;
        let hash = self.hash(password_digest)?;
        self.store.with_connection(|c| {
            c.execute("UPDATE accounts SET password_hash = ?2 WHERE id = ?1", params![account.id, hash])
        })?;
        self.revoke_tokens(account.id, None)?;
        self.by_name(username)
    }

    fn set_role(&self, username: &str, role: Role) -> Result<AccountRecord> {
        let account = self.by_name(username)?;
        self.store
            .with_connection(|c| c.execute("UPDATE accounts SET role = ?2 WHERE id = ?1", params![account.id, role.as_str()]))?;
        self.by_name(username)
    }

    fn set_disabled(&self, username: &str, disabled: bool) -> Result<AccountRecord> {
        let account = self.by_name(username)?;
        self.store
            .with_connection(|c| c.execute("UPDATE accounts SET disabled = ?2 WHERE id = ?1", params![account.id, disabled]))?;
        if disabled {
            self.revoke_tokens(account.id, None)?;
        }
        self.by_name(username)
    }

    fn record_login(&self, id: u32) -> Result<()> {
        let now = self.now();
        self.store.with_connection(|c| c.execute("UPDATE accounts SET last_login = ?2 WHERE id = ?1", params![id, now]))?;
        Ok(())
    }

    fn issue_token(&self, account_id: u32, kind: TokenKind, ttl_secs: i64) -> Result<IssuedToken> {
        let mut token = [0u8; 32];
        while token == [0u8; 32] {
            rand::rng().fill_bytes(&mut token);
        }
        let now = self.now();
        let expires_at = now.saturating_add(ttl_secs);
        self.store.with_connection(|c| {
            c.execute(
                "INSERT INTO account_tokens (account_id, kind, token_hash, cluster_id, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![account_id, kind.as_str(), token_hash(&token).as_slice(), self.cluster_id, now, expires_at],
            )
        })?;
        Ok(IssuedToken { token, expires_at })
    }

    fn authenticate_token(&self, token: &[u8; 32], kinds: &[TokenKind]) -> Result<AccountRecord> {
        type Row = (i64, u32, String, String, i64, bool);
        let row: Option<Row> = self.store.with_connection(|c| {
            c.query_row(
                "SELECT id, account_id, kind, cluster_id, expires_at, revoked FROM account_tokens WHERE token_hash = ?1",
                params![token_hash(token).as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()
        })?;
        let (token_id, account_id, kind, cluster, expires_at, revoked) = row.ok_or(AccountError::TokenUnknown)?;
        if cluster != self.cluster_id {
            return Err(AccountError::TokenWrongCluster);
        }
        if revoked {
            return Err(AccountError::TokenRevoked);
        }
        let now = self.now();
        if expires_at <= now {
            return Err(AccountError::TokenExpired);
        }
        if !TokenKind::parse(&kind).is_some_and(|k| kinds.contains(&k)) {
            return Err(AccountError::TokenUnknown);
        }
        let account = self.find_by_id(account_id)?.ok_or(AccountError::NoAccount)?;
        if account.disabled {
            return Err(AccountError::Disabled);
        }
        self.store.with_connection(|c| c.execute("UPDATE account_tokens SET last_used = ?2 WHERE id = ?1", params![token_id, now]))?;
        self.record_login(account_id)?;
        Ok(AccountRecord { last_login: Some(now), ..account })
    }

    fn revoke_token(&self, token: &[u8; 32]) -> Result<bool> {
        let n = self.store.with_connection(|c| {
            c.execute("UPDATE account_tokens SET revoked = 1 WHERE token_hash = ?1", params![token_hash(token).as_slice()])
        })?;
        Ok(n > 0)
    }

    fn revoke_tokens(&self, account_id: u32, kind: Option<TokenKind>) -> Result<usize> {
        Ok(self.store.with_connection(|c| {
            c.execute(
                "UPDATE account_tokens SET revoked = 1
                 WHERE account_id = ?1 AND revoked = 0 AND (?2 IS NULL OR kind = ?2)",
                params![account_id, kind.map(TokenKind::as_str)],
            )
        })?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;
    use crate::password_digest;

    fn cheap() -> argon2::Params {
        argon2::Params::new(8, 1, 1, None).unwrap()
    }

    fn accounts(cluster: &str) -> (SqliteAccounts, Arc<AtomicI64>) {
        let time = Arc::new(AtomicI64::new(1_000_000));
        let t = time.clone();
        let a = SqliteAccounts::new(Arc::new(SqliteStore::in_memory().unwrap()), cluster)
            .with_params(cheap())
            .with_clock(move || t.load(Ordering::SeqCst));
        (a, time)
    }

    #[test]
    fn password_digest_is_sha256_of_utf16le() {
        // sha256 of the bytes 61 00 ("a" in UTF-16LE), computed independently (python).
        assert_eq!(hex::encode(password_digest("a")), "ffe9aaeaa2a2d5048174df0b80599ef0197ec024c4b051bc9860cff58ef7f9f3");
        assert_ne!(password_digest("a"), password_digest("A"));
    }

    #[test]
    fn creates_authenticates_and_stores_only_an_argon2id_hash() {
        let (a, _) = accounts("c1");
        let d = password_digest("hunter22");
        let rec = a.create_account("Alice", &d, Role::Player).unwrap();
        assert!(rec.has_password && !rec.disabled && rec.role == Role::Player);
        // Case-insensitive uniqueness and lookup.
        assert_eq!(a.create_account("ALICE", &d, Role::Player).unwrap_err(), AccountError::NameTaken);
        assert_eq!(a.find("aLiCe").unwrap().unwrap().id, rec.id);
        assert_eq!(a.authenticate("alice", &d).unwrap().id, rec.id);
        assert_eq!(a.authenticate("alice", &password_digest("nope")).unwrap_err(), AccountError::WrongPassword);
        assert_eq!(a.authenticate("bob", &d).unwrap_err(), AccountError::NoAccount);
        assert!(a.find("alice").unwrap().unwrap().last_login.is_some());
        let stored: String = a.store.with_connection(|c| {
            c.query_row("SELECT password_hash FROM accounts WHERE id = ?1", params![rec.id], |r| r.get(0)).unwrap()
        });
        assert!(stored.starts_with("$argon2id$"), "{stored}");
        assert!(!stored.contains(&hex::encode(d)));
        // Disabling: the right password is refused, enabling restores it.
        a.set_disabled("alice", true).unwrap();
        assert_eq!(a.authenticate("alice", &d).unwrap_err(), AccountError::Disabled);
        a.set_disabled("alice", false).unwrap();
        a.authenticate("alice", &d).unwrap();
        assert_eq!(a.set_role("alice", Role::Admin).unwrap().role, Role::Admin);
        assert_eq!(a.list().unwrap().len(), 1);
    }

    #[test]
    fn rejects_bad_usernames() {
        let (a, _) = accounts("c1");
        let d = password_digest("x");
        for bad in ["ab", "has space", "ünï", &"x".repeat(65)] {
            assert!(matches!(a.create_account(bad, &d, Role::Player), Err(AccountError::InvalidName(_))), "{bad}");
        }
        a.create_account("me@example.com", &d, Role::Player).unwrap();
    }

    #[test]
    fn machine_hash_accounts_have_no_password() {
        let (a, _) = accounts("c1");
        use poe_store::Store;
        let old = a.store.create_account("k", "Exile").unwrap();
        let rec = a.find_by_id(old.id).unwrap().unwrap();
        assert!(!rec.has_password && rec.role == Role::Player);
        let name = rec.username;
        assert_eq!(a.authenticate(&name, &password_digest("")).unwrap_err(), AccountError::WrongPassword);
    }

    #[test]
    fn tokens_expire_revoke_and_belong_to_a_cluster() {
        let (a, time) = accounts("c1");
        let rec = a.create_account("alice", &password_digest("hunter22"), Role::Player).unwrap();
        let kinds = [TokenKind::GameRemember];
        let t = a.issue_token(rec.id, TokenKind::GameRemember, 100).unwrap();
        assert_eq!(t.expires_at, 1_000_100);
        // Only the hash is stored.
        let raw_found: bool = a.store.with_connection(|c| {
            c.query_row("SELECT count(*) FROM account_tokens WHERE token_hash = ?1", params![t.token.as_slice()], |r| r.get::<_, i64>(0)).unwrap() > 0
        });
        assert!(!raw_found);
        assert_eq!(a.authenticate_token(&t.token, &kinds).unwrap().id, rec.id);
        // Wrong kind, unknown token.
        assert_eq!(a.authenticate_token(&t.token, &[TokenKind::Web]).unwrap_err(), AccountError::TokenUnknown);
        assert_eq!(a.authenticate_token(&[7; 32], &kinds).unwrap_err(), AccountError::TokenUnknown);
        // Expiry.
        time.store(1_000_099, Ordering::SeqCst);
        a.authenticate_token(&t.token, &kinds).unwrap();
        time.store(1_000_100, Ordering::SeqCst);
        assert_eq!(a.authenticate_token(&t.token, &kinds).unwrap_err(), AccountError::TokenExpired);
        // Revocation, single and by account.
        time.store(1_000_000, Ordering::SeqCst);
        assert!(a.revoke_token(&t.token).unwrap());
        assert_eq!(a.authenticate_token(&t.token, &kinds).unwrap_err(), AccountError::TokenRevoked);
        let t2 = a.issue_token(rec.id, TokenKind::GameRemember, 100).unwrap();
        let t3 = a.issue_token(rec.id, TokenKind::Launcher, 100).unwrap();
        assert_eq!(a.revoke_tokens(rec.id, Some(TokenKind::GameRemember)).unwrap(), 1);
        assert!(a.authenticate_token(&t2.token, &kinds).is_err());
        a.authenticate_token(&t3.token, &[TokenKind::Launcher]).unwrap();
        // A new password revokes everything; a disabled account's tokens do not work.
        a.set_password("alice", &password_digest("another-pw")).unwrap();
        assert!(a.authenticate_token(&t3.token, &[TokenKind::Launcher]).is_err());
        let t4 = a.issue_token(rec.id, TokenKind::GameRemember, 100).unwrap();
        a.set_disabled("alice", true).unwrap();
        assert!(a.authenticate_token(&t4.token, &kinds).is_err());
    }

    #[test]
    fn tokens_from_another_cluster_are_invalid() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let mk = |id: &str| SqliteAccounts::new(store.clone(), id).with_params(cheap());
        let (one, two) = (mk("cluster-one"), mk("cluster-two"));
        let rec = one.create_account("alice", &password_digest("hunter22"), Role::Player).unwrap();
        let t = one.issue_token(rec.id, TokenKind::GameRemember, 3600).unwrap();
        one.authenticate_token(&t.token, &[TokenKind::GameRemember]).unwrap();
        // Same database (e.g. restored from a backup into another cluster), other cluster id.
        assert_eq!(two.authenticate_token(&t.token, &[TokenKind::GameRemember]).unwrap_err(), AccountError::TokenWrongCluster);
    }
}
