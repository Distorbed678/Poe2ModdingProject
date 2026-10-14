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
- Pressing **Space** (`use_dodge_roll`) sends **`0x1a5` UseSkill** with the dodge roll's skill id and the cursor position (relative). It also sends `0x1ac` (`u8 local player`) and `0x2e6` (`u8 local player, u8`: 1 on press, 0 on release).
- **Skill stats come from the server.** The client reads a skill's stats from the stat list in its Actor entry, not from the tables, and it reads the *computed* stats.
  - The roll's length is `dodge_roll_travel_distance` (`0x141aa90d0`: stat key `0x57fa` × (1 + `movement_velocity_+permyriad`)).
  - Without it the client logs "Trying to do pathfinding action movement with base travel distance <= 0" (`0x141b8be20`) and does not roll.
  - The server sends the stat set's constant stats and its implicit stats (as 1). There is no stat calculator yet (M7), so each `..._base_...` stat is also sent under its name without `base_` (`dodge_roll_base_travel_distance` 46 → `dodge_roll_travel_distance` 46).
- The server answers with StartAction for that skill and simulates the roll (`Motion::Roll`):
  - **Length:** 46 cells toward the target.
  - **Speed profile** (measured): it starts at about 3.9 cells per 33 ms frame, loses about 0.225 cells per frame every frame, then holds about 0.93 cells per frame. 46 cells take about 28 frames, driven by the roll animation on the client.
  - **Walls:** the roll does not pathfind around them. It goes straight and slides along a wall (the clearance gradient gives the wall's direction), and stops when blocked.

## Accuracy (until M4)
The server walks A* paths on the client-exported grid (layer 0: a 0–5 clearance field, 0 = wall). In the open, the server's end cells equal the client's: clicks to within a cell, rolls to within a cell. Near walls the client's pathfinder keeps a little more distance and slides differently, so positions can drift by a few cells (up to about 20 after a roll along a wall). The client then snaps to the server's position when the next input is answered. An exact port of the client's pathfinding and collision belongs with the server-side terrain (M4).

## Pause
`0x1ff` PauseRequest also pauses the server simulation (`Player::paused`). Nothing moves on either side while the escape menu is open in a solo game.

## Test tooling
- **`poe-hook.keys` entries `in:<vk>[:ms]`:** a synthetic Raw Input key (`WM_INPUT`, answered by the hooked `GetRawInputData`, with `GetKeyState` reporting the key held). WASD is read this way.
- **`inclick:x,y` / `inmove:x,y`:** override `GetCursorPos` and post mouse messages.
- **Posted keys (`0x20`):** reach the input bindings that are read from window messages, such as Space for dodge roll.
- Nothing is sent to other windows. `SendInput` is not used, because it goes to whatever window has focus.
- The DLL's `dump_object_types` logs the player's world position at every simulation frame (`frame #n … step … grid`) and the WASD direction (Actor +0x400) when it changes.
