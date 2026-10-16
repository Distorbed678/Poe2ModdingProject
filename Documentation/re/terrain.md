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
   Details (`crates/poe-world/src/area.rs` `topologies`, `generate_with`):
   - `0x141fdfe30` keeps the topologies whose last Topologies column (topology +0x40, league
     or mode) equals the area's mode; if none match, it keeps those marked 0x39 ("any"). With
     neither it retries with mode 1. Act areas use 1, and their league variants use other
     values (54).
   - `0x141fa6440` matches the world map. `0x141fa6150` lists the directions of the area's
     connections (WorldAreas connections → their map pins), one entry per connection. Each
     topology's direction list (Topologies list column, +0x24) is compared in order, turned
     by each allowed orientation (+0x34 list; 8 = all). There are three passes: exact
     (`0x141fa6960`), with topology wildcards 8 (`0x141fa6a50`), then also adjacent
     directions (`0x141fa6b40`). The LCG index picks among those candidates. Only when there
     are none does `0x141ff90f0` pick from the whole list. The map-pin part is not ported, so
     `G1_2` seed 1001 picks topology 15 (`felling_S_N_W`) in the client, while the plain index
     gives 3.
   - Stats on the area (generator +0x38 → +0x128, filled from AreaInfo's flags0 bit 7 stat
     records, `0x1423baa70`: varuint count, then big-endian u16 key and value per stat):
     `map_force_topology` (key 0xb92, Stats row 2961) = 1-based index into the filtered list,
     and `map_force_orientation` (0xb93) = orientation + 1. A forced topology still draws the
     `below(0xffffffff)` value. The server can use these to choose the layout itself
     (`generate_with(.., forced)`).
   - `.dgr` and `.tgr` graphs take the same path (`0x141ff2c50` → `0x1420bec40`); `.dgr`
     only adds dungeon fields (`0x14292e8e0`, kept as raw tokens in `graph.rs`).
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

**Edge smoothing** (`crates/poe-world/src/smoothing.rs`; DLL trace kinds `arc_start`, `arc_end`,
`round`, `graph_after_smooth1`; `cargo run -p poe-world --example smooth_check -- <trace>`
replays the logged arc steps). After routing, `0x1420bd650(layout, 1)` (no RNG):
1. Room nodes were split per exit by `0x141ef2800` (the node takes its first edge's exit, every
   other edge gets a new node at its exit, appended in tilise order), so the graph numbering is
   original nodes then exit nodes.
2. Arcs: virtual edges with a start (end) offset get points on a circle tangent to the offset
   direction (`0x1420d1b70` / `0x1420d2470`: centre on the node's perpendicular, radius from the
   chord, 45° steps, polar `0x1421c4f40` on the 256-entry sine table `0x142f10660` with swapped
   interpolation weights and ±127 clamping, so the arc radius is half the computed one); end
   arcs are appended last-first; repeated up to 5 times while anything changes.
3. Corner rounding (`0x1420b3bd0` up to 5 times; per edge `0x1420b2f60`): degree-1 nodes' edges
   first, then every slot of nodes with ≥ 2 edges; walking an edge (and on through degree-2
   nodes), each corner B between A and C with a sharp enough angle is replaced by the tangent
   points of a circle of radius `max width + 10` (one or both, the second inserted with
   `0x1420ced20`), or removed / collapsed to the midpoint (`0x1420b2cd0`) when the tangents
   would overrun.

**Verified:** the whole graph after smoothing equals the client's (seed 1000).

**Virtual edges and the last graph passes** (`crates/poe-world/src/split.rs`; DLL dumps
`graph_after_split`, `graph_before_tiles`):
1. Room exits of virtual edges get a pair of virtual end points (virtual exit, exit), ordered by
   `0x1420c6860` (side of the edge they fall on) and `0x1420c5b50` (side of the exit direction).
