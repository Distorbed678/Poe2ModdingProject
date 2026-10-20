# CLAUDE.md

A private server for **Path of Exile 2, client 0.5.5e**, written in Rust and designed for future client and server modding. Personal and educational use.

## Hard rules
- **Never touch the Steam install** at `~/.local/share/Steam/steamapps/common/Path of Exile 2` or its Proton prefix (`compatdata/2694490`). Use only `GameFiles/`, the provided copy.
- `GameFiles/` may be modified, but only through `cargo xtask deploy-client` / `revert-client`, which back files up first. Never hand-edit it.
- **Official servers: read-only only** (rule changed by the user 2026-10-14). The debug DLL may observe and log official sessions, and nothing else: no sending, altering or injecting anything while connected to a non-local server (the DLL's read-only latch enforces this). Use the GameFiles copy (`cargo xtask deploy-client --official`), never the Steam install. The user plays official sessions; Claude does not log in to official servers on its own. Otherwise reverse engineering is static (Ghidra), plus the client against *our* server.
- All external tools go in `third_party/` (see `third_party/README.md`; `third_party/fetch-tools.sh` reproduces them). Reference repos are in `third_party/reference/`.
- Documentation goes in `docs/` (start at `docs/index.md`). Update it in the same commit as the code it describes. RE findings and addresses go in `docs/re/`.
- Commit and push (`origin main`, private repo `Distorbed678/Poe2ModdingProject`) after each working step.
- Avoid hardcoding:
  - Opcodes go in `modules/protocol/opcodes/<version>/{login,instance,hnc}.toml` (Poe2Protocol repo).
  - Ports, leagues and start areas go in `config/server.toml`.
  - Classes, areas and items come from game data (`poe-data`).
  - Subsystems sit behind traits so implementations can be swapped.

## Reverse engineering access
- **Ghidra MCP** (`mcp__ghidra__*`). Call `connect_instance("Poe2ModdingProject")` first; `/PathOfExileSteam.exe` (image base 0x140000000) is open and analyzed.
  - Use targeted calls (`search_strings`, `get_xrefs_to`, `decompile_function`).
  - Huge functions (packet factories) give 20k+ token decompiles, so prefer `disassemble_function` on ranges or search by constant.
  - Rename functions and add comments in Ghidra as understanding grows, and record stable findings in `docs/re/addresses-0.5.5e.md`.
- **x64dbg / dynamic debugging**: not installed. **Ask the user** when it would help, for example runtime values such as keys, crash triage, or verifying a hook.
- The client DLL can log plaintext packets (trace feature) for analyzing our own sessions.

## Layout
- `crates/`: libraries.
  - `poe-config`: TOML config.
  - `poe-data`: game data from Bundles2 plus dat-schema.
  - `poe-store`: persistence (SQLite).
  - `poe-cluster`: tickets, login sessions (logout, transfers), instance directory.
  - `poe-world`: terrain generation (port of the client's generator).
  - `poe-game`: simulation (movement, stats).
  - Planned: `poe-mod-api` (mod hooks).
- `servers/`: `login-server`, `instance-server`, and `poe-server` (a dev launcher that runs both).
- `modules/`: git submodules (private repos under `Distorbed678`); **excluded from the workspace**, each its own Cargo workspace, used as path dependencies. Clone with `--recursive`; commit and push inside a submodule first, then bump the pointer here.
  - `modules/protocol` (Poe2Protocol): `poe-proto` (packet definitions; opcode tables as constants `poe_proto::opcodes::v0_5_5e::*`), `poe-net` (byte codec, cipher trait, hexdump; TCP `Connection` behind the `tokio` feature, enabled by the servers), `opcodes/<version>/*.toml`.
  - `modules/client-hook` (Poe2ClientHook): the client DLLs, built for `x86_64-pc-windows-msvc` with cargo-xwin: `poe-hook` = `version.dll` loader (permanent hooks), `poe-hook-core` = hot-reloadable debug core and overlay, `poe-hook-shared` = config and ABI. `poe-hook-deploy` = host-side deploy/revert/launch library used by xtask. Has `modules/client-hook/protocol` as a nested submodule. Mod manifest spec: `modules/client-hook/docs/mod-manifest.md`.
- `xtask/`: dev automation (`cargo xtask <cmd>`).
- `scripts/`: shell and Python helpers (`test-run.sh`, `shot.sh`, `re/*.py`).
- `third_party/`: downloaded tools and reference repos (git-ignored except README and fetch script).
- `dumps/<version>/`: datamined and generated outputs (terrain grids/layouts, area exports, vectors, client tables, Ghidra exports, packet captures, strings; git-ignored, see `dumps/README.md`; `scripts/dumps/migrate.sh` sorts old files in). The client DLL writes there via `dump_dir`.
- `runtime/`: server and test output (sqlite, logs, traces, crash dumps; git-ignored).
- `config/`, `docs/`.

## Commands
- `cargo build` / `cargo test`. The toolchain is pinned to `nightly-2026-08-10` because `poe_data_tools` needs nightly.
- `cargo run -p poe-server` runs the login and instance servers using `config/server.toml`.
- `cargo xtask build-dll` builds the client DLLs; `cargo xtask deploy-core` installs only the core, which a running client hot-reloads.
- `cargo xtask deploy-client` / `cargo xtask revert-client` install and remove the gateway entry, DLL and config in `GameFiles/`.
- `cargo xtask run-client` launches `GameFiles/PathOfExileSteam.exe` through Proton with its own prefix (`third_party/proton-prefix`). Steam must be running.
- `cargo xtask dll-data` rewrites only the DLL's name tables in `GameFiles/poe-hook-data/` (stats, action types, areas).
- Unattended in-game tests (Steam running): start the server writing to `runtime/server-run.log`, then `scripts/test-run.sh <name> [seconds]` logs in and enters the game; `scripts/re/send_dev.py auto <command>` runs a server command; `echo 0x2d > GameFiles/poe-hook.keys` presses Insert; `scripts/shot.sh <name>` screenshots. See docs/setup.md.
- Server commands and admins: docs/server-admin.md (`[admin] accounts`, chat prefix `!`).
- `third_party/poe_data_tools/poe_data_tools -p 2 --steam GameFiles list '<glob>'` lists and extracts game files.

## Reference data from live play
- `~/.local/share/Steam/steamapps/common/Path of Exile 2/logs/Client.txt` is the user's client log from **real play sessions on official servers** (about 80 MB). It shows what a correct session looks like: area/seed pairs, tile and doodad hashes, loading timings, the order of client events, and warnings. Use it as a reference.
- **Read only.** Never modify, move or truncate it, or anything else in the Steam install. Copy excerpts into `runtime/` (git-ignored) if needed. Do not commit its contents, since it may hold account details.
- Analysis scripts: `scripts/re/clientlog_*.py`.

## Logs
- Client: `GameFiles/logs/Client.txt`.
- DLL: `GameFiles/poe-hook.log`, plus a per-process session folder `GameFiles/logs/poe-hook/<date_time>_<pid>/` (logs, packets, snapshots) and the in-game overlay (Insert).
- Server: stdout, with the level set by `RUST_LOG` or `config/server.toml`.
