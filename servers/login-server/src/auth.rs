//! Authentication decisions for the standalone login (`0x0003`), apart from the connection so
//! they can be tested with encoded packets. Account model: `docs/architecture/accounts.md`.

use std::net::IpAddr;

use poe_accounts::{AccountError, AccountRecord, AccountStore, LoginLimiter, TokenKind};
use poe_config::AccountsConfig;
use poe_proto::login::{AuthenticationResult, Credentials};

/// `AuthenticationResult.result` codes: rows of the client's `BackendErrors` table, named once
/// in `poe_proto::backend_error` (code = row + 1; `crate::codes::verify` checks them against
/// the real table at start). The client prints the table text unless `notice` is non-empty;
/// we send our own wording only where the table's is wrong for us.
pub mod code {
    use poe_proto::backend_error as be;

    /// "The specified account does not exist."
    pub const NO_ACCOUNT: u16 = be::ACCOUNT_DOES_NOT_EXIST.code;
    /// "The password is not correct for this account."
    pub const WRONG_PASSWORD: u16 = be::INVALID_PASSWORD.code;
    /// "Access to the game is not currently enabled": logins of this kind are switched off.
    pub const NOT_ENABLED: u16 = be::NOT_ALLOWED_GAME_LOGIN.code;
    /// Stale, revoked, expired or unknown saved login (and one of another cluster): the client
    /// deletes its saved credentials file and shows our notice, so the next start asks for the
    /// password. (`0x62` would open the email Unlock window, `0x61` keeps the file.)
    pub const TOKEN_INVALID: u16 = be::UNEXPECTED_DISCONNECT.code;
    /// Database error.
    pub const DATABASE: u16 = be::DATABASE_ERROR.code;
    /// Banned: used for disabled accounts.
    pub const DISABLED: u16 = be::ACCOUNT_BANNED.code;
    /// "You have attempted to log in too often within a short time period."
    pub const RATE_LIMITED: u16 = be::LOGIN_RATE_LIMITED.code;
}

/// Notice for a saved login that cannot be used (shown instead of the table text of
/// `UnexpectedDisconnect`; the wording of `SavedCredentialsBad`).
const TOKEN_STALE_NOTICE: &str = "Your saved credentials are out of date. Please re-enter your password.";

/// How a login ended.
#[derive(Debug)]
pub enum Verdict {
    Accepted {
        account: AccountRecord,
        /// The Remember Me token for `session_token` (a fresh one on every accepted login that
        /// asks to save, a rotated one on token logins).
        token: Option<[u8; 32]>,
    },
    Refused {
        result: u16,
        /// Empty: the client prints the table text of `result`.
        notice: String,
    },
}

fn refused(result: u16, notice: &str) -> Verdict {
    Verdict::Refused { result, notice: notice.to_owned() }
}

fn map_error(e: AccountError, token_login: bool) -> Verdict {
    match e {
        AccountError::NoAccount => refused(code::NO_ACCOUNT, "The specified account does not exist."),
        AccountError::WrongPassword => refused(code::WRONG_PASSWORD, "The password is not correct for this account."),
        AccountError::Disabled => refused(code::DISABLED, "This account has been disabled."),
        AccountError::TokenExpired | AccountError::TokenRevoked | AccountError::TokenUnknown => {
            refused(code::TOKEN_INVALID, TOKEN_STALE_NOTICE)
        }
        AccountError::TokenWrongCluster => {
            refused(code::TOKEN_INVALID, "Your saved login is not valid on this server. Please enter your password.")
        }
        other => {
            tracing::warn!(token_login, "account store: {other}");
            refused(code::DATABASE, "The account database is not available.")
        }
    }
}

/// The refusal for a locked account name or IP.
pub fn rate_limited(retry_after_secs: u64) -> Verdict {
    let wait = if retry_after_secs >= 120 {
        format!("{} minutes", retry_after_secs.div_ceil(60))
    } else {
        format!("{retry_after_secs} seconds")
    };
    refused(
        code::RATE_LIMITED,
        &format!("You have attempted to log in too often within a short time period. Try again in {wait}."),
    )
}

