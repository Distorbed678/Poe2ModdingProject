# Roadmap from 2026-10-28 (after handoff 13)

Written with the user on 2026-10-28, after the natural topology choice, WP7.13 and WP7.15 were
done (state: [handoff-agent-13.md](handoff-agent-13.md), [../status/terrain.md](../status/terrain.md)).
Nothing here is started yet. The original plans this continues are
[orchestration-2026-10.md](orchestration-2026-10.md) (phase 2: movement; phase 7: terrain) and
[terrain-all-areas.md](terrain-all-areas.md); the small leftovers are in [open-items.md](open-items.md).

## Decisions (user, 2026-10-28)
- **Order:** the area generator is finished first (every layout of batches 1 and 2 matches), then a
  third batch; if it matches, the generator is complete.
- **Batch 3:** 50 new random seeds for every topology × orientation, plus a natural-choice section
  (every area × 20 seeds, nothing forced). "Complete" = every layout matches, except combinations
  where the client itself errors on every seed (the 39 listed in terrain.md).
- **Movement timing:** measured (delta) time, after RE: first find what the client does with the
  tick message (`0x2f4`) and its frame playback, then the server measures the real elapsed time per
  frame, simulates with it and sends the same value if the client reads one; ticks scheduled
  against absolute deadlines, target 33 ms (30 fps). Whatever the client does on its own, the
  server does too (server authority).
- **Movement proof:** every input sequence of the official captures replays with the same reply
  opcodes, fields and frame offsets.
- **Captures:** the user makes one read-only official session soon (checklist in R4.1).
- **Collision:** done when the server's grid equals the client's on a large batch sample, a live
  movement test passes, and `[terrain] generated_grid` is on by default; the client terrain upload
  stays as an off-by-default fallback.
- **Unlocks:** per character, **locked by default**; an admin command (and a web admin button)
  unlocks a character's acts and waypoints.
- **UI rework:** game-styled and minimal (dark, warm gold accents, serif headings, few elements on
  screen). Done by a **separate agent (Opus 5.5, medium effort) in parallel**, after the user has
  seen the brief (track U).

## Main line

### R1. Area generator complete (terrain)
1. MapCaldera (2 layouts in batch 1, 4 in batch 2) and MapCrimsonShores (seed 921210055): client
   traces (`POE_HOOK_GEN_TRACE=1`, `area_trace`), port, gate.
2. Full replay of batch 2 (`full2`, 218,802 layouts; last replay before the BossRush and later
   fixes); fix what fails; both batches all-match (client errors excepted).
3. Batch 3: `terrain_batch.py plan` with a new master seed (50 seeds × 4,508 combinations, about
   1.3 h client time) plus the natural section (`topology 0`, `orientation -1`, every non-fixed
   area × 20 seeds, like `w13-nat1`); replay (`forced_verify`, `area_vectors --natural` /
   `forced_natural`); fix and re-gate until it all matches.
4. Close: matrix and tracker updated, `terrain.md` says "complete", vectors merged.

Acceptance: batches 1, 2 and 3 match on every layout the client can generate.

### R2. Server-side collision
1. Grid exports at scale: an export list for a batch sample (a few thousand layouts across all
   areas, every topology), `check_grids` against the client's `grids.bin`.
2. Fix the known differences (lattice flag 8 in MapSlash and others, MapCrimsonShores' blob, which
   the entity stage of R4 may explain) until the sample is byte-identical.
3. Live test with server movement on the generated grid (walls, cliffs, narrow passages, rolls into
   walls), drift checker on.
4. `[terrain] generated_grid` on by default; the client upload (`[spawn.upload]`) off by default as
   a fallback (closes audit item S-08 for servers that keep it off).

Acceptance: the sample is byte-identical; live movement collides like the client.

