# Terrain generation (0.5.5e)

What the client's terrain generator does, as far as it is known, and the tools for porting it
to the server (M5). Addresses are Ghidra addresses (image base `0x140000000`).

## Where generation runs
- `ClientWorld_LoadArea` `0x14167b270` (the in-game world) builds a client generator (base
  constructor `TerrainGen_Construct` `0x141fefa20`, client vtable `0x143385c70`) and runs it
  through `0x1415a1a40` → `TerrainGen_Run` `0x141ffe940`.
- `HeadlessWorld_LoadArea` `0x14212f500` is a second world class (vtable data at
  `0x1435084d0`) that runs the **base** generator (vtable `0x1434ede68`) without rendering.
  This is server-side code compiled into the client, and it is what our server must reproduce.
  The client subclass only overrides object creation and rendering slots (+0x20, +0x68,
  +0x78..+0xa8); every generation phase is shared.
- The world: +0x98 the area row (`+0x2a` WorldAreas HASH16, `+0x1b4` area kind: 0x20/0x21/0x23
  are world-map panels, which the client also generates, several in parallel), +0x114 the tile
  hash, +0x118 the doodad hash, +0x4b4 the seed.
- The generator object: +0x28 terrain, +0x30 generation state (+4 seed; +0x10 regions,
  0x98 bytes each, with +0x58 their tile placements of 0x38 bytes), +0x38 world.

## Phases (`TerrainGen_Run`, in order)
| Address | Name (provisional) | Notes |
|---|---|---|
| `0x141ffe7f0` | prepare | seeds the RNG with 0 and draws ~24 `range` values, before the real seeding |
| `0x141f9c020` | `TerrainRng_Seed(seed)` | the real seed (from generation state +4) |
| `0x141ffcad0` | | |
| `0x141ffc360` | `TerrainGen_Layout` | **almost all RNG draws** (11,000 of 11,300 for a Riverbank layout): tile placement |
| `0x141ffdab0` | | only when generator +0xe |
| `0x141ffde60`, `0x141ffb930`, `0x141d0e5f0` | | |
| vt+0x50 `0x141ffab20` | | |
| vt+0x60 `0x141ff5550` | `TerrainGen_Doodads` | per region and tile placement: `TerrainGen_TileDoodads` `0x141ff3310`; writes the doodad hash; no RNG on the client |
| vt+0x28 `0x141ff3eb0`, vt+0x58 `0x141ff4730`, `0x141ff4b50` | | |
| `0x141ffe0a0` | | height post-pass on tile records (+0x32/+0x33 nibbles) |
| `0x141d141d0` | `Terrain_TileHash` | the tile hash (world +0x114; 1 if it comes out 0) |

The doodad pass has a second branch that runs only when the globals `0x1445c991c` /
`0x14458c705` are set (they also suppress the "Generating …" log lines, so likely a headless
server mode). It scatters objects found by type hash (`0x99e2009b`, `0x892aa9d3`, `0x407c7f95`,
`0x9e3b9e73`) with the RNG and saves/restores the RNG state around it (`0x142cb7380`, 5000
bytes). This is likely server-side placement (spawns), relevant for area entity loading.

## Random number generator
- Thread-local (`TLS + 0x20190`): `u32 seed, u8 disabled, u32 index, u32 state[1248]`. It is
  MSVC's `std::mt19937` (buffered 2×624-word state, tempering mask at the end, standard
  constants `0x9908b0df`, `0x6c078965`).
- `TerrainRng_Seed` `0x141f9c020(seed, disabled)`; `Mt19937_Next` `0x140a58a70(&index)`;
  `UniformInt_Draw` `0x141f0d7f0(&{&index, bits=32, mask=0xffffffff}, n)` is MSVC's
  `uniform_int` rejection loop (`_Rng_from_urng`); `TerrainRng_Below` `0x141f9c270(n)` →
  `0..n` (n 0 is treated as 1); `TerrainRng_Range` `0x141f9c340(a, b)` → `a..=b`. With
  `disabled` set every draw returns 0 / `a`.
