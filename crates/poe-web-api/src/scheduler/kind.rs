//! What a task does: the [`TaskKind`] enum.
//!
//! # Adding a kind
//!
//! 1. Add a variant to [`TaskKind`] with its options as fields (they are stored as JSON in the
//!    task's `params` column and shown as JSON by the API; serde's `kind` tag names it).
//! 2. Give it a `tag` (the snake_case variant name), a check in [`TaskKind::validate`], a line
//!    in [`TaskKind::describe`] and its action in [`TaskKind::execute`].
//! 3. Add the variant to the web admin's task form (`modules/web-admin/src/pages/Schedule.svelte`
//!    and `src/lib/schedule.ts`) and run `cargo xtask openapi`.
//!
//! The database and the scheduler need no change: they treat `kind` and `params` as opaque.

use std::sync::Arc;

use poe_accounts::{AccountStore, Role};
use poe_cluster::AdminControl;
use poe_store::TaskRow;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::lifecycle::{self, Lifecycle, Refused, ShutdownKind};

/// A task's action and its options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskKind {
    /// Restart the server like `POST /admin/server/restart`: the players are warned during the
    /// countdown, then disconnected, and the server starts again (`[server] restart_mode`).
    Restart {
        /// Countdown in seconds (0 to 3600).
        #[serde(default)]
        delay_secs: u32,
        /// Added to every announcement (at most 200 characters).
        #[serde(default)]
        message: Option<String>,
    },
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Ok,
    Failed,
    /// Not run on purpose (for example a shutdown was already pending).
    Skipped,
    /// Due while the server was down or frozen: not run late.
    Missed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Missed => "missed",
        }
    }
}

/// The result of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub status: RunStatus,
    pub message: String,
}

