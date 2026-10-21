# Launcher

The player launcher: one small desktop program (Tauri 2 with a Svelte 5 UI) that does what `cargo xtask deploy-client` and `run-client` do for the developer ([setup.md](setup.md)), without this repository. Repo [Poe2Launcher](https://github.com/Distorbed678/Poe2Launcher) (private), the submodule `modules/launcher`. Its own README covers development; this page covers using it and how it fits the server.

What it does:
- **Servers:** profiles (a name and the cluster's web API address); shows what `GET /server` reports.
- **Account:** log in or register through the web API ([../architecture/web-api.md](../architecture/web-api.md)); the token sits in the OS keyring.
- **Install:** downloads client 0.5.5e from Steam with *your own* Steam login into the launcher's own folder, then verifies it.
- **Play:** deploys the hook ([../architecture/client-dll.md](../architecture/client-dll.md)), signs the game in without typing a password, starts it (Proton on Linux, native on Windows).
- **Mods:** installs, enables and orders mods by the `mod.toml` spec ([../../modules/client-hook/docs/mod-manifest.md](../../modules/client-hook/docs/mod-manifest.md)). The game cannot load them yet; see [Mods](#mods).

## Building and running

Commands to build and run it (`npm ci`, `npm run tauri dev`): [setup.md](setup.md#launcher-build-and-test).

Linux needs the WebKitGTK stack (Arch: `webkit2gtk-4.1 gtk3 libsoup3 librsvg`; Debian: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libdbus-1-dev librsvg2-dev`), Node 22 and Rust. The result is `src-tauri/target/{debug,release}/poe2-launcher` plus deb, rpm and AppImage bundles. CI (Poe2Launcher): Ubuntu runs `npm run check/test/build`, `cargo fmt --check`, `clippy -D warnings` and `cargo test`; Windows runs `cargo check`. The private submodules are fetched with two read-only deploy keys (repository secrets `HOOK_DEPLOY_KEY`, `PROTOCOL_DEPLOY_KEY`).

## One-time set-up

1. **Hook DLLs.** Settings, *Hook DLL directory*: a build of Poe2ClientHook holding `poe_hook.dll` and `poe_hook_core.dll`, normally `modules/client-hook/target/x86_64-pc-windows-msvc/release` (built with `build-dll`, [setup.md](setup.md#build-and-test)). Bundling them into the launcher is not done.
2. **Keyring (Linux).** Login tokens are stored only in the Secret Service (GNOME Keyring, or KWallet with its Secret Service interface switched on in System Settings). Without a provider the login is refused with an explanation, and the token is revoked again. Windows uses the Credential Manager.
3. **Linux:** Steam must be running when you press Play, and the Proton set in Settings (default `Proton - Experimental`) must exist under `<Steam>/steamapps/common`. The launcher only reads Proton from there.
4. **Server:** a server that has the web API on (`[web] enabled`, reachable from the player; use TLS outside a trusted network) and an account (`open_registration = true`, or `account create`).

## The flow

1. **Servers:** *Add server*, e.g. `127.0.0.1:8080` (`http://` is added, a pasted `/api/v1` is dropped). *Check* shows realm, client version, players, whether registration is open and the game address.
2. **Account:** *Log in* (`POST /login`, kind `launcher`, valid `[accounts] remember_days`) or *Create account* (only if registration is open; it logs in afterwards). *Log out* revokes the token on the server (and removes it locally even if the server is down, saying so).
3. **Install:** your Steam account name and password, optionally *Remember the Steam login*. A Steam Guard code is typed into the launcher when Steam asks. The download is about 150 GB.
4. **Play** (footer, for the selected server) once the game is installed and you are logged in.

### Install in detail

The version file is `versions/<version>.toml` of the main repo (the single definition of a supported build; the launcher embeds a byte-identical copy that `cargo xtask versions` keeps, see [new-game-version.md](new-game-version.md)). For 0.5.5e: app 2694490, depot 2694491 manifest 7366357142270402355 (content), depot 2694492 manifest 246779027001700092 (Windows exe), in that order into one directory (exe over content).

- **DepotDownloader 3.4.0**, the self-contained release (no .NET), is fetched from its GitHub release into the launcher's data directory, pinned by version and SHA-256 (`versions/depotdownloader.toml`); a mismatch deletes the download.
- It runs **once for both depots** (`-app 2694490 -depot 2694491 -manifest ... -depot 2694492 -manifest ... -username U -dir DIR`), so the Steam login and Steam Guard code are asked once. `-no-mobile` is added by default so a typed code is requested instead of the mobile-app confirmation (Settings). `-remember-password` is added only if you tick the box (then DepotDownloader keeps its own login token; the launcher still never stores the password).
- **The Steam password** is typed in the launcher, written once to the downloader's standard input when it asks for it, then wiped from memory. It is not on a command line, not logged, not saved. Steam Guard prompts are detected in the output (they have no line end) and shown as a code box.
- **Never the Steam install.** The launcher does not read or write `~/.local/share/Steam/steamapps/common/Path of Exile 2` or `compatdata/2694490`; an install directory inside any Steam library is refused. The default is `<data>/poe2-launcher/game/0.5.5e` (`~/.local/share/poe2-launcher/...`).
- **Verification** afterwards (and on *Verify files*): file count and total size floors, key files with exact sizes, SHA-256 of `PathOfExileSteam.exe`. **The reference values were measured on the `GameFiles/` copy in this repository, not on a launcher download**, so they are unconfirmed: the executable checksum is a warning (not a failure) until `exe_sha256_enforce = true` is set after the first real install matched. *Repair* re-runs the download with `-validate`.

### Play in detail

1. `GET /server` gives `login_host`, `login_port` (the public one), `instance_port` and `gateway_region`. If the server still advertises loopback while the launcher reached it elsewhere, the address the launcher used is taken.
2. `POST /api/v1/game-token` exchanges the launcher token for a one-off `game_remember` token. The launcher token itself is **not** written for the game: the game's login retires a token when it uses it (rolling tokens, [../architecture/accounts.md](../architecture/accounts.md#tokens)), which would log the launcher out.
3. `write_login_token` puts the 32-byte token and `[LOGIN] username=<account>, save_login=true` where the client reads its Remember Me ([../re/standalone-login.md](../re/standalone-login.md), section 4). The client logs in on its own when it starts.
4. `deploy` writes `gateway_list.txt` (one entry for this server), installs `version.dll` and `poe_hook_core.dll`, sets `poe-hook.toml` (`mode = "private"`, `login_mode = "standalone"`, `trusted_servers` = the server's login and instance `host:port`) and writes `steam_appid.txt`. Always private mode: official servers are never touched.
5. Start: Linux `proton run PathOfExileSteam.exe --success-event 0` with the launcher's own prefix (`<data>/proton-prefix`, created with `proton run wineboot -u` on first use; crash dumps are set aside as in `run-client`); Windows runs the exe directly. Output goes to `<data>/logs/game-<time>.log`.

## Mods

Mods live in `<install>/poe-hook-mods/<id>/` (never in game files), as the spec says. The launcher's side of the contract with the future loader is `poe-hook-mods/state.toml`:

```toml
enabled = ["core-lib", "my-mod"]    # load order; the mods' own mod.toml files are never changed
```

(The loader's name for this file in the spec is `state.toml`, so the launcher does not use another name such as `mods.toml`.) The launcher validates every rule of the spec and lists all problems, checks `game.versions` against the install (an incompatible mod needs an explicit override to be enabled) and `game.hook_api` (the launcher offers 1), enables dependencies first with a semver check, refuses conflicts and ordering cycles, and orders by dependencies and `load.after/before` with your arrangement deciding free choices (*Auto order* = the spec's ties by id). Zip and folder installs refuse escaping paths and symlinks. **The DLL loader does not exist yet**: enabling a mod only changes files.

## Testing the launcher against a local server

The real-server test needs a scratch server config (shifted ports, `open_registration = true`, an account made offline); commands: [setup.md](setup.md#launcher-build-and-test), config: [setup.md](setup.md#web-api).

The launcher's own tests already cover profiles with a mock keyring, login/logout/register and the whole Play preparation against a mock web API and a fake Proton; an ignored test downloads and starts the pinned DepotDownloader (never logs in).

## End-to-end test on a real machine

1. Start the server with the web API and an account (above, or the real config).
2. Start the launcher in dev mode ([setup.md](setup.md#launcher-build-and-test)); set the DLL directory (Settings).
3. Servers: add the server; Account: log in.
4. Install: enter your Steam account, answer the Steam Guard prompt, wait for "Done" and the verification table. Report any failed check; the warning on the executable checksum would mean the recorded value needs correcting.
5. Start Steam (Linux), press Play. Expected: the client opens on the realm and logs in by itself.

## Not done

- Bundling the hook DLLs; downloading them from a release.
- The Windows launch path is written but untested (no Windows machine); `cargo check` runs in CI.
- Verification reference values are unconfirmed (above); the Steam download and the Play launch have not been run by a real install yet.
- Mod loader (DLL side); mod signatures and packaging beyond a zip.
- Code signing and auto-update of the launcher itself.
