# dumps/

Local datamined and generated outputs. Everything here except this file is git-ignored, so a fresh clone has an empty tree. Anything below can be regenerated (see the last column); the captures and the tables the client DLL writes need a play session.

Versioned by client build: `dumps/0.5.5e.hotfix1/`. The folder is written by the client DLL (`dump_dir` in `GameFiles/poe-hook.toml`, set by `cargo xtask deploy-client`) and read by the server (`[spawn] layout_dir`, `[dev] known_seeds`, `[dev] sweep_results` in `modules/server/config/server.toml`).

```
dumps/0.5.5e.hotfix1/
  terrain/
    grids/         terrain_grid_<tile hash>.bin     client DLL, on every area load
    layouts/       terrain_layout_<tile hash>.tsv   client DLL, on every area load
    area-exports/  <session>/<n>_<hash16>_<seed>/   client DLL (area_export, generator_trace.tsv)
                   town_export_*, river_export_*    older hand-made exports, same format
    traces/        act1_traces/, town_trace_*       generator traces kept for the port (area_trace)
    vectors/       terrain-vectors.tsv, sweep-*      test vectors and seed sweeps
    batch/         <run>/                            in-client batch generation runs (jobs.txt, batch-results.tsv, exports/)
  client-tables/   object_types.txt, terrain_tiles.txt   client DLL (dump_object_types)
  ghidra/          ghidra_named_functions_p*.json, instance_factory*_s2c.json, packet_classes_*.json
  captures/
    official/      packet sessions from official servers (read-only observation)
    local/         packet sessions against our server
  strings/         clientstrings.txt
```

| Path | Produced by | Regenerate |
|---|---|---|
| `terrain/grids/`, `terrain/layouts/` | Client DLL (`dump_object_types`), written when an area loads. The server reads them to place the spawn and for the walkability grid. `scripts/test-run.sh` deletes them before a run. | Play the area (any server). |
| `terrain/area-exports/<session>/` | Client DLL, `[features] area_export` (on by default, also read-only); `generator_trace.tsv` with `generator_trace` or `POE_HOOK_GEN_TRACE=1`. `<session>` is the DLL session folder name (`<date>_<time>_<pid>`). Checked with `cargo run -p poe-world --example check_exports`. | Play, or run a seed sweep (`scripts/re/sweep_drive.py`). |
| `terrain/batch/<run>/` | `scripts/re/terrain_batch.py plan` (`jobs.txt`, `seeds.json`) and the client DLL's `gen_batch` (`batch-results.tsv`: area, seed, topology, orientation, status, tile hash, doodad hash, size, ms, detail; `exports/<job>/` for the jobs of `export.txt`); the script adds `crashes.tsv`, `state.json`, `suspects.txt`. Format and files: [docs/re/terrain/batch.md](../docs/re/terrain/batch.md). | `scripts/re/terrain_batch.py plan <run> ...` then `run <run>` (a few minutes per 10,000 layouts, no login). |
| `terrain/traces/` | `generator_trace.tsv` copies of areas the port is compared against (`area_trace`, `town_check`, `river_gen` examples). | Re-export with `generator_trace` on. |
| `terrain/vectors/terrain-vectors.tsv` | `scripts/re/clientlog_terrain.py <Client.txt> <out.tsv>` from a real play log (`[dev] known_seeds`). | Rerun the script. |
| `terrain/vectors/area-vectors-results.txt`, `probe-results.tsv`, `forced-results.tsv` | Inputs of `cargo xtask terrain-matrix`: `area_vectors` stdout, `area_probe` stdout, forced-layout client verifications (format in `xtask/src/terrain_matrix.rs`). | `cargo run --release -p poe-world --example area_vectors -- <vectors.tsv> all > ...`; `... --example area_probe > ...`; forced results per [docs/plans/terrain-all-areas.md](../docs/plans/terrain-all-areas.md). |
| `terrain/vectors/sweep-results.tsv` | The server's seed sweep (`[dev] sweep`). `sweep-vectors.tsv`, `sweep-*.txt` are sweep inputs and extracts for `area_vectors`. | `scripts/re/sweep_drive.py`. |
| `client-tables/` | Client DLL (`dump_object_types`): object types, and the tile grid's raw records. | Play. |
| `ghidra/ghidra_named_functions_p*.json` | Ghidra MCP `search_functions_enhanced(has_custom_name=true)`; read by `scripts/re/export_symbols.py`. | Export again from Ghidra. |
| `ghidra/instance_factory*_s2c.json`, `packet_classes_*.json` | `scripts/re/dump_factory.py` and the vtable scan in [docs/protocol/instance.md](../docs/protocol/instance.md#client-packet-classes). | Rerun the script on the exe. |
| `captures/{official,local}/` | Copies of `GameFiles/logs/poe-hook/<session>/` (`packets.jsonl`, `packets.txt`, `protocol.tsv`, snapshots). Official = the session ran in official mode or connected to a non-loopback address. | Play; `scripts/dumps/migrate.sh` copies new sessions. |
| `strings/clientstrings.txt` | Strings extracted from the exe. | Rerun the extraction. |

## Moving old outputs in

`scripts/dumps/migrate.sh [--dry-run]` sorts the outputs that piled up in `GameFiles/`, `runtime/` and `docs/re/` into this tree. It moves our generated files (never game files: it matches an explicit list of patterns) and copies the packet sessions (the `areas/` export folders of a session are moved to `terrain/area-exports/<session>/`). Safe to run again; anything already in place is skipped.

The DLL writes straight into this tree once `dump_dir` is deployed, so `migrate.sh` is needed only for old files and for copying new capture sessions.
