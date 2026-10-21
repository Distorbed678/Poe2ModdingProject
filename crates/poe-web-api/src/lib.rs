//! HTTP API for the web admin and the launcher (WP4.1). See `docs/architecture/web-api.md`.
//!
//! REST under `/api/v1`, a WebSocket log stream, and optionally the built web admin (SPA)
//! as static files. Accounts and tokens go through [`poe_accounts::AccountStore`] exactly
//! like the game login (the server hashes the plain password the way the client does), and
//! administration goes through [`poe_cluster::AdminControl`], which runs the same command
//! registry as the in-game chat.

mod admin;
mod auth;
mod dto;
mod error;
pub mod logs;
mod openapi;
mod public;

use std::{sync::Arc, time::Instant};

use axum::{
    Router,
    http::{HeaderValue, Method, header},
    routing::{get, post},
};
use poe_accounts::AccountStore;
use poe_cluster::{AdminControl, InstanceDirectory};
use poe_config::Config;
use poe_store::Store;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
};
use tracing::{info, warn};

pub use error::{ApiError, ErrorBody};
pub use logs::{LogBuffer, LogEvent, LogFilter, LogLayer, LogLevel};
pub use openapi::{openapi, openapi_json};

/// Lifetime of a web token (the browser session).
pub const WEB_TOKEN_TTL_SECS: i64 = 12 * 3600;

/// Everything the handlers use. Cheap to clone.
#[derive(Clone)]
pub struct WebState {
    pub config: Arc<Config>,
    pub accounts: Arc<dyn AccountStore>,
    pub store: Arc<dyn Store>,
    pub instances: Arc<dyn InstanceDirectory>,
    pub admin: Arc<dyn AdminControl>,
    pub logs: Arc<LogBuffer>,
    pub started: Instant,
}

/// The API router (`/api/v1/...`), the CORS layer and, when `[web] static_dir` is a directory,
/// the static web admin with an `index.html` fallback for client-side routes.
pub fn router(state: WebState) -> Router {
    let web = &state.config.web;
    let api = Router::new()
        .route("/server", get(public::server))
        .route("/register", post(public::register))
        .route("/login", post(public::login))
        .route("/logout", post(public::logout))
        .route("/password", post(public::password))
        .route("/game-token", post(public::game_token))
        .route("/admin/sessions", get(admin::sessions))
        .route(
            "/admin/instances",
            get(admin::instances).post(admin::create_instance),
        )
        .route("/admin/instances/{id}/close", post(admin::close_instance))
        .route("/admin/players/{name}/kick", post(admin::kick))
        .route("/admin/players/{name}/mute", post(admin::mute))
        .route("/admin/players/{name}/unmute", post(admin::unmute))
        .route("/admin/players/{name}/transfer", post(admin::transfer))
        .route("/admin/announce", post(admin::announce))
        .route("/admin/accounts", get(admin::accounts))
        .route(
            "/admin/accounts/{name}/disable",
            post(admin::disable_account),
        )
        .route("/admin/accounts/{name}/enable", post(admin::enable_account))
        .route("/admin/accounts/{name}/role", post(admin::set_role))
        .route("/admin/characters", get(admin::characters))
        .route("/admin/characters/{id}", get(admin::character))
        .route("/admin/logs/ws", get(admin::logs_ws))
        .route("/openapi.json", get(openapi::serve))
        .fallback(|| async { ApiError::not_found("no such endpoint") });
    let mut app = Router::new().nest("/api/v1", api);

    if !web.static_dir.as_os_str().is_empty() {
        if web.static_dir.is_dir() {
            let index = web.static_dir.join("index.html");
            let files = ServeDir::new(&web.static_dir).fallback(ServeFile::new(index));
            app = app.fallback_service(files);
            info!(dir = %web.static_dir.display(), "serving the web admin");
        } else {
            warn!(dir = %web.static_dir.display(), "[web] static_dir is not a directory: not serving the web admin");
        }
    }

    let origins: Vec<HeaderValue> = web
        .cors_origins
        .iter()
        .filter_map(|o| match o.parse() {
            Ok(v) => Some(v),
            Err(_) => {
                warn!(origin = %o, "[web] cors_origins: invalid origin ignored");
                None
            }
        })
        .collect();
    if !origins.is_empty() {
        app = app.layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
        );
    }
    app.with_state(state)
}

/// Listens on `[web] bind` until the process ends.
pub async fn serve(state: WebState) -> anyhow::Result<()> {
    let bind = state.config.web.bind;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(addr = %bind, "web api listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
