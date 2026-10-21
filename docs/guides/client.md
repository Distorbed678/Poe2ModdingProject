# Client and install guide

How a player gets the game client onto a private server of this project: through the launcher, or by hand with `cargo xtask`, on Linux (Proton) or Windows. For running a server for others see [hosting.md](hosting.md); for developers [setup.md](setup.md) has every command.

**Unofficial.** This project is not affiliated with or endorsed by Grinding Gear Games. No game files are distributed: you need your own legal copy of Path of Exile 2, on the build the server supports (**0.5.5e**; the exact build and its executable checksum are in `versions/0.5.5e.toml`). The client hook is a DLL that is loaded into the game; the game's anti-cheat may detect it. Use it only for the private server and never on an account you care about; if you want to use it to observe an official session, read [official-mode.md](official-mode.md) first ([overview](#going-back-to-the-official-game)). See the [disclaimer](../../README.md#disclaimer).

## What gets installed

| Piece | What it is | Where |
|---|---|---|
| The game (0.5.5e) | Your own copy of the client files | a folder you choose; never the folder Steam keeps your real install in |
| `version.dll` | The hook loader (our `poe_hook.dll`, under the name the game loads from its own folder), plus `poe_hook_core.dll` (features and the in-game overlay) | next to `PathOfExileSteam.exe` |
| `poe-hook.toml` | The hook's config: mode, login mode, the servers it trusts | next to the exe |
| `gateway_list.txt` | The realm list, replaced by one entry that points at your server (the original is kept as `gateway_list.txt.orig`) | next to the exe |
| `steam_appid.txt` | Makes the Steam API start as Path of Exile 2 | next to the exe |
| a login token (optional) | Your saved login, so the client signs in without typing | the Documents folder of the game's prefix/profile |

Everything the deploy changes is inside the game copy; `cargo xtask revert-client` and the launcher's uninstall put it back.

## Option 1: through the launcher (recommended)

The launcher does all of the above for you: it downloads client 0.5.5e from Steam with your own Steam login into its own folder, installs the hook, writes the config for the server you pick, signs your account in and starts the game. Download, install and update of the launcher itself, the one-time set-up (hook DLL directory, Linux keyring, Proton) and the step-by-step flow are in [launcher.md](launcher.md). In short:

