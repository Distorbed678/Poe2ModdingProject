//! Scheduler tests: a controlled clock, an in-memory database, a recording admin. The loop test
//! runs on tokio's paused clock.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, Ordering},
};

use jiff::{Timestamp, tz::TimeZone};
use poe_accounts::{AccountStore, Role, SqliteAccounts, password_digest};
use poe_cluster::{AdminControl, AdminOutcome, NewInstance, OnlinePlayer, TransferTarget};
use poe_store::{SqliteStore, TaskStore};

use super::*;
use crate::lifecycle::{Lifecycle, ShutdownKind};

struct ManualClock(AtomicI64);

impl Clock for ManualClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct Admin {
    calls: Mutex<Vec<String>>,
}

impl AdminControl for Admin {
    fn online(&self) -> Vec<OnlinePlayer> {
        Vec::new()
    }
    fn run_command(&self, actor: &str, line: &str) -> AdminOutcome {
        self.calls.lock().unwrap().push(format!("{actor}: {line}"));
        AdminOutcome::ok("done")
    }
    fn create_instance(&self, _: &str, _: &NewInstance) -> Result<u32, AdminOutcome> {
        Err(AdminOutcome::err("unused"))
    }
    fn transfer_player(&self, _: &str, _: &str, _: TransferTarget) -> AdminOutcome {
        AdminOutcome::err("unused")
    }
    fn record(&self, actor: &str, action: &str, result: &str) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{actor}: record {action} [{result}]"));
    }
}

struct World {
    sched: Arc<Scheduler>,
    clock: Arc<ManualClock>,
    admin: Arc<Admin>,
    lifecycle: Arc<Lifecycle>,
    shutdown: tokio::sync::mpsc::UnboundedReceiver<ShutdownKind>,
    accounts: Arc<dyn AccountStore>,
    store: Arc<SqliteStore>,
}

fn ts(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_second()
}

fn world_with(start: i64, clock: Option<Arc<dyn Clock>>) -> World {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let accounts: Arc<dyn AccountStore> = Arc::new(
        SqliteAccounts::new(store.clone(), "c")
            .with_params(poe_accounts::Params::new(8, 1, 1, None).unwrap()),
    );
    accounts
        .create_account("root", &password_digest("hunter22"), Role::Admin)
        .unwrap();
    accounts
        .create_account("bob", &password_digest("hunter22"), Role::Player)
        .unwrap();
    let manual = Arc::new(ManualClock(AtomicI64::new(start)));
    let admin = Arc::new(Admin::default());
    let (lifecycle, shutdown) = Lifecycle::new();
    let config = poe_config::SchedulerConfig {
        enabled: true,
        history_keep: 5,
        missed_grace_secs: 120,
    };
    let sched = Arc::new(Scheduler::new(
        store.clone(),
        admin.clone(),
        lifecycle.clone(),
        accounts.clone(),
        clock.unwrap_or_else(|| manual.clone()),
        TimeZone::fixed(jiff::tz::offset(2)),
        &config,
    ));
    World {
        sched,
        clock: manual,
        admin,
        lifecycle,
        shutdown,
        accounts,
        store,
    }
}

fn world() -> World {
    // 2026-10-22 03:00:00 local (+02:00) = 01:00 UTC.
    world_with(ts("2026-10-22T01:00:00Z"), None)
}

fn restart_input(name: &str, schedule: Schedule) -> TaskInput {
    TaskInput {
        name: name.into(),
        enabled: true,
        task: TaskKind::Restart {
            delay_secs: 30,
            message: Some("nightly".into()),
        },
        schedule,
    }
}

fn daily(t: &str) -> Schedule {
    Schedule::Daily { time: t.into() }
}

