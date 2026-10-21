# WP2.4 handoff (standing roll drift, in-game verification)

Not committed. `cargo build` passes. Only `cargo test -p poe-game -p instance-server` was run (all green, 2 pre-existing ignored); no full workspace test run.

## Root cause

1. **The missing `0x1a8` was the test tool, not our replies.** `0x20` / `0x20:ms` posts WM_KEYDOWN and WM_KEYUP back to back. The roll skill's held state is polled with `GetKeyState`, which the hook reports as released. So `0x1a5` and `0x1ac` (SkillInputEnd) reach the server in the same frame, before the roll's `0x1c8`. The client then does not steer: 4 of 4 such taps sent no `0x1a8`. With a real hold the client sends `0x1a8` every 2 frames, with skill id 1, and our server answers each with `0x1af`. So the skill id (1 against official `0x80000040`) is not the cause. The slot byte (official `0x40`/`0x41`, ours 0) and the `u18/u19/u1a` header fields were not shown to matter either.
2. **A real hold that works:** `indown:0x20`, `0x20`, `wait:250`, `inup:0x20`. For a sprint hold, use `indown:0x20`, `0x20:1400`, `inup:0x20`.
3. **Why the server's standing roll did not move in the failed run:** the server rolled along the `0x1a5` cursor offset, `(-25,148)`. The client, unsteered, rolled along its own initial heading (the last facing, quantized to 45 degrees). The server's heading hit a wall after 12 cells. The old code then switched the roll to Idle on the first blocked step, so the server stopped while the client went on.
4. **Initial heading of a roll.** For the first 2-3 frames the client rolls in a 45-degree-quantized direction (seen in our tests and in official sessions), then follows the `0x1a8` offsets. This is not modelled. Near obstacles it can put the client on the other side of a rock (a 28-cell drift in one test, a rock 14 cells north of (2379,734)).
5. **Sprint rolls move differently from tap rolls.** Frames 16-30 speed up from 0.9 to 1.7 cells per frame, with a catch-up step of about 2.4 at frame 30 (7 official sprint rolls). The old server model tapered to zero, so it lagged about 10 cells after a sprint roll.

## Changes

- `crates/poe-game/src/movement.rs`:
  - `roll_speed` is replaced by the official per-frame table `ROLL_STEPS` (30 values, sum 45.9).
  - New `ROLL_SPRINT_STEPS` (frames 16-30 of a roll that became a sprint) and `Mover::sprint_roll()`. `Motion::Roll` gains a `sprint: bool` field, and `roll_speed(t, sprint)` takes it.
  - A blocked step no longer ends the roll. It costs no distance, and the roll ends when the table is over.
  - New tests: `roll_follows_the_official_steps`, `a_sprint_roll_speeds_up_after_frame_15`. `roll_covers_its_length` now expects 30 frames.
- `crates/poe-game/src/locomotion.rs`: `start_roll` passes `sprint: false`, and `Due::SprintStart` calls `self.mover.sprint_roll()`.
- `runtime/server-drift.toml` was regenerated from `config/server.toml` with `drift_check = true` (git-ignored).
- The docs, the tracker and history.md are **not** updated yet.

## Drift (cells; max over a second / value after the stop)

The last build includes the sprint-roll profile. Teleport blips (40-200) in the logs are the `teleport` command, not drift.

| Scenario | Max | After stop |
|---|---|---|
| WASD walk and turn, open ground (2556,763) | 3.2 | 0.4 |
| Roll while walking W | 4.1 | 1.8 |
| Roll while walking D | 3.9 | 3.1 |
| Standing roll, cursor west | 3.9 | 0.9 |
| Standing roll, cursor north-west | 3.9 | 0.7 |
| Standing roll, cursor 2 (1900,1000) | 28 | 28 (the cursor offset was not read; it may have rolled into a wall) |
| Sprint W hold (before the sprint-roll profile) | 8.9 | 2.8 |
| Sprint W hold (after the profile) | 11.2 | 10.0 (probably the client sliding into a wall at the end of W's 99-cell run; check) |
| Standing sprint hold | 3.9 | 2.4 |

Not run: click-to-move and the `user_input_mode` edit.

## Next steps

1. Re-run the sprint W hold with a shorter W run, or one that does not reach the wall, to see whether the sprint-roll profile gives under 8. Check the client trajectory in the DLL session's `poe-hook.log` ("frame #" steps).
2. Look at the (1900,1000) cursor roll: the client trace shows whether it hit the rock at (2369-2394, 693-711).
3. Optional: model the roll's initial 2-3 frame heading, or make the server slide around obstacles like the client.
4. Click-to-move test (edit `poe2_production_Config.ini` only while the client is closed).
5. Cleanup:
   - `git grep -n 'turn_cell|turn_timing|face_cursor|cursor_to_grid'`.
   - Keep the DLL's `movement_log` and document it.
   - Add the key-tool finding above to `docs/guides/setup.md`, the new roll profiles to `docs/protocol/movement.md`, and update `docs/status/tracker.toml` (`cargo xtask tracker`) and `docs/status/history.md`.
   - Run `python3 scripts/check-links.py`.
6. Format with `rustfmt --edition 2024 <file>` only.
