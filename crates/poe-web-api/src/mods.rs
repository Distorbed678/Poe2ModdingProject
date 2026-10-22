//! The server's mods over HTTP: the client mods in `GET /server`, the package download
//! (`GET /mods/{id}/{version}.zip`) and the admin's list (`GET /admin/mods`).
//!
//! The download is public like `/server` (a player needs the mods before logging in). It
//! serves exactly the package whose SHA-256 the server advertises, verified at startup and
//! again when read; the `ETag` is that hash, so clients and caches revalidate cheaply.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use poe_cluster::ModAdvert;

use crate::{
    ApiError, WebState,
    auth::AdminAuth,
    dto::{ModInfo, ModSource, ModStatusInfo, ModsAdminInfo},
    public::blocking,
};

/// `/api/v1/mods/{id}/{version}.zip`
fn package_path(id: &str, version: &str) -> String {
    format!("/api/v1/mods/{id}/{version}.zip")
}

fn info(a: ModAdvert, extra_urls: &[String]) -> ModInfo {
    let mut sources = vec![ModSource {
        kind: "server".into(),
        path: Some(package_path(&a.id, &a.version)),
        url: None,
    }];
    sources.extend(extra_urls.iter().map(|u| ModSource {
        kind: "url".into(),
        path: None,
        url: Some(u.replace("{id}", &a.id).replace("{version}", &a.version)),
    }));
    ModInfo {
        id: a.id,
        name: a.name,
        version: a.version,
        side: a.side,
        sha256: a.sha256,
        size: a.size,
        required: a.required,
        hook_api: a.hook_api,
        game_versions: a.game_versions,
        code: a.contains_code,
        authors: a.authors,
        description: a.description,
        sources,
    }
}

/// The `mods` field of `GET /server`: `None` when the server offers no client mod.
pub fn advertised(s: &WebState) -> Option<Vec<ModInfo>> {
    let list: Vec<ModInfo> = s
        .mods
        .adverts()
        .into_iter()
        .map(|a| info(a, &s.config.mods.extra_urls))
        .collect();
    (!list.is_empty()).then_some(list)
}

fn valid_part(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
}

/// The package zip of an offered mod. `ETag` is the package's SHA-256; `If-None-Match` gets 304.
#[utoipa::path(get, path = "/api/v1/mods/{id}/{file}", tag = "public",
    params(("id" = String, Path, description = "mod id"), ("file" = String, Path, description = "`<version>.zip`")),
    responses((status = 200, description = "the package (application/zip)", content_type = "application/zip"),
        (status = 304, description = "unchanged"),
        (status = 404, description = "no such offered mod or version", body = crate::ErrorBody)))]
pub async fn package(
    State(s): State<WebState>,
    Path((id, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let version = file.strip_suffix(".zip").unwrap_or("");
    let not_found = || ApiError::not_found("no such mod package");
    if !valid_part(&id) || !valid_part(version) {
        return Err(not_found());
    }
    let etag = s.mods.package_sha256(&id, version).ok_or_else(not_found)?;
    let quoted = format!("\"{etag}\"");
    let cache = (header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
    let tag = (header::ETAG, HeaderValue::from_str(&quoted).map_err(ApiError::internal)?);
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim().trim_start_matches("W/") == quoted));
    if matches {
        return Ok((StatusCode::NOT_MODIFIED, [tag, cache]).into_response());
    }
    let mods = s.mods.clone();
    let (id2, version2) = (id.clone(), version.to_owned());
    let pkg = blocking(move || mods.package(&id2, &version2)).await?.ok_or_else(not_found)?;
    let name = format!("{id}-{version}.zip");
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/zip")),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).map_err(ApiError::internal)?,
            ),
            tag,
            cache,
        ],
        pkg.bytes,
    )
        .into_response())
}

/// Every mod in the server's mods folder with its state, and whether a restart would change
/// what is loaded (admin).
#[utoipa::path(get, path = "/api/v1/admin/mods", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = ModsAdminInfo), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn admin_list(State(s): State<WebState>, _: AdminAuth) -> Result<Json<ModsAdminInfo>, ApiError> {
    let mods = s.mods.clone();
    let (status, restart_required, active) =
        blocking(move || (mods.status(), mods.restart_required(), mods.active())).await?;
    Ok(Json(ModsAdminInfo {
        active,
        mods: status
            .into_iter()
            .map(|m| ModStatusInfo {
                id: m.id,
                name: m.name,
                version: m.version,
                side: m.side,
                enabled: m.enabled,
                state: m.state,
                detail: m.detail,
                commands: m.commands,
            })
            .collect(),
        restart_required,
        require_client_mods: s.config.mods.require_client_mods,
        allow_extra_client_mods: s.config.mods.allow_extra_client_mods,
    }))
}
