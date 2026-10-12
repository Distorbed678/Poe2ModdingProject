# Architecture

```
client (PathOfExileSteam.exe + poe-hook version.dll)
   │  gateway_list.txt → login host:port
   ▼
login-server ──(ticket + instance address)──► client ──► instance-server
   │                                                      │
   └──────────── shared services (poe-server process) ────┘
        poe-config · poe-data (game files) · poe-store (DB) · ticket/instance directory
```

## Crates
| Crate | Role |
|---|---|
| `crates/poe-config` | Typed `config/server.toml`: ports, hosts, leagues, paths, logging. |
| `crates/poe-net` | Transport: buffered TCP `Connection`, `StreamCipher` trait (`Plain` now), big-endian `Reader`/`Writer` with PoE string encodings, hexdump. |
| `crates/poe-data` | Game data: `GameFiles` (Bundles2 through `poe_data_tools`, with override directories in front) and `GameData::table("WorldAreas")` (`.datc64` decoded with the community schema, columns accessed by name). |
| `servers/login-server` | Gateway/login protocol: auth, leagues, characters, hand-off to an instance. |
| `servers/instance-server` | Instance protocol: areas, entities, simulation. |
| `servers/poe-server` | Development launcher that runs both servers in one process. |
| `tools/xtask` | `cargo xtask`: DLL build, client deploy/revert/run. |
| `client/poe-hook` | Client DLL (its own Cargo project, Windows target). |

Planned crates, added as their milestone arrives:
- `poe-proto`: packet definitions, with opcodes loaded from `protocol/<version>/opcodes.toml`.
- `poe-store`: `Store` trait plus SQLite.
- `poe-world`: areas, terrain generation, walkability, pathfinding.
- `poe-game`: ECS simulation and tick loop.
- `poe-mod-api`: hooks for server mods.

## Design rules
- **No hardcoded game content or deployment values.**
  - Ports, hosts, leagues and paths live in config.
  - Classes, areas and items come from game tables.
  - Opcodes and other values specific to a client version live in `protocol/<version>/`.
- **Behind traits:** cipher, storage, game data source, and instance directory/tickets, so implementations can be swapped (for example SQLite → Postgres, or one process → a split deployment).
- **Version-specific RE knowledge** (addresses, struct offsets) lives in `Documentation/re/` and in the DLL's pattern signatures. It is never spread through server code.
- **Mod-friendly:** game files are read through an override stack, and gameplay events will go through a hook bus (see [modding.md](modding.md)).
