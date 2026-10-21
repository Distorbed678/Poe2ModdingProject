# Setup

This is the only place in `docs/` for commands, log locations and unattended tests. The rules
(what may be touched, official servers read-only) are in [../../CLAUDE.md](../../CLAUDE.md).

## Requirements
- Linux with rustup. The toolchain is pinned by `rust-toolchain.toml` to `nightly-2026-08-10`, because `poe_data_tools` needs nightly. Targets: `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`.
- clang and lld (`clang-cl`, `lld-link`), used by cargo-xwin to link the Windows DLL.
- `gh` and `git`, for the tool downloads.
- Steam with Proton Experimental, to run the client.
- `GameFiles/`: a copy of the 0.5.5e client install (not in git). The Steam install itself is never used or modified.

## Tools
Run `third_party/fetch-tools.sh` to download every external tool into `third_party/` (listed in [third_party/README.md](../../third_party/README.md)): poe_data_tools CLI, dat-schema, LibGGPK3, cargo-xwin, the Python venv `pyenv` for `scripts/re` ([../re/README.md](../re/README.md)), reference repos and the pinned toolchain.

Game files (what the files are: [../re/datamining.md](../re/datamining.md)), run from the repo root:
```sh
T=third_party/poe_data_tools/poe_data_tools
$T -p 2 --steam GameFiles list 'data/balance/*.datc64'
$T -p 2 --steam GameFiles extract 'metadata/characters/str/*' out/
$T -p 2 --steam GameFiles dump-tables ...      # tables -> CSV/JSON using the schema
```

## Clone
The repo uses git submodules: `modules/protocol` (Poe2Protocol), `modules/client-hook` (Poe2ClientHook, with a nested `protocol`), `modules/web-admin` (Poe2WebAdmin) and `modules/launcher` (Poe2Launcher, with nested `client-hook` and `protocol`). They are private repos, so run `gh auth login` first:
```sh
git clone --recursive https://github.com/Distorbed678/Poe2ModdingProject.git
# or, after a plain clone / after pulling a changed submodule pointer:
git submodule update --init --recursive
```
Submodule changes: commit and push inside the submodule (`modules/...`) first, then commit the new pointer here.

## Build and test
```sh
cargo build                 # server workspace
cargo test                  # unit tests; poe-data tests read GameFiles if present
cargo xtask build-dll       # modules/client-hook -> version.dll (loader) + poe_hook_core.dll (x86_64-pc-windows-msvc);
                            # output modules/client-hook/target/x86_64-pc-windows-msvc/release/
python3 scripts/check-links.py   # relative links and #anchors in the docs, README.md, CLAUDE.md, dumps/README.md
```
`build-dll` runs `third_party/cargo-xwin/bin/cargo-xwin xwin build -p poe-hook -p poe-hook-core --target x86_64-pc-windows-msvc --release` in `modules/client-hook/` and keeps the MSVC SDK cache in `third_party/xwin-cache`.

## Run
```sh
cargo run -p poe-server     # login (20481) + instance (6112) servers + web API (127.0.0.1:8080), config/server.toml (add --release for play)
cargo xtask deploy-client   # builds the DLLs and installs them into GameFiles/ (--official: read-only, official servers; --no-build)
cargo xtask deploy-core     # rebuilds and installs only the debug core; a running client reloads it within about a second (--no-build)
cargo xtask dll-data        # rewrites only GameFiles/poe-hook-data/ (stats, action types, areas)
cargo xtask run-client      # launches the client through Proton (Steam must be running)
cargo xtask revert-client   # restores GameFiles/ to the original state
cargo xtask openapi         # writes docs/architecture/openapi.json from the web API code (--check: fail if stale)
cargo xtask tracker         # renders docs/status/tracker.toml into the README.md status tables (--check: fail if README.md is stale)
cargo xtask terrain-matrix  # writes docs/status/terrain-matrix.tsv (area x topology x orientation) from the game data and dumps/<version>/terrain/vectors/ (--dumps, --out)
cargo xtask check-build <exe> # what a game build breaks: signatures, offsets, opcode drift, strings, game data, identity (report dumps/<version>/check-build.md; --bundles, --version, --report; exit 0/1/2): new-game-version.md
cargo xtask versions        # copies versions/<version>.toml into the launcher (modules/launcher/versions/); --check only compares
cargo xtask login-token <account> <64 hex digits>   # remember-me token for the standalone client, see below
cargo xtask select-gateway  # only points the remembered gateway of the prefix at our server
```

