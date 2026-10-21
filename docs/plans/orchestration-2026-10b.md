# Orchestration plan 2: update resilience, login/accounts, public release, launcher releases, hosting

**Executor:** a new Opus 5.5 orchestrator at medium effort. It delegates each work package (WP) to a Sonnet 5.5 subagent: `Agent(model: "sonnet", effort: "high", subagent_type: "general-purpose")`.
**Location:** `docs/plans/orchestration-2026-10b.md`.

## Context
The first plan (`docs/plans/orchestration-2026-10.md`) is done. The project now has:
- server-authoritative movement;
- accounts and the standalone login;
- remote hosting;
- two-client multiplayer;
- the web API and Poe2WebAdmin;
- Poe2Launcher.

The next goals:
- survive game updates;
- finish the Login and Accounts tracker sections, including removing the "suspicious login activity" popup on the standalone login;
- make all five repos public safely, with contributor, hosting and client guides;
- ship the launcher as downloadable releases with signed auto-update;
- let users set up a server from the launcher;
- close the open items left over from plan 1.

## Decisions made with the user
| Topic | Decision |
|---|---|
| Publishing | All 5 repos (Poe2ModdingProject, Poe2Protocol, Poe2ClientHook, Poe2WebAdmin, Poe2Launcher) go public **with history kept**, after secrets are scrubbed from history (`git filter-repo`) |
| Launcher updates | Signed auto-update with the Tauri updater plugin against GitHub releases. The signing key is a GitHub secret |
| Release formats | Linux AppImage. Windows `.exe` installer (Tauri NSIS) |
| Server via launcher | Release CI builds `poe-server` for Linux and Windows. The launcher downloads it, writes its config, uses the launcher's installed game data, and starts and stops it. An "Open web admin" button appears for servers where the account is admin |
| Accepted suggestions | Signature-based addresses plus `cargo xtask check-build`; a security review before publishing; a legal disclaimer and a "no game data" check |

## Order and rationale
1. **Phase A: update resilience** (tasks 1 and 2). It changes the most code, so it comes first. The new `check-build` tool helps all later RE work.
2. **Phase B: Login and Accounts** (tasks 7 and 8, plus the RE-type leftovers). It is RE plus server work. WP B1 can run in parallel with WP A1, because one is analysis and the other is Ghidra.
3. **Phase C: leftovers that need code** (movement gaps, spawn, remote tint, mod loader). This finishes the code before the audit.
4. **Phase D: public release** (tasks 3 and 4). The audit runs on the final code, and the history scrub happens once, just before the repos flip to public.
5. **Phase E: launcher releases and auto-update** (task 5). Releases and update checks need public repos, otherwise every user would need a token.
6. **Phase F: server setup in the launcher** (task 6). It reuses the release pipeline from Phase E for the server binaries.

The terrain port has its own plan (`docs/plans/terrain-all-areas.md`) and is out of scope here. It continues after this plan, or in parallel only if the user asks.

## Orchestrator rules
Same as plan 1, which has the full text. Specifically:
- **Start-up:**
  - Read CLAUDE.md, MEMORY.md and this plan.
  - Check `gh auth status`.
  - Keep progress in a work-state memory and in `docs/status/tracker.toml`.
- **Each subagent prompt contains:**
  - the goal;
  - the files and docs to read first;
  - the hard rules;
  - the acceptance criteria;
  - "update the docs in the same change and report the files changed".
- **The orchestrator verifies:**
  - Read the diff.
  - Run `cargo build && cargo test`.
  - Run `cargo xtask build-dll` when the client changes.
  - Run `npm run check` in the touched web repos.
  - Run `python3 scripts/check-links.py`.
  - Run `cargo xtask tracker --check`.
  - Commit and push the submodules first, then bump the pointers.
- **Parallel only when write sets don't overlap.**
  - Tell agents never to run `cargo fmt --all`.
  - Commit only the files of a finished WP, and leave other agents' in-progress files out.
- **Usage pauses** (user preference):
  - Stop agents at a safe point.
  - Each agent writes `docs/plans/handoff-<wp>.md`.
  - On resume, start a *fresh* agent from the handoff instead of resuming the old one.
