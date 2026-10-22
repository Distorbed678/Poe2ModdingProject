# Poe2 mod index

The public index of **Path of Exile 2** mods that the [Poe2 launcher](https://github.com/Distorbed678/Poe2Launcher) shows under *Library > Mods > Browse*. The index is one file, [`index.json`](index.json); anyone adds a mod with a pull request. The mod files themselves are **not** stored here: the index lists where each version can be downloaded (an `https://` address, usually a GitHub release asset) and the SHA-256 of the package.

The launcher reads `https://raw.githubusercontent.com/Distorbed678/Poe2Mods/main/index.json`, checks every entry, keeps a saved copy (so Browse still works offline) and installs a version into its mod cache: it downloads the package, compares its SHA-256 and size with the index, runs the package checks (`poe_hook_mods::verify_package`: file names, paths, links, size limits, `mod.toml`) and only then keeps it. A download that fails any check installs nothing.

> This is an unofficial fan project for the private server of Poe2ModdingProject. It is not connected to Grinding Gear Games. A mod with a client DLL is native code that runs inside the game on a player's computer; being listed here is not a security review. The hash proves which file it is, not that it is safe. Players decide whom to trust.

## Submit a mod

1. Build and pack your mod with the [Poe2ModSDK](https://github.com/Distorbed678/Poe2ModSDK): `poe-mod new client|server|both`, `poe-mod pack`, `poe-mod verify`. The result is `<id>-<version>.zip` plus a `.sha256` file; the zip's SHA-256 is the package identity.
2. Publish the zip at a stable `https://` address (a GitHub release asset is ideal; the address must keep serving exactly that file).
3. Fork this repository and add your mod to the `mods` list in `index.json` (copy it from [`index.example.json`](index.example.json)), or add a new `versions` entry to your existing mod. Put the **newest version first**.
4. Run `python3 scripts/validate.py` (Python 3, nothing to install). To also download your packages and compare size and SHA-256, run `python3 scripts/validate.py --online`. Fix what it reports.
5. Open a pull request. The check in the pull request runs the same script, including the download.

To change a mod, send another pull request. The `id` never changes (it is the `id` of your `mod.toml`). To withdraw a version, set `"yanked": true` instead of deleting it, so players who have it see why it is gone.

### The entry

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | The mod's id from its `mod.toml`: `[a-z0-9][a-z0-9_-]{1,63}`. Unique in the index. |
| `name` | yes | Shown to players, up to 80 characters. |
| `description` | no | Up to 600 characters, plain text. |
| `author` | no | Up to 80 characters. |
| `side` | yes | `client` (runs in the game), `server` (runs on a server; the launcher lists it but cannot install it into the game) or `both` (one package; the launcher installs the client half only). The same as `side` in `mod.toml`. |
| `tags` | no | Up to 12 of `a-z`, `0-9`, `-` (24 characters each), for example `ui`, `qol`, `gameplay`, `loot`, `admin`. |
| `homepage` | no | An `https://` page (source, documentation). |
| `licence` | no | The mod's licence, for example `MIT`. |
| `versions` | yes | 1 to 200 versions, newest first (below). |

Each version:

| Field | Required | Meaning |
|---|---|---|
| `version` | yes | A semantic version, the one in the package's `mod.toml`. Unique per mod. |
| `released` | no | A date such as `2026-10-22`, for display. |
| `game_versions` | no | The client builds it was made for, for example `["0.5.5e"]`. The launcher offers a version only for a game version it lists (empty means "not stated"). |
| `hook_api` | for a client DLL | The client mod ABI it was built for (`game.hook_api` / `client.hook_api`). The launcher compares it with the ABI of the installed hook. |
| `server_api` | for a server library | The server API it was built for. |
| `sha256` | yes | The SHA-256 of the whole zip, 64 hex digits. |
| `size` | yes | The zip's size in bytes (up to 512 MiB). |
| `url` | yes | `https://` address of `<id>-<version>.zip`. |
| `yanked` | no | `true` withdraws the version: the launcher shows it struck out and does not install it by default. |

[`index.schema.json`](index.schema.json) is the same description as a JSON Schema (for editors). The file has a `schema` number (now `1`); a launcher that does not know a newer number says so and shows the index it saved before. Unknown keys are refused by the validation script (a typo is worse than a refused pull request) and ignored by the launcher.

### Rules

- One entry per mod id. A new version of your mod is a new item in its `versions`, not a new entry.
- The package at `url` must be exactly the file with that `sha256`; never replace a published file, publish a new version.
- No misleading names: do not pretend to be another author or the official game. No mods that attack players, the game's anti-cheat or other people's servers.
- A mod that needs another mod says so in its `mod.toml` (`[dependencies]`); the launcher can only pull in dependencies that are in its cache.

## Maintainers

- Merge a pull request only when the `validate` check is green, the download works and the mod is what it says. The launcher is forgiving (it skips a broken entry or version and keeps the rest), so a mistake that slips through never breaks the index for everybody.
- Creating this repository: copy the contents of `docs/mod-index-template/` of Poe2ModdingProject (including `.github/`) into the root of `Distorbed678/Poe2Mods` (`index.json` must be at the root of the `main` branch).
- `updated` is a free text (a date); keep it current when you merge.

## Licence

The index is public data. The scripts are MIT licensed.
