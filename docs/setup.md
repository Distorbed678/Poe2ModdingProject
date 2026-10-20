# Setup

## Requirements
- Linux with rustup. The toolchain is pinned by `rust-toolchain.toml` to `nightly-2026-08-10`, because `poe_data_tools` needs nightly. Targets: `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`.
- clang and lld (`clang-cl`, `lld-link`), used by cargo-xwin to link the Windows DLL.
- `gh` and `git`, for the tool downloads.
- Steam with Proton Experimental, to run the client.
- `GameFiles/`: a copy of the 0.5.5e client install (not in git). The Steam install itself is never used or modified.

## Tools
Run `third_party/fetch-tools.sh` to download every external tool into `third_party/` (listed in [third_party/README.md](../third_party/README.md)): poe_data_tools CLI, dat-schema, LibGGPK3, cargo-xwin, the Python venv `pyenv` for `scripts/re`, reference repos and the pinned toolchain.

## Build and test
```sh
cargo build                 # server workspace
cargo test                  # unit tests; poe-data tests read GameFiles if present
cargo xtask build-dll       # client/ -> version.dll (loader) + poe_hook_core.dll (x86_64-pc-windows-msvc)
```

## Run
```sh
cargo run -p poe-server     # login (20481) + instance (6112) servers, config/server.toml
cargo xtask deploy-client   # builds the DLLs and installs them into GameFiles/ (--official: read-only, official servers)
cargo xtask deploy-core     # rebuilds and installs only the debug core; a running client reloads it
cargo xtask dll-data        # rewrites only GameFiles/poe-hook-data/ (stats, action types, areas)
cargo xtask run-client      # launches the client through Proton (Steam must be running)
cargo xtask revert-client   # restores GameFiles/ to the original state
```

`deploy-client` does the following:
- Backs up `GameFiles/gateway_list.txt` to `gateway_list.txt.orig` (only once).
- Writes a single UTF-16LE gateway entry, built from `[gateway]`/`[login]` in `config/server.toml`.
- Copies the loader to `GameFiles/version.dll` and the debug core next to it.
- Writes `dump_dir` (`<repo>/dumps/<version>` as a `Z:\` path) into `GameFiles/poe-hook.toml`; the DLL writes terrain grids and layouts, area exports and client tables there ([../dumps/README.md](../dumps/README.md)).
- Writes the DLL's name tables to `GameFiles/poe-hook-data/` (`stats.tsv`, `action_types.tsv`, `areas.tsv` for the area picker).
- Copies `client/poe-hook/poe-hook.toml`, unless a copy already exists (so your local edits survive).
- Sets `position_sync_port` in that file's `[features]` to `[instance] public_port`, so position reports go to the instance server.
- Writes `steam_appid.txt`.

`run-client` uses its own Proton prefix (`third_party/proton-prefix`) with `WINEDLLOVERRIDES=version=n,b`.

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
- Admin commands (`!help`, the overlay's Commands and Instances pages) need your account name in `[admin] accounts` in `config/server.toml` (accounts are named `Exile<id>`).
- Movement: WASD works in the default input mode. For click-to-move, pick the mouse input method in the game's options (in the config file: `user_input_mode=mouse_and_keyboard`). Space is the dodge roll in both modes.
- After rebuilding the DLL, run `cargo xtask deploy-client` again; the shortcut needs no changes.

## Logs
- Client: `GameFiles/logs/Client.txt`.
- DLL: `GameFiles/poe-hook.log`, plus a per-process session folder `GameFiles/logs/poe-hook/<date_time>_<pid>/` (logs, `packets.jsonl`, snapshots). Generated data (terrain grids and layouts, area exports, client tables) goes to `dumps/<version>/` instead; `scripts/dumps/migrate.sh` sorts older files there and copies packet sessions to `dumps/<version>/captures/{official,local}/`.
- Server: stdout, filtered by `RUST_LOG` or `[logging] filter`. Admin commands are also appended to `[commands] audit_log`.

## Unattended client tests
With Steam running and the server writing to `runtime/server-run.log` (`target/debug/poe-server > runtime/server-run.log 2>&1`):
- `scripts/test-run.sh <name> [seconds]` launches the client, logs in and enters the game by itself (`auto_keys`). It first deletes `dumps/<version>/terrain/layouts/*` and `grids/*` so the run shows whether the DLL exports them again (the server waits for the layout by tile hash).
- `scripts/re/send_dev.py auto <command>` sends a server command over the debug link (like the overlay).
- `echo 0x2d > GameFiles/poe-hook.keys` presses a key in the client (0x2d = Insert, the overlay); `scripts/shot.sh <name>` takes a screenshot.
