# Handoff 8: single working agent (2026-10-25, stopped on low usage)

Read [handoff-agent-7.md](handoff-agent-7.md) first (preferences, commit rules, CPU limits, method, tools); this file only adds what changed after it. Goal unchanged: every remaining terrain job matching (the user: "get all the remaining ones matching, best first").

## State
- Wave 6 merged and pushed (Poe2Server `a1e3279`, hub `d4a2fec`): **206,746 of 218,705** match, 0 lost. Baseline `runtime/forced-baseline.tsv` = `runtime/wave6/gate/forced-w6.tsv` (pre-wave-6 copy `runtime/forced-baseline-pre-w6.tsv`). Worktrees removed.
- After it, pushed to Poe2Server `main`:
  - `f447d9f` GATED: subgraph windows share the parent's taken cells (G2_4_1 1,600/1,600; match sample 0 lost).
  - `4a8c82e` GATED on match sample: second lattice settle after overlay pass 1 (MapFrozenFalls +808), no league rolls when the area's `EndgameMaps` column 28 is set (+717 on 63 areas: JadeCitadel, CopperCitadel, logbooks).
  - `ef8b07d` **WIP, NOT GATED**: filler lists from all the area's room sets with summed weights (MapSunTemple, `Files::area_room_sets`), `OverrideSpawns` flat outer fill only with `DefaultKillZone` (MapExcavation). Checked: SunTemple and Excavation traced seeds match; G2_6/G3_3/MapSteamingSprings sample (15 jobs) match; the 461-job `OverrideSpawns` sample (`runtime/w7/ovsp-in.tsv`) changes only Excavation (fixed).
- **No full gate has run since wave 6.** First step: build, then one full gate (`runtime/w7/gate1/`: copy `forced_verify` there, run detached with `setsid nohup`, 6-8 threads, ~2 h; `forced_verify` writes its output only at the end, background tasks die at 2 h), compare with `runtime/wave5/b/regress.py`. If 0 lost: merge into the vectors, new baseline, `cargo xtask terrain-matrix && cargo xtask tracker`, tracker.toml note, a wave entry in `docs/status/terrain.md`, bump the hub. If lost: bisect between `a1e3279`, `f447d9f`, `4a8c82e`, `ef8b07d` with the lost jobs.
- Findings documented in `docs/re/terrain/dungeon.md` "Single agent, 2026-10-25" (hub, this commit).
- Nothing of mine runs (gate and the client run `w7-1` stopped; resume the client with `nohup runtime/w7/runclient.sh w7-1 &`, 25 of 172 jobs left).

## Tools added (runtime/w7/)
`run.sh <area regex> <out.tsv> [threads]` (per-area gate vs baseline), `sum.sh <run> <outdir>` (first divergence of every export of a run), `runclient.sh`. Debug switches: `POE_DBG_FILL`, `POE_DBG_WIN`, `POE_DBG_MARK=x,y`, `POE_DUMP_LAT2`, `POE_DBG_BLANK`; off switches `POE_NO_INHERIT`, `POE_NO_RESETTLE`, `POE_OWN_ROOMSET_ONLY`, `POE_OVSP_ANY`. `tsi_keys <key>...` lists masters with those keys.

## Next leads by value (classes from `runtime/w7/sum/`, current build)
1. **Door-pick candidate count `0x141fa81a3`** (client 1 candidate, ours 2-4): G3_11 300, G3_16, G3_12 likely, earlier G2_4_2 class. Every G3_11/G3_16 export diverges there.
2. **Sub-dungeon / outer "failed to find a fill tile or room at (-1,-1)"**: G3_14, G3_8, MapAlpineRidge, MapAzmerianRanges, P3_2, Abyss_Pinnacle, ChayulaLeague_TowerBoss, MapUberBoss_Delirium: probably the same window-occupancy family as `inherit_cells` (a fill at x or y = -1).
3. **Encounter requests `0x1420ab667`** (P3_1..P3_6, see dungeon.md "Encounter requests").
4. **Overlay candidate totals `0x141fa249c` / `0x141fa2692`**: G4_13 (400), G1_11 several topologies, world maps (G1/G4), MapBloomingField, P2_3, MapOrnateChambers.
5. **Doodads `0x141ff6049`** (layout matches, doodad pass differs): MapBloodwood, MapRiverside (4 cells), MapHiddenGrotto, MapStronghold, MapPenitentiary (9 cells), MapTerrace: small layout diffs after all draws in step.
6. `0x14209ea03` (MapUniqueFreight_, CopperCitadel_Quest, G4_6), `0x141fdfd6d` G4_11_2, `0x14209c29e` G1_13_2 (7), `0x1420aff12` ExpeditionLogBook_Basin, CopperCitadel `0x1420b2652` (filler size total 1 vs ours 2: a non-square room listed as turned too where the client does not).
7. World-map own ground: room data +10 is set by the `.arm` parser for some rooms (labels have arm bool 1) but bool 1 alone lost 10 G4_WorldMap jobs (`POE_ARM_OWN=1`).
8. MapKarst (paired rooms), DerelictMansion (StoneCircle feature room), "could not fix heights" (G2_12, Design_Lite, G4_4_3).
