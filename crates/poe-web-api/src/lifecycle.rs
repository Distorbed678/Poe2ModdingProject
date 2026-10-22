//! Stop and restart requests (`POST /admin/server/stop|restart`).
//!
//! A request starts a countdown task: it announces the shutdown to the players in game (the
//! `announce` command, so it shows as a `<Server>` chat line and lands in the audit file), waits
//! out `delay_secs`, announces the final line, and then hands the request to the process over a
//! channel ([`Lifecycle::new`] returns the receiver). The process (`servers/poe-server`) then
//! drains the sessions and exits or starts itself again; this crate does not end the process.
//! Only one request can be pending; until the final announcement it can be cancelled.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use poe_cluster::AdminControl;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::mpsc,
    task::AbortHandle,
    time::{Instant, sleep, sleep_until},
};
use tracing::{info, warn};
use utoipa::ToSchema;

/// The longest countdown a request may ask for.
pub const MAX_DELAY_SECS: u32 = 3600;
/// The longest custom message (characters, after whitespace is normalised).
pub const MAX_MESSAGE_CHARS: usize = 200;
/// Seconds before the end at which the countdown is announced (besides the start).
const MARKS: [u64; 7] = [600, 300, 120, 60, 30, 10, 5];
/// Time between the final announcement and the hand-over, so the chat line reaches the clients.
const FINAL_GRACE: Duration = Duration::from_millis(1500);

/// What the process does once the countdown is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ShutdownKind {
    /// Shut down and exit with code 0.
    Stop,
    /// Shut down and start again (`[server] restart_mode`).
    Restart,
}

impl ShutdownKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }

    fn progressive(self) -> &'static str {
        match self {
            Self::Stop => "stopping",
            Self::Restart => "restarting",
        }
    }
}

/// A shutdown that was requested and has not happened yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PendingShutdown {
    pub action: ShutdownKind,
    /// Seconds until the players are disconnected (0 once the final announcement is out).
    pub remaining_secs: u64,
    /// The countdown the request asked for.
    pub delay_secs: u32,
    /// The admin's message to the players.
    pub message: Option<String>,
    /// The admin account that asked.
    pub requested_by: String,
    /// `false` once the final announcement is out: it can no longer be cancelled.
    pub cancellable: bool,
}

struct Pending {
    kind: ShutdownKind,
    delay_secs: u32,
    message: Option<String>,
    actor: String,
    ends: Instant,
    committed: bool,
    task: AbortHandle,
}

impl Pending {
    fn view(&self) -> PendingShutdown {
        let left = self.ends.saturating_duration_since(Instant::now());
        PendingShutdown {
            action: self.kind,
            remaining_secs: left.as_secs() + u64::from(left.subsec_nanos() > 0),
            delay_secs: self.delay_secs,
            message: self.message.clone(),
            requested_by: self.actor.clone(),
            cancellable: !self.committed,
        }
    }
}

/// Why a request was not accepted.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Another shutdown is already pending.
    Busy(PendingShutdown),
    /// Nothing to cancel.
    Nothing,
    /// Too late to cancel.
    Committed,
}

/// The pending shutdown, if any, and the channel to the process.
pub struct Lifecycle {
    pending: Mutex<Option<Pending>>,
    tx: mpsc::UnboundedSender<ShutdownKind>,
}

impl Lifecycle {
    /// The controller and the receiver the process waits on.
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<ShutdownKind>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                pending: Mutex::new(None),
                tx,
            }),
            rx,
        )
    }

    /// The pending shutdown, if any.
    pub fn status(&self) -> Option<PendingShutdown> {
        self.pending.lock().unwrap().as_ref().map(Pending::view)
    }

    /// Starts the countdown (the caller checked the arguments with [`normalize`]).
    pub fn request(
        self: &Arc<Self>,
        admin: Arc<dyn AdminControl>,
        actor: &str,
        kind: ShutdownKind,
        delay_secs: u32,
        message: Option<String>,
    ) -> Result<PendingShutdown, Refused> {
        let mut slot = self.pending.lock().unwrap();
        if let Some(p) = slot.as_ref() {
            return Err(Refused::Busy(p.view()));
        }
        let ends = Instant::now() + Duration::from_secs(u64::from(delay_secs));
        let task = tokio::spawn(countdown(
            self.clone(),
            admin,
            actor.to_owned(),
            kind,
            delay_secs,
            message.clone(),
            ends,
        ))
        .abort_handle();
        let pending = Pending {
            kind,
            delay_secs,
            message,
            actor: actor.to_owned(),
            ends,
            committed: false,
            task,
        };
        let view = pending.view();
        *slot = Some(pending);
        Ok(view)
    }

    /// Cancels the pending shutdown while it is still in its countdown.
    pub fn cancel(&self) -> Result<PendingShutdown, Refused> {
        let mut slot = self.pending.lock().unwrap();
        match slot.as_ref() {
            None => Err(Refused::Nothing),
            Some(p) if p.committed => Err(Refused::Committed),
            Some(_) => {
                let p = slot.take().expect("checked above");
                p.task.abort();
                Ok(p.view())
            }
        }
    }

    /// Past the point of no return: the final line is announced.
    fn commit(&self) {
        if let Some(p) = self.pending.lock().unwrap().as_mut() {
            p.committed = true;
        }
    }
}