#[tokio::test(start_paused = true)]
async fn tasks_are_validated_stored_and_get_their_next_run() {
    let w = world();
    let task = w
        .sched
        .create("root", restart_input(" Nightly ", daily("4:00")))
        .unwrap();
    // 04:00 local is 02:00 UTC, an hour from the clock's 01:00 UTC.
    assert_eq!(
        (
            task.name.as_str(),
            task.kind.as_str(),
            task.next_run_at,
            task.created_by.as_str()
        ),
        (
            "Nightly",
            "restart",
            Some(ts("2026-10-22T02:00:00Z")),
            "root"
        )
    );
    assert_eq!(
        task.schedule, r#"{"type":"daily","time":"04:00"}"#,
        "normalised"
    );
    assert_eq!(task.params, r#"{"delay_secs":30,"message":"nightly"}"#);
    assert_eq!(w.sched.timezone_name(), "UTC+02:00");
    assert_eq!(w.sched.utc_offset(), "+02:00");

    // Bad input is refused with a reason and stores nothing.
    let bad = |input: TaskInput| match w.sched.create("root", input) {
        Err(SchedError::Invalid(m)) => m,
        other => panic!("{other:?}"),
    };
    assert!(bad(restart_input("", daily("04:00"))).contains("name"));
    assert!(bad(restart_input(&"x".repeat(81), daily("04:00"))).contains("name"));
    assert!(bad(restart_input("a", daily("25:00"))).contains("schedule"));
    assert!(
        bad(restart_input(
            "a",
            Schedule::Cron {
                expr: "* * *".into()
            }
        ))
        .contains("5 fields")
    );
    let mut long = restart_input("a", daily("04:00"));
    long.task = TaskKind::Restart {
        delay_secs: 99_999,
        message: None,
    };
    assert!(bad(long).contains("3600"));
    assert_eq!(w.sched.tasks().unwrap().len(), 1);

    // Editing recomputes; disabling clears the next run; enabling works it out again.
    let edited = w
        .sched
        .update(task.id, restart_input("Nightly 2", daily("05:30")))
        .unwrap();
    assert_eq!(
        (edited.name.as_str(), edited.next_run_at),
        ("Nightly 2", Some(ts("2026-10-22T03:30:00Z")))
    );
    let off = w.sched.set_enabled(task.id, false).unwrap();
    assert_eq!((off.enabled, off.next_run_at), (false, None));
    let on = w.sched.set_enabled(task.id, true).unwrap();
    assert_eq!(on.next_run_at, Some(ts("2026-10-22T03:30:00Z")));
    assert!(matches!(
        w.sched.update(99, restart_input("x", daily("04:00"))),
        Err(SchedError::NotFound)
    ));
    assert!(matches!(
        w.sched.set_enabled(99, true),
        Err(SchedError::NotFound)
    ));
    w.sched.delete(task.id).unwrap();
    assert!(matches!(w.sched.delete(task.id), Err(SchedError::NotFound)));

    // A limit on how many.
    for i in 0..MAX_TASKS {
        w.sched
            .create("root", restart_input(&format!("t{i}"), daily("04:00")))
            .unwrap();
    }
    assert!(
        matches!(w.sched.create("root", restart_input("one more", daily("04:00"))), Err(SchedError::Invalid(m)) if m.contains("at most"))
    );
}

#[tokio::test(start_paused = true)]
async fn a_due_restart_runs_once_and_moves_to_the_next_day() {
    let mut w = world();
    let task = w
        .sched
        .create("root", restart_input("Nightly", daily("04:00")))
        .unwrap();
    let due = task.next_run_at.unwrap();

    // Not yet.
    assert_eq!(w.sched.run_due(due - 1), 0);
    assert!(w.lifecycle.status().is_none());

    // Due: the restart is requested as the task's creator, the history says ok, and the stored
    // next run is already tomorrow.
    assert_eq!(w.sched.run_due(due), 1);
    let pending = w.lifecycle.status().expect("a restart is pending");
    assert_eq!(
        (
            pending.action,
            pending.delay_secs,
            pending.requested_by.as_str(),
            pending.message.as_deref()
        ),
        (ShutdownKind::Restart, 30, "root", Some("nightly"))
    );
    let row = w.store.task(task.id).unwrap().unwrap();
    assert_eq!(
        (row.next_run_at, row.last_status.as_deref(), row.last_run_at),
        (Some(due + 86_400), Some("ok"), Some(due))
    );
    assert!(row.last_message.unwrap().contains("30 s"));
    assert!(
        w.admin
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.starts_with("root: record scheduled restart")),
        "audited"
    );
    let runs = w.store.runs(task.id, 10).unwrap();
    assert_eq!(
        (runs.len(), runs[0].status.as_str(), runs[0].due_at),
        (1, "ok", due)
    );

    // The same second again does nothing (no double run).
    assert_eq!(w.sched.run_due(due), 0);
    assert_eq!(w.sched.run_due(due + 3600), 0);

    // The countdown ends and the process would be told to restart.
    tokio::time::sleep(std::time::Duration::from_secs(40)).await;
    assert_eq!(w.shutdown.recv().await, Some(ShutdownKind::Restart));
}

#[tokio::test(start_paused = true)]
async fn a_restart_is_skipped_while_another_shutdown_is_pending() {
    let w = world();
    let task = w
        .sched
        .create("root", restart_input("Nightly", daily("04:00")))
        .unwrap();
    w.lifecycle
        .request(w.admin.clone(), "root", ShutdownKind::Stop, 600, None)
        .unwrap();
    let due = task.next_run_at.unwrap();
    assert_eq!(w.sched.run_due(due), 1);
    let row = w.store.task(task.id).unwrap().unwrap();
    assert_eq!(row.last_status.as_deref(), Some("skipped"));
    assert!(row.last_message.unwrap().contains("already pending"));
    assert_eq!(
        w.lifecycle.status().unwrap().action,
        ShutdownKind::Stop,
        "the earlier request stands"
    );
    assert_eq!(row.next_run_at, Some(due + 86_400));
}

