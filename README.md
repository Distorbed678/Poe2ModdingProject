# Poe2ModdingProject

A private server for **Path of Exile 2 (client 0.5.5e)**, written in Rust and designed for future client and server modding. It is a personal, educational project: it never touches the official Steam install, and on official servers the client hook only observes and logs (read-only).

## How the repos fit together

This repo holds the server. The shared protocol code, the client hook and the web admin live in their own repos, included as git submodules. All repos are private, under `Distorbed678`.

| Repo | Where | Role |
|---|---|---|
| **Poe2ModdingProject** (this repo) | `/` | `servers/` (login, instance, and the `poe-server` launcher that runs both), `crates/` (config, data, store, cluster, terrain generator, simulation), `xtask`, `docs/`, `scripts/` |
| [Poe2Protocol](https://github.com/Distorbed678/Poe2Protocol) | `modules/protocol` | `poe-proto`, `poe-net` (codec and ciphers) and the opcode tables `opcodes/<version>/*.toml`, used by the server and the client hook |
| [Poe2ClientHook](https://github.com/Distorbed678/Poe2ClientHook) | `modules/client-hook` | The client DLLs (`version.dll` loader, hot-reloadable debug core with the in-game overlay) and `poe-hook-deploy`; carries Poe2Protocol as a nested submodule |
| [Poe2Launcher](https://github.com/Distorbed678/Poe2Launcher) | `modules/launcher` | Player launcher (Tauri 2 + Svelte 5): server profiles and login (keyring), installs 0.5.5e from Steam depots, deploys the hook and starts the game, manages mods ([docs/guides/launcher.md](docs/guides/launcher.md)); carries Poe2ClientHook as a nested submodule |
| [Poe2WebAdmin](https://github.com/Distorbed678/Poe2WebAdmin) | `modules/web-admin` | Web admin site (Svelte 5 + Vite + TypeScript) on top of the server's web API; build it and set `[web] static_dir = "modules/web-admin/dist"` ([docs/guides/server-admin.md](docs/guides/server-admin.md#web-admin)) |

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
> **Current task:** [Phase 2: movement rewrite (WP2.4 verified in game; gaps #11/#12 open; capture gate G-MOVE open)](docs/plans/orchestration-2026-10.md)

| Subsystem | Progress | Done |
|---|---|---|
| [Login](#login) | `████████░░ 80%` | 3/5 |
| [Accounts](#accounts) | `██████░░░░ 63%` | 5/8 |
| [Instances/Transfers](#instancestransfers) | `████████░░ 75%` | 4/6 |
| [Movement](#movement) | `████░░░░░░ 43%` | 0/7 |
| [Multiplayer](#multiplayer) | `███████░░░ 67%` | 3/6 |
| [Chat/Commands](#chatcommands) | `██████████ 100%` | 4/4 |
| [Terrain](#terrain) | `██░░░░░░░░ 23%` | 1/11 |
| [Inventory](#inventory) | `██░░░░░░░░ 17%` | 0/3 |
| [Monsters/Skills](#monstersskills) | `███░░░░░░░ 30%` | 1/5 |
| [Client Hook](#client-hook) | `█████████░ 90%` | 4/5 |
| [Launcher](#launcher) | `████████░░ 83%` | 4/6 |
| [Web Admin](#web-admin) | `█████████░ 88%` | 7/8 |
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
| 🟡 | [Standalone email/password login screen](docs/re/standalone-login.md) | RE done (WP3.1), the DLL forces it (WP3.3, `login_mode`) and the login server handles 0x0003 (WP3.2, tested with encoded packets); the in-game check is open |

### Accounts

Docs: [docs/architecture/accounts.md](docs/architecture/accounts.md)

| | Item | Note |
|---|---|---|
| ✅ | [Machine-hash accounts with SQLite storage (accounts, characters, unique names)](docs/architecture/accounts.md) |  |
| ✅ | Tickets, resume tickets and login sessions (many accounts at once) |  |
| ✅ | [Passwords (argon2id) and chosen account names; roles; disable (poe-accounts, migration 0006)](docs/architecture/accounts.md) | WP3.2; verified by tests, not yet with the real standalone screen (WP3.3) |
| ✅ | [Remember Me and per-cluster tokens (hashed, expiring, revocable, rolling)](docs/architecture/accounts.md) | game tokens in use; launcher and web tokens issued by the web API (WP4.1) |
| ✅ | [Duplicate login ends the old session](docs/architecture/accounts.md) | LoginSessions::supersede; unit-tested, not yet tried with two clients |
| ⬜ | [Rate limiting of password attempts](docs/architecture/accounts.md) |  |
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
| 🟡 | [Instance actor (one World per instance)](docs/architecture/instance-world.md) | WP3.6: one task per instance owns the World, the entity counter and the frame clock; sessions join, send input and leave through messages; unit-tested with fake sessions (two players, join, relay, leave, supersede, pause, idle end) and checked in game with one client (spawn, walk, roll, drift) and in WP3.7 with two |

### Movement

Docs: [docs/protocol/movement.md](docs/protocol/movement.md)

| | Item | Note |
|---|---|---|
| 🟡 | [WASD and click-to-move (held click, turns)](docs/protocol/movement.md) | server-authoritative World/Locomotion with the official reply sequences, verified against captures by replay tests and in game 2026-10-20 (drift checker): WASD walk/turn 1-3 cells, click-to-move 0-1.3, held click 2.2; gaps in the movement spec's gap list |
| 🟡 | [Dodge roll](docs/protocol/movement.md) | official reply batch and per-frame speed profile (tap and sprint rolls); in game: standing roll on open ground 0.5-3.9 cells, while walking 4, while steering by a moving cursor 7; open: roll heading in the first frames and in click-to-move mode (gap #12) |
| 🟡 | [Facing the cursor](docs/protocol/movement.md) | 0x62 aim echoed as 0x1dc like official servers (replay-tested); the cursor/0x1df workarounds and their config keys are gone |
| 🟡 | [Sprint](docs/protocol/movement.md) | official sequence (3 buffs, stats, 0x64 stream, end) and the sprint-roll speed profile; in game: sprint 1.0 s 2.7 cells after the stop, 1.4 s with W held since before the roll 10-13 (gap #11: the client slows down on its own after the sprint) |
| 🟡 | [Capture analysis: 0x1ae / 0x1af / 0x1b1 / 0x1ac, official sequences](docs/protocol/movement.md) | decoded; 2026-10-20 capture covers click-to-move, standing/chained rolls, roll direction and speed profile, input-mode stance; remaining items in the gap list |
| 🟡 | [Server-authoritative movement on the walkability grid (official packets only)](docs/plans/orchestration-2026-10.md) | built (WP2.3), verified in game (WP2.4, 2026-10-20): drift under ~4 cells walking and rolling, 0-3 after stops; open: gaps #11 and #12 of the movement spec |
| ❌ | [Server collision](docs/status/terrain.md) | needs the terrain generator to cover more areas |

### Multiplayer

Docs: [docs/guides/hosting.md](docs/guides/hosting.md)

| | Item | Note |
|---|---|---|
| 🟡 | [Remote client login (trusted_servers in the DLL)](docs/guides/hosting.md) | WP3.3: DLL side built and unit-tested (trusted servers are not blocked, not latched, get the crypto bypass); not yet tried against a LAN server in game |
| ✅ | [Remote readiness: advertised host, layout/grid upload, session-keyed debug link](docs/guides/hosting.md) | WP3.4: terrain layout/grid upload over the instance TCP port (offer, then zlib data, cached per tile hash); PSY/debug-link datagrams replaced by HMAC-SHA256 frames keyed by session (key from the ticket keys, replay window, version byte); [network.login] public_port and a loopback public_host warning; verified in game with an empty layout dir (upload, spawn, authenticated drift check); not yet tried from a second machine |
| ✅ | [Remote-player protocol (add/remove, broadcast actions)](docs/architecture/instance-world.md) | WP3.5 decoded it from the official town capture (docs/protocol/entities.md); WP3.6 sends it (remote AddObject blob, removal sequence, relayed simulation messages); WP3.7: two real clients see each other, walk and roll are relayed, the leave sequence on a transfer works (remote model looks darker than the local one: not investigated) |
| 🟡 | [Shared world: players in one instance see each other](docs/architecture/instance-world.md) | WP3.6: all sessions of an instance share one World; every entity's events go to every session in the frame they are produced, join/leave add and remove the remote player; pause only for a lone player; WP3.7 verified with two game clients (docs/guides/hosting.md); open: players spawn on one cell, no combat/party sync |
| ✅ | [Two-client local test harness](docs/guides/hosting.md) | WP3.7: scripts/test-mp.sh (two prefixes, LAN IP, token logins, per-client input files, F8 shots); login, shared instance, walk/roll relay, chat, transfer leave sequence and duplicate-login kick passed; found the town freeze (InitialState flags 7 in a town) |
| ❌ | Parties |  |

### Chat/Commands

Docs: [docs/guides/server-admin.md](docs/guides/server-admin.md)

| | Item | Note |
|---|---|---|
| ✅ | Chat channels (join, global, local, whisper) |  |
| ✅ | Server commands in chat (!) and in the overlay command palette |  |
| ✅ | Live registry: who, kick, tell, broadcast, mute |  |
| ✅ | [Admins by account role, account commands (create, role, disable, passwd, list)](docs/architecture/accounts.md) |  |

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
| 🟡 | [Skills, skill replies 0x1ae / 0x1af / 0x1b1](docs/protocol/instance.md) | decoded (poe-proto skill.rs); not sent by the server yet |
| ❌ | Buffs |  |

### Client Hook

Docs: [docs/architecture/client-dll.md](docs/architecture/client-dll.md)

| | Item | Note |
|---|---|---|
| ✅ | Loader (version.dll proxy) and hot-reloadable debug core |  |
| ✅ | Overlay (Vulkan imgui), packet capture, snapshots |  |
| ✅ | Read-only mode on official servers (latch) |  |
| ✅ | [Deploy and launch library (poe-hook-deploy)](docs/architecture/crates.md) |  |
| 🟡 | [Standalone login screen, token auto-login, trusted_servers](docs/architecture/client-dll.md#login-mode-and-trusted-servers) | WP3.3: login_mode patch, shared is_trusted, `cargo xtask login-token`; in-game test plan pending |

### Launcher

Docs: [docs/guides/launcher.md](docs/guides/launcher.md)

| | Item | Note |
|---|---|---|
| ✅ | [Poe2Launcher repo (Tauri 2 + Svelte)](docs/guides/launcher.md) | WP5.1; submodule modules/launcher (nests Poe2ClientHook); Rust + vitest tests, CI on Linux and Windows |
| ✅ | [Server profiles and account login through the web API](docs/guides/launcher.md#the-flow) | profiles, register, login (kind launcher), token in the OS keyring, logout revokes; tested with a mock keyring and a real server |
| ✅ | [Game-token exchange for auto-login (`POST /api/v1/game-token`)](docs/architecture/web-api.md#endpoints) | launcher token -> one-off game_remember token (the game's rolling login would revoke the launcher token) |
| 🟡 | [Install 0.5.5e from Steam depots](docs/guides/launcher.md#install-in-detail) | DepotDownloader 3.4.0 pinned, args/prompts/progress tested with a stand-in; the real Steam login and download not run yet; verification values unconfirmed |
| 🟡 | [Deploy and launch via poe-hook-deploy](docs/guides/launcher.md#play-in-detail) | deploy, token file and Proton command tested with a fake Proton; real game start not run; Windows path untested |
| ✅ | [Mod install and ordering](docs/guides/launcher.md#mods) | files and poe-hook-mods/state.toml only; the DLL loader does not exist yet |

### Web Admin

Docs: [docs/architecture/web-api.md](docs/architecture/web-api.md)

| | Item | Note |
|---|---|---|
| ✅ | [Web API crate (axum, in poe-server)](docs/architecture/web-api.md) | WP4.1; register/login/password, tokens, 20 endpoints; tested with oneshot and a smoke run on shifted ports |
| ✅ | [OpenAPI spec (utoipa) for the SPA and launcher types](docs/architecture/openapi.json) | docs/architecture/openapi.json, `cargo xtask openapi` |
| ✅ | [Poe2WebAdmin SPA (Svelte 5 + Vite, submodule modules/web-admin)](docs/architecture/web-api.md#the-web-admin-spa) | WP4.2; login, dashboard, players, instances, accounts, characters, announce; typed from openapi.json; vitest + CI; served via `[web] static_dir` |
| ✅ | [Live log over WebSocket (ring buffer, level/target/text filter)](docs/architecture/web-api.md) | server side |
| ✅ | [Live log viewer page (filters, pause, autoscroll, reconnect)](docs/architecture/web-api.md#the-web-admin-spa) | WP4.2; 3000 lines in the DOM at most |
| ✅ | [Admin endpoints: sessions, instances, kick/mute/transfer/announce, accounts, characters](docs/architecture/web-api.md) | run the command registry via AdminControl |
| ✅ | [Players, instances, accounts and characters pages](docs/architecture/web-api.md#the-web-admin-spa) | WP4.2; kick/mute/transfer, close/create, role/disable, character detail, announce dialog; confirmations on destructive actions |
| ⬜ | [Inventory and passive-tree panels](docs/architecture/web-api.md#the-web-admin-spa) | stubs behind VITE_FEATURE_INVENTORY / VITE_FEATURE_PASSIVES; waiting for the inventory work and server endpoints |

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
