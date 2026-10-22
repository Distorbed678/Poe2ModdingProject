//! Description of the common config keys, for the web admin's form editor.
//!
//! The form is generated from this list: one entry per key with its section, type, bounds,
//! default and help text. The raw editor covers every other key. A test checks that each
//! entry's default, minimum and maximum parse as a valid config, so the list cannot drift away
//! from the typed config in `lib.rs`. To offer another key in the form, add an entry here.

use crate::edit::EditValue;

/// The type of a form field.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Bool,
    /// Whole number within `min..=max`.
    Int {
        min: i64,
        max: i64,
    },
    /// One line of text, or a few lines for `multiline`, at most `max_len` characters.
    Text {
        multiline: bool,
        max_len: usize,
    },
    /// One of these words.
    Choice(&'static [&'static str]),
    /// A list of one-line strings.
    List,
}

/// One key of the config file.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSpec {
    /// The table, e.g. `web` or `network.login`.
    pub section: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: FieldKind,
    /// The value that applies when the key is absent.
    pub default: EditValue,
    /// Whether a running server needs a restart to use a changed value (everything but the
    /// message of the day, which the editor applies live).
    pub restart: bool,
}

impl FieldSpec {
    /// `section.key`.
    pub fn path(&self) -> String {
        format!("{}.{}", self.section, self.key)
    }
}

fn spec(
    section: &'static str,
    key: &'static str,
    label: &'static str,
    kind: FieldKind,
    default: EditValue,
    help: &'static str,
) -> FieldSpec {
    FieldSpec {
        section,
        key,
        label,
        help,
        kind,
        default,
        restart: true,
    }
}

fn int(min: i64, max: i64) -> FieldKind {
    FieldKind::Int { min, max }
}

fn text(max_len: usize) -> FieldKind {
    FieldKind::Text {
        multiline: false,
        max_len,
    }
}

fn t(s: &str) -> EditValue {
    EditValue::Text(s.to_owned())
}

