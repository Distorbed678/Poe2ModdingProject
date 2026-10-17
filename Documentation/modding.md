# Modding

Modding is a design goal from the start. This page records the hooks that already exist and those that are planned.

## Server
- **File overrides (exists):** `poe_data::GameFiles::push_override(dir)` puts a directory in front of Bundles2. A file at `<dir>/<virtual path>` (for example `data/balance/worldareas.datc64`) replaces the bundled one. Later overrides win.
- **Server commands (exists):** one registry (`servers/instance-server/src/commands.rs`, run by `exec.rs`). A command added there works in chat (`!name`) and in the overlay's Commands page without client changes ([server-admin.md](server-admin.md)).
- **Hook bus (planned, `poe-mod-api`):** events for login, character creation, area enter/leave, tick, packet in/out and entity spawn. Mods subscribe to these events. Native Rust mods come first; a scripting runtime (Lua or WASM) is chosen later.
- **Data-driven content:** server logic reads game tables by column name, so modified tables flow through without code changes.

## Client
- **Debug core (exists):** `client/poe-hook-core` is hot-reloadable (`cargo xtask deploy-core` while the game runs); its features are toggled in `poe-hook.toml` ([client-dll.md](client-dll.md)). The loader (`client/poe-hook`) keeps the permanent hooks.
- **Planned:** a loose-file override hook so client mods can ship replacement assets (sharing the same `mods/<name>/` layout as the server), plus packet hooks for custom messages.
