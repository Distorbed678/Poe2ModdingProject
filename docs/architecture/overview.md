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

## Where things are

- Crates, servers and submodules, with the role of each: [crates.md](crates.md).
- The client DLLs: [client-dll.md](client-dll.md).
- Accounts and identity: [accounts.md](accounts.md). Instances, the instance actor and the simulation: [instance-world.md](instance-world.md). Web administration: [web-api.md](web-api.md). Server mods and the client mod check: [mod-host.md](mod-host.md).
- Wire formats: [../protocol/](../protocol/framing.md). Reverse-engineering notes: [../re/README.md](../re/README.md).
- Commands, logs and tests: [../guides/setup.md](../guides/setup.md); the rules: [../../CLAUDE.md](../../CLAUDE.md).

## Design rules
The project rules (no hardcoded opcodes, ports, leagues or game data; subsystems behind traits) are in [../../CLAUDE.md](../../CLAUDE.md). In practice:
- Ports, hosts, leagues, paths and command settings live in `config/server.toml`; classes, areas and items come from game tables; opcodes and other client-version values live in `modules/protocol/opcodes/<version>/`.
- The traits are: cipher, storage, game data source, tickets, login sessions and the instance directory, so implementations can be swapped (for example SQLite → Postgres, or one process → a split deployment).
- **Version-specific RE knowledge** (addresses, struct offsets) lives in `docs/re/` and in the DLL's pattern signatures. It is never spread through server code.
- **Mod-friendly:** game files are read through an override stack, server commands come from one registry, and gameplay events will go through a hook bus (see [../guides/modding.md](../guides/modding.md)).
