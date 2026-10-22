//! Scheduled tasks: tasks stored in the database, run by a loop at their time.
//!
//! - A task is a name, a [`TaskKind`] (the action, currently only a server restart), a
//!   [`Schedule`] (daily, weekly, interval, cron) and an enabled flag. The tables are created by
//!   migration 0008; `poe-store` keeps `kind`, `params` and `schedule` as opaque JSON.
//! - Times are computed in the server's local time zone (the `TZ` variable, else the system
//!   zone); the pages show which zone that is.
//! - [`Scheduler::run`] polls once a second and runs what is due. **Missed runs are skipped,
//!   not replayed**: a run found more than `[scheduler] missed_grace_secs` after its time (the
//!   server was down, the machine slept) is recorded as `missed` in the history and the task
//!   moves on to its next time. At start the next runs are worked out again from the current
//!   time, so a daily 04:00 restart is never made up at 09:00.
//! - The next time is stored *before* a task runs, so a run that ends the process (a restart)
//!   cannot run again when the server comes back.
//!
//! [`TaskKind`]: kind::TaskKind
//! [`Schedule`]: schedule::Schedule

pub mod api;
pub mod kind;
pub mod schedule;

use std::{sync::Arc, time::Duration};

use jiff::tz::TimeZone;
use poe_accounts::AccountStore;
use poe_cluster::AdminControl;
use poe_store::{NewRun, StoreError, TaskRow, TaskSpec, TaskStore};
use tracing::{info, warn};

use crate::lifecycle::Lifecycle;
use kind::{RunContext, RunResult, RunStatus, TaskKind};
use schedule::Schedule;

/// Most tasks that can exist (guards the poll loop and the page).
pub const MAX_TASKS: usize = 50;
/// Longest task name (characters).
pub const MAX_NAME_CHARS: usize = 80;

/// Unix seconds. A trait so tests can run on a controlled clock.
pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

/// The system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }
}

/// The editable part of a task as the API takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInput {
    pub name: String,
    pub enabled: bool,
    pub task: TaskKind,
    pub schedule: Schedule,
}

#[derive(Debug)]
pub enum SchedError {
    /// The input is not acceptable (the message says why).
    Invalid(String),
    NotFound,
    Store(StoreError),
}

impl From<StoreError> for SchedError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// A stored task decoded.
#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub row: TaskRow,
    /// `Err`: the stored kind or schedule cannot be read (a newer or hand-edited database).
    pub decoded: Result<(TaskKind, Schedule), String>,
}

pub struct Scheduler {
    store: Arc<dyn TaskStore>,
    admin: Arc<dyn AdminControl>,
    lifecycle: Arc<Lifecycle>,
    accounts: Arc<dyn AccountStore>,
    clock: Arc<dyn Clock>,
    tz: TimeZone,
    enabled: bool,
    history_keep: u32,
    grace_secs: i64,
}