/// The keys the form offers, in display order.
pub fn common_fields() -> Vec<FieldSpec> {
    use EditValue::{Bool, Int, List};
    let mut motd = spec(
        "server",
        "motd",
        "Message of the day",
        FieldKind::Text {
            multiline: true,
            max_len: crate::MOTD_MAX_CHARS,
        },
        t(""),
        "Plain text the launcher's Home page shows. Applied at once when saved here.",
    );
    motd.restart = false;
    vec![
        motd,
        spec(
            "server",
            "restart_mode",
            "Restart mode",
            FieldKind::Choice(&["exec", "exit"]),
            t("exec"),
            "What a restart does: exec = the process starts itself again, exit = exit code 75 for a supervisor (systemd, the launcher).",
        ),
        spec(
            "gateway",
            "display_name",
            "Server name",
            text(64),
            t("Local Server"),
            "Shown in the client's realm list and by the launcher.",
        ),
        spec(
            "gateway",
            "region",
            "Region",
            text(32),
            t("Local"),
            "Realm region written into the client's gateway list.",
        ),
        spec(
            "network",
            "public_host",
            "Public host",
            text(255),
            t("127.0.0.1"),
            "Address the game clients are sent to. Set the LAN or public address for other players.",
        ),
        spec(
            "accounts",
            "open_registration",
            "Open registration",
            FieldKind::Bool,
            Bool(false),
            "Whether anyone can create an account through the web API. Keep off unless the API sits behind HTTPS.",
        ),
        spec(
            "accounts",
            "remember_days",
            "Remember Me (days)",
            int(1, 3650),
            Int(30),
            "How long a saved login stays valid.",
        ),
        spec(
            "accounts",
            "max_characters",
            "Character slots",
            int(1, 255),
            Int(24),
            "Character slots per account.",
        ),
        spec(
            "security",
            "login_max_failures_per_account",
            "Failed logins per account",
            int(0, 1_000_000),
            Int(5),
            "Within the window before the account name is locked. 0 = no limit.",
        ),
        spec(
            "security",
            "login_max_failures_per_ip",
            "Failed logins per IP",
            int(0, 1_000_000),
            Int(20),
            "Within the window before the client address is locked. 0 = no limit.",
        ),
        spec(
            "security",
            "login_failure_window_secs",
            "Failure window (s)",
            int(1, 86_400),
            Int(300),
            "Window in which failed logins are counted.",
        ),
        spec(
            "security",
            "login_lockout_secs",
            "Lockout (s)",
            int(1, 86_400),
            Int(300),
            "How long a locked name or address stays locked.",
        ),
        spec(
            "security",
            "trust_forwarded_for",
            "Trust X-Forwarded-For",
            FieldKind::Bool,
            Bool(false),
            "Only behind a TLS reverse proxy you control that sets the header; otherwise a client can pick its address.",
        ),
        spec(
            "security",
            "allow_unknown_client",
            "Allow unknown client builds",
            FieldKind::Bool,
            Bool(false),
            "Accept an exe that is not a known build of this server's version.",
        ),
        spec(
            "instance",
            "idle_minutes",
            "Idle instance (min)",
            int(0, 100_000),
            Int(15),
            "Empty instances close after this long. 0 = never.",
        ),
        spec(
            "web",
            "enabled",
            "Web API",
            FieldKind::Bool,
            Bool(true),
            "Start the HTTP API and web admin with the server. Turning it off here locks you out of this page after a restart.",
        ),
        spec(
            "web",
            "bind",
            "Web bind address",
            text(64),
            t("127.0.0.1:8080"),
            "Where the API listens. Loopback by default; use a TLS reverse proxy before exposing it.",
        ),
        spec(
            "web",
            "static_dir",
            "Web admin files",
            text(255),
            t(""),
            "Directory with the built web admin. Empty = none.",
        ),
        spec(
            "web",
            "cors_origins",
            "CORS origins",
            FieldKind::List,
            List(Vec::new()),
            "Browser origins allowed to call the API cross-origin, e.g. a Vite dev server.",
        ),
        spec(
            "web",
            "log_buffer",
            "Log buffer (events)",
            int(10, 1_000_000),
            Int(2000),
            "Recent log events kept for the live log viewer.",
        ),
        spec(
            "web",
            "web_token_ttl_secs",
            "Sign-in link lifetime (s)",
            int(1, 3600),
            Int(60),
            "Lifetime of the launcher's single-use web sign-in token.",
        ),
        spec(
            "web",
            "config_editor",
            "Config editor",
            FieldKind::Bool,
            Bool(true),
            "Allow editing this file from the web admin. Off: the editor refuses to save.",
        ),
        spec(
            "web",
            "config_backup_keep",
            "Config backups kept",
            int(1, 1000),
            Int(20),
            "Previous versions of this file kept when it is saved here.",
        ),
        spec(
            "metrics",
            "enabled",
            "Metrics",
            FieldKind::Bool,
            Bool(true),
            "Sample players, tick time, memory and CPU for the dashboard charts.",
        ),
        spec(
            "metrics",
            "interval_secs",
            "Sample interval (s)",
            int(1, 3600),
            Int(5),
            "Seconds between samples.",
        ),
        spec(
            "metrics",
            "retention_hours",
            "Metrics kept (h)",
            int(1, 168),
            Int(24),
            "Hours of samples held in memory (they are lost on a restart).",
        ),
        spec(
            "scheduler",
            "enabled",
            "Scheduled tasks",
            FieldKind::Bool,
            Bool(true),
            "Run the tasks of the Schedule page.",
        ),
        spec(
            "scheduler",
            "history_keep",
            "Runs kept per task",
            int(1, 10_000),
            Int(100),
            "Length of each task's history.",
        ),
        spec(
            "scheduler",
            "missed_grace_secs",
            "Missed run grace (s)",
            int(1, 86_400),
            Int(120),
            "A run found later than this after its time is skipped, not run late.",
        ),
        spec(
            "commands",
            "prefix",
            "Chat command prefix",
            text(8),
            t("!"),
            "Prefix of server commands typed in chat.",
        ),
        spec(
            "logging",
            "filter",
            "Log filter",
            text(256),
            t("info"),
            "tracing EnvFilter syntax; RUST_LOG overrides it.",
        ),
    ]
}

