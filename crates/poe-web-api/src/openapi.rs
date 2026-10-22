//! The OpenAPI document (`GET /api/v1/openapi.json`; `cargo xtask openapi` writes it to
//! `docs/architecture/openapi.json` for the web admin and the launcher to generate types).

use axum::Json;
use utoipa::{
    Modify, OpenApi,
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};

use crate::{
    admin, bans, configfile, dto, error, lifecycle, logs, metrics, motd, public, scheduler,
};

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("64 hex characters from POST /login")
                    .build(),
            ),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "PoE2 private server API",
        description = "Accounts, administration and live log of the private server. Passwords are sent as plain text: use HTTPS outside the local machine.",
        version = "1"
    ),
    paths(
        public::server, public::register, public::login, public::logout, public::password,
        public::game_token, public::me, public::web_token, public::redeem_web_token,
        admin::sessions, admin::instances, admin::create_instance, admin::close_instance,
        admin::kick, admin::mute, admin::unmute, admin::transfer, admin::announce,
        admin::server_stop, admin::server_restart, admin::server_cancel, admin::server_shutdown,
        admin::accounts, admin::disable_account, admin::enable_account, admin::set_role,
        admin::characters, admin::character, admin::logs_ws_ticket, admin::logs_ws,
        bans::list, bans::create, bans::lift, metrics::get, motd::set,
        configfile::get, configfile::validate, configfile::fields, configfile::save,
        configfile::backups, configfile::backup,
        scheduler::api::overview, scheduler::api::preview, scheduler::api::create,
        scheduler::api::update, scheduler::api::enable, scheduler::api::disable,
        scheduler::api::delete, scheduler::api::runs,
    ),
    components(schemas(
        error::ErrorBody, dto::ServerInfo, dto::AccountInfo, dto::RegisterRequest, dto::LoginRequest,
        dto::LoginResponse, dto::WsTicketResponse, dto::WebTokenResponse, dto::WebTokenRedeemRequest, dto::PasswordRequest, dto::OnlinePlayerInfo, dto::InstanceView,
        dto::NewInstanceRequest, dto::CreatedInstance, dto::TransferRequest, dto::MuteRequest,
        dto::AnnounceRequest, dto::RoleRequest, dto::CommandResult, dto::LocationInfo,
        dto::ShutdownRequest, dto::ShutdownStatus, lifecycle::PendingShutdown, lifecycle::ShutdownKind,
        dto::CharacterInfo, logs::LogEvent, logs::LogFilter, logs::LogLevel,
        bans::BanView, bans::BanRequest, metrics::MetricsPoint, metrics::MetricsResponse,
        motd::MotdRequest, motd::MotdResponse,
        configfile::ConfigFile, configfile::ConfigField, configfile::ValidateRequest,
        configfile::ConfigProblem, configfile::ValidateResponse, configfile::FieldsRequest,
        configfile::FieldsResponse, configfile::SaveRequest, configfile::SaveResponse,
        configfile::BackupInfo, configfile::BackupContent,
        scheduler::api::TaskBody, scheduler::api::TaskView, scheduler::api::ScheduleOverview,
        scheduler::api::RunView, scheduler::api::PreviewRequest, scheduler::api::PreviewResponse,
        scheduler::schedule::Schedule, scheduler::kind::TaskKind,
    )),
    modifiers(&BearerAuth),
    tags(
        (name = "public", description = "No login needed"),
        (name = "account", description = "Any logged-in account"),
        (name = "admin", description = "Admin role and a web token"),
    )
)]
struct ApiDoc;

pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// The document as pretty JSON.
pub fn openapi_json() -> String {
    openapi()
        .to_pretty_json()
        .expect("the OpenAPI document serializes")
}

pub async fn serve() -> Json<utoipa::openapi::OpenApi> {
    Json(openapi())
}