- **Hard rules, repeated in every prompt:**
  - Never touch the Steam install or `compatdata/2694490`.
  - GameFiles changes only through `cargo xtask deploy-*`.
  - Official servers are read-only. Claude never logs in; captures are the user's.
  - No hardcoded opcodes, ports or game data.
  - Never use SendInput.
  - Use only in-game F8 screenshots.
  - Run `pkill` on the server in its own Bash call.
- **Gates that need the user:**
  - official captures (G-LOGIN, G-PARTY);
  - Steam running for in-game tests;
  - flipping repos public (final confirmation);
  - creating the signing key and secrets;
  - the launcher end-to-end test with a Steam depot login;
  - turning on Secret Service.

## Phase A: game update compatibility (tasks 1 and 2)

**WP A1: assessment (analysis only, no code)**
- Inventory everything that breaks when GGG ships a new build:
  - fixed Ghidra addresses: the DLL hooks, `login_patch` (`ClientPlatform_Detect` 0x142196ec0, `StandalonePatcher_Run`), the side-channel key hook 0x14212aa90, the trace and dump hooks, `symbols.tsv`, and `docs/re/addresses-0.5.5e.md`;
  - struct offsets (Positioned +0x444/+0x490, Actor +0x400, Player slots +0x214);
  - opcodes (`modules/protocol/opcodes/0.5.5e/*.toml`);
  - message layouts (poe-proto);
  - the dat-schema and table columns (poe-data);
  - the bundle paths and the version string in config, xtask, launcher `versions/*.toml`, and DepotDownloader manifests;
  - verify values (exe sha256);
  - the `InitialState` and blob layouts;
  - the terrain generator constants;
  - the `Client.txt` parsing in scripts.
- Rate each item: how likely it is to break per patch, how it fails (crash, silent, refused), and how hard it is to detect.
- Output: `docs/re/update-compatibility.md`, a table with a proposed fix per item, ranked by risk.

**WP A2: signatures and version-scoped data** (client-hook and protocol)
- Replace the fixed addresses with byte-signature lookups, with wildcards, resolved once at load. Keep them in a versioned table, e.g. `modules/client-hook/signatures/<version>.toml`, which also holds the expected original bytes for patches.
- On a mismatch, disable that feature and log it. Never patch blindly. Keep the existing safety checks (verify bytes, never in official mode).
- Resolve struct offsets from the same table, with signatures where they can be derived from instructions.

**WP A3: `cargo xtask check-build <exe> [--bundles <dir>]`**
- Scans a new exe for every signature, reports found, ambiguous or missing for each, and diffs the opcode table where it can be derived. It reuses the factory-dump logic of `scripts/re/dump_factory.py` and `find_switches.py`.
- Also checks the dat-schema and table columns against the Bundles2 data via poe-data, and the exe sha against the launcher's `versions/*.toml`.
- Writes a report to `dumps/<version>/check-build.md`.

**WP A4: version plumbing**
- One place defines the supported versions, `versions/<ver>.toml`, shared by xtask, the launcher and the server. It holds the depots, manifests, exe sha, protocol dir and signature table.
- The server refuses a client with an unknown version and gives a clear message.
- Add a doc `docs/guides/new-game-version.md` with the step-by-step update procedure.

**Acceptance:**
- `check-build` on the current GameFiles exe reports everything found.
- `check-build` on a byte-mutated copy reports the right failures.
- The DLL still works in game: the standalone login, the patcher skip, the side channel, and the overlay.

## Phase B: Login and Accounts (tasks 7 and 8)

**WP B1: login unknowns RE** (Ghidra plus captures; can run with A1)
- Explain the remaining CharacterList entry fields and the character-creation and refusal result codes:
  - find the client's message table, i.e. the strings shown per code;
  - find the decoder for each field.
- Cover any login field still marked unknown in `docs/protocol/login.md`, such as `0x0004` fields and the account flag bits.
- **Suspicious-login popup:** find which `0x0004` or `AuthenticationResult` field or flag, or which follow-up message, triggers the "suspicious login activity" popup on the standalone path. Likely candidates are a new-device or location flag, or a notice tied to the machine hash or `token_login`. Search for the popup string and its xrefs.
- **Gate G-LOGIN** (only if static RE stalls): ask the user for a read-only official capture of a character create with an invalid or taken name, plus a delete.

