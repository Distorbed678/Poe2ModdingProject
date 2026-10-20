//! Public endpoints (`/server`, `/register`, `/login`) and the ones any logged-in account may
//! call (`/password`, `/logout`).
//!
//! Passwords arrive as plain text. They are turned into the client's digest here
//! (`poe_accounts::password_digest`, SHA-256 of the UTF-16LE text) and then handled by
//! `AccountStore` exactly like the game login, so a password works on the web and in game.

use axum::{Json, extract::State, http::StatusCode};
use poe_accounts::{AccountError, MIN_PASSWORD_LEN, Role, TokenKind, password_digest};

use crate::{
    ApiError, WEB_TOKEN_TTL_SECS, WebState,
    auth::Auth,
    dto::{AccountInfo, LoginRequest, LoginResponse, PasswordRequest, RegisterRequest, ServerInfo},
    error::ApiJson,
};

/// Longest accepted password (guards the hashing cost).
const MAX_PASSWORD_LEN: usize = 256;

/// Runs blocking work (argon2, SQLite) off the async threads.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(ApiError::internal)
}

fn check_password(password: &str) -> Result<(), ApiError> {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err(AccountError::WeakPassword.into());
    }
    if len > MAX_PASSWORD_LEN {
        return Err(ApiError::bad_request(format!(
            "password must be at most {MAX_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

fn ttl(state: &WebState, kind: TokenKind) -> i64 {
    match kind {
        TokenKind::Launcher => i64::from(state.config.accounts.remember_days) * 86_400,
        _ => WEB_TOKEN_TTL_SECS,
    }
}

async fn issue(
    state: &WebState,
    account: poe_accounts::AccountRecord,
    kind: TokenKind,
) -> Result<LoginResponse, ApiError> {
    let accounts = state.accounts.clone();
    let ttl = ttl(state, kind);
    let id = account.id;
    let issued = blocking(move || accounts.issue_token(id, kind, ttl)).await??;
    Ok(LoginResponse {
        token: hex::encode(issued.token),
        kind: kind.as_str().into(),
        expires_at: issued.expires_at,
        account: account.into(),
    })
}

/// Server name, version, cluster, players online and where the game client connects.
#[utoipa::path(get, path = "/api/v1/server", tag = "public",
    responses((status = 200, body = ServerInfo)))]
pub async fn server(State(s): State<WebState>) -> Json<ServerInfo> {
    let c = &s.config;
    Json(ServerInfo {
        name: c.gateway.display_name.clone(),
        version: c.game.version.clone(),
        cluster_id: c.cluster.id.clone(),
        online: s.admin.online().len(),
        open_registration: c.accounts.open_registration,
        login_host: c.network.public_host.clone(),
        login_port: c.network.login.bind.port(),
        uptime_secs: s.started.elapsed().as_secs(),
    })
}

/// Creates a player account (only when `[accounts] open_registration` is on).
#[utoipa::path(post, path = "/api/v1/register", tag = "public", request_body = RegisterRequest,
    responses((status = 201, body = AccountInfo), (status = 400, body = crate::ErrorBody),
        (status = 403, description = "registration is closed", body = crate::ErrorBody),
        (status = 409, description = "name taken", body = crate::ErrorBody)))]
pub async fn register(
    State(s): State<WebState>,
    ApiJson(req): ApiJson<RegisterRequest>,
) -> Result<(StatusCode, Json<AccountInfo>), ApiError> {
    if !s.config.accounts.open_registration {
        return Err(ApiError::forbidden("registration is closed"));
    }
    check_password(&req.password)?;
    let accounts = s.accounts.clone();
    let account = blocking(move || {
        accounts.create_account(&req.username, &password_digest(&req.password), Role::Player)
    })
    .await??;
    tracing::info!(account = %account.username, "registered through the web api");
    Ok((StatusCode::CREATED, Json(account.into())))
}

/// Username and password for a token (`kind` `web` by default, or `launcher`).
#[utoipa::path(post, path = "/api/v1/login", tag = "public", request_body = LoginRequest,
    responses((status = 200, body = LoginResponse),
        (status = 401, description = "wrong username or password", body = crate::ErrorBody),
        (status = 403, description = "account is disabled", body = crate::ErrorBody)))]
pub async fn login(
    State(s): State<WebState>,
    ApiJson(req): ApiJson<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let kind = match req.kind.as_deref() {
        None | Some("web") => TokenKind::Web,
        Some("launcher") => TokenKind::Launcher,
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "unknown token kind {other:?} (web or launcher)"
            )));
        }
    };
    let accounts = s.accounts.clone();
    let result =
        blocking(move || accounts.authenticate(&req.username, &password_digest(&req.password)))
            .await?;
    let account = match result {
        Ok(a) => a,
        // One answer for both, so names cannot be probed.
        Err(AccountError::NoAccount | AccountError::WrongPassword) => {
            return Err(ApiError::unauthorized("invalid username or password"));
        }
        Err(e) => return Err(e.into()),
    };
    Ok(Json(issue(&s, account, kind).await?))
}

/// Changes the caller's password. Revokes all of the account's tokens (game logins included)
/// and returns a fresh token of the same kind as the one used.
#[utoipa::path(post, path = "/api/v1/password", tag = "account", request_body = PasswordRequest,
    security(("bearer" = [])),
    responses((status = 200, body = LoginResponse), (status = 400, body = crate::ErrorBody),
        (status = 401, body = crate::ErrorBody),
        (status = 403, description = "wrong current password", body = crate::ErrorBody)))]
pub async fn password(
    State(s): State<WebState>,
    Auth(who): Auth,
    ApiJson(req): ApiJson<PasswordRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    check_password(&req.new_password)?;
    let accounts = s.accounts.clone();
    let name = who.account.username.clone();
    let result = blocking(move || {
        accounts.authenticate(&name, &password_digest(&req.old_password))?;
        accounts.set_password(&name, &password_digest(&req.new_password))
    })
    .await?;
    let account = match result {
        Ok(a) => a,
        Err(AccountError::WrongPassword) => {
            return Err(ApiError::forbidden("wrong current password"));
        }
        Err(e) => return Err(e.into()),
    };
    tracing::info!(account = %account.username, "password changed through the web api");
    Ok(Json(issue(&s, account, who.kind).await?))
}

/// Revokes the token that was used.
#[utoipa::path(post, path = "/api/v1/logout", tag = "account", security(("bearer" = [])),
    responses((status = 204), (status = 401, body = crate::ErrorBody)))]
pub async fn logout(State(s): State<WebState>, Auth(who): Auth) -> Result<StatusCode, ApiError> {
    s.accounts.revoke_token(&who.token)?;
    Ok(StatusCode::NO_CONTENT)
}
