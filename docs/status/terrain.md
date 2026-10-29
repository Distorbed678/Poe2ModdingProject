# Terrain generator: status and next steps

Last updated 2026-10-27 (single-agent fixes of handoffs 9 and 10, second 50-seed client batch; single-agent fixes after wave 6 2026-10-26; G1_12 and bulk verification tools 2026-10-22, matrix 2026-10-21, results consolidated 2026-10-20). This is the only place
for terrain status and the resume point for the server-side terrain generator (`modules/server/crates/poe-world`).
How the client's generator works, and the tools used below, are in
[../re/terrain/](../re/terrain/README.md): [README](../re/terrain/README.md#tools) (tools),
[tgr.md](../re/terrain/tgr.md), [dungeon.md](../re/terrain/dungeon.md),
[shared-passes.md](../re/terrain/shared-passes.md).
The plan for the remaining areas (scope matrix, verification at scale, blockers, work packages) is
[../plans/terrain-all-areas.md](../plans/terrain-all-areas.md).
The per-area coverage (every area x topology x orientation: match, fail or untested, plus the generator probe) is the generated table [terrain-matrix.tsv](terrain-matrix.tsv) (`cargo xtask terrain-matrix`, [../guides/setup.md](../guides/setup.md#run)); its totals are in the README tracker. The play-log vectors of every area now feed it too (see "Bulk vectors" below); the planned target is 50 random seeds per combination from the client batch (WP7.16).

## Goal

All act 1 areas generate server-side exactly as the client does (same tile hash for the same
area and seed), so the server can later do collision, pathing and spawns. Area transfer (dev
command and overlay) is done ([../guides/server-admin.md](../guides/server-admin.md)); collision comes later.
Server-side generation is off by default (`[terrain] generate`); `!terrain` runs it for the
current instance.

The client builds its own terrain from the area and seed the server sends, so it loads any area
regardless of our generator. The server's copy is what has to match.

## Current results

State 2026-10-28 (Poe2Server `af1e7e9`; the history is in the dated entries below):
- **Complete (2026-10-29, roadmap R1, Poe2Server `3b05682`):** all three 50-seed client batches
  match on every layout the client generates (tile hash, and the doodad hash on every match):
  batch 1 218,705 of 218,705, batch 2 (`full2`) 218,802 of 218,802, batch 3 (new seeds) 218,678 of
  218,678; matrix 4469 match / 0 partial / 0 fail / 39 untested / 10 n/a
  ([terrain-matrix.tsv](terrain-matrix.tsv)). Vectors: `dumps/0.5.5e/terrain/vectors/
  forced-results{,-full2,-batch3}.tsv`.
- **Natural choice** (no forced layout): ported; 4,173 of 4,173 (`w13-nat1`) and 8,368 of 8,368 (batch 3) unforced client generations match
  (batch `w13-nat1`, new seeds), and the instance server forces that choice
  (`[terrain] force_layout`, WP7.13).

### The rows that do not match (WP7.15, 2026-10-28)
- **partial:** none since 2026-10-29 (MapCaldera and MapCrimsonShores fixed, [../re/terrain/tgr.md](../re/terrain/tgr.md) "R1").
- **untested (39):** the client itself fails on every seed of these combinations, so there is no
  layout to compare (errors from `dumps/0.5.5e/terrain/batch/full/batch-results.tsv`):
  `ExpeditionLogBook_Heath` 1:1/3/5/7 ("Non-feature node overlapping another"), `G2_10_1` 5:1/3/5/7
  and 7:1/3/5/7 ("Failed to connect edge"), `HideoutPietyArena` 1:0 and `MapUniqueVault` 1:1/3/5/7,
  2:1/3/5/7 ("No possible rooms found for room"), `IncursionTemple` / `IncursionTemplePresent` 1:0
  ("No walkable area in the level"; Incursion temples are built by their league mechanic, out of
  scope), `MapBackwash` 1:0..7 ("Failed to tilise at tile"), `MapRockpools` 1:0..3 ("While
  tilising .../bossman.arm"), `Sanctum_1`..`Sanctum_4` 1:0 (`BufferUnderflow` reading their graph
  in the headless batch). The natural choice never picks them in the client either (`w13-nat1`
  has the same errors for these areas).
- **n/a (10):** areas without `Topologies` rows (`NULL`, `CharacterSelect`, `PersonalHideout`,
  `GuildHideout__`, `CurrentTown`, `G2_3s`, `G2_8a`, `MapLeaguePortal`, `EndgameDoodad_MeteorCrater`,
  `EndgameDoodad_BeastGroundZero`); the 6 world-map panels have no client hash.

## Verified stages

Each stage of the Riverbank (`G1_1`, seed 1000) and Clearfell ports was compared with the client's
generator trace; the examples are listed in [../re/terrain/README.md](../re/terrain/README.md#tools).
- Clearfell Encampment seed 1: `town_check` reproduces all 71 room-pass placements (tile and
  orientation, RNG in step: 48 draws).
- Riverbank: `river_gen` matches the first 110 draws (topology, 19 edge rolls, 30 jitters); places all 12
  room nodes where the client does (204 draws, through the room passes); routes all 19 edges exactly
  as the client (5022 draws in step); the whole graph after smoothing equals the client's; the graph
  before tile placement equals the client's (5055 draws); all 103 edge tiles (17 paths) equal the
  client's (5261 draws); the lattice equals the client's at every step up to the fill (4891 points);
  the fill equals the client's (561 pieces, 13 fill rooms; 6393 draws); de-repetition equals the
  client's (6412 draws).
- Riverbank (`G1_1`) seed 1000 generates identically to the client (2026-10-15): tile hash
  138293894, all 5382 cells (tile, orientation, sub-tile) equal, 6414 draws in step. The draws
  after that (`0x1415a5005`, …) belong to the doodad/entity stage. With node tiles and inheriting
  tiles, `area::generate` also matches seeds 1001 and 1002 (`intro_4`, current-build sweep vectors
  in `dumps/0.5.5e/terrain/vectors/sweep-results.tsv`).
- `G1_2` seed 1001 matches through the fill (5722 draws, 87 edge tiles).
- Fill-room, overlay and lattice work for these areas is in
  [../re/terrain/shared-passes.md](../re/terrain/shared-passes.md).

## G1_12 seed 1001 (done 2026-10-22)

- Forced topology `2:7`, `CanopyE_S.dgr`. The main graph is two `graph` nodes (3×3 units of 29
  tiles each): `_CanopyE_S.dgr` (`canopy_master.tsi`) and `Trees_Subscene.dgr`
  (`subscene_master.tsi`).
- The map was 169×85 against the client's 174×87 because the grid overlap came from the first
  room's `.arm` dimension (1) instead of the master's `RoomOverlap` (0 for G1_12, unit 29).
  Master +0x14b is that key (default 1; parser `0x1429285e0`, defaults in `0x14292e190`, see
  [../re/terrain/dungeon.md](../re/terrain/dungeon.md)). `area.rs` now reads `RoomOverlap` from
  the `.tsi` and passes it to `dungeon::layout`.
- Result: `area_trace G1_12 1001 dumps/0.5.5e/terrain/traces/act1_traces/G1_12_1001 2:7` gives
  174×87, all 15138 cells equal the client's layout, tile hash 3369045138 (the client's), 2385
  draws in step; `area_vectors` finds it as `match (forced 2:7)`. The 25 earlier vector matches
  are unchanged (26 of 33 now), and the probe is unchanged (701 of 1141 topologies at seed 1001).
- The subgraph region sits inside its master's `OuterBufferSize` margins (10 units × 6 + 1 = 61
  tiles + 13 per side = 87), and each subgraph's outer fill and margin strips run on its block
  (`outer_buffer_window`, `TileMap::window`); this was already in place and is now confirmed by
  the equal layout.

## Verification at scale (2026-10-22, WP7.2 and the port side of WP7.16)

**Tools** (all in `modules/server/crates/poe-world/examples/`, usage in each header and in
[../re/terrain/README.md](../re/terrain/README.md#tools)):
- `area_vectors` runs on threads (`--threads N`), takes a build filter (`all`, `N`, `>=N`),
  `--natural`, `POE_VECTOR_BUDGET=<seconds>` per (area, seed) (`skipped (budget)` when out),
  and orders the forced search with `--hints <earlier results>` and `--probe <probe results>
  [--runnable-only]`. Vectors with the same area and seed share one search. Every result line now
  ends with `build=N` so the matrix can tell matches of older client builds from current ones.
- `area_probe` runs on threads with `--seeds a,b,c`, `--orientations first|all|N`; one TSV line
  per attempt, in job order, with the seed as the last column.
- `forced_verify <batch-results.tsv> [--out forced-results.tsv]` replays the client batch of
  WP7.16 (columns `area seed topology orientation status tile_hash doodad_hash width height ms
  detail`) with the same forced choice and writes `forced-results.tsv`. Tested on a hand-made
  batch from known vectors (matches, a wrong hash, a wrong map size, a generator error, a client
  status that is not `ok`, a duplicate row); the client's real batch file does not exist yet.
  The doodad hash is not compared until the port has a doodad pass: matches say `doodads not
  ported`.
- Speed: the bundle reader behind `GameFiles` decompresses under one lock, which made parallel
  generation no faster than serial (an attempt took 14 s on 16 threads). `generate::CachedFiles`
  caches the files a run reads (`area::generate_with_files`); the tools share one, after which
  an attempt takes tens of milliseconds. The probe of 13,524 attempts takes 15 minutes
  (the first 4 minutes fill the cache), the bulk vector run 7 minutes.
- The matrix (`cargo xtask terrain-matrix`) now keeps the seeds per combination: `seeds`
  (matched/tested), status `match` (all tested seeds matched), `partial`, `fail` (forced results
  only: a vector that does not match gives no combination), `untested`; `old` counts matched
  seeds that only an older build's vector vouches for. The probe cell covers every probed seed
  and orientation (`ok 6/6`, `error (4/6 ok): ...`).

**Bulk vectors.** `terrain_vectors.tsv` (play-log) plus `sweep-vectors.tsv`, 1973 distinct
vectors of 260 areas, `area_vectors ... all --hints ... --probe probe-results.tsv
--runnable-only` (budget 180 s, no vector skipped): 365 match (261 natural, 104 forced) in 59
areas, 983 differ, 625 error. Beyond act 1: the town hubs (`G2_town`, `G4_town`, `P1_Town`,
`P2_Town`, `P3_Town`, `G_Endgame_Town`, ...), `G2_13` (29 vectors), `Abyss_Hub` (55), `G3_2_2`,
`P2_5`, and one or more vectors of 18 `Map*` areas. 107 of the 128 matched combinations are
vouched for only by older builds' vectors (the hash is of the generated output, so the match is
valid, but a different build might behave differently on mismatches). `--runnable-only` skips
topologies whose probe failed at every attempt, so a vector that matches only through a topology
that failed at the probed seeds is not found; rerun without it for a specific area.

**Probe** (seeds 1001, 1002, 1003, every allowed orientation, 4508 combinations = 13,524
attempts, 30 s timeout): 8147 ok, 5377 error, no panic or timeout. 745 of 1141 topologies run in
at least one attempt, 588 in all of theirs (254 of 432 areas run in all). The comparable run
(seed 1001, first orientation) gives the same 701 of 1141 as before and no topology went from
`ok` to `error`.

**Matrix (2026-10-22):** 442 areas (10 without topologies), 1141 topologies, 4508 combinations:
128 match (283 of 283 tested seeds), 0 partial, 0 fail, 4380 untested (act 1: 76, act 2: 10,
act 3: 3, act 4: 3, act 6: 5, act 10: 31).

## 50-seed client batch (2026-10-22, WP7.16)

The acceptance data of the 1:1 port: `scripts/re/terrain_batch.py plan full --seeds 50` and
`run full` made the client generate every combination of the matrix (4508) with 50 random seeds,
225,400 jobs, in 1.3 h of one client launch (8 threads, 48 layouts/s, mean 170 ms per layout, 0
crashes; how: [../re/terrain/batch.md](../re/terrain/batch.md)). The client itself threw a
generator exception for 6,695 jobs (100 areas, most in `G2_10_1`, `MapBackwash`, `G2_1`,
`MapUniqueVault`, `G1_12`; `IncursionTemple*` needs session data). `forced_verify` ran the port on
the other 218,705 and its result is the matrix input `dumps/0.5.5e/terrain/vectors/forced-results.tsv`.

- **37,380 of 218,705 layouts match (17 %)**; combinations: 501 match on every seed, 508 partial,
  3460 fail, 39 untested (all seeds a client error); areas: 85 match on every job, 44 some, 297 none.
- **Wrong output, no error: 91,453 jobs** (dgr 78,285, tgr 9,429, town 3,739), the largest class,
  invisible to the probe. Top areas `G2_5_1`, `MapCrypt`, `G3_6_1`, `MapHeadland`,
  `MapSteamingSprings`, `G2_9_1`, `MapWoodland`, `MapLostTowers`. Areas that match on most seeds
  and differ on others (seed-dependent bugs in ported code): `G1_9` 3713/4783, `G1_7` 4287/4562,
  `G1_12` 2864/4490, `G1_6` 1485/1600, `G1_4` 938/2201, `G2_10_1` 703/2592.
- **Map size differs: 4,661.**
- **Port errors** (unported systems, jobs): 3x3 door spots 12,957; dungeon edge types with
  chirality, all `.et` together about 25,000 (`UnwalkableLedge`, `woodscliff_height`, `woods_ledge`,
  `vaal_water`, `dune_to_ocean_virt`, `cavern_ledge`, `courtyard_outerwall`, ...); overlay phase 2
  4,429; blank special rooms 3,600; fill orientation masks 3,198; tgr routing retries 2,644; node
  tiles 4,204; no RoomSet 2,100; no border tile 1,714; no root key 1,571; failed to connect 1,411;
  `//` path 1,781; room overlap 1,004.
- **Wave 2 (2026-10-22, agents A, B, C, merged into Poe2Server `612667e`):** 72,718 of 218,705
  client layouts match (from 37,380), 0 lost on the full replay; combinations 747 match, 980
  partial, 2742 fail. Fixes: league mechanic rolls before the topology draw (`0x141ffcad0`, every
  map area), `.dgr` node offset ranges and axis flags turned with the graph, junction explode with
  room exits, room-only `.gt` flags, post-unique tile pass, overlay phase 2 and SizeOrder, routing
  retries, off-map ends of virtual dead ends, fill-room tile height, `RoomOverlap`, `//` paths,
  `EnvironmentOverride`, container graphs, non-key room roots. Biggest gains: MapCrimsonShores,
  MapWoodland, MapHeadland, G1_12, MapSump, MapBloodwood, MapWetlands, MapMire, MapDeforestation.
  Open items per class and the client traces taken for them (`batch/b1`, `b2`, `b3`, `c1`, `a1`,
  `a2`): [../re/terrain/dungeon.md](../re/terrain/dungeon.md), [../re/terrain/tgr.md](../re/terrain/tgr.md).
- **Wave 3 (2026-10-22/23, agents D, E, F, merged into Poe2Server `2167c89`):** 108,545 of
  218,705 client layouts match (from 72,718), 0 lost; 108,518 of them also match the doodad hash
  (27 differ). Combinations 1074 match, 1658 partial, 1737 fail. Dungeon fixes (agent D):
  overlays inside the `.dgr` tile pass, per-room lattice settle `0x142099e90` with the graph
  grounds' `.gt` flags and the uniform-walk rule, pass-context grounds, default edge type,
  overlay exclusions and MatchAllTileKeys; chirality guard removed, blank special rooms, overlay
  room height `0x141fa10b0` (agent E). Biggest gains: G2_5_1, MapCrypt, MapPit, MapWillow,
  MapSandspit, G2_9_1, G2_4_2, Kalguur, MapRiverside. Findings and open items:
  [../re/terrain/dungeon.md](../re/terrain/dungeon.md).
- **R1 complete, 2026-10-29 (Poe2Server `3b05682`, three full replays):** batch 3 (new seeds)
  found one more failure (P1_4: the pre-unique room is an overlay feature room on `.dgr` maps);
  after the fix every layout of the three batches matches (218,705 + 218,802 + 218,678) and the
  natural part 8,368 of 8,368. The generator port is complete for every combination the client can
  generate.
- **R1, 2026-10-29 (Poe2Server `33981f0`, two full replays):** batch 1 218,705 of 218,705 and
  batch 2 218,802 of 218,802 match (from 218,702 and 218,795 expected), 0 lost; matrix 4469 / 0 /
  0 / 39. Fixed: MapCaldera (league rooms are overlay feature rooms on `.tgr` maps) and
  MapCrimsonShores (junction explode drops points within twice its step); details
  [../re/terrain/tgr.md](../re/terrain/tgr.md) "R1". Baseline `runtime/forced-baseline.tsv` (previous
  `runtime/forced-baseline-pre-r1.tsv`).
- **Single agent, 2026-10-28 (handoff 13, Poe2Server `0da535f`, `7654c1a`): natural topology
  choice.** The world-map pins (102 of 102, acts 1, 2, 3, 4, 6) and the client's direction
  layout and three-pass match are ported; without a forced layout the generator now picks what
  the client picks: 4,173 of 4,173 unforced client generations (batch `w13-nat1`, every non-fixed
  area × 10 seeds) and a live `G1_2` load. Forced results unchanged (the forced path is not
  touched). Details: [../re/terrain/shared-passes.md](../re/terrain/shared-passes.md). WP7.13 done
  the same day (Poe2Server `af1e7e9`): the instance server forces that choice in AreaInfo
  (`[terrain] force_layout`, default on); live `transfer G1_6` / `G1_8` match the server's
  generation.
- **Single agent, 2026-10-28 (handoff 12, Poe2Server `bf9ed79`, one full replay):** 218,702 of
  218,705 client layouts match (from 218,653), 0 lost; combinations 4466 match, 3 partial, 0
  fail, 39 untested. Fixed: BossRush_Area1, all 49 (nested subgraphs queue behind the subgraphs
  already waiting instead of running right after their parent). In the second batch (`full2`)
  213 of its 217 failures match now (BossRush_Area1 50 fixed too; full replay of that batch not
  rerun). Open: MapCaldera (2, and 4 in `full2`), MapCrimsonShores (1). The replay ran on the
  0.5.5e.hotfix1 game files, whose terrain inputs are byte-identical to 0.5.5e. Details:
  [../re/terrain/dungeon.md](../re/terrain/dungeon.md) "Session 2026-10-27" (BossRush_Area1 entry).
- **Single agent, 2026-10-27 (from handoff 10, Poe2Server `5d6b639` to `95ceda7`, one full replay):** 218,653 of
  218,705 client layouts match (from 218,490), 0 lost; combinations 4465 match, 3 partial, 1
  fail, 39 untested. In the second batch (`full2`) 163 of its 217 failures match now (the
  rest: BossRush_Area1 50, MapCaldera 4). Fixed: ChayulaLeague_Tower and _TowerBoss (outer
  fill alignment in map coordinates, strip fill fallback to the outer ground's tile, levelling
  of empty cells, StoreyHeight by the region holding a tile's corner), G2_12 (size map door type
  from the root slot), G1_13_2 (pre-unique accessibility as listed), RitualLeagueBoss (overlay
  whole-tile check). New generator-trace checkpoints (`tdiff`, `roomgrid`, `preunique_cells`,
  `occupancy`, `placeholder_sizes`, `strip_grounds`, `map_pins`; [../re/terrain/batch.md](../re/terrain/batch.md)).
  Open: BossRush_Area1 (49), MapCaldera (2), MapCrimsonShores (1). Details:
  [../re/terrain/dungeon.md](../re/terrain/dungeon.md) "Session 2026-10-27".
- **Single agent, 2026-10-26/27 (handoffs 9 and 10, Poe2Server `801ae2a` to `df606a3`, gated in one full replay):** 218,490 of
  218,705 client layouts match (from 213,860), 0 lost. Combinations 4457 match, 8 partial, 4
  fail. A second client batch with new seeds (`full2`, master seed 20261026, 50 seeds per area
  on every topology and orientation, 225,400 jobs, 6,598 client errors): 218,585 of 218,802
  match; its 217 failures are all in the areas still open in the first batch. Fixes: door-edge
  walkways, the subgraph window ring, map bounds, league roll chances and Incursion requests,
  PairedRoomLayoutPlugin, the repair flag, subgraph slot alignment, post-unique overhang,
  feature rooms, fill-slot heights, a room's own ground, key-less overlay roots and the extra
  jobs' strip (G4_13), UltimatumPlugin's `end_lowlevel` (G3_10), fill-room heights without a
  passed height (G1_11), the `.tgr` height passes with world-unit heights (Design, P2_1), and
  overhanging subgraph tiles' storey (BossRush_Area3). Still failing: ChayulaLeague_Tower(+Boss),
  BossRush_Area1, G2_12, G1_13_2, RitualLeagueBoss, MapCaldera, MapCrimsonShores. Findings:
  [../re/terrain/dungeon.md](../re/terrain/dungeon.md), [../re/terrain/tgr.md](../re/terrain/tgr.md);
  next steps: [../plans/handoff-agent-10.md](../plans/handoff-agent-10.md).
- **Single agent, 2026-10-25/26 (handoff 8, Poe2Server `f447d9f` to `d9f296b`, gated in one full replay):** 213,860 of
  218,705 client layouts match (from 206,746), 0 lost. Combinations 4222 match, 167 partial, 80
  fail. Subgraph windows share the parent's occupancy (G2_4_1), a second lattice settle after
  overlay pass 1 (MapFrozenFalls), no league rolls on `EndgameMaps` column 28 (JadeCitadel,
  CopperCitadel, logbooks), `OverrideSpawns` with `DefaultKillZone` only (MapExcavation), corridor
  height marking of asymmetric door types (G3_11, G3_12, G3_16), de-repetition skipping
  wildcard-edged uniform tiles (G4_11_2), door-less exits of any height in the `(best)` room pick
  (ExpeditionSubArea_Catacomb), side slots in the prune's order and fill slots in the overlay
  room height (ExpeditionLogBook_Basin), case-sensitive `door` feature (ExpeditionLogBook_Tundra),
  `MatchAllTileKeys` over all slots (P2_3, MapOrnateChambers). After the gate, pushed and gated on
  the match sample only: the filler double count from a virtual default door type (`13d9132`).
  Findings and open items: "Single agent, 2026-10-25 (after wave 6)" in
  [../re/terrain/dungeon.md](../re/terrain/dungeon.md); next steps:
  [../plans/handoff-agent-9.md](../plans/handoff-agent-9.md).
- **Wave 6 (2026-10-24/25, agents A, B, C, merged into Poe2Server `a1e3279`):** 206,746 of
  218,705 client layouts match (from 196,094), 0 lost; all of them also match the doodad hash.
  Combinations 4025 match, 259 partial, 185 fail. Agent A: `.dgr` node slots by edge index
  `0x141d1c580` (with the merged-edge rule `0x141fec1c0`), post-unique tiles for `.dgr` regions
  `0x141ff2090`, edgeless typed nodes accessible. Agent B: oversized `.tgr` subgraphs keep their
  overhanging cells, nested subgraphs depth first, key ground with `.gt` flag 0x10 read as none,
  the world-map `ForceOuterGT` ring and G4_13's Kalguur extra job (not matching yet). Agent C:
  league mission rooms sorted by placed-room index `0x1420cf1d0`, mission rooms as overlay feature
  rooms with one league feature list per job. Biggest gains: G1_13_1, G1_13_2, G3_1, MapSinkhole,
  the stone-circle league seeds. Findings and open items: the "Wave 6" sections of
  [../re/terrain/dungeon.md](../re/terrain/dungeon.md) and [../re/terrain/tgr.md](../re/terrain/tgr.md).
- **Wave 5 (2026-10-23/24, agents A, B, C in three runs each, merged into Poe2Server `b021b44`):**
  196,094 of 218,705 client layouts match (from 176,336), 0 lost; 196,093 of them also match the
  doodad hash. Combinations 3286 match, 877 partial, 306 fail. Dungeon wrong output (agent A):
  the `Spawner` flag from the art `.tmd` or the `.tdt`'s own `.tmd`, case-insensitive `grid_fits`,
  corner writes of tiles overhanging the region replayed with place's rule `0x141d0bad0` and
  `set_heights` outside the map. Dungeon errors (agent B): the tile lookup's second index with
  flattened corner heights `0x141d0a4c0`, EdgeFields by `.dgr` version, lettered unique tags with a
  per-job history, `.tgr` subgraphs inside a `.dgr` window, world-map panels without a sub-seed,
  comma-list room types. `.tgr`, towns and league (agent C): node-tile settle, border dead ends of
  junction splits, `.tst` includes, lock-orientation nodes, league mechanic rooms (StoneCircle) for
  `.tgr` and `.dgr` maps including the mission-room path `0x1420aacf0`. Biggest gains: MapCrimsonShores,
  MapOrbalasCrossing, MapSwampTower, G3_1, G3_6_2, the world maps, the league seeds of every map.
  Findings and open items: the "Wave 5" sections of [../re/terrain/dungeon.md](../re/terrain/dungeon.md)
  and [../re/terrain/tgr.md](../re/terrain/tgr.md).
- **Wave 4 (2026-10-23, agents A, B, C, merged into Poe2Server `03d738b`):** 176,336 of
  218,705 client layouts match (from 108,545), 0 lost; 176,308 of them also match the doodad hash
  (28 differ, all ExpeditionLogBook_Heath topology 1 orientation 6). Combinations 1990 match,
  1888 partial, 591 fail. Dungeon wrong output (agent A): branch-pass corridor heights, overlay
  pick weights and the `room_fits` ground flag, `place_tile` mirror bit, `DungeonFillRoomHeightOverride`,
  special rooms vs feature rooms, case-sensitive corridor key sort, typed-room door rules,
  pre-unique room. Dungeon errors (agent B): 3x3 door spots `0x14209dc90` (WP7.8), FileGroups
  subgraphs, the ShipPlugin second graph job (G4 ship areas), per-graph StoreyHeight, outer fill
  height overrides, multi-rotation `.gft` lines. `.tgr` and towns (agent C): hideouts run the
  terrain RNG disabled, room settle spiral, loop chains split in halves, side-by-side virtual
  edges. Biggest gains: G1_13_1, G1_11, G3_2_1, G2_1, MapUniqueWildwood, MapBackwash, MapLostTowers,
  the hideouts, G4_7, MapBazaar. Findings and open items: the "Wave 4" sections of
  [../re/terrain/dungeon.md](../re/terrain/dungeon.md) and [../re/terrain/tgr.md](../re/terrain/tgr.md).
- **Doodads and collision grid (2026-10-22, WP7.11/7.12, merged):** all 72,718 tile-matching
  layouts also match the client's doodad hash; the generated collision grid equals the client's
  `grids.bin` for 273 of 303 exports (details and open grid fails:
  [../re/terrain/doodads.md](../re/terrain/doodads.md)); server switch `[terrain] generated_grid`.
- Settled (agent C): the client checks the forced topology against the unfiltered topology list (all league
  modes); the matrix counts the mode 1 list. The 11 areas with other-mode topologies need a check.

## To do, in order

1. Done 2026-10-28: `[terrain] generate` on by default (the user's decision 2026-10-27, once the
   natural choice was verified).
2. Done 2026-10-29: MapCaldera / MapCrimsonShores, the full replay of the second batch (all match).
3. The object stage for entities (handoff 10 "Entities"): transitions, waypoints, checkpoints.
4. A live test of `generated_grid` with server movement; then whether it defaults to on.

## Not ported yet (collected from the RE notes)

- World-map pins for the natural topology choice (`0x141fa6440`; the forced search covers it).
- The height fallback of `0x14209f050` (`0x141d05560` / `0x14209fb00`).
- Overlays: phase 2, height offset `0x141fa10b0` (0 for now), `SizeOrder`, exclusions `0x141fa3470`,
  `MatchAllTileKeys` `0x141fa3780`.
- Off-map ends of virtual edge splits (`0x1420ba340`); virtual nodes that no tile fits at the room
  stage (`0x1420caa40` places those that fit).
- Transition tiles (`0x141fe0b40`) and the tile-set option `+0x165` of the unwalkable-corner flood.
- The corner ground fix-ups of a de-repetition replacement (`0x141fde880`).
- `unique<N>` lettered tags from session history.

## Done 2026-10-16

- G1_8: door tile placement (`0x14209d180`); secret-chamber subgraph generated into its
  reserved window and turned with the region; per-graph outer fill and height repair
  (`0x141ff7a10`); edgeless graphs skip the corridor stage; typeless node sizes from the first
  non-default edge type (`0x1420afa40`).
- Pass B (`0x14209fbc0`): named border tiles; merging over a placed tile; the `0x14209f050`
  fallbacks (middle exits, `AllowInteriorOverlapFixes`, height flattening, neighbour repair
  `0x14209ec70`).
- G1_6: exit wiggle after the fill (`0x1420a1f70`, slots recorded in pass A and by the fill);
  `DisableDoors`.
- G1_12 groundwork: `.ecf` rules without a height number; subgraph outer buffers.
