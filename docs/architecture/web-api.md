# Web API

The server's HTTP and WebSocket API, for the web admin site (Poe2WebAdmin, WP4.2) and the launcher (WP5.1). It is the crate `crates/poe-web-api` (axum 0.8), started by `poe-server` next to the login and instance servers. The machine-readable description is [openapi.json](openapi.json), generated from the code (`cargo xtask openapi`) and also served live at `GET /api/v1/openapi.json`.

Related: accounts, tokens and hashing in [accounts.md](accounts.md); the commands the admin endpoints run in [../guides/server-admin.md](../guides/server-admin.md); the plan in [../plans/orchestration-2026-10.md](../plans/orchestration-2026-10.md) (phase 4).

## Running it

`poe-server` starts the API when `[web] enabled` is true (the default). Config (`config/server.toml`, every key in `config/server.example.toml`):

| Key | Default | |
|---|---|---|
| `[web] enabled` | `true` | start the API with poe-server |
| `[web] bind` | `127.0.0.1:8080` | listen address. Loopback by default: passwords travel in request bodies, so put a TLS reverse proxy in front before exposing it |
| `[web] static_dir` | empty | directory with the built web admin; served at `/` with an `index.html` fallback for client-side routes. A set but missing directory logs a warning and serves nothing |
| `[web] cors_origins` | `[]` | browser origins allowed to call the API (for example `["http://localhost:5173"]` for the Vite dev server). Empty: no CORS headers |
| `[web] log_buffer` | `2000` | recent log events kept for the live log viewer |
| `[accounts] open_registration` | `false` | whether `POST /register` works |

```sh
cargo run -p poe-server -- account create alice "a long password"   # offline, the first admin
cargo run -p poe-server -- account role alice admin
cargo run -p poe-server                                              # API on http://127.0.0.1:8080
curl localhost:8080/api/v1/server
T=$(curl -s -XPOST localhost:8080/api/v1/login -H 'content-type: application/json' \
      -d '{"username":"alice","password":"a long password"}' | jq -r .token)
curl localhost:8080/api/v1/admin/accounts -H "authorization: Bearer $T"
```

`cargo xtask openapi` rewrites `docs/architecture/openapi.json` (`--check` fails when it is stale; a unit test in the crate fails too). The SPA and the launcher generate their types from that file.

## Authentication

- **Passwords are plain text in the request body** (`register`, `login`, `password`). The server turns them into the client's digest itself (`poe_accounts::password_digest`: SHA-256 of the UTF-16LE text) and then calls `AccountStore` exactly as the game login does (argon2id over the digest). So an account made on the web logs in in game with the same password, and the other way round. Minimum 8 characters, maximum 256.
- **Tokens** come from `AccountStore::issue_token`: 32 random bytes sent as 64 hex characters, only their SHA-256 stored, bound to `[cluster] id`, expiring, revocable. `POST /login` returns one of two kinds:
  - `web` (default): 12 hours. For the browser; the only kind the admin endpoints accept.
  - `launcher` (`"kind":"launcher"`): `[accounts] remember_days`. For the launcher, which keeps it in the OS keyring; the game's token login accepts it too. Never valid for `/admin/*`, so a leaked launcher token cannot administer the server.
- Send `Authorization: Bearer <token>`. The log WebSocket also takes `?token=<token>`, because browsers cannot set headers on a WebSocket.
- A token stops working when it is revoked (`POST /logout`, a password change or `account passwd`, disabling the account), expired, from another cluster, or its account is disabled: `401` with the reason. The role is read at every call, so a demoted admin loses access at once.
- Wrong username and wrong password give the same `401` (names cannot be probed); a disabled account with the right password gets `403`.
- Not done: rate limiting of login attempts (also missing in game, see [accounts.md](accounts.md)).

Errors are JSON, `{"error": "..."}`, with `400` bad input, `401` no or bad token, `403` not an admin (or registration closed), `404` unknown player, instance, account or character, `409` name taken.

## Endpoints

All under `/api/v1`. Full schemas: [openapi.json](openapi.json).

| Endpoint | Auth | |
|---|---|---|
| `GET /server` | none | name, version, cluster id, players online, `open_registration`, `login_host`/`login_port` (where the game client connects), uptime |
| `POST /register` `{username, password}` | none | new player account; `403` unless `open_registration`, `409` when taken |
| `POST /login` `{username, password, kind?}` | none | `{token, kind, expires_at, account}` |
| `POST /logout` | any token | revokes the token used (`204`) |
| `POST /password` `{old_password, new_password}` | any token | changes the password, revokes every token of the account (game Remember Me included) and returns a fresh token of the same kind |
| `GET /openapi.json` | none | the OpenAPI document |
| `GET /admin/sessions` | admin | players in game: character, account, instance, area |
| `GET /admin/instances` | admin | every instance (area, seed, forced layout, players, age, persistent) |
| `POST /admin/instances` `{area, seed?, topology?, orientation?}` | admin | creates an instance (`201 {id}`) |
| `POST /admin/instances/{id}/close` | admin | closes it; its players are disconnected |
| `POST /admin/players/{name}/kick` | admin | disconnects a character |
| `POST /admin/players/{name}/mute` `{minutes?}` / `.../unmute` | admin | chat mute |
| `POST /admin/players/{name}/transfer` `{instance}` or `{area, seed?, topology?, orientation?}` | admin | moves a character into an existing or a new instance |
| `POST /admin/announce` `{message}` | admin | a `<Server>` chat line to everyone |
| `GET /admin/accounts` | admin | every account (never the password hash) |
| `POST /admin/accounts/{name}/disable` / `enable` | admin | returns the account; disabling revokes its tokens and ends its sessions |
| `POST /admin/accounts/{name}/role` `{role}` | admin | `player` or `admin`; not on yourself |
| `GET /admin/characters[?account=name]` | admin | characters of all accounts or one, with an `online` flag |
| `GET /admin/characters/{id}` | admin | one character (level, area, saved location, ...) |
| `GET /admin/logs/ws` | admin | live log, below |

