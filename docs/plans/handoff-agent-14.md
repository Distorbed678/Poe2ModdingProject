# Handoff 14: single working agent (2026-10-29)

Start here. Read [CLAUDE.md](../../CLAUDE.md) (hard rules), [handoff-agent-7.md](handoff-agent-7.md)
section 2 and 3 (the user's preferences, commit rules, CPU limits; still valid), then the roadmap
[roadmap-2026-11.md](roadmap-2026-11.md) (the user's plan, decisions at its top) and your memory
(`decisions-2026-10-28-roadmap.md`). This file says what is done and what comes next.

## State
- **R1 (area generator complete): DONE.** Both 50-seed client batches match on every layout
  (batch 1 218,705 of 218,705, batch 2 218,802 of 218,802), batch 3 (new seeds, master seed 20261028) 218,678 of 218,678 after one more fix (P1_4: the
  pre-unique room is an overlay feature room on `.dgr` maps); Poe2Server `3b05682`, fully gated on
  all three batches.
  Matrix 4469 match / 0 partial / 0 fail / 39 untested (combinations where the client itself errors
  on every seed, listed in [../status/terrain.md](../status/terrain.md#the-rows-that-do-not-match-wp715-2026-10-28)).
  Without a forced layout the generator picks the client's natural topology and orientation
  (`poe-world/src/map_dirs.rs`; 4,173 of 4,173 in `w13-nat1` and 8,368 of 8,368 in batch 3's natural
  part), the instance server forces that choice in AreaInfo (`[terrain] force_layout`, default on,
  WP7.13) and generates every new instance's terrain in the background (`[terrain] generate`,
  default on, at most 2 at once). RE: [../re/terrain/shared-passes.md](../re/terrain/shared-passes.md)
  ("The natural choice as ported"), [../re/terrain/tgr.md](../re/terrain/tgr.md) ("R1").
- **Track U (UI rework): DONE**, signed off by the user per app. Released: launcher **v0.5.0**
  (Poe2Launcher), client hook **hook-v0.4.0** (Poe2ClientHook, also the first release with the
  0.5.5e Hotfix 1 signatures); the web admin ships inside the server. `GameFiles/` has the
  hook-v0.4.0 DLL deployed.
- **Server release:** **server-v0.3.0** (Poe2Server, 2026-10-29; game 0.5.5e.hotfix1, needs the new
  `dat-schema-0.5.5e.hotfix1` release, created this session; `server-v0.3.0` is marked Latest).
- Nothing of ours runs (no gate, no client); `runtime/client.lock` absent.

## Next: R2, server-side collision (roadmap section R2)
The server's collision grid exists (`poe-world/src/collision.rs`, `[terrain] generated_grid`,
default off; used only when the server's tile hash equals the client's) and equalled the client's
`grids.bin` for 273 of 303 exports when last checked (2026-10-22; known: lattice flag 8 in MapSlash
and others, MapCrimsonShores' blob; [../re/terrain/doodads.md](../re/terrain/doodads.md)).
1. Grid exports at scale: an `export.txt` (same format as `jobs.txt`) for a sample of batch 3's
   jobs across every area and topology (a few thousand; exports write `layout.tsv`,
   `doodads.tsv`, `grids.bin`, `entities.tsv`), run with `terrain_batch.py run <run> --threads 8
   --no-deploy`, then `check_grids <run dir> --threads 8` (example in `poe-world`).
2. Fix the differences until the sample is byte-identical (the `entities.tsv` of an export may
   explain blockers that are not doodads).
3. Live test with server movement on the generated grid (`scripts/test-run.sh`, drift checker:
   [../protocol/movement.md](../protocol/movement.md) "Drift checker"): walls, cliffs, narrow
   passages, rolls into walls.
4. `generated_grid` on by default; the client upload (`[spawn.upload]`) off by default as a
   fallback; docs ([../guides/server-admin.md](../guides/server-admin.md),
   [../architecture/instance-world.md](../architecture/instance-world.md)), audit item S-08.
Then R3 (movement; starts with the timing RE: the client's per-frame dt comes from InitialState's
`frame_ms` at world `+0x11c`, which we send as 33) and R4 (entities, transitions, waypoints, world
map; needs the user's capture: checklist in [../re/captures.md](../re/captures.md) "Planned: capture
for R4"; tell the user when you are ready for it, and stop every client job before they record).

## Tools and lessons from this session
- **Client lock:** whoever drives the game client writes `runtime/client.lock` (name, time) and
  removes it afterwards; check it and `pgrep -a -f PathOfExileSteam` before starting the client.
- **Never `pkill -f`** (it matched and killed the shell that ran it); kill by PID.
- `POE_DBG_OCC=<file>` writes the overlay occupancy to a file named by its value (`=1` left a
  file named `1` in the hub root); other debug switches added: `POE_DBG_ROUND`, `POE_DBG_ROUTE`
  (stderr, like the trace's `route_edge` lines), `POE_NO_LEAGUE_RECTS`, `POE_EXPLODE_R1`.
- `area_trace` compares every lattice checkpoint (`borders` .. `fill`, now also `bdae0` and
  `be360`) and prints the first RNG divergence: the fastest way into a failing layout.
- `forced_natural <vectors>` (poe-world example): forces `area::choose_layout` and compares with
  unforced client generations; `map_dirs <area> [seed...]` prints an area's pin directions,
  candidates and picks; `area_vectors --natural` checks the natural pick.
- Baselines: `runtime/forced-baseline.tsv` (= `runtime/r1/g4-b1.tsv`, all match; previous
  `runtime/forced-baseline-pre-r1.tsv`); vectors in `dumps/0.5.5e/terrain/vectors/
  forced-results{,-full2,-batch3}.tsv`; the final gate script is `runtime/r1/gate4.sh` (all three
  batches, about 6 h at 8 threads). Batch runs: `dumps/0.5.5e/terrain/batch/
  {full,full2}`, `dumps/0.5.5e.hotfix1/terrain/batch/{batch3,batch3-nat,w13-nat1,r1-tr1}`.
- Rebuild an example before using it (`cargo build --release -p poe-world --example <name>`): an
  old `forced_natural` binary almost replayed batch 3 on pre-fix code.
- Delegated agents (T3 Code): when one stops on a usage limit, continue it in its own thread
  (`t3_thread_send`); the user does not want a fresh agent in its place.

## Commits of this session
Poe2Server `7654c1a` (natural choice), `af1e7e9` (WP7.13), `6784ec2` (generate on), `33981f0`
(R1 fixes), `b4ecbe1` (nested pointers), `3b05682` (P1_4), `a33a37d` + tag `server-v0.3.0`; Poe2ClientHook `06e66fa` + tag `hook-v0.4.0`;
Poe2Launcher `d626f7d` + tag `v0.5.0`; hub from `fda54bf` to this file (roadmap `242db78`,
R1 closed `739b67f`). The UI agent's commits are in the roadmap's Track U section.
