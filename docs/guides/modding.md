# Modding

One mod system for the client and the server: a mod is **one package** with a `mod.toml` that says where it runs (`side = "client" | "server" | "both"`), built and packed with the public SDK. The decisions behind it are in [../plans/server-modding-analysis.md](../plans/server-modding-analysis.md) ("Decisions (user, 2026-10-22)"): native Rust server mods (a `cdylib` with a versioned C ABI) plus data-only mods, never hot-reloaded, no install guard, exact version and SHA-256 pinning.

## What exists now and what is coming

| | State |
|---|---|
| Client mods (DLL loaded by the hook), launcher cache/enable/order per server | **Exists** (manifest v1 and v2) |
| Manifest v2: `side`, `[client]`, `[server]`, `[[commands]]`, fail-closed for old readers, shared parser | **Exists** (`modules/client-hook/poe-hook-mods`) |
| Package format `<id>-<version>.zip` + `.sha256`, reproducible packer, package verification | **Exists** (`poe-mod pack/verify`, `poe_hook_mods::verify_package`) |
| Server mod ABI (`poe-mod-api`), safe wrappers (`poe-mod-sdk`), templates and examples, the `poe-mod` tool | **Exists** (the SDK repository) |
| The client loader refusing a server-only mod and ignoring the server half of a `both` mod | **Exists** |
| The **server loader** for native and data mods (`mods/`, `state.toml`, plan, events, commands, `[mods]` config, `poe-server mod` CLI) | **Exists** (`modules/server/crates/poe-mod-host`, [mod-host.md](../architecture/mod-host.md)) |
| The server **advertising** its client mods (`GET /api/v1/server` field `mods`, `GET /api/v1/mods/{id}/{version}.zip`) and **checking the client's mods at login** | **Exists** |
| The launcher's **mod sync** (per-server mod sets, Off / Ask / Auto, versioned cache, SHA-256 and `verify_package` checks), the launcher using the shared parser with side badges, and its mod browser (the `Poe2Mods` index, [template](../mod-index-template/README.md)) | **Exists** (launcher; it consumes the server's `mods` advertisement) |
| Server `[data]` table overlays (`push_override`) and `[[commands]]` from mods | **Exists** (applied at start); `config_defaults` and server `opcodes` are parsed but **not applied yet** (logged) |
| Client `[data]` overlays, ed25519 signatures, a mod index | Later |

