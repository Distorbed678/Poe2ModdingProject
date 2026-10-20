# Crates and servers

`modules/` holds git submodules: each is its own Cargo workspace and is used by the main workspace as a path dependency (root `Cargo.toml` excludes `modules`).

| Crate | Role |
|---|---|
| `crates/poe-config` | Typed `config/server.toml`: ports, hosts, leagues, paths, logging, terrain toggle, admins, commands, instances. |
| `modules/protocol/poe-net` | Transport (Poe2Protocol submodule): `StreamCipher` trait (Salsa20 for login and instance streams), big-endian `Reader`/`Writer` with PoE string and varint encodings, hexdump; the buffered TCP `Connection` is behind the `tokio` cargo feature (the servers enable it, the client DLL does not). |
| `modules/protocol/poe-proto` | Message definitions (Poe2Protocol submodule); opcodes come from `modules/protocol/opcodes/<version>/{login,instance,hnc}.toml`. `describe` decodes captured messages for the DLL. |
| `crates/poe-data` | Game data: `GameFiles` (Bundles2 through `poe_data_tools`, with override directories in front) and `GameData::table("WorldAreas")` (`.datc64` decoded with the community schema, columns accessed by name). |
| `crates/poe-store` | `Store` trait plus SQLite: accounts, characters, saved positions. |
| `crates/poe-cluster` | Coordination between servers: tickets (`TicketRegistry`), login sessions (logout, transfers), the instance directory (`InstanceDirectory`). In-process implementations now; traits for a split deployment later. |
| `crates/poe-world` | Terrain generation (the client's generator, ported; see [../status/terrain.md](../status/terrain.md)). |
| `crates/poe-game` | Simulation pieces: movement, stats. |
| `servers/login-server` | Login protocol: auth, leagues, characters, hand-off and transfers to instances. |
| `servers/instance-server` | Instance protocol: areas, player, movement, chat, server commands (`commands.rs`, `exec.rs`), live sessions. |
| `servers/poe-server` | Development launcher that runs both servers in one process. |
| `xtask` | `cargo xtask` (commands: [../guides/setup.md](../guides/setup.md)): thin wrappers over `poe-hook-deploy` plus the DLL name tables. |
| `modules/client-hook` | Poe2ClientHook submodule (own Cargo workspace): the client DLLs (Windows target: loader, debug core, shared config/ABI) and `poe-hook-deploy`, the host-side library behind `cargo xtask deploy-client`/`revert-client`/`deploy-core`/`run-client`. Nested submodule `protocol/` = Poe2Protocol. |

Planned: `poe-mod-api` (hooks for server mods), when mod support starts.

