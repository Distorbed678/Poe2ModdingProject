# Handoff: WP C1 in-game drift checks (2026-10-21)

**Status (movement-path agent, 2026-10-21): problem 1 (exactly diagonal cursor) and problem 2 (sprint towards a diagonal wall, the client's own pathfinding) are fixed and checked in game, see "Update, movement-path agent" at the end; what is left is problem 5 (sprint ended with W held since before the roll) and the checks listed there.**

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
- Server: `python3 scripts/dev-config.py --out runtime/server-c1.toml` (the committed `config/server.toml` is the safe one since WP D2: no Steam machine-hash login, no drift checker), set `drift_check = true` there, `POE_CONFIG=runtime/server-c1.toml target/debug/poe-server > runtime/server-run.log 2>&1 &` (own Bash call to stop it: `pkill -x poe-server`).
- Client: `scripts/test-run.sh c1 15` (`mode = "private"`, `login_mode = "steam"` in `GameFiles/poe-hook.toml`; do not deploy). Close it with the entry `close` in `GameFiles/poe-hook.keys`.
- Teleport into the open box: `python3 scripts/re/send_dev.py "teleport 650 500"` (corner start for long sprints: `612 462`).
- Key entries (one per line in `GameFiles/poe-hook.keys`): the cursor only changes reliably after a double move: `inmove:1000,300`, `wait:300`, `inmove:<x>,<y>`, `wait:400`. A tap roll: `indown:0x20`, `0x20`, `wait:250`, `inup:0x20` (`held` stays false); a Space hold for a sprint: `indown:0x20`, `0x20:1400`, `inup:0x20`.
- Screen to grid directions (window 2560x1440, player at about 1280,590): cursor right gives offset `(+38,-29)`, left `(-30,+32)`, up `(+45,+45)`, down `(-35,-35)`.
- The summary helper used: for each run, take the `drift server=...` lines after the entries were sent; `max_cells` per second and the last `cells`.


## Update, third agent (2026-10-21, KWin Agents rules off)
Done, results in movement.md (second table of "In-game check, 2026-10-21" and problems 1, 3, 4, 5):
- **Problem 4 solved**: W/A/S/D did not move the player because the prefix was in click mode (`user_input_mode=mouse_and_keyboard`); real WASD mode is `user_input_mode=wasd` (+ `last_selected_KBM_input_mode=wasd`). Injected W works with the game window active or not. The first agent's "WASD mode" rows were click mode (`use_wasd_to_move` is unrelated). setup.md says so now.
- **Problem 3**: the Space-only free-angle cursor sprint coasts correctly again with the rules off (11 runs, click and WASD mode, 0.1-4.2 cells after the stop); the `0x1ac` timing is not the cause (still 2-3 frames late). Likely the hidden Agents desktop; to prove it, run the same case once with the rules on. Any sprint-end check must run with the rules off.
- **Problem 1 re-measured, still open**: exact diagonal cursor 20 cells after the stop (4 of 4); the client's `(0, 0)` comes 4-5 frames after the sprint-off batch, beyond `sprint_release_window_frames` (3), so the saved patch (`git stash@{0}`; copy in the first agent's scratchpad) is not enough and not applied.
- **New, problem 5 (open, server change wanted)**: a sprint ended with W held since before the roll (no `0x63` stream) is 14-17 cells off: the unscaled `0x64` does not restart the client, which coasts 30 frames and resumes the walk by itself 31 frames after the batch (same 13.8/11.2 as on 2026-10-20). Proposed model in problem 5; `Locomotion::end_sprint`/`finish_sprint_by_release` is the place (no-stream + key held: no `UnscaledAck`, coast, then a `Due` that resumes the walk in the key direction), with a replay fixture from `runtime/movement-trace.tsv` of such a run (not kept) or a new unit test.
- Not run: W released before Space while sprinting, W pressed during a sprint (the key entries are sequential; a combined entry in the DLL would be needed, which also gives `inup` and the posted key-up in one frame like a real keyboard), and the official answer to problem 5 (needs a capture).
- How to run, additions: set `user_input_mode` per mode while the client is closed; a KWin script (`workspace.activeWindow = ...`) activates the game window for a few seconds; `GameFiles/logs/poe-hook/<session>` and `runtime/movement-trace.tsv` (client `report` rows per frame, server `out-*` rows) show the per-frame client steps against the server's messages.