2. `0x1420bc400`: edges with virtual end points first (from the end that has them), then the
   rest; each chain of virtual edges through degree-2 nodes is split (`0x1420bb4f0`): the edge is
   cloned, one copy takes split type 0 and the other split type 1 with chirality flags
   (`0x1420bb0c0`), the node moves to the first virtual point and a new node at the second takes
   the clone; at a junction a width `below((max₁+max₂−min₁−min₂)/2 + 1)` and a miter
   (`0x1420b9c30`) give the points, and `0x1420ba050` re-attaches whichever edge does not cross;
   every internal point gets `below(max−min+1)+min` and the two banks are placed by the miter
   (`0x1420baa20`); at the chain's far end the far node takes the second virtual point and a new
   node the first. Off-map ends (`0x1420ba340`) are not ported yet.
3. Then: rounding again, nodes sorted by priority (stable, `0x141f04fe0`), dead ends near the
   map border pushed one cell off the map with an inward offset (`0x1420cade0`), node slots
   compacted (`0x1420bd5f0`), rounding again, and every edge gets a point one step out of each
   end node along its offset (`0x1420b6660`).

**Verified:** the graph before tile placement equals the client's; 5055 draws in step.

Node flags (`+0x88` "fixed", `+0x89` "virtual end"): fixed = room nodes and their exit
nodes, plus dead ends pushed off the map; virtual end = next to a virtual edge in the topology
graph (`0x1420c7400`, `.et` flag 4). The virtual split copies both flags to each new node, and
at a chain's far end sets virtual end on both far nodes unless the far node lies on the map
border (the clone's far node also copies fixed).

**Node tiles** (`crates/poe-world/src/node_tiles.rs`, `0x1420cade0`): at the start of the tile
stage `0x1420c7400` sorts the room-less nodes into lists. The tile list takes dead ends (a
virtual one only at the border), nodes with three or more edges, and nodes whose two edges have
different `.et` types; two edges of one type pass through. Nodes touching a virtual edge
otherwise go to a separate list. `0x1420cade0` walks the tile list. A dead end near the border
(`0x1420c75c0`) is pushed off the map; any other node is tilised like a room node
(`0x1420c87a0`):
1. Key and sides from the node's edges (`0x1420d2e20`, same as rooms).
2. Candidates: the tile-set branch of `0x1420c76d0`. The sizes in which feature-less tiles have
   the key's edges are listed sorted (`0x141d09700`, map +0x68). For each size, every tile
   entry of any feature with those edges and that size is taken (`0x141d0a7d0`, map +0x20,
   tst order then orientation 0..7), using the swapped size when there is none. The list is
   then filtered by the node's orientation, or for `(any)` by `0x1420b87b0` (climb check
   first, then without).
3. `below(total weight)`. Position, alignment and fit work as for rooms; the map cells must be
   free (`0x141d0d880`). On failure the first entry with that tile and orientation is dropped.
   Then `0x1420c59e0`. For `(any)`, `0x1420c9d70` turns the tile a quarter either way
   (orientation table `0x141d00f00`, key turned by `0x141d08a40`) when the summed edge-to-side
   angle (`0x1420b8e20`, degrees) drops. A turn is not allowed if it would put an edge against
   the border the tile touches.
