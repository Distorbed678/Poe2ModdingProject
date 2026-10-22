# Terrain port to all areas (plan for the next agent, WP7.1)

Written 2026-10-21. This is the plan only: no generator code was changed. Status of the port stays
in [../status/terrain.md](../status/terrain.md) (single source); how the client's generator works
is in [../re/terrain/](../re/terrain/README.md). The matrix this plan is built on is
[../status/terrain-matrix.tsv](../status/terrain-matrix.tsv), rendered into the README tracker.

## Where things stand

- The port (`modules/server/crates/poe-world`, 24 modules) reproduces the client's tile hash for 26 of 33 act 1
  vectors: Town, `G1_1`, `G1_2`, `G1_3`, `G1_6`, `G1_7`, `G1_8`, `G1_9` (3/3 each), `G1_4` (2/3),
  `G1_12` (1/1, fixed 2026-10-22, WP7.4). `G1_5` and `G1_11` give 0 matches and `G1_13`-`G1_15`
  have no client trace yet.
- Update 2026-10-22 (WP7.2): the bulk run over the play-log vectors (1973 distinct vectors, 260 areas)
  matches 365 of them in 59 areas, so areas outside act 1 do match (towns, `G2_13`, `Abyss_Hub`,
  18 `Map*` areas; 107 of the 128 matched combinations only through older builds' vectors).
  The numbers below this line are the 2026-10-21 baseline, before that run.
- Nothing outside act 1 had been compared with a client hash. A first probe (below) shows that
  the generator already runs to the end for many other areas, but a run is not a match.
- Doodads, the doodad hash and the collision grid are not ported at all.
- The server can force a topology and orientation in AreaInfo (`map_force_topology`,
  `map_force_orientation`; verified in game 2026-10-17), so a layout can be reproduced without the
  client's world-map-pin choice.

Matrix totals (2026-10-21, `cargo xtask terrain-matrix`):

| | |
|---|---|
| WorldAreas rows | 442 (432 with topologies, 10 without) |
| Topologies (default league mode) | 1141 (84 more exist for other modes, see 1) |
| Combinations (topology x allowed orientation) | 4508 |
| match / fail / untested | 18 / 1 / 4489 |
| Generator probe, seed 1001, first orientation | 701 of 1141 topologies run to the end; 284 of 432 areas run for every topology |

## Decisions (user, 2026-10-22)

- **Goal:** a 1:1 port: every area in every topology (default league mode: 1141 topologies, 4508
  topology x orientation combinations) and every allowed orientation reproduces the client's
  **tile hash, doodad hash and collision grid** (the client's server-only spawn scatter in the
  doodad pass is out of scope: items and entities are not implemented).
- **Acceptance test:** a script makes the client generate every combination with **50 random
  seeds** (about 225,000 layouts) and compares the port with it. A combination is done when all
  50 seeds match, or it has a documented reason.
- **Method: in-client batch generation.** The login sweep (20 s per layout) would take about 52
  days, so the DLL drives the client's own generator (`HeadlessWorld_LoadArea` / base generator)
  in a loop on worker threads, private mode only. This replaces the "optional acceleration" of
  section 2 and makes it the primary data source; the forced login sweep (WP7.3) stays as a
  cross-check for a handful of combinations.
- **Dynamic debugging:** agents may use x64dbg on the GameFiles client in private mode (one agent
  at a time, never on official servers).
- The 84 league-mode topologies stay out of scope.

Work package WP7.16 (batch harness) and the 50-seed matrix are added on top of section 5; the
generator WPs keep their order, with the batch results as their regression gate.

## 1. Scope inventory

**Which rows.** Every `WorldAreas` row whose `Topologies` column is not empty generates terrain
from a graph file: 432 of 442. The matrix reads them with `poe_data` (`WorldAreas`, `Topologies`)
and `poe_world::area::topology_entries`, which applies the client's selection (`0x141fdfe30`: the
topologies whose mode column equals 1, else those marked 0x39). The 1141 topologies of mode 1 are
what `generate` uses and what `map_force_topology` indexes (1-based). The unfiltered list has 1225;
the 84 others belong to other league modes of 11 areas (for example `G1_2` 33 raw, 18 in mode 1)
and are not in the matrix. Add them only when league content is in scope.

**Generator type** (the `kind` column; code in `xtask/src/terrain_matrix.rs`):

| kind | Areas | Rule | Notes |
|---|---|---|---|
| `town` | 92 | `IsTown` or `IsHideout` | `G1_town` is a one-node `.tgr`; the other 91 (town hubs and hideouts) are `.dgr` |
| `tgr` | 19 | graph file ends in `.tgr` | act 1 outdoor areas (`G1_1`, `G1_2`, `G1_4`, `G1_5`), `G2_1`, some maps |
| `dgr` | 321 | graph file ends in `.dgr` | dungeon generator (`dungeon.rs`, `dungeon_tiles.rs`) |
| `fixed` | 10 | no `Topologies` rows | `NULL`, `CharacterSelect`, `PersonalHideout`, `GuildHideout__`, `CurrentTown`, `G2_3s`, `G2_8a`, `MapLeaguePortal`, `EndgameDoodad_MeteorCrater`, `EndgameDoodad_BeastGroundZero`; not generated from a graph, out of scope until someone finds what they load |

