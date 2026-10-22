//! Admin endpoints (`/admin/...`, admin role and a web token).
//!
//! Actions run through `AdminControl::run_command`, i.e. the server's command registry: the
//! same checks, log lines and audit file as the in-game chat commands.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::Response,
};
use poe_accounts::Role;
use poe_cluster::{AdminOutcome, NewInstance, TransferTarget};
use serde::Serialize;
use tokio::sync::broadcast::error::RecvError;

use crate::{
    ApiError, LogBuffer, LogEvent, LogFilter, PendingShutdown, ShutdownKind, WebState,
    auth::{AdminAuth, WsAdminAuth},
    dto::*,
    error::ApiJson,
    lifecycle,
    public::blocking,
};

/// A name that is safe to put in a command line (the registry splits on whitespace).
fn word(what: &str, value: &str) -> Result<(), ApiError> {
    if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ApiError::bad_request(format!("invalid {what}")));
    }
    Ok(())
}

/// Maps a finished command to a response.
fn done(out: AdminOutcome) -> Result<Json<CommandResult>, ApiError> {
    if out.ok {
        return Ok(Json(CommandResult { message: out.text }));
    }
    let t = out.text.to_lowercase();
    let status = if t.contains("admins only") {
        StatusCode::FORBIDDEN
    } else if t.contains("not online")
        || t.contains("no instance")
        || t.contains("no such account")
        || t.contains("no area")
    {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    };
    Err(ApiError::new(status, out.text))
}

fn failed(out: AdminOutcome) -> ApiError {
    done(out)
        .err()
        .unwrap_or_else(|| ApiError::internal("failed outcome reported ok"))
}

/// Players in game now, with their account, character level and class, where they are, since
/// when, their address and whether they are muted. (The address is only in the admin API.)
#[utoipa::path(get, path = "/api/v1/admin/sessions", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = Vec<OnlinePlayerInfo>), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn sessions(
    State(s): State<WebState>,
    _: AdminAuth,
) -> Result<Json<Vec<OnlinePlayerInfo>>, ApiError> {
    let state = s.clone();
    let list = blocking(move || -> Result<Vec<OnlinePlayerInfo>, ApiError> {
        let online = state.admin.online();
        if online.is_empty() {
            return Ok(Vec::new());
        }
        // Level and class come from the character rows.
        let characters: HashMap<String, (u32, u32)> = state
            .store
            .all_characters()?
            .into_iter()
            .map(|c| (c.name.to_lowercase(), (c.level, c.class_id)))
            .collect();
        Ok(online
            .into_iter()
            .map(|o| {
                let found = characters.get(&o.character.to_lowercase()).copied();
                let mut info = OnlinePlayerInfo::from(o);
                info.level = found.map(|f| f.0);
                info.class_id = found.map(|f| f.1);
                info
            })
            .collect())
    })
    .await??;
    Ok(Json(list))
}

/// Every instance with its players.
#[utoipa::path(get, path = "/api/v1/admin/instances", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = Vec<InstanceView>), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn instances(State(s): State<WebState>, _: AdminAuth) -> Json<Vec<InstanceView>> {
    Json(s.instances.list().into_iter().map(Into::into).collect())
}

fn new_instance(req: NewInstanceRequest) -> NewInstance {
    NewInstance {
        area: req.area,
        seed: req.seed,
        topology: req.topology,
        orientation: req.orientation,
    }
}

