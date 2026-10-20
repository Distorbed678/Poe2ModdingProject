# third_party (downloaded tools)

`fetch-tools.sh` downloads everything listed here into this folder. Git ignores everything in it except this README and the script.

| Tool | Version | Source | Purpose |
|---|---|---|---|
| `poe_data_tools/poe_data_tools` | cli-v2.2.1 | https://github.com/adamthedash/poe_data_tools | CLI that lists, extracts and dumps tables from `GameFiles/Bundles2`. Example: `third_party/poe_data_tools/poe_data_tools -p 2 --steam GameFiles list 'data/balance/*.datc64'` |
| `dat-schema/schema-poe2.min.json` | rolling `latest` | https://github.com/poe-tool-dev/dat-schema | Community .datc64 schema. `poe-data` loads it at runtime |
| `LibGGPK3/linux-x64/` | v2.7.5 | https://github.com/aianlinb/LibGGPK3 | GUI and CLI bundle browser, for cross-checking only (AGPL, never linked) |
| `cargo-xwin/bin/cargo-xwin` | latest | https://github.com/rust-cross/cargo-xwin | Cross-compiles the client DLL (`x86_64-pc-windows-msvc`) using clang/lld. Its MSVC CRT/SDK cache goes in `xwin-cache/` |
| `pyenv/` | capstone 5.0.9, numpy 2.5.4, pefile 2024.8.26, git-filter-repo 2.47.0 | PyPI | Python venv for the offline RE scripts in `scripts/re/` (`third_party/pyenv/bin/python scripts/re/<script>.py`). Venvs embed absolute paths, so recreate it (re-run the fetch script) after moving the repo |
| `reference/poe_data_tools` | git HEAD | adamthedash/poe_data_tools | Source of the library `poe-data` builds on. FORMATS.md documents the file formats |
| `reference/poeformats` | git HEAD | annalithic/poeformats | C# parsers for .tgr/.tdt/.arm/.tgt, used as reference for terrain generation (milestone 4) |
| `reference/KiwiNet` | git HEAD | Crypto137/KiwiNet | Emulator for PoE1 0.8.8: login/instance flow and object serialization (AGPL, read only) |
| `reference/exileSniffer` | git HEAD | ncatlin/exileSniffer | PoE1 3.x (2018) protocol dissector: packet IDs and crypto key layout (read only) |
| `reference/dat-schema` | git HEAD | poe-tool-dev/dat-schema | GraphQL sources for the schema (column notes) |

The rustup toolchain `nightly-2026-08-10` (pinned in `rust-toolchain.toml`) is installed under `~/.rustup` by the fetch script. It is not stored here.

Dynamic debugging (x64dbg) is not in this folder. Ask the user for it when needed.
