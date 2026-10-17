# Terrain port: status and next steps

Last updated 2026-10-16 (commit `4f45ba1`). This is the resume point for the server-side
terrain generator. The RE details behind it are in [re/dungeon.md](re/dungeon.md) (`.dgr`
dungeons) and [re/terrain.md](re/terrain.md) (`.tgr` graphs).

## Goal

All act 1 areas generate server-side exactly as the client does (same tile hash for the same
area and seed), so the server can later do collision, pathing and spawns. After that: area
transfer through a dev command and a DLL overlay panel. Collision itself comes later.

The client builds its own terrain from the area and seed the server sends, so it loads any area
regardless of our generator. The server's copy is what has to match.

## Current results

`cargo run --release -p poe-world --example area_vectors -- data/sweep-vectors.tsv all [area]`

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
server should force it by sending `map_force_topology` / `map_force_orientation`.

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

Command: `cargo run --release -p poe-world --example area_trace -- G1_12 1001 data/act1_traces/G1_12_1001 2:7`

## To do, in order

1. Finish G1_12.
2. G1_4 seed 1003: overlay phase 2.
3. G1_5: edge routing.
4. G1_11: height-change edges.
5. G1_13–15: the user records client traces, then port what they need.
6. Doodads (the doodad hash), not ported anywhere yet.
7. Generator features no tested area uses yet: `0x14209dc90` (3×3 door spots), FileGroups
   subgraphs, nested subgraphs, request-driven extra rooms and shrines.
8. Server: send the forced topology and orientation in AreaInfo.
9. Area transfer: dev command plus DLL overlay panel.
10. Later: collision and walkability.

## Tools

- `area_trace <area> <seed> <trace dir> [topo[:o] | all]`: compares draws, layout and tile
  hash with a client trace in `data/act1_traces/<area>_<seed>/`. `all` searches every topology
  and orientation.
- `area_vectors <vectors.tsv> all [area]`: regression against client tile hashes. It tries
  forced topologies when the natural choice fails.
- `wiggle_check <generator_trace.tsv> <stop line>`: which client tiles the exit wiggle could
  move.
- Debug env vars: `POE_DBG` (rooms, stuck height repair), `POE_DBG_WIGGLE`, `POE_DBG_KEYS`,
  `POE_DBG_SLOTS`, `POE_DBG_GRID`, `POE_DBG_PATHS`, `POE_DUMP_DRAWS`, `POE_DUMP_PIECES=path`,
  `POE_LAYOUT_DIFFS=n`.
- Ghidra MCP: `connect_instance("Poe2ModdingProject")`. Addresses are noted in
  [re/dungeon.md](re/dungeon.md).

## Done this session (2026-10-16)

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
