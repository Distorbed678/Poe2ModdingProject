# Web API

The server's HTTP and WebSocket API, for the web admin site ([Poe2WebAdmin](#the-web-admin-spa)) and the launcher ([../guides/launcher.md](../guides/launcher.md)). It is the crate `crates/poe-web-api` (axum 0.8), started by `poe-server` next to the login and instance servers. The machine-readable description is [openapi.json](openapi.json), generated from the code (`cargo xtask openapi`) and also served live at `GET /api/v1/openapi.json`.

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
| `[web] web_token_ttl_secs` | `60` | lifetime of the single-use web sign-in token ([below](#opening-the-web-admin-from-the-launcher)) |
| `[web] config_editor` | `true` | the web admin's config editor may save the config file ([below](#config-editor)); `false`: it is read-only |
| `[web] config_backup_dir`, `config_backup_keep` | `runtime/config-backups`, `20` | where the editor keeps the previous file on every save, and how many |
| `[server] motd` | empty | message of the day, `GET /server` field `motd` ([below](#message-of-the-day)) |
| `[metrics] enabled`, `interval_secs`, `retention_hours` | `true`, `5`, `24` | the sampler behind the dashboard charts ([below](#metrics)) |
| `[scheduler] enabled`, `history_keep`, `missed_grace_secs` | `true`, `100`, `120` | scheduled tasks ([below](#scheduled-tasks)) |
| `[accounts] open_registration` | `false` | whether `POST /register` works |

Start the server and make an admin as described in [../guides/setup.md](../guides/setup.md#web-api) (`curl localhost:8080/api/v1/server`, `POST /api/v1/login` with `{"username", "password"}`, then `Authorization: Bearer <token>` on `/api/v1/admin/...`).


`cargo xtask openapi` rewrites `docs/architecture/openapi.json` (`--check` fails when it is stale; a unit test in the crate fails too). The SPA and the launcher generate their types from that file.

## Authentication

- **Passwords are plain text in the request body** (`register`, `login`, `password`). The server turns them into the client's digest itself (`poe_accounts::password_digest`: SHA-256 of the UTF-16LE text) and then calls `AccountStore` exactly as the game login does (argon2id over the digest). So an account made on the web logs in in game with the same password, and the other way round. Minimum 8 characters, maximum 256.
- **Tokens** come from `AccountStore::issue_token`: 32 random bytes sent as 64 hex characters, only their SHA-256 stored, bound to `[cluster] id`, expiring, revocable. `POST /login` returns one of two kinds:
  - `web` (default): 12 hours. For the browser; the only kind the admin endpoints accept.
  - `launcher` (`"kind":"launcher"`): `[accounts] remember_days`. For the launcher, which keeps it in the OS keyring; the game's token login accepts it too, but the launcher does not use it that way (the game would revoke it): it calls `POST /game-token` instead. Never valid for `/admin/*`, so a leaked launcher token cannot administer the server.
- Send `Authorization: Bearer <token>`. The log WebSocket cannot use the header from a browser, so it takes a **single-use ticket** instead: `POST /admin/logs/ws-ticket` (admin web token) returns `{ticket, expires_in_secs}` (valid `[web] web_token_ttl_secs`, separate from the web sign-in tokens), and `GET /admin/logs/ws?ticket=<ticket>` redeems it once. The session token itself is never accepted in a URL (proxy access logs record URLs). Non-browser clients can use the `Authorization` header on the WebSocket request.
- A token stops working when it is revoked (`POST /logout`, a password change or `account passwd`, disabling the account), expired, from another cluster, or its account is disabled: `401` with the reason. The role is read at every call, so a demoted admin loses access at once.
- Wrong username and wrong password give the same `401` (names cannot be probed); a disabled account with the right password gets `403`.
- **Rate limiting:** `POST /login` and the old-password check of `POST /password` count failures per account name and per client IP (`[security]`, shared with the game login; see [accounts.md](accounts.md#password-attempt-limits)). A locked name or IP gets `429` with a `Retry-After` header (seconds) before the password is checked, also for the right password. The client IP is the socket address; with `[security] trust_forwarded_for = true` (behind your own TLS proxy) the **last** `X-Forwarded-For` entry, which is the one the proxy appended (a client can prepend its own, so the first entry is never used).

### Opening the web admin from the launcher

The launcher's "Open web admin" button (WP F3) opens the browser already signed in, without a second password prompt. It uses three endpoints:

1. `GET /me` with the launcher token: the account and its `role`. The launcher shows the button only for `admin`. (`POST /login` returns the same `account` object, so a launcher that keeps the role from its login does not need this call; `/me` is for a launcher that restarts with only a stored token, and it reflects role changes at once.)
2. `POST /web-token` with the launcher token: returns `{token, expires_in_secs, login_path}`. The token is 32 random bytes as 64 hex characters, valid for `[web] web_token_ttl_secs` (60 by default) and usable **once**. `login_path` is `/#/token-login?t=<token>`: the launcher opens `<server web address>` + `login_path` in the system browser.
3. The web admin's `#/token-login` page reads the token and removes it from the address bar and history right away (`history.replaceState`), then calls `POST /web-token/redeem` `{token}`, which returns the same response as `POST /login` with `kind: "web"` (a 12 hour session). The SPA keeps it in `sessionStorage` like a password login and shows the dashboard. A link that is expired, used, malformed or for a non-admin leads to the login page with a notice.

Security properties:

- **Single use and short-lived.** Redeeming removes the token whatever the outcome; an expired token is refused. Asking for a new one replaces the account's older unused token. At most 1024 unused tokens exist at once (`503` beyond that).
- **Bound to the account.** The token maps to one account id, fixed when it is minted from a launcher token of that account. Only a launcher token of an admin may mint one (a web token, a game token or a player's token gets `403`/`401`). A disabled account's token is refused (`403`) at redemption.
- **Not stored raw, not logged.** The server keeps only the SHA-256 of the token, in memory (a restart drops pending tokens). Lookup is by that hash of a random 256-bit value, so there is no comparison to time. Neither the token nor the web session token it yields appears in a log line (the log says `web sign-in token issued` / `redeemed` with the account name).
- **Kept out of requests and logs on the way.** The token travels in the URL *fragment*, which the browser never sends to the server, a proxy or a `Referer` header. The mint and redeem responses carry `Cache-Control: no-store`.
- **Rate limited like login.** Failed redemptions (unknown, used, expired or malformed token) are counted per client IP in the `[security]` limiter shared with the password logins, under a per-IP pseudo name, so one client cannot lock the others out; at the limit `429` with `Retry-After`. The token itself is a 256-bit random value and cannot be guessed; the limiter only bounds the noise.
- **No effect on other tokens.** Redeeming issues a new `web` token; the launcher token is untouched.

Errors are JSON, `{"error": "..."}`, with `400` bad input, `401` no or bad token, `403` not an admin (or registration closed), `404` unknown player, instance, account or character, `409` name taken, `429` too many failed password attempts (with `Retry-After`).

## Endpoints

All under `/api/v1`. Full schemas: [openapi.json](openapi.json).

| Endpoint | Auth | |
|---|---|---|
| `GET /server` | none | name, version, cluster id, players online, `open_registration`, `login_host`/`login_port` (where the game client connects; the public login port), `instance_port` (public), `gateway_region`, uptime, and `motd` (the message of the day, plain text of at most 280 characters; absent when there is none) |
| `POST /register` `{username, password}` | none | new player account; `403` unless `open_registration`, `409` when taken |
| `POST /login` `{username, password, kind?}` | none | `{token, kind, expires_at, account}` |
| `POST /logout` | any token | revokes the token used (`204`) |
| `POST /password` `{old_password, new_password}` | any token | changes the password, revokes every token of the account (game Remember Me included) and returns a fresh token of the same kind |
| `POST /game-token` | launcher token | exchanges it for a fresh `game_remember` token (same response as `/login`, `kind: "game_remember"`) for the client's credentials file; web tokens get `403`. The launcher uses it for auto-login: the game's token login retires the token it uses, so the launcher token itself must never be handed to the game |
| `GET /me` | any token | the calling account (`username`, `role` `player`/`admin`, state); for the launcher to decide whether to offer "Open web admin" |
| `POST /web-token` | launcher token of an admin | `{token, expires_in_secs, login_path}`: a single-use web sign-in token (`[web] web_token_ttl_secs`), see [Opening the web admin from the launcher](#opening-the-web-admin-from-the-launcher). `403` for web or non-admin tokens, `503` when too many unused tokens wait |
| `POST /web-token/redeem` `{token}` | none (the token is the credential) | the same response as `POST /login` (`kind: "web"`); `401` unknown, used or expired (all alike), `403` disabled account, `429` too many failures from this IP |
| `GET /openapi.json` | none | the OpenAPI document |
| `GET /admin/sessions` | admin | players in game: character, account, level and class id (from the character row), instance, area, `connected_at`, `ip` (the client's address: admin API only), `muted` |
| `GET /admin/instances` | admin | every instance (area, seed, forced layout, players, age, persistent) |
| `POST /admin/instances` `{area, seed?, topology?, orientation?}` | admin | creates an instance (`201 {id}`) |
| `POST /admin/instances/{id}/close` | admin | closes it; its players are disconnected |
| `POST /admin/players/{name}/kick` | admin | disconnects a character |
| `POST /admin/players/{name}/mute` `{minutes?}` / `.../unmute` | admin | chat mute |
| `POST /admin/players/{name}/transfer` `{instance}` or `{area, seed?, topology?, orientation?}` | admin | moves a character into an existing or a new instance |
| `POST /admin/announce` `{message}` | admin | a `<Server>` chat line to everyone |
| `GET /admin/bans[?active=true]` | admin | the bans, newest first, with the history of lifted and expired ones ([Bans](#bans)) |
| `POST /admin/bans` `{account` or `ip, reason?, duration_secs?` or `expires_at?}` | admin | bans an account (logins refused, tokens revoked, sessions ended) or an address; `201`. `400` for yourself, an admin, your own address, a bad address, reason or length; `404` unknown account; `409` already banned |
| `POST /admin/bans/{id}/lift` | admin | lifts an active ban early (`404` unknown, `409` already over) |
| `GET /admin/metrics[?since=<ms>&range_secs=&max_points=]` | admin | the samples of the dashboard charts ([Metrics](#metrics)) |
| `POST /admin/motd` `{motd}` | admin | sets the message of the day: live, and written into the config file ([Message of the day](#message-of-the-day)) |
| `GET /admin/config` | admin | the config file the server runs from: text (token-like values hidden), revision, the form schema and values, warnings ([Config editor](#config-editor)); `409` when the server was not started from a file |
| `POST /admin/config/validate` `{content}` | admin | parses the text like the server would: `{valid, error{message,line,column}, warnings}` |
| `POST /admin/config/fields` `{content, set{path: value}}` | admin | writes form values into the text (comments kept) and returns it with its form values; with an empty `set` it only reads them |
| `POST /admin/config/save` `{content, base_revision, restart?}` | admin | validates, backs up and replaces the file; `restart` (same options as the restart endpoint) restarts afterwards. `400` with the line and column, `403` editor off, `409` the file changed since `base_revision` |
| `GET /admin/config/backups`, `/{name}` | admin | the saved previous versions and their text |
| `GET /admin/schedule` | admin | the time zone and every task with its next and last run ([Scheduled tasks](#scheduled-tasks)) |
| `POST /admin/schedule/tasks`, `/{id}`, `/{id}/enable`, `/{id}/disable`, `/{id}/delete` | admin | create (`201`), replace, turn on or off, delete a task (`TaskBody`: `{name, enabled?, task{kind: "restart", delay_secs?, message?}, schedule{type: ...}}`) |
| `GET /admin/schedule/tasks/{id}/runs` | admin | the task's history, newest first |
| `POST /admin/schedule/preview` `{schedule}` | admin | the schedule in words and its next 5 runs, or `400` with the problem |
| `POST /admin/server/stop` `{delay_secs?, message?}` | admin | stops the server: `202` with the pending shutdown at once, then the countdown runs on the server (see [Stop and restart](#stop-and-restart)). `400` for a delay above 3600 s or a message above 200 characters, `409` when a stop or restart is already pending. The body is optional (no delay) |
| `POST /admin/server/restart` `{delay_secs?, message?}` | admin | the same, then the server starts again (`[server] restart_mode`) |
| `POST /admin/server/cancel` | admin | cancels the pending stop or restart while its countdown runs (`404` nothing pending, `409` too late) |
| `GET /admin/server/shutdown` | admin | `{pending: null}` or `{pending: {action, remaining_secs, delay_secs, message, requested_by, cancellable}}` |
| `GET /admin/accounts` | admin | every account (never the password hash) |
| `POST /admin/accounts/{name}/disable` / `enable` | admin | returns the account; disabling revokes its tokens and ends its sessions |
| `POST /admin/accounts/{name}/role` `{role}` | admin | `player` or `admin`; not on yourself |
| `GET /admin/characters[?account=name]` | admin | characters of all accounts or one, with an `online` flag |
| `GET /admin/characters/{id}` | admin | one character (level, area, saved location, ...) |
| `POST /admin/logs/ws-ticket` | admin | single-use ticket for the log WebSocket |
| `GET /admin/logs/ws` | admin | live log, below (`?ticket=` or the `Authorization` header) |

Character names and account names in paths must not contain whitespace (they go into registry command lines).

### How the admin endpoints run

They do not reimplement anything. `poe_cluster::AdminControl` (`crates/poe-cluster/src/admin.rs`) is the small surface the API needs from the instance server, which implements it (`servers/instance-server/src/admin.rs`):

- `online()`: the live sessions (`Live::online`).
- `run_command(actor, line)`: parses `line` with the command registry (`commands.rs`), checks the actor's admin role now, and runs the session-free commands (`who`, `kick`, `mute`, `unmute`, `announce`, `instances`, `instance_new`, `close`, `account ...`) through the same code as chat (`exec.rs`: `run_global`/`execute_global`). Commands that need the caller's own game session (`where`, `join`, `teleport`, `terrain`, `area_info`) are refused. The call is logged and appended to the audit file as `<admin> (web)`, with passwords redacted.
- `create_instance(actor, ...)` and `transfer_player(actor, character, target)`: `instance_new`, and the `transfer` command applied to another player. A transfer hands the account's login connection a ticket for the instance and then ends the player's instance session; the client falls back to character selection and receives the pending `InstanceDetails` ([../guides/server-admin.md](../guides/server-admin.md#transfers)).

- `record(actor, action, result)`: logs an admin action that is not a registry command (a server stop, restart or cancel, a ban, a config save, a schedule change) and appends it to the audit file, as `<admin> (web)`.
- `end_sessions(account_id)`: ends an account's login connections and game sessions (`LoginSessions::supersede`), after a ban. `online()` rows also carry the account id, when the session started, the client's address (the instance connection's peer) and whether the character is muted.

Instance listing reads `InstanceDirectory` directly; characters come from `poe-store` (`Store::all_characters`, `character_by_id`); accounts from `AccountStore`. The crate depends on those traits only, so the tests run it against an in-memory database and a mock `AdminControl`.

### Live log: `GET /admin/logs/ws`

`poe-server` adds `poe_web_api::LogLayer` to its tracing subscriber. The layer stores each event (`seq`, `ts_ms`, `level`, `target`, `message`, `fields`) in a ring buffer of `[web] log_buffer` events and broadcasts it. What the buffer holds is what the server's own log filter lets through (`[logging] filter`, `RUST_LOG`): to see `debug` events in the viewer, enable them there.

Protocol (JSON text frames), after the upgrade:

1. Server: `{"type":"hello","buffer":2000}`.
2. Client: a filter, `{"min_level":"info","target":"instance_server","text":"kick","history":200}`. Every field is optional (`min_level` defaults to `info`, `history` to 200; `target` is a prefix; `text` matches message and fields, case-insensitively). If nothing arrives within 500 ms, the defaults apply.
3. Server: the newest `history` matching buffered events, oldest first, then new matching events as they happen: `{"type":"event","event":{...}}`.
4. The client may send a new filter at any time; the server answers `{"type":"reset"}` and replays the history under the new filter.
5. `{"type":"lagged","missed":n}` when the client reads too slowly; `{"type":"error","message":...}` for a bad filter.

### Stop and restart

`crates/poe-web-api/src/lifecycle.rs` owns the request, `servers/poe-server/src/lifecycle.rs` the end of the process.

1. `POST /admin/server/stop|restart` validates the body (`delay_secs` 0 to 3600, `message` up to 200 characters, whitespace normalised), refuses a second request with `409`, audits `server stop|restart` and answers `202`. A countdown task starts.
2. The task announces through the `announce` command (a `<Server>` chat line, audited like every announce): at the start (`Server restarting in 1 min 30 s: <message>`), then at each of 10 min, 5 min, 2 min, 1 min, 30 s, 10 s, 5 s below the delay, and `Server restarting now` at the end. Until that last line the request can be cancelled (`POST /admin/server/cancel` announces the cancellation).
3. 1.5 s after the last line the task hands the request over a channel (`Lifecycle::new` returns the receiver) to `poe-server`. The process stops the login and instance servers, kicks every session and waits up to 5 s for them to end (a session saves its character when it ends; the database commits every write at once, so nothing else is pending), then ends according to `[server] restart_mode`:

| Request | `restart_mode` | The process |
|---|---|---|
| stop (also SIGTERM and Ctrl+C, which skip the countdown) | any | exits with code 0 |
| restart | `exec` (default) | starts itself again: Unix `exec` of the same binary with the same arguments, environment and working directory (same pid, so a parent that waits for the process, like the launcher's Host page or a systemd unit, keeps seeing it; every socket is closed by the exec); Windows has no `exec`: it spawns the new process (which waits 2 s for the ports, `POE_SERVER_RESTART_WAIT_MS`) and exits 0, so a parent sees an exit |
| restart | `exit` | exits with code **75** (`poe_config::RESTART_EXIT_CODE`, `EX_TEMPFAIL`) and leaves the start to a supervisor |

A binary that was replaced while running (an upgrade) is found again at its path (Linux reports `<path> (deleted)`). If the restart itself fails (exec error), the process logs it and exits 75. Web sessions are rows in the database, so they survive a restart; the SPA polls `GET /server` and `GET /admin/server/shutdown`, shows the countdown and "restarting" in a banner, and signs nobody out. Under a supervisor use `restart_mode = "exit"` ([../guides/hosting.md](../guides/hosting.md#running-it-permanently)).

### Bans

`POST /admin/bans` bans an **account** or an **IP address**, for a length (`duration_secs`, 60 s to 10 years), until a date (`expires_at`) or for good (neither). The row goes into the `bans` table (migration 0007) and stays as history when it expires or is lifted (`POST /admin/bans/{id}/lift`).

- **Account ban.** `AccountStore::authenticate` and `authenticate_token` (`crates/poe-accounts`) refuse a banned account with `AccountError::Banned { reason, until }` *after* the password check, so a wrong password never learns whether the account is banned. That one place covers the game login (the client shows the ban's text: "This account is banned until 2026-10-29 04:00 UTC. Reason: ..."), the web login (`403` with the same text), launcher and game tokens and the web sign-in link. Creating the ban also revokes the account's tokens and ends its login connections and game sessions at once (`AdminControl::end_sessions`, which is `LoginSessions::supersede`, the same as disabling an account).
- **IP ban.** The address (IPv4, or IPv6 with IPv4-mapped addresses folded into IPv4) is checked before the password and the rate limiter by the game login (`servers/login-server/src/auth.rs`) and by the web API's `login`, `register` and sign-in-link redemption, using the same client address as the login limiter (the socket, or the last `X-Forwarded-For` entry with `[security] trust_forwarded_for`). It is one address, no ranges. Behind a shared connection a ban affects everyone using it.
- **Safety rules.** An admin cannot be banned (take the role away first), nobody can ban themselves, and the address you are calling from cannot be banned (it would lock you out). A second active ban of the same target is `409`. The reason is plain text up to 200 characters and is shown to the player.
- **Audit.** `ban account|ip`, `unban account|ip` and the target (and the reason) are appended to the audit file as `<admin> (web)`.

### Metrics

`poe_cluster::metrics` (`crates/poe-cluster/src/metrics.rs`). The instance actors record the time of every simulation frame in `Actors::ticks` (`TickStats`); a sampler task (`spawn_sampler`, started by `poe-server`) takes a `Sample` every `[metrics] interval_secs` into a ring buffer sized for `[metrics] retention_hours` (24 h at 5 s: 17 280 samples, a few MB; lost on a restart).

Each sample holds the time (ms), `players` (sessions in game), `instances` and `occupied` (with a player), the frames of the interval (`tick_count`) with their mean and longest time in microseconds (the meter is read and reset by each sample; no frames while nobody is in game), `rss_bytes` of the server process and `cpu_pct` (percent of one core since the previous sample). Memory and CPU come from `/proc/self` on Linux (`VmRSS`, `utime + stime` at 100 ticks per second); elsewhere they are `0` and absent.

`GET /admin/metrics` answers `{enabled, interval_secs, retention_secs, now, samples}`. `since` (ms) returns only newer samples, `range_secs` the last N seconds, `max_points` (2 to 5000, default 600) merges longer ranges into buckets that keep the peak of players, instances, memory and longest frame, the frame-weighted mean frame time and the mean CPU. The dashboard reloads its range every 5 s (15 min, 1 h) up to every minute (24 h) instead of using a WebSocket or SSE stream; `since` is there for other clients that want to append.

### Config editor

`crates/poe-web-api/src/configfile.rs`. The editor reads and writes **the file the server was started with**: `Config::load` records its absolute path (`Config::source_path`: `POE_CONFIG`, else `config/server.toml`) and nothing else is ever written (`409` when the server has no file, as in tests). `[web] config_editor = false` makes it read-only (`403` on save).

- **Form and raw editor.** `GET /admin/config` returns the text, its `revision` (SHA-256 prefix), the form's field schema (`poe_config::schema::common_fields`: section, key, label, help, type, bounds, default, restart needed; a test checks every default, minimum and maximum against the typed config) and the values the file sets. The form edits those keys; `POST /admin/config/fields` writes them into the text with `toml_edit`, so comments and layout stay, and returns the new text. The raw tab edits the text itself; `POST /admin/config/validate` is the live syntax check. Keys outside the form are edited raw.
- **Validation before saving.** The text is checked exactly like at start: TOML syntax, the typed sections (unknown values, wrong types, ranges), then the `[game] version` file (`Config::check_text`). An invalid text is refused with its **line and column** (`400`) and nothing changes. A valid text with dangerous settings (`Config::security_warnings`: a non-loopback web bind, open registration on a reachable API, the removed Steam login key) is saved and returns the warnings; the page shows them.
- **Diff, backup, atomic write.** The page shows a line diff of the old and new text before saving. A save copies the previous file to `[web] config_backup_dir` as `<name>.<UTC timestamp>` (`-001`, `-002` for several in one second; mode 0600; the oldest beyond `config_backup_keep` are deleted), then writes a temporary file beside the original and renames it over it (the permissions of the original are kept). `GET /admin/config/backups` lists them and the page can load one into the editor.
- **Conflicts.** The save carries the `revision` of the text the editor opened; if the file changed since (a hand edit), it is refused with `409` and nothing is overwritten.
- **Save and restart.** `restart: {delay_secs?, message?}` requests the restart of [Stop and restart](#stop-and-restart) after the file is written (a pending shutdown makes it `restart_error` while the file stays saved). Most keys are read only at start; the message of the day is applied live.
- **Secrets.** The config holds none and must not. Any string value under a token-like key (`token`, `secret`, `password`, `passwd`, `api_key`, `private_key`, `credential`) is replaced by `<hidden>` in everything the editor shows (`poe_config::edit::redact`) and restored from the file on save; a placeholder with nothing to restore is refused. Contents are never logged or audited: the audit line says `config save`, the number of changed lines and the backup name.
- **Safe defaults.** The committed `config/server.toml` has the safe values (loopback binds, no registration, no developer tools; tests keep it valid and warning-free). The editor never loosens anything by itself. When it edits that committed file inside a git checkout it says so: run a server others use from your own copy (`POE_CONFIG=runtime/server.toml`), which the editor then edits. New keys come with safe defaults in `config/server.toml` and `config/server.example.toml`.

### Message of the day

`[server] motd`: plain text, at most 280 characters, line breaks allowed, no other control characters (`poe_config::check_motd`; the config refuses a longer one at load). `GET /server` returns it as `motd` when it is not empty (the launcher's Home page shows it). `POST /admin/motd {motd}` changes it: it applies at once (`WebState::motd`, a live copy), and the same text is written into the config file through the config editor's save path (backup, validation, atomic write) so it survives a restart; when the file cannot be written (editor off, no file, read-only) the response says `persisted: false` with the reason and the message holds until a restart. Saving the config in the editor refreshes the live message from the new text.

### Scheduled tasks

`crates/poe-web-api/src/scheduler/`. Tasks live in the database (`scheduled_tasks` and `scheduled_task_runs`, migration 0008): name, `kind` and its `params`, `schedule` (JSON), enabled flag, who made it, last run, last result, next run. `poe-store` keeps `kind`, `params` and `schedule` as opaque JSON; the scheduler owns their meaning.

- **Schedules** (`schedule.rs`), in the server's **local time zone** (the `TZ` variable, else the system zone; the page and `GET /admin/schedule` show the zone name and offset): `daily` (`HH:MM`), `weekly` (`days` mon..sun, `time`), `interval` (`every_secs`, 60 s to 30 days, counted from the last run or the server start) and `cron` (five fields `minute hour day-of-month month day-of-week`; `*`, numbers, ranges `a-b`, lists, steps `*/n` `a-b/n` `a/n`, names `jan` and `mon-fri`; `0` and `7` are Sunday; if both day fields are restricted either may match, as in Vixie cron). A local time skipped when the clocks go forward runs right after the gap; one that happens twice when they go back runs the first time only.
- **Kinds** (`kind.rs`): the enum `TaskKind`, serialized with a `kind` tag. Today only `restart` (`delay_secs`, `message`: like `POST /admin/server/restart`, announced in chat during the countdown). To add a kind: a variant with its options, a `tag`, `validate`, `describe` and `execute` arm, then the web admin's task form and `cargo xtask openapi`; the database and the loop need no change. A task runs as the admin account that made it (so the announcements are audited as that admin) and fails with a clear message when that account is no longer an admin.
- **Running** (`mod.rs`): `Scheduler::run` polls once a second. Before a task runs, its next time is stored, so a run that ends the process (a restart) cannot run again when the server is back. **Missed runs are skipped, not replayed**: a run found more than `[scheduler] missed_grace_secs` (120) after its time, or due while the server was down, is recorded as `missed` in the history and the task moves on; at start the next runs are worked out again from the current time. A restart that finds another shutdown pending is recorded as `skipped`. Results are `ok`, `failed`, `skipped`, `missed`; `history_keep` runs are kept per task. `[scheduler] enabled = false` lists tasks but runs nothing.
- Every create, update, enable, disable and delete is audited, and so is each scheduled restart (`scheduled restart`, task id and name).

## Serving the web admin

Every response (API and static files) carries `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY` and a `Content-Security-Policy` that allows only the app's own scripts, styles, images and connections (`default-src 'self'`, `connect-src 'self'`, `frame-ancestors 'none'`); a reverse proxy that sets the same headers wins. The server has no TLS of its own: put a TLS proxy in front ([../guides/hosting.md](../guides/hosting.md#tls-reverse-proxy-for-the-web-api)).

With `[web] static_dir` set to the SPA build (`modules/web-admin/dist`), the same port serves it: files as they are, any unknown non-API path as `index.html`. Unknown `/api/v1/...` paths stay JSON `404`s. During SPA development run Vite on its own port; its dev server proxies `/api` (and the log WebSocket) to `[web] bind`, so `cors_origins` is only needed when the page calls the API cross-origin.

## The web admin SPA

Repo [Poe2WebAdmin](https://github.com/Distorbed678/Poe2WebAdmin) (private), the submodule `modules/web-admin`. Svelte 5 + Vite + TypeScript, plain CSS in the launcher's design language (the same tokens, sidebar, cards, chips, dialogs and toasts; dark by default, a light theme, the system preference until the user toggles it; animations stop under `prefers-reduced-motion`; icon rail on narrow windows, bottom tab bar on phones). Build and run: [../guides/setup.md](../guides/setup.md#web-admin-build); usage: [../guides/server-admin.md](../guides/server-admin.md#web-admin); the repo's own README covers development.

| Page (hash route) | Uses |
|---|---|
| Login | `POST /login` with `kind: "web"`; accounts that are not admins are refused and their token revoked |
| `#/token-login?t=<token>` | the launcher's one-time sign-in link: strips the token from the URL, redeems it with `POST /web-token/redeem`, then behaves like a login (same admin check); failures land on the login page with a notice |
| `#/dashboard` | `GET /server`, `/admin/instances`, `/admin/sessions`, every 5 s; the **server control** card: Restart and Stop each open a confirmation dialog (optional message to the players, countdown in seconds) and call `POST /admin/server/restart|stop`; a running countdown shows with a Cancel button (`POST /admin/server/cancel`). The **Activity** card draws players and instances, simulation frame time (mean and longest), memory and CPU from `GET /admin/metrics` (range picker 15 min, 1 h, 6 h, 24 h; hand-rolled SVG with the theme's tokens, hover or arrow keys for exact values, reloaded every 5 to 60 s depending on the range). The **Message of the day** card calls `POST /admin/motd` |
| every page | a banner under the top bar from `GET /server` and `GET /admin/server/shutdown` (every 5 s, every 1.5 s while a shutdown is pending or the server does not answer): countdown, restarting, stopped, unreachable; when the server answers again the page checks the session (`GET /me`), toasts and reloads the dashboard |
| `#/logs` | `POST /admin/logs/ws-ticket`, then `GET /admin/logs/ws?ticket=...` for every connection attempt: level, target and text filters go to the server as the filter message (after 300 ms of typing); pause, autoscroll, at most 3000 lines in the DOM, colour by level, reconnect with backoff (0.5 s doubling to 15 s). A socket that never opens triggers an authenticated call so an expired token returns to the login |
| `#/players` (`#/sessions` still works) | **Online** tab: `GET /admin/sessions` every 5 s: character, class and level, account, area, instance, connected for, IP address (admins only), filter and sort; kick, mute (minutes or until unmuted), unmute, transfer (instance id or area/seed/topology/orientation) and **Ban**. **Bans** tab (`#/players?tab=bans`): `GET /admin/bans` with the history optional, New ban (account or address), Lift. The ban dialog takes a reason and a length (quick buttons or `30m`, `12h`, `7d`, `2w`, Permanent) and can ban the player's address too; the Accounts page has a Ban button as well |
| `#/schedule` | `GET /admin/schedule` every 10 s: tasks with next run (relative and in the server's zone), last result, an on/off switch; New or Edit task (restart countdown and message; daily, weekly days, interval, cron) with the server's preview of the next 5 runs (`POST /admin/schedule/preview`); History per task; Delete |
| `#/config` | `GET /admin/config`: **Form** (keys grouped by section with help, defaults, bounds and a "live" or "needs a restart" mark) and **Raw TOML** (live syntax check with line and column, Go to line); **Review changes** shows the diff and the validation, then **Save** or **Save and restart** (`POST /admin/config/save`); Backups lists the saved previous files and loads one into the editor |
| `#/instances` | `GET /admin/instances`; close, create |
| `#/accounts` | `GET /admin/accounts`; role change, disable and enable |
| `#/characters`, `#/characters/{id}` | `GET /admin/characters[?account=]`, `/admin/characters/{id}` |
| Announce (top bar dialog) | `POST /admin/announce` |
| `#/inventory`, `#/passives` | stubs ("waiting for the inventory work"), only when built with `VITE_FEATURE_INVENTORY` / `VITE_FEATURE_PASSIVES` = `true` |

Close, kick, unmute, ban (permanent ones twice), lift, disable, delete and role changes ask for confirmation. The bearer token is kept in `sessionStorage` (tab-lifetime, never `localStorage`); a `401` from any call other than login clears it and shows the login page. The TypeScript types (`src/lib/api/schema.d.ts`) are generated from [openapi.json](openapi.json) with `npm run gen:api` in the SPA repo: after `cargo xtask openapi` changes the spec, regenerate, run `npm run check` and commit the result in Poe2WebAdmin, then bump the submodule here. CI in Poe2WebAdmin runs `npm ci`, `npm run check`, `npm test` and `npm run build`.

## Code map

| File | |
|---|---|
| `crates/poe-web-api/src/lib.rs` | `WebState` (including the shared `LoginLimiter` and the `Lifecycle`), `router`, `serve` |
| `lifecycle.rs` | stop and restart requests: validation helpers, the countdown task, the pending state |
| `auth.rs` | bearer extractors (`Auth`, `AdminAuth`, `WsAdminAuth`) |
| `webtoken.rs` | `WebTokens`: the in-memory store of single-use sign-in tokens |
| `public.rs`, `admin.rs` | handlers (each carries its `utoipa` description) |
| `dto.rs`, `error.rs` | request/response types, error body |
| `logs.rs` | `LogBuffer`, `LogLayer`, `LogFilter` |
| `bans.rs`, `metrics.rs`, `motd.rs` | the ban, metrics and message-of-the-day endpoints |
| `configfile.rs` | the config editor (read, validate, form values, save with backup, backups) |
| `scheduler/` | `schedule.rs` (schedule syntax, next run), `kind.rs` (`TaskKind`, how a task runs), `mod.rs` (`Scheduler`: tasks, the loop, missed runs), `api.rs` (endpoints) |
| `openapi.rs` | the OpenAPI document |
| `tests.rs`, `tests_features.rs` | router tests with tower `oneshot` (plus one real-socket WebSocket test); `tests_features.rs` covers players, bans, metrics, the config editor, the message of the day and the scheduled tasks |

Tests: register on and off, login good and bad, rate limiting per account and per IP (`429`, `Retry-After`, `X-Forwarded-For` only when trusted), tokens required, admin vs player (`403`), revoked token (`401`), foreign cluster, password change, `/me`, web token mint/redeem (single use, lifetime from the config, launcher-admin only, replacement, disabled account, `no-store`, rate limit per IP), accounts, commands reaching the registry, stop and restart (validation, countdown with a paused clock, announcements, 409, cancel, auth and the admin role), instances and characters, OpenAPI coverage and freshness, SPA fallback, CORS, the log ring buffer and the WebSocket stream. The features' tests: every new endpoint without a token (`401`), as a player and with a launcher token (`403`/`401`); ban validation and its effects (login refused with the text, tokens revoked, sessions ended, lifting, address bans on login and register); metrics (since, range, merging, off); the config editor (line and column errors, form values, backup and atomic save, stale revision, restart, trimming, hidden values, editor off, the file that was loaded); the message of the day (live, persisted, limits); tasks (validation, next runs, history, preview). Elsewhere: `poe-config` (`check_text`, `edit`, the form schema against the typed config), `poe-store` (migrations 0007 and 0008 on an upgraded database, the task tables), `poe-accounts` (bans with a controlled clock), `poe-cluster` (the metrics ring buffer, merging and the sampler on a paused clock), `login-server` (the refusal text through the wire) and the scheduler (a paused clock: due runs, skipped, missed and late runs, restart at start, the loop).

## Not done

- TLS in the server itself (use a reverse proxy).
- Inventory and passive-tree endpoints (waiting for the inventory work).
- A registry listing endpoint (`GET /admin/commands`) for generic command forms.
- Class names in the players list (it shows the class id: the API has no game data), address ranges for IP bans, `ban`/`unban` as chat commands or `poe-server account` subcommands (the web admin is the only way to ban today), a push stream for the metrics (the page polls).
