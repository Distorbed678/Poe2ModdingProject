# Handoff: orchestrator 4 (2026-10-23)

For a new orchestrator. Read [CLAUDE.md](../../CLAUDE.md), your MEMORY.md, [docs/overview.md](../overview.md), [handoff-orchestrator-3.md](handoff-orchestrator-3.md) (repos, releases, how to work, the user's earlier decisions: still valid), then this file. Orchestrator 4 started from handoff 3, wrote [open-items.md](open-items.md), then spent the session on the terrain 1:1 port at the user's request.

## Start-up checks
1. `gh auth status`; `git status` and `git submodule status --recursive` in the hub (all repos are submodules under `modules/`, nested ones too). Report anything uncommitted or out of sync.
2. CI of all repos (`gh run list -R Distorbed678/<repo> -L 3`), `cargo xtask versions --check`, `cargo xtask tracker --check`, `python3 scripts/check-links.py`.
3. No leftover processes: `ps -eo pid,etime,pcpu,args | grep -E "forced_verify|terrain_batch|PathOfExile"`.

## How to work (additions to handoff 3)
- Each work package goes to a **fresh** Sonnet subagent (`model: "sonnet"`, `effort: "high"`, general-purpose, background). Give it a self-contained prompt: files to read, data paths, its file set, the regression gate, the hard rules, "report once and end".
- **Terrain waves:** three agents in parallel, each in its own Poe2Server worktree: `git -C modules/server worktree add ../../runtime/wt/<x> -b terrain-<x>`. `git submodule update --init` fails there, so agents symlink `modules/{client-hook,protocol,web-admin}` to `modules/server/modules/*` and commit with `git add crates` only. Build with `CARGO_TARGET_DIR=runtime/wt/<x>-target` and `--manifest-path runtime/wt/<x>/Cargo.toml`, running from the hub. Local commits only; the orchestrator merges into Poe2Server `main` after the gate, pushes, bumps the hub, and removes the worktrees.
- **Only one agent at a time uses the game client** (the batch). The others write their client requests to `runtime/<x>/client-requests.txt` (`area seed topology orientation trace|export`), and the client agent or the orchestrator runs them.
- **Process hygiene:** agents kill only PIDs they started (one used `pkill -f` and killed another agent's run). Never wait with `while pgrep -f <pattern>`: it matches its own shell and never ends (two such loops ran for hours). Wait on a PID (`while kill -0 <pid>`) or use a background task notification.
- **Usage limits:** when the user's usage runs low they ask to "stop at a safe point". Message every agent to finish or revert the edit in progress, commit locally (WIP allowed), stop its processes, send one short report and end. On resume: if the same account continues, `SendMessage` each agent to continue. If the user switched accounts and cannot afford reloading the agents' contexts, start fresh agents with self-contained prompts built from the last reports (that worked well).
- Agents tend to resend their report while their own background job runs: tell them to report once and end.
- **Permission classifier:** in auto mode it once blocked starting an agent ("Data Exfiltration", a false positive for a doodad/grid task). Ask the user to switch to manual mode; do not work around it.
- Questions to the user go through AskUserQuestion with the recommended option first. Report when a WP finishes, at a gate, or on a failure. Keep status messages short.

## User decisions of this session (2026-10-22/23)
- **Items and entities are not implemented: do nothing that depends on them.** Remote equipment visuals, dynamic obstacles in the pathfinder, and party/trade accept/decline are deferred ([open-items.md](open-items.md)).
- **Terrain 1:1 port:** tile hash, doodad hash and collision grid; default league mode only (4508 topology x orientation combinations, the 84 league-mode topologies are out of scope). Acceptance: the client generates every combination with 50 random seeds and the port matches every one (or a documented reason).
- Method: in-client batch generation (`gen_batch` in the hook, `scripts/re/terrain_batch.py`, [../re/terrain/batch.md](../re/terrain/batch.md)). x64dbg is allowed on the private client.
- Waves of three parallel agents, as above.

## What is left from the original plan (handoff 3, before the terrain work)
In the order the user gave on 2026-10-22:
1. **The user's end-to-end test** of launcher v0.4.0 (AppImage): install the game (Steam depot), get the hook, Host a server (first real update from server-v0.1.0 to 0.2.1), Play, web admin, restart. Not done yet. Fix what the user reports. Before the user starts it: no batch or client test running. The short form is in [../guides/launcher.md](../guides/launcher.md#end-to-end-test-on-a-real-machine).
2. **G-PARTY and movement problem 5 capture** (the user's, on official servers): before it, nothing running and `cargo xtask deploy-client --official`. Afterwards `deploy-client` back to private. Implement **movement problem 5** (sprint ended with W held, [../protocol/movement.md](../protocol/movement.md)). Party/trade accept and decline stay deferred (items and entities), but the capture can be taken.
3. **Small open items** ([open-items.md](open-items.md), "Can be done without the user"): mod overlays `config_defaults` and server `opcodes`, class names on the web admin Players page, ban commands in chat and CLI, a rate limit on the mod download endpoint, the launcher's nested client-hook pin (875afe4 -> current), and reconciling the movement gaps #11/#12 tracker rows. **Ask the user which ones first.**
4. **Windows** (needs a Windows machine or VM): launcher installer and update, Host a server, Play, restart.
5. Then the terrain work below.

## Terrain: where it stands (pushed)
- Full client batch: `dumps/0.5.5e.hotfix1/terrain/batch/full/` (225,400 jobs = every combination x 50 seeds; 218,705 generated, 6,695 client-side generator errors). One run takes 1.3 h (8 threads, ~16 GB memory cap with automatic client restarts).
- **The port matches 108,545 of 218,705 client layouts** (37,380 at the first run, 72,718 after wave 2). 108,518 of them also match the doodad hash (27 differ). Combinations: 1074 match on all seeds, 1658 partial, 1737 fail, 39 untested (the client fails every seed). Poe2Server `2167c89`, hub `533d934`. Per-wave history: [../status/terrain.md](../status/terrain.md).
- Collision grid: equal to the client's `grids.bin` for 273 of 303 exports. The server switch `[terrain] generated_grid` (default off) uses it only when the tile hash equals the client's, else the uploaded grid. Not yet tested in a live game.
- **Regression gate** (from the hub, after building the branch's examples):
  `POE_FORCED_BUDGET=60 <target>/release/examples/forced_verify dumps/0.5.5e.hotfix1/terrain/batch/full/batch-results.tsv --out runtime/forced-<x>.tsv --threads 14` (about 60-70 min; a budget of 20 aborts on slow `G1_13_1` jobs). Compare with the baseline `runtime/forced-baseline.tsv` on (area, seed, topology, orientation), column 5: no `match` may turn into a fail. After merging: copy the result into `dumps/0.5.5e.hotfix1/terrain/vectors/forced-results.tsv` (merge by key), `cargo xtask terrain-matrix && cargo xtask tracker`, update terrain.md, commit the server first, then the hub. Use the new result as the next baseline.
- Tools (poe-world examples): forced_verify (with the doodad hash), layout_diff, lay_cmp, tgr_diff, area_trace (client generator traces), check_doodads, check_grids, area_probe, area_vectors. Env debug switches are listed in [../re/terrain/dungeon.md](../re/terrain/dungeon.md) and [../re/terrain/tgr.md](../re/terrain/tgr.md).
- Client runs (git-ignored, `dumps/0.5.5e.hotfix1/terrain/batch/`): fails1 (2534 failing jobs, before wave 2), d1 (1850, one failing seed per dungeon combination), d2 (132 traced, 3 per area for the 45 largest failing dungeon areas), a1/a2 (act 1, traced), b1/b2/b3 (tgr, traced), c1 (containers, traced). Many of these are outdated now that the port has changed: re-derive failing jobs from the latest gate file, not from these lists.
- Pending client requests: `runtime/e/client-requests.txt` (chirality leftovers), `runtime/f/client-requests.txt` (grid fails: lattice dumps for MapSlash, MapReservoir, MapCrimsonShores, MapSeepage, MapMonument; MapVaalVault angle trace).

## Wave 4 (proposed; ask the user before starting)
First rebuild a fresh failure breakdown from `runtime/forced-baseline.tsv` (classes by error text, "tile hash" = wrong output, "map" = size), because waves 2-3 moved many jobs between classes. Then three agents:
- **A, dungeon (`.dgr`), the client user.** The open list from wave 3:
  - **Strips and wiggles:** lattice strips in `buried_roof_fill` rooms (G2_4_1, G2_5_2, G2_9_2; suspect filler height `0x1420b2520`); exit-wiggle set 139 vs 140 (G2_8, MapSpiderWoods, MapSteamingSprings; draw 2974, `0x1420a219f`).
  - **Divergence points:** MapLostTowers existence rolls; G3_6_1 typed room pick `0x1420aed49`; G3_5 `0x1420b13dc`; corridor shuffle (MapDerelictMansion, G2_2, G3_3); G1_7 corridor tie-break (A* pop order `0x1420a44b0`); G1_11 overlay `room_fits` margin (draw 1055).
  - **Not started:** 3x3 door spots `0x14209de10` (WP7.8, ~15,000 jobs, e.g. MapBazaar 2,031); "could not fix heights" in the outer fill (MapCreek, MapMesa, MapUniqueInitialTower, P3_1, MapKarst); map-size jobs; FileGroups (G2_3, G3_1); EdgeFields v19 (`Programming`); MapDerelictMansion border tile; G4_13 overlay room without root key.
- **B, `.tgr` leftovers, offline** (traces in batch/b3): G2_1 "could not route edges" (room placement or repulsion differs); MapBackwash "two virtual edges side by side" (`0x1420c6670`, ~800 jobs) and its bridge "no room fits"; MapInferno topology 1 (river edge at tile (18,33)); MapUniqueWildwood "loop chain" (`edge_tiles.rs`); MapSinterRift `desert_dune.et` node tile; G1_1 virtual node tile at the room stage (189) and lumberjack "no room fits"; G1_2 "no room fits" (campsite, boss, endcap); border-tile orientation diffs (MapDeforestation, G1_5 621990373 8:3); overlay pieces never checked against data. Also remove the env-gated debug prints B left in routing.rs, node_tiles.rs, lattice.rs and fill.rs.
- **C, towns, grid and doodads, offline:**
  - Towns with wrong output (~3,800 jobs, 92 town/hideout areas; some hashes vary with quest state).
  - The 30 collision-grid fails: lattice flag 8, the tile-set option `+0x165` in `0x141fe1150`, the stubbed `sealed_inside` (`.tdt` `+0x16f` bit 0), and a circular blocker in MapCrimsonShores that is not in doodads.tsv.
  - The 27 doodad-hash differences.
  - The MapVaalVault angle (`random_orientation`).
  - Then a live in-game test of `generated_grid` with server movement (needs the client: coordinate with A).

## Terrain after wave 4 (the rest of [terrain-all-areas.md](terrain-all-areas.md))
- Further waves until every combination is `match` on all 50 seeds or has a documented reason. Biggest remaining levers in the plan: rooms (no room fits, feature rooms `GetFeatureRooms`, Incursion temples), special rooms, the lettered `unique<N>` tags (`area.rs:220`, needs a session-history trace), virtual nodes and edge pairs, and the rare features of WP7.14.
- **Fresh full client batch** with new random seeds once the match rate is high, so the port is not tuned to one seed set (the plan's T3 idea; 1.3 h).
- **WP7.13:** the instance server always sends `map_force_topology` / `map_force_orientation` equal to the generator's own pick (exposed as a function; keep a `natural` override for the admin command). The forced index is the raw WorldAreas list; the mode-1 entries are a prefix, so the matrix ids agree. `exec.rs` `new_instance` checks against the mode-1 list.
- **Out of scope or last:**
  - The 10 `fixed` WorldAreas rows (no topologies).
  - The 6 world-map panels (the client logs no hashes: export compare only).
  - `IncursionTemplePresent` (needs session data).
  - The effect of successful league mechanic rolls (the rolls themselves are ported).
- **WP7.15:** close the matrix (every combination match / fail with a reason / untested with a reason). Then decide with the user whether `[terrain] generate` and `generated_grid` default to on, which removes the need for the client's terrain upload.
- Release: after a large terrain step, offer a server release (`server-v0.2.x` in Poe2Server, process in handoff 3, "What is left" 1) so the launcher's Host page gets it.

## State left behind
- All repos clean and pushed. No worktrees, no agents, no processes running.
- GameFiles/: private mode, current hook DLL with `gen_batch` (off unless `POE_HOOK_GEN_BATCH` is set).
- Disk: `dumps/0.5.5e.hotfix1/terrain/batch/` holds about 50 GB of exports and traces (fails1 alone is 26 GB). Prune old run dirs when space matters (267 GB were free on 2026-10-22).
