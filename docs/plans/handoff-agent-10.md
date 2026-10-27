# Handoff 10: single working agent (2026-10-27)

Read [handoff-agent-7.md](handoff-agent-7.md) first (preferences, commit rules, CPU limits, method, tools) and [handoff-agent-8.md](handoff-agent-8.md) (tools in `runtime/w7/`, debug switches); [handoff-agent-9.md](handoff-agent-9.md) has the previous state. This file only adds what changed after them. Goal unchanged: every terrain job matching; the user also asked for a fresh 50-seed client batch on every orientation and topology, verified and fixed.

## State
- **Full gate done and merged:** 218,490 of 218,705 client layouts match (from 213,860), 0 lost (`runtime/w9/old.tsv`, Poe2Server `df606a3`). It is now `runtime/forced-baseline.tsv` (previous: `runtime/forced-baseline-pre-w9.tsv`) and merged into `dumps/0.5.5e/terrain/vectors/forced-results.tsv`; matrix 4457 match / 8 partial / 4 fail / 39 untested.

- **Fixes of handoff 9's session and this one** (all gated on the failures set `runtime/w7/fails-in.tsv` and the 15,000-job match sample with 0 lost; RE in `docs/re/terrain/dungeon.md` and `tgr.md`):
  - door-edge walkways, subgraph window ring, map bounds, league roll chance and Incursion requests, PairedRoomLayoutPlugin, repair flag, subgraph slot alignment, post-unique overhang, feature rooms, slot height, room own ground (handoff 9 list);
  - key-less overlay roots and the extra jobs' strip (G4_13 400);
  - UltimatumPlugin's `end_lowlevel` below area level 75 (G3_10 50);
  - fill rooms without a passed height take it over their extent (G1_11 224, MapBloomingField);
  - the `.tgr` height passes `0x1420c0a60` / `0x1420c19d0`, world-unit tile heights for `.tgr` maps, height repair of a raised `.tgr` map's outer buffer (Design, Design_Lite, P2_1);
  - overhanging subgraph tiles keep the subgraph's storey (BossRush_Area3).
- Failures set: 4,630 of the 4,845 baseline failures match now (gate `runtime/w7/both-s4.res`).

## New client batch (`full2`)
- `scripts/re/terrain_batch.py run full2` (master seed 20261026, 50 seeds per area on every topology and orientation): 225,400 jobs, results in `dumps/0.5.5e/terrain/batch/full2/batch-results.tsv`; 6,598 are client errors (the same areas as in the first batch: G2_10_1, MapBackwash, MapUniqueVault, G2_1, G1_12, ...).
- `forced_verify` on it (same build): **218,585 of 218,802 match** (`runtime/w9/full2.tsv`, copied to `dumps/0.5.5e/terrain/vectors/forced-results-full2.tsv`; not in the matrix, which reads `forced-results.tsv` only). Failures: G2_12 50, ChayulaLeague_TowerBoss 50, ChayulaLeague_Tower 50, BossRush_Area1 50, G1_13_2 8, RitualLeagueBoss 5, MapCaldera 4. No new failure class.

## Remaining failures of the old batch (215 jobs)
- **ChayulaLeague_Tower / TowerBoss (100):** the parent's `TowerAbyss_Fill_01` tiles inside the `DownstairsEntrance` subgraph window are at storey 1 (h32) in the client, 0 in ours; every client fill placement there is h0 (`141f90664`), so a later pass raises them. Not the dungeon refix (`refix_fills` in `build_dungeon` made it worse) and not `height_pass` after the dungeon tile pass (no gain). Next: a client trace with height logging of the subgraph's outer fill (`0x141ff7a10`).
- **BossRush_Area1 (49):** placements diverge in the two rightmost zones; the client crashed on it before, no trace.
- **G2_12 (50):** `POE_SM_SKIP=forced_connect` (debug switch in `size_map_final`) matches 44 of 50; the client's rule for leaving `1x1_Carts_forced_connect.arm` out of the size map is not found (not uniqueness, not unique and second `.arm` bool, not "placed uniques leave"). The other 6 (traces in `batch/w10-1`, 2026-10-27): with the skip the totals match (8477), then the placeholder fill `0x1420a6140` ends one cell earlier in the client (G2_12 392042115: ours still finds cell (13,59) free and draws a 2x2 that does not fit; the client has no free cell left after (7,59) and goes on to the `below(128)` record costs), so the room grid already differs there; the traces do not show the grid.
- **G1_13_2 (7):** the pre-unique room takes a cell the client rejects at the same shuffled index; the cell list order (slot visit order of the tile pass) differs. `0x142097cb0` lists cells like ours (y outer, x inner, clamped to the view).
- **RitualLeagueBoss (6):** one extra 2x3 overlay candidate (ours (1,5), client frame (23,11)); something one row off in the occupancy. `runtime/w8/ofitdiff.py` (fixed for a first collection) with offset 22,6.
- MapCaldera (2), MapCrimsonShores (1): older open cases (tgr.md). Traces of 3 G1_13_2, 2 RitualLeagueBoss and 2 MapCaldera jobs of the new batch are in `batch/w10-1/exports`.

## Next
1. Traces with more logging (client-hook) for the remaining classes: Chayula (subgraph outer fill heights), G2_12 (the room grid before the placeholder fill), G1_13_2 (tile pass slot visit order: log the ground-cell list), RitualLeagueBoss (overlay occupancy). BossRush_Area1 crashes the client.
2. Then WP7.13, WP7.15 and the `[terrain] generate` default question (handoff 7 section 4.3).
