//! `/admin/schedule/...`: the scheduled tasks page.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use poe_store::{RunRow, TaskRow};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{SchedError, Scheduler, TaskInfo, TaskInput, kind::TaskKind, schedule::Schedule};
use crate::{ApiError, WebState, auth::AdminAuth, error::ApiJson, public::blocking};

/// Runs `GET .../runs` returns at most.
const RUNS_LIMIT: u32 = 100;

fn default_true() -> bool {
    true
}

/// The body of create and update.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaskBody {
    /// 1 to 80 characters.
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// What the task does (`{"kind":"restart","delay_secs":300,"message":"..."}`).
    pub task: TaskKind,
    /// When it runs.
    pub schedule: Schedule,
}

/// A task as the page shows it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaskView {
    pub id: u32,
    pub name: String,
    pub enabled: bool,
    /// Absent when the stored task cannot be read (`problem` says why).
    pub task: Option<TaskKind>,
    pub schedule: Option<Schedule>,
    /// The task and the schedule in words.
    pub task_text: Option<String>,
    pub schedule_text: Option<String>,
    pub problem: Option<String>,
    pub created_at: i64,
    pub created_by: String,
    pub updated_at: i64,
    /// Unix seconds.
    pub last_run_at: Option<i64>,
    /// `ok`, `failed`, `skipped` or `missed`.
    pub last_status: Option<String>,
    pub last_message: Option<String>,
    /// When it runs next (unix seconds); absent for a disabled task.
    pub next_run_at: Option<i64>,
}

impl From<TaskInfo> for TaskView {
    fn from(info: TaskInfo) -> Self {
        let TaskRow {
            id,
            name,
            enabled,
            created_at,
            created_by,
            updated_at,
            last_run_at,
            last_status,
            last_message,
            next_run_at,
            ..
        } = info.row;
        let (task, schedule, problem) = match info.decoded {
            Ok((t, s)) => (Some(t), Some(s), None),
            Err(e) => (None, None, Some(e)),
        };
        Self {
            id,
            name,
            enabled,
            task_text: task.as_ref().map(TaskKind::describe),
            schedule_text: schedule.as_ref().map(Schedule::describe),
            task,
            schedule,
            problem,
            created_at,
            created_by,
            updated_at,
            last_run_at,
            last_status,
            last_message,
            next_run_at,
        }
    }
}

/// `GET /admin/schedule`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ScheduleOverview {
    /// The zone the schedules are in (`Europe/Berlin`, or `UTC+02:00` when it has no name).
    pub timezone: String,
    /// The zone's offset from UTC now, `+02:00`.
    pub utc_offset: String,
    /// The server's clock, unix seconds.
    pub now: i64,
    /// `[scheduler] enabled`: when off, tasks are listed but never run.
    pub enabled: bool,
    pub tasks: Vec<TaskView>,
}

/// One entry of a task's history.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RunView {
    pub id: u32,
    /// When the run was scheduled for (unix seconds).
    pub due_at: i64,
    pub started_at: i64,
    /// `ok`, `failed`, `skipped` or `missed`.
    pub status: String,
    pub message: String,
}

impl From<RunRow> for RunView {
    fn from(r: RunRow) -> Self {
        Self {
            id: r.id,
            due_at: r.due_at,
            started_at: r.started_at,
            status: r.status,
            message: r.message,
        }
    }
}

/// `POST /admin/schedule/preview`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreviewRequest {
    pub schedule: Schedule,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PreviewResponse {
    /// The schedule in words.
    pub description: String,
    /// The next runs (unix seconds), up to 5.
    pub next_runs: Vec<i64>,
    pub timezone: String,
}

fn sched_error(e: SchedError) -> ApiError {
    match e {
        SchedError::Invalid(m) => ApiError::bad_request(m),
        SchedError::NotFound => ApiError::not_found("no such task"),
        SchedError::Store(e) => ApiError::internal(e),
    }
}

fn input(body: TaskBody) -> TaskInput {
    TaskInput {
        name: body.name,
        enabled: body.enabled,
        task: body.task,
        schedule: body.schedule,
    }
}

async fn run<T: Send + 'static>(
    s: &WebState,
    f: impl FnOnce(&Scheduler) -> Result<T, SchedError> + Send + 'static,
) -> Result<T, ApiError> {
    let sched = s.scheduler.clone();
    blocking(move || f(&sched)).await?.map_err(sched_error)
}

/// The time zone, and every task with its next run and last result.
#[utoipa::path(get, operation_id = "get_schedule", path = "/api/v1/admin/schedule", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = ScheduleOverview), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn overview(
    State(s): State<WebState>,
    _: AdminAuth,
) -> Result<Json<ScheduleOverview>, ApiError> {
    let tasks = run(&s, |sc| sc.tasks()).await?;
    Ok(Json(ScheduleOverview {
        timezone: s.scheduler.timezone_name(),
        utc_offset: s.scheduler.utc_offset(),
        now: s.scheduler.now(),
        enabled: s.scheduler.enabled(),
        tasks: tasks.into_iter().map(Into::into).collect(),
    }))
}

