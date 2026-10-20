//! The `account` server command (`create`, `role`, `disable`, `enable`, `passwd`, `list`),
//! shared by the in-game/overlay command registry and the `poe-server account ...` CLI so
//! both behave the same. The server hashes the password the way the client does
//! ([`crate::password_digest`]) before the argon2id step.

use crate::{AccountError, AccountRecord, AccountStore, MIN_PASSWORD_LEN, Role, password_digest};

/// What a command did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub text: String,
    /// An account whose login and instance sessions should end (it was disabled).
    pub kick_account: Option<u32>,
}

fn out(text: impl Into<String>) -> CommandOutput {
    CommandOutput { text: text.into(), kick_account: None }
}

fn ago(now: i64, then: Option<i64>) -> String {
    let Some(then) = then else { return "never".into() };
    let s = (now - then).max(0);
    match s {
        0..=119 => "just now".into(),
        120..=7199 => format!("{} min ago", s / 60),
        7200..=172_799 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

fn describe(a: &AccountRecord, now: i64) -> String {
    format!(
        "{} (#{}, {}{}{}, last login {})",
        a.username,
        a.id,
        a.role.as_str(),
        if a.disabled { ", disabled" } else { "" },
        if a.has_password { "" } else { ", no password (dev)" },
        ago(now, a.last_login)
    )
}

fn err(e: AccountError) -> String {
    e.to_string()
}

/// Runs `account <sub> [name] [value]`. `actor` is the account running it (None on the CLI):
/// an actor cannot demote or disable itself.
pub fn run_account_command(
    accounts: &dyn AccountStore,
    actor: Option<&str>,
    sub: &str,
    name: Option<&str>,
    value: Option<&str>,
) -> Result<CommandOutput, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let need_name = || name.ok_or_else(|| format!("account {sub}: missing <name>"));
    let need_value = |what: &str| value.ok_or_else(|| format!("account {sub}: missing <{what}>"));
    let is_self = |n: &str| actor.is_some_and(|a| a.eq_ignore_ascii_case(n));
    match sub {
        "list" => {
            let list = accounts.list().map_err(err)?;
            let rows: Vec<String> = list.iter().map(|a| describe(a, now)).collect();
            Ok(out(format!("{} accounts: {}", rows.len(), rows.join("; "))))
        }
        "create" => {
            let (name, password) = (need_name()?, need_value("password")?);
            if password.chars().count() < MIN_PASSWORD_LEN {
                return Err(err(AccountError::WeakPassword));
            }
            let a = accounts.create_account(name, &password_digest(password), Role::Player).map_err(err)?;
            Ok(out(format!("created {}", describe(&a, now))))
        }
        "passwd" => {
            let (name, password) = (need_name()?, need_value("password")?);
            if password.chars().count() < MIN_PASSWORD_LEN {
                return Err(err(AccountError::WeakPassword));
            }
            let a = accounts.set_password(name, &password_digest(password)).map_err(err)?;
            Ok(out(format!("password set for {}; its saved logins are revoked", a.username)))
        }
        "role" => {
            let (name, role) = (need_name()?, need_value("role")?);
            let role = Role::parse(role).ok_or_else(|| format!("unknown role {role:?} (player or admin)"))?;
            if is_self(name) && role != Role::Admin {
                return Err("you cannot demote yourself".into());
            }
            let a = accounts.set_role(name, role).map_err(err)?;
            Ok(out(format!("{} is now {} (applies to commands at once)", a.username, a.role.as_str())))
        }
        "disable" => {
            let name = need_name()?;
            if is_self(name) {
                return Err("you cannot disable yourself".into());
            }
            let a = accounts.set_disabled(name, true).map_err(err)?;
            Ok(CommandOutput { text: format!("disabled {}; it is logged out", a.username), kick_account: Some(a.id) })
        }
        "enable" => {
            let a = accounts.set_disabled(need_name()?, false).map_err(err)?;
            Ok(out(format!("enabled {}", a.username)))
        }
        other => Err(format!("unknown account subcommand {other:?} (create, role, disable, enable, passwd, list)")),
    }
}

/// The command line with passwords blanked, for logs and the audit file.
pub fn redact(sub: &str, line: &str) -> String {
    if !matches!(sub, "create" | "passwd") {
        return line.to_string();
    }
    // `account <sub> <name> <password...>`: keep the first three words.
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() <= 3 {
        return line.to_string();
    }
    let kept = &words[..3];
    format!("{} ***", kept.join(" "))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use poe_store::SqliteStore;

    use super::*;
    use crate::SqliteAccounts;

    fn accounts() -> SqliteAccounts {
        SqliteAccounts::new(Arc::new(SqliteStore::in_memory().unwrap()), "t")
            .with_params(argon2::Params::new(8, 1, 1, None).unwrap())
    }

    #[test]
    fn account_commands() {
        let a = accounts();
        let run = |actor: Option<&str>, sub, name, value| run_account_command(&a, actor, sub, name, value);
        assert!(run(None, "create", Some("alice"), Some("short")).unwrap_err().contains("at least"));
        run(None, "create", Some("alice"), Some("correct horse")).unwrap();
        assert!(run(None, "create", Some("Alice"), Some("correct horse")).unwrap_err().contains("taken"));
        // The password is the client's digest.
        a.authenticate("alice", &password_digest("correct horse")).unwrap();
        run(None, "passwd", Some("alice"), Some("battery staple")).unwrap();
        a.authenticate("alice", &password_digest("battery staple")).unwrap();
        run(None, "role", Some("alice"), Some("admin")).unwrap();
        assert_eq!(a.find("alice").unwrap().unwrap().role, Role::Admin);
        assert!(run(None, "role", Some("alice"), Some("root")).is_err());
        // Not on yourself.
        assert!(run(Some("Alice"), "role", Some("alice"), Some("player")).is_err());
        assert!(run(Some("alice"), "disable", Some("ALICE"), None).is_err());
        let id = a.find("alice").unwrap().unwrap().id;
        let d = run(None, "disable", Some("alice"), None).unwrap();
        assert_eq!(d.kick_account, Some(id));
        assert!(a.find("alice").unwrap().unwrap().disabled);
        run(None, "enable", Some("alice"), None).unwrap();
        assert!(run(None, "list", None, None).unwrap().text.starts_with("1 accounts: alice"));
        assert!(run(None, "nope", None, None).is_err());
        assert!(run(None, "disable", Some("ghost"), None).unwrap_err().contains("no such account"));
    }

    #[test]
    fn redacts_passwords() {
        assert_eq!(redact("create", "account create bob my secret pw"), "account create bob ***");
        assert_eq!(redact("passwd", "account passwd bob pw"), "account passwd bob ***");
        assert_eq!(redact("list", "account list"), "account list");
        assert_eq!(redact("create", "account create bob"), "account create bob");
    }
}
