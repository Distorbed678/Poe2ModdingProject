# Movement (M3)

How the client moves the player and what the server sends back. Code:
- `servers/instance-server/src/player.rs` (session state, replies);
- `crates/poe-game/src/movement.rs` and `pathfinding.rs` (the simulation);
- messages in `crates/poe-proto/src/instance.rs`.

## Lockstep: the server starts every movement
- The in-game world is created in frame mode 2 (world +0x661, constructor `0x14212eb60` called from `0x14100b7e9` with a constant 2): **lockstep**.
- The client's `[NETWORKING] networking_type` option (`auto`/`lockstep`/`predictive`, read by `0x141939e40`) is a PoE1 leftover. With `auto`, the session update (`0x142120b70`) only switches a world whose +0x661 is 3. Below 70 ms latency it picks 2, above 90 ms it picks 0.
- So pressing W or clicking sends a request and nothing moves until the server answers.
- The answer starts the movement on the client, which then moves the entity itself every simulation frame (`dt = frame_ms / 1000`, see [instance.md](instance.md)). It walks along paths from its own pathfinder, around walls, until the movement ends.
- The server must simulate the same movement to know where the player is. Messages that carry a position (`0x64`, `0x1c8`) make the client snap to the server's position only when its own is **46 cells or more** away (squared distance ≥ `0x844`: `0x142149640` for `0x64`, `0x141e41ae0` for `0x1c8`; the snap is `0x141ddb570` SetLocationForcedNoCallbacks). Closer than that, `0x64` only bends the walk direction, and `0x1c8` keeps the client's position. That distance rule applies only outside lockstep (world vfunc +0x1d8 = 1): in lockstep a `0x64` whose cell differs from the client's is always a forced placement, **but only when its direction is not zero**; a `(0,0)` update ignores its cell (verified 2026-10-14 with the DLL's teleport dev command).
- **Frame playback** (`0x1421302c0`): the client plays received frames in real time with a speed factor (world +0x670). It plays at 1.01× with one spare frame queued, 1.5× with 2–29, 2× with 30–59 and 5× with 60 or more, and goes back to 1.0× once the queue is empty. The world never runs past the last received frame. So late frames freeze movement and bursts make it rush. The server sends one frame marker per tick from a steady timer, with `TCP_NODELAY` set.

## Walking speed
- The Pathfinding component (constructor `0x141dcd020`) stores the template's `Pathfinding.base_speed` as a float at +0x550 and a multiplier of 1.0 at +0x55c.
- The speed getter `0x141dcce20` returns `base_speed × (1 + movement_velocity_+permyriad / 10000) × (1 + virtual_action_speed_+% / 100) × multiplier`, capped by `movement_velocity_cap`.
- Units are **grid cells per second**, with 23 cells per tile and a cell of 250/23 world units. Characters have `base_speed = 37`.
- Measured with the DLL's per-frame log: a 60-cell click-to-move took 49.1 frames of 33 ms, which is 37.0 cells/s. The positions the client logs are interpolated for rendering, so single frames can show half or one-and-a-half steps.

## WASD
- **C→S `0x63` MoveDirection**: `u8, varint x, varint y, u8[3]`. Sent when the pressed direction changes. The vector is in grid axes with a length of about 180: W = (127, 127), D = (180, 0), S = (−127, −127). (0, 0) means stop. The trailing bytes have been zero so far.
- **S→C `0x64` MoveDirectionUpdate** (simulation message; reader `0x141f6b0e0`, Actor handler `0x142149640`): header, `varint dir x, varint dir y, varint grid x, varint grid y, u8`.
  - With a non-zero direction, the client first moves the entity to the given cell if it is elsewhere (`0x141ddb570`), then stores the direction (Actor +0x400) and starts walking (`0x141e4f9c0`).
  - A zero direction stops it, with no position check.
