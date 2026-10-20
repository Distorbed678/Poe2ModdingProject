# Launcher

Planned, nothing built yet. The work package is in [../plans/orchestration-2026-10.md](../plans/orchestration-2026-10.md) (phase 5).

Intended job: one small program per player (Tauri 2 with a Svelte UI, own repo) that does what `cargo xtask deploy-client` and `run-client` do for the developer ([setup.md](setup.md)), without this repo:
- **Server profiles:** cluster URL, account login through the web API ([../architecture/web-api.md](../architecture/web-api.md)), a stored token; a register flow ([../architecture/accounts.md](../architecture/accounts.md)).
- **Game install:** download client 0.5.5e through the player's own Steam login, then verify.
- **Deploy and launch:** the `poe-hook-deploy` library (gateway entry, DLLs, `poe-hook.toml` with the trusted-server list and the token, `steam_appid.txt`), natively on Windows or through Proton on Linux ([hosting.md](hosting.md)).
- **Mods:** read client mod manifests (`mod.toml`, spec in [../../modules/client-hook/docs/mod-manifest.md](../../modules/client-hook/docs/mod-manifest.md)) and enable them ([modding.md](modding.md)).
