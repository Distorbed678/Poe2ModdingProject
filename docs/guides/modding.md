# Modding

One mod system for the client and the server: a mod is **one package** with a `mod.toml` that says where it runs (`side = "client" | "server" | "both"`), built and packed with the public SDK. The decisions behind it are in [../plans/server-modding-analysis.md](../plans/server-modding-analysis.md) ("Decisions (user, 2026-10-22)"): native Rust server mods (a `cdylib` with a versioned C ABI) plus data-only mods, never hot-reloaded, no install guard, exact version and SHA-256 pinning.

## What exists now and what is coming

| | State |
|---|---|
| Client mods (DLL loaded by the hook), launcher install/enable/order | **Exists** (manifest v1) |
| Manifest v2: `side`, `[client]`, `[server]`, `[[commands]]`, fail-closed for old readers, shared parser | **Exists** (`modules/client-hook/poe-hook-mods`) |
| Package format `<id>-<version>.zip` + `.sha256`, reproducible packer, package verification | **Exists** (`poe-mod pack/verify`, `poe_hook_mods::verify_package`) |
| Server mod ABI (`poe-mod-api`), safe wrappers (`poe-mod-sdk`), templates and examples, the `poe-mod` tool | **Exists** (the SDK repository) |
| The client loader refusing a server-only mod and ignoring the server half of a `both` mod | **Exists** |
| The **server loader** for native and data mods (`mods/`, `state.toml`, plan, events, commands), `[mods]` config, `poe-server mod` CLI | Next (WP-S1 to S4) |
| The server **advertising** its mods (`GET /api/v1/server`, `/api/v1/mods/{id}/{version}.zip`) and checking client mods at login | Next (WP-M2) |
| The launcher's **mod sync** (per-server mod sets, Off / Ask / Auto, cache, approvals) and the launcher using the shared parser | Next (WP-M3/M4) |
| Applying `[data]` overlays (game tables, `push_override`) and `config_defaults` from mods | Next (WP-S2); today `GameFiles::push_override` is a library capability only, **nothing calls it** |
| Client `[data]` overlays, ed25519 signatures, a mod index | Later |

Until the server loader lands, a server mod can be written, built, packed and verified, and the SDK test loads the built example library the way the server will, but a running `poe-server` does not load mods yet.

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

The SDK takes the client's ABI crate and the manifest parser from `Poe2ClientHook` as git dependencies pinned to a commit (one source of truth, no vendored copy to drift); the server loader stays in this repository and will use the same `poe-hook-mods` crate. Guides: [writing-server-mods.md](https://github.com/Distorbed678/Poe2ModSDK/blob/main/docs/writing-server-mods.md), [writing-client-mods.md](https://github.com/Distorbed678/Poe2ModSDK/blob/main/docs/writing-client-mods.md).

## Server

- **Data-driven content (exists):** server logic reads game tables by column name, so modified tables flow through without code changes.
- **File overrides (library only):** `poe_data::GameFiles::push_override(dir)` puts a directory in front of Bundles2 (a file at `<dir>/<virtual path>` replaces the bundled one, later overrides win). It is an API of the data crate; **no config key or startup code calls it yet**, so a server owner cannot use it until the data tier (WP-S2) wires it to enabled mods.
- **Server commands (exists, closed):** one registry (`servers/instance-server/src/commands.rs`, run by `exec.rs`); a command added there works in chat (`!name`) and in the overlay's Commands page ([server-admin.md](server-admin.md)). It is a compile-time list; mods will add commands through a dynamic registry (WP-S2/S3).
- **Native server mods (planned loader, ABI exists):** every mod in the server's `mods/` folder that is enabled in its `state.toml` is loaded (the owner is responsible; ABI and game-version checks still refuse incompatible mods); changing server mods needs a restart, never a hot reload. Mod authors: the SDK guides above.

## Client

- **Debug core (exists):** `modules/client-hook/poe-hook-core` is hot-reloadable (a running client reloads a redeployed core, see [setup.md](setup.md)); its features are toggled in `poe-hook.toml` ([../architecture/client-dll.md](../architecture/client-dll.md)). The loader (`modules/client-hook/poe-hook`) keeps the permanent hooks.
- **Mod loader (exists):** a client mod is a folder `poe-hook-mods/<id>/` next to `PathOfExileSteam.exe` with a `mod.toml` and, in `poe-hook-mods/state.toml`, the enabled ids in load order. The launcher installs, enables and orders mods ([launcher.md](launcher.md#mods)); the loader in `version.dll` maps the enabled entry DLLs in that order and calls their `poe_mod_init(&HookApi)` (a logging callback, the game build, the read-only latch and the resolved signature table). A mod with another ABI version, a missing dependency or a failing init is logged and skipped; a server-only mod is skipped with "server-only mod"; the server half of a `both` mod is ignored; **in official mode no mod loads at all**. Format, ABI and log lines: [../../modules/client-hook/docs/mod-manifest.md](../../modules/client-hook/docs/mod-manifest.md); the sample mod is `modules/client-hook/examples/sample-mod/` (manifest v1).
- **Install from the repository:** `cargo xtask deploy-mod [dir]` (default: the sample mod; `--dll`, `--disable`, `--no-build`) or `cargo xtask deploy-client --sample-mod` put a mod into `GameFiles/poe-hook-mods/<id>/` and enable it, through `poe-hook-deploy` (a server-only mod is refused; of a `both` mod only the client half is installed); `revert-client` removes what these installed. Never copy into GameFiles by hand.
- **Trust.** A client mod is native code (a DLL) that runs inside the game process with your user's rights; the installer and the loader cannot make that safe. Install only mods you trust, from sources you trust. The same holds for native server mods in the server process. The loader loads no mod in official mode, and the launcher checks a mod's manifest and paths but not what its code does; a package's hash proves which file it is, not that it is safe.
- **Planned:** applying a mod's `[data]` overlays (game tables, opcodes, config defaults), a loose-file override hook so client mods can ship replacement assets, plus packet hooks for custom messages.
