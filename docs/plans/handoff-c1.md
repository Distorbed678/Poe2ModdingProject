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

## Update 2026-10-21 (second agent)
Done: the click-mode held-click roll, the WASD-mode standing rolls and WASD-mode cursor sprints were run, the results are in movement.md ("In-game check, 2026-10-21") and the tracker. No server code changed (the problem 1 patch is in `git stash`, `stash@{0}`, not applied). Client closed, server stopped, `use_wasd_to_move=false` restored and checked, `runtime/server-c1.toml` deleted.

Open, in this order:
1. **Why the client no longer coasts after a Space-only cursor sprint** (movement.md problem 3): 1.4-2.7 cells after the stop in the first session, 23-28 in all later ones. Needs a release of Space with `0x2e6` and `0x1ac` in the same frame (a real keyboard or a hook change; `inup:0x20` gives `0x1ac` about 100 ms later), or an official click-mode capture. Only then decide on the problem 1 patch (stash: `CursorDirection` for `MoveDirection` with trailer byte 1, and the unscaled ack before a `(0, 0)` that arrives in F + 1).
2. **W/A/S/D injection is dead under the Agents rules** (problem 4): the sprint-end-with-W and walking-roll cases are not run. Try a client run with the rules off (`scripts/agent-desktop.sh off`, the user's go-ahead), or give the hook a focus fake for keyboard Raw Input.
3. How to run is unchanged (below); the helper scripts used were in the scratchpad and are gone: per case teleport, wait 3 s, write the entries, read the `drift server=...` lines.
## How to run (what worked)
- Server: `cp config/server.toml runtime/server-c1.toml`, set `drift_check = true` there, `POE_CONFIG=runtime/server-c1.toml target/debug/poe-server > runtime/server-run.log 2>&1 &` (own Bash call to stop it: `pkill -x poe-server`).
- Client: `scripts/test-run.sh c1 15` (`mode = "private"`, `login_mode = "steam"` in `GameFiles/poe-hook.toml`; do not deploy). Close it with the entry `close` in `GameFiles/poe-hook.keys`.
- Teleport into the open box: `python3 scripts/re/send_dev.py "teleport 650 500"` (corner start for long sprints: `612 462`).
- Key entries (one per line in `GameFiles/poe-hook.keys`): the cursor only changes reliably after a double move: `inmove:1000,300`, `wait:300`, `inmove:<x>,<y>`, `wait:400`. A tap roll: `indown:0x20`, `0x20`, `wait:250`, `inup:0x20` (`held` stays false); a Space hold for a sprint: `indown:0x20`, `0x20:1400`, `inup:0x20`.
- Screen to grid directions (window 2560x1440, player at about 1280,590): cursor right gives offset `(+38,-29)`, left `(-30,+32)`, up `(+45,+45)`, down `(-35,-35)`.
- The summary helper used: for each run, take the `drift server=...` lines after the entries were sent; `max_cells` per second and the last `cells`.