Character names and account names in paths must not contain whitespace (they go into registry command lines).

### How the admin endpoints run

They do not reimplement anything. `poe_cluster::AdminControl` (`crates/poe-cluster/src/admin.rs`) is the small surface the API needs from the instance server, which implements it (`servers/instance-server/src/admin.rs`):

- `online()`: the live sessions (`Live::online`).
- `run_command(actor, line)`: parses `line` with the command registry (`commands.rs`), checks the actor's admin role now, and runs the session-free commands (`who`, `kick`, `mute`, `unmute`, `announce`, `instances`, `instance_new`, `close`, `account ...`) through the same code as chat (`exec.rs`: `run_global`/`execute_global`). Commands that need the caller's own game session (`where`, `join`, `teleport`, `terrain`, `area_info`) are refused. The call is logged and appended to the audit file as `<admin> (web)`, with passwords redacted.
- `create_instance(actor, ...)` and `transfer_player(actor, character, target)`: `instance_new`, and the `transfer` command applied to another player. A transfer hands the account's login connection a ticket for the instance and then ends the player's instance session; the client falls back to character selection and receives the pending `InstanceDetails` ([../guides/server-admin.md](../guides/server-admin.md#transfers)).

Instance listing reads `InstanceDirectory` directly; characters come from `poe-store` (`Store::all_characters`, `character_by_id`); accounts from `AccountStore`. The crate depends on those traits only, so the tests run it against an in-memory database and a mock `AdminControl`.

### Live log: `GET /admin/logs/ws`

`poe-server` adds `poe_web_api::LogLayer` to its tracing subscriber. The layer stores each event (`seq`, `ts_ms`, `level`, `target`, `message`, `fields`) in a ring buffer of `[web] log_buffer` events and broadcasts it. What the buffer holds is what the server's own log filter lets through (`[logging] filter`, `RUST_LOG`): to see `debug` events in the viewer, enable them there.

Protocol (JSON text frames), after the upgrade:

1. Server: `{"type":"hello","buffer":2000}`.
2. Client: a filter, `{"min_level":"info","target":"instance_server","text":"kick","history":200}`. Every field is optional (`min_level` defaults to `info`, `history` to 200; `target` is a prefix; `text` matches message and fields, case-insensitively). If nothing arrives within 500 ms, the defaults apply.
3. Server: the newest `history` matching buffered events, oldest first, then new matching events as they happen: `{"type":"event","event":{...}}`.
4. The client may send a new filter at any time; the server answers `{"type":"reset"}` and replays the history under the new filter.
5. `{"type":"lagged","missed":n}` when the client reads too slowly; `{"type":"error","message":...}` for a bad filter.

## Serving the web admin

With `[web] static_dir` set to the SPA build (`Poe2WebAdmin/dist`), the same port serves it: files as they are, any unknown non-API path as `index.html`. Unknown `/api/v1/...` paths stay JSON `404`s. During SPA development run Vite on its own port and put its origin in `cors_origins`.

## Code map

| File | |
|---|---|
| `crates/poe-web-api/src/lib.rs` | `WebState`, `router`, `serve` |
| `auth.rs` | bearer extractors (`Auth`, `AdminAuth`, `WsAdminAuth`) |
| `public.rs`, `admin.rs` | handlers (each carries its `utoipa` description) |
| `dto.rs`, `error.rs` | request/response types, error body |
| `logs.rs` | `LogBuffer`, `LogLayer`, `LogFilter` |
| `openapi.rs` | the OpenAPI document |
| `tests.rs` | router tests with tower `oneshot` (plus one real-socket WebSocket test) |

Tests: register on and off, login good and bad, tokens required, admin vs player (`403`), revoked token (`401`), foreign cluster, password change, accounts, commands reaching the registry, instances and characters, OpenAPI coverage and freshness, SPA fallback, CORS, the log ring buffer and the WebSocket stream.

## Not done

- Rate limiting and lockout of password attempts.
- TLS in the server itself (use a reverse proxy).
- Inventory and passive-tree endpoints (waiting for the inventory work).
- A registry listing endpoint (`GET /admin/commands`) for generic command forms.
