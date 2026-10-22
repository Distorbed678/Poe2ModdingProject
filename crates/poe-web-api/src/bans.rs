//! Bans: `GET /admin/bans`, `POST /admin/bans`, `POST /admin/bans/{id}/lift`.
//!
//! An account ban refuses the account's logins (game and web) with the ban's own text, revokes
//! its tokens and ends its sessions at once. An IP ban refuses logins from one address. Both can
//! expire; lifting one early keeps the row as history. Admin accounts cannot be banned (demote
//! them first), nobody can ban themselves, and an admin cannot ban the address they are using.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use poe_accounts::{BanKind, BanRecord, Role};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    ApiError, WebState,
    auth::{AdminAuth, ClientIp},
    error::ApiJson,
    public::blocking,
};

/// Shortest and longest ban given as a duration.
pub const MIN_DURATION_SECS: u64 = 60;
pub const MAX_DURATION_SECS: u64 = 10 * 365 * 86_400;
/// Rows `GET /admin/bans` returns at most.
const LIST_LIMIT: usize = 1000;

/// A ban, active or not.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BanView {
    pub id: u32,
    /// `account` or `ip`.
    pub kind: String,
    /// The banned account's name (account bans).
    pub account: Option<String>,
    pub account_id: Option<u32>,
    /// The banned address (IP bans).
    pub ip: Option<String>,
    pub reason: String,
    pub created_by: String,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds; absent: permanent.
    pub expires_at: Option<i64>,
    pub lifted_at: Option<i64>,
    pub lifted_by: Option<String>,
    /// `active`, `expired` or `lifted`.
    pub status: String,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl BanView {
    fn new(b: BanRecord, now: i64) -> Self {
        Self {
            status: b.status(now).into(),
            id: b.id,
            kind: b.kind.as_str().into(),
            account: b.account,
            account_id: b.account_id,
            ip: b.ip,
            reason: b.reason,
            created_by: b.created_by,
            created_at: b.created_at,
            expires_at: b.expires_at,
            lifted_at: b.lifted_at,
            lifted_by: b.lifted_by,
        }
    }
}

/// `POST /admin/bans`: give `account` or `ip`, and optionally how long.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BanRequest {
    /// Account name to ban.
    pub account: Option<String>,
    /// IPv4 or IPv6 address to ban.
    pub ip: Option<String>,
    /// Shown to the player (at most 200 characters).
    pub reason: Option<String>,
    /// Ban length in seconds (60 to 10 years). Neither this nor `expires_at`: permanent.
    pub duration_secs: Option<u64>,
    /// End of the ban, unix seconds in the future (instead of `duration_secs`).
    pub expires_at: Option<i64>,
}

/// Query of `GET /admin/bans`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct BanQuery {
    /// Only the bans in force now.
    pub active: Option<bool>,
}

