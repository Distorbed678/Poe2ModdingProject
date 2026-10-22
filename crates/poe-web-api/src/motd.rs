//! The message of the day: `[server] motd`, held live so the web admin can change it without a
//! restart, shown in `GET /server` and written back into the config file.

use std::sync::RwLock;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{ApiError, WebState, auth::AdminAuth, configfile};

/// The live message of the day.
#[derive(Debug, Default)]
pub struct Motd(RwLock<String>);

impl Motd {
    pub fn new(initial: &str) -> Self {
        Self(RwLock::new(initial.to_owned()))
    }

    pub fn get(&self) -> String {
        self.0.read().map(|m| m.clone()).unwrap_or_default()
    }

    pub fn set(&self, text: &str) {
        if let Ok(mut m) = self.0.write() {
            *m = text.to_owned();
        }
    }
}

/// `POST /admin/motd`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MotdRequest {
    /// Plain text, at most 280 characters, line breaks allowed. Empty clears it.
    pub motd: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MotdResponse {
    /// The message now in effect (trimmed).
    pub motd: String,
    /// Whether it was also written into the config file, so it survives a restart.
    pub persisted: bool,
    /// Why it was not written (config editor off, no file, read-only file, ...).
    pub note: Option<String>,
}

/// Sets the message of the day. It takes effect at once (`GET /server`, the launcher's Home
/// page) and is written into the config file the server runs from (with a backup, like the
/// config editor); when that is not possible the message still applies until a restart and
/// `persisted` is false.
#[utoipa::path(post, operation_id = "set_motd", path = "/api/v1/admin/motd", tag = "admin", security(("bearer" = [])),
    request_body = MotdRequest,
    responses((status = 200, body = MotdResponse), (status = 400, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn set(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    crate::error::ApiJson(req): crate::error::ApiJson<MotdRequest>,
) -> Result<Json<MotdResponse>, ApiError> {
    let motd = poe_config::check_motd(&req.motd).map_err(ApiError::bad_request)?;
    s.motd.set(&motd);
    let outcome = configfile::persist_fields(
        &s,
        &who.account.username,
        vec![(
            "server.motd".into(),
            poe_config::edit::EditValue::Text(motd.clone()),
        )],
    )
    .await;
    let (persisted, note) = match outcome {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.message)),
    };
    s.admin.record(
        &who.account.username,
        "motd",
        &format!(
            "{} characters, saved to the config file: {}",
            motd.chars().count(),
            if persisted { "yes" } else { "no" }
        ),
    );
    Ok(Json(MotdResponse {
        motd,
        persisted,
        note,
    }))
}
