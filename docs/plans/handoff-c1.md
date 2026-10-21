# Handoff: WP C1 in-game drift checks (2026-10-21)

State: code and docs of WP C1 are committed as `c947a15` (movement gaps #11 and #12). Nothing has changed in the code since; `cargo build` passes. The in-game checks were stopped half-way (low usage). Client and server are stopped, the test prefix's `use_wasd_to_move` was never edited and is still `false`, the temporary server config (`runtime/server-c1.toml`) is deleted.

## Results so far (click-to-move mode, `use_wasd_to_move=false`, drift checker, cells: max in a second / after the stop)
Server `target/debug/poe-server` with `[dev] drift_check = true`, log `runtime/server-run.log` (lines `drift server=... max_cells=...`; the log is overwritten by the next run). Test area `G1_1`, open ground = the 100 x 100 box x 600..710, y 450..560 of the client grid `dumps/0.5.5e/terrain/grids/terrain_grid_3958003549.bin` (found with a run-length scan of layer 0).

| Case | max | after stop |
|---|---|---|
| standing tap roll, cursor right / left / up / down (4 runs) | 3.9 | 0.34 / 0.61 / 0.36 / 0.56 |
| click east, roll 1 s later (click walk, then roll) | 3.9 (1.3 during the walk) | 0.47 |
| standing roll east into a wall (first runs, player in a pocket at the east edge) | 3.1 | 2.4, then 0.7 |
| sprint, Space only (free-angle cursor `1500,200`), Space held 1.0 s | 3.9 | 1.44 |
| same, Space held 1.4 s | 3.9 | 2.73 |

The two sprint rows are the gap #11 coast case (cursor sprint released with Space only): the server coasts like the client (before WP C1 it stopped at the `(0,0)` and was about 18 off).

Problems found:
1. **Exactly diagonal cursor**: with the cursor straight up on the screen the grid vector is exactly `(45, 45)`, which `is_key_vector` (`locomotion.rs`) takes for a key. The server then treats the zero vector as a key release (stops at once) while the client coasts: after stop 20.4 and 19.9. Real cursors are almost never exactly diagonal. A better rule is wanted (idea: remember whether a key vector was held when the roll started, or count the 0x63 vectors that changed each frame while Space was down); `a_key_released_just_after_the_sprint_stops_at_once` and `a_sprint_ended_after_the_cursor_direction_coasts_to_a_stop` pin the current rule.
2. **Sprint towards a diagonal wall**: a 1.4 s Space-only sprint east from `650 500` ended at 11.2 cells drift: the client turned along the wall at about 10 cells clearance (it bent at (716,458) where the wall is at y 447), the server hugged the wall (ended at 730.6, 450.0). That is the pathfinder clearance gap (movement.md: the pathfinder is an approximation), not the sprint end.

## Not run yet
All WASD-mode cases (needs `use_wasd_to_move=true` in the test prefix's `poe2_production_Config.ini`, line 105, set while the client is closed; restore to `false` at the end):
- WASD sprint end: W held since before the roll, Space held 1.4 s, release Space, keep W 2 s, then release W (old result 13 / 10; target under 8 and about 0 after the stop). Also W held with Space released first, then W released.
- standing roll in WASD mode (three cursor directions), walking roll (W, then D).
- click-to-move: roll with a held click (`inpress`/`inrelease`).

## How to run (what worked)
- Server: `cp config/server.toml runtime/server-c1.toml`, set `drift_check = true` there, `POE_CONFIG=runtime/server-c1.toml target/debug/poe-server > runtime/server-run.log 2>&1 &` (own Bash call to stop it: `pkill -x poe-server`).
- Client: `scripts/test-run.sh c1 15` (`mode = "private"`, `login_mode = "steam"` in `GameFiles/poe-hook.toml`; do not deploy). Close it with the entry `close` in `GameFiles/poe-hook.keys`.
- Teleport into the open box: `python3 scripts/re/send_dev.py "teleport 650 500"` (corner start for long sprints: `612 462`).
- Key entries (one per line in `GameFiles/poe-hook.keys`): the cursor only changes reliably after a double move: `inmove:1000,300`, `wait:300`, `inmove:<x>,<y>`, `wait:400`. A tap roll: `indown:0x20`, `0x20`, `wait:250`, `inup:0x20` (`held` stays false); a Space hold for a sprint: `indown:0x20`, `0x20:1400`, `inup:0x20`.
- Screen to grid directions (window 2560x1440, player at about 1280,590): cursor right gives offset `(+38,-29)`, left `(-30,+32)`, up `(+45,+45)`, down `(-35,-35)`.
- The summary helper used: for each run, take the `drift server=...` lines after the entries were sent; `max_cells` per second and the last `cells`.

## What is left
1. Run the not-run cases above (WASD mode) and add the numbers.
2. Decide on problem 1 (diagonal cursor) and fix it on top of `c947a15` if cheap; update the unit tests.
3. `docs/protocol/movement.md`: add a short "In-game check, 2026-10-21" table next to the 2026-10-20 one in the Drift checker section (the rows above plus the WASD ones), and the two problems.
4. `docs/status/tracker.toml`: the notes of "Dodge roll" (still says "open: roll heading ... (gap #12)") and "Sprint" (still says gap #11) are to be rewritten with the drift numbers; keep the status `partial` unless every case is under 8 cells and about 0 after the stop.
5. Run `python3 scripts/check-links.py`, `cargo build && cargo test`, do not commit (the coordinator does).