**WP B2: implement**
- The server sends correct values for every field.
- Creation and refusal codes map to real errors: name taken, invalid name, slot limit, and so on.
- Fix the popup flag.
- Finish the Accounts tracker section:
  - rate limiting of password attempts (it is a tracker item, so it is part of "finish the section": login server and web API, per account and per IP, configurable);
  - verify duplicate login with two clients (already done in WP3.7, so update the note);
  - refresh the stale notes.
- Tests: encoded-packet tests, plus an in-game check that the standalone login shows no popup and that creation errors display.

**Acceptance:** every Login and Accounts tracker item is `done`.

## Phase C: leftovers from plan 1 that need code
- **WP C1: movement gaps #11 and #12** (`docs/protocol/movement.md`):
  - #11: the client's slowdown at the sprint end, which needs RE of the client's WASD path at sprint end;
  - #12: the roll heading during the first frames and in click-to-move mode.
  - Verify in game with the drift checker. Target: under 8 cells, and about 0 after the stop.
- **WP C2: multiplayer polish:**
  - spread the spawn cells, so a second player doesn't stand on the first;
  - fix the remote player's dark or orange tint by comparing our remote blob with the official ones in `dumps/0.5.5e/captures/official/2026-10-20_21-51-49_336`: the missing stat sets, buffs, flag words and equipment visuals (see `docs/protocol/entities.md`);
  - check the remote roll animation;
  - verify with `scripts/test-mp.sh`.
- **WP C3: party and trade replies:**
  - Implement invite, cancel, request and cancel from the decoded messages.
  - **Gate G-PARTY:** accept and decline are unknown. Ask the user for a read-only official capture where a friend accepts and declines a party invite and a trade. Implement them after that.
- **WP C4: mod loader in the DLL:**
  - Implement the `mod.toml` loader (spec: `modules/client-hook/docs/mod-manifest.md`).
  - Settle the directory name: the launcher uses `poe-hook-mods/<id>/` plus `state.toml`, while `docs/guides/modding.md` says `mods/`. Pick the launcher's layout and update the docs.
  - Load enabled mods in order, with an ABI version check.
  - Ship a sample mod.

## Phase D: public release (tasks 3 and 4)

