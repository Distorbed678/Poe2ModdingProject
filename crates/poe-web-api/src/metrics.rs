//! `GET /admin/metrics`: the samples behind the dashboard charts (`poe_cluster::metrics`).

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{ApiError, WebState, auth::AdminAuth};

/// Points returned when `max_points` is not given.
pub const DEFAULT_POINTS: usize = 600;
/// The most points a request may ask for.
pub const MAX_POINTS: usize = 5000;

/// Query of `GET /admin/metrics`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct MetricsQuery {
    /// Only samples after this time (unix milliseconds): to fetch what is new since the last
    /// call. Overrides `range_secs`.
    pub since: Option<i64>,
    /// Only the last this many seconds (default: everything kept).
    pub range_secs: Option<u64>,
    /// At most this many points (default 600, at most 5000): longer ranges are merged into
    /// buckets that keep the peak of players, instances, memory and frame time.
    pub max_points: Option<usize>,
}

/// One sample.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MetricsPoint {
    /// Unix milliseconds.
    pub t: i64,
    /// Players in game.
    pub players: u32,
    /// Instances that exist.
    pub instances: u32,
    /// Instances with at least one player.
    pub occupied: u32,
    /// Simulation frames in the interval, and their mean and longest time in microseconds.
    pub tick_count: u32,
    pub tick_avg_us: u32,
    pub tick_max_us: u32,
    /// Resident memory of the server process in bytes (0: unknown on this platform).
    pub rss_bytes: u64,
    /// CPU use of the server process, percent of one core; absent for the first sample and on
    /// platforms where it is unknown.
    pub cpu_pct: Option<f32>,
}

impl From<poe_cluster::Sample> for MetricsPoint {
    fn from(s: poe_cluster::Sample) -> Self {
        Self {
            t: s.t_ms,
            players: s.players,
            instances: s.instances,
            occupied: s.occupied,
            tick_count: s.tick_count,
            tick_avg_us: s.tick_avg_us,
            tick_max_us: s.tick_max_us,
            rss_bytes: s.rss_bytes,
            cpu_pct: s.cpu_pct,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MetricsResponse {
    /// `[metrics] enabled`; when off there are no samples.
    pub enabled: bool,
    /// Seconds between samples.
    pub interval_secs: u64,
    /// Seconds of history kept.
    pub retention_secs: u64,
    /// The server's clock, unix milliseconds.
    pub now: i64,
    /// Oldest first.
    pub samples: Vec<MetricsPoint>,
}

/// Players, instances, simulation frame time, memory and CPU of the server over time, sampled
/// every `[metrics] interval_secs` and kept in memory for `[metrics] retention_hours`.
#[utoipa::path(get, operation_id = "get_metrics", path = "/api/v1/admin/metrics", tag = "admin", security(("bearer" = [])),
    params(MetricsQuery),
    responses((status = 200, body = MetricsResponse), (status = 400, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn get(
    State(s): State<WebState>,
    _: AdminAuth,
    Query(q): Query<MetricsQuery>,
) -> Result<Json<MetricsResponse>, ApiError> {
    let max_points = q.max_points.unwrap_or(DEFAULT_POINTS);
    if !(2..=MAX_POINTS).contains(&max_points) {
        return Err(ApiError::bad_request(format!(
            "max_points is between 2 and {MAX_POINTS}"
        )));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let since = match (q.since, q.range_secs) {
        (Some(t), _) => Some(t),
        (None, Some(r)) => {
            Some(now.saturating_sub(i64::try_from(r).unwrap_or(i64::MAX).saturating_mul(1000)))
        }
        (None, None) => None,
    };
    let enabled = s.config.metrics.enabled;
    let samples = if enabled {
        s.metrics.query(since, max_points)
    } else {
        Vec::new()
    };
    Ok(Json(MetricsResponse {
        enabled,
        interval_secs: s.metrics.interval_secs(),
        retention_secs: s.metrics.retention_secs(),
        now,
        samples: samples.into_iter().map(Into::into).collect(),
    }))
}
