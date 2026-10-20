# Terrain generator: status and next steps

Last updated 2026-10-16 (commit `4f45ba1`), results consolidated 2026-10-20. This is the only place
for terrain status and the resume point for the server-side terrain generator (`crates/poe-world`).
How the client's generator works, and the tools used below, are in
[../re/terrain/](../re/terrain/README.md): [README](../re/terrain/README.md#tools) (tools),
[tgr.md](../re/terrain/tgr.md), [dungeon.md](../re/terrain/dungeon.md),
[shared-passes.md](../re/terrain/shared-passes.md).

## Goal

All act 1 areas generate server-side exactly as the client does (same tile hash for the same
area and seed), so the server can later do collision, pathing and spawns. Area transfer (dev
command and overlay) is done ([../guides/server-admin.md](../guides/server-admin.md)); collision comes later.
Server-side generation is off by default (`[terrain] generate`); `!terrain` runs it for the
current instance.

The client builds its own terrain from the area and seed the server sends, so it loads any area
regardless of our generator. The server's copy is what has to match.

## Current results

Measured with the `area_vectors` example on `dumps/0.5.5e/terrain/vectors/sweep-vectors.tsv` (tools: [../re/terrain/README.md](../re/terrain/README.md#tools)); 25 of 33 vectors match.

| Area | Matching seeds (1001–1003) | Blocker |
|---|---|---|
| Town | 2/2 | — |
| G1_1, G1_2, G1_3, G1_6, G1_7, G1_8, G1_9 | 3/3 each | — |
| G1_4 | 2/3 | seed 1003: overlay placement after the fill (phase 2, `overlay.rs`) |
| G1_5 | 0/3 | edge routing: "could not tilise edge 14" |
| G1_11 | 0/3 | height-change (cliff) edge types: chirality of `woodscliff_height.et` (`0x141d047b0`) |
| G1_12 | 0/1 | in progress, see below |
| G1_13–15 | — | no client traces recorded yet |

G1_6 and G1_8 match only with forced topologies (e.g. G1_8 `1:3` / `11:3` / `4:1`, G1_6 `2:7` /
`2:7` / `1:6`). The client picks its topology from world-map pins, which aren't ported. The
server can force it (`map_force_topology` / `map_force_orientation`, done for instances
created with a topology/orientation); the natural choice is still not reproduced.

Other matches:
- Tile hashes equal the client's for Clearfell Encampment (seeds 1, 2: all 990 tiles, orientations
  and sub-tiles), `G1_1` (1000–1002), `G1_2` (1001–1003, forced topology 15) and `G1_4` (1001, 1002;
  topology 10, orientation 2).
- `check_exports` recomputes the tile and doodad hashes of the DLL's area exports: 42 match, the only
  2 that differ are doodad files from before the parallel world-map capture fix.

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

## Where I stopped: G1_12 seed 1001

- Forced topology `2:7`, `CanopyE_S.dgr`. The main graph is two `graph` nodes (3×3 units of 29
  tiles each): `_CanopyE_S.dgr` (`canopy_master.tsi`) and `Trees_Subscene.dgr`
  (`subscene_master.tsi`).
- All 2385 terrain draws match the client.
- Our map is 169×85; the client's is 174×87. G1_12's `master.tsi` has `RoomOverlap 0` (unit
  29), but the grid overlap (`Grid::overlap`, master +0x14b) looks like it defaults to 1.
  **Next step:** find where `dungeon::layout` reads the overlap, fix it, then compare the layout
  (`POE_LAYOUT_DIFFS=50`).
- New and only checked for "no regression" so far: the subgraph region now sits inside its
  master's `OuterBufferSize` margins (10 units × 6 + 1 = 61 tiles + 13 per side = 87), and each
  subgraph's outer fill and margin strips run on its block (`outer_buffer_window`,
  `TileMap::window`).

Compare with the `area_trace` example: `G1_12 1001 dumps/0.5.5e/terrain/traces/act1_traces/G1_12_1001 2:7`.

## To do, in order

1. Finish G1_12.
2. G1_4 seed 1003: overlay phase 2.
3. G1_5: edge routing.
4. G1_11: height-change edges.
5. G1_13–15: the user records client traces, then port what they need.
6. Doodads (the doodad hash), not ported anywhere yet.
7. Generator features no tested area uses yet: `0x14209dc90` (3×3 door spots), FileGroups
   subgraphs, nested subgraphs, request-driven extra rooms and shrines (`0x1420aab60`,
   `0x1420aacf0`, `0x1420abf60`), injector and blank special rooms.
8. Later: collision and walkability.
9. Done 2026-10-17: forced topology/orientation in AreaInfo, area transfer (command and
   Instances page).

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