**WP D1: audit** (read-only, all 5 repos and their **full history**)
- **Secrets:** use gitleaks or trufflehog in `third_party/`, run over all history: tokens, keys, `.env` files, hardcoded passwords (for example the `tester1`/`hunter2pass` and `mp1`/`mp2` test credentials, if any are committed), and PATs in CI files.
- **Personal data:** emails, local paths (the dev machine's home directory), the dev machine's LAN IP, account names from captures, other players' names, chat text, and `Client.txt` excerpts.
- **Game data:**
  - no GGG assets, bundles, extracted tables or dumps committed;
  - check the test fixtures, which hold movement-only capture data, and the anonymised blobs;
  - decide on the RE address docs (they stay: they are research, but are covered by the disclaimer).
- **Security review:**
  - web API defaults (bind, CORS, token lifetimes, admin endpoints);
  - side channel;
  - the launcher's token handling, the update path, and the zip-slip and path guards;
  - the DLL's official-mode guarantees.
- **Output:** `docs/plans/public-release-audit.md` with findings, severity, and a fix per finding, including the history-scrub list for filter-repo.

**WP D2: fix and scrub**
- Fix the findings in the code.
- Rewrite the history of each repo with `git filter-repo`, using `--replace-text` and `--path` lists from D1:
  - Work on fresh mirror clones.
  - Keep a private backup mirror of every repo first, for example `Distorbed678/<repo>-private-backup` or a local bundle.
  - Force-push only after the user confirms.
  - Re-check the submodule pointers, since rewritten SHAs change: update the nested `.gitmodules` and gitlinks consistently.
- Re-run the scanners on the rewritten history. Acceptance is zero findings.

**WP D3: guides and repo hygiene**
- In the main repo:
  - `CONTRIBUTING.md`, the pull-request guide: setup, branch and PR flow, commit style, the tests and checks CI runs, the docs-with-code rule, and how RE findings are documented;
  - `docs/guides/hosting.md` reworked into a full **hosting guide**;
  - a new `docs/guides/client.md` **client and install guide**: install through the launcher; manual install with xtask; Linux/Proton and Windows.
- In every repo:
  - `SECURITY.md`;
  - issue and PR templates;
  - `DISCLAIMER` text in each README: unofficial, not affiliated with Grinding Gear Games, no game files distributed, users need their own legal copy, and use on official servers is read-only only;
  - consistent licenses (MIT OR Apache-2.0).
- Make CI work for public repos: replace `SUBMODULE_TOKEN` and the deploy keys with plain public checkouts, then remove those secrets and keys.

**WP D4: go public** (user confirmation gate)
- `gh repo edit --visibility public --accept-visibility-change-consequences` for each repo.
- Then verify a fresh anonymous `git clone --recursive` builds everything.

## Phase E: launcher releases with auto-update (task 5)
- **WP E1: release CI** in Poe2Launcher:
  - triggered by a tag `v*`;
  - Linux job: AppImage via `tauri build --bundles appimage`;
  - Windows job: NSIS `.exe` installer;
  - both upload to a GitHub release (tauri-action) with notes generated from the commits.
- **WP E2: auto-update:**
  - the Tauri updater plugin with the endpoint `https://github.com/Distorbed678/Poe2Launcher/releases/latest/download/latest.json`;
  - signing key generated with `tauri signer generate`; the private key and password are GitHub secrets (**user gate**: the user creates the secrets or approves them);
  - the public key embedded in the app;
  - UI: check on start and from Settings, show "update available", then download, verify and restart.
  - AppImage self-update on Linux; NSIS update on Windows.
- **WP E3:** a test release `v0.1.0`, then `v0.1.1`. Verify an installed v0.1.0 detects and installs v0.1.1, on Linux, and on Windows if a Windows machine or VM is available (otherwise document it as untested).
- Docs: `docs/guides/launcher.md` (download, install, update) and the release process in `CONTRIBUTING.md`.

## Phase F: server setup through the launcher (task 6)
- **WP F1: server release CI** in the main repo:
  - triggered by a tag;
  - builds `poe-server` for Linux and Windows. The nightly toolchain is pinned; check that `poe_data_tools` builds on Windows;
  - packages `config/server.example.toml` and the web-admin `dist`, with checksums.
- **WP F2: a "Host a server" page in the launcher:**
  - download and verify the server release matching the launcher's game version;
  - setup wizard:
    - server name and cluster id;
    - ports;
    - public host, auto-detecting the LAN IP and optionally the public IP, with a port-forwarding hint linking the hosting guide;
    - open registration on or off;
    - an admin account, created with `poe-server account create` and `role admin`;
    - the `game_dir` pointing at the launcher's installed game;
  - start, stop and status, with the log tail streamed from the process;
  - autostart optional.
  - The server profile is added to Servers automatically.
- **WP F3: an "Open web admin" button:**
  - Shown on a server profile when the logged-in account's role is admin. Get the role from `/login`, or add `GET /api/v1/me`.
  - Opens `<server>/` in the system browser.
  - Optionally passes a short-lived web token: add `POST /api/v1/web-token`, exchanging the launcher token for a web token, plus a SPA route that accepts it once. This avoids a second login.
- **Acceptance:** on a clean profile, the launcher sets up a local server, logs in, Play works, and Open web admin lands logged in.

## Verification (end to end)
- Every phase:
  - `cargo build && cargo test` in the main repo and every submodule;
  - `cargo xtask build-dll`;
  - npm `check`/`test`/`build` in web-admin and launcher;
  - CI green;
  - the link checker and `tracker --check`.
- **Phase A:** `cargo xtask check-build GameFiles/PathOfExileSteam.exe` reports all found. A mutated exe makes the right features disable themselves cleanly in game, with no crash.
- **Phase B:** the standalone login shows no suspicious-login popup, creation errors show the right text, and rate limiting triggers. All Login and Accounts tracker items are done.
- **Phase C:** drift numbers within target, two clients see each other undistorted on separate cells, the party and trade flows work, and a sample mod loads.
- **Phase D:**
  - The scanners are clean on the rewritten history.
  - An anonymous recursive clone builds.
  - The guides exist.
  - The disclaimer is present in every repo.
- **Phase E:** the AppImage and the Windows installer are attached to a release, and v0.1.0 auto-updates to v0.1.1.
- **Phase F:** the launcher hosts a server, the user plays on it, and Open web admin works.
- **Still needs the user from plan 1:** turn on Secret Service (KWallet) and do the Steam depot download for the launcher end-to-end test. This is now part of the WP E3 and F checks.
