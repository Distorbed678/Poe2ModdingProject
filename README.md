# Poe2ModdingProject

A private server for **Path of Exile 2 (client 0.5.5e)**, written in Rust and designed for future client and server modding. It is a personal, educational project: it never touches the official Steam install, and on official servers the client hook only observes and logs (read-only).

## How the repos fit together

This repo holds the server. The shared protocol code, the client hook and the web admin live in their own repos, included as git submodules. All repos are private, under `Distorbed678`.

| Repo | Where | Role |
|---|---|---|
| **Poe2ModdingProject** (this repo) | `/` | `servers/` (login, instance, and the `poe-server` launcher that runs both plus the web API), `crates/` (config, data, store, accounts, cluster, web API, terrain generator, simulation), `xtask`, `docs/`, `scripts/` |
| [Poe2Protocol](https://github.com/Distorbed678/Poe2Protocol) | `modules/protocol` | `poe-proto`, `poe-net` (codec and ciphers) and the opcode tables `opcodes/<version>/*.toml`, used by the server and the client hook |
| [Poe2ClientHook](https://github.com/Distorbed678/Poe2ClientHook) | `modules/client-hook` | The client DLLs (`version.dll` loader, hot-reloadable debug core with the in-game overlay) and `poe-hook-deploy`; carries Poe2Protocol as a nested submodule |
| [Poe2Launcher](https://github.com/Distorbed678/Poe2Launcher) | `modules/launcher` | Player launcher (Tauri 2 + Svelte 5): server profiles and login (keyring), installs 0.5.5e from Steam depots, deploys the hook and starts the game, manages mods ([docs/guides/launcher.md](docs/guides/launcher.md)); carries Poe2ClientHook as a nested submodule |
| [Poe2WebAdmin](https://github.com/Distorbed678/Poe2WebAdmin) | `modules/web-admin` | Web admin site (Svelte 5 + Vite + TypeScript) on top of the server's web API; build it and set `[web] static_dir = "modules/web-admin/dist"` ([docs/guides/setup.md](docs/guides/setup.md#web-admin-build), [docs/architecture/web-api.md](docs/architecture/web-api.md)) |

Other folders: `third_party/` (downloaded tools, git-ignored), `runtime/` (server data and logs, git-ignored), `dumps/` (datamined outputs, git-ignored; [dumps/README.md](dumps/README.md)), `GameFiles/` (your copy of the 0.5.5e client, never in git). Every crate and server: [docs/architecture/crates.md](docs/architecture/crates.md).

## Quick start

Needs Linux with rustup, Steam with Proton Experimental, and a copy of the 0.5.5e client in `GameFiles/`. The full requirements, options and the unattended test tools are in [docs/guides/setup.md](docs/guides/setup.md).

The steps are in [docs/guides/setup.md](docs/guides/setup.md): [clone](docs/guides/setup.md#clone) with submodules, [fetch the tools](docs/guides/setup.md#tools), [build](docs/guides/setup.md#build-and-test), [start the server and deploy the client](docs/guides/setup.md#run) into `GameFiles/`, and [play](docs/guides/setup.md#playing-from-steam-non-steam-game-shortcut) or launch the client through Proton. `revert-client` restores `GameFiles/`.

Server commands and admins: [docs/guides/server-admin.md](docs/guides/server-admin.md).

## Documentation

Start at [docs/index.md](docs/index.md): guides, architecture, the wire protocol, reverse-engineering notes, status and plans. The rules for working in this repo are in [CLAUDE.md](CLAUDE.md).

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
| [Multiplayer](#multiplayer) | `████████░░ 75%` | 4/6 |
| [Chat/Commands](#chatcommands) | `██████████ 100%` | 4/4 |
| [Terrain](#terrain) | `███░░░░░░░ 25%` | 2/14 |
| [Inventory](#inventory) | `██░░░░░░░░ 17%` | 0/3 |
| [Monsters/Skills](#monstersskills) | `███░░░░░░░ 30%` | 1/5 |
| [Client Hook](#client-hook) | `████████░░ 83%` | 4/6 |
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
| ✅ | [Duplicate login ends the old session](docs/architecture/accounts.md) | LoginSessions::supersede; verified with two real clients (plan 1 WP3.7, docs/guides/hosting.md) |
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
| ✅ | [Remote client login (trusted_servers in the DLL)](docs/guides/hosting.md) | trusted servers are not blocked or latched and get the crypto bypass; verified with two clients over the LAN IP (WP3.7); a second physical machine is untested |
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
| 🟡 | [Server-side generator, act 1: 25 of 33 sweep vectors match](docs/status/terrain.md) | Town and G1_1, 2, 3, 6, 7, 8, 9 match 3/3; all areas: the Terrain matrix below |
| 🟡 | [G1_12: grid overlap (map size 169x85 vs 174x87)](docs/status/terrain.md) | all 2385 terrain draws match; layout differs |
| 🟡 | [G1_4: overlay phase 2 (seed 1003)](docs/status/terrain.md) | 2/3 seeds match |
| ❌ | [G1_5: edge routing ("could not tilise edge 14")](docs/status/terrain.md) | 0/3 seeds |
| ❌ | [G1_11: height-change (cliff) edge types, woodscliff_height.et chirality](docs/status/terrain.md) | 0/3 seeds |
| ❌ | [G1_13 to G1_15: client traces, then port](docs/status/terrain.md) | the user records the traces |
| ❌ | [World-map pins (natural topology choice)](docs/status/terrain.md) | forced topology covers it for now |
| ❌ | [Doodads and the doodad hash](docs/status/terrain.md) |  |
| ✅ | [Per-area x topology x orientation matrix (cargo xtask terrain-matrix)](docs/plans/terrain-all-areas.md) | 2026-10-21: 442 WorldAreas rows (10 without topologies), 1141 topologies, 4508 combinations: 18 match, 1 fail, 4489 untested; generator probe: 701 of 1141 topologies run at seed 1001, 284 of 432 areas run for every topology (no hash check); table below the Terrain items |
| ⬜ | [Verification at scale: forced-layout sweeps and bulk vectors (WP7.2, WP7.3)](docs/plans/terrain-all-areas.md#2-verification-at-scale) | every topology once is about 1141 client layouts at 20 s each; needs unattended client runs |
| ❌ | [Dungeon height-change and virtual edge types (chirality), unlocks 44 areas](docs/plans/terrain-all-areas.md#4-unported-systems-ranked-by-areas-unlocked) | 149 topologies stop at the guard in dungeon.rs; G1_11 is the first test |
| ❌ | [3x3 door spots (0x14209dc90), unlocks 27 areas; FileGroups and nested subgraphs, 11 areas](docs/plans/terrain-all-areas.md#4-unported-systems-ranked-by-areas-unlocked) |  |
| ⬜ | [Collision and walkability output for server movement](docs/plans/terrain-all-areas.md#4-unported-systems-ranked-by-areas-unlocked) | poe-world emits the PWG1 cell grid that poe_game::grid::Grid reads (GridSource); checked against the DLL's grids; the client's upload covers it until then |

### Terrain matrix

Generated by `cargo xtask terrain-matrix` into [docs/status/terrain-matrix.tsv](docs/status/terrain-matrix.tsv) (area x topology x orientation; plan: [docs/plans/terrain-all-areas.md](docs/plans/terrain-all-areas.md)). **442 areas (10 without topologies), 1141 topologies, 4508 combinations: 18 match, 1 fail, 4489 untested**. Probe = the generator runs to the end for one seed (no hash check).

| Act | Areas | Topologies | Combinations | Match | Fail | Untested | Probe ok |
|---|---|---|---|---|---|---|---|
| 1 | 111 | 260 | 966 | 18 | 1 | 947 | 183/260 |
| 2 | 45 | 141 | 543 | 0 | 0 | 543 | 102/141 |
| 3 | 28 | 117 | 361 | 0 | 0 | 361 | 65/117 |
| 4 | 28 | 88 | 301 | 0 | 0 | 301 | 37/88 |
| 6 | 24 | 31 | 33 | 0 | 0 | 33 | 18/31 |
| 10 | 206 | 504 | 2304 | 0 | 0 | 2304 | 296/504 |
| **all** | 442 | 1141 | 4508 | 18 | 1 | 4489 | 701/1141 |

<details><summary>Per area (442 rows)</summary>

| Area | Type | Topologies | Combinations | Match | Fail | Untested | Vectors | Probe ok |
|---|---|---|---|---|---|---|---|---|
| NULL | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| CharacterSelect | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| PersonalHideout | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| GuildHideout__ | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| CurrentTown | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| HideoutCave | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutShipgraveyard | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutSpace | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArenaGraveyardTrio | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutPillarsOfArun | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutAtziriArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutInnocenceArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutShaperArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutKaomArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutDaressoArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutDominusArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutPietyArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutLunarisSolarisArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutKitavaArenaAct10 | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutDoomguard | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutSunspire | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutDarkwood | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArenaElder | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutChiyouSpring | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankAbyss | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankCity | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankClouds | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankDesert | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankDirt | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankGrass | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankSea | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankSnow | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutThaumaturgy | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutYaochi | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutHumanoidPet | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutRitualLeague | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArenaMaven | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutHasina | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutAzuriteCave | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutSynthesisHub | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutVeritaniaArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankBlack | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutTencentApocalypse | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutOlrothArena | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutLight | town | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| HideoutDark | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArenaBlackStar_ | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArenaInfiniteHunger | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutCosmicAtlas | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArenaEaterOfWorlds | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArenaSearingExarch | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBeaconOfSalvation | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutCanopy | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutDreadnought | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutFelled | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutShrine | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutLimestone | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutCanal | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutMonolith | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutTwisted | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutTranscendent | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutHourglass | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutChaos | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutSummit | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutRacetrack | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutShipwreck | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutFarmlands | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutPrison | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutShoreline | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArenaKulemak | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutGrinding | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArcaneIsle | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutSacrifice | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutVerdant | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankUrban | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankMountain | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankDesert2 | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankCrypt | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutArenaIncursion | town | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| HideoutSunkenPalace | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutColossalTitan | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankJungle | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankVolcanicCave | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutVampireManor | town | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| HideoutRemidusMonastery_ | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankIce | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutBlankFire | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| HideoutShip | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapHideoutFarmlands_Claimable | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapHideoutPrison_Claimable | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapHideoutShoreline_Claimable | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapHideoutFelled_Claimable | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapHideoutLimestone_Claimable | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapHideoutShrine_Claimable | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapHideoutCanal_Claimable | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Design | tgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Design_Lite | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| BlackTest | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Programming | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Programming_Lite | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G_login | town | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G1_WorldMap | dgr | 6 | 6 | 0 | 0 | 6 | - | 6/6 |
| G2_WorldMap | dgr | 8 | 8 | 0 | 0 | 8 | - | 0/8 |
| G3_WorldMap | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G4_WorldMap | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G6_WorldMap | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| TN_WorldMap | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G1_town | town | 1 | 1 | 1 | 0 | 0 | 2/2 | 1/1 |
| G1_1 | tgr | 5 | 10 | 2 | 0 | 8 | 3/3 | 3/5 |
| G1_2 | tgr | 18 | 18 | 1 | 0 | 17 | 3/3 | 16/18 |
| G1_3 | dgr | 4 | 32 | 2 | 0 | 30 | 3/3 | 4/4 |
| G1_4 | tgr | 12 | 48 | 1 | 0 | 47 | 2/3 | 10/12 |
| G1_5 | tgr | 8 | 32 | 0 | 0 | 32 | 0/3 | 5/8 |
| G1_6 | dgr | 4 | 32 | 2 | 0 | 30 | 3/3 | 4/4 |
| G1_7 | dgr | 12 | 96 | 3 | 0 | 93 | 3/3 | 11/12 |
| G1_8 | dgr | 12 | 96 | 3 | 0 | 93 | 3/3 | 12/12 |
| G1_9 | dgr | 12 | 96 | 3 | 0 | 93 | 3/3 | 12/12 |
| G1_10 | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| G1_11 | dgr | 12 | 96 | 0 | 0 | 96 | 0/3 | 0/12 |
| G1_12 | dgr | 12 | 96 | 0 | 1 | 95 | 0/1 | 12/12 |
| G1_13_1 | dgr | 16 | 128 | 0 | 0 | 128 | - | 0/16 |
| G1_13_2 | dgr | 12 | 37 | 0 | 0 | 37 | - | 0/12 |
| G1_14 | dgr | 6 | 6 | 0 | 0 | 6 | - | 0/6 |
| G1_15 | dgr | 16 | 16 | 0 | 0 | 16 | - | 0/16 |
| G2_town | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G2_1 | tgr | 8 | 64 | 0 | 0 | 64 | - | 0/8 |
| G2_2 | dgr | 8 | 16 | 0 | 0 | 16 | - | 8/8 |
| G2_3 | dgr | 12 | 12 | 0 | 0 | 12 | - | 0/12 |
| G2_3a | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G2_3s | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| G2_4_1 | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| G2_4_2 | dgr | 6 | 48 | 0 | 0 | 48 | - | 6/6 |
| G2_4_3 | dgr | 8 | 16 | 0 | 0 | 16 | - | 8/8 |
| G2_5_1 | dgr | 12 | 96 | 0 | 0 | 96 | - | 12/12 |
| G2_5_2 | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| G2_6 | dgr | 4 | 8 | 0 | 0 | 8 | - | 3/4 |
| G2_7 | dgr | 8 | 16 | 0 | 0 | 16 | - | 8/8 |
| G2_8 | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| G2_8a | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| G2_9_1 | dgr | 12 | 48 | 0 | 0 | 48 | - | 12/12 |
| G2_9_2 | dgr | 4 | 16 | 0 | 0 | 16 | - | 4/4 |
| G2_10_1 | dgr | 8 | 64 | 0 | 0 | 64 | - | 7/8 |
| G2_10_2 | dgr | 4 | 8 | 0 | 0 | 8 | - | 4/4 |
| G2_11 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G2_12 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G2_13 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Sanctum_1_Foyer_1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_1_Foyer_2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_1_Foyer_3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_2_Foyer_1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_2_Foyer_2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_2_Foyer_3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Sanctum_3_Foyer_1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_3_Foyer_2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_3_Foyer_3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_4 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Sanctum_4_Foyer_1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_4_Foyer_2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Sanctum_4_Foyer_3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G3_town | town | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G3_1 | dgr | 8 | 64 | 0 | 0 | 64 | - | 0/8 |
| G3_2_1 | dgr | 9 | 72 | 0 | 0 | 72 | - | 0/9 |
| G3_2_2 | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| G3_3 | dgr | 8 | 16 | 0 | 0 | 16 | - | 8/8 |
| G3_4 | dgr | 4 | 16 | 0 | 0 | 16 | - | 3/4 |
| G3_5 | dgr | 20 | 40 | 0 | 0 | 40 | - | 12/20 |
| G3_6_1 | dgr | 12 | 48 | 0 | 0 | 48 | - | 12/12 |
| G3_6_2 | dgr | 4 | 16 | 0 | 0 | 16 | - | 4/4 |
| G3_7 | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| G3_8 | dgr | 6 | 6 | 0 | 0 | 6 | - | 5/6 |
| G3_9 | dgr | 2 | 2 | 0 | 0 | 2 | - | 1/2 |
| G3_10 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G3_10_Airlock | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G3_11 | dgr | 6 | 6 | 0 | 0 | 6 | - | 0/6 |
| G3_12 | dgr | 8 | 8 | 0 | 0 | 8 | - | 8/8 |
| G3_14 | dgr | 6 | 6 | 0 | 0 | 6 | - | 0/6 |
| G3_15 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G3_16 | dgr | 6 | 9 | 0 | 0 | 9 | - | 0/6 |
| G3_17 | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| G4_town | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G4_ship | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G4_1_1 | dgr | 8 | 20 | 0 | 0 | 20 | - | 8/8 |
| G4_1_2 | dgr | 4 | 16 | 0 | 0 | 16 | - | 0/4 |
| G4_2_1 | dgr | 8 | 64 | 0 | 0 | 64 | - | 0/8 |
| G4_2_2 | dgr | 9 | 36 | 0 | 0 | 36 | - | 0/9 |
| G4_3_1 | dgr | 8 | 26 | 0 | 0 | 26 | - | 8/8 |
| G4_3_2 | dgr | 4 | 24 | 0 | 0 | 24 | - | 2/4 |
| G4_4_1 | dgr | 1 | 2 | 0 | 0 | 2 | - | 1/1 |
| G4_4_2 | dgr | 1 | 2 | 0 | 0 | 2 | - | 0/1 |
| G4_4_3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G4_5_1 | dgr | 4 | 8 | 0 | 0 | 8 | - | 4/4 |
| G4_5_2 | dgr | 5 | 5 | 0 | 0 | 5 | - | 0/5 |
| G4_6 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| G4_7 | dgr | 8 | 48 | 0 | 0 | 48 | - | 8/8 |
| G4_8a | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G4_8b | dgr | 3 | 5 | 0 | 0 | 5 | - | 0/3 |
| G4_9_ | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G4_10 | dgr | 5 | 5 | 0 | 0 | 5 | - | 0/5 |
| G4_11_1a | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G4_11_1b | dgr | 4 | 8 | 0 | 0 | 8 | - | 0/4 |
| G4_11_2 | dgr | 3 | 12 | 0 | 0 | 12 | - | 0/3 |
| G4_12 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G4_13 | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| G4_14 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P1_Town | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P1_1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P1_2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P1_3 | tgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P1_4 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P1_5 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P1_6 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P2_Town | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P2_1 | tgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P2_2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P2_3 | dgr | 2 | 4 | 0 | 0 | 4 | - | 2/2 |
| P2_4 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P2_5 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P2_6 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P2_7 | dgr | 4 | 4 | 0 | 0 | 4 | - | 4/4 |
| P3_Town | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P3_1 | dgr | 4 | 4 | 0 | 0 | 4 | - | 0/4 |
| P3_2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P3_3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P3_4 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| P3_5 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P3_6 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| P3_7 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapLeaguePortal | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| MapVoidReliquary | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| MapRustbowl | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapBackwash | tgr | 8 | 64 | 0 | 0 | 64 | - | 5/8 |
| MapBurialBog | dgr | 3 | 24 | 0 | 0 | 24 | - | 3/3 |
| MapInferno | tgr | 4 | 32 | 0 | 0 | 32 | - | 1/4 |
| MapWetlands | tgr | 4 | 32 | 0 | 0 | 32 | - | 0/4 |
| MapBloomingField | dgr | 4 | 16 | 0 | 0 | 16 | - | 0/4 |
| MapCrimsonShores | tgr | 12 | 96 | 0 | 0 | 96 | - | 0/12 |
| MapCenotes | dgr | 4 | 16 | 0 | 0 | 16 | - | 4/4 |
| MapSavanna | dgr | 8 | 16 | 0 | 0 | 16 | - | 8/8 |
| MapFortress | dgr | 5 | 20 | 0 | 0 | 20 | - | 5/5 |
| MapPenitentiary | dgr | 5 | 5 | 0 | 0 | 5 | - | 5/5 |
| MapBloodwood | dgr | 4 | 32 | 0 | 0 | 32 | - | 3/4 |
| MapSandspit | dgr | 4 | 32 | 0 | 0 | 32 | - | 0/4 |
| MapForge | dgr | 4 | 4 | 0 | 0 | 4 | - | 4/4 |
| MapSulphuricCaverns | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapMire | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapAugury | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapWoodland | tgr | 6 | 48 | 0 | 0 | 48 | - | 6/6 |
| MapSump | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapWillow | dgr | 4 | 30 | 0 | 0 | 30 | - | 0/4 |
| MapHive | dgr | 1 | 7 | 0 | 0 | 7 | - | 1/1 |
| MapHeadland | dgr | 6 | 48 | 0 | 0 | 48 | - | 6/6 |
| MapLoftySummit | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapNecropolis | dgr | 12 | 12 | 0 | 0 | 12 | - | 0/12 |
| MapCrypt | dgr | 12 | 96 | 0 | 0 | 96 | - | 11/12 |
| MapHiddenGrotto | dgr | 4 | 32 | 0 | 0 | 32 | - | 2/4 |
| MapSteamingSprings | dgr | 6 | 48 | 0 | 0 | 48 | - | 6/6 |
| MapSeepage | dgr | 2 | 16 | 0 | 0 | 16 | - | 1/2 |
| MapRiverside | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapRavine | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapSpiderWoods | dgr | 5 | 40 | 0 | 0 | 40 | - | 5/5 |
| MapMarrow | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapGrimhaven | dgr | 2 | 16 | 0 | 0 | 16 | - | 0/2 |
| MapVaalVillage | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapSlick | dgr | 2 | 16 | 0 | 0 | 16 | - | 2/2 |
| MapVaalCity | dgr | 3 | 24 | 0 | 0 | 24 | - | 2/3 |
| MapSteppe | dgr | 4 | 32 | 0 | 0 | 32 | - | 2/4 |
| MapRockpools | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapCreek | dgr | 4 | 32 | 0 | 0 | 32 | - | 0/4 |
| MapBazaar | dgr | 6 | 48 | 0 | 0 | 48 | - | 0/6 |
| MapDerelictMansion | dgr | 8 | 32 | 0 | 0 | 32 | - | 0/8 |
| MapOutlands | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapBastille | dgr | 3 | 6 | 0 | 0 | 6 | - | 3/3 |
| MapDecay | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapMineshaft | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapDeserted | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapOasis | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapSunTemple | dgr | 8 | 8 | 0 | 0 | 8 | - | 8/8 |
| MapChannel | dgr | 4 | 16 | 0 | 0 | 16 | - | 4/4 |
| MapVaalFoundry | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapVaalFactory | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapPrecursorTower | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapPrecursorTowerDesert | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapPrecursorTowerForest | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapPrecursorTowerGrass | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapPrecursorTowerMountain | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapPrecursorTowerSwamp | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapLostTowers | dgr | 6 | 48 | 0 | 0 | 48 | - | 6/6 |
| MapSwampTower | dgr | 12 | 24 | 0 | 0 | 24 | - | 0/12 |
| MapAlpineRidge | dgr | 1 | 2 | 0 | 0 | 2 | - | 0/1 |
| MapMesa | dgr | 4 | 8 | 0 | 0 | 8 | - | 0/4 |
| MapBluff | dgr | 6 | 12 | 0 | 0 | 12 | - | 4/6 |
| MapKarst | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapTerrace | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| MapPlantation | dgr | 4 | 16 | 0 | 0 | 16 | - | 0/4 |
| MapAzmerianRanges | dgr | 1 | 4 | 0 | 0 | 4 | - | 0/1 |
| MapDeforestation | tgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapEpitaph | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapAridPlains | tgr | 8 | 32 | 0 | 0 | 32 | - | 0/8 |
| MapSinterRift | tgr | 4 | 4 | 0 | 0 | 4 | - | 3/4 |
| MapPit | dgr | 4 | 32 | 0 | 0 | 32 | - | 0/4 |
| MapReservoir | dgr | 2 | 8 | 0 | 0 | 8 | - | 2/2 |
| MapGeode | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapRupture | dgr | 6 | 6 | 0 | 0 | 6 | - | 6/6 |
| MapOvergrown | dgr | 6 | 6 | 0 | 0 | 6 | - | 0/6 |
| MapCaldera | tgr | 8 | 8 | 0 | 0 | 8 | - | 0/8 |
| MapChasm | dgr | 4 | 4 | 0 | 0 | 4 | - | 3/4 |
| MapSpring_ | dgr | 1 | 2 | 0 | 0 | 2 | - | 1/1 |
| MapSevenWaters | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapCavernCity | dgr | 4 | 4 | 0 | 0 | 4 | - | 4/4 |
| MapTrenches | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapFrozenFalls | dgr | 4 | 32 | 0 | 0 | 32 | - | 0/4 |
| MapSwarm | dgr | 2 | 16 | 0 | 0 | 16 | - | 2/2 |
| MapWaywardIsle | dgr | 5 | 5 | 0 | 0 | 5 | - | 0/5 |
| MapStronghold | dgr | 5 | 5 | 0 | 0 | 5 | - | 0/5 |
| MapPort | dgr | 6 | 24 | 0 | 0 | 24 | - | 0/6 |
| MapCliffside | dgr | 3 | 3 | 0 | 0 | 3 | - | 0/3 |
| MapSinkhole | dgr | 1 | 2 | 0 | 0 | 2 | - | 0/1 |
| MapFlotsam | dgr | 9 | 9 | 0 | 0 | 9 | - | 0/9 |
| MapMortuary | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| MapOrbalasCrossing | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapMonument | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapGreenhouse | dgr | 4 | 8 | 0 | 0 | 8 | - | 4/4 |
| MapSlash | tgr | 6 | 6 | 0 | 0 | 6 | - | 6/6 |
| MapSanctuary | dgr | 5 | 5 | 0 | 0 | 5 | - | 5/5 |
| MapPrecipice | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapArroyo | dgr | 4 | 4 | 0 | 0 | 4 | - | 4/4 |
| MapCanyon | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapOrnateChambers | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| MapIceCave | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapRugosa | dgr | 4 | 12 | 0 | 0 | 12 | - | 4/4 |
| MapRiverhold | dgr | 1 | 4 | 0 | 0 | 4 | - | 0/1 |
| MapRazedFields | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapMezzanine | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapVaalVault | dgr | 4 | 4 | 0 | 0 | 4 | - | 4/4 |
| MapSnowfall | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| MapExcavation | dgr | 4 | 8 | 0 | 0 | 8 | - | 0/4 |
| MapUniqueUntaintedParadise | dgr | 3 | 24 | 0 | 0 | 24 | - | 3/3 |
| MapUniqueVault | dgr | 2 | 16 | 0 | 0 | 16 | - | 2/2 |
| MapUniqueCastaway | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapUniqueMegalith | dgr | 4 | 32 | 0 | 0 | 32 | - | 4/4 |
| MapUniqueLake | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| MapUniqueSelenite | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| MapUniqueMerchant01_Chimeral | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapUniqueMerchant01_Oasis | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapUniqueMerchant01_Sandswept | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapUniqueMerchant02_Crimson | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| MapUniqueMerchant02_Farmland | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| Map_HildaCampsite | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUniqueMerchant03_Beach | dgr | 1 | 6 | 0 | 0 | 6 | - | 1/1 |
| MapUniqueMerchant03_Tropical | dgr | 1 | 6 | 0 | 0 | 6 | - | 1/1 |
| MapUniqueMerchant03_Raft | dgr | 1 | 6 | 0 | 0 | 6 | - | 1/1 |
| MapUniqueMerchant04_PirateShip | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUniqueWildwood | tgr | 5 | 40 | 0 | 0 | 40 | - | 3/5 |
| MapUniqueFreight_ | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUberBoss_IronCitadel | dgr | 2 | 2 | 0 | 0 | 2 | - | 2/2 |
| MapUberBoss_CopperCitadel | dgr | 2 | 2 | 0 | 0 | 2 | - | 2/2 |
| MapUberBoss_StoneCitadel | dgr | 2 | 2 | 0 | 0 | 2 | - | 2/2 |
| MapUberBoss_JadeCitadel | dgr | 3 | 12 | 0 | 0 | 12 | - | 0/3 |
| MapUberBoss_Monolith | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUberBoss_FallenStar | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUberBoss_Delirium | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUberBoss_Ritual | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUberBoss_Divinity | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUberBoss_IronCitadel_Quest | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUberBoss_CopperCitadel_Quest | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapUberBoss_StoneCitadel_Quest | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionLogBook_Peninsula | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| ExpeditionLogBook_Tropical | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| ExpeditionLogBook_Tundra | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| ExpeditionLogBook_Atoll | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionLogBook_Digsite | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| ExpeditionLogBook_Reef | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionLogBook_Wastes | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| ExpeditionLogBook_Heath | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| ExpeditionLogBook_Basin | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| ExpeditionLogBook_Gully | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| ExpeditionLogBook_Prairie | dgr | 1 | 8 | 0 | 0 | 8 | - | 0/1 |
| ExpeditionSubArea_Cavern | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionSubArea_Kalguur | dgr | 4 | 32 | 0 | 0 | 32 | - | 3/4 |
| ExpeditionSubArea_Kalguur_Act1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionSubArea_Kalguur_Act2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionSubArea_Kalguur_Act3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionSubArea_Kalguur_Act4 | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionSubArea_OlrothBoss | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionSubArea_UhtredBoss | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionSubArea_VoranaBoss | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionSubArea_MedvedBoss | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionSubArea_BlackKnightBoss | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ExpeditionSubArea_Precursor | dgr | 1 | 2 | 0 | 0 | 2 | - | 1/1 |
| ExpeditionSubArea_Lair | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionSubArea_Siren | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionSubArea_Volcano | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionSubArea_Vein | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionSubArea_Laboratory | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionSubArea_Sanctuary | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionSubArea_Fissure | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionSubArea_Catacomb | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionSubArea_Pond | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| ExpeditionLeagueBoss | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Delirium_Act1Town_Quest | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Delirium_Act1Town | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Delirium_Act2Reveal | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| Delirium_Act3Town | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Delirium_VastiriOutskirts | dgr | 1 | 8 | 0 | 0 | 8 | - | 1/1 |
| Delirium_HungerBoss | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| BreachDomain_01 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| ChayulaLeague | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| ChayulaLeague_Tower | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| ChayulaLeague_TowerBoss | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| RitualLeagueBoss | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Abyss_Intro | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Abyss_Hub | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| Abyss_Depths1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Abyss_Depths2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Abyss_Depths3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Abyss_Boss1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Abyss_Boss2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| Abyss_Pinnacle | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| BossRush_Area1 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| BossRush_Area2 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| BossRush_Area3 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| G_Endgame_Town | town | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| IncursionTemplePresent | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| IncursionTemple | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| IncursionHub | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| IncursionHubEndgame | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| DruidTrailer | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| MapMothersoul_Male | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| MapMothersoul_Female | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| MapMothersoul_Male_Quest | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| MapMothersoul_Female_Quest | dgr | 1 | 4 | 0 | 0 | 4 | - | 1/1 |
| MapUniqueReactor_01 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| MapUniqueReactor_02 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| MapUniqueReactor_03 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| MapUniqueReactor_04 | dgr | 1 | 1 | 0 | 0 | 1 | - | 0/1 |
| PrecursorVault | dgr | 1 | 1 | 0 | 0 | 1 | - | 1/1 |
| EndgameDoodad_MeteorCrater | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| EndgameDoodad_BeastGroundZero | fixed | 0 | 0 | 0 | 0 | 0 | - | 0/0 |
| MapUniqueInitialTower | dgr | 4 | 8 | 0 | 0 | 8 | - | 0/4 |

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
| 🟡 | [Game update compatibility: signature table, `cargo xtask check-build`, `versions/<version>.toml`, unknown-client refusal](docs/guides/new-game-version.md) | WP A2-A4: DLL addresses and offsets from byte signatures; check-build (signatures, offsets, opcode fingerprints, strings, game data) with report and exit codes; one version file for xtask, server and launcher; the server refuses an unknown `client_hash` once `hashes_confirmed`; the client_hash is derived from the code, the in-game check must confirm it; renumbered opcodes are not derived |

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
