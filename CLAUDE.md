# CLAUDE.md

A private server for **Path of Exile 2, client 0.5.5e**, written in Rust and built for future client and server modding (personal and educational use).
Main repo = server crates and dev tooling; submodules under `modules/` hold the shared protocol (Poe2Protocol) and the client hook (Poe2ClientHook).
Progress and the current task: the tracker in [README.md](README.md).

## Hard rules
- **Never touch the Steam install** (`~/.local/share/Steam/steamapps/common/Path of Exile 2`) or its Proton prefix (`compatdata/2694490`). Use only `GameFiles/`, the provided copy.
- `GameFiles/` changes only through `cargo xtask deploy-client` / `revert-client` / `deploy-core` (they back files up first). Never hand-edit it.
- **Official servers: read-only.** The DLL may observe and log official sessions the user plays (GameFiles copy, `deploy-client --official`); it never sends, alters or injects anything there (read-only latch). Claude never logs in to official servers; captures are the user's.
- The user's live `Client.txt` in the Steam install is a read-only reference: never modify it, never commit its contents (copy excerpts into `runtime/`).
- No hardcoding: opcodes in `modules/protocol/opcodes/<version>/*.toml`; ports, leagues, start areas in `config/server.toml`; classes, areas, items from game data (`poe-data`); subsystems behind traits.
- External tools go in `third_party/` (`third_party/fetch-tools.sh`); generated/datamined data in git-ignored `dumps/<version>/`; local runtime files in `runtime/`.
- Docs go in `docs/` and are updated in the same commit as the code; RE findings and addresses go in `docs/re/`. Run `python3 scripts/check-links.py` after doc changes.
- Commit and push after each working step (`origin main`). Submodules: commit and push inside the submodule first, then bump the pointer in the main repo.
- Ask the user for x64dbg/dynamic debugging, for official captures, and before relying on Steam being running.

## Roadmap (details: [docs/plans/orchestration-2026-10.md](docs/plans/orchestration-2026-10.md), status: README tracker)
1. Foundation: restructure, dumps, repo split, docs, config, tracker
2. Server-authoritative movement from official captures
3. Accounts (per cluster, passwords) and multiplayer
4. Web API + web admin
5. Launcher (Tauri; installs 0.5.5e from Steam depots, mods)
6. Consistency pass; 7. terrain port to all areas (handoff plan)

## Where things are
- Commands, logs, unattended in-game tests: [docs/guides/setup.md](docs/guides/setup.md)
- Server commands and admins: [docs/guides/server-admin.md](docs/guides/server-admin.md)
- Architecture and crates: [docs/architecture/](docs/architecture/overview.md)
- Protocol: [docs/protocol/](docs/protocol/framing.md); RE how-to (Ghidra MCP, `scripts/re/`): [docs/re/README.md](docs/re/README.md)
- Terrain status: [docs/status/terrain.md](docs/status/terrain.md); history: [docs/status/history.md](docs/status/history.md)
- Everything else: [docs/index.md](docs/index.md)
