# Modding

Modding is a design goal from the start. This page records the hooks that already exist and those that are planned.

## Server
- **File overrides (exists):** `poe_data::GameFiles::push_override(dir)` puts a directory in front of Bundles2. A file at `<dir>/<virtual path>` (for example `data/balance/worldareas.datc64`) replaces the bundled one. Later overrides win.
- **Server commands (exists):** one registry (`servers/instance-server/src/commands.rs`, run by `exec.rs`). A command added there works in chat (`!name`) and in the overlay's Commands page without client changes ([server-admin.md](server-admin.md)).
- **Hook bus (planned, `poe-mod-api`):** events for login, character creation, area enter/leave, tick, packet in/out and entity spawn. Mods subscribe to these events. Native Rust mods come first; a scripting runtime (Lua or WASM) is chosen later.
- **Data-driven content:** server logic reads game tables by column name, so modified tables flow through without code changes.

## Client
- **Debug core (exists):** `modules/client-hook/poe-hook-core` is hot-reloadable (a running client reloads a redeployed core, see [setup.md](setup.md)); its features are toggled in `poe-hook.toml` ([../architecture/client-dll.md](../architecture/client-dll.md)). The loader (`modules/client-hook/poe-hook`) keeps the permanent hooks.
- **Mod loader (exists):** a client mod is a folder `poe-hook-mods/<id>/` next to `PathOfExileSteam.exe` with a `mod.toml` (id, version, client version, mod ABI, load order, dependencies, entry DLL, data files) and, in `poe-hook-mods/state.toml`, the enabled ids in load order. The launcher installs, enables and orders mods ([launcher.md](launcher.md#mods)); the loader in `version.dll` maps the enabled entry DLLs in that order and calls their `poe_mod_init(&HookApi)` (a logging callback, the game build, the read-only latch and the resolved signature table). A mod with another ABI version, a missing dependency or a failing init is logged and skipped; **in official mode no mod loads at all**. Format, ABI and log lines: [modules/client-hook/docs/mod-manifest.md](../../modules/client-hook/docs/mod-manifest.md); the sample mod is `modules/client-hook/examples/sample-mod/`.
- **Install from the repository:** `cargo xtask deploy-mod [dir]` (default: the sample mod; `--dll`, `--disable`, `--no-build`) or `cargo xtask deploy-client --sample-mod` put a mod into `GameFiles/poe-hook-mods/<id>/` and enable it, through `poe-hook-deploy`; `revert-client` removes what these installed. Never copy into GameFiles by hand.
- **Planned:** applying a mod's `[data]` overlays (game tables, opcodes, config defaults), a loose-file override hook so client mods can ship replacement assets, plus packet hooks for custom messages.
