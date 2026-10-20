# Architecture

```
client (PathOfExileSteam.exe + version.dll loader + poe_hook_core.dll)
   │  gateway_list.txt → login host:port
   ▼
login-server ──(ticket + instance address)──► client ──► instance-server
   │   ▲                                                  │
   │   └── transfers (pending InstanceDetails) ◄──────────┤
   └──────────── shared services (poe-server process) ────┘
        poe-config · poe-data (game files) · poe-store (DB) · poe-cluster (tickets, login sessions, instances)
```

## Crates
`modules/` holds git submodules: each is its own Cargo workspace and is used by the main workspace as a path dependency (root `Cargo.toml` excludes `modules`).

| Crate | Role |
|---|---|
| `crates/poe-config` | Typed `config/server.toml`: ports, hosts, leagues, paths, logging, terrain toggle, admins, commands, instances. |
| `modules/protocol/poe-net` | Transport (Poe2Protocol submodule): `StreamCipher` trait (Salsa20 for login and instance streams), big-endian `Reader`/`Writer` with PoE string and varint encodings, hexdump; the buffered TCP `Connection` is behind the `tokio` cargo feature (the servers enable it, the client DLL does not). |
| `crates/poe-proto` | Message definitions; opcodes come from `modules/protocol/opcodes/<version>/{login,instance,hnc}.toml`. `describe` decodes captured messages for the DLL. |
| `crates/poe-data` | Game data: `GameFiles` (Bundles2 through `poe_data_tools`, with override directories in front) and `GameData::table("WorldAreas")` (`.datc64` decoded with the community schema, columns accessed by name). |
| `crates/poe-store` | `Store` trait plus SQLite: accounts, characters, saved positions. |
| `crates/poe-cluster` | Coordination between servers: tickets (`TicketRegistry`), login sessions (logout, transfers), the instance directory (`InstanceDirectory`). In-process implementations now; traits for a split deployment later. |
| `crates/poe-world` | Terrain generation (the client's generator, ported; see [terrain-status.md](terrain-status.md)). |
| `crates/poe-game` | Simulation pieces: movement, stats. |
| `servers/login-server` | Login protocol: auth, leagues, characters, hand-off and transfers to instances. |
| `servers/instance-server` | Instance protocol: areas, player, movement, chat, server commands (`commands.rs`, `exec.rs`), live sessions. |
| `servers/poe-server` | Development launcher that runs both servers in one process. |
| `xtask` | `cargo xtask`: DLL build, client deploy/revert/run, DLL data tables. |
| `modules/client-hook` | Poe2ClientHook submodule (own Cargo workspace): the client DLLs (Windows target: loader, debug core, shared config/ABI) and `poe-hook-deploy`, the host-side library behind `cargo xtask deploy-client`/`revert-client`/`deploy-core`/`run-client`. Nested submodule `protocol/` = Poe2Protocol. |

Planned: `poe-mod-api` (hooks for server mods), when mod support starts.

## Design rules
- **No hardcoded game content or deployment values.**
  - Ports, hosts, leagues, paths, admins and command settings live in config.
  - Classes, areas and items come from game tables.
  - Opcodes and other values specific to a client version live in `modules/protocol/opcodes/<version>/`.
- **Behind traits:** cipher, storage, game data source, tickets, login sessions and the instance directory, so implementations can be swapped (for example SQLite → Postgres, or one process → a split deployment).
- **Version-specific RE knowledge** (addresses, struct offsets) lives in `docs/re/` and in the DLL's pattern signatures. It is never spread through server code.
- **Mod-friendly:** game files are read through an override stack, server commands come from one registry, and gameplay events will go through a hook bus (see [modding.md](modding.md)).
