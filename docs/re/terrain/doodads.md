# Doodads and the collision grid (0.5.5e Hotfix 1)

The two passes that run after the tile layout: the doodad pass (which objects the rooms bring,
the doodad hash) and the collision grid (the four 4-bit cell layers the server needs for walkability).
Port: `modules/server/crates/poe-world/src/{doodads,collision,tdt}.rs`; checks: the examples
`check_doodads`, `check_grids`, `forced_verify` ([README.md](README.md#tools)). Status and numbers:
[../../status/terrain.md](../../status/terrain.md). Reference data: the DLL area exports
(`doodads.tsv`, `grids.bin`) in `dumps/0.5.5e.hotfix1/terrain/area-exports/`.

## Doodad pass

Order in `TerrainGen_Run` (see [README.md](README.md#phases-terraingen_run-in-order)): after the
tile layout (and its random draws), `TerrainGen_Doodads` `0x141ff56d0` walks every region's room
placements (the list the de-repetition pass also uses, region +0x58, 0x38-byte entries) and calls
`TerrainGen_TileDoodads` `0x141ff3490` for each. The port places the doodads of rooms (`.arm`) only; all 303
tile-matching exports agree with that.

1. **Cell and angle** (`0x141fe7a50`). An `.arm` doodad entry has a cell `(ex, ey)` inside the
   room, an angle (`radians1`), a scale, a flip flag, an object type (`stub`, e.g.
   `Metadata/MiscellaneousObjects/Doodad`) and an `.ao` path. The placed cell is the entry cell
   turned by the placement's orientation `o` within the room's `w x h` tiles, plus the placement
   corner times 23. The turn is the table at `0x142f02118` (three bytes per orientation, 9 rows),
   which picks x and y from `(w*23 - ex - 1, ex, h*23 - ey - 1, ey)`: `turn_cell`. The angle:
   orientations 4..7 (mirrored) negate it (`2pi - a`), each quarter turn (`o & 3`) adds `pi/2`,
   wrapped into `0..=2pi`: `turn_angle`.
2. **AOSet pick** (`0x141fe76a0`, orientation pick `0x1421c3f40`). The `.ao` file is looked up
   (component `AOSet` merged over the `extends` chain, base first; `FUN_141d5af20` finds the
   component). While the file has `ao = "<weight> <path>"` choices and no `fixed_ao`, one
   `TerrainRng_Below(total weight)` draw picks the next file (`0x141fe7737`); the path `null`
   drops the entry (no object). A file without `AOSet`, or with `fixed_ao` entries, ends the
   chain (the leaf); then the last `AOSet` seen draws, in this order and only when it uses the
   feature:
   - rotation, when `random_rotation` (range `-pi..pi` without min/max) or a min/max range is set:
     `below(0x200)`, angle `(max - min) * r / 512 + min`;
   - orientation, when `random_orientation` / `random_flip` set the allowed mask at +0x5c (`0xff`
     / `0x11`): `below(0x348)`, then `orientation_pick(mask, u)` (mask 0 or 0xff: `u & 7`, else
     the `u % n`th allowed bit); the angle is turned by it and orientations 4..7 mirror;
   - scale, when the range is not `1..1`: `below(0x200)`, `(max - min) * u / 512 + min`;
   - translation, when `random_translate_radius > 0`: `below(0x200)` for the distance
     `sqrt(r^2 * u / 512)` and `below(0x200)` for the angle `u / 512 * 2pi`; the offset is
     `floor(d*sin + 0.5), floor(-d*cos + 0.5)`.
   The pick adds its angle and offset to the entry, mirrors the hull (`flip = placement mirrored
   XOR pick mirrored XOR entry flip`) and multiplies the scale. The draws continue the layout's
   MT stream in placement order (the client's generator trace shows the draws after the layout's).
3. **Hash filter** (`+0x2d0` / `+0x2d4` of the object type's data, `0x141ff3490`). An entry adds
   `y + Murmur2_Utf16(.ao path) + (x << 16)` (cells, the path as the room names it, not the
   leaf) to the doodad hash ([README.md](README.md#hashes)) when its object type exists (it has an
   `.ot` of its own; a path with only an `.ao` extending another type is none) and the type's merged `BaseEvents` sets neither
   `server_only` nor `client_only` and no `league_category` (a type with one counts only when it
   equals the area's league category, 0 for areas without a league mechanic rolled): `type_counts`.
4. **Static flag.** Only types whose `Positioned.static` is true stamp a hull into the
   collision grid (`type_solid`); the others (monsters, markers) do not.

Result: all 303 tile-matching exports of the checked batches equal `doodads.tsv` (list and hash)
and the client's doodad hash. The exception: `MapVaalVault` seed 35611400 topology 1 orientation
0 has two angles off by `pi/2` (the `random_orientation` pick, see "Results and open points" below). The hash does not
include the angle, so that pick difference is invisible to it.

## Collision grid (`0x141d0e770`)

The terrain object holds four 4-bit cell layers (terrain `+0xd0`, `+0xe8`, `+0x100`, `+0x118`, one
nibble per cell, 23 x 23 cells per tile; exported as `grids.bin`, the `PWG1` format of
`poe_game::grid::Grid`: magic, cells x, cells y, bytes per row, layer count, then each layer 4 bits
per cell, low nibble first). Every layer starts at 5 ("open"). A cell value is a clearance:
0 blocked, 1..4 distance to the nearest blocked cell, 5 open. Layer 0 is the walkability layer
(the server's `walkable`: value above 0), layer 1 the projectile layer; layers 2 and 3 stay at 5 in
every export checked (both exports and port).

`collision::build(map, files, doodads)` does what `0x141d0e770` does, in two stages:

**Tiles.** Row by row over the tile records (`ty` outer):
- the record's sub-tile (`.tdt` sub-tile at the record's index; `SubTile.flags`, `b1`, the LZ4
  list) gives a plane of cell codes (low nibble layer 0, high nibble layer 1) over
  `x0..x0+w, y0..y0+h` of the 23 x 23 tile (planes may reach 4 cells past a side: flag 0x20 up,
  0x40 right, 0x80 down, `b1` bit 0 left);
- flags 1 and 2 both set: the **default plane** (`0x141d05050` + `0x142917610`): all cells 0 on a
  layer whose blocked bit is set (flag 8 layer 0, flag 0x10 layer 1), else open, with 1..4 ramps in
  the margins that are present;
- otherwise plane number `i` of the sub-tile's LZ4 stack (`0x141d051c0`), where `i` is the first
  entry of the `.tdt` hull-plane masks (data +0x173; mask bit per orientation, 0 = all) that
  serves the record's orientation;
- the plane is turned by the orientation (the same `0x142f02118` table, on a 23-cell tile) and
  written as the minimum with what is there (5 counts as unset);
- the sub-tile flags decide what is written: flag 2 clear: with flag 1 clear and fewer than 4
  unwalkable corners both layers take the plane, else layer 0 is blocked over the tile if flag 8 is
  set or all four corners are unwalkable, and only layer 1 takes the plane; flag 2 set: with flag
  0x10 both layers are blocked over the tile, else (flag 1 clear, fewer than 4 unwalkable corners)
  layer 0 takes the plane, else layer 0 is blocked if flag 8 or all four corners are unwalkable;
- a record without a sub-tile blocks both layers over the whole tile;
- **blocking a whole tile** (`0x141d0f610`, `0x141d0fbe0`) also reaches 4 cells into each of the
  four neighbours that has a sub-tile open on that layer, with the default plane's 1..4 ramp;
- a corner is **unwalkable** when it lies outside the map or its lattice flags (the corner-lattice
  of [shared-passes.md](shared-passes.md#corner-lattice)) have bit 8 set.

**Doodad hulls** (`0x141d10650`). Each placed static object stamps the `Hull` block of its leaf
`.ao` (merged over `extends`, base first). A block is a list of sections (`section = ...` starts a
new one) of `walk_tri x0 y0 x1 y1 x2 y2`, `proj_tri ...`, `walk_circle x y r` and
`proj_circle x y r`. The transform is rotation by the doodad angle times the scale, mirrored in x
when `flip`. Walk shapes go to layer 0, projectile shapes to layers 0 and 1 (`stamp_hull`):
- a triangle (`0x141d10d60`) zeroes the cells it covers (the 4-bit fixed-point scan of
  `0x141d1d3d0` / `0x141d18ee0` with the top-left rule), zeroes each corner and stamps the 48-entry
  corner kernel `0x1434bfc30` (values 1..4 around the corner), and for each edge stamps the
  distance (rounded perpendicular distance, up to 4 cells) over two triangles on the outer side
  of the edge;
- a circle (`0x141d194f0`) lowers every cell within `r + 4` to `ceil(distance - r)`;
- all writes take the minimum (5 counts as unset; 0 always writes), so the order of the doodads
  does not matter.

## Tools and checks

- `check_doodads <export or run dirs> [--only forced.tsv]`: generates each export's job and
  compares the doodad set `(x, y, path)` and the hash with `doodads.tsv` and `meta.txt`
  (`--kinds`: per object type how many the client counted; `--verbose`: first differences).
- `check_grids <dirs> [--only F] [--blame] [--dump DIR]`: compares the four layers with `grids.bin`
  (differing cells per layer, bounding box of the first difference; `--blame` attributes cells
  to the tile or doodad that wrote them; `--dump` writes the port's `PWG1` bytes).
- `forced_verify` reports `doodads match|differ` after the tile hash.
- Debug variables: `POE_DBG_DOODADS`, `POE_DBG_DOODAD_ORDER` (picks and draws), `POE_DBG_HULLS`,
  `POE_DBG_GRID_TILE=x,y` (one tile's flags, corners and plane).

## Server use

`[terrain] generated_grid` (default off) makes the instance server build the grid with the port
instead of using the grid the client uploaded or the layout cache holds, but only when the
generated tile hash equals the client's; otherwise it warns and keeps the uploaded grid.
Dependency decision: `poe-world` depends neither on `poe-game` nor on `poe-config`; the instance
server is the adapter (`poe_world::collision::build(..).pwg1()` into `poe_game::grid::Grid::parse`),
see [../../architecture/instance-world.md](../../architecture/instance-world.md#terrain-grid-sources).

## Results and open points

### Results

- Doodads: 302 of 303 tile-matching exports equal in set, hash and draws; the one exception is the
  `MapVaalVault` angle case below. `forced_verify` over the batch compares the doodad hash of every
  tile match: all 72,718 tile matches of the 50-seed batch (218,705 client layouts, 4,508 combinations) also match the client's doodad hash (2026-10-22; `runtime/forced-f.tsv`).
- Collision grid: 273 of 303 exports equal `grids.bin` in all four layers (layers 2 and 3 in all
  303). The 30 that differ are listed by family below.
- Object types: a kind counts, stamps a hull and is placed only if it has an `.ot` of its own. A
  path that only has an `.ao` (G1_3's `Metadata/Chests/SirenEggs/SirenEgg_02` extends
  `Metadata/Parent`) is not an object type: counting it broke the doodad hash of every G1_3 layout
  (1,600 of 1,600). Type flags are read once per process; reading them through `GameData` took its
  bundle lock on every layout and made the batch threads run one at a time.

### Open: the 30 grid failures (all differ in layer 0 unless noted)

Found with `check_grids --blame` (tile flags of the differing cells, `WORST` lines: the tiles with
most differing cells), `POE_DBG_GRID_TILE=x,y` (a tile's sub-tile flags, corner count and plane),
`lattice_probe <export> x0 y0 x1 y1` (the corner lattice around a tile range: ground letter and
flags per corner) and `POE_DBG_NEAR=x,y,r` (the port's doodads near a cell). Client data to settle
them is requested in `runtime/f/client-requests.txt` (traces with the lattice dumps; exports with
`entities.tsv`).

| Family (exports) | Cells | What differs | Findings |
|---|---|---|---|
| `MapSlash` (6) | 771 each | A pocket of three by three tiles around a `CvSM_01.tdt` tile is open in the port and fully blocked in the client | Forcing all nine tiles to four unwalkable corners (a 4 x 4 block of lattice corners with flag 8, so every `clear()` margin falls where the client has it) reproduces `grids.bin` exactly, so the cause is the lattice, not the planes. The pocket's inner corners have no ground in the port's lattice (`-3`), their neighbours are mud. `0x141fe0cc0` (ported, `lattice::mark_enclosed_fills`) is not the cause: it needs a uniform edgeless tile whose outline is already flagged. Suspects: the third unported part of `0x141fe1150`, the tile-set option `+0x165` block (tiles with `+0x16c & 0x40` whose four corner grounds equal the tile-set's `+0x230`, which flags the whole footprint), or flood paths blocked by the stubbed `sealed_inside` (tile data `+0x16f` bit 0, read by `0x141fe2500` and `0x141fe1f70` through the record at the corner's own index) |
| `MapSeepage` (5), `MapUberBoss_IronCitadel` (2, partly) | 250 to 690 | The port blocks more than the client: whole tiles cleared with the margin ramp, or a wall plane 4 cells longer | The opposite direction to `MapSlash`: the port flags corners the client does not, which fits the `sealed_inside` stub (a sealed tile stops the flood). The `.tdt` field behind `+0x16f` is not mapped yet (the flag word of `0x142915630` is set bit by bit through a vtable slot `+0x40`) |
| `MapCrimsonShores` (8) | 400 to 1,800 | A circular blob of radius about 10 with the 1..4 clearance ramp is blocked in the client, absent in the port, centred in a `fill_01.tdt` tile (3 x 3, flags 0x03, plane-less) | Not a tile plane and not one of the room doodads (none near it). Probably a generator object that is not in `doodads.tsv` (an entity from tile data `+0x1b8`, or a scatter object); an export with `entities.tsv` of a failing layout decides |
| `MapMonument` (2), `MapFortress`, `G4_5_2`, `G2_10_1` (2), `MapReservoir` (2) | 150 to 4,900 | Feature tiles (`PrecursorGroundAnchor_*`, `fortress_boss_01`, `Warden_Office`, `JamanraRitualSite_02`) and fill tiles | Not analysed beyond the tile names; the same two suspects (unported unwalkable-corner rules, objects outside `doodads.tsv`) |
| `MapVaalVault` (1) | 53 | Two doodads (`SunkenPillar_set.ao`, `random_orientation`, `AOSet` with one choice) have the angle off by pi/2 (ours 3pi and 2.5pi after the pick, the client 2.5pi and 2pi) | `0x1421c3f40` is ported as decompiled (`mask 0xff`: `u & 7`) and the picks of all other sets (`random_flip`, 1,879 doodads) agree, so the draw `u` (99 and 314, `below(0x348)`) or the mapping from the picked orientation to the angle differs for the full eight-way case; two samples are not enough. A client trace of this layout (`MapVaalVault 35611400 1 0`) shows the real draws |

