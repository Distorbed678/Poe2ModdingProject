# Launcher

The player launcher: one small desktop program (Tauri 2 with a Svelte 5 UI) that does what `cargo xtask deploy-client` and `run-client` do for the developer ([setup.md](setup.md)), without this repository. Repo [Poe2Launcher](https://github.com/Distorbed678/Poe2Launcher) (public), the submodule `modules/launcher`. Its own README covers development; this page covers using it and how it fits the server.

What it does (the screens are a sidebar with the pages, a title bar with the **game version dropdown**, and a Play bar at the bottom):
- **Game version dropdown (top of the window):** *Official (read-only)* at the top, then every private-server version this launcher supports, newest first. The choice drives the install folder and depots, file verification, the hook deploy, Play, the server-release filter of Host a server and the compatibility of the servers in the list ([below](#game-versions-and-the-official-mode)).
- **Servers:** profiles (a name and the cluster's web API address); shows what `GET /server` reports. Admins get an **Open web admin** button ([below](#open-web-admin)); plain `http://` to another machine needs your confirmation first ([below](#plain-http-warning)).
- **Account:** log in or register through the web API ([../architecture/web-api.md](../architecture/web-api.md)); the token sits in the OS keyring.
- **Host a server:** downloads the matching `poe-server` release (or takes a local package), sets it up, starts and stops it, streams its log, and adds it to Servers ([below](#host-a-server)).
- **Install:** downloads the selected client (a pinned private version such as 0.5.5e, or the current official build) from Steam with *your own* Steam login into the launcher's own folder, then verifies it.
- **Play:** deploys the hook ([../architecture/client-dll.md](../architecture/client-dll.md)), signs the game in without typing a password, starts it (Proton on Linux, native on Windows).
- **Settings:** per-version install folder, hook DLL folder, Proton, **Log out of Steam**, theme, window frame, *Minimize the launcher while the game is running*, the desktop entry (Linux AppImage) and launcher updates. **About:** version, supported game versions, disclaimer.
- **Mods:** installs, enables and orders mods by the `mod.toml` spec ([../../modules/client-hook/docs/mod-manifest.md](../../modules/client-hook/docs/mod-manifest.md)). The hook's mod loader loads the enabled ones (never in official mode); see [Mods](#mods).

## Download, install and update

Releases of the launcher are on the [GitHub releases page of Poe2Launcher](https://github.com/Distorbed678/Poe2Launcher/releases) (v0.1.0 is the first, 2026-10-21). Each has the Linux AppImage, the Windows installer, their `.sig` files and `latest.json`.

- **Linux:** download `Poe2.Launcher_<version>_amd64.AppImage`, `chmod +x` it and run it. Keep it where it is: the launcher updates this file in place. (AppImages need FUSE 2 on the system, or start it with `--appimage-extract-and-run`.)
- **Windows:** download and run `Poe2.Launcher_<version>_x64-setup.exe` (per-user NSIS installer, no administrator rights). Windows SmartScreen may warn because the installer is not code-signed (the update signature below is not a Windows code-signing certificate).
- **Updates:** the launcher looks for a newer release when it starts and on Settings, *Updates*, *Check for updates*. When one exists, a badge "Update <version> available" appears in the top bar and Settings shows the version and the release notes. *Download and install* downloads the new AppImage or installer, verifies its signature against the public key built into the launcher (`plugins.updater.pubkey` in `tauri.conf.json`), installs it (Windows: the installer runs in passive mode, with a progress bar and no questions) and restarts the launcher. A failed check (no release yet, repository private, offline) shows "No update info available" quietly; a failed download or signature check shows the error and changes nothing. Updating only works from an installed AppImage or NSIS install, not from a `tauri dev` or plain `cargo build` binary.
- **App identifier:** `org.poe2modding.launcher` (`tauri.conf.json`). It names the window class and the webview's own storage folder, not the launcher's data: the keyring service and the config and data folders are fixed names (`poe2-launcher`, listed under "Data and secrets" in the launcher README), so they do not depend on it.
- The endpoint is `https://github.com/Distorbed678/Poe2Launcher/releases/latest/download/latest.json`; `releases/latest` ignores drafts and pre-releases (tags with a `-`).

## Building and running

Commands to build and run it (`npm ci`, `npm run tauri dev`): [setup.md](setup.md#launcher-build-and-test).

Linux needs the WebKitGTK stack (Arch: `webkit2gtk-4.1 gtk3 libsoup3 librsvg`; Debian: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libdbus-1-dev librsvg2-dev`), Node 22 and Rust. The result is `src-tauri/target/{debug,release}/poe2-launcher` plus deb, rpm and AppImage bundles. CI (Poe2Launcher, `.github/workflows/ci.yml`): Ubuntu runs `npm run check/test/build`, `cargo fmt --check`, `clippy -D warnings` and `cargo test`; Windows runs `cargo check`. The submodules are public and fetched by `actions/checkout`. Releases: see [Releasing](#releasing).

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

## Game versions and the official mode

The version dropdown in the title bar lists every target the launcher can work with. The list comes from the embedded `versions/<version>.toml` files (one per supported client build, copies of the main repo's files, [new-game-version.md](new-game-version.md#the-launcher-and-the-version-dropdown)), so a second supported build appears as a second entry without any UI change. The choice is saved in the launcher's `settings.json` (`game_version`; empty or a version this build no longer knows means the newest) and survives restarts. It cannot be changed while the game is being downloaded, starting or running, or while the hosted server runs.

What follows the selection:

| Part | Private version `<v>` | Official (read-only) |
|---|---|---|
| Install folder | `<data>/game/<v>` or the folder set for that version in Settings (each version has its own) | `<data>/game/official`, separate from every private version |
| Download | the depots with the pinned manifests of `versions/<v>.toml` | the same depots **without manifests**: the current official build |
| Verify | file counts, key file sizes and the exe checksum of that version file | the main files exist and the size floors (the build is not pinned) |
| Play | the selected server profile, auto-login token, `mode = "private"` | no server, no token, `mode = "official"` ([below](#official-read-only)) |
| Servers, Account, Host a server, Mods | available; a server that reports another client version is marked *incompatible* | hidden (these are for private versions) |
| Host a server | downloads the `poe-server` package whose name carries this game version | not possible |

**Server compatibility.** *Check* (and a quiet check at start) reads `GET /server`. A server whose client version equals the selection is *compatible*. A server that reports another version this launcher supports shows *needs client X* with a one-click *Switch to X*, and Play is blocked for it. A server reporting a version the launcher does not know is marked *incompatible* with a warning; Play is not blocked then (the check can be wrong).

### Official (read-only)

For observing a session on the official servers, exactly what `cargo xtask deploy-client --official` does for a developer ([official-mode.md](official-mode.md)); read that page's risk section first. Selecting it and pressing Play:

1. asks once for the warning to be accepted (the risk text of official-mode.md and an "I understand" box; saved as `official_ack`; the backend refuses to start without it too);
2. deploys the hook with `mode = "official"`, `login_mode = "steam"`, an **empty** `trusted_servers`, the original `gateway_list.txt` put back from `gateway_list.txt.orig`, no position sync port and no output folder. The launcher never writes a trusted server or a private gateway for this mode, and the hook's read-only latch is not touched;
3. starts the game without the standalone-login argument, with its own Steam login (Linux: through Proton in a **separate** prefix `<data>/proton-prefix-official`, so no private login or realm choice leaks into it; Steam must be running). The launcher sends nothing to any server and logs in nowhere.

If the game was updated and the hook's signature table does not match the new build, the hook switches its features off by itself (the log line says `NOT verified`): expected, the game just runs without the observer. The unit tests cover the deploy files (official versus private) with a temporary game folder; it has not been played against the official servers from the launcher.

The Steam download for the official build is the same *Install* page; *Download again* updates to whatever Steam currently serves, *Repair* re-checks files.

### Install in detail

The version file (the selected version's) is `versions/<version>.toml` of the main repo (the single definition of a supported build; the launcher embeds a byte-identical copy that `cargo xtask versions` keeps, see [new-game-version.md](new-game-version.md)). For 0.5.5e: app 2694490, depot 2694491 manifest 7366357142270402355 (content), depot 2694492 manifest 246779027001700092 (Windows exe), in that order into one directory (exe over content).

- **DepotDownloader 3.4.0**, the self-contained release (no .NET), is fetched from its GitHub release into the launcher's data directory, pinned by version and SHA-256 (`versions/depotdownloader.toml`); a mismatch deletes the download.
- It runs **once for both depots** (`-app 2694490 -depot 2694491 -manifest ... -depot 2694492 -manifest ... -username U -dir DIR`), so the Steam login and Steam Guard code are asked once. `-no-mobile` is added by default so a typed code is requested instead of the mobile-app confirmation (Settings). `-remember-password` is added only if you tick the box (then DepotDownloader keeps its own login token; the launcher still never stores the password).
- **Remembered Steam login and *Log out of Steam* (Settings).** DepotDownloader 3.4.0 keeps the token in `account.config`, in .NET's isolated storage (`<local data>/IsolatedStorage/<id>/<id>/Url.<hash>/AssemFiles/account.config`). On Linux the launcher starts it with `XDG_DATA_HOME=<data>/steam-login`, so that file lives in the launcher's own data folder; *Log out of Steam* (with a confirmation) deletes it and clears the saved account name and the *remember* switch. The Settings card says *Logged in as X* or *Not logged in*. On Windows .NET asks the system for the folder (no environment override), so the file is under `%LOCALAPPDATA%\IsolatedStorage` and only files named `account.config` at the isolated-storage depth are removed (that also logs out any other DepotDownloader the same Windows user runs). Never the Steam client or its files. A login remembered by an older launcher version sits in the shared `~/.local/share/IsolatedStorage` and is not used any more (type the password once; delete that old file by hand if you want).
- **The Steam password** is typed in the launcher, written once to the downloader's standard input when it asks for it, then wiped from memory. It is not on a command line, not logged, not saved. Steam Guard prompts are detected in the output (they have no line end) and shown as a code box.
- **Never the Steam install.** The launcher does not read or write `~/.local/share/Steam/steamapps/common/Path of Exile 2` or `compatdata/2694490`; an install directory inside any Steam library is refused. The default is `<data>/poe2-launcher/game/<version>` (`~/.local/share/poe2-launcher/...`), one folder per game version.
- **Verification** afterwards (and on *Verify files*): file count and total size floors, key files with exact sizes, SHA-256 of `PathOfExileSteam.exe`. **The reference values were measured on the `GameFiles/` copy in this repository, not on a launcher download**, so they are unconfirmed: the executable checksum is a warning (not a failure) until `exe_sha256_enforce = true` is set after the first real install matched. *Repair* re-runs the download with `-validate`.

### Play in detail

1. `GET /server` gives `login_host`, `login_port` (the public one), `instance_port` and `gateway_region`. If the server still advertises loopback while the launcher reached it elsewhere, the address the launcher used is taken.
2. `POST /api/v1/game-token` exchanges the launcher token for a one-off `game_remember` token. The launcher token itself is **not** written for the game: the game's login retires a token when it uses it (rolling tokens, [../architecture/accounts.md](../architecture/accounts.md#tokens)), which would log the launcher out.
3. `write_login_token` puts the 32-byte token and `[LOGIN] username=<account>, save_login=true` where the client reads its Remember Me ([../re/standalone-login.md](../re/standalone-login.md), section 4). The client logs in on its own when it starts.
4. `deploy` writes `gateway_list.txt` (one entry for this server), installs `version.dll` and `poe_hook_core.dll`, sets `poe-hook.toml` (`mode = "private"`, `login_mode = "standalone"`, `trusted_servers` = the server's login and instance `host:port`) and writes `steam_appid.txt`. Private mode for a private version; the official mode has its own deploy ([above](#official-read-only)).
5. Start: Linux `proton run PathOfExileSteam.exe --success-event 0` with the launcher's own prefix (`<data>/proton-prefix`, created with `proton run wineboot -u` on first use; crash dumps are set aside as in `run-client`); Windows runs the exe directly. Output goes to `<data>/logs/game-<time>.log`.
6. **Minimize while playing** (Settings, off by default): when on, the launcher window minimizes when the game process starts and is shown and focused again when it ends (only if the launcher was the one that minimized it).

## Window, icon and desktop entry

- **Frame.** The window has no system decorations: the launcher draws its own title bar (icon, name, the version dropdown, update badge, minimize / maximize-restore / close; drag the bar to move, double-click to maximize) and eight resize handles on the edges. It is square with a 1 px border: a transparent window with rounded corners was tried and did not paint reliably under WebKitGTK on Wayland (parts of the page stayed empty), so it is not used. Settings, *Use the launcher's own window frame*, switches to the system title bar and borders if the custom frame misbehaves on your desktop.
- **Icon.** An original mark (a glowing doorway under an arch with a keystone rune; source `modules/launcher/assets/icon.svg`, generated sizes in `src-tauri/icons/` with `npx tauri icon assets/icon.svg`). Windows uses `icon.ico`. It contains nothing of Path of Exile or Grinding Gear Games.
- **Linux/Wayland: the icon comes from a `.desktop` file.** A Wayland compositor shows the icon of the `.desktop` file that matches the window's `app_id`, and the launcher's `app_id` is its identifier `org.poe2modding.launcher` (the program name is set to it before GTK starts). The `.deb`/`.rpm` packages install a matching entry. The **AppImage** has none on your system (its own `.desktop` file, with `StartupWMClass=org.poe2modding.launcher` and the icons, is inside the image for AppImageLauncher and similar tools). Either use such a tool, or open **Settings, Desktop entry, Install desktop entry**: it writes `~/.local/share/applications/org.poe2modding.launcher.desktop` (`Exec` = the AppImage file, found through `$APPIMAGE`, or the running binary) and the icon to `~/.local/share/icons/hicolor/{32x32,128x128,256x256,512x512}/apps/` and `scalable/apps/`; *Remove* deletes those files again. If you move the AppImage the card says the entry points elsewhere: press *Update desktop entry*.

## Host a server

The **Host a server** page runs a private server on the machine you play on. It is WP F2 of [../plans/orchestration-2026-10b.md](../plans/orchestration-2026-10b.md); the package it uses is made by the server release CI ([releasing-server.md](releasing-server.md)). Hosting works on Linux and Windows. Everything the page does is also listed, with the code, in the launcher README.

1. **Server program.** *Download the latest release* asks the GitHub releases of `Distorbed678/Poe2ModdingProject` for tags `server-v*` and takes the newest stable release whose package is named `poe-server-v<ver>-game-<your game version>-<linux|windows>-x86_64.tar.gz` (Linux) or `.zip` (Windows). The `.sha256` file of the release is read first, the archive is downloaded, and its SHA-256 must match or the download is deleted and nothing is installed. Until the repository is public GitHub answers 404 for a release list: the page then says so and points to the alternative. *Use a package file...* or *Use a package folder...* takes a package you have: an archive (checked against a `.sha256` file next to it when there is one, otherwise installed with a note that it was not checked) or an unpacked package folder (copied). Both routes check the package itself: `release.json` must say your OS and the game version your launcher installs, and the files the server needs from its own folder must be there. Archives are unpacked with path, link and size checks. The package lands in `<data>/poe2-launcher/server/packages/<name>`; an older package is removed after a newer one is installed, the database stays (it lives in `<data>/poe2-launcher/server/data/`). No game files are in a package.
2. **Set up.** The form asks for:
   - the **server name** (shown to players, and the name of the profile that is added to Servers) and the **cluster id** (login tokens are bound to it);
   - the three **ports**: login (TCP), game (TCP and UDP, the client hook's side channel uses the same number) and web (TCP, the web API and the web admin). The page refuses ports that something else already uses;
   - the **public host**, the address players use to reach this computer: *Use my LAN address* fills in the address of the interface your default route uses (nothing is sent to find it); *Look up my public IP* asks `api.ipify.org` and happens **only when you press that button**. The page lists the ports to forward in a router for players outside your network and links to [hosting.md](hosting.md);
   - **open registration** (anyone can create an account) and whether the **web API is reachable from other computers** (binds `0.0.0.0` instead of loopback; it is plain http, see [hosting.md](hosting.md#tls-reverse-proxy-for-the-web-api));
   - the **admin account**: a name and a password (at least 8 characters, typed twice).

   *Set up and start* writes `config/server.toml` into the package folder from the package's own `config/server.example.toml` (only these keys change: `game_dir` = the launcher's installed game, `[network]` and the two binds, `[cluster] id`, `[gateway] display_name`, `[web]` enabled, bind and `static_dir = "web-admin"`, `[accounts] open_registration`, `[database] path` below the launcher's folder; every comment and other key stays), then creates the admin (see below), starts the server, waits until its web API answers, adds the profile `http://127.0.0.1:<web port>` to Servers and logs in with the new account. The game must be installed first (Install page): the server reads its data from there. Later the same form is *Apply settings* (the server must be stopped); an empty password keeps the accounts as they are.
3. **The admin account.** A password must never be on a command line (any user of the machine can read it in the process list), so the launcher uses the server's own offline commands and gives the password on standard input: it runs `poe-server account create <name> --password-stdin` in the package folder with the new `config/server.toml` (the password is written to the child's standard input as one line, then the pipe is closed and the launcher's copy is wiped), and then `poe-server account role <name> admin`, which carries no secret. Both commands work on the database directly; **the server is not started** for this (the setup used to start it once, privately, with a loopback config and open registration; that path is gone). If the name already exists the create step reports it, and the account is just promoted and its password stays. A password with a line break is refused. The password is not stored, not in the config, not in the log, not in an environment variable, and not in the process list.
4. **Run.** *Start* and *Stop* (a stop is SIGTERM on Linux, with a hard kill if the server has not ended after 8 seconds; on Windows it is a hard stop), the state (starting while the game data loads, running when the web API answers, the number of players online), *Start this server when the launcher starts* (autostart), and the **server log**: the process's standard output and error, shown live (the last 2000 lines), also written to `<data>/poe2-launcher/server/server.log` (replaced at each start). **The server runs while the launcher is open; closing the launcher stops it.** A server that ends by itself is shown as stopped unexpectedly with its exit code and the last log lines.

Players outside your network add your server as `<public host>:<web port>` (use https if you put a TLS proxy in front), install the game with their own launcher and play ([hosting.md](hosting.md#what-a-remote-player-needs)).

Settings are kept in `<config>/poe2-launcher/host.json` (no secrets).

## Open web admin

On a server profile whose logged-in account is an **admin** (the Servers page shows an *admin* badge and the button, the Account page the role and the button; the Host page has one too), **Open web admin** opens the server's web admin in your system browser already signed in, without typing the password again.

- The role comes from the login (`POST /login` returns the account with its role) and is re-read with `GET /api/v1/me` when you press *Check*, when the launcher starts, and every time you press the button, so a promotion or demotion shows without logging in again.
- The button asks `POST /api/v1/web-token` (launcher token of an admin only; the server answers with a single-use token valid for `[web] web_token_ttl_secs`, 60 seconds by default) and then opens `<server address>/#/token-login?t=<token>` with the Tauri opener. The token travels in the URL fragment, which the browser never sends to the server; the web admin removes it from the address bar and redeems it once ([../architecture/web-api.md](../architecture/web-api.md#opening-the-web-admin-from-the-launcher)).
- The launcher checks that the link the server returned is exactly that form, with the token it was just given, before it opens anything; it never logs the link. A player account, an old server without the endpoint, a refused request and a missing browser each give their own message and open nothing.

## Plain http warning

A password or login token sent over `http://` can be read by anyone on the way (audit finding S-03, [../plans/public-release-audit.md](../plans/public-release-audit.md)).

- To **this computer** (`localhost`, `127.0.0.1`, `::1`) plain http is fine and needs nothing.
- To **any other address** the launcher shows a warning when you add or edit the server (a checkbox *I understand* is required before it saves) and, for a server that was added earlier, a notice on the Servers and Account pages with *I understand, allow plain http for this server*. Until you agree for that address the launcher sends nothing there: no log in, no register, no token for Play or the web admin. The backend enforces this, not just the UI. The agreement is per address: changing the address withdraws it.
- A bare `host:port` you type gets `http://` when it is this computer or a local network address (192.168.x.x, 10.x.x.x, 172.16-31.x.x, `*.local`, single-label names) and `https://` for everything else. An explicit `http://` is kept.
- The server you host from the launcher uses plain http on loopback. If you open its web API to other computers, put TLS in front ([hosting.md](hosting.md#tls-reverse-proxy-for-the-web-api)) and use the `https://` address in the players' launchers.

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

Set them with `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/poe2-launcher.key` and `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD < ~/.tauri/poe2-launcher.key.password` (the project owner does this; the secrets are not created by the tooling). Nothing else is needed: the submodules (client-hook, protocol) are public and checked out with `actions/checkout`; the former read-only deploy keys and `SUBMODULE_TOKEN` were deleted on 2026-10-21 (WP D3).

**Key location and care.** The key pair was generated locally in `~/.tauri/` (`poe2-launcher.key`, `poe2-launcher.key.pub`, `poe2-launcher.key.password`, mode 600, outside every git repository). Only the **public** key is in the repository (`plugins.updater.pubkey` in `tauri.conf.json`). Back the three files up somewhere private: if the private key is lost, installed launchers can never be updated again (their embedded public key no longer matches; players would have to reinstall). If it leaks, generate a new pair, put the new public key in `tauri.conf.json`, release once more signed with the **old** key (so existing installs accept it and learn the new key), then switch the secrets. Never put the key or password in a file in a repository, an issue or a log.

**First real run (2026-10-21, WP E3).** Result: v0.1.0 and v0.1.1 were released by the workflow, and an installed v0.1.0 updated itself to v0.1.1 on Linux.

- *What was fixed:* the first v0.1.0 tag failed in `create-release`: `gh release create --draft` succeeded, but the list API did not show the new draft yet ("release for v0.1.0 not found"). The step now asks up to 10 times, 3 seconds apart (launcher commit 7107277). No release had been published, so the draft and the tag were deleted and `v0.1.0` was set again on the fixed commit. The CI switch to public checkouts (`actions/checkout` with `submodules: recursive`) worked at once on both runners.
- *Release contents (both versions):* `Poe2.Launcher_<v>_amd64.AppImage` (about 83 MB) and `.sig`, `Poe2.Launcher_<v>_x64-setup.exe` (about 3 MB) and `.sig`, `latest.json` with `linux-x86_64`, `linux-x86_64-appimage`, `windows-x86_64` and `windows-x86_64-nsis`. The URLs in `latest.json` are GitHub API asset URLs (`api.github.com/repos/.../releases/assets/<id>`); the updater downloads them anonymously, which works because the repository is public. A whole release run takes about 7 minutes.
- *Update test (Linux):* the v0.1.0 AppImage was downloaded from the release into `runtime/e3/` and started: Settings said "Installed: 0.1.0, The launcher is up to date." After v0.1.1 was released, the same file, started again, showed the badge "Update 0.1.1 available" and in Settings "Update available: 0.1.1"; *Download and install* replaced the AppImage file in place (new SHA-256, same name), the launcher restarted and Settings showed "Installed: 0.1.1, The launcher is up to date." (About: Launcher 0.1.1). The test drove the UI through the WebKit remote inspector (`WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:9334` in the environment, a WebSocket client sending `Runtime.evaluate` to the page target), with the launcher's config and data in `runtime/e3/xdg/`. No game client or Play button was involved.
- *Windows:* the NSIS installer is built and signed by the same run and the Windows CI job passes, but nothing was installed or updated on Windows (no Windows machine here). The Windows update (passive NSIS install over the old version) is **untested**.
- *Not tried:* an offline start and a tampered download (the signature check is covered by the plugin; the UI's error state by unit tests).

## Mods

Mods live in `<install>/poe-hook-mods/<id>/` (never in game files), as the spec says. The launcher's side of the contract with the DLL's mod loader (`modules/client-hook/poe-hook/src/mods.rs`, in the game since 2026-10-21) is `poe-hook-mods/state.toml`:

```toml
enabled = ["core-lib", "my-mod"]    # load order; the mods' own mod.toml files are never changed
```

(The loader's name for this file in the spec is `state.toml`, so the launcher does not use another name such as `mods.toml`.) The launcher validates every rule of the spec and lists all problems, checks `game.versions` against the install (an incompatible mod needs an explicit override to be enabled) and `game.hook_api` (the launcher offers 1), enables dependencies first with a semver check, refuses conflicts and ordering cycles, and orders by dependencies and `load.after/before` with your arrangement deciding free choices (*Auto order* = the spec's ties by id). Zip and folder installs refuse escaping paths and symlinks. The launcher only changes files; the loader in `version.dll` reads `state.toml` at the client start, loads the enabled mods that have an entry DLL in that order and skips (with a log line) a mod whose `game.hook_api` is not the loader's ABI version, so a mod's *hook API* must equal the number the launcher offers (1). The loader refuses a DLL mod that states no `game.hook_api`, which the launcher still lets you enable.

## Testing the launcher against a local server

The real-server test needs a scratch server config (shifted ports, `open_registration = true`, an account made offline); commands: [setup.md](setup.md#launcher-build-and-test), config: [setup.md](setup.md#web-api).

The launcher's own tests already cover profiles with a mock keyring, login/logout/register and the whole Play preparation against a mock web API and a fake Proton; the plain-http guard and Open web admin against a mock web API (the browser is a recorder, nothing is opened); finding, downloading, verifying and unpacking server packages against a mock GitHub API; an ignored test downloads and starts the pinned DepotDownloader (never logs in).

**Hosting against a real server package** (an ignored test, no game client involved): build a package as in [releasing-server.md](releasing-server.md#checking-a-package-locally), then

```sh
cd modules/launcher/src-tauri
POE2_TEST_SERVER_PACKAGE=<out dir>/poe-server-v<ver>-game-0.5.5e-linux-x86_64.tar.gz \
POE2_TEST_GAME_DIR=<a game install, read only; the GameFiles/ copy works> \
  cargo test --test host_flow real_package -- --ignored --nocapture
```

It installs the package (checksum verified), sets it up in a temp launcher folder with ports 29500-29502 and a temp database (the admin is made by the one-shot `account create --password-stdin` and `account role` commands with a random throwaway password; the server is not started for it), starts it, waits for the web API, checks status, the log tail and `curl /api/v1/server`, logs in as the new admin (role `admin`), asks the real server for a web sign-in link (recorded, never opened), stops it and checks that the ports are free and the password is in no file. It takes about 15 seconds.

## End-to-end test on a real machine

1. Start the server with the web API and an account (above, or the real config).
2. Start the launcher in dev mode ([setup.md](setup.md#launcher-build-and-test)); set the DLL directory (Settings).
3. Servers: add the server; Account: log in.
4. Install: enter your Steam account, answer the Steam Guard prompt, wait for "Done" and the verification table. Report any failed check; the warning on the executable checksum would mean the recorded value needs correcting.
5. Start Steam (Linux), press Play. Expected: the client opens on the realm and logs in by itself.

## Not done

- The custom window frame, the version dropdown, the official mode, the Steam log-out, the minimize option and the desktop entry have been built and unit-tested; the frame and the app id were checked in a real KWin Wayland session (window class `org.poe2modding.launcher`, no server-side decoration), but resizing by dragging, the minimize/restore around a running game and the official Play have not been exercised by hand, and none of it has run on Windows.
- Bundling the hook DLLs; downloading them from a release.
- Host a server: the real GitHub download has not run against a published `server-v*` release (none exists yet); the Windows paths (zip, `taskkill`, no console window) compile but have not run; the in-app page has not been clicked through on a desktop, its logic is covered by unit tests and the backend by the tests above.
- The Windows launch path is written but untested (no Windows machine); `cargo check` runs in CI.
- Verification reference values are unconfirmed (above); the Steam download and the Play launch have not been run by a real install yet.
- Mod loader (DLL side); mod signatures and packaging beyond a zip.
- Windows code signing (SmartScreen warning); macOS builds.
- The self-update has been built and the AppImage signature verified against the embedded public key locally; the release workflow, the NSIS installer and an installed v0.1.0 updating to v0.1.1 are untested until the first real release (WP E3, needs the repositories public and the two signing secrets).
