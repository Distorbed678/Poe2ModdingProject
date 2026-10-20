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
| Crate | Role |
|---|---|
| `crates/poe-config` | Typed `config/server.toml`: ports, hosts, leagues, paths, logging, terrain toggle, admins, commands, instances. |
| `crates/poe-net` | Transport: buffered TCP `Connection`, `StreamCipher` trait (Salsa20 for login and instance streams), big-endian `Reader`/`Writer` with PoE string and varint encodings, hexdump. |
| `crates/poe-proto` | Message definitions; opcodes come from `protocol/<version>/{login,instance,hnc}.toml`. `describe` decodes captured messages for the DLL. |
| `crates/poe-data` | Game data: `GameFiles` (Bundles2 through `poe_data_tools`, with override directories in front) and `GameData::table("WorldAreas")` (`.datc64` decoded with the community schema, columns accessed by name). |
| `crates/poe-store` | `Store` trait plus SQLite: accounts, characters, saved positions. |
| `crates/poe-cluster` | Coordination between servers: tickets (`TicketRegistry`), login sessions (logout, transfers), the instance directory (`InstanceDirectory`). In-process implementations now; traits for a split deployment later. |
| `crates/poe-world` | Terrain generation (the client's generator, ported; see [terrain-status.md](terrain-status.md)). |
| `crates/poe-game` | Simulation pieces: movement, stats. |
| `servers/login-server` | Login protocol: auth, leagues, characters, hand-off and transfers to instances. |
| `servers/instance-server` | Instance protocol: areas, player, movement, chat, server commands (`commands.rs`, `exec.rs`), live sessions. |
| `servers/poe-server` | Development launcher that runs both servers in one process. |
| `xtask` | `cargo xtask`: DLL build, client deploy/revert/run, DLL data tables. |
| `client/` | Client DLLs (own Cargo workspace, Windows target): loader, debug core, shared config/ABI. |

Planned: `poe-mod-api` (hooks for server mods), when mod support starts.

## Design rules
- **No hardcoded game content or deployment values.**
  - Ports, hosts, leagues, paths, admins and command settings live in config.
  - Classes, areas and items come from game tables.
  - Opcodes and other values specific to a client version live in `protocol/<version>/`.
- **Behind traits:** cipher, storage, game data source, tickets, login sessions and the instance directory, so implementations can be swapped (for example SQLite → Postgres, or one process → a split deployment).
- **Version-specific RE knowledge** (addresses, struct offsets) lives in `docs/re/` and in the DLL's pattern signatures. It is never spread through server code.
- **Mod-friendly:** game files are read through an override stack, server commands come from one registry, and gameplay events will go through a hook bus (see [modding.md](modding.md)).