/// The bans, newest first (history included unless `?active=true`).
#[utoipa::path(get, operation_id = "list_bans", path = "/api/v1/admin/bans", tag = "admin", security(("bearer" = [])),
    params(BanQuery),
    responses((status = 200, body = Vec<BanView>), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn list(
    State(s): State<WebState>,
    _: AdminAuth,
    Query(q): Query<BanQuery>,
) -> Result<Json<Vec<BanView>>, ApiError> {
    let accounts = s.accounts.clone();
    let active = q.active.unwrap_or(false);
    let rows = blocking(move || accounts.bans(active)).await??;
    let now = now();
    Ok(Json(
        rows.into_iter()
            .take(LIST_LIMIT)
            .map(|b| BanView::new(b, now))
            .collect(),
    ))
}

/// Bans an account (its logins are refused, its tokens revoked and its sessions ended) or an IP
/// address. `400` for an admin, yourself, your own address, a bad address, reason or duration;
/// `404` for an unknown account; `409` when the target already has an active ban.
#[utoipa::path(post, operation_id = "create_ban", path = "/api/v1/admin/bans", tag = "admin", security(("bearer" = [])),
    request_body = BanRequest,
    responses((status = 201, body = BanView), (status = 400, body = crate::ErrorBody), (status = 404, body = crate::ErrorBody),
        (status = 409, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn create(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    ClientIp(caller_ip): ClientIp,
    ApiJson(req): ApiJson<BanRequest>,
) -> Result<(StatusCode, Json<BanView>), ApiError> {
    let now = now();
    let reason =
        poe_accounts::clean_reason(req.reason.as_deref().unwrap_or_default()).ok_or_else(|| {
            ApiError::bad_request(format!(
                "the reason is at most {} characters",
                poe_accounts::REASON_MAX
            ))
        })?;
    let expires_at = match (req.duration_secs, req.expires_at) {
        (Some(_), Some(_)) => {
            return Err(ApiError::bad_request(
                "give duration_secs or expires_at, not both",
            ));
        }
        (Some(d), None) => {
            if !(MIN_DURATION_SECS..=MAX_DURATION_SECS).contains(&d) {
                return Err(ApiError::bad_request(
                    "a ban lasts from 1 minute to 10 years",
                ));
            }
            Some(now + d as i64)
        }
        (None, Some(t)) if t <= now => {
            return Err(ApiError::bad_request("expires_at must be in the future"));
        }
        (None, Some(t)) => Some(t),
        (None, None) => None,
    };
    let by = who.account.username.clone();
    let accounts = s.accounts.clone();
    let (ban, ended) = match (req.account, req.ip) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(ApiError::bad_request("give either account or ip"));
        }
        (Some(name), None) => {
            let target = blocking({
                let accounts = accounts.clone();
                move || accounts.find(&name)
            })
            .await??
            .ok_or_else(|| ApiError::not_found("no such account"))?;
            if target.id == who.account.id {
                return Err(ApiError::bad_request("you cannot ban yourself"));
            }
            if target.role == Role::Admin {
                return Err(ApiError::bad_request(
                    "an admin cannot be banned: take the admin role away first",
                ));
            }
            let id = target.id;
            let ban = blocking(move || accounts.ban_account(id, &reason, &by, expires_at))
                .await?
                .map_err(ApiError::from)?;
            // Login connections and game sessions end now, not at the next login.
            s.admin.end_sessions(id);
            (ban, Some(target.username))
        }
        (None, Some(text)) => {
            let ip = poe_accounts::parse_ip(&text)
                .ok_or_else(|| ApiError::bad_request("not an IP address"))?;
            if ip == poe_accounts::canonical_ip(caller_ip) {
                return Err(ApiError::bad_request(
                    "that is the address you are using: banning it would lock you out",
                ));
            }
            let ban = blocking(move || accounts.ban_ip(&ip, &reason, &by, expires_at))
                .await?
                .map_err(ApiError::from)?;
            (ban, None)
        }
    };
    let target = ended.or_else(|| ban.ip.clone()).unwrap_or_default();
    let until = ban.expires_at.map_or("permanently".to_owned(), |t| {
        format!("until {}", poe_accounts::format_utc(t))
    });
    s.admin.record(
        &who.account.username,
        &format!("ban {}", ban.kind.as_str()),
        &format!(
            "{target} {until}{}",
            if ban.reason.is_empty() {
                String::new()
            } else {
                format!(": {}", ban.reason)
            }
        ),
    );
    Ok((StatusCode::CREATED, Json(BanView::new(ban, now))))
}

/// Lifts a ban that is in force. `404` unknown ban, `409` already over.
#[utoipa::path(post, operation_id = "lift_ban", path = "/api/v1/admin/bans/{id}/lift", tag = "admin", security(("bearer" = [])),
    params(("id" = u32, Path, description = "ban id")),
    responses((status = 200, body = BanView), (status = 404, body = crate::ErrorBody), (status = 409, body = crate::ErrorBody),
        (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn lift(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(id): Path<u32>,
) -> Result<Json<BanView>, ApiError> {
    let accounts = s.accounts.clone();
    let by = who.account.username.clone();
    let ban = blocking(move || accounts.lift_ban(id, &by))
        .await?
        .map_err(ApiError::from)?;
    let target = match ban.kind {
        BanKind::Account => ban.account.clone().unwrap_or_default(),
        BanKind::Ip => ban.ip.clone().unwrap_or_default(),
    };
    s.admin.record(
        &who.account.username,
        &format!("unban {}", ban.kind.as_str()),
        &target,
    );
    Ok(Json(BanView::new(ban, now())))
}
