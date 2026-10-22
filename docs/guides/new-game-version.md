# When GGG ships a new game build

Step by step, from the new files to the in-game check. Background and the list of what breaks: [../re/update-compatibility.md](../re/update-compatibility.md). Everything that names a build lives in one file per build, `versions/<version>.toml` at the repository root; this guide ends with adding one.

## What defines a supported build

| Piece | Where | Read by |
|---|---|---|
| **The version file** `versions/<version>.toml`: depots and manifests, the exe's sha256, size and PE fingerprint (timestamp, image size), the build revision, key-file sizes and floors, the opcode folder, the signature table path, the features that must resolve, the known `client_hash` values, strings the exe must contain, data anchors and row counts, the dat-schema pin | repository root | `cargo xtask` (deploy, `check-build`), the servers (`[game] version`), the launcher |
| The launcher's copy | `modules/launcher/versions/<version>.toml`, byte-identical, written by `cargo xtask versions` (`--check` compares; `check-build` and a test compare too) | the launcher (it is a separate repository and must build alone; its `build.rs` embeds every file there) |
| Opcode tables | `modules/protocol/opcodes/<version>/` (`login.toml`, `instance.toml`, `hnc.toml`, `inventories.tsv`, `deserializers.tsv`) and `poe_proto::opcodes::EMBEDDED` | servers (loaded from the folder), the DLL (compiled in) |
| Signature table | `modules/client-hook/signatures/<version>.toml`, listed in `poe-hook-sig/src/embedded.rs` `TABLES` | the DLL, `check-build` |
| Data | `dumps/<version>/` (git-ignored), `docs/re/addresses-<version>.md` (history) | tools, scripts (`scripts/re/version.py` picks the newest version file, or `$POE_VERSION`) |

`[game] version` in `config/server.toml` selects the version file. `{version}` in the config's path settings (`known_seeds`, `sweep_results`, `layout_dir`) is replaced by it, and `protocol_dir` defaults to the file's `[protocol] opcodes`, so a new build is a new file plus one changed config value. A version the server has no file for is a start-up error that names the known ones.

Kinds of patch (rating in the compatibility doc): **data** (`Bundles2` changes, the exe is byte-identical: only steps 1, 2, 4 and the data part of 5), **hotfix** (a rebuilt exe, the common case: functions move; most signatures survive), **major** (new league or 0.6: messages and generator passes change).

## 1. Update `GameFiles/`