/// Creates a task. `400` names what is wrong with the schedule or the options.
#[utoipa::path(post, operation_id = "create_task", path = "/api/v1/admin/schedule/tasks", tag = "admin", security(("bearer" = [])),
    request_body = TaskBody,
    responses((status = 201, body = TaskView), (status = 400, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn create(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    ApiJson(body): ApiJson<TaskBody>,
) -> Result<(StatusCode, Json<TaskView>), ApiError> {
    let by = who.account.username.clone();
    let row = run(&s, move |sc| sc.create(&by, input(body))).await?;
    let info = s.scheduler.info(row);
    s.admin.record(
        &who.account.username,
        "schedule create",
        &format!("task #{} {:?}", info.row.id, info.row.name),
    );
    Ok((StatusCode::CREATED, Json(info.into())))
}

/// Replaces a task's name, action, schedule and enabled flag.
#[utoipa::path(post, operation_id = "update_task", path = "/api/v1/admin/schedule/tasks/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = u32, Path, description = "task id")), request_body = TaskBody,
    responses((status = 200, body = TaskView), (status = 400, body = crate::ErrorBody), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn update(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(id): Path<u32>,
    ApiJson(body): ApiJson<TaskBody>,
) -> Result<Json<TaskView>, ApiError> {
    let row = run(&s, move |sc| sc.update(id, input(body))).await?;
    let info = s.scheduler.info(row);
    s.admin.record(
        &who.account.username,
        "schedule update",
        &format!("task #{id} {:?}", info.row.name),
    );
    Ok(Json(info.into()))
}

async fn toggle(
    s: WebState,
    who: String,
    id: u32,
    enabled: bool,
) -> Result<Json<TaskView>, ApiError> {
    let row = run(&s, move |sc| sc.set_enabled(id, enabled)).await?;
    let info = s.scheduler.info(row);
    s.admin.record(
        &who,
        if enabled {
            "schedule enable"
        } else {
            "schedule disable"
        },
        &format!("task #{id} {:?}", info.row.name),
    );
    Ok(Json(info.into()))
}

/// Turns a task on; its next run is worked out from now.
#[utoipa::path(post, operation_id = "enable_task", path = "/api/v1/admin/schedule/tasks/{id}/enable", tag = "admin", security(("bearer" = [])),
    params(("id" = u32, Path, description = "task id")),
    responses((status = 200, body = TaskView), (status = 400, description = "the stored task cannot be read", body = crate::ErrorBody), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn enable(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(id): Path<u32>,
) -> Result<Json<TaskView>, ApiError> {
    toggle(s, who.account.username, id, true).await
}

/// Turns a task off; it keeps its history.
#[utoipa::path(post, operation_id = "disable_task", path = "/api/v1/admin/schedule/tasks/{id}/disable", tag = "admin", security(("bearer" = [])),
    params(("id" = u32, Path, description = "task id")),
    responses((status = 200, body = TaskView), (status = 400, body = crate::ErrorBody), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn disable(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(id): Path<u32>,
) -> Result<Json<TaskView>, ApiError> {
    toggle(s, who.account.username, id, false).await
}

/// Deletes a task and its history.
#[utoipa::path(post, operation_id = "delete_task", path = "/api/v1/admin/schedule/tasks/{id}/delete", tag = "admin", security(("bearer" = [])),
    params(("id" = u32, Path, description = "task id")),
    responses((status = 204), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn delete(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(id): Path<u32>,
) -> Result<StatusCode, ApiError> {
    run(&s, move |sc| sc.delete(id)).await?;
    s.admin.record(
        &who.account.username,
        "schedule delete",
        &format!("task #{id}"),
    );
    Ok(StatusCode::NO_CONTENT)
}

/// A task's history, newest first (at most 100).
#[utoipa::path(get, operation_id = "list_task_runs", path = "/api/v1/admin/schedule/tasks/{id}/runs", tag = "admin", security(("bearer" = [])),
    params(("id" = u32, Path, description = "task id")),
    responses((status = 200, body = Vec<RunView>), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn runs(
    State(s): State<WebState>,
    _: AdminAuth,
    Path(id): Path<u32>,
) -> Result<Json<Vec<RunView>>, ApiError> {
    let rows = run(&s, move |sc| {
        sc.store().task(id)?.ok_or(SchedError::NotFound)?;
        Ok(sc.store().runs(id, RUNS_LIMIT)?)
    })
    .await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

/// Checks a schedule and lists its next runs, for the form. `400` names the problem.
#[utoipa::path(post, operation_id = "preview_schedule", path = "/api/v1/admin/schedule/preview", tag = "admin", security(("bearer" = [])),
    request_body = PreviewRequest,
    responses((status = 200, body = PreviewResponse), (status = 400, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn preview(
    State(s): State<WebState>,
    _: AdminAuth,
    ApiJson(req): ApiJson<PreviewRequest>,
) -> Result<Json<PreviewResponse>, ApiError> {
    let schedule = req.schedule.normalized();
    schedule.validate().map_err(ApiError::bad_request)?;
    let now = s.scheduler.now();
    Ok(Json(PreviewResponse {
        description: schedule.describe(),
        next_runs: schedule.upcoming(now, None, s.scheduler.tz(), 5),
        timezone: s.scheduler.timezone_name(),
    }))
}
