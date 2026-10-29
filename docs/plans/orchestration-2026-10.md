# Orchestration plan: project restructure, accounts and multiplayer, launcher, web admin, movement rewrite

**Executor:** an Opus 5.5 orchestrator at medium effort. It delegates each work package (WP) to a Sonnet 5.5 subagent: `Agent(model: "sonnet", effort: "high", subagent_type: "general-purpose")`.

## Context

The PoE2 0.5.5e private server works for a single local player: login, characters, areas, transfers, chat, commands, and terrain for 25 of 33 act 1 seeds. It is not ready to grow, for these reasons:
- **Movement** was built without official captures. It needs PSY2 position reports, plus the `turn_cell`, `turn_timing` and `face_cursor` workarounds, and it uses `0x1cb` and `0x1df`, which official servers never send. Six official capture sessions now exist, and they show three skill replies we don't handle: `0x1ae`, `0x1af` and `0x1b1`.
- **No multiplayer:**
  - The DLL blocks or latches any non-loopback server.
  - Account = the client-sent machine hash.
  - The server depends on files the DLL writes into GameFiles.
  - Every session is a private simulation.
- **The repo layout is confusing:**
  - `Tools/` and `tools/` serve different purposes.
  - Dumped data is mixed into GameFiles (78 `terrain_grid_*.bin`).
  - Docs repeat the terrain status 4 times.
  - Comments say "M5" with an outdated meaning.
  - CLAUDE.md duplicates setup.md.
  - `config/server.toml` has dev-bridge keys.
- **New products need separate repos:** a launcher, a web admin, and the client hook as a future mod manager.

Intended outcome: a clean multi-repo project with a real per-cluster account system, official-style server-authoritative movement, and multiplayer where players see each other. It also has a web admin, a launcher that installs 0.5.5e from Steam depots and manages mods, condensed docs, a lean CLAUDE.md, and a README tracker. It ends with a handoff plan for porting the terrain generator to every area.

## Decisions already made with the user

| Topic | Decision |
|---|---|
| Launcher | Tauri 2 (Rust backend) + Svelte/TypeScript UI |
| Web admin | Admin/web API crate inside the server (axum: REST + WebSocket), plus an SPA in its own repo that the server can serve as static files |
| Auth | The DLL forces the Steam exe onto the standalone login screen (username/password to our login server), **and** the launcher can log in through the web API and pass a token for auto-login. Accounts are per cluster, passwords hashed with argon2. |
| Dumped data | Top-level git-ignored `dumps/<version>/…`. Its README and regenerate scripts are committed. |
| DLL split | Extract a shared **Poe2Protocol** repo (poe-proto, the codec parts of poe-net, `protocol/<version>/*.toml`). Server and client both use it. |
| Movement | Server-authoritative simulation on the walkability grid, official packets only. PSY2 becomes an opt-in drift checker, not an input. Built as an entity inside an instance world so multiplayer reuses it. |
| Steam depots (0.5.5e) | Content: `download_depot 2694490 2694491 7366357142270402355`. Windows exe: `download_depot 2694490 2694492 246779027001700092`. Download both into one directory: exe depot over content. |
| Repos | Private, under `Distorbed678`: **Poe2Protocol**, **Poe2ClientHook**, **Poe2Launcher**, **Poe2WebAdmin**, as submodules |

## Suggestions (included below; the user can strike any)

- **S1.** Do the structural docs cleanup and the new CLAUDE.md **early** (phase 1). Every later subagent then reads lean, correct context. Do a final consistency pass at the end (tasks 6 and 7).
- **S2.** Make the tracker data-driven. Statuses live in `docs/status/tracker.toml`, and `cargo xtask tracker` renders the README tables and progress bars between markers (in the style of the PS5 system-library and shader trackers). For terrain, the area × topology × orientation matrix is generated from `area_vectors` results, so nobody edits it by hand.
- **S3.** Move the deploy logic (gateway list, `poe-hook.toml`, `steam_appid.txt`, DLL install, Proton launch) out of `xtask/` into a `poe-hook-deploy` library in Poe2ClientHook. xtask and the launcher both call it.
- **S4.** Define a mod manifest spec (`mod.toml`) in Poe2ClientHook early. The DLL loader and the launcher then agree on one format.
- **S5.** Preserve git history when splitting repos (`git subtree split`), and add GitHub Actions CI to each repo (fmt, clippy, test).
- **S6.** Build a local two-client test harness (two Proton prefixes, two accounts, connecting via the LAN IP so the trusted-remote path is exercised). It makes multiplayer testable without a second machine.

