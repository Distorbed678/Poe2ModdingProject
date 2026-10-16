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

## Trace landmarks (G1_3 seed 1001)

The draws match the `.tgr` path for the first 11 (existence rolls `0x141feeb10`); the dungeon
starts at draw 11 (`0x1420aebc9`). Later call sites: `0x1420aff12` (`below(2964)`),
`0x1420a62b0`, `0x1420a5f8a`, `0x141fa81a3`, `0x1420b125c`/`0x1420b134c`, `0x1420a4e94`/
`0x1420a4ec7`, `0x1420a9b9f`, then the tile pass (`0x142098d18`, `0x14209f8b8`), fill, outer
buffer and de-repetition as for `.tgr`.
