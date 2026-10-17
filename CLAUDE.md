# CLAUDE.md

A private server for **Path of Exile 2, client 0.5.5e**, written in Rust and designed for future client and server modding. Personal and educational use.

## Hard rules
- **Never touch the Steam install** at `~/.local/share/Steam/steamapps/common/Path of Exile 2` or its Proton prefix (`compatdata/2694490`). Use only `GameFiles/`, the provided copy.
- `GameFiles/` may be modified, but only through `cargo xtask deploy-client` / `revert-client`, which back files up first. Never hand-edit it.
- **Official servers: read-only only** (rule changed by the user 2026-10-14). The debug DLL may observe and log official sessions, and nothing else: no sending, altering or injecting anything while connected to a non-local server (the DLL's read-only latch enforces this). Use the GameFiles copy (`cargo xtask deploy-client --official`), never the Steam install. The user plays official sessions; Claude does not log in to official servers on its own. Otherwise reverse engineering is static (Ghidra), plus the client against *our* server.
- All external tools go in `Tools/` (see `Tools/README.md`; `Tools/fetch-tools.sh` reproduces them). Reference repos are in `Tools/reference/`.
- Documentation goes in `Documentation/` (start at `Documentation/index.md`). Update it in the same commit as the code it describes. RE findings and addresses go in `Documentation/re/`.
- Commit and push (`origin main`, private repo `Distorbed678/Poe2ModdingProject`) after each working step.
- Avoid hardcoding:
  - Opcodes go in `protocol/<version>/{login,instance,hnc}.toml`.
  - Ports, leagues and start areas go in `config/server.toml`.
  - Classes, areas and items come from game data (`poe-data`).
  - Subsystems sit behind traits so implementations can be swapped.

## Reverse engineering access
- **Ghidra MCP** (`mcp__ghidra__*`). Call `connect_instance("Poe2ModdingProject")` first; `/PathOfExileSteam.exe` (image base 0x140000000) is open and analyzed.
  - Use targeted calls (`search_strings`, `get_xrefs_to`, `decompile_function`).
  - Huge functions (packet factories) give 20k+ token decompiles, so prefer `disassemble_function` on ranges or search by constant.
  - Rename functions and add comments in Ghidra as understanding grows, and record stable findings in `Documentation/re/addresses-0.5.5e.md`.
- **x64dbg / dynamic debugging**: not installed. **Ask the user** when it would help, for example runtime values such as keys, crash triage, or verifying a hook.
- The client DLL can log plaintext packets (trace feature) for analyzing our own sessions.

## Layout
- `crates/`: libraries.
  - `poe-config`: TOML config.
  - `poe-net`: TCP, byte codec, cipher trait, traces.
  - `poe-proto`: packet definitions, with opcodes from `protocol/`.
  - `poe-data`: game data from Bundles2 plus dat-schema.
  - `poe-store`: persistence (SQLite).
  - `poe-cluster`: tickets, login sessions (logout, transfers), instance directory.
  - `poe-world`: terrain generation (port of the client's generator).
  - `poe-game`: simulation (movement, stats).
  - Planned: `poe-mod-api` (mod hooks).
- `servers/`: `login-server`, `instance-server`, and `poe-server` (a dev launcher that runs both).
- `client/`: the client DLLs, built for `x86_64-pc-windows-msvc` with cargo-xwin; **excluded from the workspace** (own Cargo workspace). `poe-hook` = `version.dll` loader (permanent hooks), `poe-hook-core` = hot-reloadable debug core and overlay, `poe-hook-shared` = config and ABI.
- `tools/xtask`: dev automation (`cargo xtask <cmd>`).
- `config/`, `protocol/`, `Documentation/`, `Tools/`.

## Commands
- `cargo build` / `cargo test`. The toolchain is pinned to `nightly-2026-08-10` because `poe_data_tools` needs nightly.
- `cargo run -p poe-server` runs the login and instance servers using `config/server.toml`.
- `cargo xtask build-dll` builds the client DLLs; `cargo xtask deploy-core` installs only the core, which a running client hot-reloads.
- `cargo xtask deploy-client` / `cargo xtask revert-client` install and remove the gateway entry, DLL and config in `GameFiles/`.
- `cargo xtask run-client` launches `GameFiles/PathOfExileSteam.exe` through Proton with its own prefix (`Tools/proton-prefix`). Steam must be running.
- `cargo xtask dll-data` rewrites only the DLL's name tables in `GameFiles/poe-hook-data/` (stats, action types, areas).
- Unattended in-game tests (Steam running): start the server writing to `data/server-run.log`, then `tools/test-run.sh <name> [seconds]` logs in and enters the game; `tools/re/send_dev.py auto <command>` runs a server command; `echo 0x2d > GameFiles/poe-hook.keys` presses Insert; `tools/shot.sh <name>` screenshots. See Documentation/setup.md.
- Server commands and admins: Documentation/server-admin.md (`[admin] accounts`, chat prefix `!`).
- `Tools/poe_data_tools/poe_data_tools -p 2 --steam GameFiles list '<glob>'` lists and extracts game files.

## Reference data from live play
- `~/.local/share/Steam/steamapps/common/Path of Exile 2/logs/Client.txt` is the user's client log from **real play sessions on official servers** (about 80 MB). It shows what a correct session looks like: area/seed pairs, tile and doodad hashes, loading timings, the order of client events, and warnings. Use it as a reference.
- **Read only.** Never modify, move or truncate it, or anything else in the Steam install. Copy excerpts into `data/` (git-ignored) if needed. Do not commit its contents, since it may hold account details.
- Analysis scripts: `tools/re/clientlog_*.py`.

## Logs
- Client: `GameFiles/logs/Client.txt`.
- DLL: `GameFiles/poe-hook.log`, plus a per-process session folder `GameFiles/logs/poe-hook/<date_time>_<pid>/` (logs, packets, snapshots) and the in-game overlay (Insert).
- Server: stdout, with the level set by `RUST_LOG` or `config/server.toml`.