4. `0x1420cbe60` with the room flag clear: the tiles' rectangles push every node except those
   with exactly one edge (a tile's own node is not pushed by its own rectangle) and every
   internal point. Each tile is then centred on its node again where the cells are free, and
   `0x1420c59e0` runs with its alignment.
5. `0x1420c9970`: snapped, placed into the map, and the node's edges are cut at the tile and
   ended at its exits (`0x1420c5750`, `0x1420c5b50`). The first edge keeps the node; each other
   edge gets a new fixed node (`0x141ef2800`).

Riverbank seed 1001 needs this for one node: the narrow river changes from `S` to `M`, so the
node takes `NarrowRiver_Mud/StMS_01.tdt` (3×3, o5).

**Typed nodes without a room, junctions, hashed edges** (found on `G1_2`):
- Room types: when no room of the node's type fits, `0x1420c76d0` falls back to tile-set tiles
  whose feature is the room type (e.g. `endcap` → `Mudburrow/Trail_End_01_02.tdt`). The
  adjustment check `0x1420c5540` accepts a key when tiles of that feature or rooms of that type
  have it. The filter lambdas also keep only tiles of that feature.
- `0x1420c9d70`: an `(any)` placement may turn a quarter (orientation table `0x141d00f00`). The
  turned key, including its size, is what later steps use (the recentring in `0x1420cbe60`).
- Exploding (`0x1420cb200`, before routing): junctions (three or more edges, none virtual) and
  virtual nodes that no tile fits at the room stage (`0x1420caa40` places those that fit; not
  ported). Their edges first drop points within 23 cells of the node (`0x1420c6b80`). Each
  edge then gets a point 23 cells out of the node on its side (`0x1420d1220`, clamped to the
  map), at the start of `from` edges and at the end of the others.
- `.et` fourth number (+0x4d, e.g. `woods.et` `0 3 0 1`): the edge's tiles take their exits from
  the coordinate hash `0x1421969e0` instead of the path curve (`0x1420b7490` is not called for
  them).
- Tile-set index keys (`0x141d09900`) drop grounds whose `.gt` has flag 0x10 (the fifth bool,
  e.g. `wideriver_trim.gt`). `TileDef::key` holds that key.
- Overlays: see "Overlays (`.toy`)" below.

**Region orientation** (`0x141ff89e0`, `crates/poe-world/src/region.rs`): a topology may be laid
out turned or mirrored. Table `0x142f020c8` drives the point transform (`0x141d14b30`), the
centring of even-sized placements and the alignment snap `0x141fe7400` (a remainder of exactly
half rounds by orientation). `0x141d14990` turns the graph; mirrored orientations flip the
chirality of edges whose type is not symmetric. Forced by the `map_force_orientation` stat (o+1).

**Graph pruning** (`0x141fe9970`): node existence (`values[0]`: 0 removes, ≥100 keeps, else
`below(100)`), edge rolls, `shuffle<N>` groups (random pair swaps of room type and tags),
`unique<N>[#count]` (keeps `count` random nodes; lettered tags from session history are not
ported). Removal `0x141feb9d0`: two same-type edges at equal heights merge (`0x141fec1c0`); a
node with ≥2 edges only loses its room; otherwise `0x141fec520` retracts its edges with points
to a new node at their nearest point and erases it.

**Virtual junctions without tiles** (`0x1420c6cd0`): point drops, pair loop with `0x1420c6420`,
ordering `0x1420c6860`, node split `0x1420c5ee0`, and logical links. The split updates the links
(`0x1420baf40`). `0x1420bcff0` resolves them: pairs at identical positions, else
`0x1420bc9a0` with segment distance `0x142196290`. Link nodes of degree >2 join the tile list.

**Overlays (`.toy`)** (`crates/poe-world/src/overlay.rs`; master `Overlays`, +0x200): passes with
a header `phase1 phase2 [phase3] [Random|FileOrder|SizeOrder] [±flags]` and entries
`weight "arm" [±flags] [!excl] [limit=N] [required=N] [rotations]`. Flags: 0x01 TileOverlays,
0x02 FeatureTiles, 0x04 FeatureRooms, 0x08 FeatureRoomOverlap, 0x10 DescriptionNodes, 0x20
ExtraFeatures, 0x40 MatchAllTileKeys (default 0x18). Phase 1 runs before the fill
(`0x141fa1a70(ctx,1,0)`), phase 2 after it (`(ctx,2,1)`).
- Context `0x141fa0ac0` keeps an occupancy grid. Candidates `0x141fa1e60` step by 3 when the
  key has a size-3 edge and sort by `x·0x10000+y`.
- Fit `0x141fa1830`: occupancy, whole pieces (`0x141d0dc40`), then room-fits `0x141f9d200`
  (wild 0, margin 1: corner grounds `0x141f9cfc0`, consistent height differences, side checks
  `0x141f9cee0`/`0x141f9ca30`/`0x141f9c800`/`0x141f9c600`).
- Placement `0x141fa23e0`: `below(total)` position, `below(sum)` entry (padded to 100 with
  "nothing"), `below(840)` orientation. Overlay rooms go through the normal room pass.
- Not ported: phase 2, height offset `0x141fa10b0` (0 for now), SizeOrder, exclusions
  `0x141fa3470`, MatchAllTileKeys `0x141fa3780`.

**Fill-room uniform cells** (`0x141f8c8c0` → `0x141f8e400`): `s` slots and `k` slots without a
feature or edges on one ground take a fill tile of that ground. The orientation is chosen in the
room's frame (as for fill tiles) and composed with the room's. The tile is placed from the
smallest corner its cells reach (per-orientation offset table in `0x141f8e400`). It must fit the
map (`0x141f9e5f0`: one cell of slack, then `0x141d0d940`) and the room grid (`0x141f8c500`): it
stays inside the room and covers only uniform cells whose ground equals the tile's first corner
ground (or both are missing or have `.gt` flag 8), at the start cell's height byte (+0x72).
`G1_2` seed 1001 matches through the fill (5722 draws, 87 edge tiles).

**Region overhang**: the client's region is a window into the full map (`0x141d0bad0` checks
the full map's bounds), so a tile placed with the one cell of slack writes cells and corners into
the margin. `TileMap` keeps those writes and `embed` applies them.

**Room pass ground marking** (`0x141f8e6c0`, eighth argument): when set (overlays, `0x141fa23e0`
passes `1, 1, 9, 0`), uniform-ground slots also give their ground to the corners x in `0..w`, y
in `0..=h` (`0x141d0e130`, which sets flag 1). Node rooms (`0x1420c9970`) pass it unset. The room
pass also registers each room (`0x141faab40`), so overlay rooms count as rooms for
de-repetition.

**Fill rooms on unwalkable corners** (`0x141f91110`): only rooms whose room data +0x188 list is
empty. Entries are 0x30 bytes, which fits the points of interest (x, y, rotation, tag); it is
not the doodad list.

**De-repetition keys**: tiles are looked up by their tile-set index key (`0x141d08a40` turned,
`0x141d09900` drops `.gt` flag 0x10 grounds).

**Result (2026-10-16)**: tile hashes equal the client's for Clearfell (seeds 1, 2), `G1_1`
(1000-1002), `G1_2` (1001-1003, forced topology 15) and `G1_4` (1001, 1002; topology 10,
orientation 2). `G1_4` 1003 needs overlay placement after the fill. `G1_5` fails routing one
edge. The other act 1 areas use `.dgr` layouts (not ported). `area_vectors` tries every forced
topology and orientation when the natural choice (world-map pins) does not match.

**Inheriting tiles** (`0x1429153f0`): a `.tdt` with an `inherits` path (for example
`ZombieIntroFeature_Reorient_1.tdt`) has an override-flags byte after it and carries only what
the flags name:
- 1: the `.tgt`.
- 8: the header from `feature` on (edges, size, grounds, distances), its sub-tile bytes and
  corner points.
- 0x10: size and sub-tile geometry.
- 0x20: the orientation mask.
Everything else comes from the parent (`Tdt::resolve`, `generate::load_tdt`). The orientation
mask (vtable +0x68, data +0x171) is a `u8` after the body flags; 0 or 0xff means all
orientations. The tile index (`0x141d09ab0`) ANDs it with the `.tst` rotation list.
`ZombieIntroFeature.tdt` has mask 3 (o0 and o1 only), so a room that wants it in o7 gets
`_Reorient_1` in o0.

**Edge tiles** (`crates/poe-world/src/edge_tiles.rs`, `0x1420b6950`):
1. Occupancy grid (`0x141ef5700`, 0x20-byte cells: cost, parent, f, g, h, closed, occupied,
   open-set link): cells under room tiles are occupied. Per edge (`0x1420b6660`): +80 cost on
   every tile its line crosses (`0x141ef6ab0`, tiles rounded down to the `.et` size); an edge
   with a fixed from node goes to list A (from that end) and its fixed end tiles are taken
   (`0x141ef67a0`); otherwise a fixed to node puts it in A from the other end; otherwise list B
   (from the from end if it is a dead end).
2. `below(|A|)` (discarded), A shuffled (`uniform(n−k)` per slot), stably sorted by priority
   then split edges first. A then B: each chain (`0x1420b5f30`, walking through degree-2 nodes,
   `0x1420c4960`) is skipped if it stays within the end tiles of two touching rooms
   (`0x1420b41b0`); free dead ends inside the map repeat their position as a point
   (`0x1420b43b0`); the path builder (`0x1420b4510`) keeps one point per tile and cuts loops.
3. Fit (`0x1420b5250`): from the start node's tile, searches (`0x141ef5c80`) from each path
   point back to the previous one: seeds are all later points (cost `avg×100 + points left`,
   the first 0, estimate `isqrt(d²)×100`), steps cost `(avg + jitter + size)×100` with an
   estimate of plain `isqrt(d²)` and `jitter = ((x+1)·r₁ + (y+1)·r₂) mod 707 / 101`, where
   `r = v² + (v+1)²`, `v = below(16384) + 20000` (two per search). The open set is ordered by
   (f, cell). Found tiles are taken (+80); edges of the path get −60 first (`0x141ef6bf0`) and
   each passed segment −20 (`0x141ef7000`); failures backtrack (at most 10 steps). Paths that
   fit cost +80 on their inner tiles; those that do not are retried after the others.

4. Tiles (`0x1420b7f30`), per inner path tile: the sides of the previous and next tile
   (`0x1420b78d0`, room tiles count with their size) get the edge's `.et` and an exit position:
   the middle (`3·size/2`) for `.et` flag 8 (third number); next to a room tile, the room exit
   seen from this side (`0x141f9e990`, falling back to the virtual exit); otherwise which third of
   the side the smoothed path crosses (`0x1420b7ba0`, threshold `23·size/6`), where the crossing
   is a degree-9 de Casteljau midpoint over the tile centres padded with four copies of each end
   (`0x1420b7490`, f32). Grounds (`0x1420b7d70`/`0x1420b7c60`): left/right ground on the two
   corners of each side, swapped by the edge's chirality (`+0x41`) against the path direction;
   a turn's outer corner copies its neighbour. Lookup with no feature, `below(total weight)`.

`.et` data: `+0x48` flags (4 virtual, 8 third number, 0x80 wildcard name), `+0x4b` size,
`+0x4c` first number (the right ground's corner height in tile keys), `+0x4d` hashed crossings
(`0x1421969e0`; 0 for every Riverbank type). The DLL traces tile keys (`tile_key`, with caller)
and dumps these bytes with each graph edge.

**Verified:** all 103 edge tiles (17 paths) equal the client's (position, orientation, tile);
5261 draws in step.

**Corner lattice** (map +0x40 row stride, +0x50 3-byte points `(ground index, height, flags)`
over `(w + 1) × (h + 1)` tile corners, +0x68 the ground table, indices in order of first use):
- `.tdt` body (`0x1429153f0`, version 14): u16 flags (bit 9 cleared before v13, bit 8 before
  v14), a byte (v > 9), hull planes (byte n, then n bytes when n ≥ 2), a byte if flags bit 9,
  u16 sub-tile count; per sub-tile u16 (low byte bits: 4 = no byte map, 1|2 = no byte list),
  a byte, 3 bytes if flags bit 9, the byte map (`0x1429179c0`: header byte with type in bits
  0..2, extra size bytes in 3..4, size in 5..7; types 1..4 carry a compressed payload of that
  size), the byte list (u16 n + n bytes); then per corner point (row-major, own orientation)
  a ground string (flags bit 1) and a height byte (flags bit 0). Read for every non-inheriting
  `.tdt`, with or without a `.tgt`.
- Placing a tile (`0x141d0bad0`, map vtable +0x10) writes each corner (`orient_point`, the cell
  transform with `w − u` instead of `w − 1 − u`): flag 1, its ground if it has one
  (`0x141d0e130`), and its height plus the tile height (flag 2) when the old or new ground is
  missing or has `.gt` flag 1.
- The region driver (`0x1420bec40`) after edge tiles: copy flag 1 to flag 4, border floods
  (`0x1420b94d0`), the lattice pass (`0x1420c0a60`), heights (`0x1420b9820`), `0x141f92ce0`, clear
  flag 4, `0x1420bdae0`, `0x1420be360`, then fill (`0x141f93060`, row by row; a cell's ground
  is its top-left corner's) and de-repetition. The DLL dumps the lattice before each step
  (`lattice <step>` trace lines).

- Border floods (`0x1420b94d0` → `0x1420b9180`, `crates/poe-world/src/lattice.rs`): per placed
  tile (columns, then rows), from each border corner (top and bottom left to right, then left
  and right) the corner's ground floods breadth-first outward (straight on and to both sides) if
  it has `.gt` flag 2, over corners with flag 1 clear (with `.gt` flag 8 also over tile corners
  without a ground), stopped where `EdgeInTheWay` (`0x141fe0810`) sees a tile edge cross the
  segment: the edge's exit (or, for virtual `.et`s, virtual exit) third lies at that cell of the
  tile side (`0x141fdffe0`); corners strictly inside a tile pass only to other inside corners.
- Unwalkable corners, flag 8 (`0x141fe0fd0`): from border corners whose ground is missing,
  has `.gt` flag 4 or is `impassable_ground.gt`, an impassable flood (corners without a ground
  take the impassable ground; corners with a ground lacking flag 4 are marked and later their
  flag-4-less neighbours); then the whole border; impassable grounds inside flood the same way;
  then every inside corner next to a marked one is marked unless its ground has flag 4 (a
  two-deep ring around the Riverbank). Transition tiles (`0x141fe0b40`) and the tile-set option
  `+0x165` are not ported; it fails with "No walkable area in the level" when fewer than
  `max(10, w·h/500)` corners are walkable. This is the walkability base for collision.
- `0x1420b9820`: every corner gets flag 1; ground-less ones take the graph's default ground.

**Verified:** the lattice equals the client's at every step up to the fill (4891 points).

**Fill** (`crates/poe-world/src/fill.rs`):
- `0x141f93060`: rows top to bottom, cells left to right, skipping the rest of a placed tile's
  width. A cell's ground is its top-left corner's; it is "walkable" when that ground is missing
  or has `.gt` flag 4 and the corner has no flag 8.
- `0x141f8f930`: grounds with `.gt` flag 0x10 count as none; the group is the `.gft` section
  named like the ground (`.gt` first line), else `default` (`0x141f93960`; tiles and rooms are
  looked up separately). Without fill rooms: `below(total)` over the tiles, orientation
  (`0x141f90d60`), on failure the entry is dropped and it repeats. With fill rooms: one list,
  tiles then rooms (rooms with doodads only when walkable), same loop with the room orientation
  (`0x141f90f60`). Orientation "any": square → `below(8)` if it fits unrotated; otherwise the
  orientations that fit (0x55 unrotated, 0xaa rotated) and `below(840)` picks among them
  (`0x1421c3d00`). Tiles may hang one cell past the region's far sides (`0x141f9e5f0`); the
  same-neighbour test (`0x141f8f7c0`) looks only at the target cell and never fires. Fill tile
  height: middle of the corner heights (`0x141f90950`). A fill room (`0x141f8c8c0`) places its
  cells x outer, y inner, skips cells already taken, re-picks up to five times until a tile
  fits, writes its ground overrides (`0x141f9e780`) and gives its corners without a ground the
  fill's ground; if its own cell is still empty the fill repeats there.

**Verified:** the Riverbank fill equals the client's (561 pieces, 13 fill rooms); 6393 draws in
step.

**De-repetition** (`0x141fdd7c0`, all rooms including fill rooms): cells row by row, live
(replacements are seen by later cells). A re-pick (`0x141fdfa60`) that finds no candidate and
whose tile is outside every room rectangle (`0x141fde210`) tries moving an edge crossing
(`0x141fdef90`): for each other side with a non-virtual edge and a same-size, featureless
neighbour away from rooms and overrides, every other exit position gives candidates (not
mirroring up or left); `below(total)` picks one, and if the neighbour's key with the matching
exit (`3·size − exit − 1` on the opposite side) has candidates not mirroring any side, both are
replaced (`below` again for the neighbour); otherwise the pick is dropped and it repeats.
Ground overrides are checked at the room's turned corners. The corner ground fix-ups of the
replacement (`0x141fde880`) are not ported.

**Verified:** Riverbank de-repetition equals the client's; 6412 draws in step.

**Outer buffer** (`0x141fd8360`, `crates/poe-world/src/outer_buffer.rs`): the map is the
region plus the master's `OuterBufferSize` (top right bottom left; the Riverbank's `3 3 3 0`
gives a 69×78 map with the region at (0, 3)). Set corners get flag 2; each margin corner takes
the ground and height of the nearest set corner along its column (up/down) and row
(left/right), and margin corners get flag 8. Then the strips: below the region (columns left
to right; the strip shrinks under taken cells), above it (strict `x < x1`), right of it (rows
top to bottom), left of it (rows top to bottom, stacked from x=0 to the border; edges on sides
1/3, exits `[3s, 3s−e−1, 3s, e]`), right of it, in that order (`0x141fd8360` handles sides 0, 2,
3, 1). `OuterBufferSize` (`0x1429285e0`) is top, right, bottom, left: a missing right or bottom
copies the top and a missing left copies the right, so act 1's single values (`6`, `7`, `10`)
mean all four sides. At each step the region's border cell decides: an edge
crossing that cell side (`0x141fd5cd0`; virtual edges give the split type whose exit
crosses) makes an edge-tile key (that edge on the two sides across the strip, exits
`e`/`3s−e−1` from the border tile's exit `0x141d083d0` minus the cell offset, corner grounds
from the border corners (none for `.gt` flag 8), heights relative to their minimum),
`below(total)` picks a tile that is stacked across the margin; otherwise the ground's first fill
tile (`0x141fdd3e0`) is stacked where it fits, unless the two border grounds disagree (then one
cell is skipped). Map ground checks are off during the pass.

**Result (2026-10-15):** Riverbank (`G1_1`) seed 1000 generates identically to the client: tile
hash 138293894, all 5382 cells (tile, orientation, sub-tile) equal, 6414 draws in step. The draws
after that (`0x1415a5005`, …) belong to the doodad/entity stage. With node tiles and inheriting
tiles, `area::generate` also matches seeds 1001 and 1002 (`intro_4`, current-build sweep vectors
in `data/sweep-results.tsv`).

**Result (2026-10-15):** `crates/poe-world` generates Clearfell Encampment identically to the
client (all 990 tiles, orientations and sub-tiles; tile hash equal) for seeds 1 and 2. Not yet
ported: graph edges (routing over the cost grid), several rooms, rotated rooms, fill rooms,
heights, doodads (their hash), the Riverbank's topology choice.
