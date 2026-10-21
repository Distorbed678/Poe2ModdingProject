//! Router tests with tower's `oneshot`: no sockets, an in-memory database and a mock
//! `AdminControl` that records the commands it is given.

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use poe_accounts::{AccountStore, Role, SqliteAccounts, TokenKind, password_digest};
use poe_cluster::{
    AdminControl, AdminOutcome, InstanceDirectory, InstanceSpec, MemoryInstances, NewInstance,
    OnlinePlayer, TransferTarget,
};
use poe_config::Config;
use poe_store::{NewCharacter, SqliteStore};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::{LogBuffer, LogLevel, WebState};

const CONFIG: &str = r#"
    [game]
    version = "0.5.5e"
    game_dir = "g"
    schema = "s"
    protocol_dir = "p"
    [network]
    public_host = "play.example"
    [network.login]
    bind = "0.0.0.0:20481"
    [network.instance]
    bind = "0.0.0.0:6112"
    public_port = 6112
    [database]
    path = "d"
    [accounts]
    name_prefix = "Exile"
    max_characters = 4
    remember_days = 7
    [gateway]
    region = "Local"
    display_name = "Test Realm"
    [cluster]
    id = "test-cluster"
"#;

/// Records commands; `account` commands really run against the account store.
struct MockAdmin {
    accounts: Arc<dyn AccountStore>,
    calls: Mutex<Vec<String>>,
    online: Vec<OnlinePlayer>,
}

impl AdminControl for MockAdmin {
    fn online(&self) -> Vec<OnlinePlayer> {
        self.online.clone()
    }

    fn run_command(&self, actor: &str, line: &str) -> AdminOutcome {
        self.calls.lock().unwrap().push(format!("{actor}: {line}"));
        let mut words = line.split_whitespace();
        match words.next() {
            Some("account") => {
                let sub = words.next().unwrap();
                let name = words.next();
                let value = words.next();
                match poe_accounts::run_account_command(
                    &*self.accounts,
                    Some(actor),
                    sub,
                    name,
                    value,
                ) {
                    Ok(o) => AdminOutcome::ok(o.text),
                    Err(e) => AdminOutcome::err(e),
                }
            }
            Some("kick") if line.ends_with("Ghost") => AdminOutcome::err("Ghost is not online"),
            Some("close") if line.ends_with("999") => AdminOutcome::err("no instance #999"),
            _ => AdminOutcome::ok("done"),
        }
    }

    fn create_instance(&self, actor: &str, new: &NewInstance) -> Result<u32, AdminOutcome> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{actor}: create {new:?}"));
        if new.area == "Nowhere" {
            Err(AdminOutcome::err("no area Nowhere in WorldAreas"))
        } else {
            Ok(4242)
        }
    }

    fn transfer_player(
        &self,
        actor: &str,
        character: &str,
        target: TransferTarget,
    ) -> AdminOutcome {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{actor}: transfer {character} {target:?}"));
        AdminOutcome::ok("moving")
    }
}

struct Fixture {
    app: Router,
    state: WebState,
    mock: Arc<MockAdmin>,
}

fn fixture_with(config: &str) -> Fixture {
    let config = Arc::new(Config::parse(config).unwrap());
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let accounts: Arc<dyn AccountStore> = Arc::new(
        SqliteAccounts::new(store.clone(), config.cluster.id.clone())
            .with_params(poe_accounts::Params::new(8, 1, 1, None).unwrap()),
    );
    let mock = Arc::new(MockAdmin {
        accounts: accounts.clone(),
        calls: Mutex::new(Vec::new()),
        online: vec![OnlinePlayer {
            character: "Bob".into(),
            account: "bobby".into(),
            instance: Some(1),
            area: "G1_1".into(),
        }],
    });
    let instances: Arc<dyn InstanceDirectory> = Arc::new(MemoryInstances::default());
    instances.create(
        InstanceSpec {
            area: "G1_1".into(),
            seed: Some(5),
            ..Default::default()
        },
        true,
    );
    let limiter = Arc::new(poe_accounts::LoginLimiter::from_config(&config.security));
    let state = WebState {
        config,
        accounts,
        limiter,
        store,
        instances,
        admin: mock.clone(),
        logs: LogBuffer::new(50),
        web_tokens: Default::default(),
        ws_tickets: Default::default(),
        started: Instant::now(),
    };
    Fixture {
        app: crate::router(state.clone()),
        state,
        mock,
    }
}

fn fixture() -> Fixture {
    fixture_with(CONFIG)
}

fn open_registration() -> String {
    CONFIG.replace(
        "remember_days = 7",
        "remember_days = 7\n    open_registration = true",
    )
}

impl Fixture {
    async fn call(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let body = match body {
            Some(b) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let res = self
            .app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, value)
    }

    fn account(&self, name: &str, password: &str, role: Role) {
        self.state
            .accounts
            .create_account(name, &password_digest(password), role)
            .unwrap();
    }