## Target layout (task 10)

```
Poe2ModdingProject/
  CLAUDE.md  README.md (progress + tracker)  Cargo.toml  rust-toolchain.toml
  crates/            server libraries (+ poe-accounts, poe-web-api)
  servers/           login-server, instance-server, poe-server
  modules/           git submodules
    protocol/        Poe2Protocol   (poe-proto, poe-net codec, opcodes/<version>/*.toml)
    client-hook/     Poe2ClientHook (poe-hook, poe-hook-core, poe-hook-shared, poe-hook-deploy)
    launcher/        Poe2Launcher   (Tauri + Svelte)
    web-admin/       Poe2WebAdmin   (Svelte SPA)
  config/            server.toml (+ server.example.toml)
  docs/              (was Documentation/, restructured; see WP1.4)
  xtask/             (was tools/xtask)
  scripts/           (was tools/: test-run.sh, shot.sh, re/*.py)
  third_party/       (was Tools/: downloaded tools, reference repos, proton prefix; git-ignored except README + fetch script)
  dumps/0.5.5e.hotfix1/      git-ignored datamined data (see WP1.2)
  runtime/           (was data/: sqlite, logs, traces, crash dumps; git-ignored)
  GameFiles/  GhidraProject/   (unchanged, git-ignored)
```

## Orchestrator rules

1. **Start-up:**
   - Read CLAUDE.md, MEMORY.md and this plan.
   - Check `gh auth status`. If it fails, ask the user to authenticate before phase 1.
   - Keep progress in `docs/status/tracker.toml` once it exists, and in a work-state memory.
2. **One WP = one subagent prompt.** Each prompt contains:
   - goal
   - files and docs to read first (paths from this plan)
   - hard rules (below)
   - acceptance criteria
   - "update the docs in the same change and report the files changed"
3. **The orchestrator verifies, never the subagent alone.**
   - Read the diff.
   - Run `cargo build && cargo test`, plus `cargo xtask build-dll` when the client changed.
   - Then commit and push (`origin main`, and in the submodule repos first, then bump the submodule pointer).
4. **Parallel only when write sets don't overlap.** Use `isolation: "worktree"` for parallel coding WPs. Moves and renames (phase 1) run strictly in sequence.
5. **Hard rules repeated in every prompt:**
   - Never touch the Steam install or its Proton prefix.
   - GameFiles changes go only through deploy/revert.
   - Official servers are read-only. Claude never logs in to them; captures are the user's.
   - No hardcoded opcodes, ports or game data.
   - Tools go in `third_party/`.
   - RE findings go in `docs/re/`.
6. **Gates that need the user.** Pause with AskUserQuestion for:
   - gh auth
   - official captures (G-MOVE, G-MP)
   - Steam running for in-game tests
   - the Steam login for the DepotDownloader test
   - x64dbg help if a static RE spike stalls
7. **Unattended in-game tests:** start the server writing to the run log, then run `scripts/test-run.sh`, `send_dev.py` and `shot.sh` (the paths after phase 1). Follow the quiet-monitoring preference: respond only to crashes, exits, errors or the awaited event.

## Phase 1: Foundation (tasks 10, 12, 3, 8, 9, 11a, 7 skeleton). Strictly sequential.

**WP1.1 Restructure (task 10)**
- Move the folders to the target layout with `git mv`.
- Fix every path reference: `.gitignore`, root `Cargo.toml` members, xtask repo-root logic and the hardcoded `Tools/...` paths (`xtask/src/main.rs`), `config/server.toml` paths, `scripts/*.sh`, the `send_dev.py` log path, `third_party/fetch-tools.sh`, and doc links.
- Move existing local dirs on disk to match (`Tools`→`third_party`, `data`→`runtime`).
- Acceptance: `cargo build`, `cargo test` and `cargo xtask build-dll` pass, and `grep -r` finds no stale `Tools/`, `tools/`, `data/` or `Documentation/` paths.

