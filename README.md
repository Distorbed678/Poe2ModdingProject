# Poe2ModdingProject

A private server for **Path of Exile 2 (client 0.5.5e)**, written in Rust, designed for future client and server modding.

- `servers/`: login and instance servers, plus `poe-server` (both in one process).
- `crates/`: config, networking, protocol, game data (reads the client's Bundles2 directly), storage, instances/tickets, terrain generation, simulation.
- `modules/`: git submodules (private repos), each its own Cargo workspace:
  - `modules/protocol` ([Poe2Protocol](https://github.com/Distorbed678/Poe2Protocol)): `poe-proto`, `poe-net` (codec and ciphers; TCP behind the `tokio` feature) and the opcode tables `opcodes/<version>/*.toml`.
  - `modules/client-hook` ([Poe2ClientHook](https://github.com/Distorbed678/Poe2ClientHook)): the client DLLs (`poe-hook` = `version.dll` loader, `poe-hook-core` = hot-reloadable debug core with the in-game overlay, `poe-hook-shared`) and `poe-hook-deploy` (host-side deploy and launch library). Carries Poe2Protocol as a nested submodule.
- Clone with `git clone --recursive`, or run `git submodule update --init --recursive` afterwards.
- `xtask`: `cargo xtask build-dll | deploy-client | deploy-core | revert-client | run-client | dll-data` (thin wrappers over `poe-hook-deploy`).
- `scripts/`: test-run, screenshot and reverse-engineering helpers. `third_party/`: downloaded tools (git-ignored). `runtime/`: server data and logs (git-ignored).
- `docs/`: start at [docs/index.md](docs/index.md).

What works: login, character creation and selection, entering any area (the client generates the terrain from the area and seed), movement (WASD, click-to-move, dodge roll), instances and transfers between them, chat channels, server commands (chat `!` and the overlay), and server-side terrain generation for most act 1 areas ([docs/status/terrain.md](docs/status/terrain.md)).

Quick start: [docs/guides/setup.md](docs/guides/setup.md). Server commands and admins: [docs/guides/server-admin.md](docs/guides/server-admin.md). Personal and educational project.
