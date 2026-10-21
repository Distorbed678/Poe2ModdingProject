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
- Ask the user for x64dbg/dynamic debugging and for official captures. Steam may be used and started for private-server tests without asking; never log in to official servers.

## Roadmap (details: [docs/plans/orchestration-2026-10.md](docs/plans/orchestration-2026-10.md), status: README tracker)
Done (2026-10-20/21): foundation (restructure, dumps, repo split, docs, config, tracker); server-authoritative movement from official captures; accounts with the standalone login; remote hosting; shared instance world with two-client multiplayer; web API + web admin; launcher (end-to-end test with a depot download pending); consistency pass.
**Current task:** terrain port to all areas, following [docs/plans/terrain-all-areas.md](docs/plans/terrain-all-areas.md).
Open elsewhere: movement gaps #11/#12 (docs/protocol/movement.md), party/trade replies, the remote player tint (WP C2 bisect pending).

## Where things are
- Commands, logs, unattended in-game tests: [docs/guides/setup.md](docs/guides/setup.md)
- Server commands and admins: [docs/guides/server-admin.md](docs/guides/server-admin.md)
- Architecture and crates: [docs/architecture/](docs/architecture/overview.md)
- Protocol: [docs/protocol/](docs/protocol/framing.md); RE how-to (Ghidra MCP, `scripts/re/`): [docs/re/README.md](docs/re/README.md)
- Terrain status: [docs/status/terrain.md](docs/status/terrain.md); history: [docs/status/history.md](docs/status/history.md)
- Everything else: [docs/index.md](docs/index.md)
