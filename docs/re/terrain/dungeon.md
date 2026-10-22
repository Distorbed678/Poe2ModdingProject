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
   `0x14209dc90` (3×3 spots) is not ported.
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
