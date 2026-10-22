//! Rows of the scheduler (`scheduled_tasks`, `scheduled_task_runs`, migration 0008).
//!
//! The store does not interpret a task: `kind`, `params` and `schedule` are opaque text (JSON)
//! owned by the scheduler in `crates/poe-web-api`.

use rusqlite::{OptionalExtension, Row, params};

use crate::{Result, SqliteStore};

/// A scheduled task as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub id: u32,
    pub name: String,
    /// The kind's tag (`restart`).
    pub kind: String,
    /// JSON object with the kind's options.
    pub params: String,
    /// JSON of the schedule.
    pub schedule: String,
    pub enabled: bool,
    pub created_at: i64,
    pub created_by: String,
    pub updated_at: i64,
    pub last_run_at: Option<i64>,
    /// `ok`, `failed`, `skipped` or `missed`.
    pub last_status: Option<String>,
    pub last_message: Option<String>,
    /// When the scheduler will run it next (unix seconds); `None` for a disabled task.
    pub next_run_at: Option<i64>,
}

/// The editable part of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSpec {
    pub name: String,
    pub kind: String,
    pub params: String,
    pub schedule: String,
    pub enabled: bool,
}

/// One entry of a task's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub id: u32,
    pub task_id: u32,
    /// The moment the run was scheduled for.
    pub due_at: i64,
    pub started_at: i64,
    /// `ok`, `failed`, `skipped` or `missed`.
    pub status: String,
    pub message: String,
}

/// What `record_run` stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun {
    pub due_at: i64,
    pub started_at: i64,
    pub status: String,
    pub message: String,
}

/// Longest stored message of a run (characters).
pub const RUN_MESSAGE_MAX: usize = 500;

pub trait TaskStore: Send + Sync {
    /// All tasks, by id.
    fn tasks(&self) -> Result<Vec<TaskRow>>;
    fn task(&self, id: u32) -> Result<Option<TaskRow>>;
    fn create_task(
        &self,
        spec: &TaskSpec,
        by: &str,
        now: i64,
        next_run_at: Option<i64>,
    ) -> Result<TaskRow>;
    /// Replaces the editable part; `None` if the task does not exist.
    fn update_task(
        &self,
        id: u32,
        spec: &TaskSpec,
        now: i64,
        next_run_at: Option<i64>,
    ) -> Result<Option<TaskRow>>;
    /// Sets only the next run (the scheduler, after a run or at start).
    fn set_next_run(&self, id: u32, next_run_at: Option<i64>) -> Result<()>;
    /// `false` if the task does not exist. Its history goes with it.
    fn delete_task(&self, id: u32) -> Result<bool>;
    /// Appends a run, copies it into the task's `last_*` columns and keeps only the newest
    /// `keep` runs of the task.
    fn record_run(&self, id: u32, run: &NewRun, keep: u32) -> Result<()>;
    /// The task's runs, newest first.
    fn runs(&self, id: u32, limit: u32) -> Result<Vec<RunRow>>;
}

const COLUMNS: &str = "id, name, kind, params, schedule, enabled, created_at, created_by, updated_at, last_run_at, last_status, last_message, next_run_at";

fn task_row(r: &Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        params: r.get(3)?,
        schedule: r.get(4)?,
        enabled: r.get(5)?,
        created_at: r.get(6)?,
        created_by: r.get(7)?,
        updated_at: r.get(8)?,
        last_run_at: r.get(9)?,
        last_status: r.get(10)?,
        last_message: r.get(11)?,
        next_run_at: r.get(12)?,
    })
}

fn clip(text: &str) -> String {
    text.chars().take(RUN_MESSAGE_MAX).collect()
}

impl TaskStore for SqliteStore {
    fn tasks(&self) -> Result<Vec<TaskRow>> {
        Ok(self.with_connection(|c| -> rusqlite::Result<_> {
            let mut st = c.prepare(&format!(
                "SELECT {COLUMNS} FROM scheduled_tasks ORDER BY id"
            ))?;
            st.query_map([], task_row)?.collect()
        })?)
    }

    fn task(&self, id: u32) -> Result<Option<TaskRow>> {
        Ok(self.with_connection(|c| {
            c.query_row(
                &format!("SELECT {COLUMNS} FROM scheduled_tasks WHERE id = ?1"),
                params![id],
                task_row,
            )
            .optional()
        })?)
    }

