# Handoff 7: for a single working agent (2026-10-25)

You are **one agent doing the work yourself**, not an orchestrator: do not start subagents unless the user asks for them. This file replaces the orchestrator handoffs as the start point; it is built from [handoff-orchestrator-3.md](handoff-orchestrator-3.md), [-4](handoff-orchestrator-4.md), [-5](handoff-orchestrator-5.md), [-6](handoff-orchestrator-6.md) and the wave 4-6 agent handoffs in `runtime/wave{4,5,6}/{a,b,c}/handoff.md`. Read those only when you need a detail; everything you need to start is here.

Read first: [CLAUDE.md](../../CLAUDE.md) (hard rules), [docs/overview.md](../overview.md) (which repo does what), your MEMORY.md. The user may now use **T3 Code** (a GUI over the Claude Code CLI) instead of the VS Code extension; CLAUDE.md, memory and the CLI's MCP config are the same.

## 1. The project in one paragraph
A Rust private server for Path of Exile 2 client 0.5.5e. The hub (`/home/distorbed/Documents/Poe2ModdingProject`) holds docs, plans, RE notes, scripts, `xtask` and the tracker; the code lives in submodules under `modules/`: server (Poe2Server), protocol (Poe2Protocol), client-hook (Poe2ClientHook, the DLLs), web-admin, launcher (Tauri), mod-sdk, mod-index, server-list. All public under `github.com/Distorbed678`. Releases: launcher **v0.4.1** (2026-10-24, AppImage environment fix for Proton), hook `hook-v0.3.0`, server `server-v0.2.1` (Poe2Server), SDK v0.1.0.

