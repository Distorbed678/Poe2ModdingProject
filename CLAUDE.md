# CLAUDE.md

A private server for **Path of Exile 2, client 0.5.5e**, written in Rust and built for future client and server modding (personal and educational use).
This repo is the **hub** (docs, plans, RE notes, scripts, `xtask` dev tooling, the tracker); the code is in submodules under `modules/`: the server (`modules/server`, Poe2Server), the shared protocol (`modules/protocol`, Poe2Protocol: opcodes and the version files), the client hook, web admin, launcher, mod SDK, mod index and server list. Which repo does what: [docs/overview.md](docs/overview.md).
Progress and the current task: the tracker in [README.md](README.md).

## Hard rules
- **Never touch the Steam install** (`~/.local/share/Steam/steamapps/common/Path of Exile 2`) or its Proton prefix (`compatdata/2694490`). Use only `GameFiles/`, the provided copy.
- `GameFiles/` changes only through `cargo xtask deploy-client` / `revert-client` / `deploy-core` (they back files up first). Never hand-edit it.
- **Official servers: read-only.** The DLL may observe and log official sessions the user plays (GameFiles copy with `deploy-client --official`, or the launcher's "Official (read-only)" version entry, which deploys the same config); it never sends, alters or injects anything there (read-only latch). Claude never logs in to official servers; captures are the user's.
- The user's live `Client.txt` in the Steam install is a read-only reference: never modify it, never commit its contents (copy excerpts into `runtime/`).
- No hardcoding: opcodes in `modules/protocol/opcodes/<version>/*.toml`; ports, leagues, start areas in `modules/server/config/server.toml`; supported client builds in `modules/protocol/versions/<version>.toml`; classes, areas, items from game data (`poe-data`); subsystems behind traits.
- External tools go in `third_party/` (`third_party/fetch-tools.sh`); generated/datamined data in git-ignored `dumps/<version>/`; local runtime files in `runtime/`.
- Docs go in `docs/` and are updated in the same commit as the code; RE findings and addresses go in `docs/re/`. Run `python3 scripts/check-links.py` after doc changes.
- Commit and push after each working step (`origin main`). Submodules: commit and push inside the submodule first, then bump the pointer in the hub (the server's own submodules first inside `modules/server`).
- Ask the user for x64dbg/dynamic debugging and for official captures. Steam may be used and started for private-server tests without asking; never log in to official servers.

## Roadmap (details: [docs/plans/orchestration-2026-10.md](docs/plans/orchestration-2026-10.md), status: README tracker)
Done (2026-10-20/21): foundation (restructure, dumps, repo split, docs, config, tracker); server-authoritative movement from official captures; accounts with the standalone login; remote hosting; shared instance world with two-client multiplayer; web API + web admin; launcher (end-to-end test with a depot download pending); consistency pass.
**Current state and next steps:** [docs/plans/handoff-agent-9.md](docs/plans/handoff-agent-9.md) (213,860 of 218,705 gated, 2026-10-26; read [handoff-agent-7.md](docs/plans/handoff-agent-7.md) first for preferences and method, [handoff-agent-8.md](docs/plans/handoff-agent-8.md) for the `runtime/w7` tools) (handoff 7: user preferences, commit rules, method, start-up checklist). Older detail: the orchestrator handoffs [3](docs/plans/handoff-orchestrator-3.md) to [6](docs/plans/handoff-orchestrator-6.md) and the terrain plan [docs/plans/terrain-all-areas.md](docs/plans/terrain-all-areas.md).
Open elsewhere: movement gaps #11/#12 (docs/protocol/movement.md), party/trade replies, remote equipment visuals.

## Where things are
- Repositories and how they depend on each other: [docs/overview.md](docs/overview.md)
- Commands, logs, unattended in-game tests: [docs/guides/setup.md](docs/guides/setup.md)
- Server: build and run it from the hub with `cargo xtask server build|run|test` (working directory = the hub, so `GameFiles/`, `dumps/`, `runtime/` and `modules/web-admin/dist` resolve; `run` rebuilds the web admin when its sources are newer than `dist/build.json`). Binary: `modules/server/target/debug/poe-server`. Hosting and release docs: `modules/server/docs/`
- Dev config: the committed `modules/server/config/server.toml` is safe (loopback, no dev tools; there is no password-less login, tests sign in with a Remember Me token). For in-game tests run `python3 scripts/dev-config.py` and start the server with `cargo xtask server run --config runtime/server.dev.toml` (or `POE_CONFIG=runtime/server.dev.toml`) ([setup.md](docs/guides/setup.md#developer-config))
- Server commands and admins: [docs/guides/server-admin.md](docs/guides/server-admin.md)
- Architecture and crates: [docs/architecture/](docs/architecture/overview.md)
- Protocol: [docs/protocol/](docs/protocol/framing.md); RE how-to (Ghidra MCP, `scripts/re/`): [docs/re/README.md](docs/re/README.md)
- Terrain status: [docs/status/terrain.md](docs/status/terrain.md); history: [docs/status/history.md](docs/status/history.md)
- Everything else: [docs/index.md](docs/index.md)