`deploy-client` does the following (the logic is `poe-hook-deploy` in `modules/client-hook`; xtask passes the paths):
- Backs up `GameFiles/gateway_list.txt` to `gateway_list.txt.orig` (only once).
- Writes a single UTF-16LE gateway entry, built from `[gateway]`, `[network] public_host` and `[network.login] bind` (its port) in `config/server.toml`.
- Copies the loader to `GameFiles/version.dll` and the debug core next to it.
- Writes `dump_dir` (`<repo>/dumps/<version>` as a `Z:\` path) into `GameFiles/poe-hook.toml`; the DLL writes terrain grids and layouts, area exports and client tables there ([../../dumps/README.md](../../dumps/README.md)).
- Writes the DLL's name tables to `GameFiles/poe-hook-data/` (`stats.tsv`, `action_types.tsv`, `areas.tsv` for the area picker).
- Copies `modules/client-hook/poe-hook/poe-hook.toml`, unless a copy already exists (so your local edits survive).
- Sets `login_mode` (`standalone` by default; `--login-mode steam` keeps the Steam login) and `trusted_servers` (`[network] public_host` with the login and instance ports) at the top of that file. `--official` writes `login_mode = "steam"` and `trusted_servers = []`.
- Sets `position_sync_port` in that file's `[features]` to `[network.instance] public_port`, so the side channel (position reports, debug link, terrain upload) goes to the instance server.
- Resets `auto_keys` to `[]`, and the client's remembered gateway (`gateway_id` in the test prefix's `poe2_production_Config.ini`) to ours for private deploys (after an official session it stays on an official realm and the client never contacts our server).
- Writes `steam_appid.txt`.

`cargo xtask --prefix <dir> <command>` (global option) runs `run-client`, `login-token`, `select-gateway` and the gateway selection of `deploy-client` against another Proton prefix, for a second client (`third_party/proton-prefix-2`, a copy of the first; see [hosting.md](hosting.md#local-two-client-test)). `cargo xtask select-gateway` only writes the remembered gateway (`gateway_id`) of the chosen prefix to ours. `run-client` uses its own Proton prefix (`third_party/proton-prefix`) with `WINEDLLOVERRIDES=version=n,b`, clears `GameFiles/poe-hook.log` per launch, and moves pending crash dumps away (see [Crash reports](#crash-reports)). `build-dll` and `deploy-client` are thin wrappers over `poe-hook-deploy` ([../architecture/client-dll.md](../architecture/client-dll.md)).

### Web API

`poe-server` also serves the web API on `[web] bind` (default `127.0.0.1:8080`; `[web] enabled = false` turns it off). Try it: `curl localhost:8080/api/v1/server`. To use the admin endpoints make an admin offline ([Accounts and characters](#accounts-and-characters-offline)), log in with `POST /api/v1/login` and send the token as `Authorization: Bearer <token>`. To serve the built web admin set `[web] static_dir` to its `dist` directory; for development with Vite put its origin in `[web] cors_origins`. Details: [../architecture/web-api.md](../architecture/web-api.md). To run a second server on shifted ports (for a smoke test), copy `config/server.toml`, change `[network.login] bind`, `[network.instance] bind`/`public_port`, `[web] bind`, `[database] path` and the audit log path, and start with `POE_CONFIG=<copy>`.

### Accounts and characters (offline)

These run against the database (also while the server is up), print a result and exit. They are the way to make the first admin; every other account command also works in chat and in the overlay ([server-admin.md](server-admin.md#accounts)).

```sh
cargo run -p poe-server -- account create alice "a long password"   # also: role <name> <player|admin>, disable|enable <name>, passwd <name> <password>, list
cargo run -p poe-server -- account role alice admin
cargo run -p poe-server -- character create <account> <name> <start_state> [area] [league]
cargo run -p poe-server -- character list [account]
```

`character create` makes a character the way the character creation screen does (slot limit and name rules apply). `start_state` is a `CharacterStartStates` id (for example `DexFourDefault`), `area` a `WorldAreas` id (default: the start state's hub area, else `[player] start_area`), `league` defaults to the first `[[leagues]]` entry. A character created with an explicit town or hideout area counts as having played the intro. `POE_CONFIG=<file>` selects another config (and database) for all of these.

### Web admin (build)

The web admin is the Svelte app in the submodule `modules/web-admin` (Node 22 and npm; `git submodule update --init modules/web-admin` if the folder is empty). What it shows and how it talks to the API: [../architecture/web-api.md](../architecture/web-api.md#the-web-admin-spa).

```sh
cd modules/web-admin && npm ci && npm run build      # npm run dev: Vite on port 5173, proxies /api to [web] bind
```

Then set `[web] static_dir = "modules/web-admin/dist"` in `config/server.toml`, restart `poe-server` and open <http://127.0.0.1:8080> (the `[web] bind` address). Only accounts with the `admin` role get in; the session lasts 12 hours.

### Launcher (build and test)

The launcher is the Tauri 2 app in the submodule `modules/launcher` (Node 22, Rust, and on Linux the WebKitGTK stack; what it does and the system packages: [launcher.md](launcher.md)).

```sh
git submodule update --init --recursive
cd modules/launcher && npm ci && npm run tauri dev       # or: npm run tauri build -- --debug
cd src-tauri && cargo test                               # mock keyring, mock web API, fake Proton
```

Against a real local server (a scratch config with shifted ports as described under [Web API](#web-api), `open_registration = true` and an account made offline):

```sh
cd modules/launcher/src-tauri
POE_LAUNCHER_TEST_URL=http://127.0.0.1:18080 POE_LAUNCHER_TEST_USER=alice POE_LAUNCHER_TEST_PASS="a long password" \
  cargo test --test core_flow real_server -- --nocapture
cargo test --test fetch_tool -- --ignored                # downloads and starts the pinned DepotDownloader (never logs in)
```

### Two-client test (LAN)

`scripts/test-mp.sh` runs two real game clients against one server on one machine (Steam must be running); what it checks and the results: [hosting.md](hosting.md#local-two-client-test). Environment: `LAN_IP`, `MP_PASSWORD`, `MP_RES`, `MP_BORDERLESS`, `MP_AREA`, `MP_CLASS` (documented in the script's header).

```sh
scripts/test-mp.sh setup      # build, runtime/server-mp.toml (LAN IP, own database), accounts + one Ranger each in G1_town, prefix 2, deploy
scripts/test-mp.sh server     # poe-server, log runtime/server-mp.log
scripts/test-mp.sh clients    # token login + Play for both (client <n> [acct] for one; acct = duplicate-login test)
scripts/test-mp.sh keys 1 inmove:640,200 indown:0x57 wait:2000 inup:0x57   # poe-hook.1.keys: input for client 1
scripts/test-mp.sh shot 2     # F8 screenshot of client 2 (prints the file)
scripts/test-mp.sh status     # processes, memory, session lines of the server log
scripts/test-mp.sh stop       # kill the clients; then stop the server in its own call; puts prefix 1's window size back and redeploys the private steam-login state (`restore` does just that)
```

The second client uses `third_party/proton-prefix-2` (a copy of the first prefix with its own login credentials, screenshots and gateway) through `cargo xtask --prefix third_party/proton-prefix-2 ...`. `POE_HOOK_KEYS_FILE=poe-hook.<n>.keys` gives each DLL its own input file ([entries](#poe-hookkeys-entries)).

### Remember-me token (launcher auto-login)

With `login_mode = "standalone"` the client's Remember Me stores a 32-byte token issued by the login server (`AuthenticationResult.session_token`). A launcher, or you by hand, can sign an account in without typing the password:

```
cargo xtask login-token <account> <64 hex digits>
```

It writes the token to `third_party/proton-prefix/pfx/drive_c/users/steamuser/Documents/My Games/Path of Exile 2/poe2_production_login.credentials` (32 raw bytes, owner-only) and sets `[LOGIN] username=<account>` and `save_login=true` in `poe2_production_Config.ini` there. Start the client with `run-client` and press **Login** with the password box empty. The client reads both files itself; the DLL has no part in it. The file name is confirmed (the file exists in the test prefix after standalone logins) (`poe_hook_deploy::LoginTokenFiles` still makes the names a parameter). A rejected token (stale, revoked or unknown: the server answers `0x0f` with a notice) makes the client delete the file. Library API: `poe_hook_deploy::write_login_token(prefix_or_documents_dir, account, token)`.

### `login_mode` and `trusted_servers`

`cargo xtask deploy-client` writes `login_mode = "standalone"` into `GameFiles/poe-hook.toml` by default (the DLL's own default too): the client shows the account/password screen and sends `0x0003`, which the login server handles. Existing configs get the keys on the next deploy; to go back set `login_mode = "steam"` and `trusted_servers = []` (or deploy with `--login-mode steam`). `cargo xtask deploy-core` does not touch the config. `scripts/test-run.sh` and `auto_keys` press Enter on the Steam login screen, so for them deploy with `--login-mode steam` (and `[accounts] dev_machine_hash_login = true`); `scripts/test-mp.sh` uses the standalone login with remember-me tokens.

## Config

`config/server.toml` is the working dev config; `config/server.example.toml` is a fully commented copy with every key and its default (a `poe-config` test parses both). `POE_CONFIG=<path>` selects another file. Sections: `[game]`, `[network]` (+ `.login`, `.instance`), `[instance]`, `[movement]`, `[accounts]`, `[database]`, `[dev]`, `[terrain]`, `[spawn]` (+ `.start_tiles.<area>`, `.upload`), `[cluster]`, `[web]`, `[commands]`, `[chat]`, `[player]`, `[inventories]`, `[gateway]`, `[logging]`, `[[leagues]]`. Env overrides: `POE_CONFIG`, `POE_SWEEP` (overrides `[dev] sweep`), `RUST_LOG` (overrides `[logging] filter`).

### Config migration (2026-10)

The server no longer reads the old layout (a config with `[login]`/`[instances]` fails to parse because `[network]` is missing). Old key to new key:

| Old | New |
|---|---|
| `[login] bind` | `[network.login] bind` |
| `[login] public_host`, `[instance] public_host` | `[network] public_host` (one value for both) |
| `[login] close_after_handoff` | `[network.login] close_after_handoff` |
| `[instance] bind` | `[network.instance] bind` |
| `[instance] public_port` | `[network.instance] public_port` |
| `[instances] idle_minutes` | `[instance] idle_minutes` |
| `[instance] tick_ms` | `[instance] tick_ms` (unchanged; `[instance]` and `[instances]` are merged) |
| the four movement-workaround keys of the old `[instance]` (turn cell, turn timing, face cursor, cursor projection) | removed with WP2.3 (the server simulates movement itself, no workarounds); `[movement]` now holds `sprint_buff_rows` only |
| (new) | `[dev] drift_check` (default `false`) and `drift_interval_ms` (default 1000): the drift checker |
| `[instance] known_seeds`, `position_sync`, `movement_trace`, `debug_link`, `sweep`, `sweep_results` | `[dev]` (same names) |
| `[accounts]` (`name_prefix`, `max_characters`, `flags`, `full_access`, `stash_tabs`) | unchanged (`flags` now defaults to `[0x23, 0x03]`) |
| (new) `[security]` | password attempt limits and `trust_forwarded_for` (WP B2; [accounts.md](../architecture/accounts.md#password-attempt-limits)) |
| `[admin] accounts = [...]` | removed: admin is the account's role (`account role <name> admin`; the migration made `Exile1` an admin). A leftover `[admin]` section is ignored |
| (new) `[cluster] id` | cluster identity; login tokens from another id are invalid (default `"local"`) |
| (new) `[accounts] dev_machine_hash_login` | Steam `0x0005` login without a password, auto-creating `Exile<id>` accounts (default `false`; `config/server.toml` sets `true` for now) |
| (new) `[accounts] open_registration` | whether `POST /api/v1/register` may create accounts (default `false`) |
| (new) `[web]` | `enabled` (`true`), `bind` (`127.0.0.1:8080`), `static_dir`, `cors_origins`, `log_buffer` (`2000`): the web API ([../architecture/web-api.md](../architecture/web-api.md)) |
| (new) `[accounts] remember_days` | lifetime of a Remember Me login (default `30`) |
| (new) `[network.login] public_port` | port written into the gateway entry and the trusted list when it differs from the bind port (default: the bind port) |
| (new) `[instance] first_entity_id` | first entity id of an instance's counter (default `12000`, the official value for players) |
| (new) `[game] version` selects `versions/<version>.toml`; `[game] protocol_dir` optional | the version file is the single definition of a build (depots, exe checksum, opcode folder, signature table, accepted client hashes; [new-game-version.md](new-game-version.md)); `protocol_dir` defaults to its `[protocol] opcodes`; `{version}` in `[dev] known_seeds`, `[dev] sweep_results` and `[spawn] layout_dir` is replaced by `[game] version`; an unknown version stops the start-up |
| (new) `[security] allow_unknown_client` | default `false`: a client whose exe hash (`client_hash`) is not a known value of the version is refused with "you need to update"; `true` accepts it (logged). Nothing is refused while the version file's `[client] hashes_confirmed` is `false` ([new-game-version.md](new-game-version.md#what-the-server-does-with-the-clients-version)) |
| (new) `[spawn.upload]` | terrain upload from client DLLs: `enabled`, `max_bytes`, `max_layout_bytes`, `max_grid_bytes`, `wait_ms`, `wait_upload_secs` ([hosting.md](hosting.md#terrain-spawn-spawnupload)) |
| (new) `[player] animation_sets` | the three animation set names of a remote player's blob (official, weapon dependent; the example config omits it) |

Removed keys in a config file are ignored (unknown keys do not fail the load).

## Playing from Steam (non-Steam game shortcut)
One-time setup, after `cargo xtask deploy-client`:
1. In Steam: **Games → Add a Non-Steam Game to My Library… → Browse…** and select `<repo>/GameFiles/PathOfExileSteam.exe`. Click **Add Selected Programs**.
2. In the library, right-click the new entry → **Properties…**:
   - **Shortcut**: optionally rename it, e.g. "PoE2 Private Server". Set **Start in** to `"<repo>/GameFiles/"`.
   - **Launch options**:
     ```
     SteamAppId=2694490 SteamGameId=2694490 WINEDLLOVERRIDES="version=n,b" %command%
     ```
     `version=n,b` loads our `version.dll` (the hook) instead of Wine's. The app id makes the Steam API start as Path of Exile 2, which you own.
   - **Compatibility**: tick **Force the use of a specific Steam Play compatibility tool** and pick **Proton Experimental**.

   Steam creates a separate Proton prefix for the shortcut (`steamapps/compatdata/<shortcut id>`); the official install's `compatdata/2694490` is never used.

To play:
1. Start the server from the repo: `cargo run --release -p poe-server` (leave it running; Ctrl+C stops it).
2. Launch the shortcut from Steam. The login screen shows "Local Server". With the default `login_mode = "standalone"` sign in with an account name and password (make one offline, [above](#accounts-and-characters-offline); Remember Me works). After `deploy-client --login-mode steam` the Steam login screen appears instead and, with `[accounts] dev_machine_hash_login = true`, creates an `Exile<id>` account by itself.

Notes:
- `auto_keys` in `GameFiles/poe-hook.toml` is `[]` after every deploy (nothing is pressed for you); the unattended tests set it per run with `POE_HOOK_AUTO_KEYS` ([Unattended client tests](#unattended-client-tests)).
- Keep `dump_object_types = true`: the spawn position relies on the tile layout and cell grid the DLL exports to `dumps/<version>/terrain/{layouts,grids}/` (the server reads them through `[spawn] layout_dir`, or receives them as an upload from a remote client, [hosting.md](hosting.md); it has no collision of its own yet).
- Admin commands (`!help`, the overlay's Commands and Instances pages) need an account with the admin role. On the dev machine-hash login the first account `Exile1` is an admin; otherwise `cargo run -p poe-server -- account create <name> <password>` then `account role <name> admin`; see [server-admin.md](server-admin.md).
- Movement: WASD works in the default input mode. For click-to-move, pick the mouse input method in the game's options (in the config file: `user_input_mode=mouse_and_keyboard`). Space is the dodge roll in both modes.
- After rebuilding the DLL, run `cargo xtask deploy-client` again; the shortcut needs no changes.

## Official servers (read-only)
1. `cargo xtask deploy-client --official`: the original gateway list goes back into `GameFiles/`, and `poe-hook.toml` gets `mode = "official"`, `login_mode = "steam"` (the client is not patched; the Steam login screen shows) and `trusted_servers = []`. The GameFiles copy must be the same build as the live game; after a patch, update GameFiles first.
2. Start Steam, then `cargo xtask run-client` (the GameFiles copy in its own Proton prefix; the Steam install is never touched).
3. The user plays. The overlay shows `OFFICIAL: read-only`; packets, logs and snapshots go to the session folder (see [Logs](#logs)). What the DLL does and does not do in this mode: [../architecture/client-dll.md](../architecture/client-dll.md#modes-and-the-read-only-latch).
4. `cargo xtask deploy-client` switches back to our server.

Comparing a session with one from our server: [../re/captures.md](../re/captures.md).

## Unattended client tests
With Steam running (without it the client logs `Unable to init Steam API` and exits with code 1) and the server writing to `runtime/server-run.log` (`target/debug/poe-server > runtime/server-run.log 2>&1`):
- `scripts/test-run.sh <name> [seconds]` launches the client, logs in and enters the game by itself (`auto_keys`). It first deletes `dumps/<version>/terrain/layouts/*` and `grids/*` so the run shows whether the DLL exports them again (the server waits for the layout by tile hash). It sets these environment variables for the client:
  - `POE_HOOK_AUTO_KEYS=20000:13` (Enter at 20 s, the login; `[[ms, vk], ...]` in `auto_keys` form);
  - `POE_HOOK_MOVEMENT_LOG=1` (per-frame movement log, `movement_log` in the config);
  - `POE_HOOK_FAKE_FOCUS=1` (the hooked `GetForegroundWindow` returns the game's own window, `*[0x1445d8308]`, so a client on another desktop still sends cursor reports; never while read-only);
  - `POE_HOOK_WINDOW_TITLE="PoE2 Test Client"` (the hooks on `CreateWindowExW`/`SetWindowTextW` give the game's main window (class `POEWindowClass`; message boxes and other windows keep their titles) that title, so a desktop rule can match it).
- `scripts/agent-desktop.sh on|off|status` (KDE Plasma) creates a virtual desktop "Agents" and adds KWin rules that force the game client (window class `steam_app_2694490`), the launcher (`poe2-launcher`) and a test browser (class `poe-agent-browser`) onto it and never let them take focus (otherwise a window that activates itself makes KWin switch the view to its desktop). While the rules are on you cannot type into those windows; tests drive the client through `poe-hook.keys`. Start a visible test browser with `firefox --new-instance --name poe-agent-browser --class poe-agent-browser --profile runtime/agent-browser-profile <url>`. The game rule also matches the official game started from Steam: run `off` before playing it.
- `scripts/re/send_dev.py <command>` sends a server command over the debug link (like the overlay). The link is authenticated per session, so the script queues a `cmd:<command>` entry in `GameFiles/poe-hook.keys` and the running client DLL sends it.
- `scripts/shot.sh <name>` takes a screenshot of the game monitor (Spectacle) into `runtime/shots/`; F8 below takes one inside the game.
- `echo <entry> > GameFiles/poe-hook.keys` presses keys and moves the mouse in the client; `0x2d` is Insert (the overlay), `0x0d` Enter.

### `poe-hook.keys` entries
Each entry is one line; the DLL reads the file and posts the input to the game window only (nothing goes to other windows; `SendInput` is not used, because it types into whatever window has focus). Never while read-only.
- A plain virtual-key number (`0x0d`, `0x57:800` with a hold time in ms) is a posted key message. Plain keys (`0x20`, Space = dodge roll) reach the input bindings that are read from window messages. A plain `0x20` is down and up in one go, so the client's key state never reads "held" and a roll started that way is not steered (no `0x1a8`); for roll and sprint tests use `indown:0x20`, `0x20`, `wait:250`, `inup:0x20` ([movement spec](../protocol/movement.md#test-tooling)).
- **`in:<vk>[:ms]`** posts `WM_INPUT` with a marker handle. The game reads WASD through Raw Input, so posted key messages do not move the player; the DLL hooks `GetRawInputData` (user32) to return a synthetic keyboard record for that handle, and `GetKeyState` reports the key as held meanwhile.
- **`indown:<vk>` / `inup:<vk>` / `wait:<ms>`** hold and release a key while other input arrives.
- **`inmove:x,y` / `inclick:x,y`** make the hooked `GetCursorPos` return that client position, then post the mouse messages.
- **`inpress:x,y` / `inrelease:x,y`** hold and release the left button at a client position (held click-to-move); the hooked `GetKeyState` reports `VK_LBUTTON` held meanwhile.
- **`text:<chars>`** posts one `WM_CHAR` per character, for text boxes such as the character name.
- **`POE_HOOK_KEYS_FILE=<name or absolute path>`** (environment variable of the client process) makes that DLL read its input entries from another file than `GameFiles/poe-hook.keys`, so two clients on one GameFiles get one file each (`poe-hook.1.keys`, `poe-hook.2.keys` in `scripts/test-mp.sh`). The DLL logs `input file: ...` at start.
- **`0x77` (F8)** takes a game screenshot into the test prefix's `My Games/Path of Exile 2/Screenshots`, which shows the game even when its window is not visible.
- **`close`** posts `WM_CLOSE` to the game window (like its X button).
- Before each batch the DLL posts `WM_ACTIVATEAPP(1)`: the game ignores input while it thinks it is in the background, and this changes only its own flag, not the real focus.
- Click-to-move needs `user_input_mode=mouse_and_keyboard` in the test prefix's `poe2_production_Config.ini` (edit it only while the client is closed: the client rewrites it on exit).

## Logs
- Client: `GameFiles/logs/Client.txt`.
- DLL: `GameFiles/poe-hook.log`, plus a per-process session folder `GameFiles/logs/poe-hook/<date_time>_<pid>/` (logs, `packets.jsonl`, `packets.txt`, snapshots) and the in-game overlay (Insert). What the DLL logs and how: [../architecture/client-dll.md](../architecture/client-dll.md).
- Generated data (terrain grids and layouts, area exports, client tables) goes to `dumps/<version>/` instead; `scripts/dumps/migrate.sh` sorts older files there and copies packet sessions to `dumps/<version>/captures/{official,local}/`.
- Server: stdout, filtered by `RUST_LOG` or `[logging] filter`. Admin commands are also appended to `[commands] audit_log`. For unattended runs it is `runtime/server-run.log`; the movement trace is `runtime/movement-trace.tsv`.
- Real play on official servers: the user's `Client.txt` in the Steam install (read only, see [../../CLAUDE.md](../../CLAUDE.md)); analysis in [../re/captures.md](../re/captures.md).

## Crash reports
After a crash the client writes `PoE-<time>-<pid>.dmp` (plus `.txt`, `.log.txt` and `.poecap` side files) under `My Games/Path of Exile 2/poe2_production` in the prefix. A helper process uploads them, both right after the crash and on the next launch. With `block_external` installed from `DllMain`, the upload connections are refused. `cargo xtask run-client` also moves pending dumps to `runtime/crash-dumps/` (git-ignored) before launching, so nothing is left to upload and the dumps stay available for debugging.
