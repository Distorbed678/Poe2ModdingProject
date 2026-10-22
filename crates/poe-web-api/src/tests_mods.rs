//! The mod endpoints: `GET /server` advertises the client mods in the shape the launcher
//! consumes, `GET /mods/{id}/{version}.zip` serves exactly the advertised package, and
//! `GET /admin/mods` lists everything for the web admin.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use poe_accounts::Role;
use poe_cluster::{ModAdvert, ModDirectory, ModPackage, ModStatus};
use serde_json::json;
use tower::ServiceExt;

use crate::tests::{Fixture, fixture, fixture_with};

const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

struct Mods {
    restart: bool,
}

impl ModDirectory for Mods {
    fn adverts(&self) -> Vec<ModAdvert> {
        vec![ModAdvert {
            id: "hello-both".into(),
            name: "Hello (both)".into(),
            version: "0.1.0".into(),
            side: "both".into(),
            sha256: SHA.into(),
            size: 3,
            required: true,
            hook_api: Some(1),
            game_versions: vec!["0.5.5e".into()],
            contains_code: true,
            description: "Says hello".into(),
            authors: vec!["Distorbed".into()],
        }]
    }

    fn package(&self, id: &str, version: &str) -> Option<ModPackage> {
        (id == "hello-both" && version == "0.1.0").then(|| ModPackage { sha256: SHA.into(), bytes: b"abc".to_vec() })
    }

    fn status(&self) -> Vec<ModStatus> {
        vec![
            ModStatus { id: "hello-both".into(), name: "Hello (both)".into(), version: "0.1.0".into(), side: "both".into(), enabled: true, state: "loaded".into(), detail: String::new(), commands: vec!["hello".into()] },
            ModStatus { id: "old".into(), name: "Old".into(), version: "1.0.0".into(), side: "server".into(), enabled: true, state: "skipped".into(), detail: "built for server API 2".into(), commands: vec![] },
        ]
    }

    fn active(&self) -> bool {
        true
    }

    fn restart_required(&self) -> bool {
        self.restart
    }
}

fn with_mods(config: &str, mods: impl ModDirectory + 'static) -> Fixture {
    let mut f = fixture_with(config);
    f.state.mods = Arc::new(mods);
    f.app = crate::router(f.state.clone());
    f
}

#[tokio::test]
async fn a_server_without_mods_has_no_mods_field() {
    let f = fixture();
    let (status, body) = f.call("GET", "/api/v1/server", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("mods").is_none(), "{body}");
}

/// The JSON the launcher parses. A change here is a change of the launcher's contract.
#[tokio::test]
async fn the_advertisement_has_the_shape_the_launcher_consumes() {
    let f = with_mods(&format!("{}\n[mods]\nextra_urls = [\"https://example.invalid/{{id}}-{{version}}.zip\"]\n", crate::tests::CONFIG), Mods { restart: false });
    let (status, body) = f.call("GET", "/api/v1/server", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["mods"],
        json!([{
            "id": "hello-both",
            "name": "Hello (both)",
            "version": "0.1.0",
            "side": "both",
            "sha256": SHA,
            "size": 3,
            "required": true,
            "hook_api": 1,
            "game_versions": ["0.5.5e"],
            "code": true,
            "authors": ["Distorbed"],
            "description": "Says hello",
            "sources": [
                {"type": "server", "path": "/api/v1/mods/hello-both/0.1.0.zip"},
                {"type": "url", "url": "https://example.invalid/hello-both-0.1.0.zip"}
            ]
        }])
    );
}

async fn get_raw(f: &Fixture, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method("GET").uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = f.app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, bytes)
}

#[tokio::test]
async fn the_package_download_is_public_cacheable_and_pinned_by_hash() {
    let f = with_mods(crate::tests::CONFIG, Mods { restart: false });
    let (status, h, bytes) = get_raw(&f, "/api/v1/mods/hello-both/0.1.0.zip", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"abc");
    assert_eq!(h[header::CONTENT_TYPE], "application/zip");
    assert_eq!(h[header::ETAG], format!("\"{SHA}\""));
    assert!(h[header::CACHE_CONTROL].to_str().unwrap().contains("public"));
    assert!(h[header::CONTENT_DISPOSITION].to_str().unwrap().contains("hello-both-0.1.0.zip"));
    // Revalidation by ETag.
    let (status, h, bytes) = get_raw(&f, "/api/v1/mods/hello-both/0.1.0.zip", &[("if-none-match", &format!("\"{SHA}\""))]).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(bytes.is_empty());
    assert_eq!(h[header::ETAG], format!("\"{SHA}\""));
    // Unknown id, unknown version, a path that is not a version, other junk: all 404.
    for uri in [
        "/api/v1/mods/nope/0.1.0.zip",
        "/api/v1/mods/hello-both/9.9.9.zip",
        "/api/v1/mods/hello-both/0.1.0",
        "/api/v1/mods/hello-both/.zip",
        "/api/v1/mods/hello-both/..%2f..%2fetc.zip",
    ] {
        assert_eq!(get_raw(&f, uri, &[]).await.0, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn server_only_mods_are_not_downloadable_without_a_directory_entry() {
    // The default directory (no mods) serves nothing.
    let f = fixture();
    assert_eq!(get_raw(&f, "/api/v1/mods/hello-both/0.1.0.zip", &[]).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_admin_list_needs_an_admin_and_reports_restart_required() {
    let f = with_mods(crate::tests::CONFIG, Mods { restart: true });
    f.account("boss", "correct horse battery", Role::Admin);
    f.account("pawn", "correct horse battery", Role::Player);
    let (status, _) = f.call("GET", "/api/v1/admin/mods", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let pawn = f.login("pawn", "correct horse battery").await;
    assert_eq!(f.call("GET", "/api/v1/admin/mods", Some(&pawn), None).await.0, StatusCode::FORBIDDEN);
    let boss = f.login("boss", "correct horse battery").await;
    let (status, body) = f.call("GET", "/api/v1/admin/mods", Some(&boss), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["active"], true);
    assert_eq!(body["restart_required"], true);
    assert_eq!(body["require_client_mods"], true);
    assert_eq!(body["allow_extra_client_mods"], false);
    assert_eq!(body["mods"][0]["id"], "hello-both");
    assert_eq!(body["mods"][0]["commands"], json!(["hello"]));
    assert_eq!(body["mods"][1]["state"], "skipped");
    assert_eq!(body["mods"][1]["detail"], "built for server API 2");
}
