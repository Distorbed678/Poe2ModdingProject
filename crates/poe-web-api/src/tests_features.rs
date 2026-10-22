//! Router tests for the web admin features: players, bans, metrics, the config editor, the
//! message of the day and the scheduled tasks. Same fixture as `tests.rs`.

use axum::http::StatusCode;
use poe_accounts::{Role, TokenKind, password_digest};
use poe_cluster::Sample;
use serde_json::{Value, json};

use crate::tests::{CONFIG, Fixture, fixture, fixture_at, fixture_with, repo_root, scratch};

async fn admin(f: &Fixture) -> String {
    f.account("root", "correct horse", Role::Admin);
    f.login("root", "correct horse").await
}

async fn player(f: &Fixture) -> String {
    f.account("alice", "correct horse", Role::Player);
    f.login("alice", "correct horse").await
}

/// A fixture whose config was loaded from a real file under `target/test-tmp`.
fn with_file(extra: &str) -> (Fixture, std::path::PathBuf, std::path::PathBuf) {
    let dir = scratch("cfg");
    let path = dir.join("server.toml");
    let text = format!("{CONFIG}{extra}");
    std::fs::write(&path, &text).unwrap();
    let backups = dir.join("backups");
    // The backup folder is set in the config itself, as in production.
    let text = format!(
        "{text}\n[web]\nconfig_backup_dir = \"{}\"\nconfig_backup_keep = 3\n",
        backups.display()
    );
    std::fs::write(&path, &text).unwrap();
    (fixture_at(&text, Some(path.clone())), path, backups)
}

// ----- every new endpoint needs an admin -----

