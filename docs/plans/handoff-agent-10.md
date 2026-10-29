# Handoff 10: single working agent (2026-10-27)

Read [handoff-agent-7.md](handoff-agent-7.md) first (preferences, commit rules, CPU limits, method, tools) and [handoff-agent-8.md](handoff-agent-8.md) (tools in `runtime/w7/`, debug switches); [handoff-agent-9.md](handoff-agent-9.md) has the previous state. This file only adds what changed after them. Goal unchanged: every terrain job matching; the user also asked for a fresh 50-seed client batch on every orientation and topology, verified and fixed.

## State
- **Full gate done and merged:** 218,490 of 218,705 client layouts match (from 213,860), 0 lost (`runtime/w9/old.tsv`, Poe2Server `df606a3`). It is now `runtime/forced-baseline.tsv` (previous: `runtime/forced-baseline-pre-w9.tsv`) and merged into `dumps/0.5.5e.hotfix1/terrain/vectors/forced-results.tsv`; matrix 4457 match / 8 partial / 4 fail / 39 untested.

- **Fixes of handoff 9's session and this one** (all gated on the failures set `runtime/w7/fails-in.tsv` and the 15,000-job match sample with 0 lost; RE in `docs/re/terrain/dungeon.md` and `tgr.md`):
  - door-edge walkways, subgraph window ring, map bounds, league roll chance and Incursion requests, PairedRoomLayoutPlugin, repair flag, subgraph slot alignment, post-unique overhang, feature rooms, slot height, room own ground (handoff 9 list);
  - key-less overlay roots and the extra jobs' strip (G4_13 400);
  - UltimatumPlugin's `end_lowlevel` below area level 75 (G3_10 50);
  - fill rooms without a passed height take it over their extent (G1_11 224, MapBloomingField);
  - the `.tgr` height passes `0x1420c0be0` / `0x1420c1b50`, world-unit tile heights for `.tgr` maps, height repair of a raised `.tgr` map's outer buffer (Design, Design_Lite, P2_1);
  - overhanging subgraph tiles keep the subgraph's storey (BossRush_Area3).
- Failures set: 4,630 of the 4,845 baseline failures match now (gate `runtime/w7/both-s4.res`).

## New client batch (`full2`)
- `scripts/re/terrain_batch.py run full2` (master seed 20261026, 50 seeds per area on every topology and orientation): 225,400 jobs, results in `dumps/0.5.5e.hotfix1/terrain/batch/full2/batch-results.tsv`; 6,598 are client errors (the same areas as in the first batch: G2_10_1, MapBackwash, MapUniqueVault, G2_1, G1_12, ...).
- `forced_verify` on it (same build): **218,585 of 218,802 match** (`runtime/w9/full2.tsv`, copied to `dumps/0.5.5e.hotfix1/terrain/vectors/forced-results-full2.tsv`; not in the matrix, which reads `forced-results.tsv` only). Failures: G2_12 50, ChayulaLeague_TowerBoss 50, ChayulaLeague_Tower 50, BossRush_Area1 50, G1_13_2 8, RitualLeagueBoss 5, MapCaldera 4. No new failure class.

## Remaining failures of the old batch (215 jobs)
- **ChayulaLeague_Tower / TowerBoss (100):** the parent's `TowerAbyss_Fill_01` tiles inside the `DownstairsEntrance` subgraph window are at storey 1 (h32) in the client, 0 in ours; every client fill placement there is h0 (`141f90664`), so a later pass raises them. Not the dungeon refix (`refix_fills` in `build_dungeon` made it worse) and not `height_pass` after the dungeon tile pass (no gain). Next: a client trace with height logging of the subgraph's outer fill (`0x141ff7b90`).
- **BossRush_Area1 (49):** placements diverge in the two rightmost zones; the client crashed on it before, no trace.
- **G2_12 (50):** `POE_SM_SKIP=forced_connect` (debug switch in `size_map_final`) matches 44 of 50; the client's rule for leaving `1x1_Carts_forced_connect.arm` out of the size map is not found (not uniqueness, not unique and second `.arm` bool, not "placed uniques leave"). The other 6 (traces in `batch/w10-1`, 2026-10-27): with the skip the totals match (8477), then the placeholder fill `0x1420a62c0` ends one cell earlier in the client (G2_12 392042115: ours still finds cell (13,59) free and draws a 2x2 that does not fit; the client has no free cell left after (7,59) and goes on to the `below(128)` record costs), so the room grid already differs there; the traces do not show the grid.
- **G1_13_2 (7):** the pre-unique room takes a cell the client rejects at the same shuffled index; the cell list order (slot visit order of the tile pass) differs. `0x142097e30` lists cells like ours (y outer, x inner, clamped to the view).
- **RitualLeagueBoss (6):** one extra 2x3 overlay candidate (ours (1,5), client frame (23,11)); something one row off in the occupancy. `runtime/w8/ofitdiff.py` (fixed for a first collection) with offset 22,6.
- MapCaldera (2), MapCrimsonShores (1): older open cases (tgr.md). Traces of 3 G1_13_2, 2 RitualLeagueBoss and 2 MapCaldera jobs of the new batch are in `batch/w10-1/exports`.

