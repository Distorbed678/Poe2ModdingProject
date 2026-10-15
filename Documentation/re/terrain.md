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
