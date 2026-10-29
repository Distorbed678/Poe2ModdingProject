# Graph-based areas (`.tgr`): generation order and layout

## Generation order, town first

1. `TerrainGen_Run` seeds the RNG with the area seed (after `prepare`'s throwaway seed-0 draws).
2. Topology choice: see [shared-passes.md](shared-passes.md#topology-choice).
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
5. After the room pass (town): 990 × `below(1024)` (`0x1420cdc30`), one `below(0)`
   (`0x1420b698a`), the fill pass (66 placements, `0x141f90664`: weighted `.gft` pick for the
   cell's ground, then orientation), and a pass that re-places 10 key-slot tiles in other
   orientations (`0x141fdec67`). The encampment (15×18, `FI R270`) is placed through the room's
   fixed-tile list (`0x141f9e780`), not the place-tile calls above.
6. Generator trace kinds added for this: `below`/`range` with exact call sites, `candidates`
   (the room pass's candidate list), `place` (`0x141f9e2e0`: x, y, orientation, height, tile).
7. Fill pass: see [shared-passes.md](shared-passes.md#fill).
8. De-repetition: see [shared-passes.md](shared-passes.md#de-repetition).
9. Tile hash: the place function (`0x141d0bad0`) stores each cell's offset from the tile's
   corner at record +0x34/+0x35; only offset (0, 0) cells are hashed.

## Graph layout (Riverbank, `G1_1`)

Per region, `0x141ff2c50` loads the graph, rolls edge existence (`0x141fe9970` →
`0x141feeaf0`: per edge with exist % ≠ 0, `below(100) < exist` keeps it; exist 0 removes it
without a draw), then `0x1420bec40` lays the graph out. Seed 1000 → topology 2 of 5
(`intro_3.tgr`, 66×72 tiles; the map is 69×78 with `OuterBufferSize 3 3 3 0`).

`.tgr` v25 fields (parser `0x142aa4ad0` / `0x142aa5470`, caller `0x142927930`):
- node: x, y, edge count + indices, room type, orientation, descriptions, exist %, height,
  priority, deletion behaviour, lock-orientation flag, random offset (4 ints), then x and y
  constraint flags (read by the caller; constrained axes are not jittered).
- edge: from, to, internal points, exist %, chirality, priority, edge type, min/max width.

**Node jitter** (`0x141ef4720` builds, `0x141ef3bb0` applies; `modules/server/crates/poe-world/src/voronoi.rs`,
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

### Room placement

(`modules/server/crates/poe-world/src/rooms.rs`; trace kinds `node_candidates`, `node_key`,
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

### Repulsion, exits and routing

(`modules/server/crates/poe-world/src/rooms.rs` `repel`, `routing.rs`; DLL
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

### Edge smoothing

(`modules/server/crates/poe-world/src/smoothing.rs`; DLL trace kinds `arc_start`, `arc_end`,
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

### Virtual edges and the last graph passes

(`modules/server/crates/poe-world/src/split.rs`; DLL dumps
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

Node flags (`+0x88` "fixed", `+0x89` "virtual end"): fixed = room nodes and their exit
nodes, plus dead ends pushed off the map; virtual end = next to a virtual edge in the topology
graph (`0x1420c7400`, `.et` flag 4). The virtual split copies both flags to each new node, and
at a chain's far end sets virtual end on both far nodes unless the far node lies on the map
border (the clone's far node also copies fixed).

### Node tiles

(`modules/server/crates/poe-world/src/node_tiles.rs`, `0x1420cade0`): at the start of the tile
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

### Typed nodes without a room, junctions, hashed edges

(found on `G1_2`):
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
- Overlays: see [shared-passes.md](shared-passes.md#overlays-toy).

### Virtual junctions without tiles

(`0x1420c6cd0`): point drops, pair loop with `0x1420c6420`,
ordering `0x1420c6860`, node split `0x1420c5ee0`, and logical links. The split updates the links
(`0x1420baf40`). `0x1420bcff0` resolves them: pairs at identical positions, else
`0x1420bc9a0` with segment distance `0x142196290`. Link nodes of degree >2 join the tile list.

### Edge tiles

(`modules/server/crates/poe-world/src/edge_tiles.rs`, `0x1420b6950`):
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

## Findings from the 50-seed batch (2026-10-22, terrain-b)

Each item was found by comparing the port with client exports or traces of failing batch jobs
(`dumps/.../batch/fails1`, `b1`, `act1_traces`); code in `modules/server/crates/poe-world`.

- **League mechanic rolls before the topology choice** (`phase_cad0` `0x141ffcad0`, `league.rs`).
  Map areas draw `below(100)` for up to four mechanics before `phase_c360` (the topology draw), so
  every later draw is shifted. Mechanic index 0, 2, 9 and 0xb (indices into the client's
  mechanics table, rows of 0xee bytes from `FUN_142659480`) are rolled in that order. The chance
  comes from the area's `WorldAreaLeagueChances` row (`WorldAreas.LeagueChances`; columns at row
  offsets `+8 + 4(c-1)`: index 0 `+0x24` with a second chance at `+0x28`, index 2 `+0x40`,
  index 9 `+0x64`, index 0xb `+0x68`). `0x141e22dd0` rolls one: a chance up to 100 draws once
  (`below(100) < chance`), a larger one adds `chance / 100` and rolls the remainder; then, while
  the second chance is set, it rolls that and keeps going on a hit (up to the mechanic's maximum,
  `+0x8a` of the mechanics row). With success the client adds encounter requests
  (`BrequelExpandingEncounter`, `StoneCircle`, `IncursionEncounter` via `0x141fef180`); their
  effect on the layout is not ported (the traces of MapWoodland and MapDeforestation match
  without it). Skipped by the client when `bVar18`: row flag `+0x14` or `+0x12a`, area kind
  `0xf 0x20 0x21 0x23 0x13` or `0x27`, or a special stat; the port rolls only for `IsMapArea` areas
  (acts show no such draws). Stat conditions of `0x141e22c40` / `0x141e223d0` are skipped (no
  area stats).
- **Roll details and the Incursion request** (2026-10-26, P3_1..P3_6): the chance test is the
  callback `0x142003150`: a chance below 1 fails and one of 100 or more succeeds, both without a
  draw; only 1..99 draw `below(100) < chance` (for a chance above 100 the remainder is tested the
  same way, so a multiple of 100 draws nothing). The rolls are not limited to map areas: `bVar18`
  skips towns (WorldAreas `+0x14` `IsTown`), hideouts (`+0x12a` `IsHideout`), area kinds (column
  45, `+0x1b4`) 0xf, 0x13, 0x20, 0x21, 0x23, 0x27 and areas with the tag hash `0x939d55cf` in
  `Tags` (`+0x133`, not ported). The Act 4 interlude areas (P3_*) have an Incursion chance of 100:
  no draw, one `IncursionEncounter` request (`0x141fef180`), served by `0x1420aacf0` with the room
  set's `ridge_mission_*` rooms like the stone circle; the request's tile is the area's
  `Incursion2EncounterTilePerArea` path (`0x141fef610`), else `SpecialTiles` row 3
  (`DefaultIncursion`, `WaygateDeviceBase.tdt`, `0x141633960`), added to the tile set (a `.tst`
  would add a tile set). In P3_2 the container graph tries the request first (two `below(7)` with
  shuffles, nothing fits), then the subgraph serves it. Ported in `league.rs` (switches
  `POE_OLD_ROLL`, `POE_ROLL_MAPS_ONLY`, `POE_NO_INCURSION`); P3_1..P3_4 and P3_6 match, P3_5 (50)
  differs in its second region's fill (no trace yet).
- **Axis constraint flags turn with the region** (`region.rs`): node values 9 and 10 (x and y
  constraint flags) swap for odd orientations. Without it every area with a constrained node
  (`G1_1` orientation 7, ...) differed from the first repulsion on.
- **Junction explode sees the exits** (`0x1420c9970` cuts the rooms' edges at their exits before
  `0x1420cb200` explodes junctions): the side of a junction edge (`0x1420d1220`) is taken towards
  the first internal point or, without one, the other end, which for a room edge is the exit
  position, not the room node (`PlacedGraph.ends`). This gave `G1_5` "could not tilise edge".
- **`.gt` flags of room-only grounds**: grounds named only by a room (`arenamud.gt`) have their
  real flags in the lattice passes (`TileMap::use_room_grounds`); the default 0x0f let the border
  flood run through them.
- **Post-unique tiles** (`0x141ff2090` per region after the de-repetition, `0x141ff2920`
  chooses an orientation; `postunique.rs`): `.tst` entries whose `.tdt` feature is exactly
  `postunique` are not in the tile-set index (`0x141d09ab0` puts them on a separate list at
  tile-set `+0x80`, with their rotation mask). Per entry: every region cell that is the origin
  of a featureless tile with one corner whose lattice flag 8 is clear, and whose turned index key
  equals the entry's key in some allowed orientation, is a candidate with an orientation drawn by
  `below(n)` among the equal ones; then `below(count)` picks the cell and the entry replaces that
  tile (G1_5's `T2_WeaponDrop_01..10`). No candidate throws.
- **Overlay phase 2** (`0x141fa23e0` with `param_3` set, `overlay.rs`): before the overlay room is
  placed the tiles under it are recorded in a map keyed by `x * 0x10000 + y` (`0x141fa9e40`: sorted
  by x then y, a later insert overwrites) cell by cell over each tile's whole footprint, then
  removed; after the room (`0x141f8e6c0`) every recorded cell in that order that is empty again
  is filled like the fill pass (`0x141f8f930`, ground of its corner, walkable when that ground is
  none or has `.gt` flag 4). A cell now holding `OverlayBlank` (`SpecialTiles` row 2,
  `0x141d08fe0` / `0x141633960`, `Metadata/Terrain/overlay_blank.tdt`) gets its old tile back when
  that was 1x1 with only wildcard edges (`0x141f9e2e0`), otherwise the blank goes and the cell is
  filled; any other tile stays. The overlay context (`0x141fa0ac0`) is built once, the two phases
  run `0x141fa1a70`.
- **Fill rooms, `k` slots of one ground**: the cell first tries the ground's fill group (as the
  fill pass does); when no entry fits it is placed from the tile set by its key like any other
  slot (`G1_4` seed 1003: two failed picks of `below(2)`, `below(1)`, then the key lookup).
- **Routing retries** (`0x1420cdc30`): edges whose search fails keep their reservation and are
  tried again after the others; a round that routes none throws "Could not permute edges".
- **Off-map ends of virtual dead ends** (`0x1420ba340`, run for both ends of every virtual edge by
  the first lambda of `0x1420bc400`, `0x1420da240`): a dead end (one edge) on the map border
  without virtual end points gets them on the border, a random half width `below(max-min+1)+min`
  either side of the node (`split.rs::off_map_end`; corners use both borders with a 2-tile
  correction). The two banks then run off the map instead of meeting in a node that no tile
  fits. The nodes the splits make join the tile list of the tile stage unless fixed, so these dead
  ends are pushed one cell off the map (`MapCrimsonShores`).

## Wave 4 (agent C, 2026-10-23)

Found with the client traces of `batch/b3` and the fails1 exports; code in `modules/server/crates/poe-world`
(branch `terrain-c` of the wave-4 worktree). Effect on the 28,633 jobs of agent C's areas: 8,102 more matches in
the first measurement (after the hideout, settle, fill-group and virtual-node changes), 0 lost.

- **Hideouts seed the RNG disabled.** `TerrainGen_Run` (`0x141ffe940`, call at `0x141ffebd3`) passes
  `disabled = WorldAreas.IsHideout` (row `+0x12a`, schema column `IsHideout`) to `TerrainRng_Seed`: every draw
  returns its lower bound, so a hideout's layout does not depend on the seed (the client's batch hash is the
  same for all 50 seeds). Towns (`IsTown`) are not disabled; `G1_town` uses the batch seed. Ported in
  `area.rs` (`rng.disabled`). 3,750 of the 4,100 hideout/town-class jobs match, their doodad hashes too.
  Still failing: `Design` and `G_login` (unported master/room features), `G3_town`, `HideoutArenaIncursion`
  (`FileGroups` subgraph), `HideoutLimestone` (four 3x3 fill tiles carry height 26 in the port, 0 in the
  client; a trace is requested), `P1_3`, `P2_1`. "Some client hashes vary with quest state": not seen; the
  doodad hash of every matching hideout and town equals the client's.
- **Master paths starting with `Metadata/` are absolute** (`HideoutLight`'s `master.tsi` names its files in
  full).
- **Room settle (`0x1420c9260`, start of `0x1420c9970`).** Before a room (or node tile) is placed its margin
  rectangle is tested: left and top margin `align` where those sides have an edge, right and bottom margin
  the edge's `.et` size; the cells must be free (`0x141d0d880`), no side with a non-wildcard edge may touch the
  map border, and the four corner grounds must agree with the lattice's where it is set (equal, or both
  none/`.gt` flag 8, `0x141d0daf0`/`0x141d02530`). When any test fails the room moves in a square spiral of
  `align`-sized steps (step counts 1,1,2,2,..9, directions through the 9x9 orientation table `0x141f8c430`,
  vectors `0x142f0c608`), first with the margin rectangle, then with the room alone; no place throws
  "Overlapping node tiles". `rooms::settle`. This was the whole `G2_1` failure (rooms next to each other
  after the repulsion; the client moves them apart): 396 of 462 sampled jobs match now.
- **Empty fill groups do not exist.** A `.gft` section whose lines are all commented out (`DefaultHyenas`)
  is not a group: the ground fills from `default`.
- **Virtual-list nodes are placed at the room stage** (`0x1420caa40`, second loop; lists built by
  `0x1420c7400`: `+0x00` virtual, `+0x18` tile, `+0x30` typed, `+0x48` no-edge nodes with points). Nodes
  touching a virtual edge that are not dead ends try `0x1420c87a0` like rooms (rooms of the empty type,
  else tiles); the ones nothing fits are exploded later. `place_node_opt`. Gave `G1_1` (189 jobs) and
  `MapUniqueWildwood`.
- **Loop chains** (`0x1420b5f30`): a chain that ends where it started is split at point `n/2` into two
  halves that share the middle point (the second takes the first edge's far node's other edge), both are
  fitted as loops, the second half's tiles minus its first and last are appended to the first, a closing
  duplicate is dropped and the tiles are made at once (`0x1420b7f30`, crossings from `0x1420b70e0`: ten
  control points round the loop, one crossing per tile). The client reads crossing `i + 1` for the last
  tile's far side, one past the vector; the port uses (0, 0) there (unchecked).
- **Only `(any)` nodes take the quarter turn** (`0x1420c87a0` tests the node orientation byte against 8):
  `(best)` (9) and fixed orientations run the side check `0x1420b8b70` instead. `MapInferno` boss clearing.
- Open: the quarter turn's bottom border test is `h + x == map width` in the client (a typo for
  `y == map height`; not copied, nothing needed it yet). Room-less nodes in the tile list also try rooms
  of the empty type first (`G2_1` topology 4: `stromacliff_join*.arm` for a node whose edges are
  `desert_dune_stromatolite.et` and `desert_dune.et`; `MapSinterRift`): `node_tiles::choose` only has the
  tile branch. A draft of the edge adjustment for tile nodes is in `runtime/wave4/c/adjust-tile-nodes.patch`
  (did not help alone).
- **Two virtual edges side by side at a tile-less virtual node** (`0x1420c64f0`, from `0x1420c6cd0`): one width
  `below(max-min+1)+min` from the first edge's range, the corner of the two offset lines (candidate chosen by
  the side tests against the midpoint and the third edge) goes to both edges as one virtual end point
  (`node_tiles::virtual_corner`). `MapBackwash` 783 jobs (33 of 36 sampled now match), `MapSinterRift`.
- Measured on the 41,464-job gate (agent C's 28,633 failing/area jobs plus the 15,000 match sample, commit
  7a54698): 11,311 gained, 0 lost.
- **No rectangle overlap test at room choice** (`0x1420c87a0` tests the map, which is still empty then, and
  `0x1420c4b30`, a neighbour-exit check that needs placed tiles): rooms may overlap after the draw, the settle
  spiral separates them. Dropping the port's overlap test gave `G2_1` another 313 jobs.
- **Nodes made by the virtual-junction split are not in the tile list** (the list was built from the original
  nodes at the start of the tile stage). `G2_1` topology 4 and `MapSinterRift`.
- Gate (agent C's failing jobs plus the 15,000 match sample, 41,464 jobs, bd26b25): 11,705 gained, 0 lost.
- Tried and rejected: the levelling rounds of `fill_black` (`0x141fd71c0`) with the lower middle
  (`min + (max-min)/2`) instead of the upper one: `HideoutLimestone` (50 jobs, four fill tiles at height 26
  instead of 0) would match, but 3,929 other matches break. The cause there is elsewhere.
- Open for `MapCrimsonShores` topology 2 (400 jobs): "no edge tile to expand the outer buffer" with a key
  whose exits are 1 and 7 (a slanted crossing); needs a client trace (requested: runtime/wave4/c/client-requests.txt).

## Wave 5 (agent C, 2026-10-23)

Branch `terrain5-c` (commit 6c78b7a in the Poe2Server worktree `runtime/wt/c`). Gate on 37,177 client layouts (agent C's wrong areas, the 15,000 match sample, the Expedition/G_login/Design/P2_1/G3_town jobs): 488 gained, 0 lost.

- **Node tiles go through the room settle** (`0x1420c9970` start, `0x1420c9260`): `node_tiles::place` now runs `rooms::settle` before placing a tile, as rooms do (margin rectangle, border, lattice grounds, square spiral). `MapCrimsonShores` topology 2: the tile of a three-edge node sat where the client moves it by (-3, +3) tiles (381 jobs gained).
- **Dead ends made by the virtual-junction split** that lie near the map border are pushed off the map like other dead ends (they stay out of the tile list otherwise; `area.rs`, `junction_made`).
- **`.tst` `include "<file>"` lines** (G3_town: `tiles_waterlowered.tst` includes `tiles_walkable.tst`, `tiles.tst` includes `tiles_unwalkable.tst`) were ignored; the included entries come first (`generate::collect_tst`). G3_town now generates but its hash still differs (trace requested).
- **Room slot height (`+0x72`)**: the lowest corner height moves into the tile height for node rooms too (it was done only for overlay rooms). `P2_1` "no tile for room cell ... ForgeTransition" (keys with heights `[1,1,1,1]`) and `Design` no longer fail there.
- **Lock-orientation flag** (node value 4): a node with it keeps its orientation; the region's turn does not compose with it (`region.rs`). The `rune` room of `ExpeditionLogBook_Heath` (region orientation 6): doodads were mirrored (45 jobs gained).
- `G_login`: a master with `TileSet` and no `FillTiles` is valid (50 jobs gained).
- Draft from wave 4 applied: tile-list nodes adjust a key no tile has (`rooms::adjust_sides` on an `EdgeView`). Not enough for `MapSinterRift` (node 37 has two edges whose tile needs a middle edge type `desert_dune_titan_low_unwalkable.et`; no tile has both edges directly and the room set has no empty-type room) or `Design` node 6 (three edges); a client trace of `MapSinterRift 1081616819 1 0` is requested.
- **Sealed tiles (found, not ported)**: `0x141d053a0` sets tile data `+0x16f` bit 0 when one of the tile's objects (`.tmd`, last quoted string of a line) has a type with an `AreaTransition` component or an `InteractionAction` whose `action_name` is `waypoint` (`0x141e71ce0`); only 7+1 `.tmd` files name such types, so it cannot explain `MapSlash`/`MapSeepage`. Bit 1 is the `Spawner` one already ported.
- **HideoutLimestone** (50): four 3x3 fill tiles at inner (2,20), (5,20), (8,20), (11,20) were already placed by the dungeon fill at height 0; the outer fill's levelling rounds (`0x141fd5ff0` loop with `0x141fd71c0`, applied by `0x141d0b390`, which also rewrites the tile's lattice corner heights) lift them to 1 in the port, the client keeps 0. The lattice at stage `92ce0` is identical in both, so the corner heights seen by the rounds differ (check whether placing a tile in the fill pass writes its corners' heights to the lattice).

### Wave 5 (agent C, continuation): league mechanic rooms, tile-list nodes, open items

- **League mechanic rooms (the big one: 1,139 of the 1,142 failing jobs of MapCrimsonShores, MapBackwash, MapWoodland, MapAridPlains, MapWetlands, MapSinterRift, MapInferno, MapDeforestation, MapSlash, MapCaldera; the same mechanism sits in every map area, about 3 to 6 percent of the seeds).** The failing seeds were the ones where the roll of mechanic 9 in `phase_cad0` (`0x141ffcad0`) succeeds: that mechanic adds the `ExtraTerrainFeatures` row `StoneCircle` (room `Metadata/Terrain/Gallows/Leagues/StoneCircle/Tiles/StoneCircle_01.arm`, tiles `StoneCircle_01..03.tdt`, orientation list `[8]` = any) through `FUN_141fef180` (count 1; more than 1 only for a chance above 100, then `+0x18c` counts the rest, not ported).
  - **The roll columns were off by one**: the row struct offset of column `c` is `4c + 4`, so mechanic 9's chance `+0x64` is `WorldAreaLeagueChances` column 24 (4 or 6 percent on maps) and mechanic 0xb's `+0x68` is column 25 (0 on maps). `league.rs` read 25 and 26, so the number of draws was right (one `below(100)` either way) but a map's last draw always "succeeded" at chance 100 and a real hit (draw below 4) was missed. `league::Rolled` keeps the counts; `league::extra_rooms` turns a stone circle into the room and its tiles.
  - **Placement** is `0x1420bdae0` (called by the `.tgr` driver `0x1420bec40` right after the lattice passes, before `0x1420be360` (shrines) and the overlays; its trace line is the `lattice bdae0` view followed by its draws): `0x1420bd890` lists the cells that are empty and walkable (corner ground missing, or a ground with `.gt` flags 4 and 8; corner without flag 8), row by row, discards a `below(n)` and shuffles the list with the forward Fisher-Yates (`uniform(n - i)`, swap `i` and `i + k`; the last draw is `uniform(1)`). `0x1420bde20` draws the room's orientation once with `below(8)` (feature orientation list holding 8), then walks the shuffled list: the first cell where the rotated rectangle lies inside the map on free cells (`0x141d0d880`) takes it. The corners under the rectangle whose ground is missing or has `.gt` flag 8 take the cell's ground (`0x141d02530` tests the flag on both grounds, `0x141d0e130` sets it), the room is placed with the cell corner's height (`0x141f8c8c0`; the pre-unique pass passes `0xb`, this one `10` and a feature record from `0x141f8ac80`, which only records the rectangle for spawns, with `0x141d0c630` marking the cells), and the budget `+0x144` drops by one. The room's slot is a 3x3 key without edges, so the tile comes from the feature "StoneCircle" tiles added to the master's tile set (`0x141d09a30` -> `0x141d09ab0`, weight 1, all orientations: 24 entries, `below(24)` picks tile and orientation at once: entry `tile * 8 + orientation`). Ported: `fill::place_league_rooms`, `TileSet::add`, `Master::add_extra_tile`, `Files::league_rooms` (filled by `generate_probed_files`). Per-cell orientation draws were my first attempt and failed exactly when the first shuffled cell does not fit; the trace has one `below(8)` per room.
  - Not done: the same pass for `.dgr` areas (their extra features go through the room set's `stonecircle` / `preunique` tags, `dungeon.rs`), the other mechanics (0: `+0x188`, 2: `BrequelExpandingEncounter`, 0xb: `IncursionEncounter`; none rolls on the maps checked), a master with room-set `preunique` rooms in a `.tgr` (`0x1420bd890` + `0x1420be940` after `0x1420be360`; the list is `+0x148` of the room set, built by `0x141f99b20` from rooms tagged `preunique`), the `+0x1b4 != 0x27` area-kind condition around the roll.
- **Nodes made by the virtual-junction split keep their "not in the tile list" mark across `resolve_links`**: the link resolution erases emptied nodes and shifts the later indices, the marks were kept by old index. `MapSinterRift` (57 jobs): node 37 (a plain dune edge joined to a split bank of the ledge virtual edge, two different types) was tiled and had no tile; the client never tiles it (its tile list holds only the 3-edge nodes).
- **Limestone** (50 jobs, open): the rounds of `0x141fd5ff0` are as ported (checked against the decompile: list of cells whose tile handle is the outer ground's first fill tile, corner heights with the `-0x78` wrap, upper middle, apply only at offset (0,0), at most 15 rounds, stop after a round without a change above 1). With the rounds off the port matches the client's hash, so the client does not lift the four tiles (corner heights `{0, 1, 0, 0}` seen at (2,20); the room's last point row holds the 1s). Excluding the cells of tiles placed before the outer fill from the list makes Limestone match but loses 2,999 of 12,001 matching jobs of the sample: they belong in the list. A trace hook is requested (`runtime/wave5/c/client-requests.txt`, `round_corner` at `0x141fd71eb`).
- **P2_1** (39 jobs): every tile of the client sits one storey (16) above the port's: the base height is 1 (the first edge tile at (54,21) has height 16 where the port has 0; lattice heights differ from the `heights` step on). The port has no base height for this graph (all node heights are 0); where it comes from (`+0x146` / `+0x144` of the master and `0x1420b9820`'s two ring heights, `0x1420bec40` passes `iVar6`, `iVar20`) is not found yet.
- **G3_town** (50 jobs): the lattice at `92ce0` already differs (the client has `black_inside_wall.gt` on the top row from x 37 where the port has no ground; 4,731 points), so the ground-only slots / room grounds of this `.dgr` differ before any fill.
- **Design** (50 jobs, open): a trace of `Design 1160401208 1 0` is requested; node 6 (three edges, `I`) has no tile in the identity orientation for its geometric sides `[3, 0, 2]`.

### Wave 5 (agent C, third run): league rooms in `.dgr` areas, Limestone

- **Full gate of d87aebd** (`gate1-in.tsv`, 37,177 jobs): +1,235 gained, 0 lost vs `forced-baseline.tsv`; match-sample 15,000/15,000.
- **Where the league features go in a `.dgr`** (generator object `+0x48` holds the feature rows, `+0x144` the budget = list size for a one-graph job (`0x141f8bb60`, share from master `+0x152` via `0x141ffd3a0`), `+0x148` cap). Master offsets: `+0x1f8` RoomSet, `+0x200` Overlays, `+0x218` FillTiles (not overlays), `+0x230` OuterGroundType. Two mechanisms, chosen by the room set:
  1. **No mission rooms** (`.rs` has no `mission` tag): the feature pass `0x14209a620` -> `0x14209a880` -> `0x14209aca0`, inside the `0x14209c4c0` block (gate: master has FillTiles), before the injectors (`0x14209b310`) and the pre-unique room. Same as the `.tgr` pass `0x1420bdae0` with: cell list = the ground-only cells (shuffled: discarded `below(n)` + forward Fisher-Yates), cell must be accessible, its ground and the lattice corner ground missing or with `.gt` flags 4 and 8, rectangle in bounds and free (`0x141d0d880`), pass 0 only: squared rectangle distance (`0x14209a540`) > 16 to rooms already placed by the pass; pass 1 retries leftovers without the check. Ported: `fill::place_league_rooms_dgr` (MapRustbowl, MapSteamingSprings full).
  2. **Room set with mission rooms** (mission table = room set `+0x130`, key = number of `missiontile` slots, filtered to rooms whose `.arm` side length equals RoomUnitSize): `0x14209a620` removes the non-priority features (column 7 of `ExtraTerrainFeatures` = `+0x60`, false for StoneCircle) from its list (flag from `+0x130 != +0x138` and master `+0x15a == 0`), and the room assignment step `0x1420aacf0` (after the special rooms `0x1420aa710`, before `0x1420abf60` shrines and the second filler pass) handles them: a 3x3 feature `.arm` is queued as a request (`0x1420abb90`); a feature `.arm` with side length = unit size is instead put over a placed room by `0x1420a7da0` (not ported, errors). Requests are served by mission rooms put over placed rooms (`0x1420a7da0`, port `place_special`) in a snapshot/retry loop over the table entries (groups of the entry, then smaller keys from the back, then every entry from the front); each served request draws `below(0x348)` (orientation via `0x1421c3d00` = `region::pick(0, v)`) and joins the generator's pending list (`0x141f8ab50`), the feature leaves the list (`0x141f8b320`, budget-1). The tile pass (`0x1420983d0`) hosts them: a `missiontile` slot with no tile for its key picks `below(n)` among the pending rooms whose oriented size equals the slot (`0x141f8b470`), sets the 3x3 lattice heights/flag 2, and places the room (`0x141f8c8c0`, height = slot height, ground = lattice ground at the slot). Ported: `dungeon::extra_features`, `Grid::pending`, `TilePass::host` / `fill::place_hosted`, league tiles added to the master right after load (client adds them with the feature, `0x141f8aed0`).
  - Measured on the 3,716 stone-circle jobs of the batch (`runtime/wave5/c/sc-all-in.tsv`, built with the `league_filter` example): 2,315 match (from 0), 0 lost. Remaining: MapCrypt 169 of 196, MapHeadland 127/139, MapSpiderWoods, MapHiddenGrotto, MapFrozenFalls (all fail), MapOrbalasCrossing, MapBurialBog, MapSwampTower, MapJadeCitadel (all fail), MapDerelictMansion (the "feature room ... would be placed as a room" error: container graph with side length = unit size case), MapEpitaph/MapMire/MapDeserted/MapCreek partial. Causes not examined: other mechanics (counts of mechanic 0 and 2 = Brequel/others), the multi-graph budget share, requests > 1, `0x1420aab60` priority rooms, hosting details (cell attribute bytes, `0x141d0c630`).
- **HideoutLimestone** (client trace w5-c-2): the client never calls the levelling step `0x141fd71c0` for it (hooks `round_corner` and function start silent) while the port lifts four 3x3 tiles; rounds off in the port (`repair` false) match all 4 tested jobs. Hideouts are the only other dungeons with these outer-fill passes and all match with rounds on, so the condition that turns the rounds off for Limestone is still unknown (conditions in `0x141fd5ff0`: job+8 flag, outer ground flags bit 0 clear, master `+0x161` clear; the cell list compares tile handles `cell+8 == local_400`, the outer ground's first fill tile from `0x141f93cc0`).

## Wave 6 (agent C, 2026-10-24)

Branch `terrain6-c` (worktree `runtime/wt/c`). League stone-circle jobs (3,716 of the batch, `runtime/wave5/c/sc-all-in.tsv`): 1,401 failures before; the first fix alone gave +807 (0 lost).

- **Exact-fit candidate list is sorted by placed-room index** (`0x1420cf1d0`, a lower-bound insert keyed by the room index; `0x1420a7da0` builds the list of rooms a mission/special room fits). The port kept discovery order, so the `below(count)` pick (`0x1420a83a8`) took another entry than the client's (client `n=1` variants where the port had 2). `dungeon::place_special`. MapCrypt 169, MapHeadland 127, MapHiddenGrotto, MapEpitaph, MapMire, MapPit ... matched afterwards. Found by `area_trace` (first divergence at `0x1420a83c2`).
- **Mission rooms count as overlay "feature" rooms.** The overlay occupancy (`overlay.rs`, bits 4 and 8 over the room rectangle) must include a placed `mission` room (`special` true, `.arm` tag `mission`); only other `special` rooms stay out. MapCreek: the client never calls the candidate fit (`0x141fa1830`) for positions overlapping `mission_02`, the port did (`ofit` lines, `runtime/wave6/c/ofitcmp.py` compares our `POE_DBG_OVERLAY` FIT lines with the trace). Measured on the leftover SC jobs of MapCreek, MapOutlands, MapArroyo, MapCliffside, MapSpiderWoods, MapBurialBog: 185 gained, 0 lost (`area.rs`, `RoomRect::feature`).
- **The generator's feature list is shared by all graphs of a job** (SwampTower: five subgraphs placed the one stone circle five times). `0x141f8bb60` (called per graph from `0x141ff05f0` / `0x141ff2c50`, share = master `+0x152` read by `0x141ffd3a0`; no master sets it, so total 0 and the budget is the whole remaining list) hands each graph what is left; a placed or queued feature leaves the list (`0x141f8b320`). Ported as `Files::take_league` (called by `place_league_rooms`, `place_league_rooms_dgr`, and `extra_features` for each served request). The league tiles are added to a graph's master from the list as it stands before the layout (`area.rs`, `league` is read before `dungeon::layout`). MapSwampTower 12/12 sampled (0 of 46 before). Graph order: the port's order (container first, subgraphs in node order) worked for SwampTower; the client's job order is not checked.
- **Open: DerelictMansion (59) and CopperCitadel (4)**: error "feature room StoneCircle ... would be placed as a room". The graphs' master has `RoomUnitSize 3` = the `.arm` side length, and the room set has no mission table, so `0x1420abb90` takes its `LAB_1420abce1` branch: the feature `.arm` goes through `0x1420a7da0` as a one-entry special room (mask 0 = any orientation, `local_60 = *mask`), then `0x141f8ac80` records its rectangle, budget -1 and the feature leaves the list. Plan: build a `DungeonRoom` from the feature `.arm` (like the closure in `dungeon::layout`; `DungeonRoom` needs `Clone`), call `place_special(files, &rooms_plus_feature, &[idx], ...)` in `extra_features` instead of the error, `take_league` on success. Tell agent B (dgr code).
- **Hideout rounds (Limestone, 50 + Claimable 50)**: not solved. Findings: `0x141fd5ff0` runs the levelling rounds only when the generator byte `+8` (never written in any decompile searched: constructor zeroes it, `TerrainGen_Run`/`Layout`/`LoadArea` do not set it) is non-zero, the outer ground's `.gt` flag 1 is clear and master `+0x161` (= `OuterFillHeightOverride`, parser struct byte 0xd, value byte 0x1b -> `+0x153`) is clear. Limestone has none of these keys, its ground is the shared `black_inside_wall.gt`, and the client never calls `0x141fd71c0` (hook works in other traces). Rounds off for every hideout (`!rng.disabled`) breaks 14 baseline jobs (Atziri, Doomguard, Darkwood, Yaochi, TencentApocalypse, OlrothArena, BlankCrypt); treating `OverrideMonsterSpawns` like `OverrideSpawns` (plain fill) fixes Limestone and Claimable but breaks 12 areas (BossRush_Area2, Kalguur, G4_10, MapMothersoul, ...). The 25 fill tiles of Limestone come from the dungeon fill (draws `below` `0x141f900a7/e2`), the outer fill's own placements (`0x141fd9d15` ...) draw nothing. StoreyHeight (26 there) is not the cause (BossRush has 18 and needs rounds). Next: a client trace that logs generator `+8` (and the three conditions) at `0x141fd5ff0` entry for Limestone and Darkwood.
- **Height passes** (2026-10-27, Design 50, Design_Lite 50, P2_1 39: all match). The `lattice` trace snapshots are taken at function entry (`borders` = before `0x1420b94d0`, `flood` = before `0x1420c0a60`, `heights` = before `0x1420b9820`). Port: `lattice::height_pass` after the border floods, `lattice::refix_fills` after the overlays' second phase, world units in `area.rs` (switches `POE_NO_HEIGHT_PASS`, `POE_NO_EDGE_FALLBACK`, `POE_NO_REFIX`, `POE_NO_TGR_STOREY`, `POE_TGR_NO_REPAIR`). Decompiles in `runtime/wave6/c/re/` (`c0a60.c`, `c06f0.c`, `c0d90.c`, `h_*.c`).
  - `0x1420c0a60`: (1) row by row, every tile origin not yet reached starts the fix-up flood `0x1420c06f0` (a queue of tile origins). Per tile, `0x1420bfca0` collects the heights its reached corners imply (`0x1420c05a0`: corner height minus the tile's own point height, `0x141d08510`), only at corners whose tile ground is missing or takes heights (`.gt` flag 1); an empty cell takes the reached corner heights of its four corners (`0x1420c0400`). None: the real edges (`.et` `+0x48` bit 7 clear) give the reached neighbour cell at the edge's exit (`exits / 3` along the side): its height minus the tile's point height plus the neighbour's at the shared corner. Still none: the four outer corners, any ground. Several: the lowest (`0x142b8dcb0` is a min-element). `0x1420bfac0` sets the tile (cells and corners, `0x141d0b390`), marks its corners and cells reached and sets the map's raised flag for a non-zero height; `0x1420c3b40` queues the neighbours across corner pairs that take heights (left, right, below, above; an empty cell queues all four) and across real edges. (2) With the raised flag (view `+0x20`): the lowest tile height is subtracted from every corner (all get flag 2) and every tile. (3) `0x1420c0d90`: an unreached corner with exactly one reached neighbour corner takes its height, until nothing changes. (4) `0x1420c0f00` (room heights) is not ported.
  - `0x1420c19d0` (after the fill and the overlays): plain tiles (`0x1420c1650`: not inheriting, edges empty or wildcards, one ground on the four key corners, `.gt` flag 0x10 grounds counting as none) stay unreached, all others count as reached. `0x1420c1c10`, per cell row by row, for a plain tile on a ground without `.gt` flag 1: each neighbour left, right, above and below the cell that is not plain and whose facing key corner has the tile's ground at a position along the tile's side gives its height plus that key corner's height (`.tdt` corner bytes); one distinct value sets the tile. `0x1420c3330`, until nothing changes: an unreached plain tile whose reached neighbours of the same plain ground give one distinct height takes it.
  - Tile records keep heights in world units (`StoreyHeight` per storey; Design's is 43), for `.tgr` maps too, and the outer buffer of a raised `.tgr` map runs with the height repair like a dungeon's.
- **G3_town (50)**: first divergence is the fill-group weight (`0x141f900a7` n=10000 in the port vs 100 in the client) after the first 4,789 identical placements; the port places an extra fill tile at (0,48) of the first subgraph window before (1,0) (the client has it earlier). The `92ce0` snapshot in this container job is taken after the first subgraph was merged, so `latcmp.py` against our container dump is not comparable. Not resolved.
- **Collision grid (`POE_GRID2`)**: not run.
- **MapCaldera (2)**: the lattice at the `fill` snapshot differs in 229 corners in clusters (flag 2 and grounds), the first RNG divergence is the overlay candidate count (`0x141fa249c`); causes not found (`w5-c-2` traces). **Fixed 2026-10-28** (below).

## R1 (2026-10-28): the last failures of both batches
Traces: run `r1-tr1` under `dumps/0.5.5e.hotfix1/terrain/batch/` (the 7 jobs, `POE_HOOK_GEN_TRACE=1`).
`area_trace` now also compares the lattice at the `bdae0` (league rooms) and `be360` (shrines)
checkpoints.
- **League rooms are overlay feature rooms** (MapCaldera, 6 jobs of both batches): rooms the league
  pass places before the overlays (`0x1420bdae0`, e.g. a 3 × 3 `StoneCircle` room) join the
  generator's room list the overlays read: the whole rectangle gets occupancy bit 8, the inner one,
  shrunk by the room's own overlap (`.arm` dimensions `uint1`, 1 here), bit 4. Without it the
  overlay pass of phase 1 found 60 candidates where the client found 50 (`0x141fa261c`). Code:
  `area.rs` `.tgr` path (`POE_NO_LEAGUE_RECTS=1` turns it off).
- **Junction explode drops points within twice its step** (MapCrimsonShores 921210055):
  `0x1420cb200` (hotfix `0x1420cb380`) calls the drop `0x1420c6b80` (hotfix `0x1420c6d00`) with
  radius `2 · s` (`lea r8d, [rsi + rsi]`) and adds the exit point `s` = 23 out. Two junctions 45
  cells apart each added a point; the client's second explode dropped the first one's (27 cells
  from it). Code: `node_tiles::explode` (`POE_EXPLODE_R1=1` restores the old radius).
- **Pre-unique and league rooms are overlay feature rooms on `.dgr` maps too** (batch 3, P1_4
  1187622552): the rooms placed before the overlay context (the league rooms `0x14209a620`, the
  pre-unique room `0x14209c150`) are in the generator's room list the overlays read, marked like
  the `.tgr` league rooms (bit 8, the inner rectangle shrunk by the room's overlap bit 4). Code:
  `area.rs` `.dgr` path (same switch `POE_NO_LEAGUE_RECTS`).
- **Result:** all three client batches match on every layout the client generates: batch 1
  218,705, batch 2 218,802, batch 3 (new seeds, master seed 20261028) 218,678, plus batch 3's
  natural part 8,368 of 8,368 (`forced_natural`).
