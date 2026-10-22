# Releasing the client hook

How a release of the client DLLs (`poe_hook.dll`, `poe_hook_core.dll`, Poe2ClientHook) is made, what is in it, and how the launcher uses it. The procedure itself lives next to the code, in the submodule: [modules/client-hook/docs/releasing.md](../../modules/client-hook/docs/releasing.md); this page is the overview from the project's side. The server's equivalent: [releasing-server.md](releasing-server.md).

## Why

A player needs the DLLs next to the game. Before this, that meant a Rust toolchain and `cargo xtask build-dll` (or cargo-xwin) and a *Hook DLL directory* setting. Now the launcher downloads a verified release, so the whole setup is: install the game, get the client hook, then host a server or join one ([launcher.md](launcher.md#client-hook)).

## Cutting a release

1. Bump `[workspace.package] version` in `modules/client-hook/Cargo.toml` (and `Cargo.lock`) if needed; commit and push inside the submodule (CI green). The workflow refuses a tag that disagrees with the crate version.
2. `git tag hook-v<version>` and `git push origin hook-v<version>` in `modules/client-hook` (a suffix like `-rc1` makes a pre-release; the launcher prefers stable ones).
3. [`release.yml`](../../modules/client-hook/.github/workflows/release.yml) in Poe2ClientHook builds both DLLs for `x86_64-pc-windows-msvc` on `windows-latest` (the MSVC toolchain of the runner, plain `submodules: recursive` checkout, no secrets), packages them with `.github/scripts/package-hook.py` on Ubuntu and publishes the GitHub release with the zip, its `.sha256` and `SHA256SUMS`; the notes are the commit subjects since the previous `hook-v*` tag. A hand-started run builds the package as a workflow artifact only.
4. Bump the `modules/client-hook` pointer in this repository when the main project should build against the new commit (not needed for the launcher: it downloads releases).

## The package

`poe-hook-v<version>-game-<game versions>-windows-x86_64.zip` (game versions = the `signatures/<v>.toml` tables, joined with `_`), one top folder with `poe_hook.dll`, `poe_hook_core.dll`, `signatures/`, `LICENSE`, `NOTICE`, `README.txt` and `release.json`: hook version, supported game versions, mod ABI (`HOOK_API_VERSION`), platform, commit and the size and SHA-256 of every other file. The launcher's checks (archive checksum, safe unpacking, per-file hashes, no unlisted files, game version, again before each Play) are listed in [launcher.md](launcher.md#client-hook); the schema is in the submodule's releasing.md.

## A new game version

A client update needs a new signature table in the hook ([../re/update-compatibility.md](../re/update-compatibility.md)) and a version file ([new-game-version.md](new-game-version.md)). Release the hook with both tables: its name and `release.json` then list both game versions, and the launcher offers it for either. The launcher only needs a release of its own if it should know the new game version itself (the dropdown comes from its embedded `versions/*.toml`).
