# Poe2ModdingProject

A private server for **Path of Exile 2 (client 0.5.5e)**, written in Rust and designed for future client and server modding. It is a personal, educational project: it never touches the official Steam install, and on official servers the client hook only observes and logs (read-only).

## How the repos fit together

This repository is the **project hub**: the documentation, the plans, the reverse-engineering notes, the dev tooling (`xtask`, `scripts/`) and the progress tracker below. The code lives in separate repositories under `Distorbed678`, included here as git submodules in `modules/`. The full overview with a dependency diagram, the release channels and how a player, a host and a developer get started: **[docs/overview.md](docs/overview.md)**.

| Repo | Where | Role |
|---|---|---|
| **Poe2ModdingProject** (this repo) | `/` | Hub: `docs/`, `scripts/`, `xtask` (client deploy, DLL build, `check-build`, tracker, running the server from the hub), news |
| [Poe2Server](https://github.com/Distorbed678/Poe2Server) | `modules/server` | The server: `servers/` (login, instance, `poe-server`), `crates/` (config, data, store, accounts, cluster, web API, terrain generator, simulation, mod host), `config/`; releases `server-v*` |
| [Poe2Protocol](https://github.com/Distorbed678/Poe2Protocol) | `modules/protocol` | `poe-proto`, `poe-net` (codec and ciphers), the opcode tables `opcodes/<version>/*.toml` and the supported-build definitions `versions/<version>.toml` |
| [Poe2ClientHook](https://github.com/Distorbed678/Poe2ClientHook) | `modules/client-hook` | The client DLLs (`version.dll` loader, hot-reloadable debug core with the in-game overlay, mod loader), `poe-hook-deploy`, `poe-hook-mods`; releases `hook-v*` |
| [Poe2WebAdmin](https://github.com/Distorbed678/Poe2WebAdmin) | `modules/web-admin` | Web admin site (Svelte 5 + Vite + TypeScript) on top of the server's web API ([docs/guides/setup.md](docs/guides/setup.md#web-admin-build), [docs/architecture/web-api.md](docs/architecture/web-api.md)) |
| [Poe2Launcher](https://github.com/Distorbed678/Poe2Launcher) | `modules/launcher` | Player and host launcher (Tauri 2 + Svelte 5): server profiles and login, installs 0.5.5e from Steam depots, deploys the hook, starts the game, hosts a server, manages mods ([docs/guides/launcher.md](docs/guides/launcher.md)); releases `v*` |
| [Poe2ModSDK](https://github.com/Distorbed678/Poe2ModSDK) | `modules/mod-sdk` | SDK for server and client mods (`poe-mod-api`, `poe-mod` tool, examples) |
| [Poe2Mods](https://github.com/Distorbed678/Poe2Mods) | `modules/mod-index` | Public mod index read by the launcher |
| [Poe2ServerList](https://github.com/Distorbed678/Poe2ServerList) | `modules/server-list` | Public server list read by the launcher |

Other folders: `third_party/` (downloaded tools, git-ignored), `runtime/` (server data and logs, git-ignored), `dumps/` (datamined outputs, git-ignored; [dumps/README.md](dumps/README.md)), `GameFiles/` (your copy of the 0.5.5e client, never in git). Every crate and server: [docs/architecture/crates.md](docs/architecture/crates.md).

**Players** use the launcher ([docs/guides/client.md](docs/guides/client.md)); **hosts** take a [server release](https://github.com/Distorbed678/Poe2Server/releases) or the launcher's *Host a server* page ([hosting guide](modules/server/docs/hosting.md)); **developers** clone with `git clone --recursive`.

## Quick start

Needs Linux with rustup, Steam with Proton Experimental, and a copy of the 0.5.5e client in `GameFiles/`. The full requirements, options and the unattended test tools are in [docs/guides/setup.md](docs/guides/setup.md).

The steps are in [docs/guides/setup.md](docs/guides/setup.md): [clone](docs/guides/setup.md#clone) with submodules, [fetch the tools](docs/guides/setup.md#tools), [build](docs/guides/setup.md#build-and-test), [start the server and deploy the client](docs/guides/setup.md#run) into `GameFiles/`, and [play](docs/guides/setup.md#playing-from-steam-non-steam-game-shortcut) or launch the client through Proton. `revert-client` restores `GameFiles/`.

Build and run the server from the hub with `cargo xtask server build` and `cargo xtask server run` (it runs with this folder as its working directory and rebuilds the web admin when its sources changed); the server alone builds from its own repository ([Poe2Server](https://github.com/Distorbed678/Poe2Server)).

Server commands and admins: [docs/guides/server-admin.md](docs/guides/server-admin.md).

## Documentation

Start at [docs/index.md](docs/index.md): guides, architecture, the wire protocol, reverse-engineering notes, status and plans. The rules for working in this repo are in [CLAUDE.md](CLAUDE.md).

## Disclaimer

This is an unofficial, fan-made project for personal and educational use. It is **not affiliated with, endorsed by or connected to Grinding Gear Games**; "Path of Exile" is their trademark. **No game files are distributed** here (no assets, bundles, tables or executables): you need your own legal copy of the game, and the project only reads it from your machine. The client hook is meant for the private server; on official servers it can be used in an **observe-only** (read-only) mode, described in the [official mode guide](docs/guides/official-mode.md). Even then it patches game code in memory to observe, which the game's anti-cheat may detect, so do not use it on an account you care about. You use all of this at your own risk. More: [SECURITY.md](SECURITY.md), [CONTRIBUTING.md](CONTRIBUTING.md); licensed under MIT ([LICENSE](LICENSE)).

## Status

Generated from [docs/status/tracker.toml](docs/status/tracker.toml) by `cargo xtask tracker`; do not edit this block by hand. The history of what was built when is in [docs/status/history.md](docs/status/history.md), the terrain generator in [docs/status/terrain.md](docs/status/terrain.md).

<!-- tracker:start -->
> **Current task:** [Terrain port to all areas (docs/plans/terrain-all-areas.md)](docs/plans/terrain-all-areas.md)

| Subsystem | Progress | Done |
|---|---|---|
| [Login](#login) | `█████████░ 92%` | 5/6 |
| [Accounts](#accounts) | `██████████ 100%` | 6/6 |
| [Instances/Transfers](#instancestransfers) | `████████░░ 83%` | 5/6 |
| [Movement](#movement) | `████░░░░░░ 43%` | 0/7 |
| [Multiplayer](#multiplayer) | `███████░░░ 71%` | 4/7 |
| [Chat/Commands](#chatcommands) | `██████████ 100%` | 4/4 |
| [Terrain](#terrain) | `█████░░░░░ 46%` | 5/14 |
| [Inventory](#inventory) | `██░░░░░░░░ 17%` | 0/3 |
| [Monsters/Skills](#monstersskills) | `███░░░░░░░ 30%` | 1/5 |
| [Client Hook](#client-hook) | `████████░░ 83%` | 4/6 |
| [Launcher](#launcher) | `██████████ 100%` | 7/7 |
| [Web Admin](#web-admin) | `█████████░ 88%` | 7/8 |
| [Modding](#modding) | `█████████░ 90%` | 9/10 |

✅ done · 🟡 partial · ❌ missing · ⬜ planned. Progress counts done as 1 and partial as 0.5.

### Login

Docs: [docs/protocol/login.md](docs/protocol/login.md)

| | Item | Note |
|---|---|---|
| ✅ | [Key exchange and encryption (DLL crypto bypass)](docs/re/crypto.md) |  |
| ✅ | [Authenticate, leagues, character list, hand-off to the instance server](docs/protocol/login.md) |  |
| ✅ | Character creation, selection and deletion |  |
| ✅ | [Character create/delete refusal codes (BackendErrors table)](docs/protocol/login.md#create-and-delete-result-codes) | WP B2: name taken/invalid/short/long, class, league, slots, delete missing/foreign, store error; codes named once in poe-proto and checked against the table at start; in-game check of the texts pending |
| 🟡 | [CharacterList entry fields](docs/protocol/login.md#characterlist-entry-fields) | named: class/ascendancy, level, league banner, areas, portrait frame/variation, quest state; still unexplained and always 0 officially: entry +0x4f and the list tail +0x68, +0x74, +0x76 (need an official capture of a shapeshifted or supporter character, not a server change) |
| ✅ | [Standalone email/password login screen](docs/re/standalone-login.md) | the DLL forces the standalone screen and skips the patcher; password and remember-me token logins verified in game. WP B2: stale saved logins answer 0x0f (not the Unlock window) and the strict-unlock banner is gone (flags 0x23); in-game check pending |

### Accounts

Docs: [docs/architecture/accounts.md](docs/architecture/accounts.md)

| | Item | Note |
|---|---|---|
| ✅ | [Machine-hash accounts with SQLite storage (accounts, characters, unique names)](docs/architecture/accounts.md) |  |
| ✅ | Tickets, resume tickets and login sessions (many accounts at once) |  |
| ✅ | [Passwords (argon2id) and chosen account names; roles; disable (poe-accounts, migration 0006)](docs/architecture/accounts.md) | WP3.2; verified with the real standalone screen in game (WP3.3) |
| ✅ | [Remember Me and per-cluster tokens (hashed, expiring, revocable, rolling)](docs/architecture/accounts.md) | game tokens in use; launcher and web tokens issued by the web API (WP4.1) |
| ✅ | [Duplicate login ends the old session](docs/architecture/accounts.md) | LoginSessions::supersede; verified with two real clients (plan 1 WP3.7, modules/server/docs/hosting.md) |
| ✅ | [Rate limiting of password attempts (game login and web API)](docs/architecture/accounts.md) | WP B2: [security] per account name and per IP, result 0x49 in game, HTTP 429 with Retry-After on the web; unit and login-level tests; in-game message check pending |

### Instances/Transfers

Docs: [docs/architecture/instance-world.md](docs/architecture/instance-world.md)

| | Item | Note |
|---|---|---|
| ✅ | Instance directory (find, create, join, leave, close, idle expiry) |  |
| ✅ | [Transfers between instances (command, overlay Instances page)](docs/guides/server-admin.md) |  |
| ✅ | Forced topology and orientation per instance |  |
| ✅ | Saved location (area, seed, cell) restored on login |  |
| ❌ | [Waypoints and area transitions; transfers without character select](docs/status/history.md) | needs an official capture of a zone change |
| ✅ | [Instance actor (one World per instance)](docs/architecture/instance-world.md) | WP3.6: one task per instance owns the World, the entity counter and the frame clock; sessions join, send input and leave through messages; unit-tested with fake sessions (two players, join, relay, leave, supersede, pause, idle end) and checked in game with one client (spawn, walk, roll, drift) and in WP3.7 with two |

### Movement

Docs: [docs/protocol/movement.md](docs/protocol/movement.md)

| | Item | Note |
|---|---|---|
| 🟡 | [WASD and click-to-move (held click, turns)](docs/protocol/movement.md) | server-authoritative World/Locomotion with the official reply sequences, verified against captures by replay tests and in game 2026-10-20 (drift checker): WASD walk/turn 1-3 cells, click-to-move 0-1.3, held click 2.2; gaps in the movement spec's gap list; 2026-10-21: walks, WASD, sprints and rolls are planned with a port of the client's own pathfinder (docs/re/pathfinding.md; 536 of 541 logged client plans identical), in game WASD around rocks 29.6 -> 1.4 cells, click around a rock 6.3 -> 0, a 120-cell click 0; walk speed has no deficit (mean 1.221 cells per frame on both sides, docs/protocol/movement.md Speeds) |
| 🟡 | [Dodge roll](docs/protocol/movement.md) | official reply batch and per-frame speed profile (tap and sprint rolls); roll heading falls back to the facing (gap #12 closed, WP C1); in game 2026-10-21 (drift checker): standing roll in click and real WASD mode (user_input_mode=wasd) 3.1-4.2 cells in the second, 0.2-1.1 after the stop (2.4 into a wall), click then roll 3.9 / 0.5, held click plus roll 3.9-4.2 / 0-9.4, W and D then a roll 4.0 / 1.3; roll with W held since before it and Space released in the roll 10 / 8.9 (client coasts, see Sprint); earlier: while walking 4, steering by a moving cursor 7; 2026-10-21 rolls are planned like the client (roll into a wall 14.7 -> 1.4 cells, diagonal wall 11.5 -> 0.6; the first-leg deviation is gone) |
| 🟡 | [Facing the cursor](docs/protocol/movement.md) | 0x62 aim echoed as 0x1dc like official servers (replay-tested); the cursor/0x1df workarounds and their config keys are gone |
| 🟡 | [Sprint](docs/protocol/movement.md) | official sequence (3 buffs, stats, 0x64 stream, end) and the sprint-roll speed profile; the end coasts like the client (gap #11, WP C1, replay-tested): in game with the KWin Agents rules off, Space-only free-angle cursor sprints end 0.1-4.2 cells off in click and WASD mode (11 runs; under the rules 23-28, the hidden desktop is the likely cause, not the 0x1ac timing); an exactly diagonal cursor (the vector of W) is told from a key by the click-mode trailer byte or by the roll key's cursor offset and coasts as well (problem 1 fixed: 20 cells -> 0.5, 5 click and 2 WASD runs); open: a sprint ended with W held since before the roll (no 0x63 stream) is 14-17 cells off because the client coasts 30 frames and resumes by itself, the unscaled 0x64 does not restart it (problem 5); WASD injection was the input mode, not the focus (problem 4 solved); see movement.md; problem 2 fixed 2026-10-21 (the client sprints at pathing level 4, 5 cells off walls: sprint towards a diagonal wall 8.1 -> 0.2 cells after the stop, cursor sprints 0.5-1.0) |
| 🟡 | [Capture analysis: 0x1ae / 0x1af / 0x1b1 / 0x1ac, official sequences](docs/protocol/movement.md) | decoded; 2026-10-20 capture covers click-to-move, standing/chained rolls, roll direction and speed profile, input-mode stance; remaining items in the gap list |
| 🟡 | [Server-authoritative movement on the walkability grid (official packets only)](docs/plans/orchestration-2026-10.md) | built (WP2.3), verified in game (WP2.4, 2026-10-20): drift under ~4 cells walking and rolling, 0-3 after stops; open: gaps #11 and #12 of the movement spec |
| ❌ | [Server collision](docs/status/terrain.md) | needs the terrain generator to cover more areas |

### Multiplayer

Docs: [modules/server/docs/hosting.md](modules/server/docs/hosting.md)

| | Item | Note |
|---|---|---|
| ✅ | [Remote client login (trusted_servers in the DLL)](modules/server/docs/hosting.md) | trusted servers are not blocked or latched and get the crypto bypass; verified with two clients over the LAN IP (WP3.7); a second physical machine is untested |
| ✅ | [Remote readiness: advertised host, layout/grid upload, session-keyed debug link](modules/server/docs/hosting.md) | WP3.4: terrain layout/grid upload over the instance TCP port (offer, then zlib data, cached per tile hash); PSY/debug-link datagrams replaced by HMAC-SHA256 frames keyed by session (key from the ticket keys, replay window, version byte); [network.login] public_port and a loopback public_host warning; verified in game with an empty layout dir (upload, spawn, authenticated drift check); not yet tried from a second machine |
| ✅ | [Remote-player protocol (add/remove, broadcast actions)](docs/architecture/instance-world.md) | WP3.5 decoded it from the official town capture (docs/protocol/entities.md); WP3.6 sends it (remote AddObject blob, removal sequence, relayed simulation messages); WP3.7: two real clients see each other, walk and roll are relayed, the leave sequence on a transfer works (remote model looked darker and orange before WP C2; WP C2 built the remote blob to the official layout and fixed the remote roll's stat lists; in-game re-check 2026-10-21: no remote tint left, the remote roll animates and its packets match the official order; the bisect switches stayed unused, modules/server/docs/hosting.md) |
| 🟡 | [Shared world: players in one instance see each other](docs/architecture/instance-world.md) | WP3.6: all sessions of an instance share one World; every entity's events go to every session in the frame they are produced, join/leave add and remove the remote player; pause only for a lone player; WP3.7 verified with two game clients (modules/server/docs/hosting.md); WP C2: a second player spawns on a free walkable cell (`[spawn] player_spacing`, verified in game: cells 8 apart); open: no combat sync |
| 🟡 | [Party and trade requests (invite, cancel)](docs/protocol/entities.md#party-and-trade-flows-wp-c3) | WP C3: party invite and cancel, trade request and cancel between two sessions, verified with two real clients 2026-10-21 (invitee popups, cancel, leave; a trade request needs the two slot-0x79 AddInventory first, else both clients crash with "Invalid inventory id"); accept, decline, timeouts and joined members are not implemented: gate G-PARTY (needs an official capture where a friend answers) |
| ✅ | [Two-client local test harness](modules/server/docs/hosting.md) | WP3.7: scripts/test-mp.sh (two prefixes, LAN IP, token logins, per-client input files, F8 shots); login, shared instance, walk/roll relay, chat, transfer leave sequence and duplicate-login kick passed; found the town freeze (InitialState flags 7 in a town) |
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
| 🟡 | [Server-side generator, act 1: 26 of 33 sweep vectors match](docs/status/terrain.md) | Town and G1_1, 2, 3, 6, 7, 8, 9 match 3/3, G1_12 1/1; 365 of 1973 play-log and sweep vectors match over 59 areas; all areas: the Terrain matrix below |
| ✅ | [G1_12: grid overlap (RoomOverlap read from the master .tsi)](docs/status/terrain.md) | 2026-10-22: map 174x87, all 15138 cells and the tile hash equal the client's (seed 1001, topology 2, orientation 7) |
| 🟡 | [G1_4: overlay phase 2 (seed 1003)](docs/status/terrain.md) | 2/3 seeds match |
| ❌ | [G1_5: edge routing ("could not tilise edge 14")](docs/status/terrain.md) | 0/3 seeds |
| ❌ | [G1_11: height-change (cliff) edge types, woodscliff_height.et chirality](docs/status/terrain.md) | 0/3 seeds |
| ❌ | [G1_13 to G1_15: client traces, then port](docs/status/terrain.md) | the user records the traces |
| ❌ | [World-map pins (natural topology choice)](docs/status/terrain.md) | forced topology covers it for now |
| ✅ | [Doodads and the doodad hash](docs/re/terrain/doodads.md) | 2026-10-22 (WP7.11): doodads.rs; every one of the 72,718 tile-matching client layouts also matches the client's doodad hash; 302 of 303 exports equal in list, hash and angles (MapVaalVault: 2 angles) |
| ✅ | [Per-area x topology x orientation matrix (cargo xtask terrain-matrix)](docs/plans/terrain-all-areas.md) | 2026-10-22, after wave 3 on the 50-seed client batch: 442 WorldAreas rows (10 without topologies), 1141 topologies, 4508 combinations: 1074 match (all seeds), 1658 partial, 1737 fail, 39 untested; 108828 of 218988 seeds match (37663 after the first batch, 73001 after wave 2); generator probe: 701 of 1141 topologies run at seed 1001 (first orientation), 588 run for all orientations and seeds 1001-1003; table below the Terrain items |
| ✅ | [Verification at scale: bulk vectors (WP7.2, done), forced-layout sweeps (WP7.3) and the client batch (WP7.16)](docs/plans/terrain-all-areas.md#2-verification-at-scale) | 2026-10-22: bulk vectors, multi-seed probe, forced_verify and the in-client batch are done; full run: 225,400 client layouts (every combination x 50 seeds) in 1.3 h, one launch, 0 crashes; 218,705 generated by the client, 6,695 client-side generator errors; the port matches 37,380 (17%); forced login sweeps (WP7.3) only as a cross-check |
| ❌ | [Dungeon height-change and virtual edge types (chirality), unlocks 44 areas](docs/plans/terrain-all-areas.md#4-unported-systems-ranked-by-areas-unlocked) | 149 topologies stop at the guard in dungeon.rs; G1_11 is the first test |
| ❌ | [3x3 door spots (0x14209dc90), unlocks 27 areas; FileGroups and nested subgraphs, 11 areas](docs/plans/terrain-all-areas.md#4-unported-systems-ranked-by-areas-unlocked) |  |
| 🟡 | [Collision and walkability output for server movement](docs/plans/terrain-all-areas.md#4-unported-systems-ranked-by-areas-unlocked) | 2026-10-22 (WP7.12): collision.rs builds the PWG1 grid (0x141d0e5f0 + doodad hulls); 273 of 303 client exports equal grids.bin in all 4 layers (fails: lattice flag 8 in MapSlash and others); [terrain] generated_grid (default off) uses it when the tile hash equals the client's; no live movement test yet |

### Terrain matrix

Generated by `cargo xtask terrain-matrix` into [docs/status/terrain-matrix.tsv](docs/status/terrain-matrix.tsv) (area x topology x orientation; plan: [docs/plans/terrain-all-areas.md](docs/plans/terrain-all-areas.md)). **442 areas (10 without topologies), 1141 topologies, 4508 combinations: 1074 match, 1658 partial, 1737 fail, 39 untested; seeds matched 108828/218988**. Match = every tested seed matched the client's tile hash (the goal is 50/50 seeds per combination), partial = some did, seeds = matched/tested seeds summed over the combinations. 0 of the combinations with a match are vouched for only by vectors of older client builds. Probe = the generator runs to the end for the probed seeds (no hash check).

| Act | Areas | Topologies | Combinations | Match | Partial | Fail | Untested | Seeds | Probe ok |
|---|---|---|---|---|---|---|---|---|---|
| 1 | 111 | 260 | 966 | 462 | 198 | 304 | 2 | 27437/47385 | 155/260 |
| 2 | 45 | 141 | 543 | 183 | 144 | 204 | 12 | 13072/25748 | 81/141 |
| 3 | 28 | 117 | 361 | 54 | 53 | 253 | 1 | 3498/17538 | 51/117 |
| 4 | 28 | 88 | 301 | 10 | 43 | 248 | 0 | 1200/14938 | 34/88 |
| 6 | 24 | 31 | 33 | 11 | 1 | 21 | 0 | 584/1646 | 16/31 |
| 10 | 206 | 504 | 2304 | 354 | 1219 | 707 | 24 | 63037/111733 | 251/504 |
| **all** | 442 | 1141 | 4508 | 1074 | 1658 | 1737 | 39 | 108828/218988 | 588/1141 |

<details><summary>Per area (442 rows)</summary>

| Area | Type | Topologies | Combinations | Match | Partial | Fail | Untested | Seeds | Vectors | Probe ok |
|---|---|---|---|---|---|---|---|---|---|---|
| NULL | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| CharacterSelect | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| PersonalHideout | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| GuildHideout__ | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| CurrentTown | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| HideoutCave | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutShipgraveyard | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutSpace | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutArenaGraveyardTrio | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutPillarsOfArun | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutAtziriArena | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutInnocenceArena | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutShaperArena | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutKaomArena | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutDaressoArena | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutDominusArena | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutPietyArena | town | 1 | 1 | 0 | 0 | 0 | 1 | 0/0 | - | 1/1 |
| HideoutLunarisSolarisArena | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutKitavaArenaAct10 | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutDoomguard | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutSunspire | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutDarkwood | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutArenaElder | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutChiyouSpring | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankAbyss | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankCity | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankClouds | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankDesert | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankDirt | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankGrass | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankSea | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankSnow | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutThaumaturgy | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutYaochi | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutHumanoidPet | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutRitualLeague | town | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| HideoutArenaMaven | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutHasina | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutAzuriteCave | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutSynthesisHub | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutVeritaniaArena | town | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| HideoutBlankBlack | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutTencentApocalypse | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutOlrothArena | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutLight | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| HideoutDark | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutArenaBlackStar_ | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutArenaInfiniteHunger | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutCosmicAtlas | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutArenaEaterOfWorlds | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutArenaSearingExarch | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBeaconOfSalvation | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/3 | 1/1 |
| HideoutCanopy | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/2 | 1/1 |
| HideoutDreadnought | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/23 | 1/1 |
| HideoutFelled | town | 1 | 1 | 0 | 1 | 0 | 0 | 11/50 | 0/20 | 1/1 |
| HideoutShrine | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/11 | 1/1 |
| HideoutLimestone | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/16 | 1/1 |
| HideoutCanal | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/13 | 1/1 |
| HideoutMonolith | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutTwisted | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/1 | 1/1 |
| HideoutTranscendent | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutHourglass | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutChaos | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutSummit | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/4 | 1/1 |
| HideoutRacetrack | town | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| HideoutShipwreck | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutFarmlands | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutPrison | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutShoreline | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/14 | 1/1 |
| HideoutArenaKulemak | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutGrinding | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutArcaneIsle | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/3 | 1/1 |
| HideoutSacrifice | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutVerdant | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/2 | 1/1 |
| HideoutBlankUrban | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankMountain | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankDesert2 | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/2 | 1/1 |
| HideoutBlankCrypt | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutArenaIncursion | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/1 | 0/1 |
| HideoutSunkenPalace | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutColossalTitan | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/1 | 1/1 |
| HideoutBlankJungle | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankVolcanicCave | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutVampireManor | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/2 | 0/1 |
| HideoutRemidusMonastery_ | town | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 3/3 | 1/1 |
| HideoutBlankIce | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| HideoutBlankFire | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/1 | 1/1 |
| HideoutShip | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/2 | 1/1 |
| MapHideoutFarmlands_Claimable | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapHideoutPrison_Claimable | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapHideoutShoreline_Claimable | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapHideoutFelled_Claimable | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapHideoutLimestone_Claimable | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| MapHideoutShrine_Claimable | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapHideoutCanal_Claimable | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Design | tgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| Design_Lite | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| BlackTest | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Programming | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| Programming_Lite | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| G_login | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| G1_WorldMap | dgr | 6 | 6 | 0 | 0 | 6 | 0 | 0/300 | - | 6/6 |
| G2_WorldMap | dgr | 8 | 8 | 0 | 0 | 8 | 0 | 0/400 | - | 0/8 |
| G3_WorldMap | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| G4_WorldMap | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/48 | - | 1/1 |
| G6_WorldMap | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| TN_WorldMap | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| G1_town | town | 1 | 1 | 1 | 0 | 0 | 0 | 52/52 | 21/21 | 1/1 |
| G1_1 | tgr | 5 | 10 | 3 | 3 | 4 | 0 | 297/494 | 19/35 | 2/5 |
| G1_2 | tgr | 18 | 18 | 12 | 6 | 0 | 0 | 836/888 | 5/28 | 13/18 |
| G1_3 | dgr | 4 | 32 | 32 | 0 | 0 | 0 | 1614/1614 | 14/18 | 4/4 |
| G1_4 | tgr | 12 | 48 | 48 | 0 | 0 | 0 | 2212/2212 | 11/27 | 0/12 |
| G1_5 | tgr | 8 | 32 | 24 | 0 | 8 | 0 | 1196/1593 | 0/22 | 4/8 |
| G1_6 | dgr | 4 | 32 | 32 | 0 | 0 | 0 | 1606/1606 | 6/28 | 4/4 |
| G1_7 | dgr | 12 | 96 | 83 | 13 | 0 | 0 | 4304/4579 | 18/24 | 10/12 |
| G1_8 | dgr | 12 | 96 | 27 | 69 | 0 | 0 | 4384/4815 | 16/20 | 8/12 |
| G1_9 | dgr | 12 | 96 | 75 | 13 | 8 | 0 | 4262/4796 | 14/21 | 10/12 |
| G1_10 | dgr | 4 | 32 | 32 | 0 | 0 | 0 | 1584/1584 | - | 4/4 |
| G1_11 | dgr | 12 | 96 | 0 | 77 | 19 | 0 | 160/4734 | 0/22 | 0/12 |
| G1_12 | dgr | 12 | 96 | 80 | 16 | 0 | 0 | 4288/4495 | 5/16 | 6/12 |
| G1_13_1 | dgr | 16 | 128 | 0 | 0 | 128 | 0 | 0/6400 | 0/17 | 0/16 |
| G1_13_2 | dgr | 12 | 37 | 0 | 0 | 37 | 0 | 0/1842 | 0/14 | 0/12 |
| G1_14 | dgr | 6 | 6 | 6 | 0 | 0 | 0 | 278/278 | 0/14 | 0/6 |
| G1_15 | dgr | 16 | 16 | 0 | 0 | 16 | 0 | 0/800 | 0/14 | 0/16 |
| G2_town | town | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 19/19 | 1/1 |
| G2_1 | tgr | 8 | 64 | 0 | 0 | 64 | 0 | 0/2776 | 0/13 | 0/8 |
| G2_2 | dgr | 8 | 16 | 0 | 0 | 16 | 0 | 0/778 | 0/10 | 2/8 |
| G2_3 | dgr | 12 | 12 | 0 | 0 | 12 | 0 | 0/600 | 0/8 | 0/12 |
| G2_3a | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/2 | 0/1 |
| G2_3s | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| G2_4_1 | dgr | 4 | 32 | 0 | 0 | 32 | 0 | 0/1600 | 0/11 | 4/4 |
| G2_4_2 | dgr | 6 | 48 | 14 | 34 | 0 | 0 | 1508/2219 | 0/8 | 2/6 |
| G2_4_3 | dgr | 8 | 16 | 0 | 0 | 16 | 0 | 0/777 | 0/9 | 7/8 |
| G2_5_1 | dgr | 12 | 96 | 96 | 0 | 0 | 0 | 4800/4800 | 0/11 | 11/12 |
| G2_5_2 | dgr | 4 | 32 | 0 | 32 | 0 | 0 | 1229/1602 | 2/11 | 4/4 |
| G2_6 | dgr | 4 | 8 | 6 | 0 | 2 | 0 | 300/400 | 0/9 | 3/4 |
| G2_7 | dgr | 8 | 16 | 0 | 0 | 16 | 0 | 0/785 | 0/8 | 8/8 |
| G2_8 | dgr | 4 | 32 | 0 | 25 | 7 | 0 | 53/1599 | 0/8 | 0/4 |
| G2_8a | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| G2_9_1 | dgr | 12 | 48 | 48 | 0 | 0 | 0 | 2383/2383 | 0/7 | 12/12 |
| G2_9_2 | dgr | 4 | 16 | 0 | 16 | 0 | 0 | 390/798 | 0/7 | 0/4 |
| G2_10_1 | dgr | 8 | 64 | 0 | 26 | 30 | 8 | 1016/2593 | 1/12 | 6/8 |
| G2_10_2 | dgr | 4 | 8 | 1 | 7 | 0 | 0 | 352/400 | 0/10 | 4/4 |
| G2_11 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| G2_12 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/7 | 0/1 |
| G2_13 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 78/78 | 29/39 | 1/1 |
| Sanctum_1 | dgr | 1 | 1 | 0 | 0 | 0 | 1 | 0/0 | - | 0/1 |
| Sanctum_1_Foyer_1 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/15 | 1/1 |
| Sanctum_1_Foyer_2 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Sanctum_1_Foyer_3 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Sanctum_2 | dgr | 1 | 1 | 0 | 0 | 0 | 1 | 0/0 | - | 1/1 |
| Sanctum_2_Foyer_1 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Sanctum_2_Foyer_2 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Sanctum_2_Foyer_3 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/7 | 1/1 |
| Sanctum_3 | dgr | 1 | 1 | 0 | 0 | 0 | 1 | 0/0 | - | 0/1 |
| Sanctum_3_Foyer_1 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Sanctum_3_Foyer_2 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Sanctum_3_Foyer_3 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/5 | 1/1 |
| Sanctum_4 | dgr | 1 | 1 | 0 | 0 | 0 | 1 | 0/0 | - | 0/1 |
| Sanctum_4_Foyer_1 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/2 | 1/1 |
| Sanctum_4_Foyer_2 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Sanctum_4_Foyer_3 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/2 | 1/1 |
| G3_town | town | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/11 | 0/1 |
| G3_1 | dgr | 8 | 64 | 0 | 0 | 64 | 0 | 0/3099 | 0/6 | 0/8 |
| G3_2_1 | dgr | 9 | 72 | 0 | 0 | 72 | 0 | 0/3600 | 0/10 | 0/9 |
| G3_2_2 | dgr | 1 | 4 | 4 | 0 | 0 | 0 | 206/206 | 7/7 | 1/1 |
| G3_3 | dgr | 8 | 16 | 0 | 0 | 16 | 0 | 0/790 | 0/15 | 6/8 |
| G3_4 | dgr | 4 | 16 | 12 | 4 | 0 | 0 | 641/739 | 0/7 | 2/4 |
| G3_5 | dgr | 20 | 40 | 0 | 2 | 38 | 0 | 2/1915 | 0/6 | 3/20 |
| G3_6_1 | dgr | 12 | 48 | 4 | 31 | 13 | 0 | 572/2400 | 0/9 | 12/12 |
| G3_6_2 | dgr | 4 | 16 | 0 | 0 | 16 | 0 | 0/604 | 0/5 | 2/4 |
| G3_7 | dgr | 4 | 32 | 24 | 8 | 0 | 0 | 1480/1601 | 1/7 | 4/4 |
| G3_8 | dgr | 6 | 6 | 0 | 0 | 6 | 0 | 0/287 | 0/6 | 5/6 |
| G3_9 | dgr | 2 | 2 | 2 | 0 | 0 | 0 | 100/100 | 0/5 | 1/2 |
| G3_10 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/9 | 1/1 |
| G3_10_Airlock | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/10 | 1/1 |
| G3_11 | dgr | 6 | 6 | 0 | 0 | 6 | 0 | 0/300 | 0/5 | 0/6 |
| G3_12 | dgr | 8 | 8 | 0 | 8 | 0 | 0 | 95/400 | 0/5 | 8/8 |
| G3_14 | dgr | 6 | 6 | 0 | 0 | 6 | 0 | 0/300 | 0/5 | 0/6 |
| G3_15 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| G3_16 | dgr | 6 | 9 | 0 | 0 | 9 | 0 | 0/445 | 0/5 | 0/6 |
| G3_17 | dgr | 1 | 4 | 4 | 0 | 0 | 0 | 200/200 | 0/5 | 1/1 |
| G4_town | town | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 15/15 | 1/1 |
| G4_ship | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| G4_1_1 | dgr | 8 | 20 | 0 | 0 | 20 | 0 | 0/981 | 0/8 | 8/8 |
| G4_1_2 | dgr | 4 | 16 | 0 | 0 | 16 | 0 | 0/752 | 0/5 | 0/4 |
| G4_2_1 | dgr | 8 | 64 | 0 | 0 | 64 | 0 | 0/3199 | 0/7 | 0/8 |
| G4_2_2 | dgr | 9 | 36 | 0 | 8 | 28 | 0 | 14/1783 | 0/6 | 0/9 |
| G4_3_1 | dgr | 8 | 26 | 0 | 0 | 26 | 0 | 0/1300 | 0/7 | 8/8 |
| G4_3_2 | dgr | 4 | 24 | 0 | 22 | 2 | 0 | 424/1202 | 2/5 | 0/4 |
| G4_4_1 | dgr | 1 | 2 | 0 | 0 | 2 | 0 | 0/100 | 0/5 | 0/1 |
| G4_4_2 | dgr | 1 | 2 | 0 | 0 | 2 | 0 | 0/100 | 0/5 | 0/1 |
| G4_4_3 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/5 | 0/1 |
| G4_5_1 | dgr | 4 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/8 | 4/4 |
| G4_5_2 | dgr | 5 | 5 | 5 | 0 | 0 | 0 | 245/245 | 0/6 | 0/5 |
| G4_6 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| G4_7 | dgr | 8 | 48 | 0 | 0 | 48 | 0 | 0/2400 | 0/7 | 8/8 |
| G4_8a | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 20/50 | - | 0/1 |
| G4_8b | dgr | 3 | 5 | 0 | 0 | 5 | 0 | 0/250 | 0/6 | 0/3 |
| G4_9_ | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| G4_10 | dgr | 5 | 5 | 0 | 0 | 5 | 0 | 0/250 | 0/6 | 0/5 |
| G4_11_1a | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/6 | 0/1 |
| G4_11_1b | dgr | 4 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/5 | 0/4 |
| G4_11_2 | dgr | 3 | 12 | 0 | 12 | 0 | 0 | 255/600 | 0/5 | 0/3 |
| G4_12 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 41/41 | - | 0/1 |
| G4_13 | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/4 | 0/1 |
| G4_14 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/36 | - | 0/1 |
| P1_Town | town | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 7/7 | 1/1 |
| P1_1 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/9 | 0/1 |
| P1_2 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/47 | 0/6 | 0/1 |
| P1_3 | tgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/6 | 0/1 |
| P1_4 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/7 | 0/1 |
| P1_5 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/7 | 1/1 |
| P1_6 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/7 | 0/1 |
| P2_Town | town | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 6/6 | 1/1 |
| P2_1 | tgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/39 | 0/6 | 0/1 |
| P2_2 | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 22/50 | 0/5 | 0/1 |
| P2_3 | dgr | 2 | 4 | 0 | 0 | 4 | 0 | 0/200 | 0/5 | 2/2 |
| P2_4 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| P2_5 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 58/58 | 8/8 | 1/1 |
| P2_6 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/5 | 0/1 |
| P2_7 | dgr | 4 | 4 | 4 | 0 | 0 | 0 | 201/201 | 1/6 | 4/4 |
| P3_Town | town | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 6/6 | 1/1 |
| P3_1 | dgr | 4 | 4 | 0 | 0 | 4 | 0 | 0/199 | 0/5 | 0/4 |
| P3_2 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/5 | 1/1 |
| P3_3 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/5 | 0/1 |
| P3_4 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/5 | 1/1 |
| P3_5 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/5 | 0/1 |
| P3_6 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/49 | 0/8 | 0/1 |
| P3_7 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/6 | 1/1 |
| MapLeaguePortal | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| MapVoidReliquary | dgr | 1 | 4 | 4 | 0 | 0 | 0 | 201/201 | 1/1 | 1/1 |
| MapRustbowl | dgr | 4 | 32 | 3 | 29 | 0 | 0 | 1511/1576 | 0/8 | 4/4 |
| MapBackwash | tgr | 8 | 64 | 1 | 23 | 32 | 8 | 1145/2680 | 0/2 | 3/8 |
| MapBurialBog | dgr | 3 | 24 | 0 | 0 | 24 | 0 | 0/1200 | 0/5 | 3/3 |
| MapInferno | tgr | 4 | 32 | 2 | 22 | 8 | 0 | 1156/1594 | - | 0/4 |
| MapWetlands | tgr | 4 | 32 | 1 | 31 | 0 | 0 | 1504/1571 | 0/3 | 0/4 |
| MapBloomingField | dgr | 4 | 16 | 0 | 16 | 0 | 0 | 701/800 | 0/11 | 0/4 |
| MapCrimsonShores | tgr | 12 | 96 | 6 | 74 | 16 | 0 | 3592/4700 | 0/1 | 0/12 |
| MapCenotes | dgr | 4 | 16 | 0 | 0 | 16 | 0 | 0/788 | 0/4 | 0/4 |
| MapSavanna | dgr | 8 | 16 | 0 | 7 | 9 | 0 | 10/800 | 0/13 | 8/8 |
| MapFortress | dgr | 5 | 20 | 1 | 15 | 4 | 0 | 709/999 | 1/8 | 5/5 |
| MapPenitentiary | dgr | 5 | 5 | 0 | 5 | 0 | 0 | 180/250 | 0/6 | 5/5 |
| MapBloodwood | dgr | 4 | 32 | 5 | 27 | 0 | 0 | 1523/1592 | 0/3 | 2/4 |
| MapSandspit | dgr | 4 | 32 | 4 | 28 | 0 | 0 | 1401/1600 | 0/5 | 0/4 |
| MapForge | dgr | 4 | 4 | 0 | 1 | 3 | 0 | 2/200 | 0/13 | 4/4 |
| MapSulphuricCaverns | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/10 | 0/1 |
| MapMire | dgr | 4 | 32 | 2 | 30 | 0 | 0 | 1506/1585 | 0/7 | 3/4 |
| MapAugury | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 379/400 | 0/8 | 1/1 |
| MapWoodland | tgr | 6 | 48 | 4 | 44 | 0 | 0 | 2280/2380 | 0/10 | 6/6 |
| MapSump | dgr | 4 | 32 | 4 | 28 | 0 | 0 | 1533/1601 | 1/7 | 4/4 |
| MapWillow | dgr | 4 | 30 | 4 | 26 | 0 | 0 | 1422/1478 | 0/12 | 0/4 |
| MapHive | dgr | 1 | 7 | 0 | 0 | 7 | 0 | 0/323 | 0/4 | 0/1 |
| MapHeadland | dgr | 6 | 48 | 4 | 44 | 0 | 0 | 2261/2400 | 0/5 | 6/6 |
| MapLoftySummit | dgr | 1 | 8 | 1 | 7 | 0 | 0 | 383/399 | 0/6 | 1/1 |
| MapNecropolis | dgr | 12 | 12 | 0 | 1 | 11 | 0 | 1/588 | 0/5 | 0/12 |
| MapCrypt | dgr | 12 | 96 | 12 | 76 | 8 | 0 | 4008/4587 | 0/6 | 10/12 |
| MapHiddenGrotto | dgr | 4 | 32 | 0 | 32 | 0 | 0 | 366/1600 | 0/10 | 0/4 |
| MapSteamingSprings | dgr | 6 | 48 | 0 | 48 | 0 | 0 | 599/2399 | 0/10 | 6/6 |
| MapSeepage | dgr | 2 | 16 | 3 | 13 | 0 | 0 | 606/629 | 0/14 | 0/2 |
| MapRiverside | dgr | 4 | 32 | 2 | 30 | 0 | 0 | 1294/1383 | 0/11 | 0/4 |
| MapRavine | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/399 | 0/10 | 0/1 |
| MapSpiderWoods | dgr | 5 | 40 | 0 | 37 | 3 | 0 | 168/1835 | 0/8 | 3/5 |
| MapMarrow | dgr | 4 | 32 | 0 | 0 | 32 | 0 | 0/1581 | 0/14 | 4/4 |
| MapGrimhaven | dgr | 2 | 16 | 1 | 15 | 0 | 0 | 766/800 | 0/8 | 0/2 |
| MapVaalVillage | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/329 | - | 0/1 |
| MapSlick | dgr | 2 | 16 | 1 | 15 | 0 | 0 | 494/800 | 0/11 | 0/2 |
| MapVaalCity | dgr | 3 | 24 | 1 | 23 | 0 | 0 | 1132/1200 | 0/1 | 0/3 |
| MapSteppe | dgr | 4 | 32 | 5 | 27 | 0 | 0 | 1534/1600 | 0/12 | 2/4 |
| MapRockpools | dgr | 1 | 8 | 0 | 4 | 0 | 4 | 180/190 | 0/6 | 0/1 |
| MapCreek | dgr | 4 | 32 | 0 | 1 | 31 | 0 | 28/1492 | 0/8 | 0/4 |
| MapBazaar | dgr | 6 | 48 | 0 | 46 | 2 | 0 | 354/2400 | - | 0/6 |
| MapDerelictMansion | dgr | 8 | 32 | 0 | 0 | 32 | 0 | 0/1600 | 0/2 | 0/8 |
| MapOutlands | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/400 | - | 0/1 |
| MapBastille | dgr | 3 | 6 | 1 | 5 | 0 | 0 | 287/300 | 0/13 | 3/3 |
| MapDecay | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 287/401 | 1/9 | 1/1 |
| MapMineshaft | dgr | 1 | 8 | 1 | 7 | 0 | 0 | 386/400 | 0/3 | 1/1 |
| MapDeserted | dgr | 4 | 32 | 0 | 12 | 20 | 0 | 12/1600 | 0/3 | 4/4 |
| MapOasis | dgr | 1 | 8 | 1 | 7 | 0 | 0 | 378/400 | 0/9 | 1/1 |
| MapSunTemple | dgr | 8 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/3 | 8/8 |
| MapChannel | dgr | 4 | 16 | 0 | 16 | 0 | 0 | 682/801 | 1/8 | 4/4 |
| MapVaalFoundry | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 48/50 | - | 1/1 |
| MapVaalFactory | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/6 | 1/1 |
| MapPrecursorTower | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapPrecursorTowerDesert | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 1/3 | 1/1 |
| MapPrecursorTowerForest | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/1 | 1/1 |
| MapPrecursorTowerGrass | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/4 | 1/1 |
| MapPrecursorTowerMountain | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapPrecursorTowerSwamp | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/2 | 1/1 |
| MapLostTowers | dgr | 6 | 48 | 0 | 0 | 48 | 0 | 0/2372 | 0/2 | 4/6 |
| MapSwampTower | dgr | 12 | 24 | 0 | 0 | 24 | 0 | 0/1200 | 0/2 | 0/12 |
| MapAlpineRidge | dgr | 1 | 2 | 0 | 0 | 2 | 0 | 0/100 | 0/2 | 0/1 |
| MapMesa | dgr | 4 | 8 | 0 | 0 | 8 | 0 | 0/400 | - | 0/4 |
| MapBluff | dgr | 6 | 12 | 3 | 9 | 0 | 0 | 388/399 | 0/2 | 2/6 |
| MapKarst | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/400 | - | 0/1 |
| MapTerrace | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| MapPlantation | dgr | 4 | 16 | 0 | 0 | 16 | 0 | 0/800 | 0/5 | 0/4 |
| MapAzmerianRanges | dgr | 1 | 4 | 0 | 0 | 4 | 0 | 0/200 | 0/6 | 0/1 |
| MapDeforestation | tgr | 4 | 32 | 6 | 26 | 0 | 0 | 1477/1592 | 0/7 | 3/4 |
| MapEpitaph | dgr | 4 | 32 | 0 | 32 | 0 | 0 | 1228/1600 | 0/16 | 3/4 |
| MapAridPlains | tgr | 8 | 32 | 0 | 29 | 3 | 0 | 1345/1599 | 0/3 | 0/8 |
| MapSinterRift | tgr | 4 | 4 | 0 | 2 | 2 | 0 | 94/199 | 1/12 | 3/4 |
| MapPit | dgr | 4 | 32 | 6 | 26 | 0 | 0 | 1546/1600 | 0/4 | 0/4 |
| MapReservoir | dgr | 2 | 8 | 0 | 8 | 0 | 0 | 376/400 | 0/7 | 2/2 |
| MapGeode | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| MapRupture | dgr | 6 | 6 | 1 | 5 | 0 | 0 | 290/301 | 1/5 | 6/6 |
| MapOvergrown | dgr | 6 | 6 | 2 | 4 | 0 | 0 | 289/300 | 0/1 | 0/6 |
| MapCaldera | tgr | 8 | 8 | 2 | 6 | 0 | 0 | 386/399 | 0/5 | 0/8 |
| MapChasm | dgr | 4 | 4 | 0 | 4 | 0 | 0 | 163/171 | - | 3/4 |
| MapSpring_ | dgr | 1 | 2 | 1 | 1 | 0 | 0 | 97/101 | 1/7 | 1/1 |
| MapSevenWaters | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/393 | 0/9 | 0/1 |
| MapCavernCity | dgr | 4 | 4 | 0 | 4 | 0 | 0 | 194/200 | 0/3 | 4/4 |
| MapTrenches | dgr | 1 | 8 | 1 | 7 | 0 | 0 | 383/400 | 0/7 | 1/1 |
| MapFrozenFalls | dgr | 4 | 32 | 0 | 0 | 32 | 0 | 0/1548 | 0/2 | 0/4 |
| MapSwarm | dgr | 2 | 16 | 3 | 13 | 0 | 0 | 765/792 | - | 2/2 |
| MapWaywardIsle | dgr | 5 | 5 | 0 | 0 | 5 | 0 | 0/250 | 0/2 | 0/5 |
| MapStronghold | dgr | 5 | 5 | 0 | 5 | 0 | 0 | 232/250 | 0/4 | 0/5 |
| MapPort | dgr | 6 | 24 | 0 | 14 | 10 | 0 | 39/1200 | 0/6 | 0/6 |
| MapCliffside | dgr | 3 | 3 | 0 | 1 | 2 | 0 | 1/150 | 0/8 | 0/3 |
| MapSinkhole | dgr | 1 | 2 | 0 | 0 | 2 | 0 | 0/100 | 0/5 | 0/1 |
| MapFlotsam | dgr | 9 | 9 | 0 | 3 | 6 | 0 | 8/449 | 0/2 | 0/9 |
| MapMortuary | dgr | 1 | 4 | 0 | 4 | 0 | 0 | 6/200 | - | 0/1 |
| MapOrbalasCrossing | dgr | 4 | 32 | 0 | 0 | 32 | 0 | 0/1600 | - | 4/4 |
| MapMonument | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 378/400 | - | 1/1 |
| MapGreenhouse | dgr | 4 | 8 | 2 | 6 | 0 | 0 | 383/401 | 1/8 | 4/4 |
| MapSlash | tgr | 6 | 6 | 0 | 4 | 2 | 0 | 160/295 | 0/4 | 5/6 |
| MapSanctuary | dgr | 5 | 5 | 1 | 4 | 0 | 0 | 244/250 | 0/9 | 5/5 |
| MapPrecipice | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 120/398 | - | 0/1 |
| MapArroyo | dgr | 4 | 4 | 0 | 4 | 0 | 0 | 192/200 | - | 4/4 |
| MapCanyon | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 50/51 | 1/8 | 1/1 |
| MapOrnateChambers | dgr | 1 | 4 | 0 | 0 | 4 | 0 | 0/200 | 0/18 | 1/1 |
| MapIceCave | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 311/400 | 0/11 | 1/1 |
| MapRugosa | dgr | 4 | 12 | 0 | 12 | 0 | 0 | 558/600 | 0/4 | 3/4 |
| MapRiverhold | dgr | 1 | 4 | 0 | 0 | 4 | 0 | 0/200 | 0/21 | 0/1 |
| MapRazedFields | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/13 | 0/1 |
| MapMezzanine | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 46/50 | - | 1/1 |
| MapVaalVault | dgr | 4 | 4 | 1 | 3 | 0 | 0 | 193/200 | 0/2 | 4/4 |
| MapSnowfall | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/47 | 0/1 | 0/1 |
| MapExcavation | dgr | 4 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/12 | 0/4 |
| MapUniqueUntaintedParadise | dgr | 3 | 24 | 24 | 0 | 0 | 0 | 1204/1204 | 4/4 | 3/3 |
| MapUniqueVault | dgr | 2 | 16 | 8 | 0 | 0 | 8 | 400/400 | - | 0/2 |
| MapUniqueCastaway | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/1 | 0/1 |
| MapUniqueMegalith | dgr | 4 | 32 | 0 | 0 | 32 | 0 | 0/1525 | 0/1 | 0/4 |
| MapUniqueLake | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| MapUniqueSelenite | dgr | 1 | 8 | 4 | 0 | 4 | 0 | 200/400 | - | 0/1 |
| MapUniqueMerchant01_Chimeral | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| MapUniqueMerchant01_Oasis | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| MapUniqueMerchant01_Sandswept | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| MapUniqueMerchant02_Crimson | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| MapUniqueMerchant02_Farmland | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| Map_HildaCampsite | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapUniqueMerchant03_Beach | dgr | 1 | 6 | 0 | 0 | 6 | 0 | 0/300 | - | 1/1 |
| MapUniqueMerchant03_Tropical | dgr | 1 | 6 | 6 | 0 | 0 | 0 | 300/300 | - | 1/1 |
| MapUniqueMerchant03_Raft | dgr | 1 | 6 | 6 | 0 | 0 | 0 | 302/302 | 2/2 | 1/1 |
| MapUniqueMerchant04_PirateShip | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapUniqueWildwood | tgr | 5 | 40 | 0 | 0 | 40 | 0 | 0/1759 | - | 0/5 |
| MapUniqueFreight_ | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| MapUberBoss_IronCitadel | dgr | 2 | 2 | 0 | 2 | 0 | 0 | 97/100 | - | 2/2 |
| MapUberBoss_CopperCitadel | dgr | 2 | 2 | 0 | 0 | 2 | 0 | 0/100 | - | 2/2 |
| MapUberBoss_StoneCitadel | dgr | 2 | 2 | 0 | 2 | 0 | 0 | 93/100 | - | 2/2 |
| MapUberBoss_JadeCitadel | dgr | 3 | 12 | 0 | 0 | 12 | 0 | 0/600 | 0/4 | 0/3 |
| MapUberBoss_Monolith | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 55/55 | 5/11 | 1/1 |
| MapUberBoss_FallenStar | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 52/52 | 2/9 | 1/1 |
| MapUberBoss_Delirium | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/8 | 1/1 |
| MapUberBoss_Ritual | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 1/7 | 1/1 |
| MapUberBoss_Divinity | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 57/57 | 7/15 | 1/1 |
| MapUberBoss_IronCitadel_Quest | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 52/52 | 2/3 | 1/1 |
| MapUberBoss_CopperCitadel_Quest | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| MapUberBoss_StoneCitadel_Quest | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/5 | 1/1 |
| ExpeditionLogBook_Peninsula | dgr | 1 | 8 | 0 | 4 | 4 | 0 | 98/359 | 0/3 | 0/1 |
| ExpeditionLogBook_Tropical | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 118/400 | - | 0/1 |
| ExpeditionLogBook_Tundra | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 92/400 | 0/1 | 0/1 |
| ExpeditionLogBook_Atoll | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | 0/3 | 1/1 |
| ExpeditionLogBook_Digsite | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/400 | - | 0/1 |
| ExpeditionLogBook_Reef | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 102/400 | 0/2 | 0/1 |
| ExpeditionLogBook_Wastes | dgr | 1 | 4 | 4 | 0 | 0 | 0 | 172/172 | 0/1 | 1/1 |
| ExpeditionLogBook_Heath | dgr | 1 | 8 | 0 | 2 | 2 | 4 | 50/197 | - | 0/1 |
| ExpeditionLogBook_Basin | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/372 | 0/2 | 0/1 |
| ExpeditionLogBook_Gully | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/400 | - | 0/1 |
| ExpeditionLogBook_Prairie | dgr | 1 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/3 | 0/1 |
| ExpeditionSubArea_Cavern | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 236/400 | - | 1/1 |
| ExpeditionSubArea_Kalguur | dgr | 4 | 32 | 32 | 0 | 0 | 0 | 1499/1499 | - | 2/4 |
| ExpeditionSubArea_Kalguur_Act1 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 52/52 | 2/4 | 1/1 |
| ExpeditionSubArea_Kalguur_Act2 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/3 | 1/1 |
| ExpeditionSubArea_Kalguur_Act3 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 49/49 | 0/4 | 1/1 |
| ExpeditionSubArea_Kalguur_Act4 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| ExpeditionSubArea_OlrothBoss | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 1/1 | 1/1 |
| ExpeditionSubArea_UhtredBoss | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 1/1 | 1/1 |
| ExpeditionSubArea_VoranaBoss | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 1/1 | 1/1 |
| ExpeditionSubArea_MedvedBoss | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 1/1 | 1/1 |
| ExpeditionSubArea_BlackKnightBoss | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| ExpeditionSubArea_Precursor | dgr | 1 | 2 | 2 | 0 | 0 | 0 | 100/100 | - | 1/1 |
| ExpeditionSubArea_Lair | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| ExpeditionSubArea_Siren | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| ExpeditionSubArea_Volcano | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 297/400 | - | 1/1 |
| ExpeditionSubArea_Vein | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| ExpeditionSubArea_Laboratory | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| ExpeditionSubArea_Sanctuary | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| ExpeditionSubArea_Fissure | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| ExpeditionSubArea_Catacomb | dgr | 1 | 8 | 0 | 8 | 0 | 0 | 124/400 | - | 1/1 |
| ExpeditionSubArea_Pond | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| ExpeditionLeagueBoss | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| Delirium_Act1Town_Quest | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 1/1 | 1/1 |
| Delirium_Act1Town | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 55/55 | 5/5 | 1/1 |
| Delirium_Act2Reveal | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| Delirium_Act3Town | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 1/1 |
| Delirium_VastiriOutskirts | dgr | 1 | 8 | 8 | 0 | 0 | 0 | 400/400 | - | 1/1 |
| Delirium_HungerBoss | dgr | 1 | 4 | 4 | 0 | 0 | 0 | 200/200 | 0/5 | 0/1 |
| BreachDomain_01 | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 35/50 | - | 0/1 |
| ChayulaLeague | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 52/52 | 2/2 | 1/1 |
| ChayulaLeague_Tower | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| ChayulaLeague_TowerBoss | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| RitualLeagueBoss | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/1 | 1/1 |
| Abyss_Intro | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 15/52 | 2/9 | 0/1 |
| Abyss_Hub | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 105/105 | 55/55 | 1/1 |
| Abyss_Depths1 | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 17/45 | 3/14 | 0/1 |
| Abyss_Depths2 | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 9/52 | 2/7 | 0/1 |
| Abyss_Depths3 | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 16/55 | 5/16 | 0/1 |
| Abyss_Boss1 | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 10/41 | 0/1 | 0/1 |
| Abyss_Boss2 | dgr | 1 | 1 | 0 | 1 | 0 | 0 | 9/51 | 1/2 | 0/1 |
| Abyss_Pinnacle | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/9 | 1/1 |
| BossRush_Area1 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/49 | - | 0/1 |
| BossRush_Area2 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| BossRush_Area3 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | - | 0/1 |
| G_Endgame_Town | town | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 9/10 | 1/1 |
| IncursionTemplePresent | dgr | 1 | 1 | 0 | 0 | 0 | 1 | 0/0 | - | 0/1 |
| IncursionTemple | dgr | 1 | 1 | 0 | 0 | 0 | 1 | 0/0 | 0/4 | 0/1 |
| IncursionHub | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 53/53 | 3/3 | 1/1 |
| IncursionHubEndgame | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 51/51 | 1/1 | 1/1 |
| DruidTrailer | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| MapMothersoul_Male | dgr | 1 | 4 | 1 | 3 | 0 | 0 | 167/175 | 0/2 | 1/1 |
| MapMothersoul_Female | dgr | 1 | 4 | 0 | 4 | 0 | 0 | 188/197 | 0/1 | 1/1 |
| MapMothersoul_Male_Quest | dgr | 1 | 4 | 4 | 0 | 0 | 0 | 162/162 | 0/3 | 1/1 |
| MapMothersoul_Female_Quest | dgr | 1 | 4 | 4 | 0 | 0 | 0 | 199/199 | 0/6 | 1/1 |
| MapUniqueReactor_01 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/3 | 0/1 |
| MapUniqueReactor_02 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/5 | 0/1 |
| MapUniqueReactor_03 | dgr | 1 | 1 | 0 | 0 | 1 | 0 | 0/50 | 0/3 | 0/1 |
| MapUniqueReactor_04 | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | 0/5 | 0/1 |
| PrecursorVault | dgr | 1 | 1 | 1 | 0 | 0 | 0 | 50/50 | - | 1/1 |
| EndgameDoodad_MeteorCrater | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| EndgameDoodad_BeastGroundZero | fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0/0 | - | 0/0 |
| MapUniqueInitialTower | dgr | 4 | 8 | 0 | 0 | 8 | 0 | 0/400 | 0/4 | 0/4 |

</details>

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
| 🟡 | [Game update compatibility: signature table, `cargo xtask check-build`, `modules/protocol/versions/<version>.toml`, unknown-client refusal](docs/guides/new-game-version.md) | WP A2-A4: DLL addresses and offsets from byte signatures; check-build (signatures, offsets, opcode fingerprints, strings, game data) with report and exit codes; one version file for xtask, server and launcher; the server refuses an unknown `client_hash` once `hashes_confirmed`; the client_hash is derived from the code, the in-game check must confirm it; renumbered opcodes are not derived |

### Launcher

Docs: [docs/guides/launcher.md](docs/guides/launcher.md)

| | Item | Note |
|---|---|---|
| ✅ | [Poe2Launcher repo (Tauri 2 + Svelte)](docs/guides/launcher.md) | WP5.1; submodule modules/launcher (nests Poe2ClientHook); Rust + vitest tests, CI on Linux and Windows |
| ✅ | [Releases and auto-update (AppImage, NSIS installer, signed latest.json)](docs/guides/launcher.md#releasing) | WP E1-E3, 2026-10-21: v0.1.0 and v0.1.1 released by the tag workflow; on Linux an installed 0.1.0 found, downloaded, verified and installed 0.1.1 and restarted as 0.1.1; the Windows installer and its update are built in CI but untested (no Windows machine) |
| ✅ | [Server profiles and account login through the web API](docs/guides/launcher.md#the-flow) | profiles, register, login (kind launcher), token in the OS keyring, logout revokes; tested with a mock keyring and a real server |
| ✅ | [Game-token exchange for auto-login (`POST /api/v1/game-token`)](docs/architecture/web-api.md#endpoints) | launcher token -> one-off game_remember token (the game's rolling login would revoke the launcher token) |
| ✅ | [Install 0.5.5e from Steam depots](docs/guides/launcher.md#install-in-detail) | DepotDownloader 3.4.0 pinned; the user's end-to-end test of launcher v0.4.0 on Linux (2026-10-23) installed the game from the Steam depots and passed; Windows untested |
| ✅ | [Deploy and launch via poe-hook-deploy](docs/guides/launcher.md#play-in-detail) | the user's end-to-end test on Linux (2026-10-23): hook download and deploy, Host a server (update from server-v0.1.0 to 0.2.1), Play, web admin, restart all passed; Windows path untested |
| ✅ | [Mod install and ordering](docs/guides/launcher.md#mods) | files and poe-hook-mods/state.toml; the DLL loader reads them (WP C4) |

### Web Admin

Docs: [docs/architecture/web-api.md](docs/architecture/web-api.md)

| | Item | Note |
|---|---|---|
| ✅ | [Web API crate (axum, in poe-server)](docs/architecture/web-api.md) | WP4.1; register/login/password, tokens, 20 endpoints; tested with oneshot and a smoke run on shifted ports |
| ✅ | [OpenAPI spec (utoipa) for the SPA and launcher types](modules/server/docs/openapi.json) | modules/server/docs/openapi.json, `cargo xtask openapi` |
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
| ✅ | [Client mod loader in the DLL (mod.toml, mod ABI 1, sample mod)](modules/client-hook/docs/mod-manifest.md) | loads enabled poe-hook-mods/<id> DLLs in state.toml order, ABI check, never in official mode; [data] overlays not applied yet |
| ✅ | [Mod manifest v2 and packages (side client/server/both, verified zips)](modules/client-hook/docs/mod-manifest.md) | poe-hook-mods: manifest v1+v2, [client]/[server]/[[commands]], verify_package; Poe2ModSDK repo with poe-mod tool and examples |
| ✅ | [Server mod host (native libraries, poe-mod-api, data-tier commands)](docs/architecture/mod-host.md) | modules/server/crates/poe-mod-host: mods/ + state.toml, ABI and game-version checks, events, submitted commands, poe-server mod CLI; never unloaded; config_defaults and server opcodes overlays not applied yet; in game: !hello from the SDK example |
| ✅ | [Mod advertisement and downloads](docs/architecture/mod-host.md) | GET /api/v1/server `mods` (client and both mods, exact version + SHA-256), GET /api/v1/mods/{id}/{version}.zip |
| ✅ | [Login mod check](docs/architecture/mod-host.md) | the hook reports its loaded client mods on the side channel; the server refuses missing or wrong versions, extra client mods only with [mods] allow_extra_client_mods; verified in game |
| ✅ | [Launcher mod sync and mod browser](docs/guides/launcher.md) | per-server mod sets, Off/Ask/Auto, verified cache, Poe2Mods index; not yet run against a real server with mods |
| ✅ | [Web admin Mods page](docs/architecture/web-api.md) |  |
| ⬜ | Client loose-file override hook and packet hooks |  |
<!-- tracker:end -->