impl Scheduler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<dyn TaskStore>,
        admin: Arc<dyn AdminControl>,
        lifecycle: Arc<Lifecycle>,
        accounts: Arc<dyn AccountStore>,
        clock: Arc<dyn Clock>,
        tz: TimeZone,
        config: &poe_config::SchedulerConfig,
    ) -> Self {
        Self {
            store,
            admin,
            lifecycle,
            accounts,
            clock,
            tz,
            enabled: config.enabled,
            history_keep: config.history_keep,
            grace_secs: i64::try_from(config.missed_grace_secs).unwrap_or(i64::MAX),
        }
    }

    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    /// Whether tasks run (`[scheduler] enabled`).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn store(&self) -> &Arc<dyn TaskStore> {
        &self.store
    }

    /// The zone name for the page: `Europe/Berlin`, or the offset when the zone has no name.
    pub fn timezone_name(&self) -> String {
        self.tz
            .iana_name()
            .map_or_else(|| format!("UTC{}", self.utc_offset()), str::to_owned)
    }

    /// The zone's offset now, `+02:00`.
    pub fn utc_offset(&self) -> String {
        let now = jiff::Timestamp::from_second(self.now()).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
        let secs = self.tz.to_offset(now).seconds();
        let sign = if secs < 0 { '-' } else { '+' };
        let secs = secs.abs();
        format!("{sign}{:02}:{:02}", secs / 3600, secs % 3600 / 60)
    }

    pub fn tz(&self) -> &TimeZone {
        &self.tz
    }

    fn validate(&self, input: &TaskInput) -> Result<(), SchedError> {
        let name = input.name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err(SchedError::Invalid(format!(
                "the name is 1 to {MAX_NAME_CHARS} characters"
            )));
        }
        input.task.validate().map_err(SchedError::Invalid)?;
        input
            .schedule
            .validate()
            .map_err(|e| SchedError::Invalid(format!("schedule: {e}")))?;
        Ok(())
    }

    fn spec(&self, input: &TaskInput) -> TaskSpec {
        let (kind, params) = input.task.to_parts();
        TaskSpec {
            name: input.name.trim().to_owned(),
            kind,
            params,
            schedule: serde_json::to_string(&input.schedule).unwrap_or_default(),
            enabled: input.enabled,
        }
    }

    fn next_run(
        &self,
        schedule: &Schedule,
        enabled: bool,
        last_run: Option<i64>,
        after: i64,
    ) -> Option<i64> {
        if !enabled {
            return None;
        }
        schedule.next_after(after, last_run, &self.tz)
    }

    pub fn create(&self, by: &str, input: TaskInput) -> Result<TaskRow, SchedError> {
        let input = TaskInput {
            schedule: input.schedule.normalized(),
            ..input
        };
        self.validate(&input)?;
        if self.store.tasks()?.len() >= MAX_TASKS {
            return Err(SchedError::Invalid(format!("at most {MAX_TASKS} tasks")));
        }
        let now = self.now();
        let next = self.next_run(&input.schedule, input.enabled, None, now);
        Ok(self.store.create_task(&self.spec(&input), by, now, next)?)
    }

    pub fn update(&self, id: u32, input: TaskInput) -> Result<TaskRow, SchedError> {
        let input = TaskInput {
            schedule: input.schedule.normalized(),
            ..input
        };
        self.validate(&input)?;
        let old = self.store.task(id)?.ok_or(SchedError::NotFound)?;
        let now = self.now();
        let next = self.next_run(&input.schedule, input.enabled, old.last_run_at, now);
        self.store
            .update_task(id, &self.spec(&input), now, next)?
            .ok_or(SchedError::NotFound)
    }

    /// Turns a task on or off; turning it on works out the next run from now.
    pub fn set_enabled(&self, id: u32, enabled: bool) -> Result<TaskRow, SchedError> {
        let info = self.info(self.store.task(id)?.ok_or(SchedError::NotFound)?);
        let (task, schedule) = info.decoded.map_err(SchedError::Invalid)?;
        self.update(
            id,
            TaskInput {
                name: info.row.name,
                enabled,
                task,
                schedule,
            },
        )
    }

    pub fn delete(&self, id: u32) -> Result<(), SchedError> {
        if self.store.delete_task(id)? {
            Ok(())
        } else {
            Err(SchedError::NotFound)
        }
    }

    pub fn info(&self, row: TaskRow) -> TaskInfo {
        let decoded = TaskKind::from_parts(&row.kind, &row.params).and_then(|kind| {
            let schedule = serde_json::from_str::<Schedule>(&row.schedule)
                .map_err(|e| format!("stored schedule: {e}"))?;
            Ok((kind, schedule))
        });
        TaskInfo { row, decoded }
    }

    pub fn tasks(&self) -> Result<Vec<TaskInfo>, SchedError> {
        Ok(self
            .store
            .tasks()?
            .into_iter()
            .map(|r| self.info(r))
            .collect())
    }

    /// Work done once at start, before the loop: a run that came due while the server was
    /// down is recorded as missed (never made up), and every enabled task's next run is worked
    /// out again from the current time (the zone or the schedule may have changed).
    pub fn reconcile(&self) -> Result<(), SchedError> {
        let now = self.now();
        for row in self.store.tasks()? {
            let info = self.info(row);
            let (id, enabled, stored_next, last_run) = (
                info.row.id,
                info.row.enabled,
                info.row.next_run_at,
                info.row.last_run_at,
            );
            let Ok((_, schedule)) = &info.decoded else {
                warn!(
                    task = id,
                    "scheduled task cannot be read: {}",
                    info.decoded.as_ref().err().map_or("", String::as_str)
                );
                continue;
            };
            if !enabled {
                continue;
            }
            match stored_next {
                // Due within the grace: the first tick runs it.
                Some(due) if due <= now && now - due <= self.grace_secs => continue,
                Some(due) if due <= now => {
                    info!(
                        task = id,
                        due, "scheduled run missed while the server was down: skipped"
                    );
                    self.record(
                        id,
                        due,
                        now,
                        RunStatus::Missed,
                        "the server was not running at the scheduled time",
                    );
                }
                _ => {}
            }
            let next = self.next_run(schedule, true, last_run, now);
            self.store.set_next_run(id, next)?;
        }
        Ok(())
    }

    fn record(&self, id: u32, due: i64, started: i64, status: RunStatus, message: &str) {
        let run = NewRun {
            due_at: due,
            started_at: started,
            status: status.as_str().into(),
            message: message.into(),
        };
        if let Err(e) = self.store.record_run(id, &run, self.history_keep) {
            warn!(task = id, "recording a scheduled run: {e}");
        }
    }

    /// Runs the tasks due at `now`. Returns how many ran or were skipped as missed.
    pub fn run_due(&self, now: i64) -> usize {
        let rows = match self.store.tasks() {
            Ok(r) => r,
            Err(e) => {
                warn!("scheduler: {e}");
                return 0;
            }
        };
        let mut handled = 0;
        for row in rows.into_iter().filter(|r| r.enabled) {
            let Some(due) = row.next_run_at.filter(|d| *d <= now) else {
                continue;
            };
            let info = self.info(row);
            let id = info.row.id;
            let (task, schedule) = match &info.decoded {
                Ok(parts) => parts.clone(),
                Err(e) => {
                    // Unreadable: leave it off the schedule so it does not come up every second.
                    self.record(id, due, now, RunStatus::Failed, e);
                    let _ = self.store.set_next_run(id, None);
                    handled += 1;
                    continue;
                }
            };
            handled += 1;
            // The next time first: a restart ends the process, and the stored time must already
            // be the next one when the server comes back.
            let next = self.next_run(&schedule, true, Some(now), now);
            if let Err(e) = self.store.set_next_run(id, next) {
                warn!(task = id, "scheduler: {e}");
                continue;
            }
            if now - due > self.grace_secs {
                info!(task = id, due, "scheduled run found late: skipped");
                self.record(
                    id,
                    due,
                    now,
                    RunStatus::Missed,
                    "found too late after its time (the server was frozen or asleep)",
                );
                continue;
            }
            let ctx = RunContext {
                admin: &self.admin,
                lifecycle: &self.lifecycle,
                accounts: &self.accounts,
                task: &info.row,
            };
            let RunResult { status, message } = task.execute(&ctx);
            info!(task = id, name = %info.row.name, status = status.as_str(), "scheduled task ran: {message}");
            self.record(id, due, now, status, &message);
        }
        handled
    }

    /// The loop: reconciles, then polls every second until the runtime ends.
    pub async fn run(self: Arc<Self>) {
        if !self.enabled {
            info!("scheduler is off ([scheduler] enabled = false)");
            return;
        }
        if let Err(e) = self.reconcile() {
            warn!("scheduler start: {e:?}");
        }
        let mut every = tokio::time::interval(Duration::from_secs(1));
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            every.tick().await;
            let now = self.now();
            let me = self.clone();
            // SQLite and the audit file are blocking.
            if tokio::task::spawn_blocking(move || me.run_due(now))
                .await
                .is_err()
            {
                warn!("scheduler: a run panicked");
            }
        }
    }
}

#[cfg(test)]
mod tests;
