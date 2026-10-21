# Crates and servers

`modules/` holds git submodules. `protocol` and `client-hook` are each their own Cargo workspace; the main workspace uses `protocol` as a path dependency (root `Cargo.toml` excludes `modules`). `web-admin` (npm) and `launcher` (Tauri) are separate apps.

| Crate | Role |
|---|---|
| `crates/poe-config` | Typed `config/server.toml`: ports, hosts, leagues, paths, logging, terrain toggle, cluster id, account switches, web API, commands, instances, movement tuning, terrain upload, dev tools (`config/server.example.toml` lists every key). |
| `modules/protocol/poe-net` | Transport (Poe2Protocol submodule): `StreamCipher` trait (Salsa20 for login and instance streams), big-endian `Reader`/`Writer` with PoE string and varint encodings, hexdump; the buffered TCP `Connection` is behind the `tokio` cargo feature (the servers enable it, the client DLL does not). |
| `modules/protocol/poe-proto` | Message definitions (Poe2Protocol submodule); opcodes come from `modules/protocol/opcodes/<version>/{login,instance,hnc}.toml`. `describe` decodes captured messages for the DLL. |
| `crates/poe-data` | Game data: `GameFiles` (Bundles2 through `poe_data_tools`, with override directories in front) and `GameData::table("WorldAreas")` (`.datc64` decoded with the community schema, columns accessed by name). |
| `crates/poe-store` | `Store` trait plus SQLite: accounts, characters, saved positions; owns the schema migrations (`0006` adds passwords, roles and tokens). |
| `crates/poe-accounts` | `AccountStore` trait and SQLite implementation: usernames, argon2id passwords (over the client's SHA-256(UTF-16LE) digest), roles, revocable per-cluster tokens, the `account` command. |
| `crates/poe-cluster` | Coordination between servers: tickets (`TicketRegistry`), login sessions (logout, transfers), the instance directory (`InstanceDirectory`), `AdminControl` (what the web API needs from the instance server). In-process implementations now; traits for a split deployment later. |
| `crates/poe-web-api` | The HTTP/WebSocket API (axum): register, login, tokens, admin endpoints that run the command registry through `poe_cluster::AdminControl`, the live log layer, OpenAPI, static web admin. Started by `poe-server`. [web-api.md](web-api.md). |
| `crates/poe-world` | Terrain generation (the client's generator, ported; see [../status/terrain.md](../status/terrain.md)). |
| `crates/poe-game` | Simulation: the instance `World` (entities, frame clock), `GridSource` terrain grids (client dump `Grid`, `VecGrid`), the server-authoritative movement component `Locomotion` (typed inputs and events, no opcodes; built from [../protocol/movement.md](../protocol/movement.md)), `Mover` and A* pathfinding, character stats. Design: [instance-world.md](instance-world.md). Replay tests use fixtures under `tests/fixtures/` derived by `scripts/re/movement_fixtures.py`. |
| `servers/login-server` | Login protocol: auth, leagues, characters, hand-off and transfers to instances. |
| `servers/instance-server` | Instance protocol: areas, the instance actor (`actor.rs`: one task per instance owns the `World` and serves all its sessions), character blobs (`appearance.rs`), player handle, movement glue, chat, server commands (`commands.rs`, `exec.rs`), live sessions. |
| `servers/poe-server` | Development launcher that runs the login server, the instance server and the web API in one process. |
| `xtask` | `cargo xtask` (commands: [../guides/setup.md](../guides/setup.md)): thin wrappers over `poe-hook-deploy`, the DLL name tables, and the generators for the README tracker (`tracker.rs`), the OpenAPI document and the terrain matrix (`terrain_matrix.rs`). |
| `modules/web-admin` | Poe2WebAdmin submodule (Svelte 5 + Vite): the admin site on top of the web API, types generated from `docs/architecture/openapi.json`. [web-api.md](web-api.md#the-web-admin-spa). |
| `modules/launcher` | Poe2Launcher submodule (Tauri 2 + Svelte 5, own Cargo workspace in `src-tauri`): server profiles, Steam depot install, deploy and start of the game, mods; nests Poe2ClientHook and Poe2Protocol. [../guides/launcher.md](../guides/launcher.md). |
| `modules/client-hook` | Poe2ClientHook submodule (own Cargo workspace): the client DLLs (Windows target: loader, debug core, shared config/ABI) and `poe-hook-deploy`, the host-side library behind `cargo xtask deploy-client`/`revert-client`/`deploy-core`/`run-client`. Nested submodule `protocol/` = Poe2Protocol. |

Planned: `poe-mod-api` (hooks for server mods), when mod support starts.

