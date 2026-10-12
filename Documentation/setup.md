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
- Writes `steam_appid.txt`.

`run-client` uses its own Proton prefix (`Tools/proton-prefix`) with `WINEDLLOVERRIDES=version=n,b`. If launching through the script fails because of Steam integration, add `GameFiles/PathOfExileSteam.exe` to Steam as a non-Steam game with these launch options:
`WINEDLLOVERRIDES="version=n,b" %command%`

## Logs
- Client: `GameFiles/logs/Client.txt`.
- DLL: `GameFiles/poe-hook.log`.
- Server: stdout, filtered by `RUST_LOG` or `[logging] filter`.
