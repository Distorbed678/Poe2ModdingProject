# Launcher

The player launcher: one small desktop program (Tauri 2 with a Svelte 5 UI) that does what `cargo xtask deploy-client` and `run-client` do for the developer ([setup.md](setup.md)), without this repository. Repo [Poe2Launcher](https://github.com/Distorbed678/Poe2Launcher) (private), the submodule `modules/launcher`. Its own README covers development; this page covers using it and how it fits the server.

What it does:
- **Servers:** profiles (a name and the cluster's web API address); shows what `GET /server` reports.
- **Account:** log in or register through the web API ([../architecture/web-api.md](../architecture/web-api.md)); the token sits in the OS keyring.
- **Install:** downloads client 0.5.5e from Steam with *your own* Steam login into the launcher's own folder, then verifies it.
- **Play:** deploys the hook ([../architecture/client-dll.md](../architecture/client-dll.md)), signs the game in without typing a password, starts it (Proton on Linux, native on Windows).
- **Mods:** installs, enables and orders mods by the `mod.toml` spec ([../../modules/client-hook/docs/mod-manifest.md](../../modules/client-hook/docs/mod-manifest.md)). The hook's mod loader loads the enabled ones (never in official mode); see [Mods](#mods).

## Download, install and update

Releases of the launcher are on the GitHub releases page of Poe2Launcher (once the repository is public; until then it is private and there is nothing to download, build it as below).

- **Linux:** download `Poe2.Launcher_<version>_amd64.AppImage`, `chmod +x` it and run it. Keep it where it is: the launcher updates this file in place. (AppImages need FUSE 2 on the system, or start it with `--appimage-extract-and-run`.)
- **Windows:** download and run `Poe2.Launcher_<version>_x64-setup.exe` (per-user NSIS installer, no administrator rights). Windows SmartScreen may warn because the installer is not code-signed (the update signature below is not a Windows code-signing certificate).
- **Updates:** the launcher looks for a newer release when it starts and on Settings, *Updates*, *Check for updates*. When one exists, a badge "Update <version> available" appears in the top bar and Settings shows the version and the release notes. *Download and install* downloads the new AppImage or installer, verifies its signature against the public key built into the launcher (`plugins.updater.pubkey` in `tauri.conf.json`), installs it (Windows: the installer runs in passive mode, with a progress bar and no questions) and restarts the launcher. A failed check (no release yet, repository private, offline) shows "No update info available" quietly; a failed download or signature check shows the error and changes nothing. Updating only works from an installed AppImage or NSIS install, not from a `tauri dev` or plain `cargo build` binary.
- The endpoint is `https://github.com/Distorbed678/Poe2Launcher/releases/latest/download/latest.json`; `releases/latest` ignores drafts and pre-releases (tags with a `-`).

## Building and running

Commands to build and run it (`npm ci`, `npm run tauri dev`): [setup.md](setup.md#launcher-build-and-test).

Linux needs the WebKitGTK stack (Arch: `webkit2gtk-4.1 gtk3 libsoup3 librsvg`; Debian: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libdbus-1-dev librsvg2-dev`), Node 22 and Rust. The result is `src-tauri/target/{debug,release}/poe2-launcher` plus deb, rpm and AppImage bundles. CI (Poe2Launcher, `.github/workflows/ci.yml`): Ubuntu runs `npm run check/test/build`, `cargo fmt --check`, `clippy -D warnings` and `cargo test`; Windows runs `cargo check`. The private submodules are fetched with two read-only deploy keys (repository secrets `HOOK_DEPLOY_KEY`, `PROTOCOL_DEPLOY_KEY`). Releases: see [Releasing](#releasing).

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

## Releasing

A release is a pushed tag; `.github/workflows/release.yml` of Poe2Launcher does the rest.

1. Set the same new version in `src-tauri/tauri.conf.json` (`version`), `src-tauri/Cargo.toml` and `package.json` (and `src-tauri/Cargo.lock` follows from a build), commit and push to `main`. The workflow refuses a tag that does not match all three.
2. Tag and push: `git tag v0.1.1 && git push origin v0.1.1` (in the Poe2Launcher repository). A tag with a `-` (`v0.2.0-rc1`) makes a pre-release, which the update endpoint ignores.
3. The workflow (a) checks the secrets and the version, (b) creates a **draft** release with notes generated from the commits since the previous release, (c) builds on `ubuntu-22.04` (`tauri build --bundles appimage`, so the AppImage runs on distributions back to that glibc) and `windows-latest` (`--bundles nsis`) with `tauri-action`, which signs the updater artifacts, attaches the AppImage, the `.exe` installer, their `.sig` files and a merged `latest.json`, and (d) publishes the release only when `latest.json` lists both `linux-x86_64` and `windows-x86_64`. A failed job leaves a draft; delete the draft and the tag, or re-run the failed jobs.
4. Nothing is pushed or released from a developer machine. To test locally what CI does: `TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/poe2-launcher.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat ~/.tauri/poe2-launcher.key.password)" npx tauri build --bundles appimage` in `modules/launcher` gives `src-tauri/target/release/bundle/appimage/*.AppImage` and `.AppImage.sig` (do not commit the outputs). The NSIS installer cannot be built on Linux without `makensis`: `cargo xwin build --target x86_64-pc-windows-msvc` (with `third_party/cargo-xwin`, `XWIN_CACHE_DIR=third_party/xwin-cache`) compiles the Windows binary as a check, the installer is CI-only. The workflows can be linted with `third_party/actionlint/actionlint` (downloaded by `third_party/fetch-tools.sh`).

**Secrets** (repository secrets of Poe2Launcher; the release workflow fails at once if one is missing):

| Secret | What | How it is made |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | the updater's private signing key (minisign format, the file's content) | `npx tauri signer generate -w ~/.tauri/poe2-launcher.key -p "$(cat ~/.tauri/poe2-launcher.key.password)"` once, in `modules/launcher`; then store the content of `~/.tauri/poe2-launcher.key` as the secret |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the key's password | a random string kept in `~/.tauri/poe2-launcher.key.password`; the same file's content is the secret |

Set them with `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/poe2-launcher.key` and `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD < ~/.tauri/poe2-launcher.key.password` (the project owner does this; the secrets are not created by the tooling). `HOOK_DEPLOY_KEY` and `PROTOCOL_DEPLOY_KEY` (read-only deploy keys, as for CI) stay until the repositories are public.

**Key location and care.** The key pair was generated locally in `~/.tauri/` (`poe2-launcher.key`, `poe2-launcher.key.pub`, `poe2-launcher.key.password`, mode 600, outside every git repository). Only the **public** key is in the repository (`plugins.updater.pubkey` in `tauri.conf.json`). Back the three files up somewhere private: if the private key is lost, installed launchers can never be updated again (their embedded public key no longer matches; players would have to reinstall). If it leaks, generate a new pair, put the new public key in `tauri.conf.json`, release once more signed with the **old** key (so existing installs accept it and learn the new key), then switch the secrets. Never put the key or password in a file in a repository, an issue or a log.

## Mods

Mods live in `<install>/poe-hook-mods/<id>/` (never in game files), as the spec says. The launcher's side of the contract with the DLL's mod loader (`modules/client-hook/poe-hook/src/mods.rs`, in the game since 2026-10-21) is `poe-hook-mods/state.toml`:

```toml
enabled = ["core-lib", "my-mod"]    # load order; the mods' own mod.toml files are never changed
```

(The loader's name for this file in the spec is `state.toml`, so the launcher does not use another name such as `mods.toml`.) The launcher validates every rule of the spec and lists all problems, checks `game.versions` against the install (an incompatible mod needs an explicit override to be enabled) and `game.hook_api` (the launcher offers 1), enables dependencies first with a semver check, refuses conflicts and ordering cycles, and orders by dependencies and `load.after/before` with your arrangement deciding free choices (*Auto order* = the spec's ties by id). Zip and folder installs refuse escaping paths and symlinks. The launcher only changes files; the loader in `version.dll` reads `state.toml` at the client start, loads the enabled mods that have an entry DLL in that order and skips (with a log line) a mod whose `game.hook_api` is not the loader's ABI version, so a mod's *hook API* must equal the number the launcher offers (1). The loader refuses a DLL mod that states no `game.hook_api`, which the launcher still lets you enable.

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
- Windows code signing (SmartScreen warning); macOS builds.
- The self-update has been built and the AppImage signature verified against the embedded public key locally; the release workflow, the NSIS installer and an installed v0.1.0 updating to v0.1.1 are untested until the first real release (WP E3, needs the repositories public and the two signing secrets).