**WP1.2 Dumps folder (task 12)**
- Create `dumps/0.5.5e.hotfix1/` with this structure:
  - `terrain/grids/` (terrain_grid_*.bin)
  - `terrain/layouts/` (terrain_layout_*.tsv)
  - `terrain/area-exports/` (area_export and generator_trace sessions)
  - `terrain/traces/` (act1_traces)
  - `terrain/vectors/` (sweep/terrain vectors)
  - `client-tables/` (object_types.txt, terrain_tiles.txt)
  - `ghidra/` (named functions, factory/vtable JSON from `docs/re/*.json`)
  - `captures/official/` and `captures/local/` (packet sessions copied from `GameFiles/logs/poe-hook/`)
  - `strings/`
- Add a committed `dumps/README.md` and a `scripts/dumps/migrate.sh` that sorts the existing files from GameFiles and runtime into place. Moving out of GameFiles is allowed here, because these are our generated outputs, not game files.
- Add a DLL setting `dump_dir` (poe-hook.toml, written by deploy as an absolute path), used by `client/poe-hook-core/src/dump_types.rs`. Server `[spawn] layout_dir` becomes `dumps/0.5.5e.hotfix1/terrain`.
- Fix `test-run.sh` so it also cleans grids. Delete unreferenced duplicates (`terrain_vectors.tsv`) only after showing the list.
- Acceptance: the server still spawns correctly in an in-game test, and nothing new appears in GameFiles.

**WP1.3 Repo split (tasks 3 + protocol)**
1. `git subtree split` `crates/poe-proto` and `protocol/` (and the codec and cipher parts of `crates/poe-net`; feature-gate tokio) into **Poe2Protocol**.
2. Do the same for `client/` into **Poe2ClientHook**.
3. Create the repos with `gh repo create --private`. Add them as submodules under `modules/`.
4. Repoint path deps:
   - `poe-hook-core` uses `poe-proto` from the protocol submodule; the client repo carries Poe2Protocol as its own nested submodule or a git dependency.
   - Replace `include_str!("../../../protocol/…")` in `packets.rs:164,170` with a constant exported by the protocol crate.
5. Move the deploy code from xtask into `poe-hook-deploy` (S3). xtask keeps thin wrappers.
6. Add a README to each new repo.
- Acceptance: each repo builds standalone after a fresh `git clone --recursive`, the main repo builds, and `cargo xtask deploy-client` plus an in-game login still work.

**WP1.4 Docs condense and new structure (task 8)**
- Rename to `docs/`:
  - `index.md`
  - `guides/` (setup, server-admin, hosting/multiplayer, launcher, modding)
  - `architecture/` (overview, crates, accounts, instance world, web API)
  - `protocol/` (framing, login, instance, entities, movement)
  - `re/` (addresses-0.5.5e, crypto, `terrain/` = terrain.md and dungeon.md merged and deduplicated, captures.md = official-comparison)
  - `status/` (tracker.toml, terrain.md, history.md)
  - `research/`
  - `plans/`
- One source of truth per topic:
  - Terrain status only in `status/terrain.md`; remove the copies in dungeon.md, terrain.md and milestones.
  - Commands and logs only in `guides/setup.md`.
  - Rules only in CLAUDE.md.
- Fold `findings-2026-10-13.md` into its topic docs.
- Turn the milestone logs into a short chronological `status/history.md`.
- Retire the "M3.5/M5" vocabulary: name things by topic (terrain generator, collision, server movement).
- Acceptance: a link checker script passes, and no topic is described in two places.

**WP1.5 CLAUDE.md (task 9)** Under about 40 lines:
- a 3-line summary
- the hard rules (unchanged in meaning, new paths)
- a short roadmap (phase → status, pointing to the README tracker)
- a "where things are" pointer list to `docs/`

Move the commands and the RE how-to into `docs/guides/setup.md` and `docs/re/README.md`. Update memory paths.

**WP1.6 Config pass 1 (task 11a)**
- Restructure `config/server.toml`:
  - Merge `[instance]`/`[instances]`.
  - Group the network settings (`bind`/`public_host`/`public_port`) into `[network.login]` and `[network.instance]`, with one `public_host`.
  - Move dev and research keys (`sweep*`, `known_seeds`, `movement_trace`, `debug_link`) into `[dev]`.
  - Group the account-protocol flags under `[accounts]`.
  - Move `[spawn]` to the dumps paths.
- Update the `poe-config` structs and defaults (`crates/poe-config/src/lib.rs`), with a migration note in the docs.
- Add `server.example.toml`.
- Movement keys stay until phase 2; accounts and auth keys are added in phase 3.