A server mod can be written, built and packed with the SDK, installed with `poe-server mod install`, and is loaded by a running `poe-server` (see [Running server mods](#running-server-mods)).

## Sides

| `side` | Contains | Loaded by |
|---|---|---|
| `client` | a DLL for the game process and/or client data (v1 manifests are always this) | the client hook (`poe-hook-mods/<id>/`) |
| `server` | a native library per platform (`.so` / `.dll`), game-data overlays, config defaults and/or `[[commands]]` | the server (`mods/<id>/`, planned) |
| `both` | one package with the two halves | both; the hook ignores the server half |

`manifest_version = 2` is required for `server` and `both` (and for `[client]`/`[server]`/`[[commands]]`); a reader that only knows v1 refuses such a manifest ("fail closed"), so a server-only mod can never be mistaken for a client mod. Client-only mods can stay v1 and keep working with every launcher. Full spec: [../../modules/client-hook/docs/mod-manifest.md](../../modules/client-hook/docs/mod-manifest.md).

## Packaging

`<id>-<version>.zip` with `mod.toml` at the root plus the files it names (conventionally `client/`, `server/`, `data/`), and `<id>-<version>.zip.sha256`; the zip's SHA-256 is the package identity. `poe-mod pack` writes it reproducibly (sorted entries, fixed timestamp and permissions), `poe-mod verify` and `poe_hook_mods::verify_package` check hash, name, manifest, named files, path hygiene, symlinks, encryption, duplicates, size limits and CRCs. Details: the SDK's [docs/packaging.md](https://github.com/Distorbed678/Poe2ModSDK/blob/main/docs/packaging.md).

## The SDK repository

[github.com/Distorbed678/Poe2ModSDK](https://github.com/Distorbed678/Poe2ModSDK) (public, MIT; for local work it is cloned to `third_party/Poe2ModSDK`, which is git-ignored):

- `poe-mod-api`: the server mod C ABI, `SERVER_API_VERSION = 1`. `ServerApi` (log, config, command registration, submit), `ModInfo`, events with read-only snapshots (server start/stop, player join/leave, command, instance created/closed, tick) and commands the mod returns (send chat, broadcast, teleport, kick) that the server applies on the owning actor task. Rules: [abi-stability.md](https://github.com/Distorbed678/Poe2ModSDK/blob/main/docs/abi-stability.md).
- `poe-mod-sdk`: `ServerMod` + `declare_server_mod!`; `declare_client_mod!` with the hook's `HookApi` re-exported.
- `poe-mod-cli` (`poe-mod`): `new client|server|both`, `check`, `pack`, `verify`.
- `examples/`: `hello-client`, `hello-server` (`!hello`), `hello-both`.

The SDK takes the client's ABI crate and the manifest parser from `Poe2ClientHook` as git dependencies pinned to a commit (one source of truth, no vendored copy to drift); the server loader (`modules/server/crates/poe-mod-host`) stays in this repository and uses the same `poe-hook-mods` crate through the client-hook submodule, and the SDK's `poe-mod-api` as a git dependency pinned to its `v0.1.0` tag ([why](../architecture/mod-host.md#pin-consistency)). Guides: [writing-server-mods.md](https://github.com/Distorbed678/Poe2ModSDK/blob/main/docs/writing-server-mods.md), [writing-client-mods.md](https://github.com/Distorbed678/Poe2ModSDK/blob/main/docs/writing-client-mods.md).

## Server

- **Data-driven content:** server logic reads game tables by column name, so modified tables flow through without code changes.
- **Data mods (no code):** a server mod with only `[[commands]]` (a `reply` text or an `alias` of another command, `role` player or admin) and/or `[server] tables = [...]` folders. Table folders are pushed with `poe_data::GameFiles::push_override` in load order (a file at `<folder>/<virtual path>` replaces the bundled one, later mods win) before the server shares its game data; a table the client also reads makes the mod `both` (the client half of an overlay is not applied yet). `config_defaults` and server `opcodes` are parsed and validated but not applied yet; the server logs that.
- **Native server mods:** every mod in the server's `mods/` folder that is enabled in its `state.toml` is loaded: the owner is responsible; the server API version and the game version are still checked and a refused mod is a log line while the server keeps starting. Changing server mods needs a restart, never a hot reload. Mod authors: the SDK guides above. Design: [../architecture/mod-host.md](../architecture/mod-host.md).
- **Server commands:** one registry (`modules/server/servers/instance-server/src/commands.rs`, run by `exec.rs`) of built-in commands plus the ones mods register at startup; a command works in chat (`!name`) and in the overlay's Commands page ([server-admin.md](server-admin.md)) and takes the rest of the line as its argument.

### Running server mods

```sh
poe-server mod install hello-both-0.1.0.zip --enable   # verify, unpack into mods/hello-both/, enable
poe-server mod list                                    # id, version, side, enabled
poe-server mod disable hello-both                      # edits mods/state.toml
poe-server mod verify                                  # packages, manifests, libraries; exit code 1 on errors
```

- **Layout** (`[mods] dir`, default `mods`): `state.toml` (`enabled = [...]`, the load order; **its existence turns the mod system and the login check on**), `<id>/` folders, `packages/<id>-<version>.zip` (+ `.sha256`) for the mods that have a client half. A zip dropped into `mods/` or `packages/` is verified and used as that mod's package. A mod with a client half but no matching package is packed from its folder at start. A mod's installed `mod.toml` must be the one in its package (install does this).
- **At start** each enabled mod is checked and loaded in order; the log shows `mods: loaded <id> <version> ...` or `mods: skipped <id>: <reason>` (wrong `server_api`, another game version, a missing library for this OS, a missing or conflicting dependency, `poe_server_mod_init` returning an error, a command name that is taken). A server library is mapped only after its manifest passed.
- **Config** `[mods]` in `modules/server/config/server.toml` (all keys in `modules/server/config/server.example.toml`): `dir`; `require_client_mods` (default `true`: check the client's mods at login); `allow_extra_client_mods` (default `false`: a client mod the server does not list refuses the login); `extra_urls` (more download addresses advertised for every offered mod, `{id}` and `{version}` replaced); `report_wait_secs` (15), `max_failures` (5) and `callback_budget_ms` (50) for the login wait and the failure handling; per-mod settings under `[mods.config.<id>]`, which the mod reads as text with `config_get`.
- **What clients see.** `GET /api/v1/server` lists the enabled client and `both` mods that loaded (id, version, SHA-256 and size of the package, `required`, `hook_api`, `game_versions`, `code`, `sources`), and `GET /api/v1/mods/{id}/{version}.zip` serves the exact package ([web-api.md](../architecture/web-api.md#mods)). The launcher compares, downloads and verifies them per its *Mod sync* setting.
- **The login check.** When a player enters the world, the server compares the client mods the hook reports with the required set. Missing mods, other versions, a changed `mod.toml` or DLL, or (without `allow_extra_client_mods`) mods the server does not list: the player gets server chat lines naming what is wrong and what the server needs, and is disconnected a few seconds later; the log has `login refused: the client's mods do not match this server's`. An old hook that sends no report is refused when the server requires mods and allowed when it requires none. This is a consistency check, not anti-cheat: a modified client can report anything. The hook sends the report only to your own (trusted) server, never in official mode. For a server that wants to forbid all client mods, create an empty `mods/state.toml` and keep `allow_extra_client_mods = false`; a development server whose players run their own client mods needs `allow_extra_client_mods = true` (or no `state.toml`).
- **Web admin:** the Mods page lists every mod with its state and says when a restart is needed ([server-admin.md](server-admin.md#mods)).
- **Trust.** A native server mod runs inside the server process with its rights and can read the database and all accounts' data: install only mods you trust. A crash in a mod takes the server down (the SDK contains panics; it cannot contain an abort or a segfault).

## Client

- **Debug core (exists):** `modules/client-hook/poe-hook-core` is hot-reloadable (a running client reloads a redeployed core, see [setup.md](setup.md)); its features are toggled in `poe-hook.toml` ([../architecture/client-dll.md](../architecture/client-dll.md)). The loader (`modules/client-hook/poe-hook`) keeps the permanent hooks.
- **Mod loader (exists):** a client mod is a folder `poe-hook-mods/<id>/` next to `PathOfExileSteam.exe` with a `mod.toml` and, in `poe-hook-mods/state.toml`, the enabled ids in load order. The launcher keeps a cache of verified mod packages, switches them per server (a server's required mods are downloaded for you per the *Mod sync* setting) and writes the set into `poe-hook-mods/` at Play ([launcher.md](launcher.md#mods)); the loader in `version.dll` maps the enabled entry DLLs in that order and calls their `poe_mod_init(&HookApi)` (a logging callback, the game build, the read-only latch and the resolved signature table). A mod with another ABI version, a missing dependency or a failing init is logged and skipped; a server-only mod is skipped with "server-only mod"; the server half of a `both` mod is ignored; **in official mode no mod loads at all**. Format, ABI and log lines: [../../modules/client-hook/docs/mod-manifest.md](../../modules/client-hook/docs/mod-manifest.md); the sample mod is `modules/client-hook/examples/sample-mod/` (manifest v1).
- **Launcher:** Library, Mods shows the side of each mod (`client`, `server`, `both`), refuses a server-only mod and installs only the client half of a `both` package; each server has its own mod set; the mod browser reads the public index (submit a mod by pull request: [../mod-index-template/README.md](../mod-index-template/README.md)). Details: [launcher.md](launcher.md#mods).
- **Install from the repository:** `cargo xtask deploy-mod [dir]` (default: the sample mod; `--dll`, `--disable`, `--no-build`) or `cargo xtask deploy-client --sample-mod` put a mod into `GameFiles/poe-hook-mods/<id>/` and enable it, through `poe-hook-deploy` (a server-only mod is refused; of a `both` mod only the client half is installed); `revert-client` removes what these installed. Never copy into GameFiles by hand.
- **Trust.** A client mod is native code (a DLL) that runs inside the game process with your user's rights; the installer and the loader cannot make that safe. Install only mods you trust, from sources you trust. The same holds for native server mods in the server process. The loader loads no mod in official mode, and the launcher checks a mod's manifest and paths but not what its code does; a package's hash proves which file it is, not that it is safe.
- **Planned:** applying a mod's `[data]` overlays (game tables, opcodes, config defaults), a loose-file override hook so client mods can ship replacement assets, plus packet hooks for custom messages.