### R3. Movement overhaul (1:1 with the official game)
1. RE of the timing: the `0x2f4` tick message, the client's frame queue and playback
   (`0x1421302c0`, speed factors), where the per-frame dt comes from (InitialState `frame_ms` at
   world `+0x11c`, the frame-queue values read by `0x1421306c0`), and whether the official server's
   ticks carry a time. Output: a timing section in [../protocol/movement.md](../protocol/movement.md).
2. Server clock: deadline scheduling (no `tokio::time::interval` bursts), measured frame dt,
   catch-up rules after a stall, target 33 ms; the same dt goes into the simulation and (if the
   client reads one) the tick message.
3. Server-side copies of what the client does by itself: the action speed factor (attacks while
   walking, gap #6), sprint coasting (problem 5, needs the R4.1 capture), roll profile, the client
   pathfinder (`clientpath.rs`), stance changes; collision from R2.
4. Replay tests for every input sequence of all official captures (extend the fixtures of
   `locomotion.rs`); the movement-gap rows of the tracker reconciled (open-items).

Acceptance: every captured sequence replays identically (opcodes, fields, frame offsets).

### R4. Entities, transitions, waypoints and the world map
1. **Official capture (the user's; one read-only session, about 15 minutes).** I write a checklist
   first: open the map (M) in two acts and switch acts, use a waypoint in a town and in the field,
   walk through 2 or 3 area transitions including a fog door, touch a checkpoint, and the movement
   cases still open (sprint ending with W held since before the roll; attacks while walking).
   `deploy-client --official` before, back to private after (agents desktop rules off meanwhile).
2. Object stage in `poe-world`: the objects of placed tiles (`.tmd`), rooms (`.arm`) and node tags,
   with cell and orientation (the same objects the map pins come from, `map_pins.rs`); checked
   against the client's `entities.tsv` area exports at scale.
3. `AddObject` (`0x2f7`) for area transitions (including fog doors), waypoints and checkpoints, with
   the components the capture shows; spawned per instance from the object stage.
4. Interaction: clicking a transition sends the player to the connected area (`WorldAreas`
   connections or the object's own target) and puts them at the matching transition there;
   waypoint travel between unlocked waypoints.
5. World map (M) for acts 1 to 4 and the interludes (not the endgame): the per-character unlock
   state (locked by default; stored with the character), what the client needs to show acts and
   waypoints as unlocked (quest flags / waypoint list, from the capture), an admin command
   `unlock <character> [acts|waypoints|all]` (and lock) plus a web admin button.

Acceptance: a character unlocked by an admin can open M in every act but the endgame, travel by
waypoint, and walk from area to area through transitions without commands.

## Track U (parallel): launcher and web admin rework
Separate agent (Opus 5.5, medium effort) in its own worktrees of `modules/launcher` and
`modules/web-admin`; not before the user has reviewed its brief. Style: game-styled minimal.
1. Brief and design tokens: the top 3 tasks of each app (launcher: play, pick a server, update;
   web admin: see who is online, fix a player, change a setting), a shared token file (colours,
   type scale, spacing, radii, motion durations and easing), a component list; 2 or 3 rendered
   previews of the main screens for the user before building.
2. Launcher: one main screen (Play, server, status), the rest one level down; motion: page and
   state transitions, progress, no layout shift; `prefers-reduced-motion` honoured.
3. Web admin: a calm dashboard, the advanced tools grouped and out of the way; same tokens.
4. Checks per repo (`npm run check`, tests, `cargo clippy` for the launcher), screenshots from a
   headless browser for review, the user's sign-off per app; releases only when the user asks.

## After the roadmap (backlog, ask before starting)
- Server release (`server-v0.2.x`) after R1 or R2; launcher release after track U.
- [open-items.md](open-items.md): mod overlays, ban commands, web admin class names (may fold into
  track U), mod download rate limit, launcher nested hook pin; G-PARTY capture and Windows tests.
- Deferred until items exist: remote equipment visuals, dynamic obstacles, party/trade.
