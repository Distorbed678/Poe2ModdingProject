# Datamining

## File system
- The client's data is in `GameFiles/Bundles2`: `_.index.bin` plus Oodle-compressed `*.bundle.bin` files.
- Virtual paths are lowercase, for example `data/balance/worldareas.datc64`.
- Base-language tables are in `data/balance/`; translations are in `data/balance/<language>/`.
- Terrain graphs are `metadata/terrain/**/*.tgr` (148 files).
- Characters and objects are under `metadata/characters/**` (`.ot`, `.otc`, `.ao`, `.act`).

CLI examples (run from the repo root):
```sh
T=third_party/poe_data_tools/poe_data_tools
$T -p 2 --steam GameFiles list 'data/balance/*.datc64'
$T -p 2 --steam GameFiles extract 'metadata/characters/str/*' out/
$T -p 2 --steam GameFiles dump-tables ...      # tables -> CSV/JSON using the schema
```

## Dumps folder
Everything we extract or generate that is not source (terrain grids and layouts, area exports, test vectors, client tables, Ghidra exports, packet captures, strings) lives in the git-ignored `dumps/<version>/`. The layout, what produces each file and how to regenerate it: [../dumps/README.md](../dumps/README.md). Old files from `GameFiles/` and `runtime/` are sorted in by `scripts/dumps/migrate.sh`. Ghidra exports and factory maps that used to be committed under `docs/re/*.json` are there now (`dumps/<version>/ghidra/`); they are local data, so a fresh clone must regenerate them (`scripts/re/dump_factory.py`, Ghidra `search_functions_enhanced`).

## Tables in the server
`poe_data::GameData::open(game_dir, schema)` then `.table("WorldAreas")` returns a `Table` (cached). Rows are read by column name: `row.str("Id")`, `row.int("AreaLevel")`, `row.row_ref("ParentTown")`. Unnamed columns are addressed as `"_<index>"`.

Format of `.datc64`:
- `u32` row count, then fixed-width rows, then eight `0xBB` bytes, then variable data.
- Little-endian. Strings are UTF-16LE and null-terminated, stored as u64 offsets relative to the `0xBB` marker.
- Arrays are `u64 count, u64 offset`. A null reference is `0xFEFE…`.
- Value widths: bool=1, i16/u16=2, i32/u32/f32/enumrow=4, string/row=8, foreignrow/array=16. An interval column holds two values.

The schema is `third_party/dat-schema/schema-poe2.min.json` (dat-schema version 7, the rolling `latest` release). It is filtered to `validFor & 2` (PoE2). Many PoE2 columns are still unnamed; name them through RE when the server needs them.

## Verified against 0.5.5e
- `Characters`: 12 rows. The ids are `Metadata/Characters/<Attr>/<Attr>Four` and `...Fourb` (male/female variants).
- `WorldAreas`: `G1_1` = "The Riverbank", act 1, level 1, HASH16 3860. The schema row width (736) matches the file.