    async fn login(&self, name: &str, password: &str) -> String {
        let (status, body) = self
            .call(
                "POST",
                "/api/v1/login",
                None,
                Some(json!({"username": name, "password": password})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["token"].as_str().unwrap().to_owned()
    }

    fn calls(&self) -> Vec<String> {
        self.mock.calls.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn server_info_is_public() {
    let f = fixture();
    let (status, body) = f.call("GET", "/api/v1/server", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Test Realm");
    assert_eq!(body["version"], "0.5.5e");
    assert_eq!(body["cluster_id"], "test-cluster");
    assert_eq!(body["online"], 1);
    assert_eq!(body["open_registration"], false);
    assert_eq!(
        (body["login_host"].as_str(), body["login_port"].as_u64()),
        (Some("play.example"), Some(20481))
    );
    assert!(body["instance_port"].as_u64().is_some_and(|p| p > 0));
    assert!(body["gateway_region"].is_string());
}

#[tokio::test]
async fn server_info_reports_the_public_login_port() {
    let f = fixture_with(&CONFIG.replace(
        "bind = \"0.0.0.0:20481\"",
        "bind = \"0.0.0.0:20481\"\n    public_port = 30481",
    ));
    let (_, body) = f.call("GET", "/api/v1/server", None, None).await;
    assert_eq!(body["login_port"], 30481);
}

#[tokio::test]
async fn registration_follows_the_config_switch() {
    let closed = fixture();
    let (status, body) = closed
        .call(
            "POST",
            "/api/v1/register",
            None,
            Some(json!({"username": "alice", "password": "correct horse"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(closed.state.accounts.find("alice").unwrap().is_none());

    let open = fixture_with(&open_registration());
    let (status, body) = open
        .call(
            "POST",
            "/api/v1/register",
            None,
            Some(json!({"username": "alice", "password": "correct horse"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        (body["username"].as_str(), body["role"].as_str()),
        (Some("alice"), Some("player"))
    );
    assert!(body.get("password").is_none() && body.get("password_hash").is_none());
    // The game login checks the client's digest: the same password works there.
    open.state
        .accounts
        .authenticate("alice", &password_digest("correct horse"))
        .unwrap();
    // Taken (case-insensitively), weak, invalid name, bad JSON.
    let (status, _) = open
        .call(
            "POST",
            "/api/v1/register",
            None,
            Some(json!({"username": "ALICE", "password": "another pass"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = open
        .call(
            "POST",
            "/api/v1/register",
            None,
            Some(json!({"username": "bob", "password": "short"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = open
        .call(
            "POST",
            "/api/v1/register",
            None,
            Some(json!({"username": "b b", "password": "long enough pw"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = open
        .call(
            "POST",
            "/api/v1/register",
            None,
            Some(json!({"username": "bob"})),
        )
        .await;
    assert!(status.is_client_error());
    assert!(body["error"].is_string(), "errors are JSON: {body}");
}

#[tokio::test]
async fn login_good_and_bad() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Player);
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "correct horse"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["kind"], "web");
    assert_eq!(body["account"]["username"], "alice");
    assert_eq!(body["token"].as_str().unwrap().len(), 64);
    // Names are case-insensitive, like in game.
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "ALICE", "password": "correct horse"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // Wrong password and unknown account look the same.
    let (s1, b1) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "wrong"})),
        )
        .await;
    let (s2, b2) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "nobody", "password": "wrong"})),
        )
        .await;
    assert_eq!(
        (s1, s2),
        (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED)
    );
    assert_eq!(b1, b2);
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "correct horse", "kind": "root"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Disabled accounts cannot log in.
    f.state.accounts.set_disabled("alice", true).unwrap();
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "correct horse"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn launcher_login_issues_a_launcher_token_the_game_accepts() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Admin);
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "correct horse", "kind": "launcher"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["kind"], "launcher");
    let token = body["token"].as_str().unwrap().to_owned();
    // 7 days (`remember_days`).
    let ttl = body["expires_at"].as_i64().unwrap()
        - std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
    assert!((ttl - 7 * 86_400).abs() < 60, "{ttl}");
    let raw: [u8; 32] = hex::decode(&token).unwrap().try_into().unwrap();
    f.state
        .accounts
        .authenticate_token(&raw, &[TokenKind::Launcher])
        .unwrap();
    // Good for account endpoints, but never for the admin API, even for an admin.
    let (status, _) = f.call("POST", "/api/v1/logout", Some(&token), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let again = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "correct horse", "kind": "launcher"})),
        )
        .await
        .1;
    let (status, _) = f
        .call(
            "GET",
            "/api/v1/admin/accounts",
            again["token"].as_str(),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn game_token_exchange_gives_the_client_a_token_the_launcher_keeps() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Player);
    let launcher = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": "alice", "password": "correct horse", "kind": "launcher"})),
        )
        .await
        .1["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, body) = f
        .call("POST", "/api/v1/game-token", Some(&launcher), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "game_remember");
    assert_eq!(body["account"]["username"], "alice");
    let game: [u8; 32] = hex::decode(body["token"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    // The game's token login accepts it and retires it (rolling); the launcher token stays.
    let account = f
        .state
        .accounts
        .authenticate_token(&game, &[TokenKind::GameRemember])
        .unwrap();
    assert_eq!(account.username, "alice");
    f.state.accounts.revoke_token(&game).unwrap();
    let (status, _) = f
        .call("POST", "/api/v1/game-token", Some(&launcher), None)
        .await;
    assert_eq!(status, StatusCode::OK, "the launcher token survives game logins");
    // Not for web tokens, nor without a token, nor for the game token itself.
    let web = f.login("alice", "correct horse").await;
    let (status, _) = f.call("POST", "/api/v1/game-token", Some(&web), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f.call("POST", "/api/v1/game-token", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/game-token",
            Some(body["token"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn tokens_are_required() {
    let f = fixture();
    for (method, uri) in [
        ("POST", "/api/v1/logout"),
        ("POST", "/api/v1/password"),
        ("GET", "/api/v1/admin/sessions"),
        ("GET", "/api/v1/admin/instances"),
        ("GET", "/api/v1/admin/accounts"),
        ("GET", "/api/v1/admin/characters"),
        ("GET", "/api/v1/admin/logs/ws"),
        ("POST", "/api/v1/admin/logs/ws-ticket"),
        ("POST", "/api/v1/admin/announce"),
    ] {
        let (status, body) = f.call(method, uri, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}: {body}");
    }
    // Malformed and unknown tokens.
    let (status, _) = f
        .call("GET", "/api/v1/admin/sessions", Some("not hex"), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = f
        .call(
            "GET",
            "/api/v1/admin/sessions",
            Some(&"ab".repeat(32)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(f.calls().is_empty());
}

#[tokio::test]
async fn players_are_forbidden_from_admin_endpoints() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Player);
    let token = f.login("alice", "correct horse").await;
    for (method, uri, body) in [
        ("GET", "/api/v1/admin/sessions", None),
        ("GET", "/api/v1/admin/accounts", None),
        ("GET", "/api/v1/admin/characters/1", None),
        ("POST", "/api/v1/admin/players/Bob/kick", None),
        (
            "POST",
            "/api/v1/admin/announce",
            Some(json!({"message": "hi"})),
        ),
        (
            "POST",
            "/api/v1/admin/accounts/alice/role",
            Some(json!({"role": "admin"})),
        ),
    ] {
        let (status, _) = f.call(method, uri, Some(&token), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    assert!(f.calls().is_empty(), "nothing reached the command registry");
    // They can still use the account endpoints.
    let (status, _) = f.call("POST", "/api/v1/logout", Some(&token), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn a_revoked_token_gives_401() {
    let f = fixture();
    f.account("root", "correct horse", Role::Admin);
    let token = f.login("root", "correct horse").await;
    let (status, body) = f
        .call("GET", "/api/v1/admin/sessions", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body[0]["character"], "Bob");
    let (status, _) = f.call("POST", "/api/v1/logout", Some(&token), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = f
        .call("GET", "/api/v1/admin/sessions", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "token revoked");
    // Disabling the account revokes its tokens too.
    f.account("other", "correct horse", Role::Admin);
    let t2 = f.login("other", "correct horse").await;
    f.state.accounts.set_disabled("other", true).unwrap();
    let (status, _) = f
        .call("GET", "/api/v1/admin/sessions", Some(&t2), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_token_from_another_cluster_is_refused() {
    let f = fixture();
    f.account("root", "correct horse", Role::Admin);
    let token = f.login("root", "correct horse").await;
    // The same database under another cluster id.
    let other = {
        let mut state = f.state.clone();
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        state.accounts = Arc::new(SqliteAccounts::new(store, "elsewhere"));
        Fixture {
            app: crate::router(state.clone()),
            state,
            mock: f.mock.clone(),
        }
    };
    let (status, _) = other
        .call("GET", "/api/v1/admin/sessions", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn password_change() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Player);
    let token = f.login("alice", "correct horse").await;
    let other = f.login("alice", "correct horse").await;
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/password",
            Some(&token),
            Some(json!({"old_password": "nope nope", "new_password": "battery staple"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/password",
            Some(&token),
            Some(json!({"old_password": "correct horse", "new_password": "short"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/password",
            Some(&token),
            Some(json!({"old_password": "correct horse", "new_password": "battery staple"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The game's check uses the new password; every old token is dead, the fresh one works.
    f.state
        .accounts
        .authenticate("alice", &password_digest("battery staple"))
        .unwrap();
    assert!(
        f.state
            .accounts
            .authenticate("alice", &password_digest("correct horse"))
            .is_err()
    );
    assert_eq!(
        f.call("POST", "/api/v1/logout", Some(&other), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.call("POST", "/api/v1/logout", Some(&token), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.call("POST", "/api/v1/logout", body["token"].as_str(), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn admin_account_endpoints() {
    let f = fixture();
    f.account("root", "correct horse", Role::Admin);
    f.account("alice", "correct horse", Role::Player);
    let token = f.login("root", "correct horse").await;

    let (status, body) = f
        .call("GET", "/api/v1/admin/accounts", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["username"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["root", "alice"]);
    assert_eq!(body[0]["role"], "admin");
    assert!(body[0].get("password_hash").is_none());

    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/accounts/alice/role",
            Some(&token),
            Some(json!({"role": "admin"})),
        )
        .await;
    assert_eq!(
        (status, body["role"].as_str()),
        (StatusCode::OK, Some("admin"))
    );
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/accounts/alice/disable",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(
        (status, body["disabled"].as_bool()),
        (StatusCode::OK, Some(true))
    );
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/accounts/alice/enable",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(
        (status, body["disabled"].as_bool()),
        (StatusCode::OK, Some(false))
    );
    // The command ran as the caller, through the registry.
    assert_eq!(
        f.calls(),
        [
            "root: account role alice admin",
            "root: account disable alice",
            "root: account enable alice"
        ]
    );

    // Not on yourself; unknown names; bad roles.
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/accounts/root/disable",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/accounts/root/role",
            Some(&token),
            Some(json!({"role": "player"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/accounts/ghost/disable",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/admin/accounts/alice/role",
            Some(&token),
            Some(json!({"role": "root"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(f.state.accounts.find("root").unwrap().unwrap().role == Role::Admin);
}

#[tokio::test]
async fn admin_actions_run_through_the_command_registry() {
    let f = fixture();
    f.account("root", "correct horse", Role::Admin);
    let t = f.login("root", "correct horse").await;
    let t = Some(t.as_str());

    assert_eq!(
        f.call("POST", "/api/v1/admin/players/Bob/kick", t, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("POST", "/api/v1/admin/players/Ghost/kick", t, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.call("POST", "/api/v1/admin/players/Bob/mute", t, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/admin/players/Bob/mute",
            t,
            Some(json!({"minutes": 15}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("POST", "/api/v1/admin/players/Bob/unmute", t, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("POST", "/api/v1/admin/players/B%20ob/kick", t, None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/admin/announce",
            t,
            Some(json!({"message": "  restart\nin 5   minutes "}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/admin/announce",
            t,
            Some(json!({"message": "   "}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.call("POST", "/api/v1/admin/instances/17/close", t, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("POST", "/api/v1/admin/instances/999/close", t, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.call("POST", "/api/v1/admin/instances/x/close", t, None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    let (status, body) = f
        .call(
            "POST",
            "/api/v1/admin/instances",
            t,
            Some(json!({"area": "G1_2", "seed": 7, "topology": 3, "orientation": 1})),
        )
        .await;
    assert_eq!(
        (status, body["id"].as_u64()),
        (StatusCode::CREATED, Some(4242))
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/admin/instances",
            t,
            Some(json!({"area": "Nowhere"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    assert_eq!(
        f.call(
            "POST",
            "/api/v1/admin/players/Bob/transfer",
            t,
            Some(json!({"instance": 5}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/admin/players/Bob/transfer",
            t,
            Some(json!({"area": "G1_3"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/admin/players/Bob/transfer",
            t,
            Some(json!({}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/admin/players/Bob/transfer",
            t,
            Some(json!({"instance": 1, "area": "G1_1"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(
        f.calls(),
        [
            "root: kick Bob",
            "root: kick Ghost",
            "root: mute Bob",
            "root: mute Bob 15",
            "root: unmute Bob",
            "root: announce restart in 5 minutes",
            "root: close 17",
            "root: close 999",
            "root: create NewInstance { area: \"G1_2\", seed: Some(7), topology: Some(3), orientation: Some(1) }",
            "root: create NewInstance { area: \"Nowhere\", seed: None, topology: None, orientation: None }",
            "root: transfer Bob Instance(5)",
            "root: transfer Bob New(NewInstance { area: \"G1_3\", seed: None, topology: None, orientation: None })",
        ]
    );
}

#[tokio::test]
async fn instances_and_characters_are_listed() {
    let f = fixture();
    f.account("root", "correct horse", Role::Admin);
    let t = f.login("root", "correct horse").await;
    let t = Some(t.as_str());
    let (status, body) = f.call("GET", "/api/v1/admin/instances", t, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (
            body[0]["area"].as_str(),
            body[0]["seed"].as_u64(),
            body[0]["persistent"].as_bool()
        ),
        (Some("G1_1"), Some(5), Some(true))
    );

    let account = f.state.accounts.find("root").unwrap().unwrap();
    let new = NewCharacter {
        account_id: account.id,
        name: "Bob".into(),
        league: "Standard".into(),
        class_id: 2,
        start_state: "DexFourDefault".into(),
        level: 12,
        area: "G1_2".into(),
    };
    let c = f.state.store.create_character(&new).unwrap().unwrap();
    f.state
        .store
        .create_character(&NewCharacter {
            name: "Carl".into(),
            ..new
        })
        .unwrap()
        .unwrap();
    let (status, body) = f.call("GET", "/api/v1/admin/characters", t, None).await;
    assert_eq!(status, StatusCode::OK);
    let list = body.as_array().unwrap();
    assert_eq!(list.len(), 2);
    // Bob is online in the mock's session list; Carl is not.
    assert_eq!(
        (
            list[0]["name"].as_str(),
            list[0]["online"].as_bool(),
            list[0]["account"].as_str()
        ),
        (Some("Bob"), Some(true), Some("root"))
    );
    assert_eq!(list[1]["online"], false);
    let (status, body) = f
        .call("GET", "/api/v1/admin/characters?account=root", t, None)
        .await;
    assert_eq!(
        (status, body.as_array().map(Vec::len)),
        (StatusCode::OK, Some(2))
    );
    assert_eq!(
        f.call("GET", "/api/v1/admin/characters?account=ghost", t, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = f
        .call(
            "GET",
            &format!("/api/v1/admin/characters/{}", c.id),
            t,
            None,
        )
        .await;
    assert_eq!(
        (status, body["level"].as_u64(), body["area"].as_str()),
        (StatusCode::OK, Some(12), Some("G1_2"))
    );
    assert_eq!(
        f.call("GET", "/api/v1/admin/characters/999", t, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn openapi_lists_every_endpoint() {
    let f = fixture();
    let (status, doc) = f.call("GET", "/api/v1/openapi.json", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let paths = doc["paths"].as_object().unwrap();
    for p in [
        "/api/v1/server",
        "/api/v1/register",
        "/api/v1/login",
        "/api/v1/logout",
        "/api/v1/password",
        "/api/v1/game-token",
        "/api/v1/me",
        "/api/v1/web-token",
        "/api/v1/web-token/redeem",
        "/api/v1/admin/sessions",
        "/api/v1/admin/instances",
        "/api/v1/admin/instances/{id}/close",
        "/api/v1/admin/players/{name}/kick",
        "/api/v1/admin/players/{name}/mute",
        "/api/v1/admin/players/{name}/transfer",
        "/api/v1/admin/announce",
        "/api/v1/admin/accounts",
        "/api/v1/admin/accounts/{name}/disable",
        "/api/v1/admin/accounts/{name}/enable",
        "/api/v1/admin/accounts/{name}/role",
        "/api/v1/admin/characters",
        "/api/v1/admin/characters/{id}",
        "/api/v1/admin/logs/ws",
        "/api/v1/admin/logs/ws-ticket",
    ] {
        assert!(
            paths.contains_key(p),
            "{p} missing from the OpenAPI document"
        );
    }
    assert!(doc["components"]["securitySchemes"]["bearer"].is_object());
    assert!(doc["components"]["schemas"]["LoginResponse"].is_object());
    assert_eq!(crate::openapi_json().parse::<Value>().unwrap(), doc);
}

#[tokio::test]
async fn unknown_api_paths_are_json_404() {
    let f = fixture();
    let (status, body) = f.call("GET", "/api/v1/nope", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn serves_the_spa_with_an_index_fallback() {
    let dir = std::env::temp_dir().join(format!("poe-web-api-test-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<html>spa</html>").unwrap();
    std::fs::write(dir.join("assets/app.js"), "console.log(1)").unwrap();
    let config = format!(
        "{CONFIG}\n[web]\nstatic_dir = {:?}\n",
        dir.to_str().unwrap()
    );
    let f = fixture_with(&config);
    let (status, body) = f.call("GET", "/", None, None).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, Some("<html>spa</html>"))
    );
    let (status, body) = f.call("GET", "/assets/app.js", None, None).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, Some("console.log(1)"))
    );
    // A client-side route falls back to index.html.
    let (_, body) = f.call("GET", "/accounts/alice", None, None).await;
    assert_eq!(body.as_str(), Some("<html>spa</html>"));
    // The API keeps its JSON errors and works as usual.
    assert_eq!(
        f.call("GET", "/api/v1/nope", None, None).await.1["error"],
        "no such endpoint"
    );
    assert_eq!(
        f.call("GET", "/api/v1/server", None, None).await.0,
        StatusCode::OK
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn cors_allows_only_configured_origins() {
    let f = fixture_with(&format!(
        "{CONFIG}\n[web]\ncors_origins = [\"http://localhost:5173\"]\n"
    ));
    let ask = |origin: &'static str| {
        Request::builder()
            .method("OPTIONS")
            .uri("/api/v1/login")
            .header("origin", origin)
            .header("access-control-request-method", "POST")
            .body(Body::empty())
            .unwrap()
    };
    let ok = f
        .app
        .clone()
        .oneshot(ask("http://localhost:5173"))
        .await
        .unwrap();
    assert_eq!(
        ok.headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap()),
        Some("http://localhost:5173")
    );
    let other = f
        .app
        .clone()
        .oneshot(ask("http://evil.example"))
        .await
        .unwrap();
    assert!(other.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn log_websocket_needs_an_admin_token_or_a_ticket() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Player);
    f.account("root", "correct horse", Role::Admin);
    let player = f.login("alice", "correct horse").await;
    let admin = f.login("root", "correct horse").await;
    // The bearer header: a player is refused, an admin gets past authentication (and then
    // fails the upgrade: this is not a WebSocket).
    let (status, _) = f.call("GET", "/api/v1/admin/logs/ws", Some(&player), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f.call("GET", "/api/v1/admin/logs/ws", Some(&admin), None).await;
    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    // The web token is not accepted in the URL any more (it would end up in proxy logs).
    let (status, _) = f
        .call("GET", &format!("/api/v1/admin/logs/ws?token={admin}"), None, None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Tickets: only admins mint them.
    let (status, _) = f.call("POST", "/api/v1/admin/logs/ws-ticket", Some(&player), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f.call("POST", "/api/v1/admin/logs/ws-ticket", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = f.call("POST", "/api/v1/admin/logs/ws-ticket", Some(&admin), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["expires_in_secs"], 60);
    let ticket = body["ticket"].as_str().unwrap().to_owned();
    assert_ne!(ticket, admin, "a ticket is not the web token");
    // A ticket works once (then it is gone) and is not a bearer token.
    let uri = format!("/api/v1/admin/logs/ws?ticket={ticket}");
    let (status, _) = f.call("GET", &uri, Some(&ticket), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "tickets are not tokens");
    let (status, _) = f.call("GET", &uri, None, None).await;
    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    let (status, _) = f.call("GET", &uri, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "single use");
    // A ticket of an account that lost the admin role is refused.
    let (_, body) = f.call("POST", "/api/v1/admin/logs/ws-ticket", Some(&admin), None).await;
    let ticket = body["ticket"].as_str().unwrap().to_owned();
    f.state.accounts.set_role("root", Role::Player).unwrap();
    let (status, _) = f
        .call("GET", &format!("/api/v1/admin/logs/ws?ticket={ticket}"), None, None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Events pushed to the buffer reach a subscriber.
    f.state
        .logs
        .push(LogLevel::Info, "t", "hello".into(), Default::default());
    assert_eq!(f.state.logs.history(&Default::default()).len(), 1);
}

/// Every response carries the security headers (API and static files alike).
#[tokio::test]
async fn responses_carry_security_headers() {
    let f = fixture();
    let req = Request::builder().uri("/api/v1/server").body(Body::empty()).unwrap();
    let res = f.app.clone().oneshot(req).await.unwrap();
    let h = res.headers();
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["x-frame-options"], "DENY");
    let csp = h["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"), "{csp}");
    // Errors too.
    let req = Request::builder().uri("/api/v1/nope").body(Body::empty()).unwrap();
    let res = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
}

/// The log stream over a real socket: history first, then live events, filtered per client.
#[tokio::test]
async fn log_websocket_streams_filtered_events() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let f = fixture();
    f.account("root", "correct horse", Role::Admin);
    let token = f.login("root", "correct horse").await;
    let logs = f.state.logs.clone();
    logs.push(
        LogLevel::Info,
        "login_server",
        "old info".into(),
        Default::default(),
    );
    logs.push(
        LogLevel::Debug,
        "poe_net",
        "old debug".into(),
        Default::default(),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = f.app.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // An unknown ticket is refused before the upgrade.
    let bad = tokio_tungstenite::connect_async(format!(
        "ws://{addr}/api/v1/admin/logs/ws?ticket={}",
        "00".repeat(32)
    ))
    .await;
    assert!(bad.is_err());

    let (status, body) = f
        .call("POST", "/api/v1/admin/logs/ws-ticket", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ticket = body["ticket"].as_str().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{addr}/api/v1/admin/logs/ws?ticket={ticket}"
    ))
    .await
    .unwrap();
    async fn next(
        ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    ) -> Value {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("a message in time")
            .unwrap()
            .unwrap();
        serde_json::from_str(msg.to_text().unwrap()).unwrap()
    }
    assert_eq!(next(&mut ws).await, json!({"type": "hello", "buffer": 50}));
    ws.send(Message::text(
        json!({"min_level": "debug", "target": "poe_net"}).to_string(),
    ))
    .await
    .unwrap();
    let e = next(&mut ws).await;
    assert_eq!(
        (e["type"].as_str(), e["event"]["message"].as_str()),
        (Some("event"), Some("old debug"))
    );

    logs.push(
        LogLevel::Info,
        "login_server",
        "ignored".into(),
        Default::default(),
    );
    logs.push(
        LogLevel::Debug,
        "poe_net::conn",
        "live".into(),
        Default::default(),
    );
    let e = next(&mut ws).await;
    assert_eq!(e["event"]["message"], "live");

    // Changing the filter resets and replays the history.
    ws.send(Message::text(
        json!({"min_level": "info", "text": "OLD"}).to_string(),
    ))
    .await
    .unwrap();
    assert_eq!(next(&mut ws).await, json!({"type": "reset"}));
    let e = next(&mut ws).await;
    assert_eq!(e["event"]["message"], "old info");
    ws.send(Message::text("{not json")).await.unwrap();
    assert_eq!(next(&mut ws).await["type"], "error");
}

/// `docs/architecture/openapi.json` is what `cargo xtask openapi` writes.
#[test]
fn committed_openapi_document_is_current() {
    let committed = include_str!("../../../docs/architecture/openapi.json");
    assert_eq!(
        committed,
        crate::openapi_json() + "\n",
        "run `cargo xtask openapi`"
    );
}

/// A login request from `ip` (socket address, as `serve` provides it) with optional headers.
async fn login_from(
    f: &Fixture,
    ip: &str,
    name: &str,
    password: &str,
    forwarded: Option<&str>,
) -> (StatusCode, Option<String>, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/login")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(x) = forwarded {
        req = req.header("x-forwarded-for", x);
    }
    let mut req = req
        .body(Body::from(
            json!({"username": name, "password": password}).to_string(),
        ))
        .unwrap();
    let addr: std::net::SocketAddr = format!("{ip}:5555").parse().unwrap();
    req.extensions_mut().insert(axum::extract::ConnectInfo(addr));
    let res = f.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let retry = res
        .headers()
        .get(header::RETRY_AFTER)
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, retry, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn login_is_rate_limited_per_account_with_429() {
    let f = fixture_with(&format!(
        "{CONFIG}\n[security]\nlogin_max_failures_per_account = 3\nlogin_lockout_secs = 600\n"
    ));
    f.account("alice", "correct horse", Role::Player);
    for _ in 0..3 {
        let (s, retry, _) = login_from(&f, "10.0.0.1", "alice", "wrong guess", None).await;
        assert_eq!((s, retry), (StatusCode::UNAUTHORIZED, None));
    }
    // Locked: 429 + Retry-After, even for the right password and from another IP.
    let (s, retry, body) = login_from(&f, "10.0.0.2", "alice", "correct horse", None).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(retry.as_deref(), Some("600"));
    assert!(body["error"].as_str().unwrap().contains("too many"), "{body}");
    // Other accounts are not affected (and the IP limit of 20 is far away).
    f.account("bob", "correct horse", Role::Player);
    assert_eq!(login_from(&f, "10.0.0.2", "bob", "correct horse", None).await.0, StatusCode::OK);
    // /password counts and is limited with the same keys.
    let token = login_from(&f, "10.0.0.2", "bob", "correct horse", None).await.2["token"]
        .as_str()
        .unwrap()
        .to_owned();
    for _ in 0..3 {
        let (s, _) = f
            .call("POST", "/api/v1/password", Some(&token), Some(json!({"old_password": "nope nope", "new_password": "another horse"})))
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }
    let (s, _) = f
        .call("POST", "/api/v1/password", Some(&token), Some(json!({"old_password": "correct horse", "new_password": "another horse"})))
        .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn login_is_rate_limited_per_ip_and_honours_forwarded_for() {
    let cfg = format!("{CONFIG}\n[security]\nlogin_max_failures_per_ip = 2\n");
    let f = fixture_with(&cfg);
    f.account("alice", "correct horse", Role::Player);
    for name in ["ghost1", "ghost2"] {
        assert_eq!(login_from(&f, "10.0.0.1", name, "x", None).await.0, StatusCode::UNAUTHORIZED);
    }
    // The IP is locked for every name; another IP is fine.
    assert_eq!(login_from(&f, "10.0.0.1", "alice", "correct horse", None).await.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(login_from(&f, "10.0.0.2", "alice", "correct horse", None).await.0, StatusCode::OK);
    // X-Forwarded-For is ignored unless trusted: a client cannot pick its own IP.
    assert_eq!(
        login_from(&f, "10.0.0.1", "alice", "correct horse", Some("10.9.9.9")).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );

    let f = fixture_with(&format!("{cfg}trust_forwarded_for = true\n"));
    f.account("alice", "correct horse", Role::Player);
    for name in ["ghost1", "ghost2"] {
        // A client that prepends entries of its own ("10.9.9.9") cannot hide: the proxy's own
        // entry is the last one.
        login_from(&f, "127.0.0.1", name, "x", Some("10.9.9.9, 10.0.0.1")).await;
    }
    // Same proxy socket, a different forwarded client: allowed; the locked client is not.
    assert_eq!(login_from(&f, "127.0.0.1", "alice", "correct horse", Some("10.0.0.7")).await.0, StatusCode::OK);
    assert_eq!(login_from(&f, "127.0.0.1", "alice", "correct horse", Some("10.0.0.1")).await.0, StatusCode::TOO_MANY_REQUESTS);
    // Spoofed leading entries do not choose the IP the limiter sees.
    assert_eq!(login_from(&f, "127.0.0.1", "alice", "correct horse", Some("10.0.0.7, 10.0.0.1")).await.0, StatusCode::TOO_MANY_REQUESTS);
}

/// A launcher token for the account (the launcher's login).
async fn launcher_login(f: &Fixture, name: &str, password: &str) -> String {
    let (status, body) = f
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"username": name, "password": password, "kind": "launcher"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["token"].as_str().unwrap().to_owned()
}

async fn mint_web_token(f: &Fixture, launcher: &str) -> (StatusCode, Value) {
    f.call("POST", "/api/v1/web-token", Some(launcher), None).await
}

async fn redeem(f: &Fixture, token: &str) -> (StatusCode, Value) {
    f.call(
        "POST",
        "/api/v1/web-token/redeem",
        None,
        Some(json!({"token": token})),
    )
    .await
}

#[tokio::test]
async fn me_returns_the_callers_account_for_both_token_kinds() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Admin);
    f.account("bobby", "correct horse", Role::Player);
    let launcher = launcher_login(&f, "alice", "correct horse").await;
    let (status, body) = f.call("GET", "/api/v1/me", Some(&launcher), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["username"].as_str(), body["role"].as_str()), (Some("alice"), Some("admin")));
    assert!(body.get("password_hash").is_none());
    let web = f.login("bobby", "correct horse").await;
    let (status, body) = f.call("GET", "/api/v1/me", Some(&web), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], "player");
    assert_eq!(f.call("GET", "/api/v1/me", None, None).await.0, StatusCode::UNAUTHORIZED);
    // A role change shows at once.
    f.state.accounts.set_role("bobby", Role::Admin).unwrap();
    assert_eq!(f.call("GET", "/api/v1/me", Some(&web), None).await.1["role"], "admin");
}

#[tokio::test]
async fn web_token_is_single_use_and_gives_a_web_session() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Admin);
    let launcher = launcher_login(&f, "alice", "correct horse").await;
    let (status, body) = mint_web_token(&f, &launcher).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 64);
    assert_eq!(body["expires_in_secs"], 60, "the [web] default");
    assert_eq!(body["login_path"], format!("/#/token-login?t={token}"));

    let (status, session) = redeem(&f, &token).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert_eq!(session["kind"], "web");
    assert_eq!(session["account"]["username"], "alice");
    assert_ne!(session["token"], body["token"], "a new token, not the hand-off one");
    // The session works on the admin API; the hand-off token and the launcher token do not.
    let web = session["token"].as_str().unwrap();
    assert_eq!(
        f.call("GET", "/api/v1/admin/accounts", Some(web), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("GET", "/api/v1/admin/accounts", Some(&token), None).await.0,
        StatusCode::UNAUTHORIZED,
        "the hand-off token is not a bearer token"
    );
    // Second use: refused, same answer as an unknown token.
    let (status, again) = redeem(&f, &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(redeem(&f, &"0".repeat(64)).await.1, again);
    // The launcher token is untouched.
    assert_eq!(f.call("GET", "/api/v1/me", Some(&launcher), None).await.0, StatusCode::OK);
}

#[tokio::test]
async fn web_token_follows_the_configured_lifetime() {
    let f = fixture_with(&format!("{CONFIG}\n[web]\nweb_token_ttl_secs = 5\n"));
    f.account("alice", "correct horse", Role::Admin);
    let launcher = launcher_login(&f, "alice", "correct horse").await;
    assert_eq!(mint_web_token(&f, &launcher).await.1["expires_in_secs"], 5);
    // Expiry itself is covered by the WebTokens unit tests (synthetic clock).
}

#[tokio::test]
async fn web_token_needs_an_admins_launcher_token() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Admin);
    f.account("bobby", "correct horse", Role::Player);
    // No token, a web token (even an admin's), a player's launcher token.
    assert_eq!(f.call("POST", "/api/v1/web-token", None, None).await.0, StatusCode::UNAUTHORIZED);
    let web = f.login("alice", "correct horse").await;
    assert_eq!(mint_web_token(&f, &web).await.0, StatusCode::FORBIDDEN);
    let player = launcher_login(&f, "bobby", "correct horse").await;
    assert_eq!(mint_web_token(&f, &player).await.0, StatusCode::FORBIDDEN);
    assert!(f.state.web_tokens.is_empty(), "nothing was minted");
    // A game token is not a launcher token.
    let launcher = launcher_login(&f, "alice", "correct horse").await;
    let game = f.call("POST", "/api/v1/game-token", Some(&launcher), None).await.1["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(mint_web_token(&f, &game).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_new_web_token_replaces_the_previous_one() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Admin);
    let launcher = launcher_login(&f, "alice", "correct horse").await;
    let first = mint_web_token(&f, &launcher).await.1["token"].as_str().unwrap().to_owned();
    let second = mint_web_token(&f, &launcher).await.1["token"].as_str().unwrap().to_owned();
    assert_eq!(redeem(&f, &first).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(redeem(&f, &second).await.0, StatusCode::OK);
}

#[tokio::test]
async fn a_web_token_for_a_disabled_account_is_refused() {
    let f = fixture();
    f.account("alice", "correct horse", Role::Admin);
    let launcher = launcher_login(&f, "alice", "correct horse").await;
    let token = mint_web_token(&f, &launcher).await.1["token"].as_str().unwrap().to_owned();
    f.state.accounts.set_disabled("alice", true).unwrap();
    assert_eq!(redeem(&f, &token).await.0, StatusCode::FORBIDDEN);
    // It was consumed by the attempt.
    f.state.accounts.set_disabled("alice", false).unwrap();
    assert_eq!(redeem(&f, &token).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn web_token_responses_are_not_cacheable_and_bad_redeems_are_rate_limited() {
    let f = fixture_with(&format!("{CONFIG}\n[security]\nlogin_max_failures_per_account = 3\nlogin_lockout_secs = 600\n"));
    f.account("alice", "correct horse", Role::Admin);
    let launcher = launcher_login(&f, "alice", "correct horse").await;
    let res = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/web-token")
                .header(header::AUTHORIZATION, format!("Bearer {launcher}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.headers().get(header::CACHE_CONTROL).unwrap(), "no-store");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let good = serde_json::from_slice::<Value>(&bytes).unwrap()["token"].as_str().unwrap().to_owned();

    // Malformed and unknown tokens count; after 3 the client is locked, even for the real token.
    for bad in ["nothex", &"ab".repeat(32), &"cd".repeat(32)] {
        assert_eq!(redeem(&f, bad).await.0, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = redeem(&f, &good).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // The lock is for the failing client only (here 0.0.0.0, the test default); a client with
    // another address can still redeem, and a normal login is not affected by the pseudo name.
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/web-token/redeem")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"token": good}).to_string()))
        .unwrap();
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo("10.0.0.9:5555".parse::<std::net::SocketAddr>().unwrap()));
    let res = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(login_from(&f, "10.0.0.1", "alice", "correct horse", None).await.0, StatusCode::OK);
}