    fn create_task(
        &self,
        spec: &TaskSpec,
        by: &str,
        now: i64,
        next_run_at: Option<i64>,
    ) -> Result<TaskRow> {
        let id = self.with_connection(|c| -> rusqlite::Result<i64> {
            c.execute(
                "INSERT INTO scheduled_tasks (name, kind, params, schedule, enabled, created_at, created_by, updated_at, next_run_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?6, ?8)",
                params![spec.name, spec.kind, spec.params, spec.schedule, spec.enabled, now, by, next_run_at],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        Ok(self.task(id as u32)?.expect("the row was just inserted"))
    }

    fn update_task(
        &self,
        id: u32,
        spec: &TaskSpec,
        now: i64,
        next_run_at: Option<i64>,
    ) -> Result<Option<TaskRow>> {
        let n = self.with_connection(|c| {
            c.execute(
                "UPDATE scheduled_tasks SET name = ?2, kind = ?3, params = ?4, schedule = ?5, enabled = ?6,
                 updated_at = ?7, next_run_at = ?8 WHERE id = ?1",
                params![id, spec.name, spec.kind, spec.params, spec.schedule, spec.enabled, now, next_run_at],
            )
        })?;
        if n == 0 { Ok(None) } else { self.task(id) }
    }

    fn set_next_run(&self, id: u32, next_run_at: Option<i64>) -> Result<()> {
        self.with_connection(|c| {
            c.execute(
                "UPDATE scheduled_tasks SET next_run_at = ?2 WHERE id = ?1",
                params![id, next_run_at],
            )
        })?;
        Ok(())
    }

    fn delete_task(&self, id: u32) -> Result<bool> {
        Ok(self.with_connection(|c| {
            c.execute("DELETE FROM scheduled_tasks WHERE id = ?1", params![id])
        })? > 0)
    }

    fn record_run(&self, id: u32, run: &NewRun, keep: u32) -> Result<()> {
        let message = clip(&run.message);
        self.with_connection(|c| -> rusqlite::Result<()> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO scheduled_task_runs (task_id, due_at, started_at, status, message) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, run.due_at, run.started_at, run.status, message],
            )?;
            tx.execute(
                "UPDATE scheduled_tasks SET last_run_at = ?2, last_status = ?3, last_message = ?4 WHERE id = ?1",
                params![id, run.started_at, run.status, message],
            )?;
            tx.execute(
                "DELETE FROM scheduled_task_runs WHERE task_id = ?1 AND id NOT IN
                 (SELECT id FROM scheduled_task_runs WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2)",
                params![id, keep],
            )?;
            tx.commit()
        })?;
        Ok(())
    }

    fn runs(&self, id: u32, limit: u32) -> Result<Vec<RunRow>> {
        Ok(self.with_connection(|c| -> rusqlite::Result<_> {
            let mut st = c.prepare(
                "SELECT id, task_id, due_at, started_at, status, message FROM scheduled_task_runs
                 WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            st.query_map(params![id, limit], |r| {
                Ok(RunRow {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    due_at: r.get(2)?,
                    started_at: r.get(3)?,
                    status: r.get(4)?,
                    message: r.get(5)?,
                })
            })?
            .collect()
        })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> TaskSpec {
        TaskSpec {
            name: name.into(),
            kind: "restart".into(),
            params: r#"{"delay_secs":60}"#.into(),
            schedule: r#"{"type":"daily","time":"04:00"}"#.into(),
            enabled: true,
        }
    }

    fn run(n: i64, status: &str) -> NewRun {
        NewRun {
            due_at: n,
            started_at: n + 1,
            status: status.into(),
            message: format!("run {n}"),
        }
    }

    #[test]
    fn tasks_are_created_updated_and_deleted() {
        let s = SqliteStore::in_memory().unwrap();
        assert!(s.tasks().unwrap().is_empty());
        let t = s
            .create_task(&spec("nightly"), "root", 1000, Some(5000))
            .unwrap();
        assert_eq!(
            (t.id, t.name.as_str(), t.created_by.as_str(), t.next_run_at),
            (1, "nightly", "root", Some(5000))
        );
        assert!(t.enabled && t.last_run_at.is_none() && t.updated_at == 1000);
        let mut changed = spec("nightly 2");
        changed.enabled = false;
        let u = s.update_task(t.id, &changed, 2000, None).unwrap().unwrap();
        assert_eq!(
            (
                u.name.as_str(),
                u.enabled,
                u.next_run_at,
                u.updated_at,
                u.created_at
            ),
            ("nightly 2", false, None, 2000, 1000)
        );
        assert!(s.update_task(99, &changed, 2000, None).unwrap().is_none());
        s.set_next_run(t.id, Some(9000)).unwrap();
        assert_eq!(s.task(t.id).unwrap().unwrap().next_run_at, Some(9000));
        assert_eq!(s.tasks().unwrap().len(), 1);
        assert!(s.delete_task(t.id).unwrap());
        assert!(!s.delete_task(t.id).unwrap());
        assert!(s.task(t.id).unwrap().is_none());
    }

    #[test]
    fn runs_update_the_task_and_are_trimmed() {
        let s = SqliteStore::in_memory().unwrap();
        let a = s.create_task(&spec("a"), "root", 0, None).unwrap();
        let b = s.create_task(&spec("b"), "root", 0, None).unwrap();
        for n in 0..5 {
            s.record_run(a.id, &run(n * 100, "ok"), 3).unwrap();
        }
        s.record_run(b.id, &run(7, "missed"), 3).unwrap();
        let runs = s.runs(a.id, 10).unwrap();
        // Newest first, only the newest 3 kept, the other task untouched.
        assert_eq!(
            runs.iter().map(|r| r.due_at).collect::<Vec<_>>(),
            vec![400, 300, 200]
        );
        assert_eq!(s.runs(b.id, 10).unwrap().len(), 1);
        let t = s.task(a.id).unwrap().unwrap();
        assert_eq!(
            (
                t.last_run_at,
                t.last_status.as_deref(),
                t.last_message.as_deref()
            ),
            (Some(401), Some("ok"), Some("run 400"))
        );
        assert_eq!(s.runs(a.id, 2).unwrap().len(), 2);
        // Long messages are clipped; bad statuses are refused by the table.
        let long = NewRun {
            message: "x".repeat(RUN_MESSAGE_MAX * 2),
            ..run(1, "failed")
        };
        s.record_run(a.id, &long, 3).unwrap();
        assert_eq!(
            s.runs(a.id, 1).unwrap()[0].message.chars().count(),
            RUN_MESSAGE_MAX
        );
        assert!(s.record_run(a.id, &run(1, "weird"), 3).is_err());
        // Deleting a task deletes its history.
        s.delete_task(a.id).unwrap();
        assert!(s.runs(a.id, 10).unwrap().is_empty());
    }
}