const ENDPOINTS: &[(&str, &str, Option<&str>)] = &[
    ("GET", "/api/v1/admin/bans", None),
    (
        "POST",
        "/api/v1/admin/bans",
        Some(r#"{"ip":"198.51.100.1"}"#),
    ),
    ("POST", "/api/v1/admin/bans/1/lift", None),
    ("GET", "/api/v1/admin/metrics", None),
    ("POST", "/api/v1/admin/motd", Some(r#"{"motd":"hi"}"#)),
    ("GET", "/api/v1/admin/config", None),
    (
        "POST",
        "/api/v1/admin/config/validate",
        Some(r#"{"content":""}"#),
    ),
    (
        "POST",
        "/api/v1/admin/config/fields",
        Some(r#"{"content":"","set":{}}"#),
    ),
    (
        "POST",
        "/api/v1/admin/config/save",
        Some(r#"{"content":"","base_revision":"x"}"#),
    ),
    ("GET", "/api/v1/admin/config/backups", None),
    ("GET", "/api/v1/admin/config/backups/server.toml.1", None),
    ("GET", "/api/v1/admin/schedule", None),
    (
        "POST",
        "/api/v1/admin/schedule/preview",
        Some(r#"{"schedule":{"type":"daily","time":"04:00"}}"#),
    ),
    (
        "POST",
        "/api/v1/admin/schedule/tasks",
        Some(
            r#"{"name":"x","task":{"kind":"restart"},"schedule":{"type":"daily","time":"04:00"}}"#,
        ),
    ),
    (
        "POST",
        "/api/v1/admin/schedule/tasks/1",
        Some(
            r#"{"name":"x","task":{"kind":"restart"},"schedule":{"type":"daily","time":"04:00"}}"#,
        ),
    ),
    ("POST", "/api/v1/admin/schedule/tasks/1/enable", None),
    ("POST", "/api/v1/admin/schedule/tasks/1/disable", None),
    ("POST", "/api/v1/admin/schedule/tasks/1/delete", None),
    ("GET", "/api/v1/admin/schedule/tasks/1/runs", None),
];

#[tokio::test]
async fn the_new_endpoints_need_a_token_and_the_admin_role() {
    let f = fixture();
    let alice = player(&f).await;
    for (method, uri, body) in ENDPOINTS {
        let body: Option<Value> = body.map(|b| serde_json::from_str(b).unwrap());
        let (status, _) = f.call(method, uri, None, body.clone()).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} without a token"
        );
        let (status, _) = f.call(method, uri, Some(&alice), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri} as a player");
    }
    // A launcher token (long-lived, in the OS keyring) never reaches the admin API.
    f.account("boss", "correct horse", Role::Admin);
    let boss = f.state.accounts.find("boss").unwrap().unwrap();
    let launcher = hex::encode(
        f.state
            .accounts
            .issue_token(boss.id, TokenKind::Launcher, 3600)
            .unwrap()
            .token,
    );
    for (method, uri, body) in ENDPOINTS {
        let body: Option<Value> = body.map(|b| serde_json::from_str(b).unwrap());
        let (status, _) = f.call(method, uri, Some(&launcher), body).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} with a launcher token"
        );
    }
    assert!(f.calls().is_empty(), "nothing was audited or run");
}

// ----- players -----

#[tokio::test]
async fn sessions_list_the_players_with_character_details() {
    let f = fixture();
    let t = admin(&f).await;
    let owner = f
        .state
        .accounts
        .create_account("bobby", &password_digest("correct horse"), Role::Player)
        .unwrap();
    f.state
        .store
        .create_character(&poe_store::NewCharacter {
            account_id: owner.id,
            name: "Bob".into(),
            league: "Standard".into(),
            class_id: 3,
            start_state: "x".into(),
            level: 17,
            area: "G1_1".into(),
        })
        .unwrap();
    let (status, body) = f
        .call("GET", "/api/v1/admin/sessions", Some(&t), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = &body[0];
    assert_eq!(row["character"], "Bob");
    assert_eq!(row["account"], "bobby");
    assert_eq!(row["level"], 17);
    assert_eq!(row["class_id"], 3);
    assert_eq!(row["connected_at"], 1_700_000_000);
    assert_eq!(row["ip"], "203.0.113.9");
    assert_eq!(row["muted"], false);
    assert_eq!(row["instance"], 1);
}

// ----- bans -----

fn ban_body(v: Value) -> Option<Value> {
    Some(v)
}

#[tokio::test]
async fn banning_an_account_refuses_its_logins_and_ends_its_sessions() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    f.account("alice", "correct horse", Role::Player);
    let alice_token = f.login("alice", "correct horse").await;

    let (status, ban) = f
        .call("POST", "/api/v1/admin/bans", t, ban_body(json!({"account": "Alice", "reason": "  cheating   in the hideout ", "duration_secs": 3600})))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ban}");
    assert_eq!(
        (
            ban["kind"].as_str(),
            ban["account"].as_str(),
            ban["reason"].as_str(),
            ban["status"].as_str()
        ),
        (
            Some("account"),
            Some("alice"),
            Some("cheating in the hideout"),
            Some("active")
        )
    );
    assert_eq!(ban["created_by"], "root");
    let id = ban["id"].as_u64().unwrap();
    let expires = ban["expires_at"].as_i64().unwrap();
    assert!((expires - ban["created_at"].as_i64().unwrap() - 3600).abs() <= 2);
    // Her sessions end now, the action is audited.
    let calls = f.calls();
    let alice = f.state.accounts.find("alice").unwrap().unwrap();
    assert!(
        calls.contains(&format!("end_sessions {}", alice.id)),
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("root: record ban account [alice until ")
                && c.contains("cheating in the hideout")),
        "{calls:?}"
    );

    // Her web token is dead; a new login says why, with the ban's text; wrong password does not.
    let (status, _) = f.call("GET", "/api/v1/me", Some(&alice_token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "correct horse"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let text = body["error"].as_str().unwrap();
    assert!(
        text.contains("banned until")
            && text.contains("UTC")
            && text.contains("Reason: cheating in the hideout."),
        "{text}"
    );
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "wrong horse 1"})),
        )
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (
            StatusCode::UNAUTHORIZED,
            Some("invalid username or password")
        )
    );

    // The list: newest first, the active filter, history after lifting.
    let (_, all) = f.call("GET", "/api/v1/admin/bans", t, None).await;
    assert_eq!(
        (all.as_array().unwrap().len(), all[0]["id"].as_u64()),
        (1, Some(id))
    );
    let (status, ban) = f
        .call("POST", &format!("/api/v1/admin/bans/{id}/lift"), t, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{ban}");
    assert_eq!(
        (ban["status"].as_str(), ban["lifted_by"].as_str()),
        (Some("lifted"), Some("root"))
    );
    assert!(
        f.calls()
            .iter()
            .any(|c| c == "root: record unban account [alice]")
    );
    let (_, active) = f
        .call("GET", "/api/v1/admin/bans?active=true", t, None)
        .await;
    assert!(active.as_array().unwrap().is_empty());
    let (_, all) = f.call("GET", "/api/v1/admin/bans", t, None).await;
    assert_eq!(all.as_array().unwrap().len(), 1, "the history keeps it");
    // She can log in again; lifting twice or an unknown id fails.
    f.login("alice", "correct horse").await;
    let (status, _) = f
        .call("POST", &format!("/api/v1/admin/bans/{id}/lift"), t, None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = f
        .call("POST", "/api/v1/admin/bans/9999/lift", t, None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_ban_without_an_end_is_permanent_and_expires_otherwise() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    f.account("alice", "correct horse", Role::Player);
    let (status, ban) = f
        .call(
            "POST",
            "/api/v1/admin/bans",
            t,
            ban_body(json!({"account": "alice"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ban}");
    assert!(ban["expires_at"].is_null() && ban["reason"] == "");
    let (_, body) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "correct horse"})),
        )
        .await;
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("banned permanently"),
        "{body}"
    );
    // A second active ban of the same account is refused.
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/bans",
            t,
            ban_body(json!({"account": "alice"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // An expiry date works like a duration: in the past it is refused.
    f.account("carol", "correct horse", Role::Player);
    let soon = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 120;
    let (status, ban) = f
        .call(
            "POST",
            "/api/v1/admin/bans",
            t,
            ban_body(json!({"account": "carol", "expires_at": soon})),
        )
        .await;
    assert_eq!(
        (status, ban["expires_at"].as_i64()),
        (StatusCode::CREATED, Some(soon))
    );
    // The account store's clock is real here; an expired row would need time to pass, which
    // `poe-accounts` tests with a controlled clock. The row's status is derived from the time.
    assert_eq!(ban["status"], "active");
}

#[tokio::test]
async fn ban_requests_are_validated() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    f.account("alice", "correct horse", Role::Player);
    f.account("mod", "correct horse", Role::Admin);
    let long = "x".repeat(201);
    for (body, status, what) in [
        (json!({}), StatusCode::BAD_REQUEST, "neither account nor ip"),
        (
            json!({"account": "alice", "ip": "198.51.100.1"}),
            StatusCode::BAD_REQUEST,
            "both",
        ),
        (
            json!({"account": "alice", "duration_secs": 59}),
            StatusCode::BAD_REQUEST,
            "too short",
        ),
        (
            json!({"account": "alice", "duration_secs": 99_999_999_999u64}),
            StatusCode::BAD_REQUEST,
            "too long",
        ),
        (
            json!({"account": "alice", "duration_secs": 60, "expires_at": 1}),
            StatusCode::BAD_REQUEST,
            "duration and expiry",
        ),
        (
            json!({"account": "alice", "expires_at": 1}),
            StatusCode::BAD_REQUEST,
            "expiry in the past",
        ),
        (
            json!({"account": "alice", "reason": long}),
            StatusCode::BAD_REQUEST,
            "reason too long",
        ),
        (
            json!({"account": "nobody"}),
            StatusCode::NOT_FOUND,
            "unknown account",
        ),
        (
            json!({"account": "root"}),
            StatusCode::BAD_REQUEST,
            "yourself",
        ),
        (
            json!({"account": "mod"}),
            StatusCode::BAD_REQUEST,
            "another admin",
        ),
        (
            json!({"ip": "not an ip"}),
            StatusCode::BAD_REQUEST,
            "bad address",
        ),
        (
            json!({"ip": "198.51.100.0/24"}),
            StatusCode::BAD_REQUEST,
            "a range",
        ),
        // The fixture's requests come from 0.0.0.0 (no socket): banning it would lock out the caller.
        (
            json!({"ip": "0.0.0.0"}),
            StatusCode::BAD_REQUEST,
            "your own address",
        ),
        (
            json!({"account": 5}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong type",
        ),
    ] {
        let (got, body) = f.call("POST", "/api/v1/admin/bans", t, Some(body)).await;
        assert_eq!(got, status, "{what}: {body}");
    }
    let (_, list) = f.call("GET", "/api/v1/admin/bans", t, None).await;
    assert!(list.as_array().unwrap().is_empty(), "nothing was stored");
    assert!(
        !f.calls().iter().any(|c| c.contains("ban")),
        "nothing audited"
    );
}

#[tokio::test]
async fn an_ip_ban_refuses_logins_and_registration_from_the_address() {
    let f = fixture_with(&crate::tests::open_registration());
    let t = admin(&f).await;
    let t = Some(t.as_str());
    f.account("alice", "correct horse", Role::Player);
    // The test client has no socket, so it counts as 0.0.0.0: ban it through the store, as the
    // endpoint refuses the caller's own address.
    let ban = f
        .state
        .accounts
        .ban_ip("0.0.0.0", "spam", "root", None)
        .unwrap();
    for (uri, body) in [
        (
            "/api/v1/login",
            json!({"username": "alice", "password": "correct horse"}),
        ),
        (
            "/api/v1/register",
            json!({"username": "newbie", "password": "correct horse"}),
        ),
    ] {
        let (status, body) = f.call("POST", uri, None, Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {body}");
        assert_eq!(
            body["error"],
            "Connections from your address are banned permanently. Reason: spam."
        );
    }
    assert!(f.state.accounts.find("newbie").unwrap().is_none());
    // Lifting restores access.
    f.state.accounts.lift_ban(ban.id, "root").unwrap();
    f.login("alice", "correct horse").await;

    // An address ban made through the endpoint.
    let (status, ban) = f
        .call(
            "POST",
            "/api/v1/admin/bans",
            t,
            ban_body(json!({"ip": "::ffff:198.51.100.7", "reason": "bots", "duration_secs": 600})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ban}");
    assert_eq!(
        (
            ban["kind"].as_str(),
            ban["ip"].as_str(),
            ban["account"].is_null()
        ),
        (Some("ip"), Some("198.51.100.7"), true)
    );
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/bans",
            t,
            ban_body(json!({"ip": "198.51.100.7"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        f.calls()
            .iter()
            .any(|c| c.starts_with("root: record ban ip [198.51.100.7 until ")),
        "{:?}",
        f.calls()
    );
}

#[tokio::test]
async fn a_web_sign_in_token_of_a_banned_account_is_refused() {
    let f = fixture();
    f.account("boss", "correct horse", Role::Admin);
    let boss = f.state.accounts.find("boss").unwrap().unwrap();
    let ticket = f
        .state
        .web_tokens
        .mint(
            boss.id,
            std::time::Duration::from_secs(60),
            std::time::Instant::now(),
        )
        .unwrap();
    f.state
        .accounts
        .ban_account(boss.id, "audit", "root", None)
        .unwrap();
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/web-token/redeem",
            None,
            Some(json!({"token": hex::encode(ticket)})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("banned permanently")
    );
}

// ----- metrics -----

fn sample(t_ms: i64, players: u32) -> Sample {
    Sample {
        t_ms,
        players,
        instances: 2,
        occupied: 1,
        tick_count: 30,
        tick_avg_us: 800,
        tick_max_us: 2500,
        rss_bytes: 50_000_000,
        cpu_pct: Some(4.5),
    }
}

#[tokio::test]
async fn metrics_return_the_ring_buffer() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let (status, body) = f.call("GET", "/api/v1/admin/metrics", t, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            body["enabled"].as_bool(),
            body["interval_secs"].as_u64(),
            body["retention_secs"].as_u64()
        ),
        (Some(true), Some(5), Some(86_400))
    );
    assert!(body["samples"].as_array().unwrap().is_empty());
    assert!(body["now"].as_i64().unwrap() > 1_700_000_000_000);

    let now = body["now"].as_i64().unwrap();
    for i in 0..10 {
        f.state
            .metrics
            .push(sample(now - (10 - i) * 5000, i as u32));
    }
    let (_, body) = f.call("GET", "/api/v1/admin/metrics", t, None).await;
    let samples = body["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 10);
    assert_eq!(samples[9]["players"], 9);
    assert_eq!(
        (
            samples[0]["instances"].as_u64(),
            samples[0]["tick_avg_us"].as_u64(),
            samples[0]["tick_max_us"].as_u64(),
            samples[0]["rss_bytes"].as_u64(),
            samples[0]["cpu_pct"].as_f64()
        ),
        (Some(2), Some(800), Some(2500), Some(50_000_000), Some(4.5))
    );
    // Only what is new since a time; the last N seconds; a point budget.
    let since = samples[7]["t"].as_i64().unwrap();
    let (_, body) = f
        .call(
            "GET",
            &format!("/api/v1/admin/metrics?since={since}"),
            t,
            None,
        )
        .await;
    assert_eq!(body["samples"].as_array().unwrap().len(), 2);
    let (_, body) = f
        .call("GET", "/api/v1/admin/metrics?range_secs=12", t, None)
        .await;
    assert_eq!(
        body["samples"].as_array().unwrap().len(),
        2,
        "the samples of the last 12 s (5 s apart)"
    );
    let (_, body) = f
        .call("GET", "/api/v1/admin/metrics?max_points=2", t, None)
        .await;
    let merged = body["samples"].as_array().unwrap();
    assert_eq!(
        (
            merged.len(),
            merged[0]["players"].as_u64(),
            merged[1]["players"].as_u64()
        ),
        (2, Some(4), Some(9)),
        "buckets keep the peak"
    );
    // Bad budgets.
    for q in [
        "max_points=1",
        "max_points=5001",
        "max_points=0",
        "since=abc",
    ] {
        let (status, _) = f
            .call("GET", &format!("/api/v1/admin/metrics?{q}"), t, None)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{q}");
    }
}

#[tokio::test]
async fn metrics_can_be_switched_off() {
    let f = fixture_with(&format!(
        "{CONFIG}\n[metrics]\nenabled = false\ninterval_secs = 10\nretention_hours = 2\n"
    ));
    let t = admin(&f).await;
    f.state.metrics.push(sample(1, 1));
    let (_, body) = f.call("GET", "/api/v1/admin/metrics", Some(&t), None).await;
    assert_eq!(
        (
            body["enabled"].as_bool(),
            body["interval_secs"].as_u64(),
            body["retention_secs"].as_u64()
        ),
        (Some(false), Some(10), Some(7200))
    );
    assert!(body["samples"].as_array().unwrap().is_empty());
}

// ----- message of the day -----

#[tokio::test]
async fn the_message_of_the_day_is_public_and_editable() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let (_, info) = f.call("GET", "/api/v1/server", None, None).await;
    assert!(
        info.get("motd").is_none(),
        "absent when there is none: {info}"
    );

    // No config file in this fixture: it applies live, and says it was not saved.
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/motd",
            t,
            Some(json!({"motd": "  Welcome!\r\nPatch 5 is out.  "})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["motd"].as_str(), body["persisted"].as_bool()),
        (Some("Welcome!\nPatch 5 is out."), Some(false))
    );
    assert!(body["note"].as_str().unwrap().contains("config file"));
    let (_, info) = f.call("GET", "/api/v1/server", None, None).await;
    assert_eq!(info["motd"], "Welcome!\nPatch 5 is out.");
    assert!(
        f.calls()
            .iter()
            .any(|c| c.starts_with("root: record motd ["))
    );

    // Cleared again.
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/motd",
            t,
            Some(json!({"motd": "   "})),
        )
        .await;
    assert_eq!((status, body["motd"].as_str()), (StatusCode::OK, Some("")));
    let (_, info) = f.call("GET", "/api/v1/server", None, None).await;
    assert!(info.get("motd").is_none());

    // Limits: 280 characters, no control characters, a string.
    let long = "x".repeat(281);
    for body in [json!({"motd": long}), json!({"motd": "bell\u{7}"})] {
        let (status, body) = f.call("POST", "/api/v1/admin/motd", t, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let (status, _) = f
        .call("POST", "/api/v1/admin/motd", t, Some(json!({"motd": 5})))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/motd",
            t,
            Some(json!({"motd": "x".repeat(280)})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "280 characters fit");
}

#[tokio::test]
async fn the_message_of_the_day_is_written_into_the_config_file() {
    let (f, path, backups) = with_file("\n[server]\nrestart_mode = \"exit\"  # keep\n");
    let t = admin(&f).await;
    let before = std::fs::read_to_string(&path).unwrap();
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/motd",
            Some(&t),
            Some(json!({"motd": "Maintenance on Friday"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["persisted"].as_bool(), body["note"].is_null()),
        (Some(true), true)
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("motd = \"Maintenance on Friday\""),
        "{after}"
    );
    assert!(
        after.contains("restart_mode = \"exit\"  # keep"),
        "comments stay"
    );
    // The file still parses, and the previous version is in the backups.
    poe_config::Config::parse(&after).unwrap();
    let names: Vec<String> = std::fs::read_dir(&backups)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert_eq!(
        std::fs::read_to_string(backups.join(&names[0])).unwrap(),
        before
    );
}

#[tokio::test]
async fn the_message_of_the_day_stays_in_memory_when_the_editor_is_off() {
    let (_, path, _) = with_file("\n[server]\nrestart_mode = \"exit\"\n");
    // The same file with the editor off.
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("[web]\n", "[web]\nconfig_editor = false\n");
    std::fs::write(&path, &text).unwrap();
    let g = fixture_at(&text, Some(path.clone()));
    let t = admin(&g).await;
    let (status, body) = g
        .call(
            "POST",
            "/api/v1/admin/motd",
            Some(&t),
            Some(json!({"motd": "hello"})),
        )
        .await;
    assert_eq!(
        (status, body["persisted"].as_bool()),
        (StatusCode::OK, Some(false)),
        "{body}"
    );
    assert!(body["note"].as_str().unwrap().contains("config_editor"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        text,
        "the file was not touched"
    );
    let (_, info) = g.call("GET", "/api/v1/server", None, None).await;
    assert_eq!(info["motd"], "hello");
}

// ----- config editor -----

#[tokio::test]
async fn the_config_is_read_with_its_form_schema() {
    let (f, path, backups) = with_file(
        "\n[server]\nrestart_mode = \"exit\"\nmotd = \"Hello\"\n[security]\nlogin_lockout_secs = 99\n",
    );
    let t = admin(&f).await;
    let (status, body) = f.call("GET", "/api/v1/admin/config", Some(&t), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["path"].as_str().map(std::path::Path::new),
        Some(path.as_path())
    );
    assert_eq!(
        body["content"].as_str().unwrap(),
        std::fs::read_to_string(&path).unwrap()
    );
    assert_eq!(body["editable"], true);
    assert_eq!(
        body["backup_dir"].as_str().map(std::path::Path::new),
        Some(backups.as_path())
    );
    assert_eq!(body["revision"].as_str().unwrap().len(), 16);
    // The values the file sets, by `section.key`; keys it leaves out are absent.
    assert_eq!(body["values"]["server.restart_mode"], "exit");
    assert_eq!(body["values"]["server.motd"], "Hello");
    assert_eq!(body["values"]["security.login_lockout_secs"], 99);
    assert!(body["values"].get("web.log_buffer").is_none());
    let fields = body["fields"].as_array().unwrap();
    let find = |p: &str| {
        fields
            .iter()
            .find(|f| f["path"] == p)
            .unwrap_or_else(|| panic!("{p}"))
    };
    assert_eq!(
        (
            find("server.motd")["kind"].as_str(),
            find("server.motd")["multiline"].as_bool(),
            find("server.motd")["restart"].as_bool(),
            find("server.motd")["max_len"].as_u64()
        ),
        (Some("text"), Some(true), Some(false), Some(280))
    );
    assert_eq!(
        (
            find("metrics.interval_secs")["min"].as_i64(),
            find("metrics.interval_secs")["max"].as_i64(),
            find("metrics.interval_secs")["default"].as_i64()
        ),
        (Some(1), Some(3600), Some(5))
    );
    assert_eq!(
        find("server.restart_mode")["options"],
        json!(["exec", "exit"])
    );
    assert_eq!(find("web.cors_origins")["kind"], "list");
    assert!(
        body["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("next start"))
    );
    assert!(
        body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| !w.as_str().unwrap().is_empty())
    );
}

#[tokio::test]
async fn the_config_endpoints_need_a_config_file() {
    // The default fixture was parsed from text: there is no file to edit.
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    for (method, uri, body) in [
        ("GET", "/api/v1/admin/config", None),
        (
            "POST",
            "/api/v1/admin/config/save",
            Some(json!({"content": CONFIG, "base_revision": "x"})),
        ),
        ("GET", "/api/v1/admin/config/backups", None),
    ] {
        let (status, body) = f.call(method, uri, t, body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{uri}: {body}");
    }
    // Validating and form edits work on text alone.
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/validate",
            t,
            Some(json!({"content": CONFIG})),
        )
        .await;
    assert_eq!(
        (status, body["valid"].as_bool()),
        (StatusCode::OK, Some(true)),
        "{body}"
    );
}

#[tokio::test]
async fn validation_reports_the_line_and_column_of_the_problem() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let validate = |content: String| {
        let f = &f;
        async move {
            f.call(
                "POST",
                "/api/v1/admin/config/validate",
                t,
                Some(json!({"content": content})),
            )
            .await
        }
    };
    let (status, body) = validate(CONFIG.into()).await;
    assert_eq!(
        (status, body["valid"].as_bool(), body["error"].is_null()),
        (StatusCode::OK, Some(true), true),
        "{body}"
    );

    // Syntax error on a known line.
    let broken = format!("{CONFIG}\n[web]\nlog_buffer = = 5\n");
    let line = broken
        .lines()
        .position(|l| l.starts_with("log_buffer"))
        .unwrap()
        + 1;
    let (_, body) = validate(broken).await;
    assert_eq!(body["valid"], false);
    assert_eq!(body["error"]["line"].as_u64(), Some(line as u64), "{body}");
    assert!(
        body["error"]["column"].as_u64().unwrap() > 1
            && !body["error"]["message"].as_str().unwrap().is_empty()
    );

    // A wrong type, a value out of range, an unknown game version.
    let (_, body) = validate(format!("{CONFIG}\n[web]\nlog_buffer = \"lots\"\n")).await;
    assert_eq!(
        (body["valid"].as_bool(), body["error"]["line"].is_number()),
        (Some(false), true),
        "{body}"
    );
    let (_, body) = validate(format!("{CONFIG}\n[metrics]\ninterval_secs = 0\n")).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("interval_secs"),
        "{body}"
    );
    let (_, body) = validate(CONFIG.replace("0.5.5e", "9.9.9")).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("version"),
        "{body}"
    );
    // Valid but risky: a warning, no error.
    let (_, body) = validate(format!("{CONFIG}\n[web]\nbind = \"0.0.0.0:8080\"\n")).await;
    assert_eq!(body["valid"], true);
    assert!(
        body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("not loopback")),
        "{body}"
    );
    // Too large.
    let (status, _) = validate("#".repeat(crate::configfile::MAX_CONFIG_BYTES + 1)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn form_values_are_written_into_the_text_with_comments_kept() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let text = format!(
        "# my server\n{CONFIG}\n[web]\n# who can call us\ncors_origins = []   # none\nlog_buffer = 100\n"
    );
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/fields",
            t,
            Some(json!({"content": text, "set": {
                "web.log_buffer": 500,
                "web.cors_origins": ["http://localhost:5173"],
                "server.motd": "Hi there",
                "accounts.open_registration": true,
                "server.restart_mode": "exit",
                "metrics.interval_secs": null,
            }})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let out = body["content"].as_str().unwrap();
    assert!(
        out.starts_with("# my server\n") && out.contains("# who can call us"),
        "{out}"
    );
    let cfg = poe_config::Config::parse(out).unwrap();
    assert_eq!(cfg.web.log_buffer, 500);
    assert_eq!(cfg.web.cors_origins, vec!["http://localhost:5173"]);
    assert_eq!(cfg.server.motd, "Hi there");
    assert!(cfg.accounts.open_registration);
    assert_eq!(cfg.server.restart_mode, poe_config::RestartMode::Exit);
    assert!(
        out.contains("cors_origins = [\"http://localhost:5173\"]   # none"),
        "{out}"
    );

    // Refused: unknown keys, wrong types, out-of-range values, bad choices, text that does not parse.
    for (set, what) in [
        (json!({"nonsense.key": 1}), "unknown key"),
        (json!({"web.log_buffer": "many"}), "wrong type"),
        (json!({"web.log_buffer": 1}), "below the minimum"),
        (json!({"web.log_buffer": 99_999_999}), "above the maximum"),
        (json!({"server.restart_mode": "reboot"}), "not a choice"),
        (json!({"server.motd": "x".repeat(281)}), "too long"),
        (json!({"web.cors_origins": [1]}), "list of numbers"),
    ] {
        let (status, body) = f
            .call(
                "POST",
                "/api/v1/admin/config/fields",
                t,
                Some(json!({"content": text, "set": set})),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
    }
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/config/fields",
            t,
            Some(json!({"content": "a = [", "set": {"web.log_buffer": 100}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // With nothing to set it reads the form values out of the text (raw editor to form), and
    // reports the place of a syntax error instead of failing.
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/fields",
            t,
            Some(json!({"content": text, "set": {}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            body["content"].as_str(),
            body["values"]["web.log_buffer"].as_i64(),
            body["values"]["web.cors_origins"].clone(),
            body["parse_error"].is_null()
        ),
        (Some(text.as_str()), Some(100), json!([]), true)
    );
    let (_, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/fields",
            t,
            Some(json!({"content": "[web]\nlog_buffer = = 1\n", "set": {}})),
        )
        .await;
    assert_eq!(
        (
            body["parse_error"]["line"].as_u64(),
            body["values"].as_object().map(|o| o.len())
        ),
        (Some(2), Some(0)),
        "{body}"
    );
}

#[tokio::test]
async fn saving_validates_backs_up_and_replaces_the_file() {
    let (f, path, backups) = with_file("\n[server]\nmotd = \"old\"\n");
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let (_, view) = f.call("GET", "/api/v1/admin/config", t, None).await;
    let revision = view["revision"].as_str().unwrap().to_owned();
    let original = std::fs::read_to_string(&path).unwrap();

    // An invalid text is refused, with the place, and nothing changes.
    let broken = original.replace("motd = \"old\"", "motd = = \"new\"");
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/save",
            t,
            Some(json!({"content": broken, "base_revision": revision})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body["error"].as_str().unwrap();
    assert!(
        message.contains("not valid")
            && message.contains("nothing was saved")
            && message.contains("line "),
        "{message}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert!(!backups.exists(), "no backup for a refused save");

    // A valid text is saved: the file, a backup of the previous one, the live motd, the audit.
    let edited = original.replace("motd = \"old\"", "motd = \"new\"");
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/save",
            t,
            Some(json!({"content": edited, "base_revision": revision})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            body["saved"].as_bool(),
            body["changed"].as_bool(),
            body["lines_changed"].as_u64()
        ),
        (Some(true), Some(true), Some(2))
    );
    assert_ne!(body["revision"].as_str().unwrap(), revision);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    let backup = body["backup"].as_str().unwrap();
    assert!(backup.starts_with("server.toml.20"), "{backup}");
    assert_eq!(
        std::fs::read_to_string(backups.join(backup)).unwrap(),
        original
    );
    assert_eq!(f.state.motd.get(), "new");
    assert!(
        f.calls()
            .iter()
            .any(|c| c
                .starts_with("root: record config save [2 lines changed, backup server.toml.20")),
        "{:?}",
        f.calls()
    );
    assert!(
        !f.calls()
            .iter()
            .any(|c| c.contains("new") && c.contains("motd")),
        "contents are not audited"
    );
    assert!(body["restart"].is_null());

    // Saving the same text again changes nothing and makes no backup.
    let (_, view2) = f.call("GET", "/api/v1/admin/config", t, None).await;
    let revision2 = view2["revision"].as_str().unwrap().to_owned();
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/save",
            t,
            Some(json!({"content": edited, "base_revision": revision2})),
        )
        .await;
    assert_eq!(
        (status, body["changed"].as_bool(), body["backup"].is_null()),
        (StatusCode::OK, Some(false), true),
        "{body}"
    );
    assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 1);

    // A stale revision (the file changed meanwhile) is refused.
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/save",
            t,
            Some(json!({"content": original, "base_revision": revision})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("changed since"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
}

#[tokio::test]
async fn only_the_newest_backups_are_kept() {
    let (f, path, backups) = with_file("");
    let t = admin(&f).await;
    let t = Some(t.as_str());
    // config_backup_keep = 3 in this fixture; seven saves.
    for i in 0..7 {
        let (_, view) = f.call("GET", "/api/v1/admin/config", t, None).await;
        let text = view["content"].as_str().unwrap().to_owned();
        let text = format!("{text}\n# edit {i}\n");
        let (status, body) = f
            .call(
                "POST",
                "/api/v1/admin/config/save",
                t,
                Some(json!({"content": text, "base_revision": view["revision"]})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 3);
    // The list is newest first and the content of one can be fetched; names that are not in the
    // list (paths, other files) are not served.
    let (status, list) = f.call("GET", "/api/v1/admin/config/backups", t, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 3);
    let mut sorted = names.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(names, sorted);
    assert!(
        list[0]["size"].as_u64().unwrap() > 100
            && list[0]["modified"].as_i64().unwrap() > 1_700_000_000
    );
    let (status, body) = f
        .call(
            "GET",
            &format!("/api/v1/admin/config/backups/{}", names[0]),
            t,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["content"].as_str().unwrap().contains("# edit 5"),
        "the second last version: {names:?} {}",
        body["content"]
    );
    std::fs::write(backups.join("notes.txt"), "x").unwrap();
    std::fs::write(path.parent().unwrap().join("secret.txt"), "top secret").unwrap();
    for name in [
        "notes.txt",
        "..%2Fsecret.txt",
        "server.toml.nope",
        "nothing",
    ] {
        let (status, _) = f
            .call(
                "GET",
                &format!("/api/v1/admin/config/backups/{name}"),
                t,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{name}");
    }
}

#[tokio::test]
async fn save_and_restart_requests_the_restart_after_saving() {
    let (f, path, _) = with_file("");
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let (_, view) = f.call("GET", "/api/v1/admin/config", t, None).await;
    let edited = format!("{}\n# restarting\n", view["content"].as_str().unwrap());
    // Bad restart options are refused before the file is touched.
    let (status, _) = f
        .call("POST", "/api/v1/admin/config/save", t, Some(json!({"content": edited, "base_revision": view["revision"], "restart": {"delay_secs": 99999}})))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        view["content"].as_str().unwrap()
    );

    let (status, body) = f
        .call("POST", "/api/v1/admin/config/save", t, Some(json!({"content": edited, "base_revision": view["revision"], "restart": {"delay_secs": 20, "message": "config change"}})))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            body["restart"]["action"].as_str(),
            body["restart"]["delay_secs"].as_u64(),
            body["restart"]["message"].as_str()
        ),
        (Some("restart"), Some(20), Some("config change"))
    );
    assert!(body["restart_error"].is_null());
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("# restarting")
    );
    let (_, pending) = f
        .call("GET", "/api/v1/admin/server/shutdown", t, None)
        .await;
    assert_eq!(pending["pending"]["action"], "restart");
    assert!(
        f.calls()
            .iter()
            .any(|c| c.contains("record config save") && c.contains("restart requested"))
    );

    // Another restart is pending: the second save still saves, and says why there is no restart.
    let (_, view) = f.call("GET", "/api/v1/admin/config", t, None).await;
    let again = format!("{}# more\n", view["content"].as_str().unwrap());
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/save",
            t,
            Some(json!({"content": again, "base_revision": view["revision"], "restart": {}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["restart"].is_null()
            && body["restart_error"]
                .as_str()
                .unwrap()
                .contains("already pending"),
        "{body}"
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .ends_with("# more\n")
    );
}

#[tokio::test]
async fn the_editor_can_be_switched_off() {
    let (_, path, backups) = with_file("");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("[web]\n", "[web]\nconfig_editor = false\n");
    std::fs::write(&path, &text).unwrap();
    let f2 = fixture_at(&text, Some(path.clone()));
    let t = admin(&f2).await;
    let t = Some(t.as_str());
    let (status, view) = f2.call("GET", "/api/v1/admin/config", t, None).await;
    assert_eq!(
        (status, view["editable"].as_bool()),
        (StatusCode::OK, Some(false)),
        "reading still works"
    );
    let (status, body) = f2
        .call(
            "POST",
            "/api/v1/admin/config/save",
            t,
            Some(json!({"content": format!("{text}# x\n"), "base_revision": view["revision"]})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert!(!backups.exists());
}

#[tokio::test]
async fn token_like_values_are_hidden_in_the_editor_and_survive_a_save() {
    // The server config has no secrets; if someone adds one it must not be shown or lost.
    let (f, path, _) = with_file("\n[hooks]\nadmin_token = \"s3cr3t-value\"\n");
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let (_, view) = f.call("GET", "/api/v1/admin/config", t, None).await;
    let shown = view["content"].as_str().unwrap();
    assert!(
        !shown.contains("s3cr3t-value") && shown.contains("<hidden>"),
        "{shown}"
    );
    assert!(
        !view.to_string().contains("s3cr3t-value"),
        "nowhere in the response"
    );
    // Saved with the placeholder: the stored value stays.
    let edited = format!("{shown}\n# touched\n");
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/config/save",
            t,
            Some(json!({"content": edited, "base_revision": view["revision"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        on_disk.contains("admin_token = \"s3cr3t-value\"") && on_disk.contains("# touched"),
        "{on_disk}"
    );
    assert!(!body.to_string().contains("s3cr3t-value"));
    // Nothing the server logs for admins carries it either.
    assert!(!f.calls().iter().any(|c| c.contains("s3cr3t-value")));
    // A placeholder on a new key has nothing to restore: refused.
    let (_, view) = f.call("GET", "/api/v1/admin/config", t, None).await;
    let bogus = format!(
        "{}\n[more]\npassword = \"<hidden>\"\n",
        view["content"].as_str().unwrap()
    );
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/config/save",
            t,
            Some(json!({"content": bogus, "base_revision": view["revision"]})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_editor_writes_the_file_the_server_was_started_with() {
    // Config::load records the absolute path of the file it read; the editor uses exactly that.
    let dir = scratch("load");
    let path = dir.join("custom.toml");
    std::fs::write(&path, include_str!("../../../config/server.toml")).unwrap();
    let root = repo_root();
    let loaded = poe_config::Config::load_in(&path, &root).unwrap();
    assert_eq!(
        loaded.source_path.as_deref(),
        Some(std::fs::canonicalize(&path).unwrap().as_path())
    );
    let text = std::fs::read_to_string(&path).unwrap();
    let f = fixture_at(&text, loaded.source_path.clone());
    let t = admin(&f).await;
    let (_, view) = f.call("GET", "/api/v1/admin/config", Some(&t), None).await;
    assert_eq!(
        view["path"].as_str().map(std::path::Path::new),
        loaded.source_path.as_deref()
    );
    // The committed config in a checkout carries a notice; a copy elsewhere does not.
    assert!(
        !view["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("committed config"))
    );
    let committed = repo_root().join("config/server.toml");
    let g = fixture_at(&text, Some(std::fs::canonicalize(committed).unwrap()));
    let t = admin(&g).await;
    let (_, view) = g.call("GET", "/api/v1/admin/config", Some(&t), None).await;
    assert!(
        view["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("committed config")),
        "{view}"
    );
    // The committed file itself still has the safe defaults the editor must not erode.
    let safe = poe_config::Config::parse(include_str!("../../../config/server.toml")).unwrap();
    assert!(
        safe.web.bind.ip().is_loopback()
            && !safe.accounts.open_registration
            && safe.server.motd.is_empty()
    );
    assert!(
        poe_config::Config::check_text(include_str!("../../../config/server.toml"), &root)
            .warnings
            .is_empty()
    );
}

// ----- scheduled tasks -----

fn task(name: &str, schedule: Value) -> Value {
    json!({"name": name, "task": {"kind": "restart", "delay_secs": 60, "message": "nightly"}, "schedule": schedule})
}

#[tokio::test]
async fn tasks_are_created_listed_edited_toggled_and_deleted() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let (status, overview) = f.call("GET", "/api/v1/admin/schedule", t, None).await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(
        (
            overview["timezone"].as_str(),
            overview["utc_offset"].as_str(),
            overview["enabled"].as_bool()
        ),
        (Some("UTC"), Some("+00:00"), Some(true))
    );
    assert_eq!(overview["now"], 1_792_000_000);
    assert!(overview["tasks"].as_array().unwrap().is_empty());

    // 1_792_000_000 is 2026-10-14 17:46:40 UTC: the next 04:00 is the 15th.
    let (status, made) = f
        .call(
            "POST",
            "/api/v1/admin/schedule/tasks",
            t,
            Some(task("Nightly", json!({"type": "daily", "time": "4:00"}))),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["id"].as_u64().unwrap();
    assert_eq!(
        (
            made["name"].as_str(),
            made["enabled"].as_bool(),
            made["created_by"].as_str()
        ),
        (Some("Nightly"), Some(true), Some("root"))
    );
    assert_eq!(made["schedule"], json!({"type": "daily", "time": "04:00"}));
    assert_eq!(
        made["task"],
        json!({"kind": "restart", "delay_secs": 60, "message": "nightly"})
    );
    assert_eq!(made["next_run_at"], 1_792_036_800);
    assert_eq!(
        (made["schedule_text"].as_str(), made["task_text"].as_str()),
        (
            Some("Every day at 04:00"),
            Some("Restart the server after a 1 min countdown (\"nightly\")")
        )
    );
    assert!(made["last_run_at"].is_null() && made["problem"].is_null());
    assert!(f.calls().iter().any(|c| c.starts_with(&format!(
        "root: record schedule create [task #{id} \"Nightly\"]"
    ))));

    // Edit (all fields), then off and on.
    let (status, edited) = f
        .call(
            "POST",
            &format!("/api/v1/admin/schedule/tasks/{id}"),
            t,
            Some(task(
                "Weekend",
                json!({"type": "weekly", "days": ["sun", "sat"], "time": "05:30"}),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(
        (
            edited["name"].as_str(),
            edited["schedule"]["days"].clone(),
            edited["schedule_text"].as_str()
        ),
        (
            Some("Weekend"),
            json!(["sat", "sun"]),
            Some("Every Sat, Sun at 05:30")
        )
    );
    let (_, off) = f
        .call(
            "POST",
            &format!("/api/v1/admin/schedule/tasks/{id}/disable"),
            t,
            None,
        )
        .await;
    assert_eq!(
        (off["enabled"].as_bool(), off["next_run_at"].is_null()),
        (Some(false), true)
    );
    let (_, on) = f
        .call(
            "POST",
            &format!("/api/v1/admin/schedule/tasks/{id}/enable"),
            t,
            None,
        )
        .await;
    assert_eq!(
        (on["enabled"].as_bool(), on["next_run_at"].is_number()),
        (Some(true), true)
    );
    // An interval and a cron task, listed in id order.
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/schedule/tasks",
            t,
            Some(task(
                "Hourly",
                json!({"type": "interval", "every_secs": 3600}),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, cron) = f
        .call(
            "POST",
            "/api/v1/admin/schedule/tasks",
            t,
            Some(task(
                "Cron",
                json!({"type": "cron", "expr": " 0 4  * * mon-fri "}),
            )),
        )
        .await;
    assert_eq!(
        (status, cron["schedule"]["expr"].as_str()),
        (StatusCode::CREATED, Some("0 4 * * mon-fri"))
    );
    let (_, overview) = f.call("GET", "/api/v1/admin/schedule", t, None).await;
    let names: Vec<&str> = overview["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Weekend", "Hourly", "Cron"]);

    // Delete.
    let (status, _) = f
        .call(
            "POST",
            &format!("/api/v1/admin/schedule/tasks/{id}/delete"),
            t,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = f
        .call(
            "POST",
            &format!("/api/v1/admin/schedule/tasks/{id}/delete"),
            t,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for (method, uri) in [
        ("POST", format!("/api/v1/admin/schedule/tasks/{id}/enable")),
        ("POST", format!("/api/v1/admin/schedule/tasks/{id}/disable")),
        ("GET", format!("/api/v1/admin/schedule/tasks/{id}/runs")),
    ] {
        let (status, _) = f.call(method, &uri, t, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
    let (status, _) = f
        .call(
            "POST",
            &format!("/api/v1/admin/schedule/tasks/{id}"),
            t,
            Some(task("x", json!({"type": "daily", "time": "04:00"}))),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for action in [
        "schedule update",
        "schedule disable",
        "schedule enable",
        "schedule delete",
    ] {
        assert!(
            f.calls()
                .iter()
                .any(|c| c.contains(&format!("record {action} "))),
            "{action} audited"
        );
    }
}

#[tokio::test]
async fn task_input_is_validated() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let daily = json!({"type": "daily", "time": "04:00"});
    for (body, what) in [
        (
            json!({"name": "", "task": {"kind": "restart"}, "schedule": daily}),
            "empty name",
        ),
        (
            task("x", json!({"type": "daily", "time": "25:00"})),
            "bad time",
        ),
        (
            task("x", json!({"type": "weekly", "days": [], "time": "04:00"})),
            "no days",
        ),
        (
            task(
                "x",
                json!({"type": "weekly", "days": ["someday"], "time": "04:00"}),
            ),
            "bad day",
        ),
        (
            task("x", json!({"type": "interval", "every_secs": 10})),
            "interval too short",
        ),
        (
            task("x", json!({"type": "cron", "expr": "* * *"})),
            "cron fields",
        ),
        (
            task("x", json!({"type": "cron", "expr": "61 * * * *"})),
            "cron range",
        ),
        (
            json!({"name": "x", "task": {"kind": "restart", "delay_secs": 9999}, "schedule": daily}),
            "delay too long",
        ),
        (
            json!({"name": "x", "task": {"kind": "restart", "message": "m".repeat(201)}, "schedule": daily}),
            "message too long",
        ),
    ] {
        let (status, body) = f
            .call("POST", "/api/v1/admin/schedule/tasks", t, Some(body))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
        assert!(!body["error"].as_str().unwrap().is_empty());
    }
    // Shape errors come from the JSON layer.
    for body in [
        json!({"name": "x", "task": {"kind": "reboot"}, "schedule": daily}),
        json!({"name": "x", "task": {"kind": "restart"}, "schedule": {"type": "hourly"}}),
        json!({"name": "x"}),
    ] {
        let (status, _) = f
            .call("POST", "/api/v1/admin/schedule/tasks", t, Some(body))
            .await;
        assert!(
            status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
            "{status}"
        );
    }
    let (_, overview) = f.call("GET", "/api/v1/admin/schedule", t, None).await;
    assert!(
        overview["tasks"].as_array().unwrap().is_empty(),
        "nothing was stored"
    );
    assert!(!f.calls().iter().any(|c| c.contains("schedule create")));
}

#[tokio::test(start_paused = true)]
async fn the_history_of_a_task_lists_its_runs() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let (_, made) = f
        .call(
            "POST",
            "/api/v1/admin/schedule/tasks",
            t,
            Some(task("Nightly", json!({"type": "daily", "time": "04:00"}))),
        )
        .await;
    let id = made["id"].as_u64().unwrap();
    let (status, runs) = f
        .call(
            "GET",
            &format!("/api/v1/admin/schedule/tasks/{id}/runs"),
            t,
            None,
        )
        .await;
    assert_eq!(
        (status, runs.as_array().unwrap().len()),
        (StatusCode::OK, 0)
    );

    // The scheduler runs it when due (driven by hand here; its loop has its own tests).
    let due = made["next_run_at"].as_i64().unwrap();
    f.clock.0.store(due, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(f.state.scheduler.run_due(due), 1);
    let (_, runs) = f
        .call(
            "GET",
            &format!("/api/v1/admin/schedule/tasks/{id}/runs"),
            t,
            None,
        )
        .await;
    assert_eq!(runs.as_array().unwrap().len(), 1);
    assert_eq!(
        (runs[0]["status"].as_str(), runs[0]["due_at"].as_i64()),
        (Some("ok"), Some(due))
    );
    assert!(runs[0]["message"].as_str().unwrap().contains("60 s"));
    let (_, overview) = f.call("GET", "/api/v1/admin/schedule", t, None).await;
    let row = &overview["tasks"][0];
    assert_eq!(
        (
            row["last_status"].as_str(),
            row["last_run_at"].as_i64(),
            row["next_run_at"].as_i64()
        ),
        (Some("ok"), Some(due), Some(due + 86_400))
    );
    // The restart is under way.
    let (_, pending) = f
        .call("GET", "/api/v1/admin/server/shutdown", t, None)
        .await;
    assert_eq!(
        (
            pending["pending"]["action"].as_str(),
            pending["pending"]["requested_by"].as_str()
        ),
        (Some("restart"), Some("root"))
    );
}

#[tokio::test]
async fn the_preview_lists_the_next_runs_or_names_the_problem() {
    let f = fixture();
    let t = admin(&f).await;
    let t = Some(t.as_str());
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/schedule/preview",
            t,
            Some(json!({"schedule": {"type": "cron", "expr": "0 */6 * * *"}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["description"].as_str(), body["timezone"].as_str()),
        (Some("Cron 0 */6 * * *"), Some("UTC"))
    );
    // 1_792_000_000 is 17:46:40 UTC: 18:00, then every six hours.
    assert_eq!(
        body["next_runs"],
        json!([
            1_792_000_800i64,
            1_792_022_400,
            1_792_044_000,
            1_792_065_600,
            1_792_087_200
        ])
    );
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/schedule/preview",
            t,
            Some(json!({"schedule": {"type": "cron", "expr": "nope"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("5 fields"));
    let (_, body) = f
        .call(
            "POST",
            "/api/v1/admin/schedule/preview",
            t,
            Some(json!({"schedule": {"type": "cron", "expr": "0 0 31 2 *"}})),
        )
        .await;
    assert!(
        body["next_runs"].as_array().unwrap().is_empty(),
        "a date that never comes"
    );
}
