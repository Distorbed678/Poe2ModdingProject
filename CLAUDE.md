# CLAUDE.md

A private server for **Path of Exile 2, client 0.5.5e**, written in Rust and designed for future client and server modding. Personal and educational use.

## Hard rules
- **Never touch the Steam install** at `~/.local/share/Steam/steamapps/common/Path of Exile 2` or its Proton prefix (`compatdata/2694490`). Use only `GameFiles/`, the provided copy.
- `GameFiles/` may be modified, but only through `cargo xtask deploy-client` / `revert-client`, which back files up first. Never hand-edit it.
- **No traffic to official servers** for research. Reverse engineering is static (Ghidra), plus the client against *our* server.
- All external tools go in `Tools/` (see `Tools/README.md`; `Tools/fetch-tools.sh` reproduces them). Reference repos are in `Tools/reference/`.
- Documentation goes in `Documentation/` (start at `Documentation/index.md`). Update it in the same commit as the code it describes. RE findings and addresses go in `Documentation/re/`.
- Commit and push (`origin main`, private repo `Distorbed678/Poe2ModdingProject`) after each working step.
- Avoid hardcoding:
  - Opcodes go in `protocol/<version>/opcodes.toml`.
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
  - `poe-world`: areas and terrain.
  - `poe-game`: simulation (ECS).
  - `poe-mod-api`: mod hooks.
- `servers/`: `login-server`, `instance-server`, and `poe-server` (a dev launcher that runs both).
- `client/poe-hook`: `version.dll` proxy DLL, built for `x86_64-pc-windows-msvc` with cargo-xwin. It is **excluded from the workspace** and has its own Cargo project.
- `tools/xtask`: dev automation (`cargo xtask <cmd>`).
- `config/`, `protocol/`, `Documentation/`, `Tools/`.

## Commands
- `cargo build` / `cargo test`. The toolchain is pinned to `nightly-2026-08-10` because `poe_data_tools` needs nightly.
- `cargo run -p poe-server` runs the login and instance servers using `config/server.toml`.
- `cargo xtask build-dll` builds `client/poe-hook` into `version.dll`.
- `cargo xtask deploy-client` / `cargo xtask revert-client` install and remove the gateway entry, DLL and config in `GameFiles/`.
- `cargo xtask run-client` launches `GameFiles/PathOfExileSteam.exe` through Proton with its own prefix (`Tools/proton-prefix`). Steam must be running.
- `Tools/poe_data_tools/poe_data_tools -p 2 --steam GameFiles list '<glob>'` lists and extracts game files.

## Logs
- Client: `GameFiles/logs/Client.txt`.
- DLL: `GameFiles/poe-hook.log`.
- Server: stdout, with the level set by `RUST_LOG` or `config/server.toml`.