- WASD walking is the basic **Move** action toward the current cell plus the input vector, in cells (`0x141e4f9c0`). So the client pathfinds around obstacles toward a point about 180 cells away, rather than sliding along walls.
- The server answers every `0x63` with `0x64`: the same direction plus its cell of the player (the client's reported one). It then moves straight in that direction (no terrain until M5) and follows the client's reports.

## Click-to-move
- Click-to-move is an input mode: `user_input_mode=mouse_and_keyboard` in `poe2_production_Config.ini` (the other modes are `wasd`, `touch` and `gamepad`). In WASD mode, a click on the ground does not move the player.
- **C→S `0x1a5` UseSkill** (built by `0x14210f920`; class vtable `0x1434e5b40`; body serializer `0x141ecdd00`):
  - `u8 local player, u32 skill id, u16 flags, u8, u8 target flags`;
  - `[u32 target entity if target flags bit 3]`;
  - `i32 x, i32 y`. Without a target entity, the location is **relative to the player's cell**.
  - A left click uses the Move skill `0x20002909`.
- **Held button:** while the button stays down, the client repeats `0x1a5` about every 70 ms with `target flags` bit 4 (`0x10`) set and the new relative cursor position.
  - A new StartAction for each repeat restarts the client's Move every 70 ms and loses most of a frame's step each time (measured 1.23, 0.29, 1.23, 0.38 … cells per frame).
  - `0x1cb` UpdateActionTarget cannot re-aim it: the client applies `0x1cb` only to the action in Actor slot +0x238 (case `0x14214a290`), and a Move is in slot +0x240. With `0x1cb` the client kept walking to the first target and stopped there every ~1.1 s.
  - So the server drives a held click like WASD: repeats are answered with **`0x64`** toward the cursor (vector scaled to length 180), sent only when the direction changes by more than about 8° or the client's walk has ended. A `0x64` direction change costs about 5 % of one step. On **`0x1ad`** the server sends `0x64` (0, 0), which also clears the walk direction the client would otherwise resume, and a StartAction to the last cursor point. A cursor within 3 cells of the player stops the walk.
  - Measured: a 7.5 s hold with cursor sweeps runs at a steady 1.23 cells per frame (15 direction updates, 2 StartActions).
- **C→S `0x1ad` SkillReleased** follows when the button is released: `u8 local player, u32 skill id, u8`.
- **S→C `0x1c8` StartAction** (reader `0x141f696b0`, Actor handler `0x142149640`): header, then:
  - `u16 flags`, where bit 7 means a `u32` target entity follows;
  - `varint x, varint y`: the entity's cell when the action starts;
  - `varint target x, varint target y`: absolute cells;
  - `u32 skill id, u16 action id, u8`.

  The client builds the action from the actor's skill with that id (`0x141e89130`), sets its id and target, and starts it (`0x141e3ef80`). Server action ids are 1..0x7fff. Negative ids are the client's own predictions, which the predictive mode used.
- The server answers a Move use with StartAction (the target is the player's cell plus the relative offset) and walks the same path. Measured: the server's end cell equals the client's after clicks in the open and around walls.

## Skills, ids and dodge roll
- **Basic actions** come from the class template's `Actor.basic_action` list (Move, Interaction, …). They exist on the client without the server sending them. Their skill id is `0x20000000 | ActionTypes key`, where the key is the table's second column: Move = 10505 (`0x2909`), DodgeRoll = 60748 (`0xed4c`).
- **Other skills**, dodge roll included, must be in the Actor component's skill list (see [entities.md](entities.md#actor-skills)). The server gives every character `[player] innate_skills`, which defaults to `DodgeRollPlayer`.
  - Its ids are 1, 2, … (small ids, distinct from basic actions and item skills; item skills have the top bits `100`).
  - The first area also has `DodgeRollPlayerTutorial`.
- Pressing **Space** (`use_dodge_roll`) sends **`0x1a5` UseSkill** with the dodge roll's skill id and the cursor position (relative). It also sends `0x1ac` (`u8 local player`, ignored) and `0x2e6` SprintInput (see [Sprint](#sprint)).
- **Skill stats come from the server.** The client reads a skill's stats from the stat list in its Actor entry, not from the tables, and it reads the *computed* stats.
  - The roll's length is `dodge_roll_travel_distance` (`0x141aa90d0`: stat key `0x57fa` × (1 + `movement_velocity_+permyriad`)).
  - Without it the client logs "Trying to do pathfinding action movement with base travel distance <= 0" (`0x141b8be20`) and does not roll.
  - The server sends the stat set's constant stats and its implicit stats (as 1). There is no stat calculator yet (M8), so each `..._base_...` stat is also sent under its name without `base_` (`dodge_roll_base_travel_distance` 46 → `dodge_roll_travel_distance` 46).
- The server answers with StartAction for that skill and simulates the roll (`Motion::Roll`):
  - **Length:** 46 cells toward the target.
  - **Speed profile** (measured): it starts at about 3.9 cells per 33 ms frame, loses about 0.225 cells per frame every frame, then holds about 0.93 cells per frame. 46 cells take about 28 frames, driven by the roll animation on the client.
  - **Walls:** on the client the roll does not pathfind around them: it goes straight and slides along a wall. The server has no walls until M5 and takes the client's reported position.

## Steering a running skill
- **C→S `0x1a8` UpdateSkillTarget**: `u8 local player, i32 dx, i32 dy, u32 skill`. Sent about every 70 ms while a roll (or sprint) runs, or a click is held, with the cursor position relative to the player's cell.
- **C→S `0x1aa` UpdateSkillTargetEntity**: `u8 local player, u32 entity, u32 skill` (the same, aimed at an entity).
- **S→C `0x1cb` UpdateActionTarget** (simulation message): header, `u16 action id, i32 x, i32 y` (absolute cells). The server answers each `0x1a8` for the running action with it and steers its own simulation the same way.
- Unknown client messages end the parse of their whole batch (there is no length prefix), so before `0x1a8` was decoded, a `0x1a5` in the same batch was lost. That was one cause of the input lag.

## Input during a roll
- The client keeps WASD and clicks during a roll and sends them, but it only acts on them once the roll ends. If the server answered them immediately (a Move StartAction), the roll was cut off halfway.
- The server now keeps the latest input while the roll runs (`Pending`) and starts it when the roll ends. A held WASD direction is sent with the client's own cell, so nothing snaps.

## Lag (M3.5)
- **No lag compensation to turn on.** The world is always lockstep (above), and the client plays simulation frames as they arrive. `0x1d` PingReply only feeds the latency display. Lag therefore came from the server.
- **Server frame time.** A debug build's A* (box-shaped search, heap allocations per node) took about 108 ms per 33 ms frame on the Riverbank grid, so the server fell behind. The A* now uses flat arrays over a box around start and goal (`MARGIN` 24 cells) with a node budget. It takes about 0.5 ms in a debug build and less in release. The server logs a warning when a frame takes longer than the tick. Run the server with `--release` for play.
- **Dropped batches** (unknown `0x1a8`, above).
- **No server collision (2026-10-14).** The server simulation no longer uses the client-exported walkability grid: no A* paths and no wall sliding. It moves in straight lines and takes the client's position from its reports, and relative input positions use the freshest report when the input arrives. Collision comes back with the server-side terrain (M5); the generic A* (`poe-game::pathfinding`) and the `Walkable` trait stay for that. The grid is still used to put a restored saved position on a walkable cell.
- **Measured after the fixes:** WASD frames arrive every 33 ms with a constant step of 1.23 cells (37 cells/s), including direction changes; a held click is the same. The remaining irregularities are the client's own: it slides along walls and turns at corners (the WASD target is 180 cells away and the client pathfinds to it), and every ~12th logged frame is 29 ms / 1.08 cells, from its 1.01× playback with one spare frame.
- **Reports are the player's position.** Once reports have arrived in a session, the server never moves the player itself; a gap keeps the last report. Before this (2026-10-14), a stretch without reports let the wall-less server simulation walk on, and the client was teleported when the gap reached 46 cells.
- **Direction changes.** A `0x64` or `0x1c8` sent after server frame *t* applies in client frame *t+1* (client frame numbers equal server ticks). In that frame the client puts the player on the **centre of the cell it is given** (`0x142149640` at `0x142149754`: a cell other than its own is set as its position in this world mode; with its own cell it walks from that cell's centre) and walks in the new direction from frame *t+2*. So the turn frame's step goes from where the client is to that centre, and the cell decides how the turn looks:
  - a cell from the last report (one frame old) pulled the player back by about a step at every turn;
  - the cell under the predicted position still made the turn step up to 0.7 cells too long or short;
  - the server now predicts the client's position at the start of frame *t+1* (last report + per-frame velocity × the frames in between, from the tick count) and picks, among the cells around it, the one whose centre is about one normal step away along the new heading (walkable cells only, from the client-exported grid). Standing still, it picks the neighbouring cell toward the new heading, so the first frame is already a step instead of a dead frame.
  - `[instance] turn_cell` selects the cell. Compared in play (2026-10-14, a player who changes direction every 1–2 frames, often in two key steps 50–70 ms apart):
    - `own`: the client's own cell, so it only re-aims its walk (`0x141e4f9c0` keeps a running Move and sets its new target). It then walks to the centre of its cell first and loses most of that frame: 282 of 294 turns hit the right cell, and the turn frame still averaged 0.81 cells, half under 0.6. Felt like "an invisible wall for a frame".
    - `heading`: the cell one normal step along the new heading. Positions never stall, but the player is placed at every turn. Felt the same as `own`.
    - **`natural` (default)**: the cell the client would reach in that frame, so the turn frame keeps the old motion and the new walk starts from that cell's centre. Felt "way better", with occasional micro stutters.
  - **Prediction after a turn.** The two reports around a placement do not show walking speed (a placement can read as 2 cells per frame), so with turns 1–2 frames apart the velocity from the last two reports placed the player up to 4.7 cells off. Until two reports after the last turn exist, the server predicts from that turn instead: at the end of its frame the client is at the centre of the cell sent and then walks along the new direction at walking speed. Replaying a recorded session (206 walking turns): placements more than 0.75 cells off the client's natural position went from 18 to 3, the worst from 4.70 to 1.16 cells (the average 0.47 → 0.38; rounding to cell centres alone accounts for about 0.36).
  - **Turn timing** (`[instance] turn_timing`, on): a turn may be held back by one frame when the client's path passes nearer a cell centre in that frame (by at least 0.1 cells), so the placement is smaller; an input arriving meanwhile replaces it. A reply sent from the next tick still applies in the same frame as an immediate one, so a held-back turn goes out from the tick after that. Tight-turn test: average placement 0.38 → 0.35 cells, hitches 11 → 2; some turns answer 33 ms later.
  - WASD during a dodge roll steers the roll; placing on the natural cell keeps the roll smooth, while the own cell made the roll's next frame short.
  - Measured with 32 rapid WASD turns (every 110–200 ms): the turn frames' step length deviated from the normal 1.23 cells by 0.19 on average with 13 frames off by more than 0.35 (worst: a whole step), and now by 0.09 with 3 (worst 0.38).
- **Movement trace** (`[instance] movement_trace`, on in the dev config: `runtime/movement-trace.tsv`): every client position report (one per client frame), movement input, movement reply (with the cell sent) and server tick, as TSV. `scripts/re/movement_trace.py` lists the frames where the player moved backwards or stalled, with the replies that caused them.
- **Report target.** The DLL sends reports to the instance server's port (`position_sync_port`, set by `cargo xtask deploy-client` from `[instance] public_port`). Before, it used the last loopback connection the game made, which other local connections can replace. The server logs "client position reports started" per client, and warns when none arrive within 5 s.
- **Position sync (dev bridge until M5).** The server's pathfinding and collision only approximate the client's, so positions drifted near walls (up to about 20 cells after a roll along a wall), and the client snapped back on the next server position. The client DLL now reports the player's position every frame (`position_sync`, see [client-dll.md](../client-dll.md#position-sync)). The instance server (`servers/instance-server/src/sync.rs`) listens for these reports on its own port (UDP, loopback only) and takes the client's position while reports are fresh (250 ms). It also uses the reported action to know when the client's roll ended. Without reports (option `[instance] position_sync = false`, or a non-local client) it falls back to its own simulation.

## Sprint
- Holding Space after a dodge roll makes the character sprint (`sprint_movement_speed_+%` = 50 in the character templates, after `dodge_roll_sprint_minimum_time_ms`).
- **C→S `0x2e6` SprintInput**: `u8 local player, u8 held` (1 on press, 0 on release). **S→C `0x2e7` SprintState**: header + `u8 held`. The server echoes it, which sets Actor +0x2a2 bit 5 (key held) on the client.
- The client decides at the end of a roll whether it carries on as a sprint (`0x141b76c20`). Its "is sprinting" check (`0x141e5d490`) reads Actor +0x915 or the stat `is_sprinting`.
- **Status: partial.** With the echo alone, the client starts a short sprint after the roll and falls back to walking. Setting `is_sprinting` and the speed bonus as stats (**S→C `0x1d3` StatsUpdate**: header, `u8 set, varuint n, n × (varuint key, varint value)`; **S→C `0x1d8` SkillStatsUpdate**: header, `u32 skill, u8 container, varuint n, pairs`) makes the sprint run at the right speed, but the client then drops its Move action right after the roll (`0x141e4de20` stops Move when Actor +0x2a1 bit 2 is set), and restarting it from the server only stutters. Sprinting on the official servers is most likely a buff with these stats. Full sprint therefore waits for the buff system (M8). The messages are decoded and kept, and the server simulates a sprint only when no client reports arrive.

## Facing the cursor (2026-10-15)
- On the official servers the player faces the cursor at all times (standing and while walking with WASD, armed or not), except while it walks by click-to-move.
- Client facts:
  - The facing is Positioned **+0x44c** (radians), written by SetOrientation (`0x141ddc6a0`) unless the orientation is **locked** (+0x1e5 bit 1). The angle of a grid direction (dx, dy) is `atan2(dy, dx) + π/2`, mod 2π.
  - In our sessions only the movement step (`0x141de3020`) calls it, turning the player toward its path (orientation mode +0x1e3: 1/3 along the path, 2 backwards). Moving the mouse never changes the facing, and the client sends no cursor or aim messages (`0x63` carries only the direction, `0x16` is chat).
  - **S→C `0x1df` SetOrientation** (Positioned, reader `0x141f6af00`, handler `0x141dda640`): header, `f32 angle, u8 locked`. The handler calls SetOrientation (skipped while locked), then sets or clears the lock. The Positioned initial data also carries the facing (+0x44c) and a lock bit (flags0 bit 7).
  - How the official client gets cursor facing is not known yet.
- The server does it (`[instance] face_cursor`, dev bridge like position sync): the client DLL adds the cursor (window client position and size) to its position reports (`PSY2`). The server maps it to a grid offset from the player with an affine fit (`[instance] cursor_to_grid`, from ten dodge-roll targets at 2560×1440; 1–4 cells off at 35 cells; scaled by window height), and sends `0x1df` locked toward it when the direction changes by more than ~3°. A locked facing cannot be changed, so a new angle is sent as unlock, then set + lock. While the player rolls or walks by click-to-move, the facing is unlocked so movement turns it.

## Accuracy (until M5)
The server has no terrain until M5: it moves in straight lines and follows the client's position reports, so it agrees with the client wherever reports arrive. Without reports (`[instance] position_sync = false`) its position drifts from the client's near walls, and the client snaps back only if the difference reaches 46 cells. Until 2026-10-14 the server walked A* paths on the client-exported grid (layer 0, a 0–5 clearance field); that was removed at the user's request, and an exact port of the client's pathfinding and collision belongs with the server-side terrain (M5).

## Pause
`0x1ff` PauseRequest also pauses the server simulation (`Player::paused`). Nothing moves on either side while the escape menu is open in a solo game.

Verified: with D held, Escape after 0.7 s, Escape again 2 s later, then D released 0.7 s after that. The client stopped for the 2 s, and both sides ended one cell apart, (330, 1106) on the server and (331, 1105) on the client.

## Saved location
- When a session ends (exit to character selection, log out, closing the game or a dropped connection), the instance server stores the character's area, the area's terrain seed and its grid cell. The fields are `poe_store::Location`, in migration `0005_character_location.sql`.
- On the next login into that area, the login server puts the location in the ticket. The instance server sends the same seed, so the layout is the same, and spawns the player on the saved cell (or the nearest walkable cell if it is blocked).
- Characters that have never been in the game still start at the area's start position.

## Test tooling
- **`poe-hook.keys` entries `indown:<vk>` / `inup:<vk>` / `wait:<ms>`:** hold a key while other input arrives.
- **`poe-hook.keys` entries `in:<vk>[:ms]`:** a synthetic Raw Input key (`WM_INPUT`, answered by the hooked `GetRawInputData`, with `GetKeyState` reporting the key held). WASD is read this way.
- **`inclick:x,y` / `inmove:x,y`:** override `GetCursorPos` and post mouse messages. **`inpress:x,y` / `inrelease:x,y`** hold and release the left button (held click-to-move); `GetKeyState(VK_LBUTTON)` reports it held.
- Click-to-move needs `user_input_mode=mouse_and_keyboard` in the test prefix's `poe2_production_Config.ini` (edit it only while the client is closed: the client rewrites it on exit).
- **Posted keys (`0x20`):** reach the input bindings that are read from window messages, such as Space for dodge roll.
- Nothing is sent to other windows. `SendInput` is not used, because it goes to whatever window has focus.
- The DLL's `dump_object_types` logs the player's world position at every simulation frame (`frame #n … step … grid`) when `movement_log` is on (or `POE_HOOK_MOVEMENT_LOG` is set), and the WASD direction (Actor +0x400) and action slots when they change.