/// Creates an instance without moving anyone into it.
#[utoipa::path(post, path = "/api/v1/admin/instances", tag = "admin", security(("bearer" = [])),
    request_body = NewInstanceRequest,
    responses((status = 201, body = CreatedInstance), (status = 400, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn create_instance(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    ApiJson(req): ApiJson<NewInstanceRequest>,
) -> Result<(StatusCode, Json<CreatedInstance>), ApiError> {
    word("area", &req.area)?;
    match s
        .admin
        .create_instance(&who.account.username, &new_instance(req))
    {
        Ok(id) => Ok((StatusCode::CREATED, Json(CreatedInstance { id }))),
        Err(out) => Err(failed(out)),
    }
}

/// Closes an instance; its players are disconnected.
#[utoipa::path(post, path = "/api/v1/admin/instances/{id}/close", tag = "admin", security(("bearer" = [])),
    params(("id" = u32, Path, description = "instance id")),
    responses((status = 200, body = CommandResult), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn close_instance(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(id): Path<u32>,
) -> Result<Json<CommandResult>, ApiError> {
    done(
        s.admin
            .run_command(&who.account.username, &format!("close {id}")),
    )
}

/// Disconnects a player.
#[utoipa::path(post, path = "/api/v1/admin/players/{name}/kick", tag = "admin", security(("bearer" = [])),
    params(("name" = String, Path, description = "character name")),
    responses((status = 200, body = CommandResult), (status = 404, description = "not online", body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn kick(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(name): Path<String>,
) -> Result<Json<CommandResult>, ApiError> {
    word("player name", &name)?;
    done(
        s.admin
            .run_command(&who.account.username, &format!("kick {name}")),
    )
}

/// Stops a player's chat, for `minutes` or until unmuted.
#[utoipa::path(post, path = "/api/v1/admin/players/{name}/mute", tag = "admin", security(("bearer" = [])),
    params(("name" = String, Path, description = "character name")), request_body(content = MuteRequest, description = "optional"),
    responses((status = 200, body = CommandResult), (status = 400, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn mute(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(name): Path<String>,
    body: axum::body::Bytes,
) -> Result<Json<CommandResult>, ApiError> {
    word("player name", &name)?;
    let req: MuteRequest = if body.iter().all(u8::is_ascii_whitespace) {
        MuteRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| ApiError::bad_request(format!("bad JSON body: {e}")))?
    };
    let line = match req.minutes {
        Some(m) => format!("mute {name} {m}"),
        None => format!("mute {name}"),
    };
    done(s.admin.run_command(&who.account.username, &line))
}

/// Lets a muted player chat again.
#[utoipa::path(post, path = "/api/v1/admin/players/{name}/unmute", tag = "admin", security(("bearer" = [])),
    params(("name" = String, Path, description = "character name")),
    responses((status = 200, body = CommandResult), (status = 400, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn unmute(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(name): Path<String>,
) -> Result<Json<CommandResult>, ApiError> {
    word("player name", &name)?;
    done(
        s.admin
            .run_command(&who.account.username, &format!("unmute {name}")),
    )
}

/// Moves a player in game to an existing instance (`instance`) or a new one (`area`).
#[utoipa::path(post, path = "/api/v1/admin/players/{name}/transfer", tag = "admin", security(("bearer" = [])),
    params(("name" = String, Path, description = "character name")), request_body = TransferRequest,
    responses((status = 200, body = CommandResult), (status = 400, body = crate::ErrorBody), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn transfer(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(name): Path<String>,
    ApiJson(req): ApiJson<TransferRequest>,
) -> Result<Json<CommandResult>, ApiError> {
    word("player name", &name)?;
    let target = match (req.instance, req.area) {
        (Some(id), None) => TransferTarget::Instance(id),
        (None, Some(area)) => {
            word("area", &area)?;
            TransferTarget::New(NewInstance {
                area,
                seed: req.seed,
                topology: req.topology,
                orientation: req.orientation,
            })
        }
        _ => return Err(ApiError::bad_request("give either instance or area")),
    };
    done(
        s.admin
            .transfer_player(&who.account.username, &name, target),
    )
}

/// A `<Server>` chat line to every player.
#[utoipa::path(post, path = "/api/v1/admin/announce", tag = "admin", security(("bearer" = [])),
    request_body = AnnounceRequest,
    responses((status = 200, body = CommandResult), (status = 400, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn announce(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    ApiJson(req): ApiJson<AnnounceRequest>,
) -> Result<Json<CommandResult>, ApiError> {
    let message = req.message.split_whitespace().collect::<Vec<_>>().join(" ");
    if message.is_empty() {
        return Err(ApiError::bad_request("message is empty"));
    }
    done(
        s.admin
            .run_command(&who.account.username, &format!("announce {message}")),
    )
}

/// Checks the countdown and the message of a shutdown request.
pub(crate) fn shutdown_options(
    delay_secs: Option<u32>,
    message: Option<&str>,
) -> Result<(u32, Option<String>), ApiError> {
    let delay = delay_secs.unwrap_or(0);
    if delay > lifecycle::MAX_DELAY_SECS {
        return Err(ApiError::bad_request(format!(
            "delay_secs is at most {}",
            lifecycle::MAX_DELAY_SECS
        )));
    }
    let message = lifecycle::normalize(message).ok_or_else(|| {
        ApiError::bad_request(format!(
            "message is at most {} characters",
            lifecycle::MAX_MESSAGE_CHARS
        ))
    })?;
    Ok((delay, message))
}

/// Reads the optional body of a shutdown request: the countdown and the message.
fn shutdown_args(body: &[u8]) -> Result<(u32, Option<String>), ApiError> {
    let req: ShutdownRequest = if body.iter().all(u8::is_ascii_whitespace) {
        ShutdownRequest::default()
    } else {
        serde_json::from_slice(body)
            .map_err(|e| ApiError::bad_request(format!("bad JSON body: {e}")))?
    };
    shutdown_options(req.delay_secs, req.message.as_deref())
}

async fn request_shutdown(
    s: WebState,
    who: &str,
    kind: ShutdownKind,
    body: &[u8],
) -> Result<(StatusCode, Json<PendingShutdown>), ApiError> {
    let (delay, message) = shutdown_args(body)?;
    match s
        .lifecycle
        .request(s.admin.clone(), who, kind, delay, message.clone())
    {
        Ok(pending) => {
            let detail = format!(
                "in {delay} s{}",
                message.map(|m| format!(": {m}")).unwrap_or_default()
            );
            s.admin
                .record(who, &format!("server {}", kind.as_str()), &detail);
            Ok((StatusCode::ACCEPTED, Json(pending)))
        }
        Err(lifecycle::Refused::Busy(p)) => Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "a {} requested by {} is already pending",
                p.action.as_str(),
                p.requested_by
            ),
        )),
        Err(_) => Err(ApiError::internal("shutdown refused")),
    }
}

/// Stops the server: the players are warned (`delay_secs`, `message`), disconnected, and the
/// process exits with code 0. Answers 202 at once; the countdown runs on the server.
#[utoipa::path(post, path = "/api/v1/admin/server/stop", tag = "admin", security(("bearer" = [])),
    request_body(content = ShutdownRequest, description = "optional"),
    responses((status = 202, body = PendingShutdown), (status = 400, body = crate::ErrorBody), (status = 409, description = "a shutdown is already pending", body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn server_stop(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<PendingShutdown>), ApiError> {
    request_shutdown(s, &who.account.username, ShutdownKind::Stop, &body).await
}

/// Restarts the server: like stop, then the server starts again (`[server] restart_mode`:
/// the process starts itself, or exits with code 75 for a supervisor). Web sessions stay valid.
#[utoipa::path(post, path = "/api/v1/admin/server/restart", tag = "admin", security(("bearer" = [])),
    request_body(content = ShutdownRequest, description = "optional"),
    responses((status = 202, body = PendingShutdown), (status = 400, body = crate::ErrorBody), (status = 409, description = "a shutdown is already pending", body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn server_restart(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<PendingShutdown>), ApiError> {
    request_shutdown(s, &who.account.username, ShutdownKind::Restart, &body).await
}

/// Cancels a pending stop or restart while its countdown runs.
#[utoipa::path(post, path = "/api/v1/admin/server/cancel", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = CommandResult), (status = 404, description = "nothing pending", body = crate::ErrorBody), (status = 409, description = "too late: the players were already disconnected", body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn server_cancel(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
) -> Result<Json<CommandResult>, ApiError> {
    match s.lifecycle.cancel() {
        Ok(p) => {
            let text = format!("server {} cancelled", p.action.as_str());
            s.admin
                .record(&who.account.username, "server cancel", &text);
            let line = format!("announce Server {} cancelled", p.action.as_str());
            let _ = s.admin.run_command(&who.account.username, &line);
            Ok(Json(CommandResult { message: text }))
        }
        Err(lifecycle::Refused::Nothing) => Err(ApiError::not_found("no shutdown is pending")),
        Err(_) => Err(ApiError::new(
            StatusCode::CONFLICT,
            "too late to cancel: the server is going down",
        )),
    }
}

/// The pending stop or restart, if any (with the seconds left).
#[utoipa::path(get, path = "/api/v1/admin/server/shutdown", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = ShutdownStatus), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn server_shutdown(State(s): State<WebState>, _: AdminAuth) -> Json<ShutdownStatus> {
    Json(ShutdownStatus {
        pending: s.lifecycle.status(),
    })
}

/// Every account.
#[utoipa::path(get, path = "/api/v1/admin/accounts", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = Vec<AccountInfo>), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn accounts(
    State(s): State<WebState>,
    _: AdminAuth,
) -> Result<Json<Vec<AccountInfo>>, ApiError> {
    let accounts = s.accounts.clone();
    let list = blocking(move || accounts.list()).await??;
    Ok(Json(list.into_iter().map(Into::into).collect()))
}

/// Runs `account <action> <name> [value]` through the registry and returns the account.
async fn account_action(
    s: &WebState,
    actor: &str,
    action: &str,
    name: &str,
    value: Option<&str>,
) -> Result<Json<AccountInfo>, ApiError> {
    word("account name", name)?;
    let line = match value {
        Some(v) => format!("account {action} {name} {v}"),
        None => format!("account {action} {name}"),
    };
    let (admin, actor) = (s.admin.clone(), actor.to_owned());
    let out = blocking(move || admin.run_command(&actor, &line)).await?;
    let _ = done(out)?;
    let accounts = s.accounts.clone();
    let name = name.to_owned();
    let found = blocking(move || accounts.find(&name)).await??;
    found
        .map(|a| Json(a.into()))
        .ok_or_else(|| ApiError::not_found("no such account"))
}

/// Disables an account: its tokens are revoked and its sessions end.
#[utoipa::path(post, path = "/api/v1/admin/accounts/{name}/disable", tag = "admin", security(("bearer" = [])),
    params(("name" = String, Path, description = "account name")),
    responses((status = 200, body = AccountInfo), (status = 400, description = "e.g. disabling yourself", body = crate::ErrorBody), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn disable_account(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(name): Path<String>,
) -> Result<Json<AccountInfo>, ApiError> {
    account_action(&s, &who.account.username, "disable", &name, None).await
}

/// Enables an account again.
#[utoipa::path(post, path = "/api/v1/admin/accounts/{name}/enable", tag = "admin", security(("bearer" = [])),
    params(("name" = String, Path, description = "account name")),
    responses((status = 200, body = AccountInfo), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn enable_account(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(name): Path<String>,
) -> Result<Json<AccountInfo>, ApiError> {
    account_action(&s, &who.account.username, "enable", &name, None).await
}

/// Sets an account's role (`player` or `admin`). You cannot demote yourself.
#[utoipa::path(post, path = "/api/v1/admin/accounts/{name}/role", tag = "admin", security(("bearer" = [])),
    params(("name" = String, Path, description = "account name")), request_body = RoleRequest,
    responses((status = 200, body = AccountInfo), (status = 400, body = crate::ErrorBody), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn set_role(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    Path(name): Path<String>,
    ApiJson(req): ApiJson<RoleRequest>,
) -> Result<Json<AccountInfo>, ApiError> {
    let role = Role::parse(&req.role)
        .ok_or_else(|| ApiError::bad_request("role must be player or admin"))?;
    account_action(
        &s,
        &who.account.username,
        "role",
        &name,
        Some(role.as_str()),
    )
    .await
}

/// Query of `GET /admin/characters`.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct CharacterQuery {
    /// Only this account's characters (account name).
    pub account: Option<String>,
}

fn character_infos(
    s: &WebState,
    list: Vec<poe_store::Character>,
) -> Result<Vec<CharacterInfo>, ApiError> {
    let online: Vec<String> = s
        .admin
        .online()
        .into_iter()
        .map(|o| o.character.to_lowercase())
        .collect();
    let mut names: HashMap<u32, Option<String>> = HashMap::new();
    let mut out = Vec::with_capacity(list.len());
    for c in list {
        let account = match names.get(&c.account_id) {
            Some(n) => n.clone(),
            None => {
                let n = s.accounts.find_by_id(c.account_id)?.map(|a| a.username);
                names.insert(c.account_id, n.clone());
                n
            }
        };
        let is_online = online.contains(&c.name.to_lowercase());
        out.push(CharacterInfo::new(c, account, is_online));
    }
    Ok(out)
}

/// Characters of all accounts, or of one (`?account=`).
#[utoipa::path(get, path = "/api/v1/admin/characters", tag = "admin", security(("bearer" = [])),
    params(CharacterQuery),
    responses((status = 200, body = Vec<CharacterInfo>), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn characters(
    State(s): State<WebState>,
    _: AdminAuth,
    Query(q): Query<CharacterQuery>,
) -> Result<Json<Vec<CharacterInfo>>, ApiError> {
    let state = s.clone();
    let list = blocking(move || -> Result<Vec<CharacterInfo>, ApiError> {
        let list = match q.account {
            Some(name) => {
                let account = state
                    .accounts
                    .find(&name)?
                    .ok_or_else(|| ApiError::not_found("no such account"))?;
                state.store.characters(account.id)?
            }
            None => state.store.all_characters()?,
        };
        character_infos(&state, list)
    })
    .await??;
    Ok(Json(list))
}

/// One character.
#[utoipa::path(get, path = "/api/v1/admin/characters/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = u32, Path, description = "character id")),
    responses((status = 200, body = CharacterInfo), (status = 404, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn character(
    State(s): State<WebState>,
    _: AdminAuth,
    Path(id): Path<u32>,
) -> Result<Json<CharacterInfo>, ApiError> {
    let state = s.clone();
    let info = blocking(move || -> Result<CharacterInfo, ApiError> {
        let c = state
            .store
            .character_by_id(id)?
            .ok_or_else(|| ApiError::not_found("no such character"))?;
        Ok(character_infos(&state, vec![c])?.remove(0))
    })
    .await??;
    Ok(Json(info))
}

/// Messages the log WebSocket sends.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum LogMessage<'a> {
    /// First message: the buffer size.
    Hello {
        buffer: usize,
    },
    Event {
        event: &'a LogEvent,
    },
    /// The filter changed: what follows is the new history.
    Reset,
    /// The client read too slowly and missed events.
    Lagged {
        missed: u64,
    },
    Error {
        message: String,
    },
}

async fn send(socket: &mut WebSocket, m: LogMessage<'_>) -> bool {
    match serde_json::to_string(&m) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => true,
    }
}

async fn log_session(mut socket: WebSocket, logs: std::sync::Arc<LogBuffer>) {
    if !send(
        &mut socket,
        LogMessage::Hello {
            buffer: logs.capacity(),
        },
    )
    .await
    {
        return;
    }
    // The first message is the filter; defaults if it does not come soon.
    let mut filter = LogFilter::default();
    match tokio::time::timeout(std::time::Duration::from_millis(500), socket.recv()).await {
        Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str(&t) {
            Ok(f) => filter = f,
            Err(e) => {
                send(
                    &mut socket,
                    LogMessage::Error {
                        message: format!("bad filter: {e}"),
                    },
                )
                .await;
            }
        },
        Ok(None | Some(Err(_) | Ok(Message::Close(_)))) => return,
        _ => {}
    }
    let (mut history, mut rx) = logs.subscribe(&filter);
    loop {
        for e in history.drain(..) {
            if !send(&mut socket, LogMessage::Event { event: &e }).await {
                return;
            }
        }
        tokio::select! {
            msg = socket.recv() => match msg {
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<LogFilter>(&t) {
                    Ok(f) => {
                        filter = f;
                        (history, rx) = logs.subscribe(&filter);
                        if !send(&mut socket, LogMessage::Reset).await {
                            return;
                        }
                    }
                    Err(e) => {
                        if !send(&mut socket, LogMessage::Error { message: format!("bad filter: {e}") }).await {
                            return;
                        }
                    }
                },
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
            ev = rx.recv() => match ev {
                Ok(e) => {
                    if filter.matches(&e) && !send(&mut socket, LogMessage::Event { event: &e }).await {
                        return;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    if !send(&mut socket, LogMessage::Lagged { missed: n }).await {
                        return;
                    }
                }
                Err(RecvError::Closed) => return,
            },
        }
    }
}

/// Query of the log WebSocket.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[allow(dead_code)]
pub struct WsQuery {
    /// A ticket from `POST /admin/logs/ws-ticket`, because browsers cannot set headers on a
    /// WebSocket (the `Authorization` header with the admin's web token works too).
    pub ticket: Option<String>,
}

/// A single-use ticket for the log WebSocket, valid for `[web] web_token_ttl_secs`. Pass it as
/// `?ticket=` to `GET /admin/logs/ws`; ask again for every connection.
#[utoipa::path(post, path = "/api/v1/admin/logs/ws-ticket", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = WsTicketResponse), (status = 401, body = crate::ErrorBody),
        (status = 403, body = crate::ErrorBody), (status = 503, body = crate::ErrorBody)))]
pub async fn logs_ws_ticket(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
) -> Result<
    (
        [(axum::http::HeaderName, &'static str); 1],
        Json<WsTicketResponse>,
    ),
    ApiError,
> {
    let ttl = s.config.web.web_token_ttl_secs.max(1);
    let ticket = s
        .ws_tickets
        .mint(
            who.account.id,
            std::time::Duration::from_secs(ttl),
            std::time::Instant::now(),
        )
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "too many unused tickets, try again shortly",
            )
        })?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(WsTicketResponse {
            ticket: hex::encode(ticket),
            expires_in_secs: ttl,
        }),
    ))
}

/// Live log over WebSocket (upgrade). Server messages are JSON with a `type` of `hello`
/// (`buffer`), `event` (a `LogEvent`), `reset` (the filter changed, new history follows),
/// `lagged` (`missed`) or `error`. The client sends a `LogFilter` as JSON text, first and
/// whenever it changes: `{"min_level":"info","target":"poe_net","text":"kick","history":200}`.
#[utoipa::path(get, path = "/api/v1/admin/logs/ws", tag = "admin", security(("bearer" = [])),
    params(WsQuery),
    responses((status = 101, description = "switching protocols"), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn logs_ws(State(s): State<WebState>, _: WsAdminAuth, ws: WebSocketUpgrade) -> Response {
    let logs = s.logs.clone();
    ws.on_upgrade(move |socket| log_session(socket, logs))
}