**WP1.7 Tracker skeleton (task 7, S2)**
- Write `docs/status/tracker.toml`: subsystems → items → status, each with an optional link.
- Add `cargo xtask tracker` to render it into README marker blocks: a progress bar per subsystem, ✅/🟡/❌/⬜ tables, and a "current task" banner.
- Rewrite README.md: what the project is, how the repos fit together, quick start, then the tracker.
- Subsystems: Login, Accounts, Instances/Transfers, Movement, Multiplayer, Chat/Commands, Terrain (per-area matrix later), Inventory, Monsters/Skills, Client Hook, Launcher, Web Admin, Modding.

## Phase 2: Movement rewrite (task 4, then config pass 2, task 11b)

**WP2.1 Capture analysis (RE, no code)**
- Work through the 6 official sessions (now in `dumps/0.5.5e.hotfix1/captures/official/`; files are `packets.jsonl` and `packets.txt`). Use `crates/poe-proto/examples/check_capture.rs` and `scripts/re/packets_diff.py`.
- Decode `0x1ae`/`0x1af`/`0x1b1` (they look like replies to `0x1a5`/`0x1a8`/`0x1ac`), using Ghidra for the field layouts. Also decode `0x1ac`.
- Extract the exact sequences for each of these, with frame timing against `0x2f4` ticks:
  - WASD start, turn and stop
  - click-to-move (single click and held)
  - dodge roll (standing, moving, chained)
  - sprint start and stop (`0x2e6`/`0x2e7` plus any buff or StatsUpdate)
  - combat stance (`0x2f9`)
  - aim (`0x62`/`0x1dc`)
- Output:
  - `docs/protocol/movement.md` rewritten as a spec (state machines + sequences)
  - the new opcodes in `instance.toml` and their structs in Poe2Protocol
  - a **gap list**
- **Gate G-MOVE:** if any sequence is missing (for example sprint into roll, or held click with re-aim), the orchestrator gives the user a capture checklist for one read-only official session, then repeats WP2.1 on the new capture.

**WP2.2 Instance world + movement design**
- In `crates/poe-game`, add:
  - a `World` (entities with ids, a frame clock)
  - a movement component (states: Idle / Walking(direction or target) / Rolling / Sprinting) that consumes client input messages and emits official replies per frame
  - a `GridSource` trait over the existing `Walkable` (`crates/poe-game/src/movement.rs`), implemented by the client-dump grid now and by poe-world later
- Reuse `Mover` (advance/slide) and the roll speed profile where the captures confirm them. Reuse `pathfinding.rs` A* for click-to-move.
- The design must allow N player entities (phase 3 reuses it).

**WP2.3 Implement**
- Replace the movement logic in `servers/instance-server/src/player.rs` (1372 lines) with thin session glue to the World.
- Delete `TurnCell`, `turn_timing`, `face_cursor`, `cursor_to_grid`, and the use of `0x1cb`/`0x1df` for movement.
- Turn PSY2 (`sync.rs`, `position_sync.rs`) into a drift checker only: it logs server vs client position, and is off by default.
- Tests: replay test fixtures built from official capture input sequences. They assert the reply opcodes, field values and frame offsets. Keep and port the existing `player.rs` and `movement.rs` tests that still hold.

**WP2.4 In-game verification + cleanup (task 11b)**
- Run test-run.sh scenarios (WASD, click, held click, roll, sprint) with the drift checker on. Drift should stay under one cell on open ground.
- Remove the `turn_cell`, `turn_timing` and `face_cursor` config keys and server commands, and the DLL's `movement_log` if obsolete. Update the docs and tracker.

## Phase 3: Accounts and multiplayer (task 5; follows docs/guides/multiplayer, formerly multiplayer.md)

**WP3.1 RE spike: standalone login on the Steam exe (Ghidra MCP)**
- Find how `PathOfExileSteam.exe` chooses the Steam login vs the email/password login screen: look for the strings for login UI and the "Steam" checks around the `0x0005` builder (`crates/poe-proto/src/login.rs:94`, addresses in `docs/re/addresses.md`). Also find what the standalone path sends: credentials, field layout and flags.
- Output: `docs/re/standalone-login.md`, plus the hook point and patch.
- If static RE stalls, ask the user for x64dbg help.
- **Fallback (decide here):** the launcher token goes into the auth packet's unused bytes or the ticket field, and the Steam main menu is kept.

