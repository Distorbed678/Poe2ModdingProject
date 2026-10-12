# Poe2ModdingProject

A private server for **Path of Exile 2 (client 0.5.5e)**, written in Rust, designed for future client and server modding.

- `servers/`: login and instance servers, plus `poe-server` (both in one process).
- `crates/`: config, networking, game-data access (reads the client's Bundles2 directly).
- `client/poe-hook`: `version.dll` proxy DLL for the client (encryption bypass, diagnostics).
- `tools/xtask`: `cargo xtask build-dll | deploy-client | revert-client | run-client`.
- `Documentation/`: start at [Documentation/index.md](Documentation/index.md).

Quick start: [Documentation/setup.md](Documentation/setup.md). Personal and educational project.
