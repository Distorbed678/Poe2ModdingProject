# Poe2ModdingProject

A private server for **Path of Exile 2 (client 0.5.5e)**, written in Rust, designed for future client and server modding.

- `servers/`: login and instance servers, plus `poe-server` (both in one process).
- `crates/`: config, networking, protocol, game data (reads the client's Bundles2 directly), storage, instances/tickets, terrain generation, simulation.
- `client/`: the client DLLs: `poe-hook` (`version.dll` loader), `poe-hook-core` (hot-reloadable debug core with the in-game overlay), `poe-hook-shared`.
- `tools/xtask`: `cargo xtask build-dll | deploy-client | deploy-core | revert-client | run-client | dll-data`.
- `Documentation/`: start at [Documentation/index.md](Documentation/index.md).

What works: login, character creation and selection, entering any area (the client generates the terrain from the area and seed), movement (WASD, click-to-move, dodge roll), instances and transfers between them, chat channels, server commands (chat `!` and the overlay), and server-side terrain generation for most act 1 areas ([terrain-status.md](Documentation/terrain-status.md)).

Quick start: [Documentation/setup.md](Documentation/setup.md). Server commands and admins: [Documentation/server-admin.md](Documentation/server-admin.md). Personal and educational project.
