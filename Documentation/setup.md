# Setup

## Requirements
- Linux with rustup. The toolchain is pinned by `rust-toolchain.toml` to `nightly-2026-08-10`, because `poe_data_tools` needs nightly. Targets: `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`.
- clang and lld (`clang-cl`, `lld-link`), used by cargo-xwin to link the Windows DLL.
- `gh` and `git`, for the tool downloads.
- Steam with Proton Experimental, to run the client.
- `GameFiles/`: a copy of the 0.5.5e client install (not in git). The Steam install itself is never used or modified.

## Tools
Run `Tools/fetch-tools.sh` to download every external tool into `Tools/` (listed in [Tools/README.md](../Tools/README.md)): poe_data_tools CLI, dat-schema, LibGGPK3, cargo-xwin, reference repos and the pinned toolchain.

## Build and test
```sh
cargo build                 # server workspace
cargo test                  # unit tests; poe-data tests read GameFiles if present
cargo xtask build-dll       # client/poe-hook -> poe_hook.dll (x86_64-pc-windows-msvc)
```

## Run
```sh
cargo run -p poe-server     # login (20481) + instance (6112) servers, config/server.toml
cargo xtask deploy-client   # builds the DLL and installs it into GameFiles/
cargo xtask run-client      # launches the client through Proton (Steam must be running)
cargo xtask revert-client   # restores GameFiles/ to the original state
```

`deploy-client` does the following:
- Backs up `GameFiles/gateway_list.txt` to `gateway_list.txt.orig` (only once).
- Writes a single UTF-16LE gateway entry, built from `[gateway]`/`[login]` in `config/server.toml`.
- Copies the DLL to `GameFiles/version.dll`.
- Copies `client/poe-hook/poe-hook.toml`, unless a copy already exists (so your local edits survive).
- Sets `position_sync_port` in that file's `[features]` to `[instance] public_port`, so position reports go to the instance server.
- Writes `steam_appid.txt`.

`run-client` uses its own Proton prefix (`Tools/proton-prefix`) with `WINEDLLOVERRIDES=version=n,b`.

## Playing from Steam (non-Steam game shortcut)
One-time setup, after `cargo xtask deploy-client`:
1. In Steam: **Games → Add a Non-Steam Game to My Library… → Browse…** and select `<repo>/GameFiles/PathOfExileSteam.exe`. Click **Add Selected Programs**.
2. In the library, right-click the new entry → **Properties…**:
   - **Shortcut**: optionally rename it, e.g. "PoE2 Private Server". Set **Start in** to `"<repo>/GameFiles/"`.
   - **Launch options**:
     ```
     SteamAppId=2694490 SteamGameId=2694490 WINEDLLOVERRIDES="version=n,b" %command%
     ```
     `version=n,b` loads our `version.dll` (the hook) instead of Wine's. The app id makes the Steam API start as Path of Exile 2, which you own.
   - **Compatibility**: tick **Force the use of a specific Steam Play compatibility tool** and pick **Proton Experimental**.

   Steam creates a separate Proton prefix for the shortcut (`steamapps/compatdata/<shortcut id>`); the official install's `compatdata/2694490` is never used.

To play:
1. Start the server from the repo: `cargo run --release -p poe-server` (leave it running; Ctrl+C stops it).
2. Launch the shortcut from Steam. The login screen shows "Local Server"; log in and play.

Notes:
- The DLL config `GameFiles/poe-hook.toml` presses Enter 20 s after start (`auto_keys`, used by unattended tests). It just logs in; set `auto_keys = []` to turn it off.
- Keep `dump_object_types = true`: the spawn position and the server's movement collision currently rely on the layout and walkability grid the DLL exports.
- Movement: WASD works in the default input mode. For click-to-move, pick the mouse input method in the game's options (in the config file: `user_input_mode=mouse_and_keyboard`). Space is the dodge roll in both modes.
- After rebuilding the DLL, run `cargo xtask deploy-client` again; the shortcut needs no changes.

## Logs
- Client: `GameFiles/logs/Client.txt`.
- DLL: `GameFiles/poe-hook.log`.
- Server: stdout, filtered by `RUST_LOG` or `[logging] filter`.
