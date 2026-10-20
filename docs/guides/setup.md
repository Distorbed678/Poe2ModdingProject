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
The repo uses git submodules (`modules/protocol`, `modules/client-hook` with its nested `protocol`; private repos, `gh auth login` first):
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
cargo run -p poe-server     # login (20481) + instance (6112) servers, config/server.toml (add --release for play)
cargo xtask deploy-client   # builds the DLLs and installs them into GameFiles/ (--official: read-only, official servers; --no-build)
cargo xtask deploy-core     # rebuilds and installs only the debug core; a running client reloads it within about a second (--no-build)
cargo xtask dll-data        # rewrites only GameFiles/poe-hook-data/ (stats, action types, areas)
cargo xtask run-client      # launches the client through Proton (Steam must be running)
cargo xtask revert-client   # restores GameFiles/ to the original state
```

`deploy-client` does the following (the logic is `poe-hook-deploy` in `modules/client-hook`; xtask passes the paths):
- Backs up `GameFiles/gateway_list.txt` to `gateway_list.txt.orig` (only once).
- Writes a single UTF-16LE gateway entry, built from `[gateway]`/`[login]` in `config/server.toml`.
- Copies the loader to `GameFiles/version.dll` and the debug core next to it.
- Writes `dump_dir` (`<repo>/dumps/<version>` as a `Z:\` path) into `GameFiles/poe-hook.toml`; the DLL writes terrain grids and layouts, area exports and client tables there ([../../dumps/README.md](../../dumps/README.md)).
- Writes the DLL's name tables to `GameFiles/poe-hook-data/` (`stats.tsv`, `action_types.tsv`, `areas.tsv` for the area picker).
- Copies `modules/client-hook/poe-hook/poe-hook.toml`, unless a copy already exists (so your local edits survive).
- Sets `position_sync_port` in that file's `[features]` to `[instance] public_port`, so position reports go to the instance server.
- Resets `auto_keys` to `[]`, and the client's remembered gateway (`gateway_id` in the test prefix's `poe2_production_Config.ini`) to ours for private deploys (after an official session it stays on an official realm and the client never contacts our server).
- Writes `steam_appid.txt`.

`run-client` uses its own Proton prefix (`third_party/proton-prefix`) with `WINEDLLOVERRIDES=version=n,b`, clears `GameFiles/poe-hook.log` per launch, and moves pending crash dumps away (see [Crash reports](#crash-reports)). `build-dll` and `deploy-client` are thin wrappers over `poe-hook-deploy` ([../architecture/client-dll.md](../architecture/client-dll.md)).

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
2. Launch the shortcut from Steam. The login screen shows "Local Server"; log in and play.

Notes:
- The DLL config `GameFiles/poe-hook.toml` presses Enter 20 s after start (`auto_keys`, used by unattended tests). It just logs in; set `auto_keys = []` to turn it off.
- Keep `dump_object_types = true`: the spawn position relies on the tile layout and cell grid the DLL exports to `dumps/<version>/terrain/{layouts,grids}/` (the server reads them through `[spawn] layout_dir`; it has no collision of its own yet).
- Admin commands (`!help`, the overlay's Commands and Instances pages) need your account name in `[admin] accounts` in `config/server.toml` (accounts are named `Exile<id>`); see [server-admin.md](server-admin.md).
- Movement: WASD works in the default input mode. For click-to-move, pick the mouse input method in the game's options (in the config file: `user_input_mode=mouse_and_keyboard`). Space is the dodge roll in both modes.
- After rebuilding the DLL, run `cargo xtask deploy-client` again; the shortcut needs no changes.

## Official servers (read-only)
1. `cargo xtask deploy-client --official`: the original gateway list goes back into `GameFiles/`, and `poe-hook.toml` gets `mode = "official"`. The GameFiles copy must be the same build as the live game; after a patch, update GameFiles first.
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
  - `POE_HOOK_WINDOW_TITLE="PoE2 Test Client"` (the hooks on `CreateWindowExW`/`SetWindowTextW` give the game window that title, so a desktop rule can match it, for example a KWin rule that moves the window to another virtual desktop without focus; no rule is installed by the project).
- `scripts/re/send_dev.py auto <command>` sends a server command over the debug link (like the overlay).
- `scripts/shot.sh <name>` takes a screenshot of the game monitor (Spectacle) into `runtime/shots/`; F8 below takes one inside the game.
- `echo <entry> > GameFiles/poe-hook.keys` presses keys and moves the mouse in the client; `0x2d` is Insert (the overlay), `0x0d` Enter.

### `poe-hook.keys` entries
Each entry is one line; the DLL reads the file and posts the input to the game window only (nothing goes to other windows; `SendInput` is not used, because it types into whatever window has focus). Never while read-only.
- A plain virtual-key number (`0x0d`, `0x57:800` with a hold time in ms) is a posted key message. Plain keys (`0x20`, Space = dodge roll) reach the input bindings that are read from window messages.
- **`in:<vk>[:ms]`** posts `WM_INPUT` with a marker handle. The game reads WASD through Raw Input, so posted key messages do not move the player; the DLL hooks `GetRawInputData` (user32) to return a synthetic keyboard record for that handle, and `GetKeyState` reports the key as held meanwhile.
- **`indown:<vk>` / `inup:<vk>` / `wait:<ms>`** hold and release a key while other input arrives.
- **`inmove:x,y` / `inclick:x,y`** make the hooked `GetCursorPos` return that client position, then post the mouse messages.
- **`inpress:x,y` / `inrelease:x,y`** hold and release the left button at a client position (held click-to-move); the hooked `GetKeyState` reports `VK_LBUTTON` held meanwhile.
- **`text:<chars>`** posts one `WM_CHAR` per character, for text boxes such as the character name.
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
