# Poe2ModdingProject

A private server for **Path of Exile 2 (client 0.5.5e)**, written in Rust and designed for future client and server modding. It is a personal, educational project: it never touches the official Steam install, and on official servers the client hook only observes and logs (read-only).

## How the repos fit together

This repo holds the server. The shared protocol code and the client hook live in their own repos, included as git submodules. All repos are private, under `Distorbed678`.

| Repo | Where | Role |
|---|---|---|
| **Poe2ModdingProject** (this repo) | `/` | `servers/` (login, instance, and the `poe-server` launcher that runs both), `crates/` (config, data, store, cluster, terrain generator, simulation), `xtask`, `docs/`, `scripts/` |
| [Poe2Protocol](https://github.com/Distorbed678/Poe2Protocol) | `modules/protocol` | `poe-proto`, `poe-net` (codec and ciphers) and the opcode tables `opcodes/<version>/*.toml`, used by the server and the client hook |
| [Poe2ClientHook](https://github.com/Distorbed678/Poe2ClientHook) | `modules/client-hook` | The client DLLs (`version.dll` loader, hot-reloadable debug core with the in-game overlay) and `poe-hook-deploy`; carries Poe2Protocol as a nested submodule |
| Poe2Launcher | planned | Player launcher: installs 0.5.5e, logs in, deploys the hook, manages mods ([docs/guides/launcher.md](docs/guides/launcher.md)) |
| Poe2WebAdmin | planned | Web admin site on top of the server's web API ([docs/architecture/web-api.md](docs/architecture/web-api.md)) |

Other folders: `third_party/` (downloaded tools, git-ignored), `runtime/` (server data and logs, git-ignored), `dumps/` (datamined outputs, git-ignored; [dumps/README.md](dumps/README.md)), `GameFiles/` (your copy of the 0.5.5e client, never in git). Every crate and server: [docs/architecture/crates.md](docs/architecture/crates.md).

## Quick start

Needs Linux with rustup, Steam with Proton Experimental, and a copy of the 0.5.5e client in `GameFiles/`. The full requirements, options and the unattended test tools are in [docs/guides/setup.md](docs/guides/setup.md).

```sh
git clone --recursive https://github.com/Distorbed678/Poe2ModdingProject.git   # private repos: gh auth login first
cd Poe2ModdingProject
third_party/fetch-tools.sh      # downloads the external tools into third_party/
cargo build
cargo run -p poe-server         # login (20481) and instance (6112) servers, config/server.toml
cargo xtask deploy-client       # builds the DLLs, points GameFiles/ at the local server
cargo xtask run-client          # launches the client through Proton (Steam must be running)
```

Server commands and admins: [docs/guides/server-admin.md](docs/guides/server-admin.md). `cargo xtask revert-client` restores `GameFiles/`.

## Documentation

Start at [docs/index.md](docs/index.md): guides, architecture, the wire protocol, reverse-engineering notes, status and plans. The rules for working in this repo are in [CLAUDE.md](CLAUDE.md).

## Status

Generated from [docs/status/tracker.toml](docs/status/tracker.toml) by `cargo xtask tracker`; do not edit this block by hand. The history of what was built when is in [docs/status/history.md](docs/status/history.md), the terrain generator in [docs/status/terrain.md](docs/status/terrain.md).

<!-- tracker:start -->
> **Current task:** [Phase 2: movement rewrite (WP2.1 capture analysis)](docs/plans/orchestration-2026-10.md)

| Subsystem | Progress | Done |
|---|---|---|
| [Login](#login) | `███████░░░ 70%` | 3/5 |
| [Accounts](#accounts) | `███░░░░░░░ 33%` | 2/6 |
| [Instances/Transfers](#instancestransfers) | `███████░░░ 67%` | 4/6 |
| [Movement](#movement) | `███░░░░░░░ 29%` | 0/7 |
| [Multiplayer](#multiplayer) | `░░░░░░░░░░ 0%` | 0/6 |
| [Chat/Commands](#chatcommands) | `█████████░ 88%` | 3/4 |
| [Terrain](#terrain) | `██░░░░░░░░ 23%` | 1/11 |
| [Inventory](#inventory) | `██░░░░░░░░ 17%` | 0/3 |
| [Monsters/Skills](#monstersskills) | `██░░░░░░░░ 20%` | 1/5 |
| [Client Hook](#client-hook) | `████████░░ 80%` | 4/5 |
| [Launcher](#launcher) | `░░░░░░░░░░ 0%` | 0/5 |
| [Web Admin](#web-admin) | `░░░░░░░░░░ 0%` | 0/4 |
| [Modding](#modding) | `█████░░░░░ 50%` | 2/5 |

✅ done · 🟡 partial · ❌ missing · ⬜ planned. Progress counts done as 1 and partial as 0.5.

### Login

Docs: [docs/protocol/login.md](docs/protocol/login.md)

| | Item | Note |
|---|---|---|
| ✅ | [Key exchange and encryption (DLL crypto bypass)](docs/re/crypto.md) |  |
| ✅ | [Authenticate, leagues, character list, hand-off to the instance server](docs/protocol/login.md) |  |
| ✅ | Character creation, selection and deletion |  |
| 🟡 | [CharacterList entry fields and creation error codes](docs/status/history.md) | some entry fields are still unexplained; refusal codes unknown |
| ⬜ | [Standalone email/password login screen](docs/plans/orchestration-2026-10.md) | RE spike WP3.1 |

### Accounts

Docs: [docs/architecture/accounts.md](docs/architecture/accounts.md)

| | Item | Note |
|---|---|---|
| ✅ | [Machine-hash accounts with SQLite storage (accounts, characters, unique names)](docs/architecture/accounts.md) |  |
| ✅ | Tickets, resume tickets and login sessions (many accounts at once) |  |
| ⬜ | [Passwords (argon2) and chosen account names](docs/plans/orchestration-2026-10.md) | WP3.2, new crate poe-accounts |
| ⬜ | [Launcher and web tokens, per-cluster](docs/plans/orchestration-2026-10.md) |  |
| ⬜ | Admin role in the database (replaces the config list) |  |
| ⬜ | Duplicate login kicks the old session |  |

### Instances/Transfers

Docs: [docs/architecture/instance-world.md](docs/architecture/instance-world.md)

| | Item | Note |
|---|---|---|
| ✅ | Instance directory (find, create, join, leave, close, idle expiry) |  |
| ✅ | [Transfers between instances (command, overlay Instances page)](docs/guides/server-admin.md) |  |
| ✅ | Forced topology and orientation per instance |  |
| ✅ | Saved location (area, seed, cell) restored on login |  |
| ❌ | [Waypoints and area transitions; transfers without character select](docs/status/history.md) | needs an official capture of a zone change |
| ⬜ | [Instance actor (one World per instance)](docs/plans/orchestration-2026-10.md) | WP2.2 / WP3.6 |

### Movement

Docs: [docs/protocol/movement.md](docs/protocol/movement.md)

| | Item | Note |
|---|---|---|
| 🟡 | [WASD and click-to-move (held click, turns)](docs/protocol/movement.md) | works, but via 0x1cb/0x1df and workarounds that official servers never use |
| 🟡 | Dodge roll | roll speed profile only; no official reply sequence |
| 🟡 | Facing the cursor | turn_cell / turn_timing / face_cursor workarounds |
| 🟡 | [Sprint](docs/status/history.md) | a full sprint needs the buff system |
| ⬜ | [Capture analysis: 0x1ae / 0x1af / 0x1b1 / 0x1ac, official sequences](docs/plans/orchestration-2026-10.md) | WP2.1 (current task) |
| ⬜ | [Server-authoritative movement on the walkability grid (official packets only)](docs/plans/orchestration-2026-10.md) | WP2.2 to WP2.4 |
| ❌ | [Server collision](docs/status/terrain.md) | needs the terrain generator to cover more areas |

### Multiplayer

Docs: [docs/guides/hosting.md](docs/guides/hosting.md)

| | Item | Note |
|---|---|---|
| ⬜ | [Remote client login (trusted_servers in the DLL)](docs/guides/hosting.md) | WP3.3; the DLL treats every non-loopback server as official today |
| ⬜ | [Remote readiness: advertised host, layout/grid upload, session-keyed debug link](docs/plans/orchestration-2026-10.md) | WP3.4 |
| ⬜ | [Remote-player protocol (add/remove, broadcast actions)](docs/architecture/instance-world.md) | WP3.5; needs an official capture of a town with other players |
| ⬜ | [Shared world: players in one instance see each other](docs/architecture/instance-world.md) | WP3.6; every connection is a private simulation today |
| ⬜ | [Two-client local test harness](docs/plans/orchestration-2026-10.md) | WP3.7 |
| ❌ | Parties |  |

### Chat/Commands

Docs: [docs/guides/server-admin.md](docs/guides/server-admin.md)

| | Item | Note |
|---|---|---|
| ✅ | Chat channels (join, global, local, whisper) |  |
| ✅ | Server commands in chat (!) and in the overlay command palette |  |
| ✅ | Live registry: who, kick, tell, broadcast, mute |  |
| 🟡 | [Admins by account name from config](docs/architecture/accounts.md) | tied to auto-generated names; moves to a role (accounts) |

### Terrain

Docs: [docs/status/terrain.md](docs/status/terrain.md)

| | Item | Note |
|---|---|---|
| ✅ | Client builds terrain from area and seed (any area loads) |  |
| 🟡 | [Server-side generator, act 1: 25 of 33 sweep vectors match](docs/status/terrain.md) | Town and G1_1, 2, 3, 6, 7, 8, 9 match 3/3; the per-area matrix comes later |
| 🟡 | [G1_12: grid overlap (map size 169x85 vs 174x87)](docs/status/terrain.md) | all 2385 terrain draws match; layout differs |
| 🟡 | [G1_4: overlay phase 2 (seed 1003)](docs/status/terrain.md) | 2/3 seeds match |
| ❌ | [G1_5: edge routing ("could not tilise edge 14")](docs/status/terrain.md) | 0/3 seeds |
| ❌ | [G1_11: height-change (cliff) edge types, woodscliff_height.et chirality](docs/status/terrain.md) | 0/3 seeds |
| ❌ | [G1_13 to G1_15: client traces, then port](docs/status/terrain.md) | the user records the traces |
| ❌ | [World-map pins (natural topology choice)](docs/status/terrain.md) | forced topology covers it for now |
| ❌ | [Doodads and the doodad hash](docs/status/terrain.md) |  |
| ⬜ | [Per-area x topology x orientation matrix (cargo xtask terrain-matrix)](docs/plans/orchestration-2026-10.md) | WP7.1 |
| ⬜ | [Collision and walkability output for server movement](docs/plans/orchestration-2026-10.md) |  |

### Inventory

Docs: [docs/protocol/instance.md](docs/protocol/instance.md)

| | Item | Note |
|---|---|---|
| 🟡 | Inventories sent at area entry (empty, stash tab flags, full access) | structure only; no items |
| ❌ | [Items, equipment, stash](docs/re/captures.md) | the captures hold the sequences |
| ❌ | Passive tree |  |

### Monsters/Skills

Docs: [docs/protocol/entities.md](docs/protocol/entities.md)

| | Item | Note |
|---|---|---|
| ✅ | [Player entity with default stats from game data](docs/protocol/entities.md) |  |
| ❌ | NPCs (StateMachine and quest states) | POE_NPC adds one object for experiments only |
| ❌ | Monsters and spawns |  |
| ❌ | [Skills, skill replies 0x1ae / 0x1af / 0x1b1](docs/plans/orchestration-2026-10.md) | decoded in WP2.1 |
| ❌ | Buffs |  |

### Client Hook

Docs: [docs/architecture/client-dll.md](docs/architecture/client-dll.md)

| | Item | Note |
|---|---|---|
| ✅ | Loader (version.dll proxy) and hot-reloadable debug core |  |
| ✅ | Overlay (Vulkan imgui), packet capture, snapshots |  |
| ✅ | Read-only mode on official servers (latch) |  |
| ✅ | [Deploy and launch library (poe-hook-deploy)](docs/architecture/crates.md) |  |
| ⬜ | [Standalone login screen, token auto-login, trusted_servers](docs/plans/orchestration-2026-10.md) | WP3.1 / WP3.3 |

### Launcher

Docs: [docs/guides/launcher.md](docs/guides/launcher.md)

| | Item | Note |
|---|---|---|
| ⬜ | [Poe2Launcher repo (Tauri 2 + Svelte)](docs/guides/launcher.md) | WP5.1 |
| ⬜ | Server profiles and account login through the web API |  |
| ⬜ | Install 0.5.5e from Steam depots |  |
| ⬜ | Deploy and launch via poe-hook-deploy |  |
| ⬜ | Mod install and ordering |  |

### Web Admin

Docs: [docs/architecture/web-api.md](docs/architecture/web-api.md)

| | Item | Note |
|---|---|---|
| ⬜ | [Web API crate (axum, in poe-server)](docs/architecture/web-api.md) | WP4.1 |
| ⬜ | [Poe2WebAdmin SPA (Svelte 5)](docs/plans/orchestration-2026-10.md) | WP4.2 |
| ⬜ | Live log viewer over WebSocket |  |
| ⬜ | Instances, accounts and characters pages |  |

### Modding

Docs: [docs/guides/modding.md](docs/guides/modding.md)

| | Item | Note |
|---|---|---|
| ✅ | Server file overrides in front of Bundles2 |  |
| ✅ | Server command registry (one place for chat and overlay) |  |
| 🟡 | [mod.toml manifest spec for client mods](modules/client-hook/docs/mod-manifest.md) | spec only; loader does not read it yet |
| ⬜ | Hook bus for server mods (poe-mod-api) |  |
| ⬜ | Client loose-file override hook and packet hooks |  |
<!-- tracker:end -->