## 2. User preferences (keep them)
- **Usage is limited.** Keep status messages short. When the user says usage is low, stop at a safe point: commit (WIP allowed, marked NOT GATED), stop your processes, write a handoff file. They prefer starting a fresh agent from a handoff over resuming an old context.
- Ask (AskUserQuestion, recommended option first) before larger changes, releases, or anything outward-facing. Report when a step finishes, at a gate, or on a failure.
- **CPU limits** (the user's editor crashed at 100% CPU): run builds, gates and the client batch with `nice -n 19`, `CARGO_BUILD_JOBS=4`; gates at most 8 threads when you are the only heavy job (4 otherwise). Processes survive an editor restart: check `ps` before rerunning something.
- Steam may be started for private tests; never log in to official servers; official captures are the user's (`deploy-client --official` before, back to private after).
- No Spectacle screenshots, never open the user's browser (headless browsers for visual checks). The game client goes to the KWin "Agents" desktop: `scripts/agent-desktop.sh on` (it disappears after a reboot; run `status` first). The user may be playing another game (a Path of Exile 1 client); never touch processes you did not start, never `pkill -f`, never wait with `while pgrep -f` (it matches itself).
- Items and entities are not implemented: nothing that depends on them (remote equipment visuals, dynamic obstacles, party/trade replies) until they exist.
- The small open items ([open-items.md](open-items.md): mod overlays, web admin class names, ban commands, mod download rate limit, launcher nested hook pin, movement #11/#12 tracker rows) were postponed by the user ("later"). The user's gates still open: G-PARTY capture (official), Windows tests. Ask before starting any of them.
- `.vscode/settings.json` in the hub is local and uncommitted (keeps VS Code's indexer out of `dumps/` and `runtime/`); the user has not decided whether to commit it.

## 3. How to commit (unchanged rules)
- Commit and push after each working step; submodules first (inside the submodule, push), then the pointer in the hub; nested submodules (Poe2Server's `modules/{client-hook,protocol,web-admin}`) bumped together. Commit messages end with the Co-Authored-By line the session gives you.
- Checks: hub `cargo xtask tracker --check`, `cargo xtask versions --check`, `python3 scripts/check-links.py` (after any doc change); server `cargo test --workspace` in `modules/server`; client-hook `cargo fmt --all --check` (its CI checks fmt), `cargo xtask build-dll`; launcher `npm run check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. **Do not run `cargo fmt` in Poe2Server** (the repo is not fmt-clean and its CI does not check it). Watch CI with `gh run list -R Distorbed678/<repo> -L 1`.
- Docs in the same commit as the code; RE findings and addresses in `docs/re/`.

## 4. Terrain 1:1 port: the main task
Goal ([terrain-all-areas.md](terrain-all-areas.md)): the server's generator (crate `poe-world` in Poe2Server) reproduces the client's layout exactly (tile hash, doodad hash, collision grid) for every topology x orientation combination of the default league mode (4508 combinations), checked against a client batch of 218,705 layouts (50 random seeds each, `dumps/0.5.5e/terrain/batch/full/batch-results.tsv`).

**Pushed state:** **196,094 of 218,705** match (Poe2Server `b021b44`, after wave 5; 108,545 before wave 4, 176,336 after it); doodad hash equal on all matches but one; matrix 3286 match / 877 partial / 306 fail / 39 untested ([../status/terrain-matrix.tsv](../status/terrain-matrix.tsv), history [../status/terrain.md](../status/terrain.md)). Collision grid equal to the client's for 273 of 303 exports; `[terrain] generated_grid` (default off) uses it only when the tile hash matches.

**Baseline** for the regression gate: `runtime/forced-baseline.tsv` (`area seed topology(1-based) orientation match|fail detail`).

### 4.1 Wave 6: three unmerged branches (do this first)
Wave 6 ran with three agents and was stopped; nothing of it is merged. Each branch is a Poe2Server worktree, local only (not pushed):

| Worktree | Branch | Commits on top of `b021b44` | Gate status |
|---|---|---|---|
| `runtime/wt/a` | `terrain6-a` | `afa196a` `.dgr` node slots by edge index (`0x141d1c580`; also fixes the merged-edge rule `0x141fec1c0`): G1_13_1 3,777 -> 6,400/6,400 | gated (match-sample 0 lost) |
| | | `f31b64c` post-unique tiles for `.dgr` regions (`0x141ff2090`), edgeless typed nodes accessible (a rule from traces): G1_13_2 0 -> 1,835/1,842 | **not fully gated** (its gate `runtime/wave6/a/gate.sh runtime/wave6/a/fv-bin1 runtime/wave6/a/gate-m-in.tsv runtime/wave6/a/gate1.tsv` was killed at 20,000 of 66,129). If it loses jobs, drop only the edgeless-node rule (`node_accessible`). |
| `runtime/wt/b` | `terrain6-b` | `5e12a8e` oversized subgraphs keep overhanging cells, node tile choice without overlap test, nested subgraphs depth first: G3_1 2,538 -> 2,912/3,099 | gated on match-sample (binary before later edits) |
| | | `31c6bd1` key ground with `.gt` flag 0x10 read as none: MapSinkhole 96/100 | gated on match-sample |
| | | `bce3cb1` WIP: world-map `ForceOuterGT` ring, G4_13 Kalguur extra job (202x94 as in the client, not matching yet), overlay limit-skip behind env `POE_OVERLAY_LIMIT_SKIP` (off), `POE_OWN_TAG` experiment (off) | **not gated** |
| `runtime/wt/c` | `terrain6-c` | `34bd41a` league mission rooms: exact-fit candidates sorted by placed-room index (`0x1420cf1d0`): +807 stone-circle jobs | gated on the 3,716 stone-circle jobs only |
| | | `df63016` WIP: mission rooms count as overlay feature rooms, league feature list shared by all graphs of a job (`Files::take_league`), league tiles read before the layout | **not fully gated** (stone-circle jobs: 3,418 of 3,716 match, 0 lost; match-sample, `cargo test` and a full gate not run) |

The wave 6 docs sections are committed in the hub (`docs/re/terrain/dungeon.md` "Wave 6 (agent A)" and "(agent B)", `docs/re/terrain/tgr.md` "Wave 6 (agent C)"); they describe this unmerged code.

**Merge procedure** (worked for waves 4 and 5):
1. Check each branch: `git -C runtime/wt/<x> diff --stat b021b44 HEAD -- modules` must be empty (agents twice committed the worktree's `modules/*` symlinks; a pre-commit hook in `.git/modules/modules/server/hooks/pre-commit` now refuses `modules/` changes on `terrain*` branches).
2. `git -C modules/server worktree add ../../runtime/wt/m -b terrain6-merge main`; in it replace `modules/{client-hook,protocol,web-admin}` with symlinks to `modules/server/modules/*` (rmdir, `ln -s`); merge `terrain6-a`, `terrain6-b`, `terrain6-c`. Conflicts so far were always two agents extending the same function: keep both sides.
3. Build (`CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=runtime/wt/a-target nice -n 19 cargo build --release --examples -p poe-world` in `runtime/wt/m`), `cargo test --workspace`.
4. **One full gate** on the merged code: `POE_FORCED_BUDGET=60 nice -n 19 <target>/release/examples/forced_verify dumps/0.5.5e/terrain/batch/full/batch-results.tsv --out runtime/wave6/gate/forced-w6.tsv --threads 8` (1.5-2 h; copy the binary first so later builds do not replace it). Compare with the baseline on (area, seed, topology, orientation): no `match` may become a fail. If some do, find the commit by running only the lost jobs on each branch's binary, then drop or fix that commit (likely suspects: `f31b64c`'s edgeless-node rule, `bce3cb1`, `df63016`).
5. Then: `git -C modules/server merge --ff-only terrain6-merge`, commit with the result in the message, push; merge `forced-w6.tsv` into `dumps/0.5.5e/terrain/vectors/forced-results.tsv` (by key), make it the new `runtime/forced-baseline.tsv` (keep a copy of the old one), `cargo xtask terrain-matrix && cargo xtask tracker`, update the matrix row in `docs/status/tracker.toml` and add a wave entry in `docs/status/terrain.md`, commit and push the hub (bump `modules/server`); remove the worktrees (`git -C modules/server worktree remove --force runtime/wt/<x>`, delete the symlinks first) and branches.

### 4.2 Method and tools
- **Gate tool** `forced_verify` (poe-world example) replays client jobs; input = rows of `batch-results.tsv` with its header. Small inputs for iteration: grep the batch file by area. Match sample for quick regression checks: `runtime/wave6/match-sample-in.tsv` (15,000 currently matching jobs). Compare scripts: `runtime/wave4/c/cmp.py <out.tsv>`, `runtime/wave5/b/regress.py`.
- **Finding a divergence:** `area_trace -- <area> <seed> <export dir> [topology[:orientation]]` (first diverging RNG draw vs the client's generator trace; `POE_SHOW_OURS=1`), `layout_diff`, `lay_cmp`, `tgr_diff`, `check_doodads`, `check_grids`, plus the scripts in `runtime/wave{5,6}/<x>/` (`latcmp.py`, `ofit.py`, `tr.sh`, `run1.sh`, `mkin.py`, `blocks.py`). Debug env switches: listed in `docs/re/terrain/dungeon.md` and `tgr.md`.
- **Client exports and traces** (only one process may drive the game client at a time): `scripts/re/terrain_batch.py` with the hook's `gen_batch` (`docs/re/terrain/batch.md`); `POE_HOOK_GEN_TRACE=1` and one thread for traces; `runtime/wave6/a/runclient.sh <run>` resumes a run; stop with `python3 scripts/re/terrain_batch.py stop`. Runs are in `dumps/0.5.5e/terrain/batch/<run>/` (w4-*, w5-*, w6-*; `full/` is the 50-seed batch). The client takes ~2 min to start. `GameFiles/` has the debug-core hook with `gen_batch` and the trace hooks (deployed by `cargo xtask deploy-client`). x64dbg is allowed on the private client (ask the user first).
- **Ghidra:** the user must open Ghidra for the Ghidra MCP; load its tools with ToolSearch (`select:mcp__ghidra__decompile_function,mcp__ghidra__get_xrefs_to,mcp__ghidra__disassemble_function`); fallback `scripts/re/disasm.py`. RE how-to: `docs/re/README.md`.
- Port the client's logic with addresses in the docs; never special-case an area.

### 4.3 What fails (22,611 jobs at the baseline; wave 6 fixes a part of it)
- `.dgr` 22,419 (20,473 wrong output), `.tgr` 92, towns 100. Biggest areas: G1_13_1 2,623 and G1_13_2 1,842 (fixed by wave 6 A once merged), G2_4_1 1,600, MapFrozenFalls 1,548, G4_1_1 981, G2_4_3 777, G2_4_2 709, MapUberBoss_JadeCitadel 600, G3_1 561, G3_6_1 441, G4_13 / MapSunTemple / MapKarst / MapExcavation 400 each.
- **Leads, by expected value** (details in the wave 6 handoffs and docs sections):
  1. **Unclassified wrong output in A's 96 areas:** finish the trace run `w6-a-1` (144 jobs, 18 done: `nohup runtime/wave6/a/runclient.sh w6-a-1 &`), then `runtime/wave5/a/sum.sh dumps/0.5.5e/terrain/batch/w6-a-1/exports runtime/wave6/a/sum-a1` to class G2_4_1, MapFrozenFalls, G4_1_1, G2_4_3, G2_4_2, JadeCitadel, G3_6_1 by first divergence.
  2. **World maps (359 jobs):** tagged blank rooms (`Location_Label_02`, `Compass_02`) must own their ground in `dungeon_tiles.rs` `settle_rooms`; `POE_OWN_TAG=1` makes all 898 world-map jobs match but loses 95 match-sample jobs (G3_1, Delirium_HungerBoss, HideoutRemidusMonastery_): find the condition that separates them.
  3. **Sub-dungeon fill ring** (`0x141f900a7`, MapFrozenFalls, G3_14, G3_8): the window must share occupancy with the parent (a naive skip fails).
  4. **Door-pick candidate count** `0x141fa81a3` (G2_4_2, G3_11, G3_16, G3_6_1, G1_12); `0x141fe75b7` (G2_4_3, MapAzmerianRanges, MapFortress).
  5. **G3_1 topology 2, orientations 2-5 (187):** the oversized `End_S_E_1` subgraph is pasted 3 cells too far right; shift by (slot - tgr size) on the side the region orientation aligns to (exports in `w6-b-2`).
  6. **G4_13 (400):** a client overlay pass with a 6x6 single candidate the port lacks; the right overlay limit-skip rule (the env version loses G2_1/MapEpitaph/MapBloomingField).
  7. **Height pass `0x1420c0a60`** (P2_1 39, Design 50): fix-up flood, subtract the minimum tile height, height flag 2 on every corner; decompiles in `runtime/wave6/c/re/`.
  8. **MapDerelictMansion (59) + CopperCitadel (4):** the `0x1420abb90` branch for a feature `.arm` whose side equals RoomUnitSize (build a `DungeonRoom`, run `place_special`, then `take_league`).
  9. **MapKarst (400):** PairedRoomLayoutPlugin `0x141f01710`, FileGroups "Pairs" (trace in `w6-b-1`). G4_4_2 trace there too.
  10. HideoutLimestone (+ Claimable, 100): the client skips the levelling rounds `0x141fd71c0`; the gating generator byte `+8` is never written in the decompiles searched; needs a client hook trace. G3_town, MapCaldera (2), MapCrimsonShores 921210055, BossRush_Area1/3, MapPenitentiary 61, "could not fix heights" in G2_12/Design_Lite/G4_4_3 (150).
  11. **Collision grid:** `POE_GRID2` (second unwalkable-corner pass `0x141fe0fd0`) is wired but untested: `check_grids` with `runtime/wave5/c/grid-only-known.tsv` and the exports in `batch/w4-c-1`.
- **After the failures:** a fresh client batch with new seeds (1.3 h client time, so the port is not tuned to one seed set); **WP7.13** (the instance server sends `map_force_topology` / `map_force_orientation` equal to its generator's pick, keep a `natural` override); **WP7.15** (every combination match or a documented reason), then ask the user whether `[terrain] generate` and `generated_grid` default to on (ends the terrain upload); a live in-game test of `generated_grid`; then offer a server release (`server-v0.2.x`, bump the Poe2Server workspace version, tag, watch the workflow, bump the hub). Out of scope: the 10 fixed areas, the 6 world-map panels (no client hash), Incursion temples, league mechanic effects.

## 5. Start-up checklist
1. `gh auth status`; `git status` and `git submodule status --recursive` in the hub; CI of the repos (`gh run list -R Distorbed678/<repo> -L 1`); `cargo xtask versions --check`, `cargo xtask tracker --check`, `python3 scripts/check-links.py`.
2. `ps -eo pid,etime,args | grep -E "forced_verify|terrain_batch|PathOfExileSteam"`: nothing of ours should run.
3. `git -C modules/server worktree list`: `runtime/wt/{a,b,c}` with the wave 6 branches (section 4.1).
4. Ask the user what to do first; the default proposal is: finish wave 6 (gate the open commits, merge, full gate, push), then the leads in 4.3.