**WP3.2 Accounts (new crate `crates/poe-accounts` + `poe-store` migration 0006)**
- accounts: username (unique, case-insensitive), argon2id password hash, role (player/admin), created/last login, disabled flag
- sessions/tokens: launcher and web tokens with expiry, revocable
- cluster id in config, so tokens are not valid across clusters
- Login server authenticates by credentials or token instead of `machine_hash` (`servers/login-server/src/session.rs:246-265`). A dev-only option keeps the machine-hash auto-create.
- Duplicate login kicks the old session (`poe-cluster` `MemorySessions`).
- Admin comes from the role. Drop `[admin] accounts` (`exec.rs:263 is_admin`).
- Registration: a server command and the web API (WP4.1). Optionally open registration, as a config switch.

**WP3.3 DLL (Poe2ClientHook)**
- Force the standalone login screen (WP3.1).
- Add a `trusted_servers` list in poe-hook.toml (written by deploy and the launcher). Trusted servers are neither blocked nor latched, and the crypto bypass applies to them. `guard.rs`, `crypto_bypass.rs`, `ui/mod.rs custom_server()` and `abi.rs Connection.external` all use the list.
- Official servers stay read-only exactly as today.
- Token auto-login from a launcher-provided token.

**WP3.4 Remote readiness**
- Advertised host config, already grouped in WP1.6.
- Layout and grid upload over the network: the first client to generate an area uploads it, and the server caches it per tile hash in `dumps/…`. It replaces the shared-disk read.
- PSY2/debug-link datagrams get a per-session HMAC token (a key from the ticket keys) and are keyed by session, not by TCP port.
- Port-forwarding guide in `docs/guides/hosting.md`.

**WP3.5 Gate G-MP + remote-player protocol**
- The user captures (read-only) a town with other players moving, joining and leaving.
- A subagent decodes the remote-player AddObject (what differs from the local blob), the remove message, action broadcasts and any party or instance-ownership bits.
- Document it in `docs/protocol/entities.md`.

**WP3.6 Instance actor**
- Refactor `run_session` (`servers/instance-server/src/lib.rs`) into session ↔ instance actor messages. Each instance owns one `World` (WP2.2), unique entity ids, and one frame clock.
- Send `0x0024 PlayerEntity` per client.
- Remote player add/remove, interest management (whole instance at first), relay of the other players' movement and actions in the right frame.
- `MemoryInstances` holds the actor handles.

**WP3.7 Two-client test (S6)**
- Add `scripts/test-mp.sh`: two prefixes, two accounts, LAN IP.
- Acceptance: both clients log in with passwords, enter the same town, see each other walk and roll, chat, and transfer. Duplicate login kicks the first session.

## Phase 4: Web API + Web admin (task 2)

**WP4.1 `crates/poe-web-api` (axum, run inside poe-server)**
- Public endpoints: register, login → token, change password, server info.
- Admin endpoints (admin role):
  - sessions/online (`live.rs`)
  - instances list/close/new (`MemoryInstances`)
  - kick/mute/announce/transfer (reuse the `commands.rs`/`exec.rs` registry)
  - accounts (list, disable, role)
  - characters (list, detail from `poe-store`)
- Logs over WebSocket: a tracing layer with a ring buffer, plus a filter.
- It serves the built SPA from a configured directory.
- Config: `[web] bind`, `static_dir`, CORS.
- Tests use axum's test client.

**WP4.2 Poe2WebAdmin (Svelte 5 + Vite + TS)**
- Pages: login; dashboard (online, instances); live log viewer with level and target filters; instances (players, close, transfer); accounts; characters (basic info now).
- Inventory and passive-tree panels are stubbed behind feature flags, waiting for the inventory work. Track them in the tracker.
- Generate the API types from an OpenAPI spec emitted by poe-web-api (`utoipa`). The launcher reuses them.

## Phase 5: Launcher (task 1)

**WP5.1 Poe2Launcher (Tauri 2 + Svelte)**
- **Server profiles:** cluster URL, account login through the web API, stored token in the OS keyring. Register flow.
- **Game install:**
  - Bundle or fetch DepotDownloader.
  - A per-version manifest file `versions/0.5.5e.toml`: app 2694490, depot 2694491 manifest 7366357142270402355 (content), depot 2694492 manifest 246779027001700092 (Windows exe).
  - Download both into a launcher-managed directory (exe depot over content) using the **user's own Steam login** (prompted, Steam Guard supported), then verify.
  - Never read or write the Steam install.
