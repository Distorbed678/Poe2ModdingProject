# Dungeon layouts (`.dgr`, 0.5.5e)

Most act 1 areas (G1_3, G1_6, G1_7, G1_8, G1_9, G1_11, G1_12, G1_13–15) use `.dgr` topologies
instead of `.tgr`. The topology choice, the graph load and the node-existence rolls are shared
with `.tgr` ([shared-passes.md](shared-passes.md)); after them the client runs a separate dungeon
layout. Port: `modules/server/crates/poe-world/src/dungeon.rs` (layout) and `dungeon_tiles.rs` (tile pass). Which
areas match the client and what is still missing: [../../status/terrain.md](../../status/terrain.md).
The tools (`area_trace`, `area_vectors`, `wiggle_check`, debug switches) are listed in
[README.md](README.md#tools).

## Call chain

- `0x141ff05f0` (`.dgr` subgraph driver, per subgraph index):
  - loads the graph (`0x141fa9590`), runs the pruning/existence passes (`0x141fea2e0`,
    `0x141fea420`, `0x141ff03e0`, `0x141fea890`, `0x141fe8d70`), `0x141fff900`;
  - region size: graph width × (master +0x149 − master +0x14b) + master +0x14b, same for height.
    Master +0x149 is `RoomUnitSize` (G1_3: 9), +0x14b the shared border, the `.tsi` key
    `RoomOverlap` (default 1; G1_3 has no such key, G1_6/7/8/9, `G1_13_1` and `G3_5` say 1,
    G1_12 (unit 29) and `G2_1` (unit 3) say 0). G1_3 (5×6 units): 41×49 tiles, 51×59 with
    `OuterBufferSize 5`. The master parser is `0x1429285e0`: it compares the key text, parses the
    number into a parse struct (`RoomUnitSize` -> byte +0x1c, `RoomOverlap` -> byte +0x1e, from
    the `.tsi` key's own value, not from a room's `.arm`) whose constructor `0x14292e190` sets
    the defaults (the dword at +0x1c is 0x10000: unit 0, overlap 1); `0x141f941d0` copies the
    parse struct into the master (+0x149 / +0x14b). The port reads the key in `area.rs` and
    passes it to `dungeon::layout`; before 2026-10-22 it took the first room's `.arm`
    dimension, which was 1 for every tested area and wrong for G1_12 (map 169×85 instead of
    174×87; with the key the `G1_12` seed 1001 layout at `2:7` equals the client's in all 15138
    cells and the tile hash 3369045138 matches);
  - builds the dungeon context (`0x1420ad1a0`) and the **dungeon grid** (0x70 bytes):
    +0x20 generator, +0x28/+0x2c width/height in units, +0x30 room unit size, +0x34 border,
    +0x38 failed flag, +0x40 `u32` state per unit cell, +0x58 placed rooms (0x60 bytes each);
  - `0x1420ac3e0` lays the dungeon out; then per placed room `0x141ff12d0`; then the tile pass
    `0x142099c80` + `0x14209c4c0` (tile placements at `0x142098dcc` and `0x14209f9a7`); then
    `0x142000340`. Errors: "Dungeon generation failed: …", "Subgraph … extends outside the
    TileMap!".
- `0x1420ac3e0` (layout):
  1. a per-node lambda (`0x1434e9c80`), stable sort `0x141fa8a30`, `0x1420ad420` (calls
     `0x141fa8710` per entry; with master +0x248 set, marks the grid's border cells state 1);
  2. `0x1420aa500`: room placement (below);
  3. `0x1420a9860`;
  4. when any edge has content: `0x1420a5e50` (`below(128)` at `0x1420a5f8a`), `0x141fa8b60`,
     `0x1420a2c70` (corridor routing, lambda `0x143505ad8`), `0x1420a50e0`, `0x1420a99e0`,
     `0x1420a9f10`;
  5. `0x1420a9590`, `0x1420b19f0`; with master +0x1f8 (extra rooms): `0x1420aa710`, per
     `m_ExtraRooms` entry `0x1420aab60`, then `0x1420aacf0`, `0x1420abf60`.
- `0x1420aa500`: splits the graph nodes into two lists (lambda `0x143505950`), sorts both
  (`0x1420d0e00`) and places each node with `0x1420af7d0`, first list first.
- `0x1420af7d0` (place one node; skipped when node +0x88 is set):
  - node +0x58..+0x78: one edge per side; node +0x4c/+0x50 the position, scaled to the grid
    (`pos × grid size / graph size`);
  - tagged `template`: `0x1420b04e0`;
  - no room type (+0x30): `0x1420afdd0`;
  - with a room type: the room pick `0x1420ae750`, the edge pick `0x1420966f0`, then
    `0x1420a73d0` (stamps the room into the grid).
- `0x1420ae750` (room pick): candidates for the type and orientation (`0x1420ad6b0`, else
  `0x141f8b740`), filtered by `0x1420d1110`, then `below(total)` (`0x1420aebc9`), placed with
  `0x1420aefc0`; when `0x1420b1550` asks, a second `below(total)` re-pick.
- `0x1420966f0` (edge pick): the node's side edges weighted by edge +0x54 (`below(sum)`,
  `0x142096813`; two weight-100 edges give `below(200)`); returns the edge's +0x70 resource.

## File fields (`0x14292e8e0`)

- Header after the three ground lines: `Default%:` branching, extra door, fuzzy %; version > 22: a
  bool `open_extra_doors_into_impenetrable_rooms`.
- Node: the `.tgr` fields (`0x142aa4ad0`: x, y, edge indices, type, orientation, descriptions,
  exist %, height, priority, deletion behaviour, lock orientation, random offset ×4), then void
  (`N`/`Y`) and branching (0/1). The builder `0x142096220` keeps type at node +0x20, orientation
  +0x40, priority +0x42 (sort key), exist % +0x44, height +0x48, void +0x88, branching +0x89.
- Edge: the `.tgr` fields (`0x142aa5470`: from, to, points, exist %, chirality, priority, type),
  then branching chance2, chance1, depth limit, branching type, extra door %, extra door type
  (default `Metadata/Terrain/Dungeon/rooms.et`), fuzzy %, impenetrability, impenetrable value
  (`P`/…), void, accessibility. The builder `0x1420962d0` stores: +0x30 exist %, +0x38 type,
  +0x40 priority, +0x41 chirality, +0x48 chance1, +0x4c chance2, +0x50 depth limit, +0x54 extra
  door %, +0x58 fuzzy %, +0x5c impenetrability, +0x60 impenetrable, +0x61 void, +0x62
  accessible, +0x68 branching type, +0x70 extra door type.
- Node side slots (+0x58..+0x78): `0x141d1c580` gives each edge, in file order, the first free
  slot of both its nodes (so a node's slots are its edges by edge index).

## Rooms (`RoomSet`, `0x141f9aa30` / `0x141f99b20` / `0x141f98bb0`)

- The room type is the `.arm` tag (the quoted string after the sizes; `specialchance N` /
  `special…` prefixes mark special rooms). Shrine, mission, preunique and injector types go to
  separate lists.
- `.arm` exit data (`0x142931cf0`): 2×(units X + units Y) entries around the room, each a door
  type (string index, 0 = none), a height, (v>29) a height override flag, and for chiral door
  types a chirality and (v>29) its override. Order: top left→right, right top→bottom, bottom
  right→left, left bottom→top (`0x1420af280`).
- Per type, every orientation the `.rs` entry allows becomes a candidate (0x60 bytes: room,
  orientation, units W×H turned, exits turned, `.rs` weight). Turning (`0x141f964f0`,
  `0x141d00fd0`): index i → (i' + A[o]·h + B[o]·w) mod 2(w+h), with i' = 2(w+h)−1−i for
  mirrored orientations (4..7), A = [0,1,1,2,0,1,1,2], B = [0,0,1,1,1,1,2,2]; mirroring flips
  chirality on chiral door types.
- `GetPossibleRooms(type, exits, n)` (`0x141f9b8f0`, filter `0x141f9b7a0`): candidates whose exit
  multiset (door type, height, chirality) contains the node's required exits.
  `GetExactRooms` (`0x141f9b130`) serves `(best)` nodes; `GetFeatureRooms` (`0x141f8b740`) is the
  fallback (all 8 orientations, weight 1).

## Typed nodes (`0x1420ae750`)

1. Required exits: per side slot, the edge's type, max(height of its nodes) − node height, and
   chirality (`0x141f96730`: (edge from == node) xor edge chirality, chiral door types only).
2. Candidates as above; each is kept when the node orientation is `(any)`/`(best)` or equal to
   the candidate's, and the room fits (`0x1420af280`): its unit cells are free (state ≤ 1) and at
   least as many of its doors lead into the grid as the node has edges. A door facing a placed
   room whose matching door differs marks the candidate; marked candidates are dropped when
   unmarked ones remain.
3. `below(total)` (`0x1420aebc9`). Origin (`0x1420aefc0`): the node's unit cell minus the room's
   extent on mirrored axes (region orientation table), clamped into the grid. If a room next to
   that rectangle is the same `.arm` (`0x1420b1550`), one more `below(total)`.
4. `0x1420966f0`: the node's side slots weighted by extra door % (`below(sum)`, `0x142096813`),
   giving the extra door type.
5. Stamp (`0x1420a73d0`, `0x1420a6fb0`): grid cells = room index + 2; the placed record keeps
   origin, size, orientation and the exits turned back through the inverse orientation
   (`0x142f087d0`).

Nodes are placed typed first, then typeless (`0x1420aa500`), each list stable-sorted by priority
(descending).

## Trace landmarks (G1_3 seed 1001)

The client used topology 2 (`mudburrows_NW.dgr`) with orientation 4: the placed rooms (entrance
1×2 at unit (4,4), boss 2×2 at (0,0), `nospecial` at (2,0) and (0,2)) and the draws (room picks
1, 4, 1, 4; extra door sums 200 each) fit only that.

The draws match the `.tgr` path for the first 11 (existence rolls `0x141feeb10`); the dungeon
starts at draw 11 (`0x1420aebc9`). Later call sites: `0x1420aff12` (`below(2964)`),
`0x1420a62b0`, `0x1420a5f8a`, `0x141fa81a3`, `0x1420b125c`/`0x1420b134c`, `0x1420a4e94`/
`0x1420a4ec7`, `0x1420a9b9f`, then the tile pass (`0x142098d18`, `0x14209f8b8`), fill, outer
buffer and de-repetition as for `.tgr` ([shared-passes.md](shared-passes.md)).

## After the rooms (`0x1420ac3e0` step 4 onwards)

- Corridors (`0x1420a5e50`, `0x1420a2c70`): one record per placed room with a `below(128)` cost;
  edges batched by priority and container (edge types with one room size / other / default),
  shuffled (a discarded `below(n)` then a forward Fisher-Yates); placeholders fill free cells;
  each edge is joined by A* over rooms (`0x1420a4330`: g += isqrt(centre distance in half units)/2,
  h = (dist/2)(100 − fuzzy) + (impenetrability << 7)/100 + fuzzy·cost, open set by (g+h, id)).
  Unused placeholders are compacted away (swap with the last room, ids remapped).
- Branching (`0x1420a50e0` / `0x1420a5680`): per corridor key, candidates next to corridor rooms;
  count ⌊c·n/100⌋ + (below(100) < remainder); depth rounds grouped by chance2.
- Extra doors (`0x1420a99e0`): per room with an extra door %, shuffled candidate positions,
  `below(100) < pct`, pct −= 100.
- Placeholder rooms (`0x1420b0fe0`): size-bucket entries of non-unique generic rooms × allowed
  orientations, matched strict / optional extras / any; weight (empty²·10+1)·w; orientation
  `region::pick(mask, below(840))`; one re-pick when a neighbour uses the same room.
- `0x1420a9590` (open door counts) and `0x1420b19f0` (ground spreading) draw nothing.
- Special rooms (`0x1420aa710`): `.arm` tags `special` / `specialN` / `specialchance N` (room data
  +8 flag, +4 list index, 0xff = chance list, +5 chance, default 100). The room set keeps one list
  per index (`.rs` order) and appends the chance list last. Indexed lists: a discarded `below(n)`
  and a shuffle, then the first entry that places. Chance list: every room with chance ≥ 100, or
  1..99 with `below(100) < chance`. Placement (`0x1420a7da0`): reachable placed rooms with an
  `.arm` that is neither special nor tagged, whose doors match the special room in its first
  matching allowed orientation (`0x1420a7c40`: equal doors, or closed where the special door is
  optional; with optional special doors the room needs one of its own); `below(count)` picks the
  room, `below(weight)` the variant. The room keeps its doors and becomes kind 8.

## Tile pass (`0x14209c4c0`)

1. Pass A (`0x1420983d0`), per placed room in order: the `.arm` grid row by row (y outer).
   Slot anchor +0x73 (`origin`: SW 0, SE 1, NE 2, NW 3) moves a multi-cell slot's listed cell to
   its corner; position and key are turned by the room's orientation; the lowest corner height
   moves into the tile height (slot +0x72). Ground-only slots (no feature, edges or heights,
   one ground; `s` slots and `f` without a fill name too) only mark their corners' height and are
   listed for the fill (`0x142097cb0`). Odd-sized slots centred on the room's border line
   (`0x142099b90`) go to the border map. Others: tile-set lookup, `below(total)`, place, then
   `0x141d0b390` (cells and corners take the height).
2. Border map (`0x142097f80`, sorted by x·65536 + y): the first room's key is stored with the
   sides it covers (4 top, 0x10 right, 2 bottom, 8 left; bit 0 reachable); the second is merged
   (`0x1420972f0`): corners it covers fully take its ground (none for `.gt` flag 0x10) and height,
   sides only it covers take its edge, shared sides are combined by the `.ecf`
   (`0x1420a28c0`: edge pair + corner height difference → edge; no rule: a lone edge stays, two
   give none; exits kept when equal, else the middle). Heights are rebased to their minimum.
3. Pass B (`0x14209fbc0`), x-major: ground-only keys without names are marked. A partial overlap
   with placed tiles is an error; over a placed tile its key is merged in for the sides no room
   gave (0x1c↔0x1a, 0x16↔0x0e, else 0x1e), heights rebased; otherwise sides seen by one room drop
   their flag-0x10 grounds. Named slots draw a name by its tiles' total weight (1 without tiles)
   until one places (a 1×1 `door` becomes a door group cell). Then a uniform key is marked, else
   `0x14209f050` with no name (seen |= 0x20, door bookkeeping). `0x14209f050`: lookup; without
   tiles, sides with a non-virtual, non-centred edge whose exit is off the middle (size·3/2)
   and that face another border entry (any side with `.tsi` `AllowInteriorOverlapFixes`) take
   the middle exit one by one until tiles exist (`0x14209fa10`); after placing, `0x14209ec70`
   gives the cell across each moved side the middle exit too: a border entry still to come in its
   key, a placed tile with the same edge and side length by drawing it again. The height
   fallback (`0x141d05560` / `0x14209fb00`) is not ported.
4. Doors (only with `door` tiles): open exits mark their unit's middle cells on the border line
   (`0x142096ad0`); straight passages across a marked shared exit become door groups
   (`0x142097060`). `0x14209d180` (gated by master +0x38), per group without a `door` tile: the
   group runs vertically when its first two cells share x or every cell has both side markers
   (0x18), else horizontally; the anchor is the first cell with the lowest coordinate along it.
   Door sizes for the anchor tile's edges (`0x141d09700`: per feature, keyed by the 4 edges, a
   sorted set of oriented sizes, `0x141d09ab0`) that fit the group (len × 1 or 1 × len); a
   discarded `below(n)` and a shuffle; the first size whose key (size set, exits of 3 replaced by
   size·3, virtual exits [w·3, h·3, w·3, h·3]) has `door` tiles wins. Offset along the group: for
   a 1-wide door with an odd unit the cell whose coordinate mod (unit − overlap) is unit / 2, else
   `below(len − span + 1)`; `below(total)`, placed at the cell's height over the old tile.
   `0x14209dc90` (3×3 spots) is ported since wave 4 (agent B section below).
   `DisableDoors` (`.tsi`, master +0x167) skips the door placement.
5. Room ground overrides (`0x141f9e780`), then the fill: the listed ground-only cells, stable
   sorted by (x, y), each filled with its ground and its slot height (`0x14209cf50` →
   `0x141f8f930`; `0x141f90950` uses a passed height unless it is `i32::MIN`, which the `.tgr`
   fill passes).
   Exit wiggle (`0x1420a1f70`, after the fill): pass A records square unnamed slots whose edges
   all have the slot's size and one has `.et` random wiggle (fourth number) ≥ 2, or ≥ 1 on the
   room's border line (`0x142098b18`); the fill's listener (`0x14209a2c0`) records fill tiles
   the same way (wiggle ≥ 1). The set is ordered by x·65536 + y; a discarded `below(n)` and a
   shuffle; per position whose corner cell still holds such a tile, a discarded `below(4)` and a
   shuffled side order; each side with a wiggle edge takes the hashed exit (`0x1421969e0`) when
   it differs, if tiles exist for the key (grounds with `.gt` flag 0x10 left out) and for the
   tile across (its opposite exit `size·3 − exit − 1`), else it is put back. Then the tile and
   each accepted neighbour are drawn again (`below(total)`) and placed. Positions may lie one
   cell outside the region (overhanging tiles).
6. De-repetition and outer buffer as for `.tgr`. The outer buffer first fills the region's
   empty cells (grid cells without a room) with the `OuterGroundType`'s first fill tile
   (`0x141fd5ff0`). For grounds without `.gt` flag 1 it repairs heights: new tiles take the middle
   of their set corner heights (cells without any wait for neighbours), then those tiles and the
   fill's tiles of that ground are levelled to their corners' middle, again while one moved by
   more than 1 (up to 15 rounds).
7. Cell heights are stored in world units: storeys × `StoreyHeight` (`.tsi`).

## Later findings

- Region orientation table `0x142f020c8`: see [shared-passes.md](shared-passes.md#region-orientation).
- `.dgr` node random offsets (`0x141fea2e0` → `0x141fee830`, before the existence rolls): node
  values 5..9 are x min, y min, x max, y max; x += min + `range(0, max − min)`, then y. The graph
  turn (`0x141d14b30`) turns the offsets with the positions; a node with the lock flag (+0x41)
  keeps its orientation.
- Corridor types (`0x1420a50e0`): sorted by path, a discarded `below(n)` and a shuffle, then
  stable-partitioned: types with one room size (`0x1420a55b0`), other types, the default type.
- Chirality: a door type is chiral (`0x141d047b0`) when it changes height or has two grounds, or
  is a virtual edge whose halves differ or are the same chiral type with equal flags. `.arm`
  exits carry it (the third number up to version 29, the fourth after); mirrored turns flip it.
  Typed nodes require (edge from == node) xor edge chirality (edge value 1). A door between `a`
  and `b` (`0x141fa7fd0`) gives `a` `!chir` and `b` `chir`; shared positions (`0x141fa7bf0`)
  require those, and a chiral door of another type on either side empties the list. Corridors
  pass the edge's chirality (A* neighbour test `(cur, n, chir)`, expansion `(n, cur, !chir)`);
  branches, extra doors and `0x1420a4090` pass 0. Placeholder matching (`0x141f9b430`) compares
  chirality for chiral types.
- Special-room split (`0x1420a8980`, used when no room matches exactly): at the corners of each
  eligible room, per allowed orientation, a spot where the special room covers free cells and
  whole non-special untagged rooms of one height (`0x1420a8bc0`) with agreeing doors along its
  sides (`0x1420a8fc0`; a default-type special door may face a closed one, but not all of them).
  Spots are kept sorted by x·65536 + y (`0x1420cef60`); `below(spots)`, `below(variants)`; the
  covered rooms are removed (highest index first) and the special room is stamped at the height
  of the last one removed.
- Filler rooms (`0x1420b19f0`, before and after the special rooms; reserved cells are freed in
  between): per room in order, each side (right, bottom, left, top) whose two turned corners
  share a ground spreads it into the empty cells beyond (`0x1420b23a0`, height = room height +
  that door's height), first for grounds with `.gt` flag 2, then the rest, each pass repeated
  while it places anything. The ground's size list (room set +0x98) has per blank room (no tag,
  one ground, no doors) its size and, if not square, the turned size, each with the room's
  weight, in `.rs` order; `below(total)`, and a size that does not fit drops its first entry and
  is redrawn (`0x1420b276a`). The spot covers the cell: corner at the cell, then shifted left, up,
  up-left (`0x1420b2bb0`); the room is drawn among the ground's blank rooms of that size
  (`0x1420b2a21`), its orientation with `below(840)` (`0x1420b2a38`).
- `.dgr` void token is `V` (nodes and edges); void edges block their cells (Bresenham,
  `0x1420ad420` / `0x141fa8710`). `.et` numbers: height change, size, centred, random wiggle,
  `can_end` (fifth), force height zero. The size map (`0x141f99b20`) counts a room per distinct
  non-default door type; rooms with only default doors count under the default type, whose plain
  sizes need a full ring at height 0 (`0x141f98bb0`). Border keys rebase their heights by the
  signed minimum.
- Subgraphs (`0x141ff7e00` → `0x141ff05f0` per subgraph): a `graph` node (`0x1420adfd0`) reserves
  its rectangle in the main grid; after the main graph's fill and de-repetition, each subgraph
  `.dgr` (turned with the main region) runs the whole pipeline (offsets, existence rolls,
  layout, tile pass, fill, de-repetition) in a window of the main map at that rectangle
  (`0x141f9e130`). Then `0x141ff7a10` runs, per graph from the last subgraph to the main one,
  the outer fill with height repair (`0x141fd5ff0`, on that graph's rectangle with its master's
  `OuterGroundType`) and the outer buffer (`0x141fd8360`; empty for `OuterBufferSize 0`). A graph without edges has no corridor
  stage (no `below(128)` records, no type shuffle). G1_8's secret chamber
  (`_secretchamber.dgr`, `master_secretchamber.tsi`) is one typed node.
- `.ecf` rules without a height number (`canopy.ecf`) apply to any corner height difference:
  `0x1420a28c0` looks up (pair, difference, exact) first, then (pair, any).
- Subgraph outer buffers: a subgraph's region sits inside its master's `OuterBufferSize`
  margins in the reserved block (G1_12: 10 units × 6 + 1 = 61 tiles + 13 per side = 87 = 3 main
  units of 29); `0x141ff7a10` runs the outer fill and the margin strips per subgraph on that
  block (`outer_buffer_window`).
- Typeless nodes (`0x1420afdd0` → `0x1420afa40`): the size list is the one of the first edge
  (node slots in order) whose type is not the default, else the default type's.
- Fill tiles may overhang the region by one cell (`0x141f9e5f0`: x, y ≥ −1, far side ≤ size + 1; see
  [shared-passes.md](shared-passes.md#region-overhang)),
  so 3×3 ground slots on the region's border fill from −1. The outer fill aligns a 3×3 tile one
  cell outside when the region's corner and size are 1 mod 3. Fill rooms floor their slot heights
  like pass A; de-repetition keeps the replaced tile's height.

## Findings 2026-10-22 (agent A, batch comparison)

- Graph turn `0x141d14b30` also turns the `.dgr` node random-offset ranges (node values 5..8 = x min, y min, x max, y max, at node +0x78..+0x84): each (min, max) pair is a vector turned by the region table (`[-x, x, -y, y]` indexed like the position), then each axis' pair is put in order. Without it odd and mirrored orientations draw the offsets from the wrong ranges (G1_9 +536 jobs, G1_6 all 1600, G1_12, many dgr areas).
- Fill rooms (`0x141f8c8c0`): a room cell's tile height is the passed fill height (the listed ground cell's slot height) plus the slot key's lowest corner (G1_12 / G3_7 `*forcedblank`, +1,900 jobs).
- Tools: `lay_cmp <export dir> [--map] [--units margin,step,size] [--at x,y,r] [--lat x0,y0,x1,y1] [--margin M] [--dy N]` (cell/cluster diff, placement order against the export's `generator_trace.tsv`, lattice heights), `POE_DUMP_ROOMS=1` (rooms after the layout) with `runtime/a`-style room comparison against the trace's `derepeat_room` lines, `POE_DBG_ASTAR=1` (corridor search).
- Open: G1_7 4:0 seed 145312311: edge 9 (14 -> 4, fuzzy 0) takes (2,2),(1,2),(1,3),(0,3) in the port but (1,1),(1,2),(0,2),(0,3) in the client (same length; every other path and all draws up to the placeholder pick agree); search, neighbours, `connectable`, h/g and `isqrt` were checked against `0x1420a4330`. Outer-buffer left strip (Abyss_Boss2 seed 121918740): the client's row at y=36 reads lattice height 1 at x0=4, the port 2 (interior lattice points of a levelled fill tile). Limestone/Kalguur/Cavern: outer-fill tiles end at the wrong height (port too high or low). G1_12 seed 991375241: four `Fill_02/04` tiles of the dungeon fill end at storey 1 in the client.

## Wave 3 (agent D, 2026-10-22)

Branch `terrain-d` of Poe2Server (worktree `runtime/wt/d`). Client runs: `batch/d1` (1850 jobs, one failing seed per dungeon combination, exports) and `batch/d2` (132 jobs with generator traces, 3 per area for the 45 biggest failing dungeon areas).

- **Overlays belong to the dungeon tile pass too** (`0x14209c4c0`): `0x141fa0ac0` builds the context after pass B and the doors, phase 1 (`0x141fa1a70(ctx,1)`) runs, then the fill of the listed cells (`fill_cells`), then phase 2 `(ctx,2,1)`. In `.dgr` graphs the candidate scan is cell by cell (generator flag `+0x90` bit 0 turns the 3-tile stride of `0x141fa1e60` off), and phase 1 also records the tiles under an overlay for the refill (`0x141fa23e0` refill flag). The context's room list (`param_4+0x58`) is the dungeon grid's placed rooms: rooms whose `.arm` has a tag mark bit 4 on their rectangle shrunk by the shared border (when the room's own overlap equals the master's) and bit 8 on the whole rectangle; nodes with tags mark bit 0x10. Tiles the refill places feed the exit wiggle list.
- **Pick weights**: an entry with an explicit orientation list (`R90 FI ...`) counts its room once per listed orientation that turns the group's key (`overlay_superman` in `G2_8` gave 200, not 100); an entry without a list counts once.
- **`!Name` exclusions** (`0x141fa3470`): the tiles that start inside the candidate rectangle and carry a feature name must include every listed name; **`MatchAllTileKeys`** (`0x141fa3780`): every `k` slot of the room, turned and laid at the position, sits on a tile that starts there with the same feature and key (tried over the entry's orientations).
- **Per-room lattice settle** (`0x142099e90`, after the ground overrides, `dungeon_tiles.rs::settle_rooms`): lattice points of a placed room without a ground take the room's ground (a blank room's own, else the graph's default, graph line 2); a point at the lowest or highest height of the map takes the graph's low (line 3) or high (line 1) ground (not `impassable_ground.gt`); the point is then set. Without it the fill and overlays saw `-` grounds (G2_9_1, G2_5_1: +7,183 jobs).
- **Corridor containers**: the default edge type (`rooms.et`) is always the last container, even when its room-size list has one entry (MapCrypt, MapRiverside, Kalguur, G2_8 corridor draws).
- Tools: `POE_DBG_OVERLAY=1` (candidate groups, picks, `FIT` codes of `0x141f9d200`), `POE_DBG_OCC=<file>` (occupancy grid), `POE_DUMP_LATTICE=<file>` (writes `<file>.<w>x<h>`; compare with the trace's `lattice 92ce0` view of the same size).
- **Open**: (1) `G2_4_1` and kin: the lattice before the overlays differs on 2-wide strips inside filler rooms (`buried_roof_fill`): the port gives them the high ground and height 2, the client leaves 0; the filler height (`0x1420b23a0`, room height + door height) or the ground-only height marking is wrong there (first diff `(75,17)` of `G2_4_1 174093809 1:0`, `batch/d2`). (2) `G2_8`: five 3x3 `Dunes` fill tiles differ in orientation and the exit wiggle set has 139 entries against the client's 140 (draw 2974, `0x1420a201f`); the same divergence ends `MapSpiderWoods` and `MapSteamingSprings`. (3) `MapCrypt`/`MapRiverside`/`Kalguur`: now diverge after the corridors in the 5000th draw ("draws in step" class). (4) `MapLostTowers` diverges at the existence rolls (`0x141feeb10`, shared pass), `G3_6_1` at the typed room pick (`0x1420aebc9`), `G3_5` at `0x1420b125c`, `MapDerelictMansion` / `G2_2` / `G3_3` at the corridor shuffle `0x141fc010f` / `0x141fa81a3`.

### Chirality (wave 3, agent E)

- The chirality code in `dungeon.rs` (typed-node requirement, door pairs, corridor A* with `(cur, n, chir)` / `(n, cur, !chir)`, placeholder matching, mirrored `.arm` turns, `region::turn_graph`) was already complete; only the guard that rejected every edge type with a height change or a virtual edge was left. Removing it moved the "chirality not ported" class (27,215 jobs, 44 areas) to ordinary results: 8,532 match (MapPit, MapWillow, MapSandspit, MapSteppe, MapGrimhaven, MapBloomingField, MapBazaar, MapOvergrown, G1_14, G4_11_2, MapStronghold, G1_11 160, ...). No chirality-specific mismatch was found in G1_11; its remaining difference is in overlays.
- A special room with no doors (`Barrow_*.arm`, `checkpoint_fill`, `special_fill`) needs no ground match: `0x141f982b0` sets the "blank" flag (+9) only for rooms without a tag, so a `special` room never has it.
- Overlay room height (`0x141fa10b0`, `overlay::room_height`): border corners of the room's `k` slots against set terrain heights (`terrain - corner`); corners whose ground follows heights (`.gt` flag 1, or none) must agree, the others vote; `room_pass_oriented` takes the base height and puts each slot at `base + lowest corner`.
- Open (G1_11 1001, topology 7 o1): after the first thinwall overlay room the client's next candidate pair for the same weights lies at x 24..27 y 35+ (group size 4x3), ours at (11,26)/(11,27) (group 4x5): the candidate set (`room_fits`, margin 1, terrain after the first overlay) differs; first diverging tile pick is draw 1055. Other classes left in the list: door spots `0x14209dc90` (MapBazaar, MapNecropolis, MapTerrace), overlay exclusions, `could not fix heights` (MapCreek, MapMesa, P3_1: tiles with no set corner in the outer fill), GetFeatureRooms (world maps), G4_13 overlay without root key.

### Wave 3 regression fix (agent D, 2026-10-22)

- **Settle uses the room's own ground only for "walk-uniform" rooms** (room data `+10`, set by `0x141f982b0`): a blank room (no tag, no thingy `.et`, four equal corner grounds, `+9`) whose slots' corner grounds are all as walkable (none, or `.gt` flag 4) as the room's own ground. Otherwise the settle uses the graph's default ground. `Fill_02.arm` of MapBastille (courtyard ground, black-wall cells) is not uniform; the port gave its points the courtyard ground and the overlay refill then took the courtyard fill group instead of `Prison_Fill` (`settle_rooms`, `uniform_walk`).
- **The graph's grounds need their `.gt` flags**: the flag table held only the grounds of tile-set tiles and rooms. A ground that reaches the lattice through the settle (`CarverUpperGround.gt` of G3_7: `1 1 1 1 1` = 0x1f) fell back to 0x0f, so the fill (`0x141f8f930`, flag 0x10 = no ground) picked its own 12-entry group instead of `default` (1,201 entries) at draw 1919. `area.rs` loads the flags of the three graph grounds before the tile pass.
- **Overlay rooms with negative corner heights** (MapChannel `hallway_stairs_lower`): `room_pass_oriented` must subtract the lowest corner and place the tile at it (agent E's `01579b6`/`dca172a`, cherry-picked; also the `room_height` port).
- Gate over the full batch (`POE_FORCED_BUDGET=60`, 14 threads): 99,991 of 218,705 match (baseline `forced-ab.tsv` 72,718), 27,273 gained, 0 lost. With budget 20 the first jobs of `G1_13_1` can hit the "eight hung" abort under 14 cold threads (they take 2 s alone); `forced_verify` now prints `HUNG <area> <seed> <topology> <orientation>`.
- Debug switches: `POE_NO_SETTLE`, `POE_OLD_EDGE`, `POE_NO_GRFLAGS`, `POE_NO_UNIFORM_WALK`, `POE_DBG_FILLCELL` (fill cells and placed tiles), `POE_DBG_SETTLE=x,y` (which room settles a point), `POE_DBG_CELL=x,y` (tile at a cell after each dungeon stage).

## Wave 4 (agent A, 2026-10-23)

Branch `terrain-a` of Poe2Server (worktree `runtime/wt/a`), commits `8f19224` and `0788b31` (WIP). Client runs `batch/w4-a-{0,1,2,3}` (23 jobs, one failing job per big area, traces with the new `ofit` lines), `w4-b-1`, `w4-c-1` (requests of agents B and C).

- **Branch pass re-marks corridor heights** (`0x1420a5680`, first loop): for every path of the edge type, `0x1420a2fb0` runs again on each inner room. `compact` (`0x1420a6610`) removed the unused placeholders of the corridor pass and `fill_placeholders` made new ones, which would otherwise have no heights, so `branch_starts` found no candidate (the client did 9 branch doors in `G2_7`, the port none). `G2_7`: 100 of 785 jobs match, the draws are in step to the end.
- **Overlay pick weights** (`0x141fa23e0`): an entry counts its weight once, or with `MatchAllTileKeys` (group flag 0x40) once per orientation of the entry (the ones turning the group's key) whose room keys all sit on the laid tiles (`0x141fa3780`); the orientation is then drawn from the whole entry mask (`FUN_1421c3d00(mask, below(0x348))`). The earlier "explicit orientation list counts once per orientation" rule was this case.
- **Ground flag bits** (`0x142918970`): `+0x24` bit 0..4 the five bools, 0x20 `HideRoof`, 0x40 `HasWater`, 0x80 name contains Abyss/Water/Pool/Void; `+0x25` bit 0 the name `SynthesisBackground`. The corner test of `0x141f9cfc0` passes a lattice ground on `+0x25` bit 0, the port used 0x80: every Abyss/Water ground passed, so overlay candidates the client rejects (`G2_2`: 12 positions of weight 100, `4600` against `3400` in the first pick) were kept. Fixed in `overlay::room_fits` (WIP commit, not gated yet): `G2_2 751365688 1:0` 1597 -> 9 cells.
- **`place_tile` `0x141f9e2e0`**: with tile data `+0x16f` bit 1 and no edge of a real type the orientation is `& 3`. Seen as the last 9 cells of `G2_2` / `G2_10_1` (`CenterHoleLarge_01.tdt`, ours o7/o5, client o3/o1). `+0x16f` is not the `.tdt` flag word: `0x141d053a0` (run when a tile's data is loaded, `0x141d06730`) sets bit 0 when the tile's model data names an `AreaTransition` object and bit 1 when an object's type path contains `Spawner` (the `.tmd` beside the `.tgt`, UTF-16, the quoted type is the last field). Ported as `Tdt::spawner` (`load_tdt`, only computed for tiles whose edges are all wildcards or absent) and applied in `TileMap::place`: `CenterHoleLarge_01` (spawner objects) loses the mirror bit; G2_2 and G2_10_1 seeds match.
- **Tool: `ofit`** (client DLL, `terrain_tools.rs::on_room_fits`, signature `room_fits` `0x141f9d200`; uncommitted in the hub's `modules/client-hook`, `signatures/0.5.5e.toml` and `scripts/re/sig_spec_0.5.5e.py`): every `0x141f9d200` call from the overlay scan as a trace line `ofit x y WxH fFLAGS` / `code e <edges> h <heights> g <grounds>`. Positions are in client coordinates (add 6 to ours). Compare with ours using `POE_DBG_OVERLAY=1` (`FIT` lines now print flags, edges, grounds) and `runtime/wave4/a/ofit.py <export dir>` (keys differing by position, size, edges, grounds).
- Status of the 132 jobs of `batch/d2` after these fixes (`runtime/wave4/a/sum-d2-1.txt`): `G2_7`, `G3_6_2`, `G1_12` draws all in step (remaining: outer-buffer fill heights, 126 / 25 cells), `G2_2` overlay candidates fixed (9 cells), `G2_4_2`, `MapMarrow` in step; open first divergences: `G2_4_1` / `G2_5_2` / `MapBurialBog` / `G1_11` overlay `141fa249c` (use `ofit.py`), `G2_8` / `MapDeserted` / `MapSpiderWoods` / `MapSteamingSprings` exit wiggle `0x1420a201f`, `G3_3` `0x1420aff12` / `0x141fc010f`, `G3_5` `0x1420b125c`, `G3_6_1` `0x1420aebc9`, `MapLostTowers` `0x141feeb10`, `G3_2_1` / `MapSavanna` / `MapUniqueMegalith` `0x141f8fd5a`, `MapFrozenFalls` `0x141f900a7`, `G2_4_3` `0x141fe75b7`, `G1_13_2` `0x14209c1ea`.

## Wave 4 (agent B, 2026-10-23)

- **3x3 door spots `0x14209dc90` (WP7.8) ported** (`dungeon_tiles.rs`, `check_doors`): for every spot in the list pass B built (3x3 tiles touching a shared exit), if the cell holds the corner (offset 0,0) of a tile without the `door` feature, the `door` tiles for that tile's own oriented key are looked up; a hit draws `below(total)` and replaces the tile at the cell's height (`place` with flag 1). Door areas (39 areas, 29,402 jobs): +11,800 matches, 0 lost.
- **FileGroups subgraphs** (`0x1420adfd0`, `dungeon.rs` `FileGroups`): the master's `FileGroups` `.fgp` lists a group name line (no `.`) then files (relative to the `.fgp` directory unless they start with `Metadata/`); a `graph` node whose first tag is not a `.dgr`/`.tgr` draws `below(count)`, and when the group gave files before, draws again `below(unused)` among the unused ones; the pick is recorded and replaces the tag. This removes the abort for G3_1, G2_3, MapSwampTower, MapAzmerianRanges, BossRush, Chayula towers; most of them now produce wrong hashes for other reasons, G3_1 and BossRush_Area2 stop at `.tgr` subgraphs ("not a dungeon graph", not ported).
- **ShipPlugin** (`TerrainPlugins` row 14 on `WorldAreas.TerrainPlugins`: G4_1_1, G4_2_1, G4_3_1, G4_7, G4_9_, G4_11_1a/b, G4_13, G4_14; ~8,800 jobs, all failed before). The plugin's hook `0x141f03c40` (vtable slot 4) calls `0x141ff9860` -> `0x141ff89e0` for `G4_ship`: right after the main job's `below(0xffffffff)` it draws `below(topology count)` and `below(0x348)` (orientation mask pick), then the ship graph (`ship_subscene.dgr`, one 22x22 room) is generated after the main graph (its own StoreyHeight 24) and sits beside the finished map with a one-tile gap: right of it when width >= height (x = width + 1, y = 0), else below it (x = 0, y = height + 1). Port: `area.rs` `append_job`. G4_7 40/40, G4_14 26/26, G4_3_1 30/40 on the sample; full 9-area run: +3,322 matches, 0 lost.
- **Multi-rotation `.gft` lines** (`I FI`, `I FR270`, ...): the client's fill entry holds one orientation byte; with several rotations on a line the **last** one wins and the line counts once in the group's weight total (verified on the client trace of `G4_1_2 1062741105 1:0`: `Fill_02 ... I FR270` only ever places orientation 7). Port: `generate.rs` `fill_orientation`. G4_1_2 36/40, MapSulphuricCaverns 27/40, Outlands, Ezomyte/Karst fills 40/40 on a 40-per-area sample.
- **`OuterFillHeightOverride` and `OverrideSpawns`** (tsi keys): the outer fill (`0x141fd5ff0`) skips the height repair and places every grid tile plain at that height when the master has `OuterFillHeightOverride <n>` (value n), and (found by comparing client traces: MapCreek and P3_1 place all outer tiles at height 0 row-major from (-1,-1) although their ground has the same flags as the repairing `black_inside_wall.gt`) when it has `OverrideSpawns` (height 0). The parser's flag offsets are master +0x161 / value +0x153, but the store of the `OverrideSpawns` flag was not located: the rule is an inference from the 17 traced areas (every traced master with `OverrideSpawns` was plain, every other repaired). Fixes "could not fix heights" for MapMesa, MapUniqueInitialTower, MapCreek (matches), MapRavine, MapUniqueSelenite, P3_1. `Master::outer_fill_height`.
- **StoreyHeight per graph**: a subgraph's cells (and the outer fill and buffer cells its pass adds) are scaled by its own master's `StoreyHeight`, the main graph's cells by the main one (`area.rs` `owner` map, `finish_dungeon`). MapMesa matches.
- **ShipPlugin order**: the `G4_ship` job (its own node offsets, prune, tile pass, fill) runs after the main graph's subgraphs and before any outer fill; it is finished and appended afterwards (`append_job`). G4_2_1 and G4_7 match, G4_3_1 and G4_9_ do not yet (different causes).
- **Open: "no border tile"** (MapDerelictMansion 549, G1_8 431, ExpeditionLogBook_Reef 298, G4_3_2 280, MapCenotes 275, MapNecropolis 247, G4_8b 106, ...): the height fallback `0x141d05560` / `0x14209fb00` (`place_border`) finds no tile for keys with a corner without ground that holds a height (Reef `[None, coral_abyss.et, coral_ledge.et, coral_abyss.et]` heights `[0,0,1,0]` at (32,17); Derelict heights `[1,0,0,0]`). Zeroing heights of ground-less corners was tried and does not match (MapDerelictMansion 104780812 still differs). Client traces requested: `ExpeditionLogBook_Reef 1183147915 1:0`, `MapDerelictMansion 770640980 1:0`.
- **Not done (left in `runtime/wave4/b-fails.tsv`)**: `.tgr` subgraphs in FileGroups (G3_1 End_*, BossRush_Area2 SilverbackBlackfist: "not a dungeon graph", needs the tgr generator in a window), lettered unique tags (`uniqueA<N>`, 900, session history), Gallows overlay room without root key (G4_13, 400), "no room" Gates (450), special room failures (283), edge connect (209), map-size jobs of G4_1_1/G4_3_1/G4_11_1b (were the ship area; G4_3_1/G4_9_ remaining diffs are tile differences).

### Wave 4 (agent A): later findings

All commits are gated against the baseline on `runtime/wave4/in3b.tsv` style samples (5,961 stratified jobs of the 149 areas plus the 15,000 match sample): 0 lost, +749 matches on that sample at `9c85635`+`pre-unique`.

- **Corridor stage**: skipped (no records, doors, room assignment) when every edge is removed or void (`0x1420ac3e0`; MapLostTowers' container graph). Edge-type batches and the corridor-type keys sort by the resource path in the graph's own case (`0x141fade20`, a case-sensitive u16 compare; MapDerelictMansion draws fully in step).
- **Typed rooms** (`0x1420af280`): a door of the default edge type or of a `can_end` type never marks a candidate as mismatching a neighbour (G3_6_1). A room listed twice in a row in the `.rs` merges into one matcher entry with the first line's weight (G3_5 `tri_01`).
- **Fill rooms height**: `.tsi` `DungeonFillRoomHeightOverride` (master +0x14d, set flag +0x159) replaces the filler room height (`0x1420b1bd0`; G2_4_1 lattice strips).
- **Overlay context**: rooms placed by the special pass are not feature rooms (G2_5_2). Tiles placed by overlay rooms feed the exit wiggle list too (`0x14209a2c0` listener; G2_8 140 entries, MapDeserted, MapSpiderWoods, MapSteamingSprings).
- **Pre-unique room** (`0x14209c150`, runs before the overlay context is built when the master has an overlay list and the room set a `preunique` room): the ground-only cell list is shuffled (a discarded `below(n)`, forward Fisher-Yates), then walked; each iteration draws `below(0x348)` (an orientation of the `.rs` mask), and the first cell that is accessible, has no ground or a ground with `.gt` flag 8, and whose room rectangle lies free in the region takes the room (`0x141f8c8c0`, height = the cell's). None fits: "Failed to place pre-unique room". The cell list is sorted by (x, y) only afterwards (`0x141f92ce0`); `dungeon_tiles::TilePass::run` returns it unsorted (G1_13_2).
- **Settle**: a placed special room keeps its own ground like a blank room (G3_2_1 `incursion_01`).
- **Fill rooms on unwalkable ground** (`0x141f91110`): only rooms with a point of interest that has a tag are kept out; untagged points of interest do not count (MapUniqueMegalith `stones_4/9`, MapSavanna).
- **Open**: (1) `G1_13_2` 27 cells: a `Hut_St_StoryGlyph` tile appears in the client with no `place` line; the room-injector passes `0x14209a620` / `0x14209b310` (room set +0x118 list, gated by +0x148 byte of the context) are not ported (injector rooms: `room_injector*`, `telepad_injector*`, `tile_injector*`). (2) `G2_4_1`: the first subgraph's fill (`_subarea1`) has 11 fill cells in the client against about 45 in the port. (3) `MapFrozenFalls`: a fill cell takes the `upper_ground` group in the client (total 363), the port has no ground there (total 200). (4) `G3_3` / `G2_10_1` / `G2_2` 9-cell `OpenWater` / fill orientation diffs (fill orientation masks, agent B). (5) `G2_4_3` `0x141fe75b7`, door spots `0x14209dc90` (G3_5, agent B).

## Wave 5 (agent B, 2026-10-24)

Addresses are for client 0.5.5e. Commits are on branch `terrain5-b` of the server worktree (`runtime/wt/b`); every one was gated with the 15,000-job match sample plus the full jobs of the areas it changes (0 lost against `runtime/forced-baseline.tsv`; the last gate: 26,260 jobs of Programming, G2_6, G1_15, MapUniqueLake, G3_1, BossRush and the border-tile areas: +3,634 matches).

- **Tile lookup `0x141d0a4c0` has a second index and a retry** (`tileset.rs` `TileSet::candidates`; fixes most of "no border tile"): besides the tiles by key, the lookup keeps the tiles by their key with the corner heights flattened (all four corners to the lowest). When the exact key finds nothing, the client retries with the flattened key. 264 of 385 sampled jobs in 18 areas matched afterwards (G1_8 4,800/4,800, ExpeditionLogBook_Reef, MapPrecipice, G4_8b all match).
- **Edge fields by `.dgr` file version** (`0x14292e8e0`): the number of fields an edge record reads depends on the `.dgr` file version (the port rejected the older layout with ".dgr edge field N in [...]"); `Programming` 50/50.
- **Lettered unique tags** (`0x141fe9970`, `area.rs` `LETTER_HISTORY`): a tag `uniqueA<n>` takes its candidates from a history shared by all graphs of the job (main graph and subgraphs, cleared per area): the first node of letter `A` draws, the following ones of the same letter reuse the earlier pick's slot. `G2_6` 100/100, `G1_15` 652 of 800. `OuterGroundType` of a master is resolved relative to the master's `.tsi`.
- **`.tgr` subgraphs inside a `.dgr`** (`area.rs` `build_tgr` split out of the graph pipeline, `generate_dungeon` runs it for a FileGroups pick that is a `.tgr`; example `tgr_solo`): the subgraph runs the whole outdoor pipeline in a window of the parent map (its corners start as the parent's). What the window changes (all verified against G3_1 `End_*.tgr`, BossRush `SilverbackBlackfist`): the unwalkable marking and the enclosed-fill marking (`0x141fe0fd0`) are skipped; the outer fill and the outer buffer of the window run later, last subgraph first (`outer_buffer_window`); pasted pieces (`TileMap::paste`) have no tile of the parent's set, so the outer strips read their edges from the headers loaded by `preload_headers` (`piece_tdt`). **Paste also merges the subgraph's `.gt` flag table and edge flag table into the parent map** (the client's tables are global): without it the left strip saw ground `None` on the pasted jungle tiles of BossRush_Area2 and failed with "no edge tile to expand the outer buffer ... left" (corner grounds `None`); BossRush_Area2 now matches 32 of 50.
- **Typeless nodes skip the side adjustment**: `0x1420c76d0` calls `0x1420c51c0` (move edges between sides until a room matches) only for nodes with a room type; a node without one lists its candidates for the key as built. An empty candidate list draws nothing (`place_node_opt`, the draw came after the check).
- **Fixed-orientation tile nodes list every orientation of the matching tile entries**, each with the node's orientation (`node_tiles.rs` `candidates`; as for rooms, the bridge of G3_1 lists its tile twice, which changes the weight total of the draw).
- **Comma-list room types** (`Gates_N, Gates_FN`; G2_WorldMap's Gates nodes, 400 jobs): `0x141fea420` -> `0x141fee6e0` runs right after the node offsets (`0x141fea2e0` -> `0x141fee830`) for every node, in node order: a room type with a comma keeps one entry, `below(count)`. The earlier "any entry matches" rule drew nothing and gave a wrong hash. `area.rs` `random_offsets` does both passes.
- **World-map panels draw no sub-seed** (`0x141ff90f0`): the `below(0xffffffff)` before the topology index is skipped when the area kind (`WorldAreas` unnamed i32 column 45, area record `+0x1b4`) is `0x21` (G1..G4, G6 world maps; TN_WorldMap has 0x23 and still draws); the sub-seed then comes from the caller's seed word. G2_WorldMap 400/400 and G6_WorldMap 50/50 match after this and the comma pick (G1_WorldMap 300, G4_WorldMap 48 still wrong hash, G3_WorldMap 50 fail in the outer buffer: `edge_void.et` at (34,1)).
- **Overlay rooms whose root slot is not a key** (`Slot::S`, e.g. `overlay_seagullsoncorpse.arm`: a 1x1 grid with doodads and points only): the group key is a blank key of the grid's size (no edges, grounds or heights) instead of an error. G4_13 now generates but its map is 132x94 where the client's is 202x94 (open).
- **Negative result, fixed axes of split nodes**: copying the node's fixed-axes flags (`+0x8b`/`+0x8c`, `.tgr` node values 9 and 10) to the nodes the virtual junction (`0x1420c5ee0`) and the splits make fixed G3_1 topology 1 and 6 (40 of 128 sampled) but lost 171 of the 15,000 sampled matches (MapBackwash, MapCrimsonShores, MapAridPlains, MapWetlands, ...) and G3_1 topologies 3, 5, 8. The client builds the junction's new nodes with `0x141d1c750`, which constructs a node with the flag bytes 0 and copies only `+0x48`; a node clone (`0x141d1ad30`) used when a subgraph is generated (`0x141ff2c50`) does copy them. Dropped; with no copy G3_1 matches 56 of 128 sampled (topologies 3 and 6 fully, 5 and 8 mostly, 1 only once: the tile nodes of topology 1 must not be pushed, a rule not found).
- **G3_1 open: tile nodes move between choice and placement in the client.** Client traces (`batch/w5-b-1`, `batch/w5-b-2`): the `node_place` line of a tile node and the `place` line (`0x1420c9b01`) of the same tile differ for some tiles: topology 2 `JungleSand_toWaterM_03` (12,3) -> (15,6); topology 4 `JungleSand_toWaterM_02` (12,6) -> (12,3); topology 7 `JungleSand_toWaterM_02` (30,6) -> (33,6); topology 1 keeps (15,39) where we move to (15,42) (our push of +30 flips the 3-grid snap). Topology 3 never moves, we match it. The moves are one `align` step (3 cells). The repel pass `0x1420cbe60` (flag 0 for non-virtual node tiles, `0x1420cade0`; flag 1 for the special-node pass `0x1420caa40`), its push function `0x1420cb7a0` (halo 0x8a, rect margins from `0x1420cb470`), the node push apply `0x1420cc920` (skips a node with exactly one edge when the flag is 0, `+0x8b`/`+0x8c` fixed axes, clamps) and the re-centring (`0x141fe7400` snap, `0x141d0d880` free test, `0x1420c59e0` border margins) were compared and match the port; the `v112 at 0,0` `node_place` lines are failed lookups (the printed byte is the low byte of an error-string pointer), not records. **Strongest lead (not yet ported): `place` `0x1420c9970` calls the settle `0x1420c9260` for every tile record before placing it** (margin rectangle free, non-wildcard edge not on the map border, corner grounds against the lattice `0x141f9e490`; else the square spiral of `align`-sized steps), exactly the `rooms.rs` `settle` the port runs for rooms but not in `node_tiles.rs` `place_node_tiles` / `place`. The moves seen in the traces are one `align` step; the topologies that fail (1, 2, 4, 7) all have a `SandJungleBridge_01` tile placed at the room stage before the node tiles, whose corner grounds sit in the lattice (topology 3, 6, 8 have none and match). Port it by calling `settle` (key = the tile's turned key, edges by side as in `keep_off_border`) per chosen tile right before `place`, in order.
- **Not done**: MapKarst (400, `PairedRoomLayoutPlugin` `0x141f01710`, FileGroups `Pairs`: a paired subscene room is placed in the empty `_abyss` graph; the port aborts "could not fix heights"), G4_13 (map size), G4_4_2 (`Nopen_st` key, 100), G1_15 topology 1 (148 wrong hash), MapSinkhole (100, "no tile for room cell" with a 3x3 halls key whose side exits are `[9, 4, 7, 4]`), MapPenitentiary (59, fill room `fill_garrison_12.arm`: a 1x1 slot with ground `courtyard_outerground.gt` finds no fill and no tile by key), BossRush_Area1 (nested subgraphs), BossRush_Area3 (a 12x12 `boss_01.arm` Arena key), G3_WorldMap Lake rooms. Debug env switches left in the port: none new (`POE_DBG`, `POE_DBG_CELL` as before).

## Wave 5 (agent A, 2026-10-24)

Poe2Server worktree `runtime/wt/a`, branch `terrain5-a`: `025511a` (Spawner flag) and `d76cd95` (corner writes), both gated against `runtime/forced-baseline.tsv` (0 lost). `0cfbbdf` and `8498889` were WIP steps; their ideas live in the two gated commits.

- **Spawner flag is the OR of two `.tmd` sources** (`025511a`). A tile's Spawner object flag (the one `place` `0x141f9e2e0` tests to drop the mirror bit) comes from the art `.tmd` beside the `.tgt` *or* the `.tmd` the `.tdt` names; the binary `.tmd` Spawner scan reads the object table. Reading only the tdt's own `.tmd` (`0cfbbdf`) lost 301 of the 15,000 match-sample jobs (G2_2, MapMarrow, MapMesa, MapSpiderWoods, MapUniqueInitialTower, MapIceCave); the OR repairs them. Gains (full jobs): G2_9_2 390 -> 798/798, G3_3 555 -> 790/790, G4_5_1 0 -> 399/400.
- **`grid_fits` compares grounds case-insensitively** (same commit, from `0cfbbdf`): the room-slot ground compare of the overlay candidate fit lower-cases both sides, as the client's resource lookup does.
- **Corner writes of tiles that overhang the region** (`d76cd95`, root of the "outer fill heights" class: MapOrbalasCrossing, MapCenotes, MapPort, G3_6_2, MapHiddenGrotto):
  - The client's region is a window into the full map. A dungeon tile placed at `y = -1` (or past the far sides) still writes the corner points that lie outside the region window; the port keeps those writes as spill entries and replays them in `embed` / `paste`.
  - The replay applies `place`'s corner rule, `0x141d0bad0` (the second loop, `0x141d0bec7..0x141d0c0f2`): the ground is always written (through `0x141d0e130`, which has its own gate: a set cell only takes a different ground when neither ground has `.gt` flag 1 or 4 and `0x141d02530` says no; not ported, no case needed it so far); the height `d + h` and the set flag only when the new ground is null or has `.gt` flag 1, or the old ground is null or has flag 1. The old code replayed every spilled write unconditionally, which was wrong for every `Cliff_Fill` overlap in MapOrbalasCrossing (first writer wins there, 6 points in the round dump).
  - `set_heights` (`0x141d0b390`, called right after `place` at `0x142098df1` and `0x14209f9cc`, i.e. after both tile-pass placements `0x142098dcc` / `0x14209f9a7`) writes `h + d` on all corners of the tile with no flag rule. The port returned early when the tile's origin cell was outside the map; now the corners inside are set and the ones outside become `Spill::Height` entries. This is what makes the HiddenGrotto `Ledge_StSt_01` / `Cc_01` shared column come out as the client's (the lattice dump `round` of 4261360127 2:3 now has 0 differing points; the unconditional replay of `8498889`'s predecessor had matched by accident).
  - Measured (full jobs): MapOrbalasCrossing 0 -> 1543/1600, MapCenotes 341 -> 764/788, MapHiddenGrotto 643 -> 984/1600, MapPort 950 -> 1168/1200.
- **Hook additions** (client hook, hub submodule `modules/client-hook`, uncommitted at the time of writing): lattice `round` (whole map, only when it changed) and `afterround` dumps; `round_corner` (site `0x141fd71eb` inside the levelling step `0x141fd71c0`: `x y n h0..h3`, result = the cell's current height). Finding with it: for HideoutLimestone 1213549957 the client never calls the levelling step (no `round`, no `round_corner`, only the `92ce0` lattice step), so the outer fill rounds do not run there.

### Leads found but not finished (all with a client trace in `dumps/0.5.5e/terrain/batch/w5-a-1` unless noted)

- **Merged edges and the extra door pick** (`0x1420966f0`, G1_13_1, 24 jobs in `batch/w5-a-5`): the pick sums the extra door weight (edge `+0x54`, token 4 of the edge's extra fields) over the node's four slots. When a unique-group node loses its room and has exactly two edges of one type, `remove_node` (port `area.rs`, client `0x141feb9d0`/`0x141fec1c0`) merges them; the client's merged edge counts with weight 0 where the port keeps the first edge's 65 (port draws `below(130)`, client `below(65)`). Copying all of edge 2's extra fields onto edge 1 fixes the draws of all 24 jobs but loses 27 of 225 gains on a 300-job G1_13_1 sample (200 match in the baseline, 225 with the change): in 5 lost jobs the first edge's weight is kept (`e1 w65` with a typeless far end `o1` and a typeless or block `o2`, one case `e1 w0 / e2 w65` kept 0). Cases that need the second edge's weight: far node `o2` a `block` room (typed) with `e1 w65` in 17 of the failing jobs. Not yet understood: probably the client keeps the second edge object and gives it the first edge's far node, so which weight survives depends on slot order (`0x141d1c580` assigns first free slot) and on which of the two is `from`. Next: print slot order and from/to per merged edge for the 5 lost jobs (`POE_DBG_RM` style print in `remove_node`) and decide by those; the 24 jobs and the 300-job sample (`runtime/wave5/a/x1-in.tsv`, `x2-in.tsv`) are the test. Typed-typed edges are NOT zeroed in general (experiment lost 64 of 300).
- **Sub-dungeon fill rings** (G3_14 and kin, class `0x141f900a7`, 8 jobs): sub-dungeons (graph nodes holding a subgraph) are generated in a window map of their own; the port's `fill_cells` also fills the one-cell ring outside the window (list cells at `x = -1`) and draws for them, the client does not (those cells belong to the parent map and are taken). Skipping ring cells the parent has taken (a `TileMap.outside` set made in `copy_lattice`, consulted in `fill_cells`) removes the 19 extra draws but then `failed to find a fill tile or room at (0,0)`: the tile that covers the window's first cells is the parent's. Needs the window to share occupancy with the parent instead of a copy.
- **Door pick candidate count** (class `0x141fa81a3`, 9 jobs, e.g. G2_4_2 1144816777 3:5): `0x141fa8140` shuffles the list of `0x141fa7d40`; the client's list has one candidate where the port's has two for two stacked 2x1 corridor rooms. The functions `0x141fa7d40/0x141fa7ac0/0x141fa7bf0/0x141fa79e0` match the port's `shared_doors` / `facing` on a read; the state (door slots or placeholder flag of the two rooms) must differ earlier without changing a draw.
- **Encounter requests** (class `0x1420ab667`, 9 jobs: MapChannel, MapCrypt, MapDeserted, MapHeadland, MapSandspit, P3_1): `0x1420aacf0` places rooms for the generation request's entries (`ctx+0x80`, vector at `+0x48`, counter `+0x144`, added by `0x141f8b320`): the league mechanic rolls that succeed add encounter requests (see tgr.md) whose candidate rooms come from the `.rs` groups (MapChannel: the 3 `mission_0N.arm` rooms, drawn `below(3)`, shuffled with `0x1420d0eb0`, placed with `0x1420a7da0` like special rooms). The port draws the rolls but not the effect.
- Still untouched: `0x141fe75b7` (4 jobs), `0x142096813` (2, now explained by the merge lead above).

## Wave 6 (agent B, 2026-10-24)

Branch `terrain6-b` of the server worktree (`runtime/wt/b`). Gate status per commit is in `runtime/wave6/b/handoff.md`.

- **Settle for node tiles was already ported** (wave 5 agent C, `node_tiles.rs` `place` calls `rooms::settle`); the wave 5 lead is done. G3_1: 2,538 -> 2,912 of 3,099 jobs.
- **Node tile choice has no overlap test against tiles chosen earlier** (`0x1420c87a0`): the map is still empty, the settle spiral separates them (G3_1 topology 5, two `SandEdge_Water_St_01` for nodes 2 cells apart: "node 42: no tile fits the map", 184 jobs).
- **A subgraph larger than its reserved rectangle** (G3_1 topology 2, `_SandsweptMarshEnd_S_E_1.tgr` is 60 wide, the `graph` node reserves 19x12 units = 57 wide; all other End tgr are 57x36 / 36x57): the cells and corners beyond the parent map are kept as spill (`TileMap::paste`), the outer fill skips cells off the map. Orientations 0, 1, 6, 7 match. **Orientations 2-5 do not: the client places the tgr 3 cells further left** (client `marsh_exit` sub-tile offset 3 where the port has 0 on `G3_1 120675597 2:2`, draws all in step: 6531): the oversized subgraph is aligned to the far side of its slot for turned regions. Not ported.
- **Tile lookup key ground with `.gt` flag 0x10 is none** (`tileset.rs` `candidates`; `add` stores the tile key that way, the lookup key now reads the same): `YamaArena_01.tdt` (BossRush_Area3 arena key), MapSinkhole 96 of 100 match (from 0).
- **Nested subgraphs** (`area.rs` `generate_dungeon`): a subgraph's own `graph` nodes are generated right after it, depth first, into the same map (origins relative to the subgraph's grid). BossRush_Area1 now generates; 5,309 of 32,144 cells differ (the two rightmost zones), no trace (the client crashed on it twice).
- **World maps: `ForceOuterGT`** (`.tsi`, parsed in `FUN_1429285e0` `0x142929d4c`; also `DoNotFillOutside` `0x142929f09`): the outer buffer ring is not tiled by strips (no edge tiles, no draws): every empty margin cell takes the outer ground's first fill tile at height 0 (`black_nofog.tdt`, fill group `BlackInsideWall` of `fill_tiles.gft`). `outer_buffer.rs` `outer_buffer`. With the tagged-room rule below all 898 world-map jobs match (+359, 0 lost): G1 300, G3 50, G4 48.
- **Tagged blank rooms carry their ground** (`dungeon_tiles.rs` `settle_rooms`, `own`): `Location_Label_02` / `Location_Compass_02` (tag, root `K` slot with the forest ground in every corner, no lattice line) give their centre corners the room ground in the client (lattice at `0x141fa0ac0` start, 8 points differ otherwise). The port still needs `room.arm.tag.is_empty()`; env `POE_OWN_TAG=1` drops it: world maps all match, but the match-sample loses 95 jobs (G3_1 76, Delirium_HungerBoss 11, HideoutRemidusMonastery_ 8): the rule needs a discriminating condition (not found).
- **Overlay `limit=` groups** (`overlay.rs` `run`, FileOrder branch): a group whose rooms have all reached their `limit` is not collected (G4_13 `overlay_seagullsoncorpse.arm limit=1` is listed in two groups by orientation set 0x0f / 0xf0; the second has no pass in the client). Extends G4_13 draws in step from 3,806 to 3,542+ (see handoff).
- **Extra graph jobs** (`area.rs` `extra_job`): G4_13 gets a third job after the ship, `ExpeditionSubArea_Kalguur_Act4` (`0x141efcec0` adds it for the area whose id hashes to 0x1c03): the client draws `below(topology count)` and `below(0x348)` twice (traces: `141ff8a5a`, `141ff8ad9`), ship first. The abyss (`Abyss_FILL`) region right of the ship in the client layout is this job. G4_13 map size now 202x94 as the client; 42 cells differ (`layout_diff`), first draw divergence at draw 3,542 (an extra overlay pass with one 6x6 `f19` candidate at (47,41) in the client: the port's occupancy has the feature-room bit 4 there, from `entrance_01_league_flip.arm` (tag `entrance_league`, 46x31, node with `subarea=` tags); the client does not).
- **Overlay phase 1 world-map note**: the `ofit` lines of the world map overlays compare equal to the port after the tagged-room rule (use `runtime/wave6/b/latcmp.py` for the lattice snapshot `92ce0`).

## Wave 6 (agent A, 2026-10-24)

Poe2Server worktree `runtime/wt/a`, branch `terrain6-a`: `afa196a` (gated) and `f31b64c` (not fully gated).

- **Node slots are the node's edges by edge index** (`afa196a`, `prune` in `area.rs`, `.dgr` only). `0x141d1c580` gives each edge, in file order, the first free slot of both its nodes, so a node's four slots hold its edges in ascending edge index; the port used the order of the node line. `0x141fec1c0` (merge of the two edges of a removed node) takes the first two non-empty slots: the first edge object stays (its weight, `+0x54`, stays) and gets the second's far node. With the right slot order the port's merge is exactly the client's; the "merged edge weighs 0" lead of wave 5 was a wrong first/second edge (all 24 traced G1_13_1 jobs now draw the same extra-door sums, 300 of 300 sample jobs match, full G1_13_1 3777 -> 6400/6400, 0 lost). Sorting the `.tgr` node edges too loses 1,172 of the 15,000 match-sample jobs (outdoor areas): `.dgr` only.
- **Post-unique tiles run for `.dgr` regions** (`f31b64c`, `build_dungeon`, `postunique::run` after the de-repetition, also for subgraph windows). G1_13_2's tile set has `postunique` tiles (`0x141ff2090`, 43 candidates, `below(2)` per candidate from `0x141ff2be6`, then `below(43)`); the port only ran it for `.tgr`.
- **Edgeless typed node is accessible** (`dungeon.rs` `node_accessible`): the `bonfire` node of the Gallows village graphs (`village_{sn,ws,se,sw}.dgr`) has no edge and no tag, `0x142095d90` returns 0 and no extra door reaches it (the header flag is 0), yet the client's fill (`0x14209cf50` record byte `+0x10` = room flag `+0x15` bit 0 at `0x142098a11`) offers the chest fill rooms there (total 670 instead of 660). The rule behind it is NOT found (derived from traces: G1_13_2 0 -> 1835/1842 with it and the post-unique run; match-sample 15,000/15,000). Remaining 7 G1_13_2 fails: topologies 7 (orientation 1), 8:0, 9:2, 12:5 (seeds in `runtime/wave6/a/g132-out.tsv`).
- Findings without a fix (see `runtime/wave6/a/handoff.md`): G3_14 713647203 2:0 first divergence is a fill cell whose list weight differs (ours 55, client 1) in a sub-dungeon window.

## Single agent, 2026-10-25 (after wave 6)

- **Sub-dungeon windows share the parent's occupancy** (solves the "sub-dungeon fill ring" lead for G2_4_1): a `graph` node's subgraph is generated straight into the parent map, so the cells around its reserved rectangle that the parent already placed (G2_4_1 `_subarea1.dgr` / `_subarea2.dgr`: the window's column 0, column 12 and row 0) are taken for the subgraph's tile pass and fill. The port builds the subgraph in a window map; `TileMap::inherit_cells` now copies the parent's taken cells into it and `TileMap::release` clears them again before the paste, except where the subgraph's own tile covered them (`kethentrance_01.tdt` over the parent's `black_nofog` cells). The client's fill list of the window (trace `batch/w7-1/exports/G2_4_1_174093809_1_0`) has exactly the free cells. G2_4_1 1,600 of 1,600 match (from 0); match sample 15,000: 0 lost. Switch: `POE_NO_INHERIT=1` (old behaviour). Debug: `POE_DBG_FILL=1` (fill list cells with the draw index), `POE_DBG_WIN=1` (parent occupancy around each subgraph window), `POE_DBG_MARK=x,y` (the ground-only slot that lists a cell), `POE_DUMP_LAT2=<file>` (lattice after overlay pass 1).
- **Second lattice settle after overlay pass 1**: the corners an overlay room clears (`0x141fa23e0` clears its whole `(w+1) x (h+1)` corner rectangle: ground 0, flag 1 off) and its tile leaves without a ground get the rooms' settle ground (`0x142099e90`) again before the fill: MapFrozenFalls `unwalkable_ledge_overlay_bend_01.arm` leaves its far corners with the graph's `upper_ground.gt`, whose fill group (363) the client draws. The port runs `settle_rooms` again after overlay pass 1 (`POE_NO_RESETTLE=1` turns it off). Where the client runs it was not located; the rule is from the fill groups of trace `w7-1/exports/MapFrozenFalls_39628602_3_0`. MapFrozenFalls +808.
- **League rolls blocked by `EndgameMaps`** (`0x141ffcad0`): the roll phase looks the area up in `EndgameMaps` (`0x140b230e0`, keyed by `WorldArea`); when found, its last bool (column 28, row +0xf0) set means no rolls at all. The other early exits: area flags `+0x14` / `+0x12a`, kinds 0xf, 0x13, 0x20, 0x21, 0x23, an area tag with hash `0x939d55cf`, and the area stats `0x562f`, `0x24c4`, `0x2436` (not ported). JadeCitadel, CopperCitadel, the Expedition logbooks and the unique maps set the bool: +717 on those 63 areas.
- **Filler rooms of all the area's room sets** (`0x141f98bb0`): the room set loader adds every blank room to the store's per-ground size list (+0x98) and per-size room list (+0xd8) without a guard (only the orientation entries are guarded by `*param_7`); the client loads the room sets of all the area's topologies into one store, so a room listed by two sets counts twice. MapSunTemple: `generate.rs` and `generate_R270.rs` both list `open_01`..`05`, the client draws `below(10)` where one set gives 5. The draws match one entry per room with the summed weight (`Files::area_room_sets`, `dungeon::layout` `fillers`; `POE_OWN_ROOMSET_ONLY=1` turns it off). A square room is listed once per set (MapExcavation's single set: `below(1)`); the arm's second bool is not the reason (making it double squares lost G2_6, G3_3, MapSteamingSprings).
- **`OverrideSpawns` refined**: the flat outer fill at height 0 applies only with `DefaultKillZone` in the same master (MapCreek, P3_1 have both and stay flat); MapExcavation (`OverrideSpawns` without `DefaultKillZone`) repairs its outer heights like any master (client h16 on `cliff_fill_01`). Still an inference: the store behind master +0x161 is not located (the parser `0x1429285e0` writes `OverrideSpawns` to stack +0x310/+0x311: 1/1 `Dungeon`, 2/2 `Outdoor`; `OuterFillHeightOverride` to +0x32b with a set flag at +0x31d; `ExtendActivationDistance client|server|both` to +0x325/+0x324). `POE_OVSP_ANY=1` restores the old rule.
- **Room data +10** (`0x141f982b0`) is either set by the `.arm` parser or computed (blank room whose slots are as walkable as its ground); the settle uses the room's own ground (+0x13a) when it is set. The world maps' tagged label rooms have the arm's second bool set, but using that bool as +10 lost 10 G4_WorldMap jobs: open.
- **Corridor height marking of asymmetric door types** (solves the door-pick class `0x141fa81a3` of G3_11, G3_12, G3_16: +900, all three now match fully): `0x1420a2fb0` (called by `connect` `0x1420a3a20` at `0x1420a3f20` and by the first loop of the branch pass `0x1420a5680`, arguments: next room towards the start, the inner room, the previous room, the edge's own door type `+0x38`, its chirality `+0x41`) marks every door-less side of the inner room only for a symmetric type. For a type that `0x141d047b0` calls asymmetric (height change `+0x4c` set, left/right grounds `+0x20`/`+0x28` differ, or a virtual type whose split types differ, or whose shared split type is itself asymmetric with equal chirality bits) it takes the ring side of the door to the next room (`0x141fa7d40` with the chirality flipped, first position) and to the previous room, and per ground that is a wildcard or has `.gt` flag 1 (left with `1 + 2·!chir`, right with `1 + 2·chir`) marks side `(k + side) & 3` through `0x1420a34c0`. That function checks one ring side for doors and marks a different strip: ring top marks the strip above, ring right the strip below, ring bottom the strip left, ring left the strip right (`0x1420a3430`). The port marked all sides, so placeholders next to an outer-edge corridor (G3_11 edge 0, `aggoratouteredge.et`, two grounds) took height 0 as a second corridor height and `branch_starts` (`0x1420a4a10`: exactly one height, a byte vector) dropped them: one branch start fewer, the next door pick drew `below(3)` where the client drew `below(1)`. Port: `dungeon.rs` `asymmetric`, `side_marks`, `mark_around`, `mark_side`, `mark_strip`; `POE_OLD_MARK=1` restores the old marking. Debug: `POE_DBG_DOOR` (door picks, branch starts with the draw index), `POE_DBG_HMARK=x,y` (who marks a cell's room).
- **De-repetition skips wildcard-edged uniform tiles** (class `0x141fdfd6d`, G4_11_2: +345, 600 of 600 match): the scan `0x141fdd7c0` skips a tile whose key `0x141fd5c50` calls uniform: every edge empty or of a type with `.et` flag 0x80 (wildcard name, `0x141ef23f0`) and one ground on all four corners. The port required no edge at all, so the `Whare_fill_02.tdt` stack of G4_11_2 (6x3, `wildcard_double.et` on two sides, one ground) was re-picked where the client leaves it, one extra re-pick draw. Only the de-repetition call site uses the wildcard rule in the port (`generate.rs` `derepeat_pass`); the room pass and fill keep `uniform_ground`. Debug: `POE_DBG_REPICK` (every re-pick with its candidates and filters).
- **`(best)` room pick: door-less exits of any height** (ExpeditionSubArea_Catacomb, first draw `0x1420aebc9`: client `below(24)`, port `below(8)`; +276, 400 of 400 match): the area's three `feature` rooms have no doors; `Feature_02.arm` / `Feature_03.arm` (version 36) give their door-less exits the `.arm` height values -1..-4 (override flag off; the two exits with the flag on have 0). `GetExactRooms` (`0x141fc0350`) compares door, height byte `+8` and chirality `+9` per side, yet the client offers all three rooms in all 8 orientations, so its stored height of these exits must be 0 or ignored. Empirical rule in the port (`exact_sides`): a door-less side matches a door-less exit of any height. Not confirmed in the client's `.arm` exit parser (a visitor of `0x142931cf0`, vtable `0x1434e9e40`); the broad variants lose sample jobs: height 0 for every door-less exit 987 of 15,000, height 0 when the override flag (bool 1, version > 29) is off 1,855. Debug: `POE_DBG_THINGY` (every `.arm` exit), `POE_DBG_EXACT` (`(best)` candidates with their exits).
- **Side slots after the prune** (ExpeditionLogBook_Basin, draw 28 at `0x1420aff12`: client `below(19)`, port `below(72)`): the typeless room size pick (`0x1420afdd0` → `0x1420afa40`) takes the edge type of the first of the node's four slots (`node +0x58..+0x78`, edge `+0x38`) that differs from the default type. `dungeon::layout` rebuilt the slots in edge-index order; an edge the prune merged (`remove_node`, `0x141fec1c0`) sits in its new node's first free slot, which can come after higher edge indices (Basin node 1: slots 10, 13, then the merged thickwall edge 1). The layout now reads `nodes[n].edges` (the slots the prune keeps) and appends any other touching edge by index; `POE_SLOTS_BY_INDEX=1` restores the old order. Basin's draws are now in step; the last diff was the overlay room height below.
- **Overlay room height counts fill slots** (`0x141fa10b0`, ExpeditionLogBook_Basin +307, 372 of 372 match): the client walks every slot of the room grid (room data `+0x168`, 0x78 bytes each) whose flags `+0x70` / `+0x71` are clear, not only `k` slots, and subtracts the corner height (`+0x42`) and the slot height (`+0x72`) from the terrain height. `thing_02.arm` (all `f` slots, root `K 3x3`) had no `k` slot, so the port voted nothing and placed it at 0; the client's fill slots (1x1, no grounds, height 0) agree on the terrain's 1. Port: `overlay.rs` `room_height` takes `f` slots as 1x1 ground-less slots at height 0 (`POE_RH_K_ONLY=1` restores the old rule). Which slot kinds set `+0x70` / `+0x71` (`n`, `s`, `o`) is not checked; the slot height `+0x72` is not ported (0 for every slot seen so far). Debug: `POE_DBG_RH=1` (every call), `POE_DBG_RH=x,y` (its corners).
