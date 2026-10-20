# Terrain generation (0.5.5e)

What the client's terrain generator does, as far as it is known, and the tools for porting it
to the server (`crates/poe-world`, the terrain generator). Addresses are Ghidra addresses (image
base `0x140000000`). How far the port has come is in [../../status/terrain.md](../../status/terrain.md).

| File | Contents |
|---|---|
| this file | Where generation runs, phases, RNG, hashes, tile records, doodads, tools, the area data chain, a town trace |
| [tgr.md](tgr.md) | `.tgr` graph areas: generation order, node jitter, room placement, routing, smoothing, virtual edges, node tiles, edge tiles |
| [dungeon.md](dungeon.md) | `.dgr` dungeon areas (most of act 1): call chain, file fields, rooms, corridors, tile pass |
| [shared-passes.md](shared-passes.md) | Passes both kinds use: topology choice, pruning, region orientation, lattice, fill, de-repetition, outer buffer, overlays |

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
  with `Murmur2_Bytes`. The DLL recomputes it from its export (the self-check in `meta.txt`).
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

A tile is 23×23 cells (see Cell grids below).

### Cell grids

The terrain object holds four cell grids (the minimap page of the DLL draws them; see
[../../architecture/client-dll.md](../../architecture/client-dll.md)). Found with the minimap
(2026-10-14): the player walked onto a cell where grid 0 (`terrain+0xd0`) is 0 while grids 1-3 are 5,
and grid 1 shows one connected open area around the player. The server's spawn code treats grid 0
non-zero as walkable; grid 1 is the likelier walkability grid. Check this before building
collision. The corner-lattice flags that give walkability are in
[shared-passes.md](shared-passes.md#corner-lattice).

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

## Client-side facts (offline analysis, 2026-10-13)

- **Seeds:** towns and hideouts always use seed 1 (all 27 in the live log; `WorldArea::fixed_seed`).
  With seed 1 the tile hash is constant (`G1_town` gives `892597717`; loading it on our server
  should log `Tile hash: 892597717`), but the doodad hash varies between visits, so doodads depend on
  state such as quests.
- **Terrain object:** embedded in the world at +0x8d0. Size in tiles at +0x18 (x) and +0x20 (y);
  0x38-byte tile records at +0x28 in row-major order (`y * w + x`, as in `0x141d12f40`). Record +8 is
  the tile object, whose +8 is the `.tdt` path. The hashes are stored by `0x14167b270` (world
  +0x114 and +0x118, above); world-map loads (area kinds 0x20/0x21/0x23) log no hashes. The DLL dumps the generated tile grid (raw records plus reachable
  `Metadata/` and `Art/` paths) to `dumps/0.5.5e/client-tables/terrain_tiles.txt` when the player
  spawns.
- **Late tiles:** the log line `Tried to SetExtraActivationRangeAroundTile() after level generation
  was complete` (`0x141d12f40`) is client-side noise: it widens the activation range of terrain
  tiles for objects that load late. It is not a server message.
- **Start position:** the Riverbank (`G1_1`) topologies are
  `Metadata/Terrain/Gallows/Act1/1_1/Graphs/intro_{1..6}.tgr`; their nodes name rooms
  (`tutorial1_river` is the washed-up start, then `entrance`, `source`/`sink`, `lumberjack`). The
  grey view with a black bar seen in early sessions was a player standing on a non-walkable spot:
  filler tiles such as `driedriver_fill` outside the map, or the centre of `Intro_Tree_01` in rotated
  layouts (`ZombieIntroFeature_Reorient_*`). On walkable ground the area renders. How the server
  picks the start cell from the exported layout: [../../architecture/instance-world.md](../../architecture/instance-world.md).
- **Environment of the Riverbank** (`Metadata/EnvironmentSettings/Gallows/1_1.env`): camera far
  plane 3639, clouds that fade only within 500 units of the player (`clouds_fade_radius`), rain
  attached to the player.
- **Height:** the server never sends an entity's height. `0x141de5390` (Positioned "get position
  height") samples the terrain at the world position (`0x141d0d420`/`0x141d0d650`) unless an
  explicit height is set (+0x440/+0x48c, Positioned flags0 bit 2).

## Tools

- **DLL area export** (`[features] area_export`, on by default, also read-only): every
  generated area goes to `dumps/0.5.5e/terrain/area-exports/<session>/<n>_<hash16>_<seed>/`
  (`dump_dir`; before 2026-10-20 it was `GameFiles/logs/poe-hook/<session>/areas/`):
  `meta.txt` (hashes, size, and the recomputed tile and doodad hashes with match/MISMATCH),
  `layout.tsv`, `doodads.tsv`, `grids.bin` (`PWG1` format), `entities.tsv`, `probe.tsv`
  (walkability probe: per visited cell, each grid layer's value; a walkability layer is never 0
  under the player). "Export area" in the Minimap tab rewrites entities (with the server's) and
  the probe.
- **Generator trace** (`[features] generator_trace` or `POE_HOOK_GEN_TRACE=1`):
  `generator_trace.tsv` in the area export, every seed, raw MT draw, bounded draw (with the
  calling functions) and phase in order. Diff it against the server port to find the first
  divergence. A Riverbank layout is ~23,000 lines.
- **Seed sweep:** the server's `[dev] sweep` (or `POE_SWEEP=<file>`) lists
  `<WorldAreas.Id> <seed> [count]`. Each instance session then generates the next entry instead
  of the character's area, appends `area, seed, tile hash, doodad hash` to `sweep_results`
  (`dumps/0.5.5e/terrain/vectors/sweep-results.tsv`) and closes. `scripts/re/sweep_drive.py` logs the test client in again
  and presses Play for each entry (about 20 s per layout); the DLL exports every layout. A second
  AreaInfo on one connection crashes the client (the old objects sample the regenerated terrain,
  `0x141d04eb0`), hence one layout per session.
- **Dev command** `area <id> <seed>` (`scripts/re/send_dev.py auto area G1_1 12345`) sends a new
  AreaInfo on a live connection: it crashes the client for the reason above; kept for experiments.
- **Port examples** (`cargo run --release -p poe-world --example <name> -- <args>`), which compare
  the port with client data:
  - `area_trace <area> <seed> <trace dir> [topo[:o] | all]`: draws, layout and tile hash against a
    client trace in `dumps/0.5.5e/terrain/traces/act1_traces/<area>_<seed>/`; `all` searches every
    topology and orientation.
  - `area_vectors <vectors.tsv> all [area]`: regression against client tile hashes (for example
    `dumps/0.5.5e/terrain/vectors/sweep-vectors.tsv`). It tries forced topologies when the natural
    choice fails.
  - `check_exports`: recomputes the tile and doodad hashes of every DLL area export.
  - `town_check <town export> 1 <trace>`: the room-pass placements of Clearfell Encampment.
  - `river_gen <export> 1000`: Riverbank graph layout (jitter, room nodes, routing).
  - `smooth_check <trace>`: replays the logged arc steps of edge smoothing.
  - `wiggle_check <generator_trace.tsv> <stop line>`: which client tiles the exit wiggle could move.
  - `tdt_info`: a `.tdt` header.
- **Debug environment variables** of the port: `POE_DBG` (rooms, stuck height repair),
  `POE_DBG_WIGGLE` (exit wiggle keys and placements), `POE_DBG_KEYS`, `POE_DBG_SLOTS`,
  `POE_DBG_GRID`, `POE_DBG_PATHS`, `POE_DUMP_DRAWS`, `POE_DUMP_PIECES=path`, `POE_LAYOUT_DIFFS=n`
  (`area_trace`).
- Ghidra access and the other RE scripts: [../README.md](../README.md).

## Port crate map

`crates/poe-world`, checked against the client by the examples above:
- `rng`: `std::mt19937` and MSVC `uniform_int`, equal to the traced draws (unit tests).
- `hash`: the tile and doodad hashes.
- `graph` (`.tgr`), `tdt` (`.tdt` header: size in tiles, edge types per side, ground types per
  corner). `.arm`, `.tst`, `.gft`, `.tsi`, `.et`, `.gt` come from `poe_data_tools`.
- `tileset` (the tile-set index the generator looks keys up in), `export` (reads DLL area exports), `area`, `generate`, `region`, `layout`, `voronoi`, `rooms`, `routing`, `smoothing`, `split`,
  `node_tiles`, `edge_tiles`, `lattice`, `fill`, `outer_buffer`, `overlay`: the passes in
  [tgr.md](tgr.md) and [shared-passes.md](shared-passes.md); `dungeon`, `dungeon_tiles`:
  [dungeon.md](dungeon.md).
- A `.tdt` tile spans `size` tiles of 23×23 cells (most are 3×3, the encampment 15×18); each
  covered tile record names the `.tdt` and its sub-tile index.

## Area data chain

WorldAreas `Topologies` → Topologies `DGRFile` (`.tgr`; an area with several topologies picks
one) → the graph's `MasterFile` (`.tsi`: `TileSet` `.tst`, `FillTiles` `.gft`, `RoomSet` `.rs`,
materials) → rooms (`.arm`) placed at graph nodes, edges (`.et`) routed between them, the rest
filled. Clearfell Encampment (`G1_town`): one topology `town_W_1.tgr` (33×30 tiles, one node
"town" at cell (12, 12)), the room `town_1.arm` covers the whole map; the tile hash changes with
the seed (fill tiles and orientations are random), the doodad hash does not.

## Town trace (seed 1, about 1,860 draws)

24 draws in prepare (seed 0), the real seed, one draw over the whole u32 range (`0x141ff9609`),
one `n=8` (orientation), ~40 small draws (`n` 1/2/8, `0x141f8ed4d`), 990 draws of `n=1024`, then
pairs of a weighted fill-tile pick (`0x141f900a7`; `n` = the fill group's total weight, 300 for
`default` = `Blank.tdt`, 3 for the three `Slash` fills) and an orientation (`0x141f90de4`, `n=8`).