- Many callers inline the TLS access and call `UniformInt_Draw` directly.

## Hashes
- `Murmur2_Utf16` `0x140419970(units, count, seed)`: MurmurHash2 over UTF-16 code units, two
  per block; `Murmur2_Bytes` `0x142194f90(data, len, seed)`: standard MurmurHash2. Seed
  `0xf36b12` for both hashes below.
- **Tile hash:** for each tile record, column by column (x outer, y inner), skipping records
  with no sub-tile or with +0x34/+0x35 set: Murmur-style mix of `i16 +0x30` (height), `u8 +0x36`
  (orientation 0..7), x, y and `Murmur2_Utf16(tile .tdt path)`; the u32 results are hashed
  with `Murmur2_Bytes`. The DLL recomputes it from its export, and it matched every area tried.
- **Doodad hash:** the wrapping sum over placed doodads of `y + Murmur2_Utf16(doodad path) +
  (x << 16)` (cells), for doodads whose object data has `+0x2d0 == 0`. Order does not matter.

## Tile records (terrain +0x28, 0x38 bytes, row-major, `w × h` from terrain +0x18/+0x20)
| Offset | Field |
|---|---|
| +0x00 | sub-tile entry (in the tile object's array, 0x38 bytes each, in the tile's own orientation) |
| +0x08 | tile resource (+8: `std::wstring` `.tdt` path) |
| +0x10/+0x18 | vector of 16-byte entries |
| +0x30 | `i16` height |
| +0x32/+0x33 | 4-bit fields (set by the height post-pass) |
| +0x34, +0x35 | skip flags (not hashed) |
| +0x36 | orientation 0..7 (rotation and mirroring; tables at `0x142f020c8`) |
| +0x37 | flags (bit 3 used by the height post-pass) |

A tile is 23×23 cells. Cell grids: see `client-dll.md` (minimap).

## Doodads and tile entities
`TerrainGen_TileDoodads` `0x141ff3310(generator, &hash, placement, region, …)`:
- the tile's doodads come from its data (+0x70..+0x78, 0x58-byte entries; +0x38 the art
  resource, e.g. `Metadata/Terrain/Doodads/Woods/Clutter/Clutter_55_2.ao`); `0x141fe78d0`
  computes the placed cell and angle from the entry, the placement and the orientation;
- the tile's entities come from tile data +0x1b8..+0x1c0 (0x30-byte entries: x, y, path,
  +0x28 angle) and are created with `0x141e6ff30` (an entity of 0x280 bytes);
- both reach the world through generator vfunc +0x38 `TerrainGen_AddObject` `0x141ff7770`.
On the Riverbank the generator creates ~134 objects: `Doodad`/`DoodadNoBlocking`, flow
markers (`Environment/NonDefaultFlows/…`), audio and camera-zoom markers. They are client-side
objects, not sent by the server.