/// Checks a value against a field: the type, bounds and length. The typed config in `lib.rs`
/// is the final judge when the text is saved; this gives the form's own message early.
pub fn check_value(field: &FieldSpec, value: &EditValue) -> Result<(), String> {
    let label = field.label;
    match (&field.kind, value) {
        (_, EditValue::Unset) => Ok(()),
        (FieldKind::Bool, EditValue::Bool(_)) => Ok(()),
        (FieldKind::Int { min, max }, EditValue::Int(i)) => {
            if (*min..=*max).contains(i) {
                Ok(())
            } else {
                Err(format!("{label} must be between {min} and {max}"))
            }
        }
        (FieldKind::Text { multiline, max_len }, EditValue::Text(s)) => {
            if s.chars().count() > *max_len {
                return Err(format!("{label} is at most {max_len} characters"));
            }
            if s.chars()
                .any(|c| c.is_control() && !(*multiline && c == '\n'))
            {
                return Err(format!("{label} must not contain control characters"));
            }
            Ok(())
        }
        (FieldKind::Choice(options), EditValue::Text(s)) => {
            if options.contains(&s.as_str()) {
                Ok(())
            } else {
                Err(format!("{label} must be one of: {}", options.join(", ")))
            }
        }
        (FieldKind::List, EditValue::List(items)) => {
            if items.len() > 100
                || items
                    .iter()
                    .any(|i| i.chars().count() > 256 || i.chars().any(char::is_control))
            {
                Err(format!(
                    "{label}: at most 100 entries of up to 256 characters each"
                ))
            } else {
                Ok(())
            }
        }
        _ => Err(format!("{label}: wrong type of value")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, edit::set_values};

    const MINIMAL: &str = "[game]\nversion = \"0.5.5e\"\ngame_dir = \"g\"\nschema = \"s\"\nprotocol_dir = \"p\"\n[network]\npublic_host = \"h\"\n[network.login]\nbind = \"0.0.0.0:1\"\n[network.instance]\nbind = \"0.0.0.0:2\"\npublic_port = 2\n[database]\npath = \"d\"\n[accounts]\nname_prefix = \"Exile\"\nmax_characters = 1\n[gateway]\nregion = \"r\"\ndisplay_name = \"n\"\n";

    #[test]
    fn keys_are_unique_and_defaults_pass_their_own_checks() {
        let fields = common_fields();
        let mut paths: Vec<String> = fields.iter().map(FieldSpec::path).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), fields.len(), "duplicate field");
        for f in &fields {
            check_value(f, &f.default).unwrap_or_else(|e| panic!("{}: {e}", f.path()));
        }
    }

    /// Every field's default, minimum and maximum make a config the typed parser accepts, and
    /// a default equals what the parser uses when the key is absent (compared through Debug of
    /// the section).
    #[test]
    fn bounds_and_defaults_are_accepted_by_the_typed_config() {
        for f in common_fields() {
            let mut tries = vec![f.default.clone()];
            match &f.kind {
                FieldKind::Int { min, max } => {
                    tries.push(EditValue::Int(*min));
                    tries.push(EditValue::Int(*max));
                }
                FieldKind::Choice(options) => tries.extend(options.iter().map(|o| t(o))),
                FieldKind::Text { max_len, .. } => {
                    tries.push(EditValue::Text("x".repeat((*max_len).min(300))))
                }
                _ => {}
            }
            for v in tries {
                // A web bind must be an address; the placeholder text is not one.
                if f.path() == "web.bind" && matches!(&v, EditValue::Text(s) if s.starts_with('x'))
                {
                    continue;
                }
                let text = set_values(MINIMAL, &[(f.path(), v.clone())]).unwrap();
                Config::parse(&text).unwrap_or_else(|e| panic!("{} = {v:?}: {e}", f.path()));
            }
            // Out of range is refused by the typed config where it has a range of its own.
            if let FieldKind::Int { max, .. } = &f.kind
                && matches!(
                    f.path().as_str(),
                    "metrics.interval_secs"
                        | "metrics.retention_hours"
                        | "scheduler.missed_grace_secs"
                )
            {
                let text = set_values(MINIMAL, &[(f.path(), EditValue::Int(*max + 1))]).unwrap();
                assert!(
                    Config::parse(&text).is_err(),
                    "{} above its maximum",
                    f.path()
                );
            }
        }
    }

    #[test]
    fn defaults_equal_the_parsers_defaults() {
        let base = Config::parse(MINIMAL).unwrap();
        let get = |p: &str| -> Option<EditValue> {
            Some(match p {
                "server.restart_mode" => t(match base.server.restart_mode {
                    crate::RestartMode::Exec => "exec",
                    crate::RestartMode::Exit => "exit",
                }),
                "server.motd" => t(&base.server.motd),
                "accounts.open_registration" => EditValue::Bool(base.accounts.open_registration),
                "accounts.remember_days" => EditValue::Int(base.accounts.remember_days.into()),
                "security.login_max_failures_per_account" => {
                    EditValue::Int(base.security.login_max_failures_per_account.into())
                }
                "security.login_max_failures_per_ip" => {
                    EditValue::Int(base.security.login_max_failures_per_ip.into())
                }
                "security.login_failure_window_secs" => {
                    EditValue::Int(base.security.login_failure_window_secs as i64)
                }
                "security.login_lockout_secs" => {
                    EditValue::Int(base.security.login_lockout_secs as i64)
                }
                "security.trust_forwarded_for" => {
                    EditValue::Bool(base.security.trust_forwarded_for)
                }
                "security.allow_unknown_client" => {
                    EditValue::Bool(base.security.allow_unknown_client)
                }
                "instance.idle_minutes" => EditValue::Int(base.instance.idle_minutes as i64),
                "web.enabled" => EditValue::Bool(base.web.enabled),
                "web.bind" => t(&base.web.bind.to_string()),
                "web.static_dir" => t(&base.web.static_dir.to_string_lossy()),
                "web.cors_origins" => EditValue::List(base.web.cors_origins.clone()),
                "web.log_buffer" => EditValue::Int(base.web.log_buffer as i64),
                "web.web_token_ttl_secs" => EditValue::Int(base.web.web_token_ttl_secs as i64),
                "web.config_editor" => EditValue::Bool(base.web.config_editor),
                "web.config_backup_keep" => EditValue::Int(base.web.config_backup_keep as i64),
                "metrics.enabled" => EditValue::Bool(base.metrics.enabled),
                "metrics.interval_secs" => EditValue::Int(base.metrics.interval_secs as i64),
                "metrics.retention_hours" => EditValue::Int(base.metrics.retention_hours as i64),
                "scheduler.enabled" => EditValue::Bool(base.scheduler.enabled),
                "scheduler.history_keep" => EditValue::Int(base.scheduler.history_keep.into()),
                "scheduler.missed_grace_secs" => {
                    EditValue::Int(base.scheduler.missed_grace_secs as i64)
                }
                "commands.prefix" => t(&base.commands.prefix),
                "logging.filter" => t(&base.logging.filter),
                // Required keys without a parser default: the shipped example's values.
                "gateway.display_name"
                | "gateway.region"
                | "network.public_host"
                | "accounts.max_characters" => return None,
                other => panic!("no default check for {other}"),
            })
        };
        for f in common_fields() {
            if let Some(expected) = get(&f.path()) {
                assert_eq!(f.default, expected, "{}", f.path());
            }
        }
    }
}