- `GameFiles/` is a copy of the client install. Never touch the Steam install or its Proton prefix.
- Undo the deploy first, so the `.orig` backups describe the old files: `cargo xtask revert-client`.
- Replace the copy with the new files: let Steam update its own install and copy it again (read-only on the Steam side), or install the build with the launcher into another folder and point `[game] game_dir` at it.
- Do not deploy yet (the DLL's signature table does not know the build).
- A copy of the exe elsewhere also works for step 2: `check-build` takes any path.

## 2. `cargo xtask check-build GameFiles/PathOfExileSteam.exe`

```
cargo xtask check-build <exe> [--bundles <game dir>] [--version <v>] [--report <file>]
```

It reads the exe, scans it with the DLL's own matcher (`poe-hook-sig`), and writes `dumps/<version>/check-build.md`; the console shows a summary per block and every warning and failure. `--bundles` is the folder with `Bundles2/` (default `[game] game_dir`). The version file it compares with is the one whose exe sha256 (or PE fingerprint) matches, else the config's `[game] version`, else the newest (`--version` to choose).

| Block | What it reports |
|---|---|
| Identity | exe size, sha256, PE timestamp and image size against the version file; the revision string found in the exe (the one `Client.txt` prints); the signature table's `[build]` against the version file; the recorded `client_hash`; the launcher copy; the build of the terrain vectors |
| Signatures | every `[[sig]]`: **found** (address, and how far it moved from the table's `ghidra` address), **missing**, **ambiguous** (matched n times) or failed (bytes at the address, not a function start, a site inside an instruction) |
| Struct offsets | every `[[field]]`: found (value, or "changed: old -> new"; the DLL reads the new one), missing, ambiguous, or the anchor failed |
| Features | per DLL feature: on or OFF, required or research |
| Opcodes | the opcode tables load; the HNC `key_xor` found by value; **deserializer drift** (below) |
| Strings | the option strings, file names and log texts the tools depend on |
| Game data | each table the server reads (`poe_data::used::USED`): it opens, every used column is in the schema and decodes, the schema's row width equals the file's (a narrower file is an error: trailing columns would read garbage); the dat-schema file against the pin; row counts; the row anchors (`Stats` `actor_scale_+%` at 1913, `ActionTypes` `Move` with key 10505, ...); the config's sprint buff rows |
| Files | key-file sizes of the install |

Exit code: **0** everything found and the build matches; **1** warnings (a research feature's entry is missing, a row count changed, the schema pin differs, no baseline); **2** a required feature's entry is missing, ambiguous or different, a row anchor moved, a table does not decode, the identity differs. "Required" is the version file's `[client_hook] required_features`; every other feature is research. On a new build step 2 fails on the identity (it is not the recorded build) and on whatever moved: that is the work list for steps 3 and 4. A hotfix typically shows the identity differing and every signature still found or only a few missing.

**Deserializer drift.** `opcodes/<version>/deserializers.tsv` holds, for each server-to-client opcode of the client's packet factories, a fingerprint of the class's deserializer: the function's bytes with call and jump targets, RIP-relative and stack displacements, frame sizes and image addresses wildcarded (`xtask/src/fingerprint.rs`). The new exe's 100,000 functions are fingerprinted and each baseline entry is looked up: *unchanged* (the layout we send is still the one the client reads), or *layout changed* (**FAIL** when the opcode is one we name in `instance.toml`/`login.toml`, a warning otherwise: re-read that message in Ghidra and fix `poe-proto`). What is not derived from the exe: **renumbered opcodes** (find the factory switch of the new build with `scripts/re/find_switches.py <function> <length>`, dump it with `scripts/re/dump_factory.py`, and compare the fingerprints by hand), the client-to-server messages (only server-to-client deserializers are fingerprinted), and the automatic discovery of the factories (their addresses are Ghidra output today).

## 3. Regenerate the signatures

Only when step 2 shows missing or ambiguous entries, or the build changed at all (the table is chosen by the exe's PE timestamp and image size, so a rebuilt exe needs its own `[build]`).

1. Re-analyse the new exe in Ghidra (`docs/re/README.md`) and find the moved functions: with a table that mostly resolves, every entry still found gives its new address (the check-build report lists it), and the missing ones are near their old neighbours; string anchors survive best (`Unable to deserialise packet`, `General Rhoa Failure #3`, `Tile hash`, ...).
2. Copy `scripts/re/sig_spec_<old>.py` to `sig_spec_<new>.py`, set `BUILD` (version, revision, PE timestamp, image size, sha256) and the changed addresses.
3. `third_party/pyenv/bin/python scripts/re/sig_gen.py scripts/re/sig_spec_<new>.py GameFiles/PathOfExileSteam.exe > modules/client-hook/signatures/<new>.toml`
4. `cargo run --release -p poe-hook-sig --example resolve -- GameFiles/PathOfExileSteam.exe` in `modules/client-hook` (exit 1 on any difference from the table's own addresses; for a new build the table's addresses are the new ones).
5. Add the file to `TABLES` in `modules/client-hook/poe-hook-sig/src/embedded.rs` and rebuild (`cargo xtask build-dll`).

If every signature already resolves, the shortcut is to copy the old table, change `[build]` (version, `build_revision`, `pe_timestamp`, `image_size`, `exe_sha256`) and refresh `ghidra` and `value` with `sig_gen.py` or the `resolve` output.

## 4. Add `versions/<new>.toml` and the opcode folder

1. Copy `versions/<old>.toml` and change: `version`, `build_revision` (the string `check-build` printed, the `Build Revision:` of `Client.txt`), `status = "experimental"` until step 6 passes, `[exe]` (`sha256`, `size`, `pe_timestamp`, `image_size`; from the report's fingerprint table), the depots and manifests (what the Steam download dialog or DepotDownloader shows for the build; old manifests may disappear from Steam), `[verify]` sizes (`ls -l` the key files), `[protocol] opcodes`, `[client_hook] signatures`, `[client] hashes = ["<new exe sha256>"]` with `hashes_confirmed = false`, `[data]` (`schema_sha256` of the schema you checked with, `row_counts`, anchors; after a data patch the row indices of the anchors are what the server's keys depend on: change the config and the anchors together).
2. `cp -r modules/protocol/opcodes/<old> modules/protocol/opcodes/<new>`, fix the headers and any renumbered or new opcodes, add it to `EMBEDDED` in `modules/protocol/poe-proto/src/opcodes.rs` (a test fails when a folder is missing there). Regenerate the drift baseline: dump the factories (`dump_factory.py`, output `dumps/<new>/ghidra/{login,instance,instance2}_factory*_s2c.json`; `find_switches.py` prints the arguments; for a switch with an index table `--bytes <rva>`) and run `cargo xtask check-build <exe> --version <new> --write-fingerprints`.
3. `cargo xtask versions` (copies the file to the launcher), then `cargo xtask check-build GameFiles/PathOfExileSteam.exe` again: it must exit 0.
4. Set `version = "<new>"` in `config/server.toml` (and the example) and move `dumps/<old>` aside if the generated data of the old build must not mix with the new one (`known_seeds`, `layout_dir`, `sweep_results` follow `{version}`).
5. Not configuration, and left alone on purpose: `0.5.5e` in the history of docs, in comments with Ghidra addresses, `docs/re/addresses-<version>.md`, the captures' build in `servers/instance-server/src/replay_tests.rs` (`CAPTURE_VERSION`: the tests replay the official captures of that build), the launcher's test fixtures and `poe_proto::opcodes::v0_5_5e` (kept for the DLL; it selects by name today).

### The launcher and the version dropdown

The launcher's `build.rs` embeds **every** `modules/launcher/versions/<version>.toml` that has a `version` and a `build_revision` (oldest revision first; `depotdownloader.toml` is skipped), so a second supported build needs no launcher code: `cargo xtask versions` copies the new file, a rebuild embeds it, and the version dropdown in the title bar lists it (newest first, below *Official (read-only)*). The newest revision is the default for a user who never picked one. Everything version-dependent in the launcher reads the selected file: install folder (`<data>/game/<version>`, or the per-version folder in Settings), depots and manifests, verification, the hook deploy, Play, the mods' compatibility check, the server-package filter of Host a server (`poe-server-v<ver>-game-<version>-...`), and the "needs client X" marks on the server list. The old single `install_dir` setting is moved to the oldest supported version when the settings file is read.

To check it: add the file, `cargo test` in `modules/launcher/src-tauri` (the `every_embedded_version_is_selectable` test walks all versions), `npm run dev` and open `http://localhost:1420/?mock&versions` for a browser preview with a second version. "Official (read-only)" is not a version file: it reuses the newest file's depots **without** manifests (the current official build), see [launcher.md](launcher.md#game-versions-and-the-official-mode).

## 5. Tests

```
cargo build && cargo test                           # main repository
cargo test                                          # in each submodule you changed (modules/protocol, modules/client-hook)
cargo test; npm run check                           # modules/launcher/src-tauri, and the frontend if it changed
cargo xtask versions --check
cargo xtask check-build GameFiles/PathOfExileSteam.exe      # exit 0
python3 scripts/check-links.py; cargo xtask tracker --check
```

The tests of `xtask` run `check-build` on `GameFiles/PathOfExileSteam.exe` and on a copy with one byte of a required function changed (skipped without `GameFiles/`). Replay tests only protect the build whose captures they replay: for new layouts the user takes new official captures (read-only) and the replay tests get a new capture version.

## 6. The in-game check

The orchestrator's check, on the GameFiles copy of the new build (the user's decision to run; never the Steam install):

1. `cargo xtask deploy-client`, start the server, start the client: the standalone login screen (login patch), no patcher window (patcher skip), the character list and an instance (crypto bypass, key hook), movement (position sync), the overlay (F-keys), a terrain upload. The DLL's log (`GameFiles/poe-hook.log`) prints its signature summary: every feature `ok`, no "build NOT verified".
2. **The `client_hash`.** While `hashes_confirmed = false` the login server logs `client_hash cannot be checked yet ... client_hash=<hex>` for every login whose hash is not in `[client] hashes` (a hash that is listed is accepted silently: no line means it matched). The hash is a SHA-256 of the whole exe (`LoginClientHash_Get32`, `0x1420e7580` in 0.5.5e), so it should equal `[exe] sha256`; confirm it. If it does, set `hashes_confirmed = true` in the version file (and `cargo xtask versions`). If it differs, record the logged value in `[client] hashes` and then set it. From then on the server refuses any other client. 0.5.5e: confirmed in game on 2026-10-21 (a standalone login with the exe's sha256 listed was accepted as known, and with another hash listed the log showed `client_hash=7157984534da...3f89f`, the exe's sha256).
3. The refusal: with `hashes_confirmed = true`, log in with a client of another build (or temporarily change one hash in the file): the client shows "There has been a patch that you need to update to. Please restart Path of Exile." (our notice: the version the server runs). `[security] allow_unknown_client = true` lets such a client in (to try an unlisted build; logged). Both seen in game on 2026-10-21 (0.5.5e, standalone login with a saved token): the popup with our notice in the login log, and the login going through with the flag on.
4. Official read-only session (`deploy-client --official`) when the user plays one: the DLL only observes; compare with the previous build's captures.
5. Only then `status = "supported"`.

## What the server does with the client's version

The client sends no version string. The 32-byte `client_hash` of `0x0003` (standalone) and `0x0005` (Steam) is a hash of its executable, so it identifies the build before any account is looked at: the login server judges it against `[client] hashes` of the version `[game] version` selects. A known hash is accepted; an unknown one is refused with the `BackendErrors` code 8 (`InvalidProtocolVersion`, the client's own "you need to update" popup) plus a notice naming the server's version. While `hashes_confirmed = false`, or when the file lists no hash, nothing is refused and the hash is logged. `[security] allow_unknown_client = true` accepts any hash (logged as a warning). The check is skipped when the config was made without a version file (`Config::parse`, the tests). It protects the login channel; the instance channel is reached only with a ticket from a login that passed it.

## Not automated yet

- The factory dumps behind the drift baseline (`dump_factory.py` arguments come from Ghidra), renumber detection, and client-to-server layouts.
- `.tgr`/`.dgr`/`.tdt`/`.ot` format versions (the parsers read the number; a version they do not know is not rejected everywhere).
- A pinned dat-schema download (the pin is checked, `third_party/fetch-tools.sh` warns when the rolling release differs; it cannot fetch a given release yet).
- Cross-checking the DLL's side-channel hello with the server (the hello does not carry the resolved build).
- `symbols.tsv` per build.