Six rows are world-map panels (`G1_WorldMap` ... `G4_WorldMap`, `G6_WorldMap`, `TN_WorldMap`; area
kinds 0x20/0x21/0x23): the client generates them but logs no hashes, so they cannot be verified by hash.
Leave them to the end.

**Per act** (areas / topologies / combinations; "Act 10" is the endgame and map content):

| Act | Areas | Topologies | Combinations |
|---|---|---|---|
| 1 | 111 (85 of them towns and hideouts) | 260 | 966 |
| 2 | 45 | 141 | 543 |
| 3 | 28 | 117 | 361 |
| 4 | 28 | 88 | 301 |
| 6 | 24 | 31 | 33 |
| 10 | 206 | 504 | 2304 |

**Orientations.** Each topology carries a bitmask of allowed region orientations (Topologies list
column 6, bit per orientation, `8` = all). 444 topologies allow one, 145 two, 146 four, 384 all
eight (a few allow 3, 6 or 7); a topology with no list is laid out unturned (orientation 0). The
combination count (4508) is the sum over topologies of the set bits. Orientation passes through
shared code (`region::turn_graph`, edge-type chirality), so it is sampled, not exhausted, in the
verification plan (section 2).

**The matrix.** `cargo xtask terrain-matrix [--dumps <dir>] [--out <file>]` reads the game data
(`GameFiles/`) and these optional result files in `dumps/0.5.5e/terrain/vectors/`, then writes
`docs/status/terrain-matrix.tsv` (columns `area act kind topology orientation graph status probe
area_vectors evidence`; `cargo xtask tracker` renders it, so the tracker needs no game files):

| File | Written by | Meaning |
|---|---|---|
| `area-vectors-results.txt` | `area_vectors` stdout (`area_vectors <vectors.tsv> all > ...`) | `match (natural T:O)` or `match (forced T:O)` marks that combination `match`; `differ` / `error` have no known combination and show only in the area's `area_vectors` column |
| `forced-results.tsv` | forced-layout client traces (section 2) | `area seed topology orientation match\|fail detail`; the only source of `fail` for a known combination |
| `probe-results.tsv` | `area_probe` example | `ok`/`error`/`panic`/`timeout` per topology at its first allowed orientation: the generator runs, no hash check |

Since 2026-10-22 a combination keeps its seeds: `seeds` is matched/tested (one result per seed; a
match from any source beats a fail on the same seed), the status is `match` when every tested seed
matched, `partial` when some did, `fail` when none did (only forced results give a fail: a vector
that does not match has no known combination) and `untested` without a seed; `old` counts matched
seeds vouched for only by an older build's vector. The inputs of 2026-10-22: the 1973 play-log and
sweep vectors (365 matches, 128 combinations), the forced result for `G1_12` (match) and the probe
(3 seeds, all orientations).
Rerun the three producers and the matrix after every generator change:

```
V=dumps/0.5.5e/terrain/vectors            # in the hub, so GameFiles/ and dumps/ resolve
cargo run --release --manifest-path modules/server/Cargo.toml -p poe-world --example area_probe -- \
  --seeds 1001,1002,1003 --orientations all > $V/probe-results.tsv      # about 15 minutes
cat $V/sweep-vectors.tsv $V/terrain_vectors.tsv | sort -u > runtime/all-vectors.tsv
POE_VECTOR_BUDGET=180 cargo run --release --manifest-path modules/server/Cargo.toml -p poe-world --example area_vectors -- \
  runtime/all-vectors.tsv all --hints $V/area-vectors-results.txt --probe $V/probe-results.tsv --runnable-only \
  > runtime/avr.txt && mv runtime/avr.txt $V/area-vectors-results.txt   # about 7 minutes
cargo xtask terrain-matrix && cargo xtask tracker
python3 scripts/re/probe_classes.py --areas              # errors grouped by unported system
```
(`--hints` reads the earlier results, so write to a new file first; `forced-results.tsv` comes
from `forced_verify`, see WP7.16.)

**First probe findings** (seed 1001, first allowed orientation, no hash check; the generator stops
at the first error, so the counts are lower bounds): 701 of 1141 topologies run to the end, 440 stop
with an error, none hangs (3.1 s at most per attempt). The error classes and the systems behind
them are the ranking in section 4. Two warnings: a verified area is not clean in every topology
(`G1_2` matches at topology 15 but topologies with "no room fits" exist; `G1_7` matches yet one
topology fails "failed to connect" at this seed), and errors depend on seed and orientation, so the
probe must run several of both (WP7.2).

## 2. Verification at scale

**Data sources, in order of trust.**

