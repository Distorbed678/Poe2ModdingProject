# Passes shared by `.tgr` and `.dgr` areas

## Topology choice

Generator vtable slot 0 `0x141ff9280`: candidate topologies
`0x141fdfe30(world area, world+0xb8)`, filtered `0x141fa6440`; a sub-seed
`v = below(0xffffffff)` (not for area kind 0x21), then the index from an LCG seeded with
`v` (`0x141025cf0`: `x = x·1103515245 + 12345`, take `(x >> 16) & 0x7fff` in chunks of up
to 15 bits until it has `bits(n-1)` bits, reject ≥ n; n < 2 → 0 without stepping).
`0x141ff89e0` loads the chosen graph (another `below(count)` and `below(0x348)` only when no
topology was passed); `0x141ff8310` turns the graph into a region record (0x98 bytes, in the
generation state's region list).
Details (`modules/server/crates/poe-world/src/area.rs` `topologies`, `generate_with`):
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
  are none does `0x141ff90f0` pick from the whole list. **Ported** (2026-10-28, Poe2Server
  `poe-world/src/map_dirs.rs`, see "The natural choice as ported" below): `G1_2` seed 1001
  picks topology 15 (`felling_S_N_W`) like the client (live load through our server), where
  the plain index gives 3.
- **World-map pin directions in detail** (2026-10-27/28, decompiles; ported and checked against
  4,173 unforced client generations, below):
  - **Ground truth from `gen_batch` (2026-10-28, client 0.5.5e.hotfix1, runs `w12-pins2` and
    `w12-pins3` under `dumps/<version>/terrain/batch/`):** town jobs with topology 0
    (`G1_town`..`G4_town`, traced, listed in `export.txt`) do run the prepare phase: every trace
    has the act's `map_pins` (G1 18 pins, G2 23, G3 19, G4 20; `index x y area`, e.g. G1_1 at
    (424, 518), G1_town (484, 463), G1_15 (485, 291)). **The positions are the same for every
    area seed** (seeds 1, 2, 7, 123456, 987654321, 4000000000 for G1 and G3): the layout seed is
    the generation state's (`state +0`), which `gen_batch` keeps constant, or the world-map
    layout has one outcome; which of the two is open (run a job with another state seed, or
    read `0x141fa5900`'s seed argument at the call). `pin_dirs` (`state +0x70` at the topology
    checkpoint) is empty (count 0) in all town jobs, so the town's direction list is either
    built after that checkpoint or not in a throw-away world; open. The earlier statement that
    `gen_batch` runs no town job with pins was wrong (it did not trace one).
    The layout's seed is the generation state's (`0x141ffe7f0`: `r8d = [[gen +0x30]]`, i.e.
    state `+0`, passed to `0x141fa5900`, which seeds with it at `0x141fa5bd9`); the trace's
    `seed 141fa5bd9` line shows **0** in the batch jobs and also in a **live** G1_town load through
    our server (`scripts/test-run.sh`, area export `2026-10-28_16-10-31_336/01_3f70_1` under
    `dumps/<version>/terrain/area-exports/`), with the same 18 act 1 pins and `pin_dirs` count 0.
    So with our server the pins are fixed per act. Open: whether official servers make the
    state seed non-zero (what writes state `+0`, or a traced official town load, the user's), and
    when `+0x70` gets its entries.
  - **How the pin positions are made (2026-10-28, client 0.5.5e.hotfix1 addresses; 0.5.5e =
    hotfix − 0x180 in this range, `dumps/0.5.5e.hotfix1/addr_map.tsv`).** The layout `0x141fa5a80`
    does not run the generator: the `.dgr` walk `0x141fb8b50` (`.tgr`: `0x141fba130`) visits the
    world-map graph's nodes; a `graph` node recurses into its sub-graph (offset by its position);
    every other node (key at node `+0x20`, orientation byte `+0x40` < 8, position `+0x4c`/`+0x50`
    in cells) looks up the rooms of its key (`0x141f9bb90`, entries of 0x20 bytes) and runs the
    callback `0x141fb8fc0` on each **whose orientation equals the node's**:
    - base = node position `/23 · (graph +0x149 − +0x14b) · 23` + margins (`+0x174`, `+0x171`) · 23 +
      the sub-graph offset;
    - the room's object entries (`.arm`, room `+0x70`, 0x58 bytes, type at `+0x30`) whose type is a
      pin `MetadataId`, filtered by the room's object groups (`+0x88`), turned within the room's
      W × H (`+0x138`/`+0x139`) by the node orientation (table `0x142f02118` = 0.5.5e `0x142f020c8`,
      our `turn_cell`) plus base;
    - when the graph has a tile set (`+0x228`): each tile-key cell of the room (0x78 bytes at room
      `+0x168`, skipped when `+0x70`/`+0x71` are set, key at `+0x50`) takes the **first** tile of
      that key, and the callback `0x141fb9c50` adds that tile's `.tmd` objects (tile data `+0x180`):
      the tile's orientation is the first of the 8 in table `0x142f0bf48` whose oriented key
      (`0x141d08bc0`, 0x4e bytes) equals the cell's, composed with the node orientation
      (`0x141d01080`); the object's cell is turned by that within the tile's W × H (`+0x60`/`+0x61`),
      the room cell (plus its sub-tile offset `+0x20`/`+0x21`) by the node orientation within the
      room, × 23, plus base.
    So every room variant of a node contributes candidates (MudBurrow: 5). Then per pin type
    `0x141fa4640` → `0x141fa3fc0` (0.5.5e `0x141fa44c0` → `0x141fa3e40`): the non-empty positions
    sorted by (x, y), seed `state seed ^ [type +0x28] +8`, which is **MurmurHash2 (seed 0) of the
    `MetadataId` as written** (9 of 9 act 1 types with several candidates), index = the LCG draw
    (`0x141025ef0`, `x·0x41c64e6d + 0x3039`, first 15-bit chunk = high bits); that position goes
    first, the others are dropped. The pins are the act's `MapPins` rows with `WorldAreasKeys`
    (18 of 18 in act 1).
  - **Corrections and details found while porting (2026-10-28):** the walked graph is the loaded
    `.dgr` *prepared* with the seeded TerrainRng (`TerrainRng_Seed(seed, 0)` in `0x141fa5a80` right
    before the load; world maps are area kind 0x21, so no sub-seed draw comes first): random node
    offsets, comma-list room types reduced to one (`below(count)`), existence rolls, `shuffle`
    groups (act 4's `"shuffle2"` islands) and `unique` groups, as the generator does; without it
    act 4's islands trade places. The room lookup `0x141f9bb90` (→ `0x141fae1c0`) is a map by the
    node's (resolved) type, entries 0x60 bytes with the weight at `+0x58`. The tile of a tagged
    slot is the first tile of the tag; its orientation is the first one under which the tile set
    lookup of the slot's key (corner heights above the lowest, as the room pass reads them)
    gives that tile, else the node's. `0x141d01080` (= 0.5.5e `0x141d00f00`, our
    `region::compose`) also defines `compose(o, 8) = o` (the port returns 8; unused here). The
    room's `+0x88` list is the `.arm`'s doodad connections (links between the recorded objects),
    not a filter. The endgame town (`G_Endgame_Town`) lays out no world map (`map_pins` count 0).
  - **Port (Poe2Server `poe-world/src/map_pins.rs`, `0da535f`):** `act_pins(gd, files, act,
    seed)` walks as above (`graph_objects`, `area::prepared_graph`) and consolidates. Against the
    client's `map_pins` trace lines (example `map_pins <act> 0 --trace <town trace>`): acts 1, 2,
    3, 4, 6 all 102 pins (18, 23, 19, 20, 22); test `act_1_pins_are_the_clients`. Traces:
    `w12-pins2` (towns of acts 1 to 4), `w12-pins6` (P1_Town, G_Endgame_Town).
  - The direction list lives in the generation state (generator `+0x30` → `+0x70`, 0x18-byte
    entries: u8 direction, `MapPins` row, its table reference). `0x141ff9280` rebuilds it only
    when the area is a town (WorldAreas `+0x14`, IsTown) or of kind 2 (`+0x1b4`):
    `0x141fa5f30(out, area, generator +0x280, state seed (state +0), league mode)`. After a pick
    `0x141fa6c80` writes the chosen topology's direction keys (Topologies `+0x24`, 8 = keep) into
    the entries that matched the area's connections. **The state does not outlive a load**
    (2026-10-28, live trace through our server, area exports `2026-10-28_20-13-42_336`): after
    `G1_town`, `G1_2`'s topology checkpoint shows `pin_dirs` count 0, so every load starts with an
    empty list. Act areas are kind 2 (G1_2: `area_kind 0x2` in the export, 18 `map_pins` lines in
    its own trace), so they lay the pins out and rebuild the list themselves; an area that is
    neither a town nor kind 2 has no list and takes the index draw.
  - Generator `+0x280..+0x288` (0x18-byte entries i32 x, i32 y, `MapPins` row): the generator
    is built on the stack for every load (`ClientWorld_LoadArea` `0x14167b270`, client vtable
    `0x143385c70`), and its prepare phase `0x141ffe7f0` fills `+0x280` only for a town
    (WorldAreas `+0x14`) or kind 2: `0x141fa5900(out, act (WorldAreas +0x10), state seed,
    gen +0x298, league mode)`. That takes the act's world-map area (`0x142445f00(act)`), its
    topologies (`0x141fdfe30`, mode filter), picks one with the LCG `0x141025cf0` seeded with
    the seed (after `TerrainRng_Seed(seed, 0)`), loads that `.dgr` / `.tgr`, lays it out with
    the act's pins (`0x141fa57d0(act, 1)`; `.tgr` `0x141fb9fb0`, `.dgr` `0x141fb89d0`), seeds
    the result (`0x141fa44c0`) and collects the pins' positions (`0x141fa53f0`). So the pin
    positions are a seeded layout of the act's world map: deterministic from the game data
    and the state seed (no capture needed). The generation state is world `+0x4b0`
    (`TerrainGen_Construct`: world vfunc `+0x1c8`); its seed (`+0`) is the `LoadArea` word `p2`
    ([batch.md](batch.md#how-the-generator-is-run)), 0 with our server, an instance number in an
    official export (537884335).
  - The pieces of `0x141fa5900`: `Data/Balance/Acts.dat` by act number (`0x14028b0e0`, row map
    `+0x90`), its `WorldAreaMap` (row `+0x5d`) is the act's world-map area; the act's pins
    (`0x141fa57d0` → `0x142591b20`, `Data/Balance/MapPins.dat` rows with that `Act`);
    `0x141fa4280` keys each pin by its `MetadataId` (`MapPins +0x8c`) as a resource; the
    world-map layout (`0x141fb9fb0` `.tgr`, `0x141fb89d0` `.dgr`) records the cells where
    objects of those metadata ids land (0x38-byte records per resource, sorted); `0x141fa44c0`
    → `0x141fa3e40(record, seed, ...)` reduces each record to one position (error "World map did
    not properly consolidate pins of type ..."); `0x141fa53f0` pairs each pin with its record's
    first position, sorted. So the pin positions need the world map's object placement (the
    entity stage of the tile and room objects, not ported yet) before the topology choice can
    be ported.
  - `0x141fa5f30`: looks up the area's pin (a map world area → pin), builds the whole layout
    `0x141fa4da0` and copies the pin's record (0x88-byte records sorted by pin), else the plain
    neighbour list `0x141fa47b0`.
  - `0x141fa47b0(pin)`: for every other entry of `+0x280` connected to it
    (`0x141fa4b70`: a `Data/Balance/MapConnections.dat` row (pin, pin) in either order; table via
    `0x140b544e0`), the direction of (dx, dy) = other − own: dx = 0 → 2 (dy > 0) / 0 (dy < 0);
    dy = 0 → 1 (dx > 0) / 3 (dx < 0); else the angle to (1, 0) from `0x1421c4e50`
    (`c = (2·dx·127 / isqrt(dx² + dy²)) / 2` as a signed byte, `+127`, clamped to 0..254, u16
    acos table `0x142f10760` with 0x8000 = 180°), sector `(angle + 0x1000) >> 13` (0..4) mapped
    by `[1, 5, 0, 4, 3]` (dy < 0) or `[1, 6, 2, 7, 3]` (dy > 0). `isqrt` `0x14196e010` is the
    bitwise integer square root (floor). Entries sorted by pin.
  - `0x141fa4da0(pins, seed, mode)`: a neighbour list per pin with world areas (`MapPins`
    `+0x18` WorldAreasKeys); pins sorted by row, then stably by neighbour count, most first.
    Per pin in that order: candidates `0x141fa6440(first world area, its list, mode, 8)`; with
    candidates, LCG (`0x141025cf0`, seeded with `seed`) picks one; its direction keys (≠ 8)
    replace the list's directions one by one, each kept only if candidates remain. Then every
    neighbour's entry pointing back takes the opposite direction (`0x141f8c430(dir, 2)` row of
    the 9×9 table), again only if the neighbour still has candidates.
  - `0x141fa6150`: per distinct connection (by WorldAreas hash16 `+0x2a`) of the area, its pin
    (skipped when the pin lists the area itself); the direction of the list entry for that pin
    (or whose `ParentMapPin`, `MapPins +0x84`, is it).
  - Matching: a candidate is (topology, orientation) when the counts are equal and, per
    connection, `0x141f8c430(direction, turn[orientation])` equals the topology's key
    (`turn` = `0x142f0bf40` = [0, 3, 2, 1, 4, 5, 6, 7]; `0x141f8c430` is a 9×9 byte table, row =
    turn code, column = direction). Pass 2 accepts key 8, pass 3 also the adjacent directions
    (0: 4, 5; 1: 5, 6; 2: 7, 6; 3: 4, 7; 4: 0, 3; 5: 0, 1; 6: 2, 1; 7: 2, 3). Orientation 8
    (unforced) tries the topology's allowed list (`+0x34`, an entry 8 = all of 0..7).
  - **The natural choice as ported (2026-10-28, client 0.5.5e.hotfix1 addresses, 0.5.5e =
    hotfix − 0x180; Poe2Server `poe-world/src/map_dirs.rs`, `area.rs` `generate_stated`).** Read
    in full from the decompiles of `0x141ff9400`, `0x141fa60b0`, `0x141fa4f20`, `0x141fa4930`,
    `0x141fa4cf0`, `0x1421c5090`, `0x141fa65c0`, `0x141fa62d0`, `0x141fa6ae0` / `6bd0` / `6cc0`,
    `0x141fa6e00`, `0x141ff9270`, `0x141fa98e0`, `0x141fb7f00`, `0x141fb8210`:
    - Neighbour list (`0x141fa4930`): for each other pin of `+0x280` with a `MapConnections` row
      (pin, other) in either order (a map keyed by the pair), the direction of other − own,
      deduplicated and sorted by pin row; (0, 0) gives 8. The angle `0x1421c5090(a, (1, 0))` is
      `acos` from the u16 table `0x142f10780` (255 entries, 0x8000 = 180°) at index
      `(i8)(((|b|² − |b − a|² + |a|²) · 127 / max(isqrt(|a|²·|b|²), 1)) / 2) + 127`, clamped to
      0..254.
    - Layout (`0x141fa4f20`): pins with `WorldAreasKeys`, sorted by row, then a stable insertion
      sort by neighbour count, most first. Per pin: candidates of its first area with its list
      (orientation 8); with any, `lcg_index(seed, n)` with the LCG **reset to the state seed for
      every pin**; if the picked topology has a key ≠ 8 and as many keys as the list has entries,
      key i replaces entry i (index order, not connection order) unless that leaves no
      candidates. The topology's orientation is not applied to the keys. Then for each list
      entry whose pin has areas, the neighbour's entry for this pin takes `TURN[2][dir]`
      (opposite), again only while the neighbour keeps candidates.
    - The area's list (`0x141fa60b0`): the pin whose `WorldAreasKeys` holds the area (unique in
      the data), its list from the layout, else its plain neighbour list.
    - Connections (`0x141fa62d0`): `WorldAreas.Connections` in order, once per `HASH16`; the
      connected area's pin, skipped when that pin also lists the area; the first list entry whose
      pin is it or whose `ParentMapPin` (`+0x84`) is it gives the direction.
    - Candidates (`0x141fa65c0`): empty when the list is empty or the first topology has no
      keys; the client throws when no connection has a list entry. Per pass, per topology of the
      mode-filtered list, per orientation (the forced one, else the topology's list, an entry 8 =
      all eight): the connection count must equal the key count and each
      `TURN[ORIENTATION_TURN[o]][dir]` (`0x141f8c5b0`, turn codes `0x142f0bf50` = 0, 3, 2, 1, 4,
      5, 6, 7) must equal the key (pass 2: or key 8; pass 3: or an adjacent direction). The
      candidates are a **set sorted by (`Topologies` row, orientation)** (`0x141fa98e0`), not in
      list order (16 of 4,173 checks differed before this: G1_11, G2_9_1, G2_9_2, G3_7).
    - Pick (`0x141ff9400`): one `below(0xffffffff)` draw (the state seed for kind 0x21), index
      `lcg_index(v, n)` into the candidates, then the write-back `0x141fa6e00`. Without
      candidates `0x141ff9270` makes the same draw, topology `lcg_index(v, n)` and orientation
      `region::pick(mask, lcg_index(v, 0x348))` with the LCG reset to `v` for both (the port took
      the lowest allowed orientation before).
    - **Checked** against the client: run `w13-nat1` under `dumps/<version>/terrain/batch/`
      (every non-fixed area of the matrix × 10 seeds, topology 0, orientation −1 = unforced; 147
      client errors), `area_vectors --natural` on its 4,173 results: all match; and the live
      `G1_2` 1001 load. Example `map_dirs <area> [seed ...]` prints an area's list, candidates and
      picks.
- Stats on the area (generator +0x38 → +0x128, filled from AreaInfo's flags0 bit 7 stat
  records, `0x1423baa70`: varuint count, then big-endian u16 key and value per stat):
  `map_force_topology` (key 0xb92, Stats row 2961) = 1-based index into the filtered list,
  and `map_force_orientation` (0xb93) = orientation + 1. A forced topology still draws the
  `below(0xffffffff)` value. The server can use these to choose the layout itself
  (`generate_with(.., forced)`).
- `.dgr` and `.tgr` graphs take the same path (`0x141ff2c50` → `0x1420bec40`); `.dgr`
  only adds dungeon fields (`0x14292e8e0`, kept as raw tokens in `graph.rs`).

### Graph pruning

(`0x141fe9970`): node existence (`values[0]`: 0 removes, ≥100 keeps, else
`below(100)`), edge rolls, `shuffle<N>` groups (random pair swaps of room type and tags),
`unique<N>[#count]` (keeps `count` random nodes; lettered tags from session history are not
ported). Removal `0x141feb9d0`: two same-type edges at equal heights merge (`0x141fec1c0`); a
node with ≥2 edges only loses its room; otherwise `0x141fec520` retracts its edges with points
to a new node at their nearest point and erases it.

### Region orientation

(`0x141ff89e0`, `modules/server/crates/poe-world/src/region.rs`): a topology may be laid
out turned or mirrored. Table `0x142f020c8` drives the point transform (`0x141d14b30`), the
centring of even-sized placements and the alignment snap `0x141fe7400` (a remainder of exactly
half rounds by orientation). `0x141d14990` turns the graph; mirrored orientations flip the
chirality of edges whose type is not symmetric. Forced by the `map_force_orientation` stat (o+1).

The table (9 × 3 bytes): `[0,1,1] [1,0,1] [0,0,0] [1,1,0] [0,0,1] [1,0,0] [0,1,0] [1,1,1]
[0,1,1]`. One shared copy lives in `region.rs` (rows 5 and 7 were once swapped in the port).

### Region overhang

The client's region is a window into the full map (`0x141d0bad0` checks
the full map's bounds), so a tile placed with the one cell of slack writes cells and corners into
the margin. `TileMap` keeps those writes and `embed` applies them.

### Overlays (`.toy`)

(`modules/server/crates/poe-world/src/overlay.rs`; master `Overlays`, +0x200): passes with
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

### Inheriting tiles

(`0x1429153f0`): a `.tdt` with an `inherits` path (for example
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

### Corner lattice

(map +0x40 row stride, +0x50 3-byte points `(ground index, height, flags)`
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

- Border floods (`0x1420b94d0` → `0x1420b9180`, `modules/server/crates/poe-world/src/lattice.rs`): per placed
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

### Fill

(`modules/server/crates/poe-world/src/fill.rs`):
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

### De-repetition

`0x141fdd7c0`, row by row over cells, live (replacements are seen by later cells), for all rooms
including fill rooms. A tile-set tile bigger than 1×1, without a feature, with a non-uniform key
and not touching a room ground-override lattice point (`0x141fde360`) is checked against the
neighbours one tile-size away (up, right, down, left). The same tile in the mirror orientation
across the shared side (horizontal 0↔4 1↔7 2↔6 3↔5, vertical 0↔6 1↔5 2↔4 3↔7) makes
`0x141fdfa60` re-pick it from its key's candidates minus those mirroring the up/left neighbours
(`0x141fde4b0`) or that neighbour; with nothing left it stays unchanged. Room ground overrides are
the `.arm`'s last line, `(w-1)×(h-1)` string indices (not parsed by `poe_data_tools`). Tiles are
looked up by their tile-set index key (`0x141d08a40` turned; `0x141d09900` drops `.gt` flag 0x10
grounds). A re-pick (`0x141fdfa60`) that finds no candidate and
whose tile is outside every room rectangle (`0x141fde210`) tries moving an edge crossing
(`0x141fdef90`): for each other side with a non-virtual edge and a same-size, featureless
neighbour away from rooms and overrides, every other exit position gives candidates (not
mirroring up or left); `below(total)` picks one, and if the neighbour's key with the matching
exit (`3·size − exit − 1` on the opposite side) has candidates not mirroring any side, both are
replaced (`below` again for the neighbour); otherwise the pick is dropped and it repeats.
Ground overrides are checked at the room's turned corners. The corner ground fix-ups of the
replacement (`0x141fde880`) are not ported.

### Fill-room uniform cells

(`0x141f8c8c0` → `0x141f8e400`): `s` slots and `k` slots without a
feature or edges on one ground take a fill tile of that ground. The orientation is chosen in the
room's frame (as for fill tiles) and composed with the room's. The tile is placed from the
smallest corner its cells reach (per-orientation offset table in `0x141f8e400`). It must fit the
map (`0x141f9e5f0`: one cell of slack, then `0x141d0d940`) and the room grid (`0x141f8c500`): it
stays inside the room and covers only uniform cells whose ground equals the tile's first corner
ground (or both are missing or have `.gt` flag 8), at the start cell's height byte (+0x72).

### Room pass ground marking

(`0x141f8e6c0`, eighth argument): when set (overlays, `0x141fa23e0`
passes `1, 1, 9, 0`), uniform-ground slots also give their ground to the corners x in `0..w`, y
in `0..=h` (`0x141d0e130`, which sets flag 1). Node rooms (`0x1420c9970`) pass it unset. The room
pass also registers each room (`0x141faab40`), so overlay rooms count as rooms for
de-repetition.

### Fill rooms on unwalkable corners

(`0x141f91110`): only rooms whose room data +0x188 list is
empty. Entries are 0x30 bytes, which fits the points of interest (x, y, rotation, tag); it is
not the doodad list.

### Outer buffer

(`0x141fd8360`, `modules/server/crates/poe-world/src/outer_buffer.rs`): the map is the
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
