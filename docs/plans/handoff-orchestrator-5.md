# Handoff: orchestrator 5 (2026-10-24)

For a new orchestrator. Read [CLAUDE.md](../../CLAUDE.md), your MEMORY.md, [docs/overview.md](../overview.md), [handoff-orchestrator-3.md](handoff-orchestrator-3.md) (repos, releases, how to work), [handoff-orchestrator-4.md](handoff-orchestrator-4.md) (terrain method, gate, user decisions: still valid), then this file.

## What happened in this session
- The user's launcher end-to-end test (v0.4.0, Linux) passed (tracker updated).
- **Launcher v0.4.1** released: started from the AppImage, the launcher passed the AppImage's environment (`PYTHONHOME`, `PYTHONPATH`, `LD_LIBRARY_PATH`, ...) to Proton, which could not create a new prefix ("Proton could not create the prefix ... exit status: 1"). `src-tauri/src/child_env.rs` now cleans the environment of every program the launcher starts.
- Terrain **wave 4** and **wave 5** merged: the port matches **196,094 of 218,705** client layouts (108,545 before wave 4, 176,336 after it), 0 lost; matrix 3286 match / 877 partial / 306 fail / 39 untested. Details: [../status/terrain.md](../status/terrain.md); findings: the "Wave 4" and "Wave 5" sections of [../re/terrain/dungeon.md](../re/terrain/dungeon.md) and [../re/terrain/tgr.md](../re/terrain/tgr.md).
- The small open items ([open-items.md](open-items.md)) were postponed by the user ("later").

## How to work (additions to handoffs 3 and 4)
- **CPU limits.** The user's editor crashed at 100% CPU with three agents. Every build, gate and client batch runs with `nice -n 19`, `CARGO_BUILD_JOBS=4`, at most 4 threads (the orchestrator's full gate: 8 threads); one heavy job per agent. Processes survive an editor restart: after one, look for orphaned gates with `ps` and give the agents their PIDs instead of letting them rerun. The hub has a local, uncommitted `.vscode/settings.json` that keeps VS Code's indexer out of `dumps/` and `runtime/` (ask the user whether to commit it).
- **Agents commit the worktree's submodule symlinks.** It happened twice (`modules/*` turned into mode 120000). A pre-commit hook in Poe2Server's common git dir (`.git/modules/modules/server/hooks/pre-commit`) now refuses commits that stage `modules/` on a `terrain*` branch. Still check `git diff --stat <base> HEAD -- modules` per branch before merging.
- **Usage limits and restarts.** The user's usage ran out several times and the editor restarted twice. On "stop at a safe point" every agent commits (WIP allowed, marked NOT GATED) and rewrites `runtime/wave5/<x>/handoff.md` for a fresh agent. The user prefers **fresh agents from those handoffs** over resuming old ones (no re-caching); that worked well three times.
- **Merge:** merge all branches into a merge worktree, resolve conflicts (both sides usually needed: two agents extended the same function), build, `cargo test --workspace`, then **one full gate on the merged code**; only if a match is lost, bisect on the lost jobs. Do not run `cargo fmt` in Poe2Server (the repo is not fmt-clean); Poe2ClientHook's CI does check fmt.
- The auto-mode classifier blocked an agent start once more ("Data Exfiltration", false positive); the user switched to manual mode and it worked.
- Ghidra must be open for the Ghidra MCP; subagents load its tools with `ToolSearch` (`select:mcp__ghidra__decompile_function,...`); fallback `scripts/re/disasm.py`.
- The KWin Agents desktop disappears after a reboot: `scripts/agent-desktop.sh on` recreates it (the user wants the game client there while agents work).

## Terrain: what fails now (baseline `runtime/forced-baseline.tsv`, 22,611 jobs)
- **.dgr: 22,419**, of which 20,473 wrong output. Biggest areas: G1_13_1 2,623, G1_13_2 1,842, G2_4_1 1,600, MapFrozenFalls 1,548, G4_1_1 981, G2_4_3 777, G2_4_2 709, MapUberBoss_JadeCitadel 600, G3_1 561, G3_6_1 441, G4_13 400, MapSunTemple 400, MapKarst 400, MapExcavation 400. Errors: could not fix heights 927 (MapKarst: PairedRoomLayoutPlugin `0x141f01890` not ported), map size 400, no tile fits 184, Gallows tiles 100, feature room as a dungeon room 63.
- **.tgr: 92** (Design 50, P2_1 39, MapCaldera 2, MapCrimsonShores 1). **Towns: 100** (HideoutLimestone 50: the client skips the levelling rounds there, condition unknown; G3_town 50).
- Doodad hash: equal on every tile match but one. Collision grid: 30 of 303 exports still differ (second unwalkable-corner pass `POE_GRID2`, untested), not yet tested in a live game.

## Wave 6 (proposed; the leads are in the wave 5 handoffs)
Start fresh agents from `runtime/wave5/{a,b,c}/handoff.md` and the brief `runtime/wave5/README.md` (rename to wave 6, new base commit, new job lists from the baseline):
- **A (client user):** merged-edge weight (class `0x142096993`, G1_13_1: a merged edge counts 0 in the client; a first try gave +52 / -27 on 300 jobs), the sub-dungeon fill ring sharing occupancy with the parent (`0x141f90227`, MapFrozenFalls), encounter/mission requests (`0x1420ab7e7`), door-pick candidate count (`0x141fa8323`), `0x141fe7737` (G2_4_3). Serve the others' client requests first.
- **B:** node tiles through the settle `0x1420c93e0` before `place` (`0x1420c9af0`; about 1,500 G3_1 jobs), 2x2 `*BendLarge*` corridor tiles on G1/G4_WorldMap (348 jobs), MapKarst's pairing plugin, G4_13, G4_4_2, MapSinkhole, MapPenitentiary. Pending client traces: G1/G3/G4_WorldMap, G1_15, MapKarst, G4_13, G4_4_2 (`runtime/wave5/b/client-requests.txt`).
- **C:** the 1,401 stone-circle jobs that still fail (MapCrypt, MapHeadland, MapSpiderWoods, ...; traces requested: MapWillow, MapSump, MapCrypt), MapDerelictMansion's feature room placed as a dungeon room, HideoutLimestone, Design, P2_1, G3_town, the collision grid (`POE_GRID2`, `check_grids`).
- After wave 6: the fresh client batch with new seeds, WP7.13, WP7.15 and the `generate` / `generated_grid` default decision ([handoff-orchestrator-4.md](handoff-orchestrator-4.md), "Terrain after wave 4"); offer a server release.

## State left behind
- All repos pushed: Poe2Server `b021b44`, client-hook `811a3a7`, launcher `v0.4.1`; the hub commit that holds this file.
- Wave 5 worktrees and branches removed; the agents' handoffs, scripts and gate files stay in `runtime/wave5/`. Baseline `runtime/forced-baseline.tsv` = `runtime/wave5/gate/forced-w5.tsv` (pre-wave copies in `runtime/wave4/gate/` and `runtime/wave5/gate/`).
- GameFiles: private mode, hook DLL with the wave 5 trace hooks (deployed by agent A). The user's launcher points its install dir at `GameFiles/`, so it warns about the `hello-both` / `sample-mod` folders there (harmless).
- KWin Agents desktop rules: on.