- **Deploy and launch:** use `poe-hook-deploy` (S3): gateway entry, DLLs, poe-hook.toml with `trusted_servers` and the token, `steam_appid.txt`. Launch natively on Windows, or through Proton on Linux with a launcher-owned prefix. Steam must be running.
- **Mods:** install, enable/disable and order mods by the `mod.toml` spec (S4). Mod files go to the hook's `poe-hook-mods/` dir, never into the game files directly. Check version compatibility against the game version.
- Acceptance: on a clean directory, the launcher downloads 0.5.5e, logs in to the local cluster, launches, and the client auto-logs in.
- The user is needed for the Steam login during the test.

## Phase 6: Consistency pass (tasks 6, 7)

**WP6.1 Comments sweep**
- Grep all repos for milestone tags (`M3`, `M5`, `M8`), "dev bridge", removed config keys and stale paths.
- Rewrite each comment to the current behaviour. The examples found so far: `spawn.rs:3`, `sync.rs:1`, `position_sync.rs:1`, `player.rs`, `movement.rs:13`, `poe-config/src/lib.rs:155/251/283`, `seeds.rs:5`, `dump_types.rs:276`, `poe-hook-shared/src/config.rs:69`, `instance-server/src/lib.rs:1025`.

**WP6.2 Docs audit**
- Check every doc against the code (one subagent per docs folder, in parallel; this one is read-mostly).
- Fix the drift. Re-run the link checker.

**WP6.3 Tracker final**
- Update `tracker.toml`.
- Set "current task" = the terrain port to all areas.
- Regenerate the README. Update CLAUDE.md's roadmap.

## Phase 7: Terrain handoff plan (task 13; last)

**WP7.1 Plan only, no generator code.**
- Review `docs/status/terrain.md`, `docs/re/terrain/`, `crates/poe-world` (24 modules, 16 examples) and the verification tools (`area_vectors`, `area_trace`, `check_exports`, sweep).
- Write `docs/plans/terrain-all-areas.md` for the next agent. It covers:
  1. **Scope inventory:** every WorldAreas row that generates terrain (from poe-data). For each, its generator type (.dgr/.tgr/town/fixed) and the topologies and orientations it supports (from the graph files and AreaInfo). The output is a `cargo xtask terrain-matrix` that writes the tracker matrix: area × topology × orientation → untested/fail/match.
  2. **Verification at scale:** client traces for forced topologies and orientations through the already-verified AreaInfo stat records, sweep automation across all acts, and storage in `dumps/0.5.5e.hotfix1/terrain/`.
  3. **Known blockers carried over:**
     - G1_12 grid overlap (+0x14b)
     - G1_4 overlay phase 2
     - G1_5 "edge 14"
     - G1_11 `woodscliff_height.et` chirality
     - world-map-pin topology choice
  4. **Unported systems ranked by how many areas each unlocks:** doodads and the doodad hash, 3×3 door spots, FileGroups and nested subgraphs, extra rooms and shrines, and collision-grid output for server movement (feeds the `GridSource` from WP2.2).
  5. **Work packages** sized for Sonnet subagents, the order, and the acceptance per package.
- Update the tracker's terrain section with the generated matrix.

## Verification (end to end)

- Every phase: `cargo build && cargo test` in the main repo and each submodule. `cargo xtask build-dll`. CI green (S5).
- Phase 1: in-game login and area entry still work. GameFiles holds only game files plus the deployed hook. The link checker passes. `cargo xtask tracker` renders the README.
- Phase 2: capture-replay fixtures pass. In-game WASD, click, held click, roll and sprint look right, with the drift checker under one cell. No `0x1cb`/`0x1df` used for movement.
- Phase 3: `scripts/test-mp.sh`. Two password-authenticated clients see each other move. A remote-IP (LAN) client connects through `trusted_servers`. An official connection is still read-only-latched (manual check by the user).
- Phase 4: the web UI shows live logs, instances and characters. Admin actions take effect in game.
- Phase 5: a clean install of 0.5.5e through the launcher, auto-login, and a sample mod enabled.
- Phase 7: `docs/plans/terrain-all-areas.md` exists, and the tracker shows the full area matrix.