## What is left for a full 1:1 port (any seed, orientation and topology the client supports)
Tile layout and doodad hash are 1:1 for what can be tested: both client batches match 99.90 %, every matching job also matches the doodad hash; matrix 4,457 of 4,508 combinations match (8 partial, 4 fail, 39 untested, 10 areas without topologies). What remains:

1. **The remaining failure classes** (about 215 jobs per batch, 7 areas; section above): each needs client traces with more logging, the current traces cannot show the cause.
2. **Combinations no batch can check** (39 untested, plus 6,598 client errors per batch): the client itself errors on some forced combinations (G2_10_1, MapBackwash, MapUniqueVault, MapRockpools, ExpeditionLogBook_Heath, ...); Sanctum_1-4, IncursionTemple(Present) and HideoutPietyArena have no comparable client result. Fix or work around the hook's forced-generation path for them, or capture them another way.
3. **Inputs the batches hold fixed but real play varies** (the biggest gap for "any seed"; the batches force topology and orientation and use each area's default state):
   - Natural topology and orientation choice from the seed alone: the client's world-map match (`0x141fa65c0`) is not ported (the port takes the lowest allowed orientation). Port it and check it against unforced client generations.
   - Area level: G3_10's level 75+ path and the trial's chosen rooms (UltimatumPlugin `0x141f101e0`) are untested.
   - League and session state: league roll outcomes, Incursion and other extra-feature rooms, and the `unique<N>` lettered tags from session history (not ported). Feed in the server's real league state and check against client captures in those states.
   - Smaller unported paths no tested job uses: the height fallback of `0x14209f1d0`, off-map ends of virtual edge splits (`0x1420ba4c0`), transition tiles (`0x141fe0cc0`), de-repetition ground fix-ups (`0x141fdea00`); list in [../status/terrain.md](../status/terrain.md) "Not ported yet".
4. **Stages after the tile layout** (not needed for the client to draw the same map, needed for a server that uses it): the walkability grid (collision) is built but only checked on a few dozen exports (`check_grids`), not at scale (known differences: MapSlash, MapCrimsonShores); entities and spawns are not ported (deferred); server-side generation is still off by default (`[terrain] generate`, the user's decision).

## Entities (area transitions, waypoints, checkpoints, NPCs, chests)
Not doodads: the client places doodads itself from the layout (decorative `.ao` objects; the doodad hash covers them, and the waypoint models in `doodads.tsv` such as `waypoint_DigSIte_Decorative.ao` are only the decoration). Clickable objects are server entities: the client shows a transition only when the server sends it (`AddObject`, S->C `0x2f7`, [../protocol/entities.md](../protocol/entities.md)), even on a matching map. No manual placement is needed: their positions follow from the generated layout. Nothing of this is ported; the server has no in-world transitions (area changes only through the `transfer` command or the web admin, [../guides/server-admin.md](../guides/server-admin.md#transfers)).

Where they come from:
- **Tile objects** (`.tmd`, the quoted object type is the last field of a line): e.g. `metadata/terrain/woods/grimtangle/areatransitions/grimtangletransitiondown_02.tmd`, `woods/lumbermill/encampment_01.tmd`, `jungle/aggorat/features/doryanissanctumentrance_01.tmd`. The generator already reads them for the `Spawner` flag (`generate.rs`); tiles with an `AreaTransition` or `waypoint` `InteractionAction` object get tile data `+0x16f` bit 0 ("sealed", `0x141d05520`, not ported; tgr.md).
- **Room files** (`.arm` object / point-of-interest entries): e.g. `maps/uniquereactor_03/rooms/unique/entrance_01.arm`, `outer_entrance.arm`, `checkpoint_01.arm`.
- **Graph node tags**: `teleport1`, `telepad1`, `entrance1`, `default`, `killzone=N` mark exits, arrival points and spawn zones (G3_10's end node gets `teleport1` only from area level 75).

To do:
1. An entity pass in `poe-world` after the doodads: collect the objects (type, cell, orientation, height) from the placed tiles' `.tmd`, the rooms' `.arm` entries and the node tags. Find the client's pass first (the entity stage after `TerrainGen_Doodads` `0x141ff56d0`; status doc "Verified stages": the calls after `0x1415a5185` belong to the doodad/entity stage) and verify it against DLL exports with `entities.tsv` (a client-hook export to add).
2. Encode them in `AddObject`: the components a transition needs (Positioned, Animated, InteractionAction, Transitionable, the target area). NPC components are partly decoded; transition objects are not: needs an official capture of the user walking through one (Claude never logs in to official servers; captures are the user's).
3. The click: the interaction request handled by sending the player through the existing transfer flow to the target area (`WorldAreas` connections, or the transition object's own target).
4. Then waypoints and checkpoints (the same pass, their own components), chests and monster spawns (`killzone`, `.arm` points of interest; deferred until the rest works).

## Next
1. Traces with more logging (client-hook) for the remaining classes: Chayula (subgraph outer fill heights), G2_12 (the room grid before the placeholder fill), G1_13_2 (tile pass slot visit order: log the ground-cell list), RitualLeagueBoss (overlay occupancy). BossRush_Area1 crashes the client.
2. Port the natural topology and orientation choice (`0x141fa65c0`) and verify it with unforced client generations.
3. Make the client-error and untested combinations checkable.
4. Real area level and league state as inputs, checked against client captures.
5. The walkability grid at scale; then entities (section above), WP7.13, WP7.15 and the `[terrain] generate` default question (handoff 7 section 4.3).