## Tools
- **DLL area export** (`[features] area_export`, on by default, also read-only): every
  generated area goes to `GameFiles/logs/poe-hook/<session>/areas/<n>_<hash16>_<seed>/`:
  `meta.txt` (hashes, size, and the recomputed tile and doodad hashes with match/MISMATCH),
  `layout.tsv`, `doodads.tsv`, `grids.bin` (`PWG1` format), `entities.tsv`, `probe.tsv`
  (walkability probe: per visited cell, each grid layer's value; a walkability layer is never 0
  under the player). "Export area" in the Minimap tab rewrites entities (with the server's) and
  the probe.
- **Generator trace** (`[features] generator_trace` or `POE_HOOK_GEN_TRACE=1`):
  `generator_trace.tsv` in the area export, every seed, raw MT draw, bounded draw (with the
  calling functions) and phase in order. Diff it against the server port to find the first
  divergence. A Riverbank layout is ~23,000 lines.
- **Seed sweep:** the server's `[instance] sweep` (or `POE_SWEEP=<file>`) lists
  `<WorldAreas.Id> <seed> [count]`. Each instance session then generates the next entry instead
  of the character's area, appends `area, seed, tile hash, doodad hash` to `sweep_results`
  (`data/sweep-results.tsv`) and closes. `tools/re/sweep_drive.py` logs the test client in again
  and presses Play for each entry (about 20 s per layout); the DLL exports every layout. A second
  AreaInfo on one connection crashes the client (the old objects sample the regenerated terrain,
  `0x141d04eb0`), hence one layout per session.
- **Dev command** `area <id> <seed>` (`tools/re/send_dev.py auto area G1_1 12345`) sends a new
  AreaInfo on a live connection: it crashes the client for the reason above; kept for experiments.

## Server port (`crates/poe-world`)
Status (2026-10-15): foundations, each checked against the client.
- `rng`: `std::mt19937` and MSVC `uniform_int`, equal to the traced draws (unit tests).
- `hash`: tile and doodad hashes; `cargo run -p poe-world --example check_exports` recomputes
  them for every DLL area export (42 match; the only 2 that differ are doodad files from before
  the parallel world-map capture fix).
- `graph` (`.tgr`), `tdt` (`.tdt` header: size in tiles, edge types per side, ground types per
  corner; `--example tdt_info`). `.arm`, `.tst`, `.gft`, `.tsi`, `.et`, `.gt` come from
  `poe_data_tools`.
- A `.tdt` tile spans `size` tiles of 23×23 cells (most are 3×3, the encampment 15×18); each
  covered tile record names the `.tdt` and its sub-tile index.

### Area data chain
WorldAreas `Topologies` → Topologies `DGRFile` (`.tgr`; an area with several topologies picks
one) → the graph's `MasterFile` (`.tsi`: `TileSet` `.tst`, `FillTiles` `.gft`, `RoomSet` `.rs`,
materials) → rooms (`.arm`) placed at graph nodes, edges (`.et`) routed between them, the rest
filled. Clearfell Encampment (`G1_town`): one topology `town_W_1.tgr` (33×30 tiles, one node
"town" at cell (12, 12)), the room `town_1.arm` covers the whole map; the tile hash changes with
the seed (fill tiles and orientations are random), the doodad hash does not.

### Town trace (seed 1, ~1,860 draws)
24 draws in prepare (seed 0), the real seed, one draw over the whole u32 range (`0x141ff9609`),
one `n=8` (orientation), ~40 small draws (`n` 1/2/8, `0x141f8ed4d`), 990 draws of `n=1024`, then
pairs of a weighted fill-tile pick (`0x141f900a7`; `n` = the fill group's total weight, 300 for
`default` = `Blank.tdt`, 3 for the three `Slash` fills) and an orientation (`0x141f90de4`, `n=8`).

