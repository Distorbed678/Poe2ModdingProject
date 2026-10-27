# In-client batch generation (`gen_batch`)

How the client DLL runs the client's own terrain generator in a loop, for any `WorldAreas` row,
seed, forced topology and forced orientation, with no login and no session, and reads back the tile
hash, doodad hash and map size. This is the data source of the 1:1 terrain port's acceptance test
(every area, every topology and orientation, 50 random seeds:
[../../plans/terrain-all-areas.md](../../plans/terrain-all-areas.md), WP7.16). Findings from
2026-10-22; addresses are Ghidra addresses of 0.5.5e (image base `0x140000000`) and are found by
signature at run time (`modules/client-hook/signatures/0.5.5e.toml`, feature `gen_batch`). Private
mode only: the feature is refused when `poe-hook.toml` says `mode = "official"` and again before
every job when the loader's read-only latch is on.

Use: [the guide](../../guides/setup.md#terrain-batch-generation). Code:
`modules/client-hook/poe-hook-core/src/gen_batch.rs` (jobs, worlds, results),
`cxx_guard.rs` (surviving the generator's exceptions), `poe-hook-shared/src/batch_jobs.rs` (the
job and result file formats), `scripts/re/terrain_batch.py` (plan, run, status).

## Result

- The client generates a layout in **about 80 ms (act 1) to 400 ms (act 3, 4) on a worker thread**; with eight
  threads act 1 runs at **118 layouts per second** and all acts at **38 per second**, see
  [Measurements](#measurements). The 225,300 layouts of the acceptance test take about 2 hours
  including the client restarts the memory cap makes.
- Tile hash, doodad hash and size are **bit for bit the client's**: all 26 known act 1 vectors of
  `sweep-vectors.tsv` (found combinations of `area_vectors`) give the same tile and doodad hash, and
  `G1_12 1001 2:7`, which the port still gets wrong, gives the client's `3369045138` at 174x87.
- No login, no server and no game state beyond the loaded data: the client sits at its login
  screen (20 s after the start the bundle index and the `.dat` tables are ready).

## How the generator is run

`HeadlessWorld_LoadArea` `0x14212f500` is not a class of its own: it is the **base world**'s vfunc
`+0x1e0` (vtable `0x1435082f0`, constructor `0x14212eb60`). The in-game world, `ClientWorld`
(constructor `0x14167d2f0`, `new(0x17e8)`, vtable `0x14339cf18`), overrides it with
`ClientWorld_LoadArea` `0x14167b270`. The base world has no terrain (its vfuncs `+0x1a0/+0x1a8`
raise an "unimplemented" exception), so a world that can generate has to be a `ClientWorld`: its
vfunc `+0x1a8` returns the terrain object embedded at world `+0x8d0` (built by the constructor),
`+0x1c8` the generation state at world `+0x4b0`.

Both LoadAreas do the same, which is all `gen_batch` repeats per job:

1. `world +0x4b0 = p2` (u32, 0 in 904 of 929 area exports; the other value is an instance
   number), `world +0x4b4 = seed`, `+0x663 = 0` (headless; the client sets 1), the `LoadArea`
   vectors `p4` (u8 indices into a 0x65-byte table, `0x141d02ba0`) and `p5` (u64 list, `+0x508`)
   stay empty.
2. `TerrainGen_Construct(generator, world)` `0x141fefa20` (base generator vtable `0x1434ede68`,
   704 bytes in the headless version), `TerrainGen_Run` `0x141ffe940`, read tile hash (world
   `+0x114`) and doodad hash (`+0x118`), the generator destructor `0x141fefe40`.

Differences between the base generator and the client's (`0x143385c70`), which matter:

| What | Base (headless) | Client |
|---|---|---|
| vtable `+0x20` | `0x141ff45d0` | `0x1415a1700` (the "create this object?" filter, adds the client's own tests) |
| vtable `+0x68`, `+0x78`, `+0x80`, `+0xa0`.. | empty or minimal | rendering hooks |
| **generator `+0xe`** | 1 (set by the constructor) | cleared by `ClientWorld_LoadArea` (`local_2fa = 0`) |

`generator +0xe` switches the **tile map validation pass** (`0x141ffdab0` → `0x141f9df00` →
`0x141f9d9c0`, "Invalid TileMap: Mismatching ground types on corner ..."): the server's
generator validates and throws, the client's does not. Without clearing it, layouts the client
generates happily (`G1_4 1001 10:2`) throw. `gen_batch` clears it after the constructor, so its
output is what the client's generator produces; the generator's other phases are shared and the
hashes agree.

### What the world needs

| Field | Value | Notes |
|---|---|---|
| `+0x98` | the `WorldAreas` row | see below; `+0x1b4` area kind, `+0x2a` hash16, `+0x60` topology count, `+0x68` topology list (16-byte entries) |
| `+0xa0` | the `WorldAreas` table object | a counted reference (`+0x3c`): the Incursion temple rooms (`0x141eff290`) read the table from here, and the world's destructor releases it; without it `IncursionTemplePresent` crashed |
| `+0xb8` | league mode, **1** | the `Mode` column the topologies are filtered by (`0x141fdfe30`: mode equal, else 0x39, else mode 1) |
| `+0x128` | the stat map | sorted `(i32 key, i32 value)` pairs, begin `+0x128+8`, end `+0x128+0x10`; set `0x1423ba400(map, key, value)`, get `0x1423ba2a0(map, key)` (returns 0 when absent), 978 callers |
| `+0x58` | terrain plugin list | from AreaInfo; empty |

The `WorldAreas` table object comes from `0x140223710(&out)` (a counted reference; its state
`+0x40` is 2 when loaded, else wait with `0x1421d6e30`); `*(table+0x28)` points at `(begin, end)` of
the rows: **442 rows of 0x2e0 bytes**, row `+0` a pointer to the NUL-terminated UTF-16 id string.
`gen_batch` builds an id → row map once (the count comes from `poe-hook-data/areas.tsv`, which
`deploy-client` writes from the same game data).

### Forcing topology and orientation

`map_force_topology` (stat `0xb92`) and `map_force_orientation` (`0xb93`) are plain entries of the
world's stat map; the server sends them in the first stat record of AreaInfo
(`modules/server/servers/instance-server/src/lib.rs`, `transfer`), the client's AreaInfo reader
(`0x14211d1c0`) stores them in `world +0x128`. `TerrainGen` vfunc 0 (`0x141ff9280`, topology pick)
reads them: topology `ebx` = stat `0xb92` (0: not forced) and must be `<= row +0x60`, orientation =
stat `0xb93` − 1 when that is in `1..=8`, else 8 (not forced); then `0x141ff90f0(generator, out, topology, orientation, {count, row +0x68})`
picks the graph. `gen_batch` sets the same two stats with `0x1423ba400` before the generator is
built: `topology` 1-based as the server's `transfer` does, `orientation + 1`.

## Threads

Safe, as far as 1 to 12 workers over 48,300 + 3,864 + 9,016 jobs showed:

- The RNG is thread-local (`TLS + 0x20190`, seeded by every generation). The same jobs give the
  same hashes on 1, 2, 4, 8 and 12 threads ([Measurements](#measurements)).
- The shared caches (`.tdt`/`.arm`/`.tgr` loads, the tile-set indexes) are the ones the client uses
  from its own worker threads (it generates world-map panels in parallel). Resources load on first
  use (async, `0x1421d6e30` waits), so the first jobs of an area are slower.
- Every job builds its own world and generator and destroys them (`world` vfunc `+0x20` with 1:
  the deleting destructor; the generator `0x141fefe40`), allocated with the game's allocator
  (`0x142bba3c0` / `0x142bba2a0`; per-thread heaps that accept frees from any thread).
- Memory: about 5 GB idle, 8 GB after act 1, and about 1 MB per layout in act 10 that is not given back; see Measurements (the driver restarts the client above a cap).
- The ClientWorld constructor bumps a global world counter (`0x1445d8140`) without a lock; two
  workers can get the same id. Nothing seen depends on it.

## Exceptions: generation failures

The generator **throws `TerrainGenerationException`** (MSVC C++ exception, type
`.?AUTerrainGenerationException@Generation@Terrain@@`) when a layout cannot be built, and nothing
catches it in the client's own load path: a seed that fails would not load there either (the
official server picks the seed, so it never sends one). Act 1 shows what happens; about 2 % of the
jobs: "Dungeon generation failed: Failed to place a special room", "... Failed to connect edge ...",
"... No possible rooms found for room at (x, y) with type \"empty\"", "Mismatching ground types at
(x, y): Cannot place WideRiverWater over ...", "Failed to find suitable fill tile/room", "Could not
find matching overlap tile", and, for `IncursionTemplePresent` (always), "No walkable area in the level" (its
rooms come from the player's temple, which the batch does not have). They are results, not bugs:
the port should fail the same jobs, and the matrix gets `error` rows with the text.

An unhandled throw ends the process, so `cxx_guard.rs` makes a guarded call instead:

- `gb_try_call` (assembly, `.seh_proc` unwind info so the first-pass stack walk gets past it) saves
  callee-saved registers and the stack position and calls the function.
- `SetUnhandledExceptionFilter` is only reached when **no frame has a matching catch** (the game's
  internal `try`/`catch` keep working), so the filter checks that the thread is inside a guarded
  call and the exception is a C++ one, then jumps back (`gb_longjmp`), and the call returns `Err`
  with the type and the message (the exception object holds a string stream: the text is found by
  scanning the object, ending at the stream's write position).
- Returning `EXCEPTION_CONTINUE_EXECUTION` from the filter is not possible (a C++ throw is
  noncontinuable: `0xC0000025`); the filter calls `RtlRestoreContext` itself.
- **The C++ runtime's per-thread state must be put back.** The game's statically linked runtime
  keeps a chain of the catch blocks being executed in the thread's `__vcrt_ptd` (`0x142c14288`,
  field `+0x58`, also `+0x20/+0x28/+0x70`); the abandoned catch blocks leave entries that point into
  dead stack, and the next throw on the thread reads them (`_IsExceptionObjectToBeDestroyed`
  `0x142c13a00`: access violation after a few hundred recovered exceptions). `call` saves the first
  `0x98` bytes of that state and writes them back after a recovery.
- What the skipped frames held stays held (memory, a lock): the generator and the world are
  destroyed afterwards in guarded calls anyway; a destructor that throws leaks.
- Access violations are not recovered: the client dies and `terrain_batch.py` restarts it.

## Job and result files (`<run dir>`)

| File | Who | Contents |
|---|---|---|
| `jobs.txt` | `terrain_batch.py plan` | `area seed topology orientation` per line; topology 1-based (0: not forced), orientation 0-7 (negative: not forced) |
| `export.txt` | by hand | same format: jobs whose `exports/<area>_<seed>_<topology>_<orientation>/` (`layout.tsv`, `doodads.tsv`, `grids.bin`, `meta.txt`) are written |
| `suspects.txt` | `terrain_batch.py run` | jobs that were running when the client died; run first, on one thread |
| `batch-results.tsv` | the DLL | `area seed topology orientation status tile_hash doodad_hash width height ms detail`; `status` `ok`, `error` (detail: the exception or the check that failed) or `crash-skip` (written by the script); one `write` and flush per line; jobs already in the file are skipped on a restart |
| `inflight-<n>.txt` | the DLL | the job worker `n` is running (removed when the run ends) |
| `batch-done` | the DLL | written when every job has a line |
| `crashes.tsv`, `state.json`, `seeds.json` | the script | crash counts per job, timing, the plan (master seed, filters) |

Configuration: `[gen_batch]` in `poe-hook.toml` (`enabled`, `dir`, `threads`,
`start_delay_seconds`) or the environment, which is what the script uses: `POE_HOOK_GEN_BATCH=<run
directory as the game sees it, Z:\...>` (enables it), `POE_HOOK_GEN_BATCH_THREADS=<n>`. Always off
in official mode. The hooks of the area export (driver, add object, doodad hash site) are
installed for it; a batch generation is never exported by the area export itself, and objects and
doodads are only collected for the jobs of `export.txt`.

## Addresses (0.5.5e)

| Address | What |
|---|---|
| `0x14167d2f0` | `ClientWorld` constructor `(this, frame mode u8, flag u8)`, size `0x17e8`; vtable `0x14339cf18` (`+0x1a8` terrain at `+0x8d0`, `+0x1c8` state at `+0x4b0`, `+0x1e0` LoadArea, `+0x20` deleting destructor) |
| `0x14212eb60` | base world constructor (only caller: the ClientWorld constructor); vtable `0x1435082f0` |
| `0x14212f500` | `HeadlessWorld_LoadArea` (base vfunc `+0x1e0`): `(world, u32 p2, u32 seed, vector<u8>* p4, vector<u64>* p5)` |
| `0x141fefa20` / `0x141fefe40` | `TerrainGen_Construct(generator, world)` / generator destructor |
| `0x141ffe940` | `TerrainGen_Run` (the batch calls it through the area export's driver hook, which passes batch generations through) |
| `0x141ff9280` | topology pick (generator vtable `+0`): reads stats `0xb92` / `0xb93` from `world +0x128`, `world +0xb8`, row `+0x60/+0x68` |
| `0x141ff90f0` | picks the graph for a forced topology and orientation |
| `0x141ffdab0` / `0x141f9df00` / `0x141f9d9c0` | the validation pass (runs when generator `+0xe`) / its driver / the checks ("Invalid TileMap: ...") |
| `0x141d0e130` | "Mismatching ground types at (x, y): Cannot place A over B" (throws; tile placement) |
| `0x1423ba400` / `0x1423ba2a0` | sorted `(i32 key, i32 value)` map set / get (the world stats at `+0x128`) |
| `0x140223710` | `WorldAreas.dat` table getter `(out)`; sibling getters exist for every table (one function per table) |
| `0x1421d6e30` | resource "ensure loaded" (state `+0x40` == 2) |
| `0x142bba3c0` / `0x142bba2a0` | the game's allocator / free |
| `0x142c14288` | `__vcrt_getptd`: the thread's C++ exception state |
| `0x142c13a00` | `_IsExceptionObjectToBeDestroyed` (walks the frame info chain at ptd `+0x58`) |
| `0x141eff290` | Incursion temple room object constructor (reads `world +0xa0`, throws "[INCURSION2] Present-day temple contains Past-only room") |
| `0x14100b110` | creates the in-game world (`new(0x17e8)`, constructor `(2, multi)`) and destroys the old one |

## Measurements

All on the development machine (16 cores, 62 GB RAM), act 1 unless noted, one client, default settings
(`terrain_batch.py run`, 8 worker threads, memory cap 16 GB). "ms" is the time of one job on its worker
thread with all the others busy.

**Acceptance.** `terrain_batch.py plan vectors --vectors` + `run` + `verify`: the 26 known client vectors
(`sweep-vectors.tsv` at the combination `area_vectors` found) give the same tile **and** doodad hash:
`G1_8 1001 1:3` (1370641136 / 3951204914), `G1_6 1001 2:7`, `G1_2 1001 15:0`, all 26; the export of
`G1_8 1001 1:3` (layout columns, `grids.bin`, `doodads.tsv`) is byte for byte the area export the DLL
wrote when the client loaded the same area in 2026-10. `G1_12 1001 2:7` (the port's one forced fail) gives
the client's tile hash 3369045138 at 174x87.

**Act 1 x 50 seeds** (966 combinations, 48,300 jobs, seeds from master seed 20261022): one unattended
`run`, one client launch, **0 crashes, 0 hangs, 0 restarts**; 47,275 ok, 1,025 errors (generator exceptions, see above:
`G1_12` 310, `G1_7` 238, `G1_4` 199, `G1_11` 66, `HideoutPietyArena` 50, `IncursionTemplePresent` 50 ...);
**409 s from the first job to the last = 118 layouts per second**; per layout mean 69 ms, median 53 ms, p95 188 ms.

**Threads** (3,864 act 1 jobs = 966 combinations x 4 seeds, same jobs every time, time from first to last job
including the cold caches; the machine was otherwise idle):

| threads | 1 | 2 | 4 | 8 | 12 |
|---|---|---|---|---|---|
| seconds | 150 | 82 | 51 | 35 | 30 |
| layouts per second | 26 | 47 | 76 | 111 | 130 |

The results are the same on every thread count: statuses, tile hashes, doodad hashes and sizes of all 3,864
jobs are identical for 1, 2, 4, 8 and 12 threads, and the 9,016 jobs of the all-acts sample are identical
between two complete runs (one with six client restarts). Only the error *text* differs in a few lines
(about 0.1 % of the jobs): the first characters of a generator exception's message are sometimes overwritten
(`G1_World...`), by whatever reuses that buffer; classify errors by the rest of the text.

**All acts** (every combination x 2 seeds = 9,016 jobs, 8 threads, one launch): **234 s = 38.5 layouts per
second**, 0 crashes, 277 errors. Per layout (ok jobs): act 1 mean 79 ms, act 2 209 ms, act 3 331 ms, act 4 375 ms,
act 6 255 ms, act 10 231 ms (median 127 ms, p95 967 ms, max 2.7 s). **Projection for the 225,300 jobs of the acceptance
test (4,508 combinations x 50 seeds): 225,300 / 38.5 = 5,850 s = 1.6 h of generation, about 2 h with the
memory restarts** (about 25 restarts of about a minute).

**Memory** (resident set of the game process under Proton, sampled every 10 s into `rss.tsv`):

| | RSS |
|---|---|
| client 10 s after the start | 1.7 GB |
| login screen, when the first job starts (20 s) | **5.1 to 5.2 GB** (same for every thread count) |
| act 1, 3,864 jobs, peak, 1 / 2 / 4 / 8 / 12 threads | 7.8 / 7.4 / 7.9 / 7.6 / 8.1 GB: **the thread count barely matters** (a world and a generator per job, a few MB per thread) |
| act 1, 48,300 jobs (8 threads), peak | 7.7 GB, flat after the first 15,000 jobs |
| act 1, 7,728 jobs (3,864 twice with other seeds), 4 threads | 7.4 GB after the first pass, 7.9 GB after the second (+0.5 GB, about 0.13 MB per job) |
| act 10, 2,304 combinations three times with other seeds (6,912 jobs), 8 threads | 9.2 GB after the first pass (data being loaded), 11.2 GB after the second, 12.2 to 13.1 GB after the third: **a leak of about 0.7 to 0.9 MB per layout** that the world destructor does not give back (probably the generated objects and the terrain's side tables); it does not show in act 1 |
| all acts, 9,016 jobs, no restart | 14.9 GB at the end |

So memory is not bounded by the thread count but by the number of layouts generated in one client run. `terrain_batch.py run`
therefore has a **cap on the game process's RSS (`--max-rss-gb`, default 16)**: above it the DLL is asked to stop
(`<run dir>/batch-stop`), the jobs in flight finish (`batch-stopped`), the script stops the client, starts it again and the
run resumes with the jobs without a result. Tested with a 9 GB cap on the all-acts sample: 5 restarts, no job lost or
repeated, results identical to the run without restarts. A cap too close to what the client needs to run at all
(5 GB idle) would restart forever, so a launch that makes fewer than 100 jobs raises the cap by 25 %.
`--restart-every N` restarts after N layouts as well. With 8 threads the total stays below the cap for about 10,000 layouts
of act 10 or 50,000 of act 1; 12 threads add under 1 GB and gain 17 % throughput, so the default is **8 threads**
(half of the 16 cores, leaving the desktop its share).


## Open issues

- Forced topology index (settled 2026-10-22): the client checks `map_force_topology` against the
  **raw** `WorldAreas` list (row `+0x60` entries, all league modes) and takes entry `stat - 1` of
  row `+0x68` (`0x141ff9280` checks `stat <= row +0x60`, else throws "Cannot force graph N because
  area X does not have that many topologies"; `0x141ff90f0` indexes the array, no mode filter).
  The mode filter (`0x141fdfe30`: mode equal, else 0x39, else mode 1) is only used when nothing is
  forced. In all 442 areas the mode 1 (or 0x39) entries are a **prefix** of the raw list, so the
  1-based index into the port's `topology_entries` list is the same index into the raw list and
  the matrix ids, the batch plan and the server's `transfer` agree. 11 areas list further
  entries of mode 54 after the mode 1 ones (G1_2 33 raw / 18 mode 1, G1_6 8/4, G1_11 24/12,
  G1_13_1 32/16, G2_4_1 8/4, G2_5_2 8/4, G2_8 8/4, G3_2_1 18/9, G3_3 16/8, G3_4 8/4, G3_7 8/4);
  forcing an index past the mode 1 entries picks those (not in the matrix). The port's forced
  handling (`area::generate_probed_files`) now indexes `area::raw_topology_entries` and fails
  like the client for an index past its end. Evidence from the full batch: topologies 1 to 18 of
  `G1_2` and 1 to 4 of `G1_6` match for many seeds.
- Data-path failures of the 50-seed batch (fixed on the `terrain-c` branch, 2026-10-22; replay of the full batch: no match lost, +295 matches, G2_3a 50 and G4_5_2 245):
  `//` in joined game paths (`Shipwrecks//...`, `3_14//Subarea/...`: `generate::normalize_path`
  collapses repeated `/` in every lookup); `.dgr` graphs with an `EnvironmentOverride: "<id>"` line
  before the ground types (G2_3, G2_3a, G4_5_2: the header was read one line off); container
  graphs whose master names no `RoomSet`, `TileSet` or `FillTiles` (MapKarst, MapDerelictMansion,
  P1_6, HideoutVampireManor: empty set and master, no outer fill of their own); rooms whose root
  slot is not a key (MapWetlands `forced_rare.arm`: an empty key). These jobs now run but still
  fail further on: Wetlands and the vampire/derelict manors differ in tiles (the second
  subgraph of a container diverges, probably the RNG between subgraphs; needs a client generator
  trace of `P1_6 13489297 1:0`), `FileGroups` (G2_3, G3_1) and `could not fix heights` (MapKarst)
  are not ported.
- `IncursionTemple*` (and any other terrain plugin that needs data from the player's session) cannot
  be generated without that data: `error` rows with "No walkable area in the level".
- Layouts only: objects, doodad entries and the collision grid of a job are written for the export
  list; the doodad hash and the size of every job are in the results. The server-only
  doodad scatter (globals `0x1445c991c` / `0x14458c705`) is not run.
- The generator's natural topology choice (no forced stats) uses the world map pins; with none it
  falls back to a draw (`0x141ff9280`). Jobs with topology 0 run that fallback.

## Traces in batch mode

`POE_HOOK_GEN_TRACE=1 python3 scripts/re/terrain_batch.py run <name> --threads 1` writes `generator_trace.tsv` next to the export of every job in `export.txt` (the trace buffer is global: one worker thread only).

Tile-map checkpoints (2026-10-27, `terrain_tools.rs` `TILE_CHECKPOINTS`): at the entry of the graph pipeline `0x141ff05f0`, the post-layout stage `0x141ff7a10`, the outer fill `0x141fd5ff0`, the outer buffer `0x141fd8360`, the grid passes `0x141fe0fd0` / `0x141fe0ea0`, the overlay candidate scan `0x141fa1830`, the placeholder fill `0x1420a6140`, the pre-unique room `0x14209c150`, the topology choice `0x141ff9280` and every phase, `tdiff pre_<name> <caller>` lines list the tile records changed since the previous checkpoint (`x y h<old>-><new> o<old>-><new> s<skip>`, result the tile file, `old>new` when it changed), closed by a `tdiff_end` count; replaying them gives the tile map at any checkpoint. The placeholder fill adds `roomgrid` rows (its room grid, 0 = free), the pre-unique room `preunique_cells` (its ground-only cell list before the shuffle: index, x, y, accessible, height, ground), the topology choice `map_pins` (generator `+0x280`) and `pin_dirs` (generation state `+0x70`), see [shared-passes.md](shared-passes.md#topology-choice). Runs of 2026-10-22 under `dumps/0.5.5e/terrain/batch/`: `a1` (63 act 1 / dgr jobs), `a2`, `b1`, `b3`, `c1`; `b2` has exports without traces.
