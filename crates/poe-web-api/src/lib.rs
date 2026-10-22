//! HTTP API for the web admin and the launcher (WP4.1). See `docs/architecture/web-api.md`.
//!
//! REST under `/api/v1`, a WebSocket log stream, and optionally the built web admin (SPA)
//! as static files. Accounts and tokens go through [`poe_accounts::AccountStore`] exactly
//! like the game login (the server hashes the plain password the way the client does), and
//! administration goes through [`poe_cluster::AdminControl`], which runs the same command
//! registry as the in-game chat.

mod admin;
mod auth;
mod bans;
mod configfile;
mod dto;
mod error;
pub mod lifecycle;
pub mod logs;
mod metrics;
pub mod motd;
mod mods;
mod openapi;
mod public;
pub mod scheduler;
mod webtoken;

use std::{path::PathBuf, sync::Arc, time::Instant};

use axum::{
    Router,
    http::{HeaderValue, Method, header},
    routing::{get, post},
};
use poe_accounts::{AccountStore, LoginLimiter};
use poe_cluster::{AdminControl, InstanceDirectory, Metrics, ModDirectory};
use poe_config::Config;
use poe_store::Store;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
};
use tracing::{info, warn};

pub use error::{ApiError, ErrorBody};
pub use lifecycle::{Lifecycle, PendingShutdown, ShutdownKind};
pub use logs::{LogBuffer, LogEvent, LogFilter, LogLayer, LogLevel};
pub use motd::Motd;
pub use openapi::{openapi, openapi_json};
pub use scheduler::{Clock, SystemClock};
pub use webtoken::WebTokens;

/// Lifetime of a web token (the browser session).
pub const WEB_TOKEN_TTL_SECS: i64 = 12 * 3600;

/// Everything the handlers use. Cheap to clone.
#[derive(Clone)]
pub struct WebState {
    pub config: Arc<Config>,
    pub accounts: Arc<dyn AccountStore>,
    /// Password attempt limits (shared with the login server).
    pub limiter: Arc<LoginLimiter>,
    pub store: Arc<dyn Store>,
    pub instances: Arc<dyn InstanceDirectory>,
    pub admin: Arc<dyn AdminControl>,
    pub logs: Arc<LogBuffer>,
    /// Single-use sign-in tokens handed from the launcher to the browser.
    pub web_tokens: Arc<WebTokens>,
    /// Single-use tickets for the log WebSocket (`POST /admin/logs/ws-ticket`): a browser
    /// cannot send headers on a WebSocket, and a ticket in the URL is worthless once used.
    pub ws_tickets: Arc<WebTokens>,
    /// Stop and restart requests (`/admin/server/...`); the process waits on the receiver of
    /// [`Lifecycle::new`].
    pub lifecycle: Arc<Lifecycle>,
    /// Samples behind the dashboard charts (`GET /admin/metrics`); filled by
    /// `poe_cluster::metrics::spawn_sampler`.
    pub metrics: Arc<Metrics>,
    /// Scheduled tasks (`/admin/schedule/...`); the process runs [`scheduler::Scheduler::run`].
    pub scheduler: Arc<scheduler::Scheduler>,
    /// The live message of the day.
    pub motd: Arc<Motd>,
    /// The server's mods: the client mods it offers and the admin's list.
    pub mods: Arc<dyn ModDirectory>,
    /// The directory `versions/<version>.toml` is read from when the config editor checks a
    /// text (the working directory; the repository root in a checkout).
    pub config_root: PathBuf,
    pub started: Instant,
}

/// The API router (`/api/v1/...`), the CORS layer and, when `[web] static_dir` is a directory,
/// the static web admin with an `index.html` fallback for client-side routes.
pub fn router(state: WebState) -> Router {
    let web = &state.config.web;
    let api = Router::new()
        .route("/server", get(public::server))
        .route("/mods/{id}/{file}", get(mods::package))
        .route("/register", post(public::register))
        .route("/login", post(public::login))
        .route("/logout", post(public::logout))
        .route("/password", post(public::password))
        .route("/game-token", post(public::game_token))
        .route("/me", get(public::me))
        .route("/web-token", post(public::web_token))
        .route("/web-token/redeem", post(public::redeem_web_token))
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
        .route("/admin/bans", get(bans::list).post(bans::create))
        .route("/admin/bans/{id}/lift", post(bans::lift))
        .route("/admin/metrics", get(metrics::get))
        .route("/admin/mods", get(mods::admin_list))
        .route("/admin/motd", post(motd::set))
        .route("/admin/config", get(configfile::get))
        .route("/admin/config/validate", post(configfile::validate))
        .route("/admin/config/fields", post(configfile::fields))
        .route("/admin/config/save", post(configfile::save))
        .route("/admin/config/backups", get(configfile::backups))
        .route("/admin/config/backups/{name}", get(configfile::backup))
        .route("/admin/schedule", get(scheduler::api::overview))
        .route("/admin/schedule/preview", post(scheduler::api::preview))
        .route("/admin/schedule/tasks", post(scheduler::api::create))
        .route("/admin/schedule/tasks/{id}", post(scheduler::api::update))
        .route(
            "/admin/schedule/tasks/{id}/enable",
            post(scheduler::api::enable),
        )
        .route(
            "/admin/schedule/tasks/{id}/disable",
            post(scheduler::api::disable),
        )
        .route(
            "/admin/schedule/tasks/{id}/delete",
            post(scheduler::api::delete),
        )
        .route("/admin/schedule/tasks/{id}/runs", get(scheduler::api::runs))
        .route("/admin/announce", post(admin::announce))
        .route("/admin/server/stop", post(admin::server_stop))
        .route("/admin/server/restart", post(admin::server_restart))
        .route("/admin/server/cancel", post(admin::server_cancel))
        .route("/admin/server/shutdown", get(admin::server_shutdown))
        .route("/admin/accounts", get(admin::accounts))
        .route(
            "/admin/accounts/{name}/disable",
            post(admin::disable_account),
        )
        .route("/admin/accounts/{name}/enable", post(admin::enable_account))
        .route("/admin/accounts/{name}/role", post(admin::set_role))
        .route("/admin/characters", get(admin::characters))
        .route("/admin/characters/{id}", get(admin::character))
        .route("/admin/logs/ws-ticket", post(admin::logs_ws_ticket))
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
    app.layer(axum::middleware::map_response(security_headers))
        .with_state(state)
}

/// Response headers for the API and the static web admin: no MIME sniffing, no referrer, no
/// framing, and a content security policy that allows only the app's own scripts, styles and
/// connections (the web admin loads nothing from elsewhere).
async fn security_headers(mut res: axum::response::Response) -> axum::response::Response {
    const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
        img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; \
        form-action 'self'; frame-ancestors 'none'";
    let h = res.headers_mut();
    for (name, value) in [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::CONTENT_SECURITY_POLICY, CSP),
    ] {
        h.entry(name).or_insert(HeaderValue::from_static(value));
    }
    res
}

/// Listens on `[web] bind` until the process ends.
pub async fn serve(state: WebState) -> anyhow::Result<()> {
    let bind = state.config.web.bind;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(addr = %bind, "web api listening");
    // Connect info gives the limiter the client's address.
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_features;
#[cfg(test)]
mod tests_mods;
