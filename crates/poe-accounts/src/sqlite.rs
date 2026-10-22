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
    AccountError, AccountRecord, AccountStore, BanKind, BanRecord, IssuedToken, Result, Role, TokenKind,
    clean_reason, parse_ip, token_hash, validate_username,
};

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub struct SqliteAccounts {
    store: Arc<SqliteStore>,
    cluster_id: String,
    argon: Argon2<'static>,
    clock: Clock,
    /// A hash nobody knows the password of, verified when the account is unknown or has no
    /// password so that a wrong login takes as long for every name (login timing must not
    /// tell whether a name exists). Built on first use with the current argon2 cost.
    decoy: std::sync::OnceLock<String>,
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

const BAN_SELECT: &str = "SELECT b.id, b.kind, b.account_id, a.name, b.ip, b.reason, b.created_by, b.created_at, b.expires_at, b.lifted_at, b.lifted_by
     FROM bans b LEFT JOIN accounts a ON a.id = b.account_id";

fn ban_record(r: &Row<'_>) -> rusqlite::Result<BanRecord> {
    let kind: String = r.get(1)?;
    Ok(BanRecord {
        id: r.get(0)?,
        kind: BanKind::parse(&kind).unwrap_or(BanKind::Account),
        account_id: r.get(2)?,
        account: r.get(3)?,
        ip: r.get(4)?,
        reason: r.get(5)?,
        created_by: r.get(6)?,
        created_at: r.get(7)?,
        expires_at: r.get(8)?,
        lifted_at: r.get(9)?,
        lifted_by: r.get(10)?,
    })
}

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
            decoy: std::sync::OnceLock::new(),
        }
    }

    /// Different argon2 cost (tests use a tiny one). Existing hashes still verify: they carry
    /// their own parameters.
    pub fn with_params(mut self, params: argon2::Params) -> Self {
        self.argon = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        self.decoy = std::sync::OnceLock::new();
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

    /// Spends the time of a password check without a result that could matter: the digest never
    /// matches the decoy hash (random, never stored).
    fn spend_verify_time(&self, digest: &[u8; 32]) {
        let decoy = self.decoy.get_or_init(|| {
            let mut secret = [0u8; 32];
            rand::rng().fill_bytes(&mut secret);
            self.hash(&secret).unwrap_or_default()
        });
        if let Ok(parsed) = PasswordHash::new(decoy) {
            let _ = self.argon.verify_password(digest, &parsed);
        }
    }

    /// Refuses an account that is banned now.
    fn check_not_banned(&self, account_id: u32) -> Result<()> {
        match self.active_account_ban(account_id)? {
            Some(b) => Err(AccountError::Banned { reason: b.reason, until: b.expires_at }),
            None => Ok(()),
        }
    }

    fn insert_ban(&self, kind: BanKind, account_id: Option<u32>, ip: Option<&str>, reason: &str, by: &str, expires_at: Option<i64>) -> Result<BanRecord> {
        let reason = clean_reason(reason).ok_or(AccountError::InvalidBan("the reason is too long"))?;
        let now = self.now();
        if expires_at.is_some_and(|e| e <= now) {
            return Err(AccountError::InvalidBan("the expiry must be in the future"));
        }
        let id = self.store.with_connection(|c| -> rusqlite::Result<i64> {
            c.execute(
                "INSERT INTO bans (kind, account_id, ip, reason, created_by, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![kind.as_str(), account_id, ip, reason, by, now, expires_at],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        self.ban(id as u32)
    }

    fn ban(&self, id: u32) -> Result<BanRecord> {
        let found = self.store.with_connection(|c| {
            c.query_row(&format!("{BAN_SELECT} WHERE b.id = ?1"), params![id], ban_record).optional()
        })?;
        found.ok_or(AccountError::NoBan)
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
        let Some((id, hash)) = stored else {
            self.spend_verify_time(password_digest);
            return Err(AccountError::NoAccount);
        };
        let Some(hash) = hash else {
            self.spend_verify_time(password_digest);
            return Err(AccountError::WrongPassword);
        };
        let parsed = PasswordHash::new(&hash).map_err(|e| AccountError::Db(format!("stored hash: {e}")))?;
        if self.argon.verify_password(password_digest, &parsed).is_err() {
            return Err(AccountError::WrongPassword);
        }
        let account = self.find_by_id(id)?.ok_or(AccountError::NoAccount)?;
        if account.disabled {
            return Err(AccountError::Disabled);
        }
        // After the password: a wrong password never learns whether the account is banned.
        self.check_not_banned(id)?;
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
        self.check_not_banned(account_id)?;
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
    fn ban_account(&self, account_id: u32, reason: &str, by: &str, expires_at: Option<i64>) -> Result<BanRecord> {
        self.find_by_id(account_id)?.ok_or(AccountError::NoAccount)?;
        if self.active_account_ban(account_id)?.is_some() {
            return Err(AccountError::AlreadyBanned);
        }
        let ban = self.insert_ban(BanKind::Account, Some(account_id), None, reason, by, expires_at)?;
        // Saved logins (game Remember Me, launcher, web) stop working at once.
        self.revoke_tokens(account_id, None)?;
        Ok(ban)
    }

    fn ban_ip(&self, ip: &str, reason: &str, by: &str, expires_at: Option<i64>) -> Result<BanRecord> {
        let ip = parse_ip(ip).ok_or(AccountError::InvalidBan("not an IP address"))?;
        if self.active_ip_ban(&ip)?.is_some() {
            return Err(AccountError::AlreadyBanned);
        }
        self.insert_ban(BanKind::Ip, None, Some(&ip), reason, by, expires_at)
    }

    fn lift_ban(&self, id: u32, by: &str) -> Result<BanRecord> {
        let ban = self.ban(id)?;
        let now = self.now();
        if !ban.is_active(now) {
            return Err(AccountError::BanNotActive);
        }
        self.store.with_connection(|c| {
            c.execute("UPDATE bans SET lifted_at = ?2, lifted_by = ?3 WHERE id = ?1", params![id, now, by])
        })?;
        self.ban(id)
    }

    fn active_account_ban(&self, account_id: u32) -> Result<Option<BanRecord>> {
        let now = self.now();
        Ok(self.store.with_connection(|c| {
            c.query_row(
                &format!("{BAN_SELECT} WHERE b.account_id = ?1 AND b.lifted_at IS NULL AND (b.expires_at IS NULL OR b.expires_at > ?2) ORDER BY b.id DESC LIMIT 1"),
                params![account_id, now],
                ban_record,
            )
            .optional()
        })?)
    }

    fn active_ip_ban(&self, ip: &str) -> Result<Option<BanRecord>> {
        let now = self.now();
        Ok(self.store.with_connection(|c| {
            c.query_row(
                &format!("{BAN_SELECT} WHERE b.ip = ?1 AND b.lifted_at IS NULL AND (b.expires_at IS NULL OR b.expires_at > ?2) ORDER BY b.id DESC LIMIT 1"),
                params![ip, now],
                ban_record,
            )
            .optional()
        })?)
    }

    fn bans(&self, active_only: bool) -> Result<Vec<BanRecord>> {
        let now = self.now();
        Ok(self.store.with_connection(|c| -> rusqlite::Result<_> {
            let filter = if active_only { "WHERE b.lifted_at IS NULL AND (b.expires_at IS NULL OR b.expires_at > ?1)" } else { "WHERE ?1 = ?1" };
            let mut st = c.prepare(&format!("{BAN_SELECT} {filter} ORDER BY b.id DESC"))?;
            st.query_map(params![now], ban_record)?.collect()
        })?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;
    use crate::{BanKind, password_digest};

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
        assert!(a.decoy.get().is_none());
        assert_eq!(a.authenticate("bob", &d).unwrap_err(), AccountError::NoAccount);
        // The unknown name spent a verification on the decoy hash (no timing difference).
        assert!(a.decoy.get().is_some_and(|h| h.starts_with("$argon2id$")));
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

    #[test]
    fn a_banned_account_is_refused_until_the_ban_ends() {
        let (a, time) = accounts("c1");
        let d = password_digest("hunter22");
        let alice = a.create_account("alice", &d, Role::Player).unwrap();
        let token = a.issue_token(alice.id, TokenKind::GameRemember, 10_000).unwrap();
        // Bad inputs.
        assert_eq!(a.ban_account(99, "x", "root", None).unwrap_err(), AccountError::NoAccount);
        assert!(matches!(a.ban_account(alice.id, "x", "root", Some(1_000_000)), Err(AccountError::InvalidBan(_))), "expiry now is not the future");
        assert!(matches!(a.ban_account(alice.id, &"x".repeat(201), "root", None), Err(AccountError::InvalidBan(_))));
        assert!(a.active_account_ban(alice.id).unwrap().is_none());

        let ban = a.ban_account(alice.id, "  cheating \n here ", "root", Some(1_003_600)).unwrap();
        assert_eq!((ban.kind, ban.reason.as_str(), ban.account.as_deref(), ban.created_by.as_str()), (BanKind::Account, "cheating here", Some("alice"), "root"));
        assert_eq!(a.ban_account(alice.id, "again", "root", None).unwrap_err(), AccountError::AlreadyBanned);
        // Password and token logins say so, the right password only.
        let banned = a.authenticate("alice", &d).unwrap_err();
        assert_eq!(banned, AccountError::Banned { reason: "cheating here".into(), until: Some(1_003_600) });
        assert_eq!(banned.to_string(), "account is banned until 1970-01-12 14:46 UTC. Reason: cheating here.");
        assert_eq!(a.authenticate("alice", &password_digest("wrong-pw")).unwrap_err(), AccountError::WrongPassword);
        assert_eq!(a.authenticate_token(&token.token, &[TokenKind::GameRemember]).unwrap_err(), AccountError::TokenRevoked, "tokens are revoked by the ban");
        assert_eq!(a.bans(true).unwrap().len(), 1);

        // It ends by itself.
        time.store(1_003_600, Ordering::SeqCst);
        assert!(a.active_account_ban(alice.id).unwrap().is_none());
        a.authenticate("alice", &d).unwrap();
        let all = a.bans(false).unwrap();
        assert_eq!((all.len(), all[0].status(1_003_600), a.bans(true).unwrap().len()), (1, "expired", 0));
        assert_eq!(a.lift_ban(ban.id, "root").unwrap_err(), AccountError::BanNotActive);

        // A permanent one is lifted by hand; the history keeps both.
        let second = a.ban_account(alice.id, "", "root", None).unwrap();
        assert!(matches!(a.authenticate("alice", &d), Err(AccountError::Banned { until: None, .. })));
        let lifted = a.lift_ban(second.id, "mod").unwrap();
        assert_eq!((lifted.lifted_by.as_deref(), lifted.status(1_003_600)), (Some("mod"), "lifted"));
        a.authenticate("alice", &d).unwrap();
        assert_eq!(a.lift_ban(second.id, "mod").unwrap_err(), AccountError::BanNotActive);
        assert_eq!(a.lift_ban(77, "mod").unwrap_err(), AccountError::NoBan);
        assert_eq!(a.bans(false).unwrap().iter().map(|b| b.id).collect::<Vec<_>>(), vec![second.id, ban.id], "newest first");
    }

    #[test]
    fn ip_bans_match_the_canonical_address() {
        let (a, time) = accounts("c1");
        assert!(matches!(a.ban_ip("nonsense", "", "root", None), Err(AccountError::InvalidBan(_))));
        let ban = a.ban_ip("::ffff:10.1.2.3", "spam", "root", Some(1_000_100)).unwrap();
        assert_eq!((ban.kind, ban.ip.as_deref(), ban.account_id), (BanKind::Ip, Some("10.1.2.3"), None));
        assert!(a.active_ip_ban("10.1.2.3").unwrap().is_some());
        assert!(a.active_ip_ban("10.1.2.4").unwrap().is_none());
        assert_eq!(a.ban_ip("10.1.2.3", "", "root", None).unwrap_err(), AccountError::AlreadyBanned);
        time.store(1_000_100, Ordering::SeqCst);
        assert!(a.active_ip_ban("10.1.2.3").unwrap().is_none(), "expired");
        // Banned again after expiry; lifted by hand.
        let again = a.ban_ip("10.1.2.3", "", "root", None).unwrap();
        a.lift_ban(again.id, "root").unwrap();
        assert!(a.active_ip_ban("10.1.2.3").unwrap().is_none());
    }
}
