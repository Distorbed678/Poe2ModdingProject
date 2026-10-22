//! Public endpoints (`/server`, `/register`, `/login`) and the ones any logged-in account may
//! call (`/password`, `/logout`).
//!
//! Passwords arrive as plain text. They are turned into the client's digest here
//! (`poe_accounts::password_digest`, SHA-256 of the UTF-16LE text) and then handled by
//! `AccountStore` exactly like the game login, so a password works on the web and in game.

use axum::{
    Json,
    extract::State,
    http::{HeaderName, StatusCode, header},
};
use poe_accounts::{AccountError, MIN_PASSWORD_LEN, Role, TokenKind, password_digest};

use crate::{
    ApiError, WEB_TOKEN_TTL_SECS, WebState,
    auth::{Auth, ClientIp},
    dto::{
        AccountInfo, LoginRequest, LoginResponse, PasswordRequest, RegisterRequest, ServerInfo,
        WebTokenRedeemRequest, WebTokenResponse,
    },
    error::ApiJson,
    webtoken::MintError,
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

/// 403 with the ban's text when the address is banned (`POST /admin/bans`).
async fn check_ip_ban(state: &WebState, ip: std::net::IpAddr) -> Result<(), ApiError> {
    let accounts = state.accounts.clone();
    let canonical = poe_accounts::canonical_ip(ip);
    match blocking(move || accounts.active_ip_ban(&canonical)).await?? {
        Some(ban) => {
            tracing::warn!(%ip, ban = ban.id, "web request refused: banned address");
            Err(ApiError::forbidden(ban.notice()))
        }
        None => Ok(()),
    }
}

/// 429 when the account name or the IP is locked by `[security]`.
fn check_limit(state: &WebState, name: &str, ip: std::net::IpAddr) -> Result<(), ApiError> {
    state.limiter.check(name, ip).map_err(|locked| {
        tracing::warn!(account = %name, %ip, "web login refused: too many failed attempts");
        ApiError::too_many_attempts(locked.retry_after_secs)
    })
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
        login_port: c.network.login.public_port(),
        instance_port: c.network.instance.public_port,
        gateway_region: c.gateway.region.clone(),
        uptime_secs: s.started.elapsed().as_secs(),
        motd: Some(s.motd.get()).filter(|m| !m.is_empty()),
        mods: crate::mods::advertised(&s),
    })
}

/// Creates a player account (only when `[accounts] open_registration` is on).
#[utoipa::path(post, path = "/api/v1/register", tag = "public", request_body = RegisterRequest,
    responses((status = 201, body = AccountInfo), (status = 400, body = crate::ErrorBody),
        (status = 403, description = "registration is closed", body = crate::ErrorBody),
        (status = 409, description = "name taken", body = crate::ErrorBody)))]
pub async fn register(
    State(s): State<WebState>,
    ClientIp(ip): ClientIp,
    ApiJson(req): ApiJson<RegisterRequest>,
) -> Result<(StatusCode, Json<AccountInfo>), ApiError> {
    if !s.config.accounts.open_registration {
        return Err(ApiError::forbidden("registration is closed"));
    }
    check_ip_ban(&s, ip).await?;
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
        (status = 403, description = "account is disabled", body = crate::ErrorBody),
        (status = 429, description = "too many failed attempts for this account or IP; see the Retry-After header", body = crate::ErrorBody)))]
pub async fn login(
    State(s): State<WebState>,
    ClientIp(ip): ClientIp,
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
    // A banned address and locked names and IPs are refused before the (slow) password check.
    check_ip_ban(&s, ip).await?;
    check_limit(&s, &req.username, ip)?;
    let accounts = s.accounts.clone();
    let name = req.username.clone();
    let result =
        blocking(move || accounts.authenticate(&req.username, &password_digest(&req.password)))
            .await?;
    let account = match result {
        Ok(a) => {
            s.limiter.success(&name);
            a
        }
        // One answer for both, so names cannot be probed.
        Err(AccountError::NoAccount | AccountError::WrongPassword) => {
            s.limiter.failure(&name, ip);
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
        (status = 403, description = "wrong current password", body = crate::ErrorBody),
        (status = 429, description = "too many failed password attempts; see the Retry-After header", body = crate::ErrorBody)))]
pub async fn password(
    State(s): State<WebState>,
    ClientIp(ip): ClientIp,
    Auth(who): Auth,
    ApiJson(req): ApiJson<PasswordRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    check_password(&req.new_password)?;
    // The old password is a password guess too: same limits as the login.
    check_limit(&s, &who.account.username, ip)?;
    let accounts = s.accounts.clone();
    let name = who.account.username.clone();
    let result = blocking(move || {
        accounts.authenticate(&name, &password_digest(&req.old_password))?;
        accounts.set_password(&name, &password_digest(&req.new_password))
    })
    .await?;
    let account = match result {
        Ok(a) => {
            s.limiter.success(&who.account.username);
            a
        }
        Err(AccountError::WrongPassword) => {
            s.limiter.failure(&who.account.username, ip);
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

/// Exchanges a launcher token for a fresh `game_remember` token (the client's Remember Me
/// credential). The launcher writes it into the client's credentials file for auto-login.
///
/// The game login retires a token when it uses it (rolling tokens), so the launcher token
/// itself must never be written to the client: it would be revoked by the first game login
/// and the launcher would be logged out. Only launcher tokens may call this; the new token
/// lasts `[accounts] remember_days` and does not touch the account's other tokens.
#[utoipa::path(post, path = "/api/v1/game-token", tag = "account", security(("bearer" = [])),
    responses((status = 200, body = LoginResponse),
        (status = 401, body = crate::ErrorBody),
        (status = 403, description = "not a launcher token", body = crate::ErrorBody)))]
pub async fn game_token(
    State(s): State<WebState>,
    Auth(who): Auth,
) -> Result<Json<LoginResponse>, ApiError> {
    if who.kind != TokenKind::Launcher {
        return Err(ApiError::forbidden("a launcher token is required"));
    }
    let kind = TokenKind::GameRemember;
    let accounts = s.accounts.clone();
    let id = who.account.id;
    let ttl = i64::from(s.config.accounts.remember_days) * 86_400;
    let issued = blocking(move || accounts.issue_token(id, kind, ttl)).await??;
    tracing::info!(account = %who.account.username, "game token issued to the launcher");
    Ok(Json(LoginResponse {
        token: hex::encode(issued.token),
        kind: kind.as_str().into(),
        expires_at: issued.expires_at,
        account: who.account.into(),
    }))
}

/// The calling account: name, role (`player` or `admin`) and state. For the launcher, to decide
/// whether to offer "Open web admin" without logging in again. Works with a web or launcher token.
#[utoipa::path(get, path = "/api/v1/me", tag = "account", security(("bearer" = [])),
    responses((status = 200, body = AccountInfo), (status = 401, body = crate::ErrorBody)))]
