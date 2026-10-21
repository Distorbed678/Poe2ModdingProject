# Movement

Specification of how the client moves the player and what an **official** server sends back, from seven read-only official capture sessions (`dumps/0.5.5e/captures/official/`, 2026-10-14..20; the last one, `2026-10-20_18-41-14_336`, targeted the gaps and has the client's per-frame positions) and the client's code (Ghidra). The tool that produced every sequence below is `scripts/re/movement_sequences.py` (`list`, `events`, `scen`, `never`; see [Reproducing](#reproducing)). Code: `crates/poe-game/src/locomotion.rs` (the movement component built from this spec, [instance-world.md](../architecture/instance-world.md)), `world.rs`, `movement.rs` and `pathfinding.rs` (the simulation), `servers/instance-server/src/{movement,player}.rs` (the packet glue, [below](#server-implementation)), messages in `modules/protocol/poe-proto/src/instance.rs` and `skill.rs`.

Six sessions are **WASD input mode** only. The 2026-10-20 session has WASD mode (f1-3286, f4663-end), **click-to-move mode** (f3287-4662), standing and chained rolls, sprints into walls, an input-mode toggle and per-frame client positions (`poe-hook.log`, "frame #n ... world ... grid"; its frame numbers are the capture's frame numbers). What is still missing is in the [gap list](#gap-list); the session's frame index is in [Session 2026-10-20](#session-2026-10-20-index).

Conventions:
- Frame numbers count the server's `0x02f4` ticks received by the client. A **batch** is the run of simulation messages that arrives before a tick marker. `f` = frame the client sent an input in (ticks received so far); a reply "at `f+1`" arrives after one more tick. Round trip on the official servers: median 50 ms, p90 62 ms, so **the reply is at `f+1` for ~93 %** of inputs (`f+0` ~6 %, `f+2` <1 %). What the server controls is only "first tick after the input arrived, all replies of one input in the same batch" (`r+0`); `r+k` means k batches later.
- Cells are grid cells (23 per tile). Position fields in replies are the **server's** player cell at that tick.
- The header of a simulation message is `(entity, u14, u18, u1a)` ([instance.md, Frame ordering](instance.md#frame-ordering)). Official replies to the local player use `(player, 0, 0, 0)` or an ordering value in `u1a` (see the sequences); sending zeros is valid.

## Client facts

### Lockstep: the server starts every movement
- The in-game world is created in frame mode 2 (world +0x661, constructor `0x14212eb60` called from `0x14100b7e9` with a constant 2): **lockstep**.
- The client's `[NETWORKING] networking_type` option (`auto`/`lockstep`/`predictive`, read by `0x141939e40`) is a PoE1 leftover. With `auto`, the session update (`0x142120b70`) only switches a world whose +0x661 is 3. Below 70 ms latency it picks 2, above 90 ms it picks 0.
- So pressing W, clicking or pressing Space sends a request and nothing moves until the server answers. The skill senders (`0x14210f920`, `0x14210fd40`, `0x14210fdc0`, `0x14210fbb0`) skip their local path when `world +0x661 == 2`, and the Player component handlers for the replies (`0x1ae`..`0x1b2`) run only outside predictive mode ([addresses](../re/addresses-0.5.5e.md#skill-acknowledgements-buffs-sprint-wp21-2026-10-20)).
- The answer starts the movement on the client, which then moves the entity itself every simulation frame (`dt = frame_ms / 1000`, see [instance.md](instance.md)), along paths from its own pathfinder, until the movement ends. The server simulates the same movement to know where the player is.
- Messages that carry a position (`0x64`, `0x1c8`) make the client snap to the server's position only when its own is **46 cells or more** away (squared distance ≥ `0x844`: `0x142149640` for `0x64`, `0x141e41ae0` for `0x1c8`; the snap is `0x141ddb570` SetLocationForcedNoCallbacks). Closer than that, `0x64` only bends the walk direction, and `0x1c8` keeps the client's position. In lockstep (world vfunc +0x1d8 = 1 is the *other* mode) a `0x64` whose cell differs from the client's is a forced placement, **but only when its direction is not zero**; a `(0,0)` update ignores its cell (verified 2026-10-14 with the DLL's teleport command).
- **Frame playback** (`0x1421302c0`): the client plays received frames in real time with a speed factor (world +0x670): 1.01× with one spare frame queued, 1.5× with 2–29, 2× with 30–59, 5× with 60 or more, back to 1.0× once the queue is empty. It never runs past the last received frame, so late frames freeze movement and bursts make it rush. The server sends one frame marker per tick from a steady timer, with `TCP_NODELAY` set.

### Speeds
- Walking: the Pathfinding component (constructor `0x141dcd020`) stores `Pathfinding.base_speed` at +0x550 and a multiplier of 1.0 at +0x55c; the getter `0x141dcce20` returns `base_speed × (1 + movement_velocity_+permyriad/10000) × (1 + virtual_action_speed_+%/100) × multiplier`. Units are **cells per second**, characters have `base_speed = 37` (**1.23 cells per 33 ms frame**, measured 37.0 cells/s).
- Sprinting: the official stats `base_move_speed_+%_from_sprint = 50` and `movement_velocity_+permyriad = 5000` give 1.5×: measured mean step **1.83..1.89 cells per frame** in the sprint streams.
- Dodge roll: `dodge_roll_travel_distance` (`0x141aa90d0`, stat `0x57fa`) = 46 cells. Measured on the client in 30 official rolls (2026-10-20): 45.2..46.3 cells along the path in 29-31 frames, see [Roll path and speed profile](#roll-path-and-speed-profile).

## Messages

### Client → server
| Op | Name | Body |
|---|---|---|
| `0x63` | MoveDirection | `u8 1, varint x, varint y, u8[3]`. Sent when the held direction changes. Vectors of length ~180 in grid axes: `(127,127)`, `(180,0)`, `(-127,-127)` …, `(0,0)` = stop. **Arbitrary angles are the cursor direction**: with Space held and no WASD key (a sprint, also standing or after the keys are released) the client sends the direction to the cursor every frame it changes (`(94,-153)`, `(-163,76)` …); the third trailer byte is `1` in click-to-move mode, `0` in WASD mode. `(0,0)` is never sent while a roll or sprint runs (0 of 380). |
| `0x62` | AimTarget | `u8 player, u8 mode`; mode 2: `i32 x, i32 y` **absolute cell**; mode 3: `u32 entity`; modes 0/1: nothing (once at entry). About one mode-2 per frame while the cursor moves, only in combat stance. |
| `0x1a5` | UseSkill | `u8 player`, skill block: `u32 skill, u16 flags, u8 0xff, u8 target flags, [u32 entity if flags & 8], i32 x, i32 y` (x, y **relative to the player's cell**, absolute with a target entity). |
| `0x1a8` | UpdateSkillTarget | `u8 player, i32 x, i32 y (relative), u32 skill`. About every 70 ms (2 frames) while a roll or sprint runs (key held or not) and while a held skill runs. |
| `0x1aa` | UpdateSkillTargetEntity | `u8 player, u32 entity, u32 skill`. |
| `0x1ac` | SkillInputEnd | `u8 player`. Held skill input ended: Space released (always together with `0x2e6` held=0), attack button released. |
| `0x1ad` | SkillReleased | `u8 player, u32 skill, u8 0`. The release of the Move skill (a click-to-move press: 2 frames after the press for a click, at the end of a clean hold; 5 of 12 holds ended without one) and, **in click-to-move mode, of the roll key** (instead of `0x1ac`, sent with `0x2e6 held=0`). |
| `0x2e6` | SprintInput | `u8 player, u8 held`. Space down / up. |
| `0x200` | StanceRequest | `u8 player, u8 value`. Input mode toggle (mouse and keyboard <-> WASD), together with `0x62` mode 0 (value 1) or mode 1 (value 4) in the same frame. Answered by a stance delta, see [Input mode and stance](#input-mode-and-stance). |

Skill ids (official server, from the skill lists and `0x1d8` stats; basic actions are `0x20000000 | ActionTypes key`):

| Id | Skill |
|---|---|
| `0x20002909` | Move |
| `0x80000040` | **Dodge roll** (stats: `deal_no_damage`, `base_deal_no_damage`, …) |
| `0x80400040` | main attack (damage stats), `target flags` 0x08 on an entity |
| `0x2000c266` | basic action used with a target entity (interaction) |
| `0x2000b7d9` | the intro's wake-up action (reply to the first click) |
| `0x40000005`/`6` | `0x1d8` stat containers of other entities (`attack_time_ms_for_stat_scaling_effects`) |

Target flags seen on `0x1a5` (bit meaning from the client's use): `0x00` plain press, `0x08` target entity, `0x10` held repeat, `0x01`/`0x02` extra input bits (`0x13`, `0x1b` on held repeats). The server's ack echoes `flags` and `u8` and sets `0x04`.

### Server → client (all simulation messages with the 11-byte header)
| Op | Name | Body after the header |
|---|---|---|
| `0x64` | MoveDirectionUpdate | `varint dir x, varint dir y, varint cell x, varint cell y, bool`. Direction: the `0x63` vector verbatim while walking, **scaled to length 550 and truncated toward zero while sprinting** (`v × 550 / |v|`: `(0,-550)`, `(388,-388)`, `(94,-153)` → `(287,-468)`; matches 294 of 309 replies, the others answer the previous input; `×550/180` fits only 72). The cell is the server's player cell. The bool is `0xf1` (true) in 94 % of the replies, `0` otherwise (every 31st frame of a sprint stream, otherwise sporadic); the client reads it as a bool and ignores it. |
| `0x1c8` | StartAction | `u16 flags (0x80 = entity follows), [u32 entity], varint cell x/y (start), varint target x/y (absolute), u32 skill, u16 action id, u8 0xff`. **Flags = the `0x1a5` flags | `0x10` for a start an input caused** (roll/attack `0x410`, `0x450` in click-to-move mode, Move press `0x410`, held repeat `0x10`); a restart the server issues itself (held click every ~15 frames, walk resumed after a roll) carries the last request's flags without `0x10`, header `(player, 9, 1)`/`(0, 8)`. Action ids are one **server-wide counter** (32768+, e.g. 32872 → 32875 → 32877 across all entities). |
| `0x1ae` | UseSkillAck | the skill block with **absolute** x/y and `target flags | 0x04` ([skill.rs](../../modules/protocol/poe-proto/src/skill.rs)). |
| `0x1af` | UpdateSkillTargetAck | `i32 x, i32 y, u32 skill`: absolute target = server player cell + the relative offset of the `0x1a8`. |
| `0x1b0` | UpdateSkillTargetEntityAck | `u32 entity, i32 x, i32 y (the entity's cell), u32 skill`. |
| `0x1b1` | SkillInputEndAck | no body. |
| `0x1b2` | SkillReleasedAck | `u32 skill, u8` (header `u1a` = 6). A release of the roll key (`0x1ad` roll skill) is answered by `0x1b1` **and** `0x1b2`, then `0x2e7`. |
| `0x2e7` | SprintState | `u8 held`. Echo of `0x2e6` (sets Actor +0x2a2 bit 5). |
| `0x1d3` | StatsUpdate | `u8 set, varuint n, n × (varuint key, varint value)`. |
| `0x1eb` / `0x1ec` | BuffAdded / BuffRemoved | `u16 index, u16 buff row, …` / `u16 index` (see [skill.rs](../../modules/protocol/poe-proto/src/skill.rs)). |
| `0x1ca` | (action event) | no body; with every roll start and when an attack ends a sprint. Actor case `0x14214a13e` decrements a counter (+0x348). |
| `0x1dc` | SetAimTarget | `u8 kind (0 cell, 1 entity), i32 x, i32 y`, header `(player, 0, 0, 0)`. |
| `0x2f9` | ComponentDelta | player stance and flags (component 6). |
| `0x2a6` | (orientation lerp) | read by the `0x1df` reader: `f32, u8` after the header; `0.05` at entry and when the stance goes to combat, `0.2` when it goes to 0; always with a stance delta. |

Layout confirmations in Ghidra (2026-10-20): `0x1ae` reader `0x142186800` = sim header + `SkillUseParams_Read` (`0x141ecde00`), `0x1af` `0x141f6d520` (3 × u32), `0x1b0` `0x1421868e0` (4 × u32), `0x1b1` header only, `0x1ac` sender `0x14210fd40` (one `u8`), `0x1b2` (`u32, u8`). All of them decode every captured instance with zero leftover bytes ([Reproducing](#reproducing)).

### Request/reply pairs
Every client skill message has exactly one reply; counts are equal in all six sessions (1a5 : 1ae, 1a8 : 1af, 1ac : 1b1, 1aa : 1b0, 1ad : 1b2, 2e6 : 2e7).

| C→S | S→C | reply frame offset (all sessions) |
|---|---|---|
| `0x63` | `0x64` (same vector, scaled when sprinting) | `f+1` 93 %, `f+0` 6 % |
| `0x1a5` | `0x1ae` | `f+1` 510, `f+0` 33, `f+2` 3 |
| `0x1a8` | `0x1af` | `f+1` 516, `f+0` 34, `f+2` 3 |
| `0x1ac` | `0x1b1` | `f+1` 481, `f+0` 36, `f+2` 2 |
| `0x2e6` | `0x2e7` | `f+1` 29, `f+0` 1 |
| `0x1ad` | `0x1b2` | `f+1` 2, `f+2` 2 |
| `0x62` mode 2 | `0x1dc` (not always) | `f+1` ~94 % |
| `0x1a5` Move (press / held repeat) | `0x1c8` Move + `0x1ae` | same batch as the `0x1ae` (`f+1` 413 of 440 `0x1a5` in the 2026-10-20 session, `f+0` 23, `f+2` 4) |
| `0x1ad` | `0x1b2` (and `0x1b1` for the roll key) | `f+1` 12 of 13 |
| `0x200` + `0x62` mode 0/1 | `0x2a6` + `0x2f9` | `f+1` 2 of 2 |

### Opcodes official servers never send for movement
Over 6 sessions (~35 000 simulation messages): **`0x1cb` UpdateActionTarget: 0, `0x1df` SetOrientation: 0** (confirmed). Steering is done with `0x1af`, facing with `0x1dc`. (Our server uses `0x1cb`, `0x1df` and a `0x64` toward the cursor for held clicks; none of that exists officially.) Also absent: any server `0x63`-like position-only message, `0x65`, `0x1a9` (absolute target update).

## State machine

States of the local player's movement on the server: `Idle`, `Walking(v)`, `Rolling`, `Sprinting(v)`. `v` is the last `0x63` vector (its `(0,0)` is `Idle`). The `r+k` columns are batches relative to the first reply batch `r+0` (the first tick after the input arrived). `t0` is the roll's start batch.

| From | Event (C→S) | To | Reply |
|---|---|---|---|
| Idle / Walking | `0x63 v≠0` | Walking(v) | `r+0`: `0x64 v, cell` |
| Walking(v) | `0x63 v'≠0` | Walking(v') | `r+0`: `0x64 v', cell` (also while Rolling) |
| Walking(v) | `0x63 (0,0)` | Idle | `r+0`: `0x64 (0,0), cell` |
| Idle / Walking | `0x2e6 held=1` + `0x1a5 roll` | Rolling | `r+0`: roll batch (below) |
| Rolling | `0x1a8` (every 2 frames) | Rolling | `r+0`: `0x1af`, once per `0x1a8` |
| Rolling | `0x63 v` | Rolling | `r+0`: `0x64 v, cell`. A roll that runs along the WASD keys **turns with the new vector** (2 frames after the `0x63`, [roll direction](#roll-direction)); walking resumes when the roll ends |
| Rolling, key released up to `t0+14` | `0x1ac` + `0x2e6 held=0` (click-to-move mode: `0x1ad` roll + `0x2e6`) | Rolling → Walking/Idle at `t0+30` (see [roll action](#roll-action-length)) | `r+0`: `0x1b1` (+`0x1b2`), `0x2e7 held=0`; `t0+16`: roll-end stats |
| Rolling, key still down at `t0+15` (released from `t0+19` on: sprint anyway) | (nothing) | Sprinting | `t0+15`: sprint start; `t0+30`: roll end; `t0+35`: `must_sprint=0` (and the sprint end, if the key went up before) |
| Rolling | `0x1a5` roll, 0.3-0.9 s after the previous start | Rolling | `r+0`: `0x2e7`, `0x1ae`; the new roll starts at `t0+30` ([chained rolls](#roll-action-length)) |
| Rolling | `0x1a5` attack | attack at `t0+24` / `t0+35` | `r+0`: `0x1ae` only; see [roll action](#roll-action-length) |
| Idle / Walking | `0x1a5` Move press (click) | Walking(target) | `r+0`: `0x1c8` Move (flags `0x410`), `0x1ae`; release `0x1ad` → `0x1b2` |
| Walking(target), click held | `0x1a5` Move, target flags `0x10`, every 2 frames | Walking(new target) | `r+0`: `0x1c8` Move (flags `0x10`), `0x1ae`; every ~15 frames a server `0x1c8` restart |
| Sprinting | `0x63 v` | Sprinting(v) | from the first `0x63` after sprint start: a `0x64` **every frame**, direction scaled to length 550 (no `0x63` after the sprint start: no stream, the client walks on by itself) |
| Sprinting | `0x1ac` + `0x2e6 held=0` | Walking(v) | `r+0`: `0x1b1`, `0x2e7 held=0`, `0x64`, sprint-off buffs/stats; `r+1`: `0x64 v` unscaled |
| Sprinting | the player is blocked by a wall | Walking, blocked | same frame as the stall: sprint-off (first half), `r+1`: `0x64` unscaled, `+7..8`: second half ([end of sprint](#end-of-sprint)) |
| Sprinting | `0x1a5` attack | Attack | sprint-off stats + `0x1c8` attack; `0x2e7 held=0` follows when the key goes up |

### Roll batch (`r+0`), identical in all 45 rolls
Order of the reply to `0x2e6 held=1` + `0x1a5 {roll}` (client sends both in the same frame, `0x2e6` first):

1. `0x2e7` SprintState `held=1`
2. `0x1c8` StartAction: `skill 0x80000040`, flags `0x410`, start cell = server player cell, target = cell + the relative cursor offset, `action id`, `0xff`; header `(player,0,0,0)`
3. `0x1ca` header only, `u1a=9`
4. `0x1d3` `is_dodge_rolling=1`, `u1a=9`
5. `0x1ae` UseSkillAck, `u1a=10`, target flags `0x04`, absolute cell = start cell + offset
6. `0x1d3` `stun_threshold_+%_final_while_performing_action=100, virtual_heavy_stun_threshold_+%_final=100, virtual_light_stun_threshold_+%_final=100`, header `(player,player,5,6)`

If an earlier `0x1a8` is still unanswered it is answered first (`0x1af` before this batch's messages). The batch is the same **standing** (no WASD key; 24 of the 30 rolls of the 2026-10-20 session, 16 of them taps) and in click-to-move mode (`0x1a5` flags `0x440`, `0x1c8` flags `0x450`).

**Deferred start.** Three cases have `0x2e7`/`0x1ae` in `r+0` and the `0x1c8`, `0x1ca` and stats in a later batch with scheduled headers `(player,player,5,8)`, `(…,5,17)`, `(…,5,25)` (`0x1c8` `u1a` 8): (a) the press and its release arrive in one batch (inputs queued while the game was paused: `0x2e6` 1, `0x1a5`, `0x1ac`, `0x2e6` 0 in one frame; then `r+0` is `0x2e7` 1, `0x1ae`, `0x1b1`, `0x2e7` 0 and the roll starts at `r+1`); (b)/(c) a roll asked for while the previous roll's action still runs ([chained rolls](#roll-action-length)).

### Roll end and sprint (timeline in frames after the roll's `0x1c8`, `t0`)
Measured in all 45 rolls; the offsets are constant (server logic, not latency). `k` below is the client frame the key-up was sent in (a reply is in `k+1`):

| Frame | Tap (key released up to `t0+14`; 24 rolls) | Hold (key still down at `t0+15`; 21 sprints) |
|---|---|---|
| `t0+0` | roll batch | roll batch |
| key release (`k`) | `0x1b1`, `0x2e7 held=0` | (see end of sprint) |
| `t0+15` | – | `0x1eb` ×3 (indices n, n+1, n+2; buff rows 2758 `0xac6`, 2737 `0xab1`, 2853 `0xb25`, flags 0) and `0x1d3` `sprint_blend_time_override_ms=1, is_sprinting=1, must_sprint=1, all_hits_stun_you=1, virtual_skill_ignore_namelocking_dynamically=1`, header `(player,player,5,2)` |
| `t0+16` | `0x1d3` stun thresholds 0 + `is_dodge_rolling=0` (roll over) | – |
| `t0+30` | – | `0x1d3` stun thresholds 0 + `is_dodge_rolling=0`; `0x1d3` `base_move_speed_+%_from_sprint=50, movement_velocity_+permyriad=5000`; `0x1eb` ×2 (buff index n+1 `0xab1` again and n `0xac6` with flags `0x02` = f32 `0.1`) |
| `t0+35` | – | `0x1ec` buff index n; `0x1d3` `must_sprint=0`, header `(player,player,3,0)` |

So **`is_dodge_rolling` is cleared 16 frames (533 ms) after the roll on a tap and 30 frames (1 s) when Space is still down at frame 15**, and the sprint begins at frame 15 while the roll is still running.

**The boundary** (30 rolls of the 2026-10-20 session, release frame relative to `t0`): releases at `t0+2..t0+14` (the longest taps, 14 frames = 500-549 ms) never sprint; releases from `t0+19` on (667 ms and longer) always sprint (`t0+15` stats, roll stats at `t0+30`). `t0+15..t0+18` was not hit ([gap list](#gap-list)); the server code decides by "key still held when the `t0+15` batch is built" (a release sent in client frame `t0+14` arrives for that batch). **A release between `t0+15` and `t0+34` does not end the sprint** (seen: key up at `t0+19`, click-to-move mode): the release is answered at once (`0x1b1`, `0x1b2`, `0x2e7 held=0`), the `0x64` stream goes on scaled, and at `t0+35` the batch holds `0x1ec n`, `must_sprint=0`, the `0x64`, then (held click: a `0x1c8` Move, flags 0) the sprint-off events `0x1ec n+1`, sprint stats 0, `0x1ec n+2`, `all_hits_stun_you=0` (headers `(9,10)`). So the sprint is over at `t0+35` at the earliest.

### Roll action length
The roll *action* lasts **30 frames in both cases** (client log: `action start/end slot 0: DodgeRoll` exactly 30 frames apart in 23 of 30 rolls; 24 when an attack cut it (3), 31 (2), 57 across a pause (1)). After a tap roll's stats at `t0+16` the server still treats the player as rolling until `t0+30`:
- **Aim is not echoed** (`0x62` → `0x1dc`) for aims up to `t0+26` at least; the `0x1a8`/`0x1af` pairs go on.
- **A new roll waits** (2026-10-20: 4 chained cases, plus the 2026-10-14 case). Inputs `g` frames after the previous start `P`: `g=9` (key released again at `P+14`): acked (`0x2e7`, `0x1ae` in `r+0`), **no start at all**; `g=27/28`: acked at `r+0`, started at `P+30` with the scheduled headers; `g=29` (the previous start itself was late): started at `r+1 = P+31`; `g=31..34`: started at once (`r+0`, normal headers). Rule that fits all of them: a roll asked for while `r+0 <= P+30` starts at `max(r+0+1, P+30)`; whether the dropped `g=9` press was dropped because its key was already up when the lock ended or because the later press (`g=28`, 19 frames after it) replaced it is not decided (single slot, last wins: both fit).
- **An attack waits for the roll** (3 cases, all tap rolls): `0x1ae` at `r+0` for every attack press (repeats too, `0x1ae` target flags `0x14`/`0x0c`), no `0x1c8`. The **last** attack's `0x1c8` (target = that press' cursor cell, `0x1ca` `u1a` 7) comes at **`t0+24`**, the roll is cut there (the client's DodgeRoll action ends at `t0+24`, the stats had gone at `t0+16`). If Space is still held at `t0+15` the roll is a sprint and the attack starts with the sprint's end at `t0+35` (1 case): `0x1ec n`, `must_sprint=0`, the scaled `0x64`, the attack `0x1c8` (flags `0x10` when the last press was a held repeat), `0x1ec n+1`, sprint-off stats, `0x1ca`, `0x1ec n+2`, `all_hits_stun_you=0`, no second `0x1ae`. The `0x1c8` target of an attack on a hovered monster (target flags `0x08`) is the monster's current cell, not the press' coordinates (505,963 vs 454,969).
- **Click-to-move repeats** (`0x1a5` Move, target flags `0x10`) during a roll or sprint are acked (`0x1ae`) and nothing starts; the held click's walk resumes at the end of the roll with a server `0x1c8` Move (flags = last request's, header `(0,8)`) towards the latest target (`t0+30` in the one case).
- Our roll covers its 46 cells in the same number of frames, see [Roll path and speed profile](#roll-path-and-speed-profile).

### Roll direction
**The client rolls along the WASD vector when a movement key is held, and towards the cursor when none is** (30 rolls, client positions over `t0+1..t0+4` vs the key vector and the `0x1c8` target): 5 of the 6 rolls with a key held went along the key vector (within 3 degrees) although the `0x1c8` target pointed 16, 38, 38 and 156 degrees elsewhere in four of them (the sixth was against a wall); the 24 standing rolls went towards the cursor (within 4-12 degrees in the first frames, then steered every 2 frames). The same holds for the roll that our own server mis-simulated: 2 of 2 rolls along the keys. The `0x1c8` target is the cursor either way (`cell + offset of the 0x1a5`) and the client ignores it when a key is held. Details:
- **Held keys**: the direction is the key vector at any time of the roll: `0x63` sent in the roll (client frame `t0+11`, `(127,-127)` → `(180,0)`) is answered at once with a walking `0x64` and the roll **turns 2 frames after the `0x63`** (`t0+13`; the `0x64` is `f+1`, the step with the new direction `f+2`), and turns back after the next one (`t0+26` → `t0+28`). The roll distance stays 46 cells.
- **Standing**: the first 3 frames go in about the client's facing/cursor direction (within 5-15 degrees of the target), from the first `0x1af` (sent at `t0+3`, applied at `t0+4`) on the roll follows the `0x1a8` offsets (`0x1af` = server cell + offset): angle changes in the log track the offsets frame by frame.
- **A key pressed during a roll that started standing**, and a key released during a roll along the keys, were not captured.
- **Blocked**: a roll along a wall slides (SW roll against a wall at 109 degrees instead of 135).

### Roll path and speed profile
Client steps per frame from `poe-hook.log` (cells per 33 ms frame, 30 rolls, `dt` 0.033 in every frame; the 6 standing rolls on open ground, mean; the first step is in frame `t0+1`, one frame after the `0x1c8`):

| `t0+` | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| step | 3.89 | 3.74 | 3.45 | 3.27 | 3.02 | 2.72 | 2.54 | 2.30 | 1.98 | 1.76 | 1.59 | 1.35 | 1.23 | 1.16 | 1.06 |

| `t0+` | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | 25 | 26 | 27 | 28 | 29 | 30 | 31 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| step | 0.99 | 0.97 | 0.94 | 0.94 | 0.94 | 0.94 | 0.95 | 0.93 | 0.85 | 0.74 | 0.68 | 0.49 | 0.31 | 0.16 | 0.04 | 0 |

- It falls about **0.215 per frame from 3.9 to 1.2 (t0+13), levels out at 0.94 until `t0+23` and tapers to zero by `t0+31`**. Sum 45.9: the roll covers `dodge_roll_travel_distance` (46 cells) in 30 frames (36 cells in the first 16, 34 in 14); straight-line displacement 42.6-46.2 (the cursor steering bends the path).
- Walking resumes at `t0+30` (tap) with the walking speed 1.23 (WASD) or stands (no key): the standing roll ends **without any `0x64`**; the next message is the next input's reply. A held-key roll's `0x64`s are only the replies to `0x63`.
- **Positions agree**: the `0x1c8` start cell is within 0.2-2.9 cells of the client's position before the first roll step, and the `0x64` cells (600 of them, also after rolls) are within 0-2.8 cells of the client's position (median 0): the official server simulates the same roll path and needs no correction. In our own session of the same day (`GameFiles/logs/poe-hook/2026-10-20_19-14-52_336`) a roll along the keys that the server simulated towards the cursor made the client jump 36 cells at the next roll's `0x1c8` (the roll start places the client on its start cell), and the drift never recovered.
- `crates/poe-game/src/movement.rs` plays the table frame by frame (`ROLL_STEPS`, 30 values, sum 45.9; `roll_speed(t, sprint)`). A blocked step costs no distance and does not end the roll (the client's roll runs its 30 frames against a wall too).
- **A roll that becomes a sprint (key still down at `t0+15`) does not taper**: the 7 sprint rolls of the session step 0.91, 0.96, 1.02, 1.08, 1.09, 1.11, 1.20, 1.34, 1.36, 1.46, 1.49, 1.52, 1.54, 1.70 in `t0+16..t0+29` and about 2.4 in `t0+30` (2.0 to 3.4), then sprint at 1.86 (`ROLL_SPRINT_STEPS`, `Mover::sprint_roll` at the sprint start). The tap table would leave the server about 10 cells behind after a sprint roll.

### Reply details found by replay
- **Direction scaling truncates**: `v × 550 / |v|` toward zero (WP2.1b: not `v × 550 / 180`, which is only equal for lengths of exactly 180: `(94,-153)` → `(287,-468)`, not `(287,-467)`; `(-53, 172)` → `(-161, 525)`, not `-162, 526`).
- **`0x64` comes after the other replies of its frame** (input replies, timeline events); in the key-release case the sprint-off events follow it, in the roll/sprint timeline frames (`t0+30`, `t0+35`) the timeline events come first and the stream message last.
- **Attack ends the sprint** (key release and attack in one frame): `0x1a8` ack, `0x1b1`, `0x2e7 held=0`, attack `0x1c8`, `0x1ec n+1`, sprint-off stats, `0x1ca`, attack `0x1ae`, **`0x64` unscaled** at `r+0`, `0x1ec n+2`, `all_hits_stun_you=0`.

### End of sprint
`r+0` batch: `0x1b1`, `0x2e7 held=0`, `0x64` (still scaled), then `0x1ec` (index n+1), `0x1d3` `base_move_speed_+%_from_sprint=0, movement_velocity_+permyriad=0, sprint_blend_time_override_ms=0, is_sprinting=0, virtual_skill_ignore_namelocking_dynamically=0`, `0x1ec` (index n+2), `0x1d3` `all_hits_stun_you=0` (headers `(player,player,9,7)`); `r+1`: `0x64` with the **unscaled** vector. If the player used an attack (`0x1a5` `0x80400040`) while sprinting, the sprint is ended by it instead: `0x1ec`, `0x1d3 must_sprint=0`, the attack's `0x1c8`, `0x1ec`, `0x1d3` sprint stats 0, `0x1ca`, `0x1ec`, `0x1d3 all_hits_stun_you=0`; the key release (`0x1ac`+`0x2e6 held=0`) then gets `0x1b1`/`0x2e7 held=0` with no stats. Sprint end causes over the 10 sprints: key up 6, an attack skill 4.

**More causes (2026-10-20, 11 sprints: key up 4, attack 4, wall 3, held-click Move 0)**:
- **Wall**: the server stops the sprint in the batch where its own cell stops advancing (sprint into a wall, 3 of 3, Space and the keys still down): `0x1ec n+1` and the sprint-off stats (`base_move_speed...=0` ... headers `(0,4)` or `(9,6)`) in that batch, the **unscaled** `0x64` one frame later (`(173,49)`, `(-127,-127)`; none when no stream had started), and `0x1ec n+2` + `all_hits_stun_you=0` (header `(3,0)`) **7-8 frames after** the first half. The client's step in that frame is the partial one (0.28, 0.86 of 1.86). The sprint does not restart while the key stays down; the key release then gets `0x1b1`, `0x2e7 held=0` and, if WASD was released, the `0x64 (0,0)`.
- **Key up before `t0+35`**: ends at `t0+35`, see [the boundary](#roll-end-and-sprint-timeline-in-frames-after-the-rolls-0x1c8-t0).
- **An attack asked for during the roll**: ends it at `t0+35` ([roll action](#roll-action-length)).

### Sprint stream
From the **reply to the first `0x63` after the sprint started** until the sprint ends, the server sends **one `0x64` per frame** (no frame missing in 10 sprints) with the current direction scaled to length 550 and the server's cell, which advances 1.83..1.89 cells per frame (client: 1.85-1.88). A new `0x63` changes the vector in the next frame's `0x64` (`(127,-127)` → `(388,-388)`, `(0,-180)` → `(0,-550)`; non-axis vectors scale the same way). **Without a `0x63` after the sprint started there is no stream** (WASD held since before the roll: the sprint ran 19 frames, no `0x64` at all; the client walks on by itself). With Space held and **no** key (standing hold, or keys released) the client sends the cursor direction as `0x63` every frame it changes and the stream follows it (standing roll hold: 93 `0x64` in 95 frames, vectors turning from `(287,-468)` through `(548,-33)` to `(528,152)`); releasing all keys while Space stays down sends a cursor vector, not `(0,0)`. The bool of the `0x64` stream is `0` every 31st frame (12 of 12 gaps are exactly 31; the phase is not tied to the stream start).

## Scenario sequences

All numbers are from the official captures; `movement_sequences.py scen --scenario <name>` prints them per session.

### WASD start / turn / stop
- Seven sessions, ~1000 direction inputs: every `0x63` has exactly one `0x64` (at most one input per session, the last of the capture, is unanswered).
- **Start**: `f: 0x63 v` → `f+1: 0x64 v` with the cell where the player is (e.g. `0x63 (-127,-127)` at f856 → `0x64 (-127,-127) cell (284,1141)` at f857). **Turn**: same, new vector. **Stop**: `0x63 (0,0)` → `0x64 (0,0) cell`.
- No other message accompanies WASD. A stance delta (`0x2f9`) is not sent on the first Move in the 2026-10-15/17 sessions (it comes at entry, see [Stance](#input-mode-and-stance)).
- The reply cell moves ~1.23 cells per frame between replies; the client turns exactly when it receives `0x64` (frame `t+1`) and puts the entity on the centre of the cell it is given, so the server sends its own simulated cell (as the official one does) and keeps the simulation exact instead of correcting the client.
- While rolling the server still answers each `0x63` immediately with a walking `0x64` (12 inputs in rolls, 2 more in 2026-10-20), and a roll along the keys turns with it ([roll direction](#roll-direction)).
- A key combination can be a free angle: with Space down and no key the client sends the cursor direction (`(94,-153)`), see [sprint stream](#sprint-stream).

### Click-to-move
Verified on the 2026-10-20 session (click-to-move mode `f3287-4662`): 12 presses (6 single clicks of 2 frames, 6 holds of 0.2-16 s), 356 held repeats. Fixtures `click_single`, `click_held_reaim`.
- **Press** (`0x1a5` Move `0x20002909`, flags `0x400`, target flags 0, `x,y` = cursor relative to the player's cell): in the first batch after the input (`r+0`; `f+1` 413 of 440, `f+0` 23): `0x1c8` StartAction Move, **flags `0x410`**, start = the server's cell, target = cell + offset, header `(player,0,0,0)`, new action id; then `0x1ae` (flags `0x400` echoed, target flags `0x04`, absolute target = the `0x1c8` target; header `u1a` 3, 4 for the first press after a pause or at entry). No `0x64`, no `0x1af`, no stats, no `0x2f9`. The client walks to the target at 1.23 cells per frame from the frame of the `0x1c8` (positions: 1.23-1.24 per frame, a dip to 1.08 about every 11 frames) along its own path.
- **Release** (`0x1ad`, skill, 0): `0x1b2 (skill, 0)` one batch after (`f+1`, 12 of 13). Single clicks: the release follows the press by 1-2 frames and the walk goes on (a click's walk is not stopped by its release).
- **Held click**: while the button is down the client sends `0x1a5` Move with **target flags `0x10`**, flags 0, the current cursor offset, **every 2 frames** (not `0x1a8`: 0 `0x1a8` for Move in 356 repeats). Every repeat gets `0x1c8` Move (**flags `0x10`**, start = the server's cell, target = the new cursor cell, new action id) + `0x1ae` (flags 0, target flags `0x14`, absolute target) in the next batch, so a repeat re-aims the walk by starting a new action. In addition the server sends a `0x1c8` Move **about every 15 frames** (15, 15, 13, 15 in clean stretches) with the latest request's flags without `0x10` (`0x400` right after the press, else 0), header `u18=9, u1a=1`, target = the previous target: the first one 2 frames after the press in hold 1, 6 frames in hold 2 (what triggers them is not known; the walk distance between them is 17-18 cells = 15 frames).
- **Arrival**: no message at all (the client's Move action ends by itself); no walk reached its target in the session (every walk was interrupted by the next click within 17-35 frames).
- **Roll, sprint or stun during a hold**: the repeats are acked (`0x1ae`) and nothing else; the walk resumes with a server `0x1c8` Move at the end of the roll (header `(0,8)`, flags of the last request) or, after a sprint that ended at `t0+35`, in the sprint-off batch ([sprint](#sprint)). Not every repeat after that gets a `0x1c8` (the one right after a sprint's end got only the ack).
- **Holds end without a release**: 5 of the 6 holds had no `0x1ad` (rolls, attacks or stuns in between); the repeats just stop (hold 2: press f3527, last repeat f4002). A server must treat a hold that is quiet for 6 frames as over.
- **WASD while a click walks / click while rolling / click on a wall**: not captured as such (a wall click is not recognisable without the grid).
- The mouse-mode client sends **no `0x62` aims at all** (0 in `f3300-4600`), so no `0x1dc` echoes; roll and sprint messages are as in WASD mode, with `0x1a5` flags `0x440` and `0x1c8` flags `0x450` for the roll.
- The intro click (skill `0x20002909` with `u16 flags` = the player's entity id, every earlier session) is a different thing: `0x1a5` + `0x1ad` in one frame → the four `0x1d8` skill stats containers + `0x1d3` (skill-list bootstrap), `0x1c8` StartAction **skill `0x2000b7d9` (wake-up)**, `0x1ae`, and `0x1b2` in the next batch.

### Dodge roll
Rolls in the captures: 45 (15 started while a WASD direction was held; in 2026-10-20: 24 standing, 6 with a key held).
- **Moving or standing**: the batch above; standing rolls are identical and end with no `0x64` ([path](#roll-path-and-speed-profile)). `0x1c8.target = cell + offset of the cursor` (e.g. `(371,1052) + (15,-35) = (386,1017)`), not clamped to 46 cells (offsets of 9..120 cells seen; the client rolls its fixed 46 cells). The direction the client rolls in is in [roll direction](#roll-direction).
- **Steering**: every `0x1a8` (every 2 frames, key held or not, until the roll ends) → one `0x1af` with `x,y = server cell + offset`. Example (sprint): `0x1a8 (23,-27)` → `0x1af (430,1036)` with the server cell `(407,1063)`. A roll along the keys ignores it.
- **Chained** ([roll action](#roll-action-length)): a roll asked for during the previous one is acked at once and starts at `P+30`; presses 31+ frames after `P` start at once; a press 9 frames after `P` that is released before `P+14` does nothing.
- **Standing hold** (4.1 s, f1132): roll, sprint at `t0+15`, the client turns to the cursor and sends its direction each frame, stream, wall at `t0+110`, Space up `t0+124`.
- Our server sends exactly this batch ([implementation](#server-implementation)); it no longer uses `0x1cb` steering.

### Sprint
Hold Space after the roll: the sequences above (`t0+15`, `t0+30`, `t0+35`, stream, end). `0x2e6` is sent once (on down) and once (on up); the client repeats `0x1a5` with target flags `0x13` and flags 0 only in the 2026-10-14 22:23 session (7 repeats; none in the 2026-10-17 session), each acked with `0x1ae` flags 0, target flags `0x17`.
- Sprint with held Space also continues the `0x1a8`/`0x1af` pairs every 2 frames.
- Space re-pressed while sprinting / roll key re-pressed within 0.2 s: **not captured** (the only press during a roll or sprint is the chained one above).
- In click-to-move mode a sprint hold sends cursor-direction `0x63` every frame (trailer byte 1) and the held-click repeats go on (acked); the `0x64` stream is as in WASD mode.
- `0x62` aims are not echoed while sprinting (116/116 not echoed) or rolling (150/150).

### Input mode and stance
`0x2f9` ComponentDelta, component 6 (Player), mask `0x0400` = stance byte, `0x1000` = a second byte that echoes a request value. Stance changes in the captures:
- **At entry**: 2026-10-15/17 sessions: `f10: mask 0x0400, stance=1` (9 frames after the client's first `0x62` mode 1); 2026-10-14 sessions: `f10: mask 0x0402` (flag words, stance=1), `f11: mask 0x1400 (stance=0, +0x3e7=1)`; 2026-10-20: `f15: mask 0x0400, stance=1` (14 frames after the `0x62` mode 1 at f1) and a `0x2a6` with `0.05`.
- **Input-mode toggle** (2026-10-20, 2 of 2, both standing; fixtures `stance_to_mouse_mode`, `stance_to_combat_mode`): the client sends **`0x200 {player, value}` and `0x62` mode 0 or 1 in the same frame** (WASD → click-to-move: value 1, mode 0; back: value 4, mode 1) and the server answers in the next batch (`f+1`, which is `r+0`) with `0x2a6` (`f32` 0.2 for value 1, 0.05 for value 4) and a `0x2f9` of mask `0x1400`: **stance byte 0 / 1** and the second byte = the value (`00 01`, `01 04`). The client sends `0x62` aims only in combat stance and WASD mode (none in `f3300-4600`).
- A change while walking, rolling or sprinting, and the weapon swap, were **not captured**. Candidates seen but unidentified: `C>S 0xfe {1, 1|0}` at f2573/f2620 (47 frames apart, standing) answered by `S>C 0x1d5` + `0xff`.
- Other Player deltas: `mask 0x0402/0x0002` flag words after the first skill release and new areas, `mask 0x0201` experience after kills, `mask 0x2000 (00)` on level-ups.

### Aim
- C→S `0x62` mode 2 with the **absolute** cell under the cursor, at most one per frame (1530 frames with an input: 1526 have one, 4 have two), about every frame while the mouse moves.
- S→C `0x1dc` kind 0 with the same cell, header `(player,0,0,0)`, at `f+1` (94 %), `f+0` 5 %. Not every input is echoed: **never while rolling or sprinting** (every input, all sessions), mostly not while an attack is running (204/237), and 0.5 % of idle/walking inputs (`5/1114` in the last session; all of session 5's 1432 were echoed).
- In click-to-move mode (2026-10-20, `f3300-4600`) the client sends **no `0x62` at all**.
- Mode 3 (entity, hovered) is sent often (943 in the last session) but only 11 `0x1dc` with kind 1 exist in all sessions: it is not echoed one-to-one.
- Facing is entirely a `0x1dc` effect; there is no `0x1df`.

### Stun
One light stun in the 2026-10-20 session (f4072, a monster hit the player while he attacked; hit time unknown): `0x1d3` `is_light_stunned=1, is_immobilised=1` (header `(7,11)`), `0x1eb` buff row 2404 (flag `0x02`), `0x1d3 on_full_energy_shield=0` (`(7,14)`); 7 frames later `0x1d3 number_of_times_stunned_in_past_duration_from_ot=1` and `virtual_light_stun_threshold_+%_final=50`; **15 frames after the start the stats go back** (`is_light_stunned=0, is_immobilised=0`, header `(5,1)`) and the attack asked for meanwhile starts (`0x1c8` flags `0x410`, header `(5,2)`, `0x1ca`). The attacks and a Move press during the stun get their `0x1ae` and nothing else; the server cell stays put for the 15 frames. Knockback (`0x1e0`) and stuns while walking or rolling were not captured.

## Session 2026-10-20 index
`dumps/0.5.5e/captures/official/2026-10-20_18-41-14_336/` (13 299 instance messages, 4 765 frames, 33 min; `check_capture`: every movement message decodes with 0 errors and 0 leftover bytes; only `AddInventory` (known, items) mismatches). The user's checklist, as found (frame = capture frame, `T` = tap, `H` = hold, the number is the frames Space was down):

| Item | What is there |
|---|---|
| 1 standing roll, 5 taps + hold | f516 T7, f560 T8, f606 T5, f652 T3, f696 T3, f744 T4; hold f1132 H125 (sprint, cursor-follow stream, wall f1243, Space up f1257) |
| 2 taps 0.1/0.3/0.45/0.5 s | mostly **standing** (the keys were up): T3 (f652, f696, f3645, f3679), T4-8 (f516-f745, f1477, f1514, f1581, f1600, f1633), walking: T7 (f2458), T8 (f2065), T12 (f1934); T14 (f1572, f4178). No 9-frame (0.3 s) tap, 15-18 not hit |
| 3 chained re-press | f1476/1514 (38 apart), then after a pause f1570 (press+release in one frame), f1581 (+9), f1600 (+28), f1633 (+31); f3607 H20, f3645 T3 (+38), f3679 T3 (+34), f3711 T5 (+31), f3742 H155 (+29); no re-press while sprinting |
| 4 stop all WASD during sprint / roll | f1793-1796: keys released with Space down (cursor vector, then Space up, `(0,0)`); no `(0,0)` during a roll or sprint |
| 5 attack while rolling; Esc during a sprint | attacks in the roll at f1946, 1991, 2075-2088, 4033; Esc (`SetPause`) f1543-1570, 1844-1852, 3224-3238, 4612-4628, never during a roll or sprint |
| 6 weapon swap | not found (`0xfe` f2573/2620, `0x200` f3286/4662 are not weapon swaps: input mode) |
| 7 sprint into wall / monsters | wall: f1243, f1918, f2873; monsters: sprints ended by attacks f2393, 4056, 4142, 4262 |
| 8 roll with monster hovered; get hit | no `0x1aa`/`0x1b0`; one light stun f4072 |
| 9 click-to-move | hold f3325 (41 frames), singles f3375, 3410, 3441, 3473, 3492, 3509, hold f3527-4002 with rolls, f4069/4100/4160/4462 holds with attacks and rolls |
| 10 held click + Space | inside holds: f3607 H20 (sprint cut short, ends f3643), f3645 T3, f3679 T3, f3711 T5, f3742 H155 (sprint 3759-3897); f4105 H52, f4178 T14, f4216 H64 (holds f4100-4259); f4484 H100 (hold f4462-4583) |

## Reproducing
```
python3 scripts/re/movement_sequences.py never                                  # opcodes never sent
python3 scripts/re/movement_sequences.py scen --scenario wasd roll sprint stance aim [--max N] [--verbose]
python3 scripts/re/movement_sequences.py events <session> --from 2470 --to 2660 [--aim] [--all]
python3 scripts/re/movement_sequences.py list
cd modules/protocol && cargo run -p poe-proto --example check_capture -- opcodes/0.5.5e <packets.jsonl>
python3 scripts/re/movement_fixtures.py build        # rewrites crates/poe-game/tests/fixtures/*.json from PRESETS
cargo test -p poe-game --test replay                 # replays them through the World
```
The decoders in the script match `poe-proto`'s (`skill.rs` has round-trip tests built from capture bytes). The script needs no dependencies; stat and action names come from `GameFiles/poe-hook-data/` (written by `cargo xtask dll-data`) when present.

## Server implementation
Since WP2.3 the instance server does not compute movement itself. It is thin glue around the `poe-game` World (design: [instance-world.md](../architecture/instance-world.md)):

- **Input.** `servers/instance-server/src/movement.rs` `client_input` maps a client packet, looked up by its name in the opcode table, to a `MoveInput`: `0x63` MoveDirection, `0x1a5` UseSkill, `0x1a8` UpdateSkillTarget, `0x1ac` SkillInputEnd, `0x1ad` SkillReleased, `0x2e6` SprintInput, `0x62` AimTarget (mode 2 `Aim`, mode 3 `AimEntity`, modes 0/1 `AimMode`). `Player::input` queues it in the session's `World`. Nothing is answered at once.
- **Tick.** The session's frame timer (`[instance] tick_ms`) calls `World::tick()`. The events of that frame are encoded by `Codec::encode` (opcodes from `instance.toml`, stat keys from the game's `Stats` table, skill ids and the walking speed from the character class and game data, `[movement] sprint_buff_rows` from config) and sent in order, followed by the `0x2f4` SimulationTick marker. So a reply to an input arrives in the first batch after the input (`r+0`), as on the official server; the batch number is the `World`'s frame.
- **Event to message.** `MoveAck` is `0x64` (header `(entity, entity, 9, 0)`, the unscaled vector, or scaled to length 550 while sprinting), `ActionStart` is `0x1c8` (its `flags` come from the event: the request's flags plus `0x10`, WP2.1b), `ActionEvent` is `0x1ca`, `Stats` is `0x1d3` set 0, `SkillAck` is `0x1ae` (the "confirmed" target flag `0x04` is set here), `TargetAck` `0x1af`, `InputEndAck` `0x1b1`, `ReleasedAck` `0x1b2`, `SprintState` `0x2e7`, `BuffAdded`/`BuffRemoved` `0x1eb`/`0x1ec`, `AimEcho` `0x1dc` kind 0, `StanceChanged` a `0x2f9` Player component delta (header `(entity, 0xffffffff, 0, 0)`), `Teleport` a `0x64` with direction `(1, 0)` and the new cell followed by one with `(0, 0)`. All other headers are `(entity, 0, 0, 0)`.
- **`0x1ca` is skipped** until `ActionEvent = 0x01ca` is added to the protocol repository's `instance.toml` (the glue defines the empty message locally and checks the table). The client only decrements a counter for it.
- **Terrain.** The world runs on the client's walkability grid dump (`spawn::client_grid`), or without collision (`Open`) when the dump is missing: A* for click-to-move, wall sliding for the rest. The server's position is authoritative.
- **Pause.** `0x1ff` PauseRequest calls `World::set_paused`: frames still count (and the tick marker is still sent), but nothing moves or is answered, and inputs wait.
- **Unknown or unanswered.** `0x1aa` UpdateSkillTargetEntity is decoded and logged only (there are no entities to target yet, so no `0x1b0`). A movement packet the table does not know ends the batch as any unknown client message does.
- **Removed.** The earlier workarounds (turn-cell choice, turn timing, cursor facing with its projection, the use of `0x1cb` and `0x1df`, held-click-as-WASD, the roll `Pending` queue) are gone together with their three server commands. Details of what they measured stay in git history (`docs/protocol/movement.md` before 2026-10-20).

### Drift checker (PSY2)
The DLL's per-frame position report (`servers/instance-server/src/sync.rs`, DLL side `position_sync.rs`) no longer touches the simulation. With `[dev] drift_check = true` (default off; needs `[dev] position_sync` and the DLL) the session logs, every `[dev] drift_interval_ms` (default 1000 ms), one `drift` line at info level:

`drift server=x,y client=x,y cells=<now> max_cells=<largest since the last line> samples=<n> state=<server movement state>`

`cells` is the distance in grid cells between the server's exact position and the client's latest report (reports older than 250 ms count as none). The client trails the server by the reply latency, about two frames: expect up to ~2.5 cells while walking and ~4 while sprinting, and about 0 standing still. Real drift is a distance that grows over time or stays high while standing. A warning is logged once if the checker is on and no reports arrive within 5 s. The overlay's Server tab shows the same distance (`drift_cells`). Server-side only; nothing is sent to the client.

## Lag
- **No lag compensation to turn on.** The world is always lockstep, and the client plays simulation frames as they arrive. `0x1d` PingReply only feeds the latency display. Lag therefore comes from the server: late frames freeze movement.
- **Server frame time.** A debug build's A* (box-shaped search, heap allocations per node) took ~108 ms per 33 ms frame on the Riverbank grid. The A* uses flat arrays over a box around start and goal (`MARGIN` 24 cells) with a node budget (~0.5 ms debug). The server warns when a frame takes longer than the tick. Run `--release` for play.
- **Unknown client messages end the parse of their whole batch** (no length prefix): before `0x1a8` was decoded, a `0x1a5` in the same batch was lost, which was one cause of input lag. `SkillInputEnd` (`0x1ac`) stays in `[client_body_sizes]` for the same reason.
- **Report target.** The DLL sends reports (and the debug link) to `position_sync_port` (set by `cargo xtask deploy-client` from `[network.instance] public_port`); only the [drift checker](#drift-checker-psy2) reads them.

## Pause
`0x1ff` PauseRequest also pauses the server simulation (`World::set_paused`). Nothing moves on either side while the escape menu is open in a solo game. Verified: with D held, Escape after 0.7 s, Escape again 2 s later, then D released 0.7 s after that. The client stopped for the 2 s, and both sides ended one cell apart, (330, 1106) on the server and (331, 1105) on the client.

## Saved location
- When a session ends (exit to character selection, log out, closing the game or a dropped connection), the instance server stores the character's area, the area's terrain seed and its grid cell (`poe_store::Location`, migration `0005_character_location.sql`).
- On the next login into that area, the login server puts the location in the ticket. The instance server sends the same seed, so the layout is the same, and spawns the player on the saved cell (or the nearest walkable cell if it is blocked). Characters that have never been in the game start at the area's start position.

## Test tooling
Key and mouse injection for movement tests (`in:`, `inclick:`, `inpress:`, `indown:`/`inup:`/`wait:`, posted keys such as Space) and `movement_log`: [../guides/setup.md](../guides/setup.md#poe-hookkeys-entries), [../architecture/client-dll.md](../architecture/client-dll.md#movement-log). `movement_log` (the DLL's read-only per-frame position log, `frame #n ... world ... step ... grid`) is kept: it is what the official captures' client positions and the roll tables come from, and the unattended tests set it through `POE_HOOK_MOVEMENT_LOG`.

**A roll test needs a real hold of Space**: the plain `0x20` entry (and `0x20:ms`) posts key-down and key-up in one go, the client's polled key state reads "up", and `0x1a5` and `0x1ac` arrive together before the roll starts: the client then sends **no `0x1a8` at all** (4 of 4 rolls) and rolls along its facing instead of steering. Use `indown:0x20`, `0x20`, `wait:250`, `inup:0x20` (a hold: `0x20:1400` between them); with it the client sends `0x1a8` every 2 frames and each gets its `0x1af`. The skill id (ours 1, official `0x80000040`) makes no difference.

**In-game check, 2026-10-20 (drift checker, `[dev] drift_check`, cells; max in the second / after the stop; teleport lines left out)**:

| Scenario | max | after stop |
|---|---|---|
| WASD walk and turns (4 directions) | 3.2 | 0.4 |
| roll while walking (W, D) | 4.1, 3.9 | 1.8, 3.1 |
| standing roll, 3 cursor directions, open ground | 3.9 | 0.5 to 0.7 |
| standing roll, cursor steering while the player moves | 7.2 | 7.2 |
| sprint, Space held 1.0 s, W held | 6.4 | 2.7 |
| sprint, Space held 1.4 s, W held (open ground, 2 runs) | 13.8, 11.2 | 13.0, 10.1 (gap 11) |
| click-to-move (3 clicks) | 1.3 | 0 |
| held click | 2.5 | 2.2 |
| click, then roll | 5.4 | 3.8 |
| standing roll in click-to-move mode | 14.9 | 5.2 (gap 12) |

The first standing-roll runs of the day were 28-35 cells off because the test sent no `0x1a8` (above); with steering they are within the roll tolerance on open ground.

## Movement trace
`[instance] movement_trace` writes `runtime/movement-trace.tsv`; `scripts/re/movement_trace.py` lists the frames where the player moved backwards or stalled.

## Gap list
What the seven official captures **still do not cover** (after the 2026-10-20 session, which closed the former gaps 1, 2, 4, 5, 7, 10 and 14 in full and 3, 6, 8, 9, 12 in part; their results are in the sections above). A new capture is one read-only official session by the user (DLL in `Official` mode); the recording must be kept intact (`packets.jsonl`, `poe-hook.log` with `movement_log` on). The markers `UNVERIFIED (gap #n)` in `crates/poe-game/src/locomotion.rs` use these numbers.

| # | Missing | Why it matters | What to do in the session |
|---|---|---|---|
| 1 (med) | **Release between `t0+15` and `t0+18`** (only up to `t0+14` and from `t0+19` seen); a release at `t0+31..34` | exact threshold of the 16/30-frame split and of the sprint end at `t0+35` | taps of 0.5, 0.52, 0.55, 0.6 s (15-18 frames) while standing and walking |
| 2 (med) | **Click-to-move details**: a walk that reaches its target (no arrival message seen), a click on a wall or unreachable cell, WASD pressed during a click walk, the trigger of the held click's 15-frame restarts, the repeat right after a sprint's end that got no `0x1c8` | arrival state, A* goal rules, takeover rules | click once on open ground and **wait 3 s**; click on a wall and on a roof edge; walk by click and press W; hold the button 5 s without rolling or attacking |
| 3 (med) | **Rolls and sprints re-pressed**: Space pressed again **while sprinting**, a tap-roll followed by a press at 15-27 frames, a queued roll whose key is already up when the lock ends (single slot vs "key must be held"), a press exactly 30 frames after the previous start | the roll queue rule | roll, release, press again after 0.5, 0.6, 0.8 s and release at once; while sprinting release and press Space within 0.2 s |
| 4 (med) | **WASD pressed during a roll that started standing**, WASD released during a roll along the keys; WASD stop during a sprint with Space released first | the roll heading rule when the keys change | roll standing, press W during the roll; roll with W, release W during the roll |
| 5 (med) | **Esc menu during a roll or a sprint**; a click-to-move press during a roll | pause/cancel rules | open and close the menu during a sprint and during a roll |
| 6 (med) | **Attack while walking**, attacks in click-to-move mode, an attack during a roll in the last frames (`t0+25..29`) | whether attacks stop the walk, the `t0+24` window edges | attack with the main skill while walking (WASD and click) |
| 7 (med) | **Weapon swap** (nothing found), stance/input-mode change while walking, rolling or sprinting | `0x200` timing relative to movement; the `0xfe`/`0x1d5` pair | swap weapon sets while walking and standing; toggle the input mode while walking |
| 8 (low) | **Sprint against a monster** (blocked by an entity, not a wall), knockback (`0x1e0`) and stuns while walking or rolling, `0x1b0`/`0x1aa` with a Move or roll | stream while blocked by entities; movement refusal while stunned | sprint into a pack; get hit while walking and while rolling; roll while a monster is hovered |
| 9 (low) | Buffs with definition-dependent `0x1eb` fields (flag `0x10`, rows with a `u16`/vector from the definition; 6 of 51 `0x1eb` in the captures decode with 2 bytes left) | only the three sprint buffs are fully decoded | (no capture needed: decode the buff definition table) |
| 10 (low) | The monster's cell in the `0x1c8` target of an attack on a hovered monster (505,963 vs the press' 454,969): needs entity positions in the world, and the `u1a` header values (1/3/4/8/17/25) of the replies | entity-targeted attacks | (server work: entity positions) |
| 11 (med) | **Walking on after a sprint ended with the key held and no `0x64` stream** (WASD held since before the roll, Space released): our client slows down on its own from 1.3 to 0.4 cells per frame over ~30 frames (2 runs), the official client in `f2393` stepped 1.0, 0.97, 0.91, 0.74, 0.8, 0.35 (plateau 7 frames) and then rose to 1.35; the server keeps walking at 1.23 and ends 10 to 13 cells ahead | the walk-on rule after a sprint without stream (RE: what the client does with its WASD path at the sprint end, or what `0x64` it expects) | sprint with W held since before the roll, release Space, keep W 2 s |
| 12 (med) | **Roll heading in the first frames and in click-to-move mode**: the client rolls towards its facing/last direction for 2 to 7 frames (in quantized angles) before it follows the `0x1a8` offsets, and in click-to-move mode one standing roll kept west (180 degrees) while the offsets pointed north-west (-133) until it hit a wall; the server rolls along the offset from the first frame | up to ~5 cells of drift per roll, more near walls | standing rolls in click-to-move mode with the cursor 90 and 180 degrees from the last walk; rolls from a standstill after turning the cursor |

Closed by the 2026-10-20 session (kept for reference, with the evidence): click-to-move replies (sections above), standing roll, chained rolls and the roll queue rule, WASD stop during sprint (never sent: the client sends the cursor direction), held click during roll/sprint, input-mode/stance change (standing), sprint into a wall (3), roll path and speed profile (30 rolls with client positions), non-8-way `0x63` vectors (cursor direction while Space is held).