### Port notes (generation order, town first)
1. `TerrainGen_Run` seeds the RNG with the area seed (after `prepare`'s throwaway seed-0 draws).
2. Topology choice, generator vtable slot 0 `0x141ff9280`: candidate topologies
   `0x141fdfe30(world area, world+0xb8)`, filtered `0x141fa6440`; a sub-seed
   `v = below(0xffffffff)` (not for area kind 0x21), then the index from an LCG seeded with
   `v` (`0x141025cf0`: `x = x·1103515245 + 12345`, take `(x >> 16) & 0x7fff` in chunks of up
   to 15 bits until it has `bits(n-1)` bits, reject ≥ n; n < 2 → 0 without stepping).
   `0x141ff89e0` loads the chosen graph (another `below(count)` and `below(0x348)` only when no
   topology was passed); `0x141ff8310` turns the graph into a region record (0x98 bytes, in the
   generation state's region list).
3. Graph nodes are placed by `0x1420c87a0`: candidates matching the node's TileEdgeKey
   (`0x1420c76d0`, 0x68-byte entries, weight at +0x60), weighted pick `below(total)`, position =
   node cell / 23 minus half the candidate's size (+0x31/+0x32), clamped to the map; must fit
   (`0x141d0d880`) and not overlap (`0x1420c4b30`), otherwise the candidate is removed and drawn
   again. Town: total weight 8, draw 3.
4. Rooms are placed by `0x141f8e6c0` (room, x, y, orientation): **every cell** of the room grid,
   x outer, y inner (`.arm` grid row = y). Per cell:
   - `k` slot: key = its edges (file order) with exit/virtual exit, size, corner grounds and
     heights, oriented by the room's orientation. Uniform ground and no edges: skipped (the
     ground is only marked; the fill pass places those). Otherwise the tile set is looked up by
     (slot tag as feature, key) and one candidate is drawn with `below(total weight)`.
   - `f <name>` slot: key 1×1, no edges/grounds, exits 3, feature `<name>` (`forcedblank` →
     `forced_blank.tdt`).
   - The tile-set index (`0x141d09ab0`) lists `(tile, orientation, weight)` per (feature, key)
     in `.tst` order then orientation order 0..7; `.tst` rotation lists (`I R90 R180 R270 FI FR90
     FR180 FR270`, `0x142f08780`) restrict orientations; the default weight is 1.
   - **Verified:** `cargo run -p poe-world --example town_check -- <town export> 1 <trace>`
     reproduces all 71 room-pass placements of Clearfell Encampment seed 1 (tile and orientation,
     RNG in step: 48 draws).
5. After the room pass (town): 990 × `below(1024)` (`0x1420cdc30`), one `below(0)`
   (`0x1420b698a`), the fill pass (66 placements, `0x141f90664`: weighted `.gft` pick for the
   cell's ground, then orientation), and a pass that re-places 10 key-slot tiles in other
   orientations (`0x141fdec67`). The encampment (15×18, `FI R270`) is placed through the room's
   fixed-tile list (`0x141f9e780`), not the place-tile calls above.
6. Generator trace kinds added for this: `below`/`range` with exact call sites, `candidates`
   (the room pass's candidate list), `place` (`0x141f9e2e0`: x, y, orientation, height, tile).
7. Fill pass (`0x141f8f930`, row by row over empty cells): fill group = the cell's marked
   ground's `.gt` name (first line of the `.gt`), else `default`; weighted pick `below(total)`;
   orientation (`0x141f90d60`): square tiles `below(8)` if they fit unrotated, others
   `below(840)` over the set of orientations that fit (`0x1421c3d00`); a tile that does not fit
   is removed and the pick repeated.
8. De-repetition (`0x141fdd7c0`, row by row over cells): for a tile-set tile bigger than 1×1,
   without feature, with a non-uniform key and not touching a room ground-override lattice
   point (`0x141fde360`), check the neighbours one tile-size away (up, right, down, left): the
   same tile in the mirror orientation across the shared side (horizontal 0↔4 1↔7 2↔6 3↔5,
   vertical 0↔6 1↔5 2↔4 3↔7) → `0x141fdfa60` re-picks it from its key's candidates minus those
   mirroring the up/left neighbours (`0x141fde4b0`) or that neighbour; nothing left → unchanged.
   Room ground overrides: the `.arm`'s last line, `(w-1)×(h-1)` string indices (not parsed by
   `poe_data_tools`).
9. Tile hash: the place function (`0x141d0bad0`) stores each cell's offset from the tile's
   corner at record +0x34/+0x35; only offset (0, 0) cells are hashed.

### Graph layout (Riverbank, `G1_1`)
Per region, `0x141ff2c50` loads the graph, rolls edge existence (`0x141fe9970` →
`0x141feeaf0`: per edge with exist % ≠ 0, `below(100) < exist` keeps it; exist 0 removes it
without a draw), then `0x1420bec40` lays the graph out. Seed 1000 → topology 2 of 5
(`intro_3.tgr`, 66×72 tiles; the map is 69×78 with `OuterBufferSize 3 3 3 0`).

`.tgr` v25 fields (parser `0x142aa4ad0` / `0x142aa5470`, caller `0x142927930`):
- node: x, y, edge count + indices, room type, orientation, descriptions, exist %, height,
  priority, deletion behaviour, lock-orientation flag, random offset (4 ints), then x and y
  constraint flags (read by the caller; constrained axes are not jittered).
- edge: from, to, internal points, exist %, chirality, priority, edge type, min/max width.

**Node jitter** (`0x141ef4720` builds, `0x141ef3bb0` applies; `crates/poe-world/src/voronoi.rs`,
`layout.rs`):
1. Sites = node positions, then every edge's internal points (grid cells, 23 per tile).
2. Voronoi diagram (`0x141ef73e0`/`0x141ef8370`): Fortune's sweepline in the
   `VoronoiDiagramGenerator` layout, all-integer (edges unnormalised `b = dx`, `a = -dy`,
   `c = (dx²+dy²)/2 + x·dx + y·dy`; circle events at `y + isqrt(d²)` with `0x14196e010`; the hash
   tables degenerate, so the beach line is walked from an end and the event queue is one sorted
   list). Box `(0, 0, w·23, h·23)`. `clip_line` (`0x141ef7ee0`) pushes each edge to the front of
   a list with its two site numbers; vertices joined to 1 or 3 edges form the vertex list.
3. Cell graph: a node per distinct vertex; per diagram edge (list order) the **nearest** nodes
   to its ends (`0x141f05890`: clipped ends are recomputed from the line and can be a cell off),
   skipped when equal or already joined; both sites' cells get the edge. Then the four corners
   `(0,0) (0,h-1) (w-1,0) (w-1,h-1)`, and the border: nodes with ≤ 2 edges on each side, sorted
   along it, chained (`0x141ef3e10`); each chain edge goes to one cell, chosen among the sites
   of the chain node by an orientation test against the node's first edge (the first chain edge
   tests from the neighbour, the rest from the node, so the signs differ).
4. Per node (then per internal point): triangles (node, cell edge) with weight |cross|/2
   (`0x141ef3390`; a node in a border tile whose cell has an edge on that border takes that
   edge alone, weight 1), `below(total)`, then `a, b = below(128)` (folded when `a+b > 128`),
   offset `((Q-N)·b + (P-N)·a) / 160`.

**Verified:** `cargo run -p poe-world --example river_gen -- <export> 1000` matches the first 110
draws of the Riverbank trace (topology, 19 edge rolls, 30 jitters).

**Room placement** (`crates/poe-world/src/rooms.rs`; trace kinds `node_candidates`, `node_key`,
`node_place` from DLL hooks on `0x1420c76d0` and `0x1420c87a0`). Room nodes are placed in
node order, all before any room pass (the passes then run in the same order):
1. Node key (`0x1420d2e20`): each graph edge (except `.et` types named `Wildcard…`, flag 0x80)
   goes to the side it leaves by, towards its nearest internal point or the other node
   (`0x1420d1220`: `|dx| > |dy|` → right (1) / left (3), else down (2) / up (0)); a side with
   several edges gives one up to a neighbouring side, cheapest angle first (`0x1420d15b0`, angle
   table `0x142f10760`, side vectors `0x142f0c608`). The key is the `.et` per side.
2. Room table: per `.rs` room (its `.arm` tag is the room type) and allowed orientation, the
   turned root-slot key. When no entry of the node's type has the key's edges, `0x1420c51c0`
   moves an edge to a free side (two edges: the first to the following/preceding free side;
   three: the one after the free side back into it, else the one before it forward).
3. Candidates (`0x1420c76d0`): fixed-orientation node → every matching entry, in the node's
   orientation (a room symmetric under a flip appears twice); `(any)`/`(best)` → each matching
   entry in its own orientation if `0x1420b87b0` agrees: per edge, its direction (from/to xor
   chirality) picks the two corners at its end, whose ground types must equal the `.et`'s
   left/right ground (`None`/flag 8 = wildcard) and whose heights its height change; first with
   the climb check, then without.
4. `below(total weight)`; position = node tile − half the size, snapped to the node's alignment
   (`align<N>` description, else the largest edge size; `0x141fe7400` rounds up when the
   remainder is above half), clamped to the map; must fit and not overlap earlier rooms, else
   all entries of that room and orientation are dropped and another is drawn. Then
   `0x1420c59e0` keeps it a margin (largest edge size, twice for virtual edges) from the map
   border on each side with an edge.
5. `.et`: name, left/right ground (`wildcard`), numbers `height_change size flag`, then
   `virtual` (two split edges, chiralities, default min/max width) or `random_wiggle`,
   `can_end`, `force_height_zero`. `.gt` flags: the five bools of line 2 (default 1 1 1 1 0),
   bit 7 for Abyss/Water/Pool/Void names.
6. Before placement every room node is "exploded" (`0x1420cb200`): each edge gets an internal
   point 23 cells from the node towards its side (points within 23 cells removed first,
   `0x1420c6b80`); this matters for routing, not for the draws so far.

**Verified:** `river_gen` places all 12 room nodes of seed 1000 where the client does and stays
in step for 204 draws (through the room passes).

**Repulsion, exits and routing** (`crates/poe-world/src/rooms.rs` `repel`, `routing.rs`; DLL
trace kinds `route_edge` and, with `POE_HOOK_ROUTE_GRIDS=1`, `route_grid` per edge):
1. Placement stage order (`0x1420caa40`): place room nodes → `0x1420cbe60` → tilise each room
   (`0x1420c9970`: room pass, then exits).
2. `0x1420cbe60` (no RNG): every placed room gives a rectangle (room cells grown on each side
   with an edge by its size, twice for virtual edges, `0x1420cb470`); every graph node and then
   every edge internal point gets the sum of the rectangles' pushes (`0x1420cb7a0`: inside →
   out along node→point; within 138 cells of a side or corner → a linear fall-off), a room's
   own node instead gets minus the pushes of the four map-border midpoints; pushes apply after
   all are computed (constrained node axes stay), clamped to the map; then each room is centred
   on its node again (snap, fit, border margin with the alignment).
