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

/// The WebSocket route: browsers cannot set headers on a WebSocket, so `?token=` works too.
pub struct WsAdminAuth;

impl FromRequestParts<WebState> for WsAdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &WebState) -> Result<Self, ApiError> {
        let from_query = parts
            .uri
            .query()
            .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
            .map(str::to_owned);
        let token = match bearer(parts) {
            Some(t) => t.to_owned(),
            None => from_query.ok_or_else(missing)?,
        };
        authenticate(state, &token, true).map(|_| WsAdminAuth)
    }
}

/// The caller's IP for the password attempt limits: the socket address (set by `serve`), or
/// with `[security] trust_forwarded_for` the first `X-Forwarded-For` entry. An address that
/// cannot be determined (tests that call the router directly) counts as `0.0.0.0`.
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
                parts
                    .headers
                    .get("x-forwarded-for")?
                    .to_str()
                    .ok()?
                    .split(',')
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