impl RunResult {
    fn new(status: RunStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

/// What a run may touch.
pub struct RunContext<'a> {
    pub admin: &'a Arc<dyn AdminControl>,
    pub lifecycle: &'a Arc<Lifecycle>,
    pub accounts: &'a Arc<dyn AccountStore>,
    pub task: &'a TaskRow,
}

impl TaskKind {
    /// The stored `kind` column.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Restart { .. } => "restart",
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Restart {
                delay_secs,
                message,
            } => {
                if *delay_secs > lifecycle::MAX_DELAY_SECS {
                    return Err(format!(
                        "delay_secs is at most {}",
                        lifecycle::MAX_DELAY_SECS
                    ));
                }
                lifecycle::normalize(message.as_deref())
                    .map(|_| ())
                    .ok_or_else(|| {
                        format!(
                            "message is at most {} characters",
                            lifecycle::MAX_MESSAGE_CHARS
                        )
                    })
            }
        }
    }

    /// A phrase for the page.
    pub fn describe(&self) -> String {
        match self {
            Self::Restart {
                delay_secs,
                message,
            } => {
                let mut text = if *delay_secs == 0 {
                    "Restart the server at once".to_owned()
                } else {
                    format!(
                        "Restart the server after a {} countdown",
                        super::schedule::duration_text(u64::from(*delay_secs))
                    )
                };
                if let Some(m) = message.as_deref().filter(|m| !m.trim().is_empty()) {
                    text.push_str(&format!(" (\"{}\")", m.trim()));
                }
                text
            }
        }
    }

    /// The `kind` tag and the JSON of the options, as stored.
    pub fn to_parts(&self) -> (String, String) {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        if let Some(object) = value.as_object_mut() {
            object.remove("kind");
        }
        (self.tag().to_owned(), value.to_string())
    }

    /// The inverse of [`to_parts`](Self::to_parts).
    pub fn from_parts(tag: &str, params: &str) -> Result<Self, String> {
        let mut value: serde_json::Value =
            serde_json::from_str(params).map_err(|e| format!("stored options: {e}"))?;
        let object = value
            .as_object_mut()
            .ok_or("stored options are not an object")?;
        object.insert("kind".into(), tag.into());
        serde_json::from_value(value).map_err(|e| format!("task kind {tag:?}: {e}"))
    }

    /// Runs the action. Called by the scheduler when the task is due.
    pub fn execute(&self, ctx: &RunContext<'_>) -> RunResult {
        match self {
            Self::Restart {
                delay_secs,
                message,
            } => {
                // The announcements run as an admin account: the one that made the task, as
                // long as it still is one.
                let actor = ctx.task.created_by.as_str();
                match ctx.accounts.find(actor) {
                    Ok(Some(a)) if a.role == Role::Admin && !a.disabled => {}
                    Ok(_) => {
                        return RunResult::new(
                            RunStatus::Failed,
                            format!("{actor} made this task and is no longer an admin"),
                        );
                    }
                    Err(e) => {
                        return RunResult::new(RunStatus::Failed, format!("account lookup: {e}"));
                    }
                }
                let Some(message) = lifecycle::normalize(message.as_deref()) else {
                    return RunResult::new(RunStatus::Failed, "the message is too long");
                };
                match ctx.lifecycle.request(
                    ctx.admin.clone(),
                    actor,
                    ShutdownKind::Restart,
                    *delay_secs,
                    message,
                ) {
                    Ok(_) => {
                        ctx.admin.record(
                            actor,
                            "scheduled restart",
                            &format!(
                                "task #{} {:?}, countdown {delay_secs} s",
                                ctx.task.id, ctx.task.name
                            ),
                        );
                        RunResult::new(
                            RunStatus::Ok,
                            format!("restart requested, countdown {delay_secs} s"),
                        )
                    }
                    Err(Refused::Busy(p)) => RunResult::new(
                        RunStatus::Skipped,
                        format!(
                            "a {} requested by {} is already pending",
                            p.action.as_str(),
                            p.requested_by
                        ),
                    ),
                    Err(_) => RunResult::new(RunStatus::Failed, "the restart was refused"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_their_stored_parts() {
        let k = TaskKind::Restart {
            delay_secs: 300,
            message: Some("nightly".into()),
        };
        let (tag, params) = k.to_parts();
        assert_eq!(tag, "restart");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&params).unwrap(),
            serde_json::json!({"delay_secs": 300, "message": "nightly"})
        );
        assert_eq!(TaskKind::from_parts(&tag, &params).unwrap(), k);
        // Options may be left out; unknown kinds and broken options are errors.
        assert_eq!(
            TaskKind::from_parts("restart", "{}").unwrap(),
            TaskKind::Restart {
                delay_secs: 0,
                message: None
            }
        );
        assert!(
            TaskKind::from_parts("reboot", "{}")
                .unwrap_err()
                .contains("reboot")
        );
        assert!(TaskKind::from_parts("restart", "[]").is_err());
        assert!(TaskKind::from_parts("restart", "{\"delay_secs\": \"x\"}").is_err());
        assert_eq!(
            serde_json::to_string(&TaskKind::Restart {
                delay_secs: 0,
                message: None
            })
            .unwrap(),
            r#"{"kind":"restart","delay_secs":0,"message":null}"#
        );
    }

    #[test]
    fn restart_options_are_validated_and_described() {
        let ok = |d, m: Option<&str>| {
            TaskKind::Restart {
                delay_secs: d,
                message: m.map(String::from),
            }
            .validate()
        };
        assert!(ok(0, None).is_ok() && ok(3600, Some("hi")).is_ok());
        assert!(ok(3601, None).unwrap_err().contains("3600"));
        assert!(ok(0, Some(&"x".repeat(201))).unwrap_err().contains("200"));
        let k = |d, m: Option<&str>| TaskKind::Restart {
            delay_secs: d,
            message: m.map(String::from),
        };
        assert_eq!(k(0, None).describe(), "Restart the server at once");
        assert_eq!(
            k(300, Some(" back soon ")).describe(),
            "Restart the server after a 5 min countdown (\"back soon\")"
        );
    }
}