/// Decides a `0x0003` login. Blocking (argon2id): call from `spawn_blocking`. `ip` is the
/// connection's peer address, for the password attempt limits (`[security]`); saved-token
/// logins are not limited.
pub fn credentials_login(
    accounts: &dyn AccountStore,
    cfg: &AccountsConfig,
    limiter: &LoginLimiter,
    ip: IpAddr,
    creds: &Credentials,
) -> Verdict {
    let token_login = creds.token_login != 0;
    let account = if token_login {
        match accounts.authenticate_token(&creds.password_hash, &[TokenKind::GameRemember, TokenKind::Launcher]) {
            // A token belongs to one account: the saved user name must match it.
            Ok(a) if !a.username.eq_ignore_ascii_case(&creds.account_name) => {
                return map_error(AccountError::TokenRevoked, true);
            }
            Ok(a) => {
                // Rolling tokens: the client saves what the answer carries, so each token
                // login hands out a fresh one and retires the one it used.
                let _ = accounts.revoke_token(&creds.password_hash);
                a
            }
            Err(e) => return map_error(e, true),
        }
    } else {
        // Locked names and IPs are refused before the (slow) password check.
        if let Err(locked) = limiter.check(&creds.account_name, ip) {
            tracing::warn!(account = %creds.account_name, %ip, "login refused: too many failed attempts");
            return rate_limited(locked.retry_after_secs);
        }
        match accounts.authenticate(&creds.account_name, &creds.password_hash) {
            Ok(a) => {
                limiter.success(&creds.account_name);
                a
            }
            Err(e) => {
                // An unknown name counts like a wrong password (no probing through the limit).
                if matches!(e, AccountError::NoAccount | AccountError::WrongPassword) {
                    limiter.failure(&creds.account_name, ip);
                }
                return map_error(e, false);
            }
        }
    };
    let token = (creds.save_login != 0 || token_login)
        .then(|| accounts.issue_token(account.id, TokenKind::GameRemember, i64::from(cfg.remember_days) * 86_400))
        .and_then(|r| r.map_err(|e| tracing::warn!("issuing a login token: {e}")).ok())
        .map(|t| t.token);
    Verdict::Accepted { account, token }
}