/// The message of a request: whitespace normalised, empty = none. `None` if it is too long.
pub fn normalize(message: Option<&str>) -> Option<Option<String>> {
    let text = message
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return None;
    }
    Some((!text.is_empty()).then_some(text))
}

/// The moments, as seconds before the end, at which a countdown of `delay` seconds is announced:
/// the start, then each of [`MARKS`] below it. Descending; empty for no delay.
pub fn announce_marks(delay: u64) -> Vec<u64> {
    if delay == 0 {
        return Vec::new();
    }
    let mut marks = vec![delay];
    marks.extend(MARKS.iter().copied().filter(|m| *m < delay));
    marks
}

/// `90` -> "1 min 30 s".
pub fn human(secs: u64) -> String {
    match (secs / 60, secs % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// The chat line of a countdown (`left` seconds to go; 0 = now).
pub fn announcement(kind: ShutdownKind, left: u64, message: Option<&str>) -> String {
    let head = if left == 0 {
        format!("Server {} now", kind.progressive())
    } else {
        format!("Server {} in {}", kind.progressive(), human(left))
    };
    match message {
        Some(m) => format!("{head}: {m}"),
        None => head,
    }
}

async fn countdown(
    lc: Arc<Lifecycle>,
    admin: Arc<dyn AdminControl>,
    actor: String,
    kind: ShutdownKind,
    delay: u32,
    message: Option<String>,
    ends: Instant,
) {
    let say = |left: u64| {
        let line = format!("announce {}", announcement(kind, left, message.as_deref()));
        let out = admin.run_command(&actor, &line);
        if !out.ok {
            warn!(actor = %actor, "shutdown announcement failed: {}", out.text);
        }
    };
    for left in announce_marks(u64::from(delay)) {
        sleep_until(ends - Duration::from_secs(left)).await;
        say(left);
    }
    sleep_until(ends).await;
    say(0);
    lc.commit();
    info!(action = kind.as_str(), actor = %actor, "shutdown countdown over, handing over to the process");
    sleep(FINAL_GRACE).await;
    // The receiver is gone only when the process is already ending.
    let _ = lc.tx.send(kind);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_start_at_the_delay_and_skip_larger_ones() {
        assert!(announce_marks(0).is_empty());
        assert_eq!(announce_marks(3), vec![3]);
        assert_eq!(announce_marks(5), vec![5]);
        assert_eq!(announce_marks(45), vec![45, 30, 10, 5]);
        assert_eq!(announce_marks(600), vec![600, 300, 120, 60, 30, 10, 5]);
        assert_eq!(announce_marks(3600)[0], 3600);
    }

    #[test]
    fn announcement_texts() {
        assert_eq!(human(5), "5 s");
        assert_eq!(human(60), "1 min");
        assert_eq!(human(90), "1 min 30 s");
        assert_eq!(
            announcement(ShutdownKind::Restart, 90, None),
            "Server restarting in 1 min 30 s"
        );
        assert_eq!(
            announcement(ShutdownKind::Stop, 0, Some("maintenance")),
            "Server stopping now: maintenance"
        );
    }

    #[test]
    fn messages_are_normalised_and_bounded() {
        assert_eq!(normalize(None), Some(None));
        assert_eq!(normalize(Some("  \n ")), Some(None));
        assert_eq!(
            normalize(Some(" back  in\n5 ")),
            Some(Some("back in 5".into()))
        );
        assert_eq!(normalize(Some(&"x".repeat(MAX_MESSAGE_CHARS + 1))), None);
        assert!(normalize(Some(&"x".repeat(MAX_MESSAGE_CHARS))).is_some());
    }
}