1. *Forced sweeps (authoritative).* The server sends AreaInfo with the first stat record
   `map_force_topology` (key 0xb92, 1-based topology) and `map_force_orientation` (orientation +
   1); the client generates exactly that layout and reports its tile and doodad hash in
   TerrainHashes. This path exists for instances (`modules/server/servers/instance-server/src/lib.rs`, around
   line 690, `transfer <area> [seed] [topology] [orientation]`). The seed sweep (`[dev] sweep`,
   `POE_SWEEP`, `scripts/re/sweep_drive.py`) does not use it yet: its entries go through the dev
   `area_info_command` with no stats and a `<area> <seed> [count]` list, and the results file has
   no topology or orientation columns (WP7.3).
2. *Play-log vectors.* `dumps/0.5.5e/terrain/vectors/terrain_vectors.tsv` (1940 rows, 260 areas,
   from the user's `Client.txt` through `scripts/re/clientlog_terrain.py`, with the client build).
   Only 5 rows come from the current build 323943; the rest are from 36 older builds, whose
   terrain data may differ. A match from any build is valid (the hash is of the generated output);
   a mismatch from an older build is inconclusive. 171 areas with at least one runnable topology
   have 1378 of these vectors, so this is the cheapest evidence there is. `area_vectors` finds the combination
   by trying every topology and orientation when the natural pick differs, one generate after the
   other: a failing vector of an act 1 area costs about 100 s, which made a bulk run unusable
   (tried 2026-10-21 and cancelled). It needs threads, a time budget and a build filter (WP7.2).
3. *DLL area exports.* `area-exports/<session>/` (80 sessions) hold `layout.tsv`, `doodads.tsv`,
   `grids.bin` and `meta.txt` of every area the client loaded. `check_exports` only recomputes the
   hashes of the export itself (42 match, 2 old doodad files differ); the layouts, doodads and
   grids are the reference for the doodad and collision work (sections 4 and 5).

**Cost model.** One client layout per login, about 20 s (`sweep_drive.py`: a second AreaInfo on a
connection crashes the client). Tiers, by combinations:

| Tier | What | Layouts | Time |
|---|---|---|---|
| T1 | every topology at its first allowed orientation, one seed (skip topologies the probe says cannot run yet) | 1141 (701 run today) | 6.3 h (3.9 h) |
| T2 | plus up to two more orientations per multi-orientation topology (one rotation and one mirror, to cover `region::turn_graph` and chirality) | about 1250 | about 7 h |
| T3 | all remaining combinations, and a second seed for a 5 % sample | about 2100 | about 12 h |

Do T1 for act 1 first (260 topologies, 1.5 h), then act 2 to 4, 6 and 10. After each generator
fix, re-probe and rerun only the topologies whose probe went from `error` to `ok` or whose result
was `fail`.

**Pipeline** (WP7.3 builds the missing pieces):

1. `scripts/re/sweep_plan.py` (new): reads `terrain-matrix.tsv`, writes a sweep list
   `<area> <seed> <count> <topology>:<orientation>` for a tier, act and probe filter.
2. The instance server (sweep branch of `modules/server/servers/instance-server/src/lib.rs`) parses the optional
   fourth field and sends the forced stat record like `send_area`; the sweep results file gets
   two columns (`topology`, `orientation`). Old files keep working (columns are optional).
3. `scripts/re/sweep_drive.py` gets resume (`--skip N`, because `SWEEP_NEXT` is lost when the
   server restarts), a per-entry retry and a log of entries that never answered (the server
   records `FAILED` after 60 s).
4. `forced_verify` (new `poe-world` example): reads the results file, generates each entry with the
   same forced choice, writes `forced-results.tsv` (`match`, or `fail` with the first difference:
   map size, error text, or hash pair) and prints a summary per area.
5. `cargo xtask terrain-matrix`, `cargo xtask tracker --check`, commit.

Running it needs the client through Proton with Steam running, so ask the user first (CLAUDE.md:
"before relying on Steam being running"); the runs are unattended (`docs/guides/setup.md`).

**Storage** (git-ignored `dumps/0.5.5e/terrain/`, which is already 4.5 GB):

```
vectors/area-vectors-results.txt   area_vectors stdout (matrix input)
vectors/forced-results.tsv         forced_verify output (matrix input)
vectors/probe-results.tsv          area_probe output (matrix input)
vectors/sweep-results.tsv          the server's sweep, client hashes (+ topology, orientation)
sweeps/<date>-<name>/              sweep list, server log excerpt, drive log, results copy
sweeps/<date>-<name>/fail-exports/ area exports of failing combinations only
traces/<area>_<seed>_<T>-<O>/      generator trace of a failing combination (POE_HOOK_GEN_TRACE=1)
```

A sweep makes one area export per layout (several MB with `grids.bin`); keep `meta.txt` of the
matches and the whole export of the fails, delete the rest after `forced_verify` ran (add a prune
step to `scripts/dumps/`). Document the new files in `dumps/README.md`.

**Diagnosing a fail.** Re-run the one combination with `POE_HOOK_GEN_TRACE=1` (the user starts the
client), store the trace under `traces/`, then `area_trace <area> <seed> <trace dir> <T>:<O>`
(first diverging draw) and `POE_LAYOUT_DIFFS=50` (first differing cells) as for `G1_12`.

**Optional acceleration (ask the user first).** `HeadlessWorld_LoadArea` (`0x14212f500`) runs the
base generator without rendering. Driving it from the hook DLL could generate many layouts per
login with no 20 s round trip, but it needs dynamic debugging and an RE pass (x64dbg is the
user's call). Worth a day only if T2/T3 are still pending after act 1 to 4 are done.

## 3. Known blockers carried over

**G1_12, grid overlap (+0x14b).**
- Evidence: forced topology 2:7 (`CanopyE_S.dgr`) matches all 2385 terrain draws; our map is
  169x85, the client's 174x87. The region size is `units x (RoomUnitSize - overlap) + overlap`
  (`docs/re/terrain/dungeon.md`): with unit 29 and 3 units, overlap 1 gives 85 and overlap 0
  gives 87, so the height difference is exactly overlap 1 against 0. G1_12's master has
  `RoomOverlap 0`. The code takes the overlap from the first room's `.arm` dimension, not from
  the master: `modules/server/crates/poe-world/src/dungeon.rs:331-332` (comment says "Master +0x14b", code is
  `set.rooms.first().and_then(|r| r.room.arm.dimensions.uint1).unwrap_or(1)`), used again as
  `grid.overlap` in `area.rs:504-506` for the map size.
- Next step: read `RoomOverlap` from the `.tsi` (`tsi.get("RoomUnitSize")` at `area.rs:504` shows
  the pattern; default 1; confirm the key's offset in Ghidra at master +0x14b), pass it into
  `dungeon::layout`, rerun `area_vectors` (the 25 matches must stay, `G1_3`/`G1_6`/`G1_7`/`G1_8`/`G1_9`
  are the dungeon ones) and `area_trace G1_12 1001 dumps/0.5.5e/terrain/traces/act1_traces/G1_12_1001 2:7`.
  The width (169 against 174) may stay different through the subgraph outer buffer; compare with
  `POE_LAYOUT_DIFFS=50` after the height is right. WP7.4.

**G1_4, overlay phase 2.**
- Evidence: seed 1003 stops with "overlay placement after the fill (phase 2) is not ported"
  (`overlay.rs:758`); seeds 1001 and 1002 match (topology 10, orientation 2). The same class
  stops 22 topologies of 5 areas in the probe (`G1_4`, `MapInferno`, `MapAridPlains`,
  `MapCaldera`, `MapUniqueWildwood`) with four more unported pieces named in `overlay.rs`:
  `SizeOrder` (`:227`), exclusions `0x141fa3470` (`:395`), `MatchAllTileKeys` `0x141fa3780`
  (`:398`) and the height offset `0x141fa10b0` (`:429`).
- Next step: the trace `traces/act1_traces/G1_4_1003` has the client's phase-2 draws (`below(total)`
  position, `below(sum)` entry, `below(840)` orientation, `0x141fa1a70(ctx, 2, 1)`). Run
  `area_trace G1_4 1003 <trace> 10:2`, find the first draw after the fill, port phase 2 on the
  occupancy grid after the fill (the context `0x141fa0ac0` is shared), then the four missing
  pieces, one per sub-step, each checked on an area that uses it. WP7.6.

**G1_5, "could not tilise edge 14".**
- Evidence: seeds 1001 and 1002 stop with the error, seed 1003 generates a graph (`redvale_E2.tgr`)
  but with a different hash; forced search finds no matching combination, so the client's
  topology is unknown. The probe shows the same family elsewhere: `G1_5` topologies 3, 4, 7 ("edge
  0", "edge 14", "edge 4"), `MapBackwash` ("edge 10") and seven `G2_1` topologies ("edge N failed
  to route (retries not ported)", `area.rs:772`): 3 areas, 11 topologies.
- Next step: traces for all three seeds exist in `traces/act1_traces/G1_5_100x`. Run `area_trace G1_5
  1001 <trace> all` to find which topology and orientation the client's draws follow (the first
  divergence moves furthest for the right one), then look at the first diverging draw before the
  failing edge (edge routing `routing.rs`, edge tiles `edge_tiles.rs`; the retry loop of the
  client's router is the "retries" gap). The fix probably serves `G2_1` and `MapBackwash`. WP7.7.

**G1_11, `woodscliff_height.et` chirality.**
- Evidence: all three vectors fail with "dungeon edge type .../woodscliff_height.et (chirality not
  ported)", a guard at `dungeon.rs:321-325` that rejects every edge type with `height_change != 0`
  or a virtual edge before anything is laid out. The guard covers 149 topologies in 44 areas (all
  solo, the biggest single lever in this plan). The rules are already written down in
  `docs/re/terrain/dungeon.md` ("Chirality", `0x141d047b0`: chiral when it changes height, has two
  grounds, or is a virtual edge whose halves differ; typed nodes require `(edge from == node) xor
  chirality`; door pairs `0x141fa7fd0` / `0x141fa7bf0`; corridors pass the edge's chirality), and
  `dungeon.rs` already has `chiral_type` and a `chiral` map on the grid.
- Next step: remove the guard, make each of the places the notes list use chirality (typed-node
  requirement, door pairs, corridor A* neighbour test, placeholder matching, mirrored turns flip it,
  `.arm` exits carry it), run `area_trace` on `G1_11_1001` (forced topology found by `all`), fix
  the first diverging draw, repeat. Heights of cliff edges come with it (the height fallback is a
  separate class, section 4). WP7.5.

**World-map-pin topology choice.**
- Evidence: the client picks the topology from the directions of the connected areas' map pins
  against each topology's direction list, three passes from exact to adjacent (`0x141ff9280` ->
  `0x141fa6440`), and falls back to the index draw. Our `generate` only has the index draw, so it
  matches the client's natural choice rarely: of the 25 matching vectors 5 are natural (`G1_1` x3,
  town x2) and 20 needed a forced topology. The Topologies columns that hold the direction lists
  are still unnamed in the schema.
- Decision: do not port it for the server. The server sends the topology itself in AreaInfo, so the
  client and the generator agree by construction. WP7.13 makes the instance server always send
  `map_force_topology` and `map_force_orientation` equal to what `generate` would pick, so the
  natural client choice never matters. Port the pins only if replaying an official capture needs the
  official layout; the vectors with a known combination and the connections of `WorldAreas`
  (`Connections` plus the world-map tables) are the data to reverse the rule from.

## 4. Unported systems ranked by areas unlocked

Counts come from `scripts/re/probe_classes.py` on the probe results (areas that run for every
topology once only this class is fixed: "solo"). The probe stops at the first error, so a system
hidden behind another shows up only after the first is fixed; rerun the probe after each WP.

| Rank | System | Topologies | Areas touched | Areas unlocked alone | Evidence |
|---|---|---|---|---|---|
| 1 | Dungeon edge types with height change or virtual edges (chirality) | 149 | 44 | 44 | guard `dungeon.rs:321`; G1_11 |
| 2 | 3x3 door spots `0x14209dc90` | 77 | 27 | 23 | `dungeon_tiles.rs:867`; `G1_13_1`, `G3_5`, `G4_8a/b`, `G4_10`, `P1_1`, `P2_6` and 20 more |
| 3 | FileGroups subgraphs, nested subgraphs | 29 | 11 | 11 | `dungeon.rs:431` ("Upper" groups), `area.rs:429-440`; `G2_12`, `G3_1`, `P3_5`, `BossRush_Area1-3` |
| 4 | Dungeon tile pass tails: no border tile, no tile for slot, overlap tile | 12 | 10 | 9 | `dungeon_tiles.rs:406,497,584`; `G3_4`, `G4_3_2`, `G4_12` |
| 5 | Fill: orientation masks, no fill tile | 20 | 10 | 6 | `fill.rs:32,244`; `G4_1_2`, `G4_2_1`, `G4_9_`, `G1_4`, `G2_1` |
| 6 | Room set or feature rooms missing (`no RoomSet`, `GetFeatureRooms`, no root key) | 18 | 8 | 8 | `dungeon.rs:493`, `rooms.rs:187`; Incursion temples, `P1_6` |
| 7 | Height fallback `0x141d05560` / `0x14209fb00` | 8 | 8 | 8 | `outer_buffer.rs:654`; Sanctum levels, `G4_4_3`, `P3_6` |
| 8 | Dungeon rooms: no room fits / no suitable room | 6 | 5 | 5 | `dungeon.rs:519,1099` |
| 9 | Overlays: phase 2, SizeOrder, exclusions, MatchAllTileKeys, height offset | 22 | 5 | 4 | `overlay.rs:227,395,398,429,758`; `G1_4` |
| 10 | Dungeon corridors: edge ends outside the grid `0x1420a3770`, failed to connect | 5 | 5 | 5 | `dungeon.rs:1525,1536` |
| 11 | Data path handling (a `//` in a joined path) | 17 | 4 | 4 | `FileNotFound` on `.../subarea//graphs/...` |
| 12 | `.dgr` header/edge field parse (`bad number`) | 19 | 4 | 4 | `G2_3`, `G2_3a`, `G4_5_2`, `Programming` |
| 13 | Special rooms: blank (ground match), injector, split | 11 | 3 | 3 | `dungeon.rs:1917,1921` |
| 14 | Virtual nodes and virtual edge pairs (node tiles) | 5 | 3 | 2 | `area.rs:678`, `node_tiles.rs:466` |
| 15 | tgr edge routing, retries, tilising | 11 | 3 | 1 | blocker G1_5, `G2_1` |
| 16 | Lettered `unique<N>` tags (session history) | 17 | 2 | 2 | `area.rs:220`; `G1_15`, `G2_6` |

Greedy order (each step the class that makes the most areas fully run): 1, 2, 3, 4, 5, 6, 7 ...
takes the clean areas from 284 to 328, 351, 362, 372, 380, 388, 396; all classes together reach
all 432.

Systems the probe cannot see (they do not raise an error; they show up only as hash mismatches
or as missing output):

| System | Areas | Why it matters | Verification data |
|---|---|---|---|
| Doodads and the doodad hash (`TerrainGen_TileDoodads` `0x141ff3310`, cell and angle `0x141fe78d0`, entries at tile data +0x70..+0x78, 0x58 bytes each; hash in `hash.rs`) | all 432 (every area has a doodad hash; towns vary with quest state) | the client sends tile hash and doodad hash; only the tile hash is matched today. Doodads are also the entities and blockers the server will want | `doodads.tsv` in every area export (80 sessions) and the doodad hash column of every vector and sweep result |
| Collision grid output (feeds `GridSource`, `modules/server/crates/poe-game/src/grid.rs`) | all 432 | server movement and spawning on the generated terrain. Today the DLL's grid (`terrain_grid_<hash>.bin`, `PWG1`) or the client's upload over the instance port (WP3.4) stands in | `grids.bin` in every area export and `dumps/.../grids/*.bin`: compare cell by cell |
| Extra rooms and shrines, request-driven (`0x1420aab60`, `0x1420aacf0`, `0x1420abf60`) | 0 plain areas | `dungeon.rs:602` notes they place nothing without a request (league mechanics, area mods); matters only once AreaInfo carries such requests | a forced trace with a request, from the user |
| Off-map virtual ends `0x1420ba340`, transition tiles `0x141fe0b40` and tile-set option `+0x165`, de-repetition corner fix-ups `0x141fde880`, height offset of overlays | unknown | silent; decided by T1/T2 mismatches | forced-layout fails with `area_trace` |
| World-map panels (6 rows) | 6 | the client generates them, logs no hashes | export compare only |

The first two rows are independent of generator coverage: they apply to the 9 areas that already
match (`G1_1`, `G1_2`, `G1_3`, `G1_4`, `G1_6`, `G1_7`, `G1_8`, `G1_9`, Town) and can be built and
checked on those right away, in parallel with the generator work (WP7.11, WP7.12). Collision gets
one more input: the corner lattice's walkability flags (`shared-passes.md`, "Unwalkable corners",
flag 8, `0x141fe0fd0`) are the base; whether doodads add blocking cells is what the first
comparison with `grids.bin` will show.

