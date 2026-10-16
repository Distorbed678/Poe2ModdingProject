# Dungeon layouts (`.dgr`, 0.5.5e)

Most act 1 areas (G1_3, G1_6, G1_7, G1_8, G1_9, G1_11, G1_12, G1_13–15) use `.dgr` topologies
instead of `.tgr`. The topology choice, the graph load and the node-existence rolls are shared
with `.tgr` (see `terrain.md`); after them the client runs a separate dungeon layout. Port:
not started (this file records the reverse engineering as it goes).

## Call chain

- `0x141ff05f0` (`.dgr` subgraph driver, per subgraph index):
  - loads the graph (`0x141fa9590`), runs the pruning/existence passes (`0x141fea2e0`,
    `0x141fea420`, `0x141ff03e0`, `0x141fea890`, `0x141fe8d70`), `0x141fff900`;
  - region size: graph width × (master +0x149 − master +0x14b) + master +0x14b, same for height.
    Master +0x149 is `RoomUnitSize` (G1_3: 9), +0x14b the shared border (1). G1_3 (5×6 units):
    41×49 tiles, 51×59 with `OuterBufferSize 5`;
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
buffer and de-repetition as for `.tgr`.
