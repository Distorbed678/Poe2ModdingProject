//! Bearer-token authentication. Tokens are the `poe_accounts` kinds `web` and `launcher`,
//! sent as `Authorization: Bearer <64 hex characters>`.

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use poe_accounts::{AccountError, AccountRecord, Role, TokenKind};

use crate::{ApiError, WebState};

/// Who is calling.
pub struct Principal {
    pub account: AccountRecord,
    pub kind: TokenKind,
    pub token: [u8; 32],
}

pub fn parse_token(text: &str) -> Result<[u8; 32], ApiError> {
    let bytes = hex::decode(text.trim()).map_err(|_| ApiError::unauthorized("malformed token"))?;
    bytes
        .try_into()
        .map_err(|_| ApiError::unauthorized("malformed token"))
}

fn bearer(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token)
}

/// Checks a token against the store. `admin_only`: web tokens of admins only (launcher
/// tokens are long-lived and sit in the OS keyring, so they never reach the admin API).
pub fn authenticate(
    state: &WebState,
    token: &str,
    admin_only: bool,
) -> Result<Principal, ApiError> {
    let token = parse_token(token)?;
    let kinds: &[TokenKind] = if admin_only {
        &[TokenKind::Web]
    } else {
        &[TokenKind::Web, TokenKind::Launcher]
    };
    for kind in kinds {
        match state.accounts.authenticate_token(&token, &[*kind]) {
            Ok(account) => {
                if admin_only && account.role != Role::Admin {
                    return Err(ApiError::forbidden("admin role required"));
                }
                return Ok(Principal {
                    account,
                    kind: *kind,
                    token,
                });
            }
            // The token may be of the other kind.
            Err(AccountError::TokenUnknown) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(AccountError::TokenUnknown.into())
}

fn missing() -> ApiError {
    ApiError::unauthorized("missing Authorization: Bearer <token>")
}

/// Any logged-in account.
pub struct Auth(pub Principal);

impl FromRequestParts<WebState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &WebState) -> Result<Self, ApiError> {
        let token = bearer(parts).ok_or_else(missing)?;
        authenticate(state, token, false).map(Auth)
    }
}

/// A logged-in admin with a web token.
pub struct AdminAuth(pub Principal);

impl FromRequestParts<WebState> for AdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &WebState) -> Result<Self, ApiError> {
        let token = bearer(parts).ok_or_else(missing)?;
        authenticate(state, token, true).map(AdminAuth)
    }
}

/// The WebSocket route: an admin's web token in the `Authorization` header, or, because
/// browsers cannot set headers on a WebSocket, a single-use ticket from
/// `POST /admin/logs/ws-ticket` as `?ticket=`. The ticket is short-lived and dead after the
/// first use, so a proxy that logs the URL learns nothing worth having. (The web token itself
/// is not accepted in the URL.)
pub struct WsAdminAuth;

impl FromRequestParts<WebState> for WsAdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &WebState) -> Result<Self, ApiError> {
        if let Some(token) = bearer(parts) {
            return authenticate(state, token, true).map(|_| WsAdminAuth);
        }
        let ticket = parts
            .uri
            .query()
            .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("ticket=")))
            .ok_or_else(missing)?;
        let ticket = parse_token(ticket)?;
        let account_id = state
            .ws_tickets
            .redeem(&ticket, std::time::Instant::now())
            .ok_or_else(|| ApiError::unauthorized("invalid or expired ticket"))?;
        let account = state
            .accounts
            .find_by_id(account_id)?
            .ok_or_else(|| ApiError::unauthorized("invalid or expired ticket"))?;
        if account.disabled {
            return Err(AccountError::Disabled.into());
        }
        if account.role != Role::Admin {
            return Err(ApiError::forbidden("admin role required"));
        }
        Ok(WsAdminAuth)
    }
}

/// The caller's IP for the password attempt limits: the socket address (set by `serve`), or
/// with `[security] trust_forwarded_for` the **last** `X-Forwarded-For` entry, the one the
/// reverse proxy appended (a client can prepend entries of its own to the header, but not the
/// proxy's). An address that cannot be determined (tests that call the router directly)
/// counts as `0.0.0.0`.
pub struct ClientIp(pub std::net::IpAddr);

impl FromRequestParts<WebState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &WebState,
    ) -> Result<Self, Self::Rejection> {
        let socket = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip());
        let forwarded = state
            .config
            .security
            .trust_forwarded_for
            .then(|| {
                // The proxy appends to the header (or adds one more header line): the last
                // entry of the last line is its observation of the real peer.
                parts
                    .headers
                    .get_all("x-forwarded-for")
                    .iter()
                    .next_back()?
                    .to_str()
                    .ok()?
                    .rsplit(',')
                    .next()?
                    .trim()
                    .parse()
                    .ok()
            })
            .flatten();
        Ok(ClientIp(
            forwarded
                .or(socket)
                .unwrap_or(std::net::IpAddr::from([0, 0, 0, 0])),
        ))
    }
}