/// The wire reply for a verdict.
pub fn reply(cfg: &AccountsConfig, verdict: &Verdict) -> AuthenticationResult {
    match verdict {
        Verdict::Accepted { account, token } => AuthenticationResult {
            account_id: account.id,
            result: 0,
            notice: String::new(),
            // All zero = nothing to remember (the client does not save a zero token).
            session_token: token.unwrap_or([0; 32]),
            account_flags: cfg.flags,
            account_name: account.username.clone(),
        },
        Verdict::Refused { result, notice } => AuthenticationResult {
            account_id: 0,
            result: *result,
            notice: notice.clone(),
            session_token: [0; 32],
            account_flags: [0; 2],
            account_name: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    };

    use poe_accounts::{LoginLimiter, Role, SqliteAccounts, password_digest};
    use poe_net::Reader;
    use poe_proto::{Message, Protocol};
    use poe_store::SqliteStore;

    use super::*;
    use crate::session::{ClientMessage, decode_client};

    fn protocol() -> Protocol {
        Protocol::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/protocol/opcodes/0.5.5e")).unwrap()
    }

    fn cfg() -> AccountsConfig {
        let c = poe_config::Config::parse(include_str!("../../../config/server.example.toml")).unwrap();
        c.accounts
    }

    struct World {
        protocol: Protocol,
        accounts: SqliteAccounts,
        cfg: AccountsConfig,
        limiter: LoginLimiter,
        ip: IpAddr,
        time: Arc<AtomicI64>,
    }

    impl World {
        fn new(cluster: &str, store: Arc<SqliteStore>) -> Self {
            let time = Arc::new(AtomicI64::new(1_000_000));
            let (t, t2) = (time.clone(), time.clone());
            let limiter = LoginLimiter::new(3, 5, 300, 600).with_clock(move || t2.load(Ordering::SeqCst));
            let accounts = SqliteAccounts::new(store, cluster)
                .with_params(poe_accounts::Params::new(8, 1, 1, None).unwrap())
                .with_clock(move || t.load(Ordering::SeqCst));
            Self { protocol: protocol(), accounts, cfg: cfg(), limiter, ip: "10.0.0.1".parse().unwrap(), time }
        }

        /// A packet as the client builds it, through the wire: encode, decode like the
        /// connection does, decide, encode the reply, decode it like the client.
        fn login(&self, creds: &Credentials) -> (AuthenticationResult, Verdict) {
            let bytes = self.protocol.login.encode(creds).unwrap();
            assert_eq!(&bytes[..2], &[0, 3], "0x0003 from login.toml");
            let ClientMessage::Credentials(decoded) = decode_client(&self.protocol, &mut Reader::new(&bytes)).unwrap() else {
                panic!("not decoded as Credentials");
            };
            assert_eq!(&decoded, creds);
            let verdict = credentials_login(&self.accounts, &self.cfg, &self.limiter, self.ip, &decoded);
            let wire = self.protocol.login.encode(&reply(&self.cfg, &verdict)).unwrap();
            assert_eq!(&wire[..2], &[0, 4]);
            (AuthenticationResult::decode(&mut Reader::new(&wire[2..])).unwrap(), verdict)
        }
    }

    fn password_creds(name: &str, password: &str, save: bool) -> Credentials {
        Credentials {
            mode: 2,
            account_name: name.into(),
            client_hash: [1; 32],
            password_hash: password_digest(password),
            machine_hash: [3; 32],
            save_login: save as u8,
            token_login: 0,
            flags_lo: 0x40,
            flags_hi: 0,
        }
    }

    fn token_creds(name: &str, token: [u8; 32]) -> Credentials {
        Credentials { password_hash: token, save_login: 1, token_login: 1, ..password_creds(name, "", true) }
    }

    #[test]
    fn password_logins() {
        let w = World::new("c", Arc::new(SqliteStore::in_memory().unwrap()));
        let rec = w.accounts.create_account("Alice", &password_digest("hunter22"), Role::Player).unwrap();

        let (r, _) = w.login(&password_creds("alice", "hunter22", false));
        assert_eq!((r.result, r.account_id, r.account_name.as_str()), (0, rec.id, "Alice"));
        assert_eq!(r.session_token, [0; 32], "no Remember Me: nothing to save");
        assert_eq!(r.account_flags, w.cfg.flags);
        // The default flags: no strict-unlock banner (byte 0 bit 6) and none of the windows
        // (bits 2, 3, 4, 7); docs/protocol/login.md.
        assert_eq!(r.account_flags, [0x23, 0x03]);
        assert_eq!(r.account_flags[0] & (0x40 | 0x04 | 0x08 | 0x10 | 0x80), 0);

        let (r, _) = w.login(&password_creds("alice", "wrong-pass", false));
        assert_eq!(r.result, code::WRONG_PASSWORD);
        assert!(!r.notice.is_empty());
        let (r, _) = w.login(&password_creds("nobody", "hunter22", false));
        assert_eq!(r.result, code::NO_ACCOUNT);

        w.accounts.set_disabled("alice", true).unwrap();
        let (r, _) = w.login(&password_creds("alice", "hunter22", false));
        assert_eq!(r.result, code::DISABLED);
        assert!(r.notice.contains("disabled"));
    }

    #[test]
    fn remember_me_tokens_log_in_rotate_and_go_stale() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let w = World::new("c", store.clone());
        w.accounts.create_account("Alice", &password_digest("hunter22"), Role::Player).unwrap();

        let (r, _) = w.login(&password_creds("Alice", "hunter22", true));
        assert_eq!(r.result, 0);
        let first = r.session_token;
        assert_ne!(first, [0; 32]);

        // Token login: accepted, and the answer carries a new token (the client saves it).
        let (r, _) = w.login(&token_creds("Alice", first));
        assert_eq!((r.result, r.account_name.as_str()), (0, "Alice"));
        let second = r.session_token;
        assert!(second != [0; 32] && second != first);
        // The retired token is revoked: 0x0f ("UnexpectedDisconnect": the client deletes its
        // file) with our notice, never 0x62 (the email Unlock window).
        let (r, _) = w.login(&token_creds("Alice", first));
        assert_eq!(r.result, 0x0f);
        assert_eq!(r.result, code::TOKEN_INVALID);
        assert!(r.notice.contains("out of date"), "{}", r.notice);
        assert_eq!((r.account_id, r.session_token, r.account_flags), (0, [0; 32], [0; 2]));
        // Unknown token: the same.
        let (r, _) = w.login(&token_creds("Alice", [9; 32]));
        assert_eq!(r.result, 0x0f);
        assert!(r.notice.contains("out of date"));
        // A token only works for its own account name.
        w.accounts.create_account("Bob", &password_digest("hunter22"), Role::Player).unwrap();
        let (r, _) = w.login(&token_creds("Bob", second));
        assert_eq!(r.result, code::TOKEN_INVALID);

        // Expiry (remember_days).
        let (r, _) = w.login(&token_creds("Alice", second));
        assert_eq!(r.result, 0);
        let third = r.session_token;
        w.time.fetch_add(i64::from(w.cfg.remember_days) * 86_400, Ordering::SeqCst);
        let (r, _) = w.login(&token_creds("Alice", third));
        assert_eq!(r.result, code::TOKEN_INVALID);

        // A password change revokes saved logins.
        let (r, _) = w.login(&password_creds("Alice", "hunter22", true));
        let fresh = r.session_token;
        w.accounts.set_password("Alice", &password_digest("a-new-password")).unwrap();
        let (r, _) = w.login(&token_creds("Alice", fresh));
        assert_eq!(r.result, code::TOKEN_INVALID);
    }

    #[test]
    fn tokens_of_another_cluster_do_not_log_in() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let one = World::new("cluster-one", store.clone());
        one.accounts.create_account("Alice", &password_digest("hunter22"), Role::Player).unwrap();
        let (r, _) = one.login(&password_creds("Alice", "hunter22", true));
        assert_eq!(r.result, 0);
        let token = r.session_token;
        let two = World::new("cluster-two", store);
        let (r, _) = two.login(&token_creds("Alice", token));
        assert_eq!(r.result, code::TOKEN_INVALID);
        // The password still works there: the account itself is in the shared database.
        assert_eq!(two.login(&password_creds("Alice", "hunter22", false)).0.result, 0);
    }

    #[test]
    fn password_attempts_are_rate_limited() {
        let w = World::new("c", Arc::new(SqliteStore::in_memory().unwrap()));
        w.accounts.create_account("Alice", &password_digest("hunter22"), Role::Player).unwrap();
        for _ in 0..3 {
            assert_eq!(w.login(&password_creds("Alice", "wrong-pass", false)).0.result, code::WRONG_PASSWORD);
        }
        // Locked: even the right password is refused, with 73 and a wait in the notice.
        let (r, v) = w.login(&password_creds("alice", "hunter22", false));
        assert_eq!(r.result, 0x49);
        assert_eq!(r.result, code::RATE_LIMITED);
        assert!(r.notice.contains("too often") && r.notice.contains("10 minutes"), "{}", r.notice);
        assert!(matches!(v, Verdict::Refused { .. }));
        assert_eq!((r.account_id, r.session_token), (0, [0; 32]));
        // A saved-token login is not affected by the lock.
        let issued = w.accounts.authenticate("Alice", &password_digest("hunter22"));
        assert!(issued.is_ok(), "the store itself does not lock");
        let token = w.accounts.issue_token(issued.unwrap().id, TokenKind::GameRemember, 1000).unwrap().token;
        assert_eq!(w.login(&token_creds("Alice", token)).0.result, 0);
        // The lock ends.
        w.time.fetch_add(600, Ordering::SeqCst);
        assert_eq!(w.login(&password_creds("Alice", "hunter22", false)).0.result, 0);
    }

    #[test]
    fn unknown_names_and_one_ip_are_limited_too() {
        let w = World::new("c", Arc::new(SqliteStore::in_memory().unwrap()));
        w.accounts.create_account("Alice", &password_digest("hunter22"), Role::Player).unwrap();
        // 5 failures per IP over different names (the account limit is 3, so use 5 names).
        for i in 0..5 {
            assert_eq!(w.login(&password_creds(&format!("ghost{i}"), "x", false)).0.result, code::NO_ACCOUNT);
        }
        // The IP is locked for every name, Alice's right password included.
        assert_eq!(w.login(&password_creds("Alice", "hunter22", false)).0.result, code::RATE_LIMITED);
    }
}
