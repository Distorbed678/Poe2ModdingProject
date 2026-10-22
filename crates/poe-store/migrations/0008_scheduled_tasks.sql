-- Scheduled tasks (web admin, Schedule page; the scheduler in crates/poe-web-api). The store
-- keeps `params` and `schedule` as JSON text and does not interpret them: the task kind (an
-- enum in the scheduler) owns the meaning of `kind` and `params`. Times are unix seconds.
CREATE TABLE scheduled_tasks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL,
    kind         TEXT NOT NULL,
    params       TEXT NOT NULL DEFAULT '{}',
    schedule     TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   INTEGER NOT NULL,
    created_by   TEXT NOT NULL,
    updated_at   INTEGER NOT NULL,
    last_run_at  INTEGER,
    -- ok, failed, skipped (not run on purpose, e.g. a shutdown was pending) or missed (due while
    -- the server was down or frozen).
    last_status  TEXT,
    last_message TEXT,
    next_run_at  INTEGER
);

CREATE TABLE scheduled_task_runs (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id    INTEGER NOT NULL REFERENCES scheduled_tasks (id) ON DELETE CASCADE,
    due_at     INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    status     TEXT NOT NULL CHECK (status IN ('ok', 'failed', 'skipped', 'missed')),
    message    TEXT NOT NULL DEFAULT ''
);
CREATE INDEX scheduled_task_runs_task ON scheduled_task_runs (task_id, id);