1. Get the launcher: Linux AppImage or Windows installer from the Poe2Launcher releases ([launcher.md](launcher.md#download-install-and-update)).
2. **Servers**: add your server's web API address (the server owner gives it to you, for example `https://play.example.org`); **Account**: log in or create an account.
3. **Install**: enter your Steam account (the game is about 150 GB; Steam Guard asks for a code), wait for the verification.
4. **Play**. On Linux, Steam must be running and a Proton must be installed (the default is "Proton - Experimental").

The launcher never touches your real Steam install of Path of Exile 2 and never plays on official servers.

## Option 2: by hand (`cargo xtask`)

For developers and people who build from source. You need a checkout of the main repository with its submodules ([setup.md](setup.md#clone)), the tools ([setup.md](setup.md#tools)) and a copy of the game files.

### The game files

Copy a complete 0.5.5e install into `GameFiles/` of the checkout (the folder with `PathOfExileSteam.exe` and `Bundles2`). Copy it, do not point the tooling at Steam's own folder: the deploy replaces `gateway_list.txt` and adds DLLs, which would change your real install and make Steam "repair" it. The copy must be the same build as the server's `[game] version`. The checkout's git ignores `GameFiles/`.

### Linux (Proton)

Requirements: Steam running (without it the client exits with `Unable to init Steam API`), Proton Experimental installed through Steam, `rustup` (the pinned toolchain is installed on the first build), clang and lld for the Windows cross build.

```sh
third_party/fetch-tools.sh                  # once: tools, cargo-xwin, the pinned toolchain
cargo xtask build-dll                       # the hook DLLs for the Windows target
cargo xtask deploy-client --no-build        # gateway entry, version.dll, poe-hook.toml; reads [network] public_host and the ports from config/server.toml
cargo xtask run-client                      # starts the game through Proton in its own prefix (third_party/proton-prefix)
```

`deploy-client` reads the server address and ports from the config it is given: for someone else's server copy their `config/server.example.toml` values into a config of your own (only `[network]` and `[gateway]` matter) and pass `--config <file>` (or `POE_CONFIG=<file>`), exactly as the player section of [hosting.md](hosting.md#what-a-remote-player-needs) says. The default login is the account name and password screen (`--login-mode standalone`); for a development server that uses the Steam machine-hash login deploy with `--login-mode steam` ([setup.md](setup.md#developer-config)). Each deploy lists what it did; `cargo xtask revert-client` restores the original files.

To start the game from Steam instead of `run-client` (Steam overlay, controller), add `GameFiles/PathOfExileSteam.exe` as a non-Steam game with the launch options from [setup.md](setup.md#playing-from-steam-non-steam-game-shortcut).

### Windows

The launcher is the supported way on Windows. A manual install means the same four things by hand, and **has not been tested on Windows yet**:

1. Build the DLLs on Windows with the pinned toolchain (`rustup`, the MSVC build tools): in `modules/client-hook`, `cargo build --release -p poe-hook -p poe-hook-core --target x86_64-pc-windows-msvc`. Output: `target/x86_64-pc-windows-msvc/release/poe_hook.dll` and `poe_hook_core.dll`.
2. Copy your game folder somewhere (not the Steam library folder). Copy `poe_hook.dll` as `version.dll` and `poe_hook_core.dll` next to `PathOfExileSteam.exe`, and `modules/client-hook/poe-hook/poe-hook.toml` as `poe-hook.toml`.
3. In `poe-hook.toml` set `mode = "private"`, `login_mode = "standalone"` and `trusted_servers = ["<server host>:<login port>", "<server host>:<instance port>"]` (IPv4 literals; resolve a name first). In `gateway_list.txt` (UTF-16 little endian) keep one line for your server, in the format the original file uses; keep the original as `gateway_list.txt.orig`.
4. Create `steam_appid.txt` containing `2694490`, make sure Steam is running, and start `PathOfExileSteam.exe`.

The deploy code in `poe-hook-deploy` (used by `cargo xtask deploy-client`) writes exactly these files and also builds on Windows; running it there is the likely shortcut, but nobody has done so yet.

## First start

1. The login screen shows your realm (the `[gateway] display_name` of the server, for example "Local Server"). With the default `standalone` login you see the account name and password screen. Sign in with the account the server owner made for you; **Remember Me** works (the launcher signs in without a password by itself).
2. Create a character, then play. The server decides the classes, areas and leagues it offers from the game tables; it is a private server with its own database, so characters and items are not shared with official servers.
3. Controls follow the game's own options. For WASD, set the input mode to WASD in the options; for click to move use the mouse input mode.
4. **Insert** shows and hides the hook's overlay (state, packets, commands). Server commands start with `!` in chat for admins ([server-admin.md](server-admin.md)). **F8** takes a game screenshot into the profile's `Screenshots` folder.

## Updating

- **The game.** When GGG ships a new build the private server keeps working only with the build it supports. Do not update Steam's copy for it; wait for a server release and a launcher that name the new build ([new-game-version.md](new-game-version.md)). The launcher installs the build the server needs.
- **The hook.** After you rebuild or update the DLLs run `cargo xtask deploy-client` again (the launcher does it on every Play). Your edits in `poe-hook.toml` are kept across deploys.

## Going back to the official game

A client deployed for a private server refuses connections to anything but loopback and your server (`block_external`), so it cannot reach official services by accident, but it also cannot play the official game. To play the official game, use Steam's own, untouched install (the project never modifies it). If you want to remove the hook from a copy: `cargo xtask revert-client` (by hand: delete `version.dll`, `poe_hook_core.dll`, `poe-hook.toml`, `steam_appid.txt` and rename `gateway_list.txt.orig` back to `gateway_list.txt`; the launcher's uninstall does it).

The project also has an `--official` mode that lets the hook *observe* a session on the official servers (it sends and changes nothing, but it patches game code in memory to do so, and the anti-cheat may detect that). What it does, the risk to your account and the steps are in [official-mode.md](official-mode.md); the launcher does not offer it.

## Troubleshooting

| Symptom | Likely cause | What to do |
|---|---|---|
| The client exits at once, log says `Unable to init Steam API` | Steam is not running | Start Steam first |
| The login screen shows the official realms, or the client never contacts the server | `gateway_list.txt` was not replaced, or the client remembered an earlier realm | Deploy again; `cargo xtask select-gateway` resets the remembered gateway (Proton prefix); check `gateway_list.txt` |
| "There has been a patch that you need to update to" | The server does not know your executable (another build) | Use the 0.5.5e build the server names; the server owner can allow unknown builds ([hosting.md](hosting.md#troubleshooting)) |
| Login screen offers Steam login, not account and password | `login_mode = "steam"` in `poe-hook.toml` | Set `login_mode = "standalone"` or deploy without `--login-mode steam` |
| Account does not exist / wrong password | No account yet, or a typo | Ask the server owner, or "Create account" in the launcher if registration is open. Too many failures lock the account for a few minutes |
| Stuck on the loading screen, or a freeze right after entering | Port 6112 (TCP) not reachable, or the server does not have the area's layout yet | Tell the server owner; see [hosting.md](hosting.md#troubleshooting) |
| The game crashes | Many possible causes | Logs: `GameFiles/poe-hook.log`, `GameFiles/logs/poe-hook/<session>/`, `logs/Client.txt`; crash dumps are moved to `runtime/crash-dumps/` by `run-client` ([setup.md](setup.md#logs), [crash reports](setup.md#crash-reports)). Do not attach captures or logs with passwords to a public issue |

If something else breaks, open an issue and say your OS, the client build, the server release and what the logs say after you have removed account names, tokens and passwords.