pub async fn me(Auth(who): Auth) -> Json<AccountInfo> {
    Json(who.account.into())
}

/// Never cache a response that carries a credential.
const NO_STORE: (HeaderName, &str) = (header::CACHE_CONTROL, "no-store");

/// Exchanges a launcher token for a single-use web sign-in token, so "Open web admin" can open
/// the browser already logged in. The token lasts `[web] web_token_ttl_secs` (default 60) and
/// can be redeemed once with `POST /web-token/redeem`; asking again replaces the account's
/// earlier unused token. Only admins' launcher tokens may call this.
#[utoipa::path(post, path = "/api/v1/web-token", tag = "account", security(("bearer" = [])),
    responses((status = 200, body = WebTokenResponse),
        (status = 401, body = crate::ErrorBody),
        (status = 403, description = "not a launcher token, or not an admin", body = crate::ErrorBody),
        (status = 503, description = "too many unused tokens are waiting", body = crate::ErrorBody)))]
pub async fn web_token(
    State(s): State<WebState>,
    Auth(who): Auth,
) -> Result<([(HeaderName, &'static str); 1], Json<WebTokenResponse>), ApiError> {
    if who.kind != TokenKind::Launcher {
        return Err(ApiError::forbidden("a launcher token is required"));
    }
    if who.account.role != Role::Admin {
        return Err(ApiError::forbidden("admin role required"));
    }
    // 0 would make a token that is dead on arrival.
    let ttl = s.config.web.web_token_ttl_secs.max(1);
    let token = s
        .web_tokens
        .mint(
            who.account.id,
            std::time::Duration::from_secs(ttl),
            std::time::Instant::now(),
        )
        .map_err(|MintError::Full| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "too many unused web tokens, try again shortly",
            )
        })?;
    // The token itself is never logged.
    tracing::info!(account = %who.account.username, ttl_secs = ttl, "web sign-in token issued");
    let token = hex::encode(token);
    Ok((
        [NO_STORE],
        Json(WebTokenResponse {
            login_path: format!("/#/token-login?t={token}"),
            token,
            expires_in_secs: ttl,
        }),
    ))
}

/// Redeems a web sign-in token for a normal `web` session (12 hours), once. The web admin calls
/// this from its `#/token-login` page. An unknown, used or expired token gets `401`, all alike.
/// Failures count against the caller's IP like failed logins (`[security]`): `429` when locked.
#[utoipa::path(post, path = "/api/v1/web-token/redeem", tag = "public", request_body = WebTokenRedeemRequest,
    responses((status = 200, body = LoginResponse),
        (status = 401, description = "unknown, already used or expired token", body = crate::ErrorBody),
        (status = 403, description = "the account is disabled", body = crate::ErrorBody),
        (status = 429, description = "too many failed redeem attempts from this IP; see the Retry-After header", body = crate::ErrorBody)))]
pub async fn redeem_web_token(
    State(s): State<WebState>,
    ClientIp(ip): ClientIp,
    ApiJson(req): ApiJson<WebTokenRedeemRequest>,
) -> Result<([(HeaderName, &'static str); 1], Json<LoginResponse>), ApiError> {
    // The limiter keys failures by account name and IP; no account is known yet, so the name
    // part is a per-IP pseudo name (one client cannot lock the key for the others).
    let key = format!("web-token@{ip}");
    check_limit(&s, &key, ip)?;
    let invalid = || ApiError::unauthorized("invalid or expired web token");
    let redeemed = crate::auth::parse_token(&req.token)
        .ok()
        .and_then(|t| s.web_tokens.redeem(&t, std::time::Instant::now()));
    let Some(account_id) = redeemed else {
        s.limiter.failure(&key, ip);
        return Err(invalid());
    };
    let accounts = s.accounts.clone();
    let account = blocking(move || accounts.find_by_id(account_id))
        .await??
        .ok_or_else(invalid)?;
    if account.disabled {
        return Err(AccountError::Disabled.into());
    }
    check_ip_ban(&s, ip).await?;
    let accounts = s.accounts.clone();
    if let Some(ban) = blocking(move || accounts.active_account_ban(account_id)).await?? {
        return Err(ApiError::forbidden(ban.notice()));
    }
    tracing::info!(account = %account.username, "web sign-in token redeemed");
    Ok(([NO_STORE], Json(issue(&s, account, TokenKind::Web).await?)))
}