## Update, fourth agent (2026-10-21, rules off): problem 1 fixed (uncommitted when written)
Code: `MoveInput::CursorDirection` (click mode = `0x63` trailer byte 2 is 1, mapped in `servers/instance-server/src/movement.rs`), `Locomotion::is_cursor_vector` (key-shaped vector within `sprint_cursor_match_degrees` of the roll key's cursor offset = the cursor, which is what tells an exactly diagonal cursor from W in WASD mode, where the trailer bytes are `[0, 0, 0]` for both), click-mode sprints coast at the sprint-off batch, the unscaled ack goes out first even when the zero vector is in its frame, `sprint_release_window_frames` 3 -> 6, new config key `sprint_cursor_match_degrees` (10; `[movement]`, committed config and example). Unit tests: see movement.md "Coasting after a sprint". Saved patch `c1-cursor-direction.patch` is superseded (its `C1_EXP` hook is not in the code).
In game: exact diagonal Space-only sprint 20 -> 0.48 cells after the stop (click mode 4 runs, WASD mode 2 runs), free-angle 0.09-0.17 (4 + 2 runs), standing tap rolls 0.7-1.4 (roll tolerance). Table "Third part of the day" in movement.md. Timing finding: the client's `(0, 0)` comes in the same tick as the sprint-off batch; the "4-5 frames" were counted from the release.
Not done: W pressed or released during a sprint (a combined key entry in the DLL is needed), problem 5, problem 2. A W press with the cursor within 10 degrees of W during a cursor sprint is taken for the cursor (rare, unverified).
Probe used: a temporary `eprintln!` of the `0x63` trailer bytes (removed). Test method as in "How to run", plus `inmove:1000,300 wait:300 inmove:1280,300 wait:400 indown:0x20 0x20:1000 inup:0x20` from `teleport 612 462` for the diagonal (cursor straight up); the test prefix was left in `mouse_and_keyboard`, `use_wasd_to_move=false`.


## Update, movement-path agent (2026-10-21, rules off): problems 2 and the 1 % speed note
- **Walk speed: no deficit.** The client's steady step is 1.2333 (= 1.01 x 37 x 0.033, playback factor 1.01) with a short step every 9-12 frames; the mean per frame is 1.2210-1.2216 on both sides, `dt` is 0.033 in every frame, ticks are 33.00 ms. Nothing changed in the speed model; numbers in movement.md "Speeds".
- **The client has its own pathfinder** (bounded jump point search, `0x141ed0390`, pathing level 4 = 5 cells of wall clearance while it sprints). Ported: `crates/poe-game/src/clientpath.rs`, used by `Mover` (`[movement] pathfinder = "client"`, `"approx"` keeps the old one). Docs: `docs/re/pathfinding.md`, movement.md "The client's pathfinder" (before/after drift table). Tests: `cargo test -p poe-game --test clientpath` (needs `dumps/0.5.5e/terrain/grids-keep/terrain_grid_1866332366.bin`; a copy of the `G1_1` grid, git-ignored) and the unit tests.
- **DLL hook** `path_log` (client-hook submodule: `path_log.rs`, signature `path_plan`, `scripts/re/sig_spec_0.5.5e.py`), off by default: `POE_HOOK_PATH_LOG=1`. The submodule has uncommitted changes (commit and push inside it first, then bump the pointer).
- How the in-game cases were run: `runcase`-style loop of `send_dev.py "teleport x y"`, key entries, then the `drift server=...` lines of `runtime/server-run.log`; area `G1_1` (grid 1866332366, free box around x 985..1110, y 600..740); WASD cases need `user_input_mode=wasd` in the test prefix while the client is closed, restored afterwards.
- Open: dynamic obstacles (other entities) are not in the port; the one-waypoint deviation next to the start of a moving entity (5 of 541 plans); one standing roll against the diagonal wall in 4 runs ended 4.8 cells off (the other three 0.6).