3. Exits (`0x1420c5750`, `0x1420c5b50`, `0x141ef2800`): edge internal points inside the room are
   dropped; each room edge then ends at the room's exit on its side (exit `e`: tile `e / 3`,
   cell `[3, 11, 19][e % 3]` (`0x142f0df10`), sides 2/3 counted backwards; virtual edges at
   the midpoint of the virtual exit and the exit) with the side's direction as end offset.
4. Router (`0x1420cdc30`): `below(1024)` per tile (row-major), +1024 on the border, placed
   tiles occupied; every edge reserves its straight legs (`0x1420ce390`, Bresenham
   `0x1420ccdc0`: the line's tiles +1 use, tiles across within the margin (size + 2, or max
   width / 23 + 3 for virtual edges) + `(m−k+1)·200000/m/100`); edges routed in graph order
   (`0x1420cd460`): per leg its reservation is lifted, a search runs from the leg's end to its
   start (`0x1420cd110`: neighbours left/up/right/down; open set ordered by (g+h, tile index);
   h = Manhattan × 100; **g = the neighbour's own g + 100 + its cost**, i.e. not accumulated —
   a client quirk that makes it a best-first search; tiles in use or occupied are closed except
   the goal), the path is reserved and every 6th tile adds an internal point at
   `(tile·23 + below(23), tile·23 + below(23))`.

**Verified:** seed 1000 routes all 19 edges exactly as the client (grid identical before each
edge, same internal points); 5022 draws in step.

**Result (2026-10-15):** `crates/poe-world` generates Clearfell Encampment identically to the
client (all 990 tiles, orientations and sub-tiles; tile hash equal) for seeds 1 and 2. Not yet
ported: graph edges (routing over the cost grid), several rooms, rotated rooms, fill rooms,
heights, doodads (their hash), the Riverbank's topology choice.