## 5. Work packages

Sized for one Sonnet subagent each (a few hours; WP7.5, WP7.11 and WP7.12 are the large ones).
Rules for all: follow CLAUDE.md (commit and push per working step, no hardcoded data, docs in
the same commit, `python3 scripts/check-links.py`); never touch Steam or `GameFiles/` writes; do
not launch the client without the user; ask for dynamic debugging, new official captures and Steam
runs. After each WP: rerun the three producers, `cargo xtask terrain-matrix`, `cargo xtask tracker`,
update `docs/status/terrain.md` (status) and `docs/re/terrain/` (findings and addresses). Regression
gate for every generator WP: `area_vectors sweep-vectors.tsv all` keeps its 25 matches
(`cargo test -p poe-world` too) and the probe does not get worse (no topology that was `ok` turns
`error`).

Order and parallelism (files that collide are in the same wave only if they are different
packages' files):

| Wave | Packages | Notes |
|---|---|---|
| 1 | WP7.2, WP7.4, WP7.11 (RE and port start), WP7.12 | offline; WP7.4 first among the `dungeon.rs` editors (it is small) |
| 2 | WP7.3 (needs the user), WP7.5, WP7.6, WP7.7 | WP7.5 edits `dungeon.rs`/`dungeon_tiles.rs`, WP7.6 `overlay.rs`, WP7.7 `routing.rs`/`edge_tiles.rs`/`area.rs`: separate worktrees |
| 3 | WP7.8, WP7.9, WP7.10a-d | all touch `dungeon.rs`/`dungeon_tiles.rs`: run one after the other after WP7.5 |
| 4 | WP7.13, WP7.14, WP7.15 | WP7.15 needs the user and finishes the plan |

**WP7.2 Bulk vectors and a better probe** (small, offline). **Done 2026-10-22** (see [../status/terrain.md](../status/terrain.md#verification-at-scale-2026-10-22-wp72-and-the-port-side-of-wp716)).
- `area_vectors`: run the failing-vector forced search on threads, add `POE_VECTOR_BUDGET=<seconds>`
  per vector, a build filter (`area_vectors <tsv> <build|all|>=N> [area]`) and a `natural only` mode;
  keep the output format (the matrix parses it).
- `area_probe`: options for several seeds and orientations (`--seeds 1001,1002,1003`,
  `--orientations first|all|N`), threads, one TSV line per attempt with the orientation and seed;
  extend `terrain_matrix.rs` so the probe cell reflects every probed orientation and seed (it only
  reads the first orientation of the first seed today) and update its tests.
- Run `area_vectors` over `terrain_vectors.tsv` for the 171 areas with a runnable topology that have
  vectors (1378 vectors), merge into `area-vectors-results.txt`, rerun the matrix.
- Acceptance: `cargo test -p xtask` and `cargo build` pass; `area-vectors-results.txt` covers every
  vector of those areas (or records `skipped (budget)`); the matrix shows how many combinations
  matched from older builds, and the README section shows them; the plan's counts are updated in
  `terrain.md`.

**WP7.3 Forced-layout sweep tooling and the act 1 T1 run** (medium; the run needs the user).
- Items 1 to 5 of the pipeline in section 2: `sweep_plan.py`, the sweep list's fourth field and
  forced stats in `instance-server` (reuse the code at `lib.rs:690`), two results columns,
  `sweep_drive.py` resume and retry, the `forced_verify` example, `forced-results.tsv` into the
  matrix (already read by `terrain_matrix.rs`), docs in `docs/guides/setup.md` and
  `docs/re/terrain/README.md` (Tools), `dumps/README.md`. Another agent edits `modules/server/servers/` for the
  multiplayer test: coordinate or do this after it lands.
- Acceptance: a forced sweep of three known combinations (`G1_8 1001 1:3`, `G1_6 1001 2:7`,
  `G1_2 1001 15:0`) gives `match` in `forced-results.tsv` with the client's hashes equal to
  `area_vectors`'; then the act 1 T1 list (260 topologies, probe-ok first) runs unattended once;
  `forced-results.tsv` and the matrix hold the result; fails have a stored trace or export
  and are listed in `terrain.md`.

**WP7.4 G1_12 grid overlap** (small, offline). Section 3 first blocker. **Done 2026-10-22**: `RoomOverlap` read from the master; 174x87, hash 3369045138 equals the client's ([../status/terrain.md](../status/terrain.md)).
- Acceptance: `area_trace G1_12 1001 ... 2:7` reports map size 174x87 (or the next difference with
  its first differing cell named in `terrain.md`); the 25 vector matches stay; if the tile hash
  equals 3369045138, `G1_12` is `match` in the matrix and `terrain.md`.

**WP7.5 Dungeon height-change and virtual edge types (chirality)** (large, offline; unlocks 44 areas).
- Section 3, `G1_11`. Do it in sub-steps: (a) typed-node requirement and door pairs, (b) corridor
  A* and branches, (c) placeholder matching and mirrored turns, (d) heights of cliff edges. Check
  each on `G1_11_1001` to `1003` traces; port nothing the traces do not exercise without a note.
- Acceptance: `G1_11` seeds 1001-1003 match through `area_vectors` (forced search); the probe class
  "chirality not ported" is gone (rerun the probe); the number of areas that run for every
  topology rises from 284 by at least the part of the 44 that has no other class; the 25 matches
  stay.

**WP7.6 Overlays: phase 2 and the missing pieces** (medium, offline; 5 areas).
- Section 3, `G1_4`. Then `SizeOrder`, exclusions, `MatchAllTileKeys`, height offset, each on an
  area that uses it (`MapAridPlains`, `MapCaldera`, `MapInferno`, `MapUniqueWildwood` have no hash
  yet: use WP7.3 to get forced hashes, or a play-log vector).
- Acceptance: `G1_4` 3/3; probe class "overlay" gone; for each of the other pieces a forced hash or
  vector matches, or the piece is listed in `terrain.md` as untestable.

**WP7.7 tgr edge routing, retries and tilising** (medium, offline; `G1_5`, `G2_1`, `MapBackwash`).
- Section 3, `G1_5`. Start with the topology search on the three traces.
- Acceptance: `G1_5` 3/3 matches or the first diverging draw and its reason is written into
  `docs/re/terrain/tgr.md`; the probe no longer lists "retries not ported" for `G2_1`.

**WP7.8 3x3 door spots `0x14209dc90`** (medium; 27 areas).
- Port the 3x3 door spot pass (the `dungeon_tiles.rs:867` guard), starting with `G1_13_1`, whose
  traces the user has not recorded yet: get a forced hash from WP7.3 for `G1_13_1`, `G3_5` and
  `G4_8a` first (T1 list for these three areas), or a play-log vector.
- Acceptance: the class "door tile" is gone from the probe; at least two of the three areas match a
  client hash; `docs/re/terrain/dungeon.md` loses the "not ported" line.

**WP7.9 FileGroups and nested subgraphs** (medium; 11 areas).
- Implement subgraph file groups (`dungeon.rs:431`, groups such as "Upper": which file the client
  picks and with which draw) and nested subgraphs (`area.rs:440`), reusing the subgraph window code
  that `G1_8` and `G1_12` use.
- Acceptance: the class "FileGroups" is gone from the probe; two of `G2_12`, `G3_1`, `P3_5`,
  `BossRush_Area1` match a forced hash.

**WP7.10 Tail classes** (small each, one subagent for two or three):
- a. Data path handling (`//` in joined paths), `.dgr` parse errors (`bad number`), missing room
  sets and root keys: pure data fixes. Acceptance: 12 areas (4 + 4 + 4) leave the probe's error
  list or have a documented reason.
- b. Height fallback `0x141d05560` / `0x14209fb00` and "could not fix heights": 8 areas
  (Sanctum levels, `G4_4_3`, `P3_6`).
- c. Dungeon tile pass tails, dungeon corridors (`0x1420a3770`), "no room fits": 20 areas.
- d. Fill orientation masks and "no fill tile", virtual nodes and virtual edge pairs: 13 areas.
- Acceptance for b to d: the class is gone from the probe; one area per class matches a hash.

**WP7.11 Doodads and the doodad hash** (large; all areas; start offline).
- RE `TerrainGen_TileDoodads` (`0x141ff3310`), `0x141fe78d0` (cell and angle from the entry, the
  placement and orientation) and where the tile's doodad entries (tile data +0x70..+0x78) come
  from (probably the tile geometry file the `.tdt` names; `poe_data_tools` may parse it). Add a
  `doodads` module to `poe-world`, the doodad hash into `area::generate`'s result.
- Acceptance: for every export in `area-exports/` whose area the port generates (the act 1 matches),
  the generated doodad list equals `doodads.tsv` (a `check_doodads` example) and the doodad hash
  equals the client's (vectors and sweep results); town hashes that vary with quest state are
  listed separately. The matrix does not need a new column: extend the `area_vectors` output to
  report `doodads match/differ` (tell the matrix parser).

**WP7.12 Collision grid output** (large; all areas; start on the matching act 1 areas).
- Build the cell grid the client builds (four layers; layer 1 is the likelier walkability grid per
  `docs/re/terrain/README.md`, "Cell grids") from the generated tile map, the corner lattice
  walkability and (if the comparison says so) doodad blockers. Emit it as `PWG1` bytes that
  `poe_game::grid::Grid::parse` reads and implement `GridSource` for it (poe-world must not depend
  on poe-game: either a small adapter in the instance server or `poe-game` depending on
  `poe-world`; decide once and write it in `docs/architecture/`).
- Acceptance: for each of the 9 matching areas, the generated grid equals the DLL's `grids.bin` of
  the same seed cell for cell (a `check_grids` example, layer by layer, with the count of differing
  cells); the instance server can use the generated grid in place of the uploaded one (a config
  switch), and server movement (`docs/protocol/movement.md` tests) runs on it with the drift under
  one cell.

**WP7.13 Server-chosen layout** (small, offline).
- The instance server always sends `map_force_topology` and `map_force_orientation` equal to the
  generator's own pick (`area::generate`'s index draw and default orientation, exposed as a
  function), so the client's world-map-pin choice never matters. Keep a `natural` override for
  the admin command.
- Acceptance: `transfer G1_6` and `transfer G1_8` with random seeds give `Tile hash` equal to the
  server's generation in the server log (user runs it once); `docs/guides/server-admin.md` says what
  the default is. Do not port `0x141fa6440` unless an official capture needs it.

**WP7.14 Rare features** (small to medium, last): blank special rooms (ground match) and injector
special rooms (`dungeon.rs:1917,1921`), lettered `unique<N>` tags (`area.rs:220`, needs a
session-history trace from the user), request-driven extra rooms and shrines, Incursion temples
(`GetFeatureRooms`). Acceptance per feature: a forced hash or a documented reason it is deferred.

**WP7.15 Full sweep, matrix closing and the default switch** (medium; needs the user).
- Run tiers T1 to T3 over all acts, with re-probes in between; fix or document each `fail`; update
  the matrix, `terrain.md` and the tracker.
- Acceptance: every combination is `match`, `fail` with a stored reason, or `untested` with a
  reason in `terrain.md` (the 10 `fixed` rows and the 6 world-map panels listed); the README matrix
  shows the totals; then decide with the user whether `[terrain] generate` in `modules/server/config/server.toml`
  defaults to on.

**WP7.16 In-client batch generation and the 50-seed matrix** (large; needs the client, private
mode; x64dbg allowed). Added 2026-10-22, see Decisions.
- Hook feature `gen_batch` (off by default, refused in official mode): reads a job list
  (`area seed topology orientation`), runs the client's base generator for each job on worker
  threads without rendering, writes `batch-results.tsv` (`area seed topology orientation status
  tile_hash doodad_hash width height ms detail`), exports layout, doodads and grid of a job only
  on request (the fails). `scripts/re/terrain_batch.py` plans every combination x 50 random seeds
  (seed list recorded), starts the client, resumes after a crash, skips finished jobs.
- Port side (done 2026-10-22, tested on a hand-made batch): `forced_verify` reads `batch-results.tsv`,
  generates every row with the same forced choice on threads and writes `forced-results.tsv`;
  `terrain-matrix` has the seed count per combination (`seeds` matched/tested, status `match`,
  `partial`, `fail`, `untested`).
- Acceptance: the batch's hashes equal the login sweep's for `G1_8 1001 1:3`, `G1_6 1001 2:7`,
  `G1_2 1001 15:0` and the 25 known vectors; act 1 x 50 seeds runs unattended; the measured time
  per layout and the projected full-run time are in `docs/status/terrain.md`.