#[tokio::test(start_paused = true)]
async fn the_creator_must_still_be_an_admin() {
    let w = world();
    // `bob` is a player: a task of his fails when it runs, nothing restarts.
    let task = w
        .sched
        .create("bob", restart_input("Sneaky", daily("04:00")))
        .unwrap();
    assert_eq!(w.sched.run_due(task.next_run_at.unwrap()), 1);
    let row = w.store.task(task.id).unwrap().unwrap();
    assert_eq!(row.last_status.as_deref(), Some("failed"));
    assert!(row.last_message.unwrap().contains("no longer an admin"));
    assert!(w.lifecycle.status().is_none());
    // A disabled admin too.
    w.accounts.set_disabled("root", true).unwrap();
    let task = w
        .sched
        .create("root", restart_input("Gone", daily("04:00")))
        .unwrap();
    w.sched.run_due(task.next_run_at.unwrap());
    assert_eq!(
        w.store
            .task(task.id)
            .unwrap()
            .unwrap()
            .last_status
            .as_deref(),
        Some("failed")
    );
}

#[tokio::test(start_paused = true)]
async fn disabled_tasks_never_run() {
    let w = world();
    let mut input = restart_input("Off", daily("04:00"));
    input.enabled = false;
    let task = w.sched.create("root", input).unwrap();
    assert_eq!(task.next_run_at, None);
    assert_eq!(w.sched.run_due(i64::MAX / 2), 0);
    assert!(w.store.runs(task.id, 5).unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_run_found_late_is_skipped_not_made_up() {
    let w = world();
    let task = w
        .sched
        .create("root", restart_input("Nightly", daily("04:00")))
        .unwrap();
    let due = task.next_run_at.unwrap();
    // Found 121 s late (grace 120): missed. The restart does not happen; the task moves on.
    assert_eq!(w.sched.run_due(due + 121), 1);
    assert!(w.lifecycle.status().is_none());
    let row = w.store.task(task.id).unwrap().unwrap();
    assert_eq!(
        (row.last_status.as_deref(), row.next_run_at),
        (Some("missed"), Some(due + 86_400))
    );
    // Exactly at the grace it still runs.
    let task = w
        .sched
        .create("root", restart_input("Other", daily("04:00")))
        .unwrap();
    w.sched.run_due(task.next_run_at.unwrap() + 120);
    assert_eq!(
        w.store
            .task(task.id)
            .unwrap()
            .unwrap()
            .last_status
            .as_deref(),
        Some("ok")
    );
}

#[tokio::test(start_paused = true)]
async fn missed_runs_while_down_are_recorded_at_start_and_never_replayed() {
    let w = world();
    let daily_task = w
        .sched
        .create("root", restart_input("Nightly", daily("04:00")))
        .unwrap();
    let every = w
        .sched
        .create(
            "root",
            restart_input("Hourly", Schedule::Interval { every_secs: 3600 }),
        )
        .unwrap();
    let mut off = restart_input("Off", daily("04:00"));
    off.enabled = false;
    let off = w.sched.create("root", off).unwrap();
    // The server is down for two days: now it is 2026-10-24 09:00 local = 07:00 UTC.
    w.clock
        .0
        .store(ts("2026-10-24T07:00:00Z"), Ordering::SeqCst);
    w.sched.reconcile().unwrap();
    let after = w.store.task(daily_task.id).unwrap().unwrap();
    assert_eq!(
        (after.last_status.as_deref(), after.next_run_at),
        (Some("missed"), Some(ts("2026-10-25T02:00:00Z"))),
        "next 04:00 local, not 09:00 today"
    );
    assert_eq!(
        w.store.runs(daily_task.id, 10).unwrap().len(),
        1,
        "one entry, not one per missed day"
    );
    let after = w.store.task(every.id).unwrap().unwrap();
    assert_eq!(after.last_status.as_deref(), Some("missed"));
    assert_eq!(
        after.next_run_at,
        Some(ts("2026-10-24T08:00:00Z")),
        "an interval restarts from now"
    );
    assert!(w.store.runs(off.id, 10).unwrap().is_empty());
    // Nothing runs at the first tick.
    assert_eq!(w.sched.run_due(w.clock.0.load(Ordering::SeqCst)), 0);
    assert!(w.lifecycle.status().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_run_due_just_before_the_start_still_runs_within_the_grace() {
    let w = world();
    let task = w
        .sched
        .create("root", restart_input("Nightly", daily("04:00")))
        .unwrap();
    let due = task.next_run_at.unwrap();
    // The server came back 30 s after the time.
    w.clock.0.store(due + 30, Ordering::SeqCst);
    w.sched.reconcile().unwrap();
    assert_eq!(
        w.store.task(task.id).unwrap().unwrap().next_run_at,
        Some(due),
        "kept"
    );
    assert_eq!(w.sched.run_due(due + 30), 1);
    assert_eq!(
        w.store
            .task(task.id)
            .unwrap()
            .unwrap()
            .last_status
            .as_deref(),
        Some("ok")
    );
}

#[tokio::test(start_paused = true)]
async fn unreadable_tasks_fail_once_and_stop_coming_up() {
    let w = world();
    let task = w
        .sched
        .create("root", restart_input("Nightly", daily("04:00")))
        .unwrap();
    // A row written by a newer version: a kind this build does not know.
    w.store
        .update_task(
            task.id,
            &poe_store::TaskSpec {
                name: "Future".into(),
                kind: "reboot_universe".into(),
                params: "{}".into(),
                schedule: r#"{"type":"daily","time":"04:00"}"#.into(),
                enabled: true,
            },
            0,
            Some(100),
        )
        .unwrap();
    let listed = &w.sched.tasks().unwrap()[0];
    assert!(
        listed
            .decoded
            .as_ref()
            .unwrap_err()
            .contains("reboot_universe")
    );
    assert_eq!(w.sched.run_due(200), 1);
    let row = w.store.task(task.id).unwrap().unwrap();
    assert_eq!(
        (row.last_status.as_deref(), row.next_run_at),
        (Some("failed"), None)
    );
    assert_eq!(w.sched.run_due(300), 0);
    // Turning it on from the page is refused with the reason.
    assert!(
        matches!(w.sched.set_enabled(task.id, true), Err(SchedError::Invalid(m)) if m.contains("reboot_universe"))
    );
}

#[tokio::test(start_paused = true)]
async fn history_is_trimmed_to_the_configured_length() {
    let w = world();
    let task = w
        .sched
        .create(
            "root",
            restart_input("Often", Schedule::Interval { every_secs: 60 }),
        )
        .unwrap();
    let mut now = task.next_run_at.unwrap();
    for _ in 0..8 {
        // Cancel the pending restart so the next run does something other than skip.
        let _ = w.lifecycle.cancel();
        assert_eq!(w.sched.run_due(now), 1);
        now += 60;
    }
    assert_eq!(
        w.store.runs(task.id, 100).unwrap().len(),
        5,
        "history_keep = 5"
    );
}

/// The real loop on tokio's paused clock: the task becomes due while the loop sleeps and the
/// restart is requested without anyone calling `run_due`.
#[tokio::test(start_paused = true)]
async fn the_loop_runs_a_task_when_its_time_comes() {
    struct PausedClock {
        base: i64,
        start: tokio::time::Instant,
    }
    impl Clock for PausedClock {
        fn now(&self) -> i64 {
            self.base + self.start.elapsed().as_secs() as i64
        }
    }
    let base = ts("2026-10-22T01:59:30Z");
    let clock: Arc<dyn Clock> = Arc::new(PausedClock {
        base,
        start: tokio::time::Instant::now(),
    });
    let w = world_with(base, Some(clock));
    let task = w
        .sched
        .create("root", restart_input("Nightly", daily("04:00")))
        .unwrap();
    assert_eq!(task.next_run_at, Some(ts("2026-10-22T02:00:00Z")));
    let handle = tokio::spawn(w.sched.clone().run());
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    assert!(w.lifecycle.status().is_none(), "20 s in: ten seconds to go");
    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    let pending = w
        .lifecycle
        .status()
        .expect("the restart was requested at 04:00 local");
    assert_eq!(pending.action, ShutdownKind::Restart);
    // Each run is recorded; the loop does not run it again.
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    assert_eq!(w.store.runs(task.id, 10).unwrap().len(), 1);
    handle.abort();
}

#[tokio::test(start_paused = true)]
async fn a_scheduler_that_is_off_does_nothing() {
    let mut w = world();
    let config = poe_config::SchedulerConfig {
        enabled: false,
        history_keep: 5,
        missed_grace_secs: 120,
    };
    let off = Arc::new(Scheduler::new(
        w.store.clone(),
        w.admin.clone(),
        w.lifecycle.clone(),
        w.accounts.clone(),
        w.clock.clone(),
        TimeZone::UTC,
        &config,
    ));
    assert!(!off.enabled());
    off.clone().run().await; // returns at once
    assert!(w.shutdown.try_recv().is_err());
}
