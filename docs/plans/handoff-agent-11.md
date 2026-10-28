# Handoff 11: single working agent (2026-10-27)

Read [handoff-agent-7.md](handoff-agent-7.md) first (preferences, commit rules, CPU limits, method, tools), [handoff-agent-8.md](handoff-agent-8.md) (tools in `runtime/w7/`, debug switches) and [handoff-agent-10.md](handoff-agent-10.md) (the state this session started from). This file only adds what changed.

## User decisions (2026-10-27, also in memory)
- Order: finish the remaining terrain failures, then port the natural topology and orientation choice; interleave (client traces in the background while porting).
- `[terrain] generate` defaults to on once the natural topology choice is ported and verified (WP7.15).
- x64dbg is allowed on the private client for BossRush_Area1.
- The user will capture an official session walking through an area transition and a waypoint later (entities).
- `.vscode/settings.json` is committed.

## State
- **Full gate merged:** 218,653 of 218,705 client layouts match (from 218,490), 0 lost (Poe2Server `95ceda7`, `runtime/w11/gate/full.tsv`). It is now `runtime/forced-baseline.tsv` (previous: `runtime/forced-baseline-pre-w11.tsv`) and merged into `dumps/0.5.5e/terrain/vectors/forced-results.tsv`; matrix 4465 match / 3 partial / 1 fail / 39 untested.
- **Second batch (`full2`):** 163 of its 217 failures match now (`runtime/w11/chay-f2.tsv`, input `runtime/w11/full2-fails-in.tsv`); left: BossRush_Area1 50, MapCaldera 4. Its full replay was not rerun (about 2 h: `forced_verify dumps/0.5.5e/terrain/batch/full2/batch-results.tsv`); do it before quoting a total for that batch.
- **Fixed this session** (RE and addresses in `docs/re/terrain/dungeon.md` "Session 2026-10-27"): Chayula towers (outer fill alignment in map coordinates; strip fill fallback to the outer ground's tile; levelling of empty cells; StoreyHeight by the region holding a tile's corner, `0x141ffde60`), G2_12 (size map door type from the root slot, `0x141f99b20`), G1_13_2 (pre-unique accessibility as listed), RitualLeagueBoss (overlay whole-tile check, `0x141d0dc40`).
- **Remaining failures (first batch, 52):** BossRush_Area1 49, MapCaldera 2, MapCrimsonShores 1.

## Tools added
- Client hook trace lines (`POE_HOOK_GEN_TRACE=1`, docs in `docs/re/terrain/batch.md`): `tdiff` (tile records changed since the previous checkpoint at the graph pipeline, post-layout stage, outer fill and buffer, grid passes, overlay scan, placeholder fill, pre-unique room, topology choice and every phase; replay them with a small script like `/tmp/replay.py` of this session: per `tdiff` line set (x, y) to the new tile/height), `roomgrid`, `preunique_cells`, `occupancy`, `placeholder_sizes`, `strip_grounds`, `map_pins` / `pin_dirs`.
- Port debug switches: `POE_DBG_LATROWS=y0,y1` (lattice and cells at the levelling start), `POE_DBG_PU_IN`, `POE_DBG_ACC`, `POE_DBG_OCC`, `POE_DBG_STRIP`, `POE_DBG_SIZEMAP` (now with mask and unique), `POE_DBG_PIECES=x,y`, `POE_DBG_CELLST=x,y`, `POE_DBG_OWNER`; off switches for the new rules: `POE_SM_OLD_DEFAULT`, `POE_STRIP_NO_OUTER`, `POE_NO_EMPTY_LEVEL`, `POE_OLD_OWNER`, `POE_PIECES_OLD`.
- Large or slow areas traced: run `terrain_batch.py run <run> --threads 1 --no-deploy --stall 1500` with `POE_HOOK_GEN_TRACE=1` (BossRush_Area1 takes 13 minutes traced; the default 240 s stall kills it).
- `runtime/w8/ofitdiff.py`: widen its offset search (`range(-200,201)`) for large maps.

## Next
1. **BossRush_Area1 (49 + 50):** traced in `dumps/0.5.5e/terrain/batch/w11-7` (seed 496246952 1:0). `area_trace` diverges at draw 9335: after the client's last overlay pick of phase 2 (`141fa249c` n=2) the client starts `_TheForsakenSon.dgr` (existence rolls `0x141feeb10`, `below(100)`), the port draws n=1, 2, 2, ... first. The phase-2 candidate sets match (94 positions). Find what the port draws there (log the port's draws with their call sites; a likely place: the overlay placement's end, or something before the subgraph's prune that the client does not run), then fix.
2. **Natural topology and orientation choice** (RE complete in `docs/re/terrain/shared-passes.md` "World-map pin directions in detail"): for a town (or kind 2) area the client lays out the act's world map (`0x141fa5900`: the act's world-map area, a topology picked with the LCG from the town's seed, its graph laid out with the act's `MapPins`, `0x141fb9fb0` / `0x141fb89d0`, `0x141fa44c0`, positions `0x141fa53f0`), derives pin neighbour directions (`0x141fa47b0`, `MapConnections.dat`), lays out the directions of all pins (`0x141fa4da0`) and keeps the town pin's list in the generation state `+0x70`; every later area matches its connections against it (`0x141fa6440`, three passes), picks with the LCG and writes its choice back (`0x141fa6c80`). Still to RE: `0x142445f00`, `0x141fa57d0`, `0x141fb9fb0` / `0x141fb89d0`, `0x141fa44c0`, `0x141fa53f0`. Then port it in `poe-world` (`area.rs` `topology_entries` / `generate_with`), keep the per-session state in the server (the last town's list, updated per generated area), and verify against live client loads on the private server (the `map_pins` / `pin_dirs` trace lines are in the hook already; the hook must be deployed with the trace on for a normal load). Then WP7.13 / WP7.15 and turn `[terrain] generate` on (the user agreed).
3. MapCaldera (2 + 4; lattice clusters, overlay total 45 vs 50) and MapCrimsonShores (1): older open cases.
4. The full replay of the second batch, then entities (handoff 10 section "Entities").
