# Documentation index

A private server for Path of Exile 2 (client **0.5.5e**), written in Rust. Rules and the project overview: [../CLAUDE.md](../CLAUDE.md). Start with [guides/setup.md](guides/setup.md), then [architecture/overview.md](architecture/overview.md).

## Guides
| Document | Contents |
|---|---|
| [guides/setup.md](guides/setup.md) | Commands, logs and unattended tests: clone, toolchain, build, deploy to `GameFiles/`, run client and server, official read-only sessions, crash dumps |
| [guides/server-admin.md](guides/server-admin.md) | Server commands (chat `!` and overlay), admins, instances, transfers, chat, the web admin site |
| [guides/hosting.md](guides/hosting.md) | What blocks remote players and the same-machine dependencies; suggested order |
| [guides/launcher.md](guides/launcher.md) | The planned player launcher |
| [guides/modding.md](guides/modding.md) | Server and client modding hooks (planned and existing) |

## Architecture
| Document | Contents |
|---|---|
| [architecture/overview.md](architecture/overview.md) | Data flow between client, login server and instance server; design rules |
| [architecture/crates.md](architecture/crates.md) | Every crate, server and submodule and its role |
| [architecture/client-dll.md](architecture/client-dll.md) | The client DLLs: loader, hot-reloadable core, overlay, packet capture, modes (read-only), features |
| [architecture/accounts.md](architecture/accounts.md) | Account model today and planned |
| [architecture/instance-world.md](architecture/instance-world.md) | Instance directory, per-connection simulation today, the shared-world plan |
| [architecture/web-api.md](architecture/web-api.md) | The web API (accounts, admin endpoints, live log, OpenAPI) and the web admin SPA (modules/web-admin) |

## Protocol (client 0.5.5e)
| Document | Contents |
|---|---|
| [protocol/framing.md](protocol/framing.md) | Wire format: encodings, framing, encryption |
| [protocol/login.md](protocol/login.md) | Login messages, character list, leagues, hand-off |
| [protocol/instance.md](protocol/instance.md) | Instance messages: opening sequence, area info, inventories, frames, pause, chat, leaving the game |
| [protocol/entities.md](protocol/entities.md) | Entity initial-state blobs (player components, actor skills, NPCs) |
| [protocol/movement.md](protocol/movement.md) | Lockstep movement: WASD, click-to-move, skills, dodge roll, facing, lag |
| [protocol/side-channel.md](protocol/side-channel.md) | DLL to instance server side channel: HMAC-authenticated position reports and debug link (UDP), terrain upload (TCP) |

## Reverse engineering and data
| Document | Contents |
|---|---|
| [re/README.md](re/README.md) | How the client is analysed: Ghidra access, `scripts/re`, where findings go |
| [re/addresses-0.5.5e.md](re/addresses-0.5.5e.md) | Ghidra addresses of functions and structures |
| [re/standalone-login.md](re/standalone-login.md) | How the Steam exe picks the Steam vs email/password login (platform getter), the `0x0003` credentials packet, Remember Me tokens, the DLL patch to force standalone, and the fallback |
| [re/crypto.md](re/crypto.md) | Login key exchange, Salsa20 key/IV layout, the HNC side channel, the DLL bypass |
| [re/datamining.md](re/datamining.md) | Reading Bundles2 and `.datc64` tables, the schema, the dumps folder |
| [re/captures.md](re/captures.md) | Official servers vs ours, and the live client log |
| [re/terrain/README.md](re/terrain/README.md) | Terrain generator overview: phases, RNG, hashes, tile records, tools, the port crate |
| [re/terrain/tgr.md](re/terrain/tgr.md) | `.tgr` areas: generation order, graph layout, rooms, routing, edge tiles |
| [re/terrain/dungeon.md](re/terrain/dungeon.md) | `.dgr` dungeon areas: call chain, rooms, corridors, tile pass |
| [re/terrain/shared-passes.md](re/terrain/shared-passes.md) | Passes both kinds share: topology choice, lattice, fill, de-repetition, outer buffer |
| [../dumps/README.md](../dumps/README.md) | `dumps/<version>/` (git-ignored): what produces each file and `scripts/dumps/migrate.sh` |

## Status, research, plans
| Document | Contents |
|---|---|
| [status/terrain.md](status/terrain.md) | The only terrain status: results per area, where work stopped, next steps |
| [status/history.md](status/history.md) | What was built when, and the open items |
| [status/tracker.toml](status/tracker.toml) | Data for the README status tables: current task, subsystems and items with done/partial/missing/planned; edit it, then run `cargo xtask tracker` ([guides/setup.md](guides/setup.md#run)) |
| [research/poe1-prior-art.md](research/poe1-prior-art.md) | What public PoE1 research says about the protocol |
| [research/tools.md](research/tools.md) | Survey of datamining and RE tools |
| [plans/orchestration-2026-10.md](plans/orchestration-2026-10.md) | Orchestration plan (2026-10-20): restructure, repo split, movement rewrite, accounts and multiplayer, web admin, launcher, docs cleanup, terrain handoff |
| [../modules/client-hook/docs/mod-manifest.md](../modules/client-hook/docs/mod-manifest.md) | `mod.toml` spec for client mods (Poe2ClientHook submodule; spec only) |
