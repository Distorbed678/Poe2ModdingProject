# Contributing

Thanks for looking at the project. This is an unofficial, personal and educational private server for **Path of Exile 2 client 0.5.5e**, in Rust. Read the [disclaimer in the README](README.md#disclaimer) first: it is not affiliated with Grinding Gear Games, no game files are distributed, and nothing here is meant for official servers.

The work is spread over five repositories. This one holds the server and the dev tooling; the others are git submodules under `modules/`:

| Repo | Submodule | Checks |
|---|---|---|
| [Poe2ModdingProject](https://github.com/Distorbed678/Poe2ModdingProject) (this one) | `/` | `cargo build`, `cargo test`, `python3 scripts/check-links.py` |
| [Poe2Protocol](https://github.com/Distorbed678/Poe2Protocol) | `modules/protocol` | `cargo test --workspace` (also with `--features poe-net/tokio`) |
| [Poe2ClientHook](https://github.com/Distorbed678/Poe2ClientHook) | `modules/client-hook` | `cargo test`, and the DLL build for `x86_64-pc-windows-msvc` |
| [Poe2WebAdmin](https://github.com/Distorbed678/Poe2WebAdmin) | `modules/web-admin` | `npm run check`, `npm test`, `npm run build` |
| [Poe2Launcher](https://github.com/Distorbed678/Poe2Launcher) | `modules/launcher` | see its README and [docs/guides/launcher.md](docs/guides/launcher.md) |

## Ground rules

These are the project's hard rules (the long form is in [CLAUDE.md](CLAUDE.md)). A change that breaks one is not accepted.

- **No game data in git.** No game files, bundles, extracted tables, datamined dumps, official-server captures, `Client.txt` excerpts or screenshots of them. Generated data goes to `dumps/<version>/` and local runtime files to `runtime/`; both are git-ignored. Test fixtures must be your own synthetic or anonymised data.
- **Official servers are read-only.** The client DLL may observe and log an official session the user plays; it never sends, alters or injects anything there. Do not add anything that talks to official servers or changes a running official session.
- **The Steam install is never touched.** `GameFiles/` is a copy of the client install and changes only through `cargo xtask deploy-client`, `revert-client` and `deploy-core`, which back files up first. Do not hand-edit it.
- **No hardcoding.** Opcodes live in `modules/protocol/opcodes/<version>/*.toml`; ports, hosts, leagues and start areas in `config/server.toml`; classes, areas and items come from the game data (`poe-data`); subsystems sit behind traits.
- **No copied code from AGPL reference projects.** `third_party/README.md` lists research material (KiwiNet, exileSniffer, LibGGPK3 and others). Read them to understand a format, do not copy code from them into the Rust sources, and do not link them.
- **No secrets or personal data** in code, config, docs, logs or commit messages: passwords, tokens, keys, e-mail addresses, home directories, LAN addresses.
- Security problems go through [SECURITY.md](SECURITY.md), not a public issue.

## Setting up

Needs Linux with rustup (the nightly in `rust-toolchain.toml` is installed for you), `git`, `gh`, clang and lld. Steam with Proton Experimental and your own legal copy of the client (in `GameFiles/`) are only needed to run the client. Everything is described in [docs/guides/setup.md](docs/guides/setup.md); in short:

```sh
git clone --recursive https://github.com/Distorbed678/Poe2ModdingProject.git
cd Poe2ModdingProject
third_party/fetch-tools.sh            # the external tools, pinned; see third_party/README.md
cargo build && cargo test
python3 scripts/check-links.py
```

Tests that need the game files (`poe-data`) skip themselves when `GameFiles/` is missing, so a plain clone builds and tests. To run the server and a client: [docs/guides/setup.md](docs/guides/setup.md#run). The committed `config/server.toml` is safe by default (loopback, no password-less login). Developer switches live in a git-ignored `runtime/server.dev.toml` that `python3 scripts/dev-config.py` writes; start with `POE_CONFIG=runtime/server.dev.toml` ([setup.md](docs/guides/setup.md#developer-config)). Playing and deploying the client: [docs/guides/client.md](docs/guides/client.md). Hosting for others: [docs/guides/hosting.md](docs/guides/hosting.md).

## Branches and pull requests

1. Fork the repository (or branch, if you have write access) and create a topic branch from `main`: `git switch -c short-topic-name`.
2. Keep a change focused: one fix or feature, with its tests and docs. Unrelated clean-ups go in their own pull request.
3. Open the pull request against `main` and fill in the template. Say what changed and why, and how you checked it. A change that needs the game running says so, and what you saw.
4. CI must be green (below). Review comments are answered by new commits; they are squashed or kept as the maintainer prefers.

Do not reformat files you are not changing. Most of the existing code is not `rustfmt`-formatted, and a whole-repo `cargo fmt` is a separate, deliberate change. Format only the files you change substantially. The module repos (Poe2Protocol, Poe2ClientHook, Poe2Launcher) do check `cargo fmt --all --check` in their CI.

### Submodules

A change that spans repos is made bottom-up. Commit and push (or open the pull request) in the submodule repository first, then commit the new submodule pointer in the main repository (`git add modules/<name>` records the commit). A pull request in this repository that moves a pointer must point at a commit that is on the submodule's `main` or in its open pull request. Nested pointers (`client-hook` holds `protocol`, `launcher` holds `client-hook` and `protocol`) are bumped the same way, from the inside out. `cargo xtask versions` keeps the launcher's copy of `versions/<version>.toml` in step.

## Commit messages

One line that says what the change does and, when it helps, the work package or area it belongs to, written in the imperative or as a plain statement of the result. Examples from the history:

- `Client mod loader in the DLL: cargo xtask deploy-mod and deploy-client --sample-mod, docs use poe-hook-mods/ (WP C4)`
- `Server release CI (tag server-v*: Linux tar.gz and Windows zip with config, versions, opcodes, schema, web-admin, SHA-256) ...`

Add a body when the reason is not obvious: what was measured, which capture or test shows it, what is still open. Docs and code go in the same commit (next section). A commit never contains game data, captures, secrets or personal data.

## Tests and checks

What CI runs, and what to run before you push:

| Where | CI | You run |
|---|---|---|
| Main repo (`.github/workflows/ci.yml`) | `python3 scripts/check-links.py`; `cargo build --locked` and `cargo test --locked` on Linux with the pinned nightly; `cargo xtask openapi --check`; a gitleaks scan of the files | the same three commands; `cargo xtask tracker --check` if you touched `docs/status/tracker.toml` or the README status block; `cargo xtask build-dll` if you touched the client hook |
| Poe2Protocol | `cargo fmt --all --check`, clippy, `cargo test --workspace` with and without the `tokio` feature | the same |
| Poe2ClientHook | `cargo fmt --all --check`, clippy and `cargo test -p poe-hook-deploy`; a Windows job builds the DLLs and runs the `poe-hook-shared` tests | `cargo test` on the host crates; `cargo xtask build-dll` in the main repository for the DLLs |
| Poe2WebAdmin | `npm ci`, `npm run check`, `npm test`, `npm run build` | the same; after an API change `cargo xtask openapi` (main repo) then `npm run gen:api` |
| Poe2Launcher | its own `ci.yml`: `npm run check/test/build`, `cargo fmt --check`, clippy, `cargo test` | see [docs/guides/launcher.md](docs/guides/launcher.md#building-and-running) |

Rules for tests: a behaviour change comes with a test that fails without it. Wire formats are tested with byte arrays you built yourself or anonymised ones (never a raw capture). Movement and entity behaviour use the fixtures in `crates/poe-game/tests/fixtures/`. A change to the web API updates `docs/architecture/openapi.json` (`cargo xtask openapi`); CI fails when it is stale.

Things that only a running game can show (a client-side change, a message the client reacts to) are checked with the unattended test tools described in [docs/guides/setup.md](docs/guides/setup.md#unattended-client-tests). Say in the pull request what you ran and saw; if you cannot run the game, say that too and mark the change untested in-game.

## Documentation goes with the code

Docs are part of the change, not a follow-up. When you change behaviour, update the doc that describes it in the same commit: a config key (the key's comment in `config/server.toml` and `config/server.example.toml`, and [docs/guides/setup.md](docs/guides/setup.md#config)), a command ([docs/guides/server-admin.md](docs/guides/server-admin.md)), a message ([docs/protocol/](docs/protocol/framing.md)), an architecture fact ([docs/architecture/](docs/architecture/overview.md)). The map is [docs/index.md](docs/index.md); a new doc is linked from it. After any doc change run:

```sh
python3 scripts/check-links.py
```

It checks relative links and heading anchors. Style: say what is, concisely, in plain words; state what was measured and how; mark what is unverified. Status lives in one place per topic ([docs/status/](docs/status/history.md) and the tracker); do not keep a second copy.

## Documenting reverse-engineering findings

Findings from analysing the client go in [docs/re/](docs/re/README.md), not in code comments or commit messages alone. [docs/re/README.md](docs/re/README.md#where-findings-go) says which file takes which topic (addresses, crypto, standalone login, captures, terrain, data mining). The rules:

- **Addresses** (functions, structs, offsets) go in [docs/re/addresses-0.5.5e.md](docs/re/addresses-0.5.5e.md), for the 0.5.5e build only, with the Ghidra name and what it does. If the DLL uses the address, it is a signature in `modules/client-hook/signatures/<version>.toml`, not a literal ([docs/re/update-compatibility.md](docs/re/update-compatibility.md) explains the inventory).
- **Wire formats** go in [docs/protocol/](docs/protocol/framing.md), with the opcode in `modules/protocol/opcodes/<version>/*.toml`, never as a number in code.
- Write what you found and how you know: the function, the constant, the capture evidence (by session description, not by pasting packets), and what is still a guess. Do not paste decompiler output or any part of the game's code, assets or tables; describe the behaviour in your own words. A few names and offsets are research notes, long listings are copying.
- Official-server captures belong to the user who played them and stay in the git-ignored `dumps/<version>/captures/`; docs quote at most the field names and numeric values needed to define a layout. No account names, character names, other players' names or chat lines.
- Dynamic debugging (x64dbg) and official captures need the maintainer: ask in the pull request or an issue, do not attach them.

## Releases

Maintainers cut releases from tags; contributors do not need to. How: the server in [docs/guides/releasing-server.md](docs/guides/releasing-server.md) (tag `server-v<version>`), the launcher in [docs/guides/launcher.md](docs/guides/launcher.md#releasing) (tag `v<version>`, signed updates). The server package is described for hosts in [docs/guides/hosting.md](docs/guides/hosting.md#getting-a-server-release).

## Licence

By contributing you agree that your contribution is licensed under the repository's terms: MIT OR Apache-2.0 ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
