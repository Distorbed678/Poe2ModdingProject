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
- The server must simulate the same movement to know where the player is. Messages that carry a position (`0x64`, `0x1c8`) make the client snap to the server's position when its own differs.

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
- The server answers every `0x63` with `0x64`: the same direction plus its own cell. While moving, it plans a path toward that far point and re-plans every 6 frames.

## Click-to-move
- Click-to-move is an input mode: `user_input_mode=mouse_and_keyboard` in `poe2_production_Config.ini` (the other modes are `wasd`, `touch` and `gamepad`). In WASD mode, a click on the ground does not move the player.
- **C→S `0x1a5` UseSkill** (built by `0x14210f920`; class vtable `0x1434e5b40`; body serializer `0x141ecdd00`):
  - `u8 local player, u32 skill id, u16 flags, u8, u8 target flags`;
  - `[u32 target entity if target flags bit 3]`;
  - `i32 x, i32 y`. Without a target entity, the location is **relative to the player's cell**.
  - A left click uses the Move skill `0x20002909`.
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
  - The server sends the stat set's constant stats and its implicit stats (as 1). There is no stat calculator yet (M7), so each `..._base_...` stat is also sent under its name without `base_` (`dodge_roll_base_travel_distance` 46 → `dodge_roll_travel_distance` 46).
- The server answers with StartAction for that skill and simulates the roll (`Motion::Roll`):
  - **Length:** 46 cells toward the target.
  - **Speed profile** (measured): it starts at about 3.9 cells per 33 ms frame, loses about 0.225 cells per frame every frame, then holds about 0.93 cells per frame. 46 cells take about 28 frames, driven by the roll animation on the client.
  - **Walls:** the roll does not pathfind around them. It goes straight and slides along a wall (the clearance gradient gives the wall's direction), and stops when blocked.

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
- **Position sync (dev bridge until M4).** The server's pathfinding and collision only approximate the client's, so positions drifted near walls (up to about 20 cells after a roll along a wall), and the client snapped back on the next server position. The client DLL now reports the player's position every frame (`position_sync`, see [client-dll.md](../client-dll.md#position-sync)). The instance server (`servers/instance-server/src/sync.rs`) listens for these reports on its own port (UDP, loopback only) and takes the client's position while reports are fresh (250 ms). It also uses the reported action to know when the client's roll ended. Without reports (option `[instance] position_sync = false`, or a non-local client) it falls back to its own simulation.

## Sprint
- Holding Space after a dodge roll makes the character sprint (`sprint_movement_speed_+%` = 50 in the character templates, after `dodge_roll_sprint_minimum_time_ms`).
- **C→S `0x2e6` SprintInput**: `u8 local player, u8 held` (1 on press, 0 on release). **S→C `0x2e7` SprintState**: header + `u8 held`. The server echoes it, which sets Actor +0x2a2 bit 5 (key held) on the client.
- The client decides at the end of a roll whether it carries on as a sprint (`0x141b76c20`). Its "is sprinting" check (`0x141e5d490`) reads Actor +0x915 or the stat `is_sprinting`.
- **Status: partial.** With the echo alone, the client starts a short sprint after the roll and falls back to walking. Setting `is_sprinting` and the speed bonus as stats (**S→C `0x1d3` StatsUpdate**: header, `u8 set, varuint n, n × (varuint key, varint value)`; **S→C `0x1d8` SkillStatsUpdate**: header, `u32 skill, u8 container, varuint n, pairs`) makes the sprint run at the right speed, but the client then drops its Move action right after the roll (`0x141e4de20` stops Move when Actor +0x2a1 bit 2 is set), and restarting it from the server only stutters. Sprinting on the official servers is most likely a buff with these stats. Full sprint therefore waits for the buff system (M7). The messages are decoded and kept, and the server simulates a sprint only when no client reports arrive.

## Accuracy (until M4)
The server walks A* paths on the client-exported grid (layer 0: a 0–5 clearance field, 0 = wall). In the open, the server's end cells equal the client's: clicks to within a cell, rolls to within a cell. Near walls the client's pathfinder keeps a little more distance and slides differently, so positions can drift by a few cells (up to about 20 after a roll along a wall). The client then snaps to the server's position when the next input is answered. An exact port of the client's pathfinding and collision belongs with the server-side terrain (M4).

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
- **`inclick:x,y` / `inmove:x,y`:** override `GetCursorPos` and post mouse messages.
- **Posted keys (`0x20`):** reach the input bindings that are read from window messages, such as Space for dodge roll.
- Nothing is sent to other windows. `SendInput` is not used, because it goes to whatever window has focus.
- The DLL's `dump_object_types` logs the player's world position at every simulation frame (`frame #n … step … grid`) when `movement_log` is on (or `POE_HOOK_MOVEMENT_LOG` is set), and the WASD direction (Actor +0x400) and action slots when they change.
