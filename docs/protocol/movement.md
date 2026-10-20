# Movement

Specification of how the client moves the player and what an **official** server sends back, from six read-only official capture sessions (`dumps/0.5.5e/captures/official/`, 2026-10-14..17) and the client's code (Ghidra). The tool that produced every sequence below is `scripts/re/movement_sequences.py` (`list`, `events`, `scen`, `never`; see [Reproducing](#reproducing)). Code: `crates/poe-game/src/locomotion.rs` (the movement component built from this spec, [instance-world.md](../architecture/instance-world.md)), `world.rs`, `movement.rs` and `pathfinding.rs` (the simulation), `servers/instance-server/src/{movement,player}.rs` (the packet glue, [below](#server-implementation)), messages in `modules/protocol/poe-proto/src/instance.rs` and `skill.rs`.

Captures are from **WASD input mode** only. Real click-to-move, held clicks, standing rolls, chained rolls and stance changes are **not** in them; see the [gap list](#gap-list).

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
- Dodge roll: `dodge_roll_travel_distance` (`0x141aa90d0`, stat `0x57fa`) = 46 cells; measured profile on our server: ~3.9 cells per frame at the start, falling ~0.225 per frame to ~0.93.

## Messages

### Client → server
| Op | Name | Body |
|---|---|---|
| `0x63` | MoveDirection | `u8 1, varint x, varint y, u8[3] 0`. Sent when the held direction changes. Vectors of length ~180 in grid axes: `(127,127)`, `(180,0)`, `(-127,-127)` …, `(0,0)` = stop. Arbitrary angles also occur, e.g. `(75,172)`, `(-53,172)`, `(180,-9)`. |
| `0x62` | AimTarget | `u8 player, u8 mode`; mode 2: `i32 x, i32 y` **absolute cell**; mode 3: `u32 entity`; modes 0/1: nothing (once at entry). About one mode-2 per frame while the cursor moves, only in combat stance. |
| `0x1a5` | UseSkill | `u8 player`, skill block: `u32 skill, u16 flags, u8 0xff, u8 target flags, [u32 entity if flags & 8], i32 x, i32 y` (x, y **relative to the player's cell**, absolute with a target entity). |
| `0x1a8` | UpdateSkillTarget | `u8 player, i32 x, i32 y (relative), u32 skill`. About every 70 ms (2 frames) while a roll or sprint runs (key held or not) and while a held skill runs. |
| `0x1aa` | UpdateSkillTargetEntity | `u8 player, u32 entity, u32 skill`. |
| `0x1ac` | SkillInputEnd | `u8 player`. Held skill input ended: Space released (always together with `0x2e6` held=0), attack button released. |
| `0x1ad` | SkillReleased | `u8 player, u32 skill, u8`. Only seen for the Move skill (the intro click). |
| `0x2e6` | SprintInput | `u8 player, u8 held`. Space down / up. |

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
| `0x64` | MoveDirectionUpdate | `varint dir x, varint dir y, varint cell x, varint cell y, bool`. Direction: the `0x63` vector verbatim while walking, **×550/180 (e.g. `(0,-550)`, `(388,-388)`) while sprinting**. The cell is the server's player cell. The bool is `0xf1` (true) in 94 % of the replies, `0` otherwise (every 31st frame of a sprint stream, otherwise sporadic); the client reads it as a bool and ignores it. |
| `0x1c8` | StartAction | `u16 flags (0x410 for rolls/attacks, 0x80 = entity follows), [u32 entity], varint cell x/y (start), varint target x/y (absolute), u32 skill, u16 action id, u8 0xff`. Action ids are one **server-wide counter** (32768+, e.g. 32872 → 32875 → 32877 across all entities). |
| `0x1ae` | UseSkillAck | the skill block with **absolute** x/y and `target flags | 0x04` ([skill.rs](../../modules/protocol/poe-proto/src/skill.rs)). |
| `0x1af` | UpdateSkillTargetAck | `i32 x, i32 y, u32 skill`: absolute target = server player cell + the relative offset of the `0x1a8`. |
| `0x1b0` | UpdateSkillTargetEntityAck | `u32 entity, i32 x, i32 y (the entity's cell), u32 skill`. |
| `0x1b1` | SkillInputEndAck | no body. |
| `0x1b2` | SkillReleasedAck | `u32 skill, u8` (header `u1a` = 6). |
| `0x2e7` | SprintState | `u8 held`. Echo of `0x2e6` (sets Actor +0x2a2 bit 5). |
| `0x1d3` | StatsUpdate | `u8 set, varuint n, n × (varuint key, varint value)`. |
| `0x1eb` / `0x1ec` | BuffAdded / BuffRemoved | `u16 index, u16 buff row, …` / `u16 index` (see [skill.rs](../../modules/protocol/poe-proto/src/skill.rs)). |
| `0x1ca` | (action event) | no body; with every roll start and when an attack ends a sprint. Actor case `0x14214a13e` decrements a counter (+0x348). |
| `0x1dc` | SetAimTarget | `u8 kind (0 cell, 1 entity), i32 x, i32 y`, header `(player, 0, 0, 0)`. |
| `0x2f9` | ComponentDelta | player stance and flags (component 6). |

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
| Rolling | `0x63 v` | Rolling | `r+0`: `0x64 v, cell` (the client acts on it when the roll ends) |
| Rolling, key released before `t0+15` | `0x1ac` + `0x2e6 held=0` | Rolling → Walking/Idle at `t0+30` (see [roll action](#roll-action-length)) | `r+0`: `0x1b1`, `0x2e7 held=0`; `t0+16`: roll-end stats |
| Rolling, key still down at `t0+15` | (nothing) | Sprinting | `t0+15`: sprint start; `t0+30`: roll end; `t0+35`: `must_sprint=0` |
| Sprinting | `0x63 v` | Sprinting(v) | from the first `0x63` after sprint start: a `0x64` **every frame**, direction ×550/180 |
| Sprinting | `0x1ac` + `0x2e6 held=0` | Walking(v) | `r+0`: `0x1b1`, `0x2e7 held=0`, `0x64`, sprint-off buffs/stats; `r+1`: `0x64 v` unscaled |
| Sprinting | `0x1a5` attack | Attack | sprint-off stats + `0x1c8` attack; `0x2e7 held=0` follows when the key goes up |

### Roll batch (`r+0`), identical in all 15 rolls
Order of the reply to `0x2e6 held=1` + `0x1a5 {roll}` (client sends both in the same frame, `0x2e6` first):

1. `0x2e7` SprintState `held=1`
2. `0x1c8` StartAction: `skill 0x80000040`, flags `0x410`, start cell = server player cell, target = cell + the relative cursor offset, `action id`, `0xff`; header `(player,0,0,0)`
3. `0x1ca` header only, `u1a=9`
4. `0x1d3` `is_dodge_rolling=1`, `u1a=9`
5. `0x1ae` UseSkillAck, `u1a=10`, target flags `0x04`, absolute cell = start cell + offset
6. `0x1d3` `stun_threshold_+%_final_while_performing_action=100, virtual_heavy_stun_threshold_+%_final=100, virtual_light_stun_threshold_+%_final=100`, header `(player,player,5,6)`

If an earlier `0x1a8` is still unanswered it is answered first (`0x1af` before this batch's messages). In one case (second roll 28 frames after the first, i.e. 12 frames after the first roll ended) the `0x1c8` and stats came **2 batches later** with scheduled headers `(player,player,5,8)`, `(…,5,17)`, `(…,5,25)` while `0x2e7`/`0x1ae` were in `r+0`.

### Roll end and sprint (timeline in frames after the roll's `0x1c8`, `t0`)
Measured in all 15 rolls; the offsets are constant (server logic, not latency):

| Frame | Tap (key released before `t0+15`; 5 rolls) | Hold (key still down; 10 sprints) |
|---|---|---|
| `t0+0` | roll batch | roll batch |
| key release (`k`) | `0x1b1`, `0x2e7 held=0` | (see end of sprint) |
| `t0+15` | – | `0x1eb` ×3 (indices n, n+1, n+2; buff rows 2758 `0xac6`, 2737 `0xab1`, 2853 `0xb25`, flags 0) and `0x1d3` `sprint_blend_time_override_ms=1, is_sprinting=1, must_sprint=1, all_hits_stun_you=1, virtual_skill_ignore_namelocking_dynamically=1`, header `(player,player,5,2)` |
| `t0+16` | `0x1d3` stun thresholds 0 + `is_dodge_rolling=0` (roll over) | – |
| `t0+30` | – | `0x1d3` stun thresholds 0 + `is_dodge_rolling=0`; `0x1d3` `base_move_speed_+%_from_sprint=50, movement_velocity_+permyriad=5000`; `0x1eb` ×2 (buff index n+1 `0xab1` again and n `0xac6` with flags `0x02` = f32 `0.1`) |
| `t0+35` | – | `0x1ec` buff index n; `0x1d3` `must_sprint=0`, header `(player,player,3,0)` |

So **`is_dodge_rolling` is cleared 16 frames (533 ms) after the roll on a tap and 30 frames (1 s) when Space is still down at frame 15**, and the sprint begins at frame 15 while the roll is still running.

### Roll action length
Found while replaying the captures (WP2.2, `crates/poe-game/tests/replay.rs`): the roll *action* lasts **30 frames in both cases**. After a tap roll's stats at `t0+16` the server still treats the player as rolling until `t0+30`:
- **Aim is not echoed** (`0x62` → `0x1dc`) for aims up to `t0+26` at least; the `0x1a8`/`0x1af` pairs go on.
- **A new roll waits.** The one chained roll asked for 27 frames after the first one started got its `0x2e7` and `0x1ae` at `r+0` and its `0x1c8`, `0x1ca` and stats exactly at the first roll's `t0+30` (the "2 batches later" case above, 36 − 6 = 30). The three chained rolls asked for 31+ frames after the first one started began at `r+0`. The server code assumes every new roll press during a roll behaves like this (UNVERIFIED, [gap #4](#gap-list)).
- Our roll speed profile covers its 46 cells in about 28 frames, which fits an action of ~30 frames.

### Reply details found by replay
- **Direction scaling truncates**: `dir × 550 / 180` with integer division toward zero (`(-53, 172)` → `(-161, 525)`, not `-162, 526`).
- **`0x64` comes after the other replies of its frame** (input replies, timeline events); in the key-release case the sprint-off events follow it, in the roll/sprint timeline frames (`t0+30`, `t0+35`) the timeline events come first and the stream message last.
- **Attack ends the sprint** (key release and attack in one frame): `0x1a8` ack, `0x1b1`, `0x2e7 held=0`, attack `0x1c8`, `0x1ec n+1`, sprint-off stats, `0x1ca`, attack `0x1ae`, **`0x64` unscaled** at `r+0`, `0x1ec n+2`, `all_hits_stun_you=0`.

### End of sprint (all 10, key up `k` frames after the roll)
`r+0` batch: `0x1b1`, `0x2e7 held=0`, `0x64` (still scaled), then `0x1ec` (index n+1), `0x1d3` `base_move_speed_+%_from_sprint=0, movement_velocity_+permyriad=0, sprint_blend_time_override_ms=0, is_sprinting=0, virtual_skill_ignore_namelocking_dynamically=0`, `0x1ec` (index n+2), `0x1d3` `all_hits_stun_you=0` (headers `(player,player,9,7)`); `r+1`: `0x64` with the **unscaled** vector. If the player used an attack (`0x1a5` `0x80400040`) while sprinting, the sprint is ended by it instead: `0x1ec`, `0x1d3 must_sprint=0`, the attack's `0x1c8`, `0x1ec`, `0x1d3` sprint stats 0, `0x1ca`, `0x1ec`, `0x1d3 all_hits_stun_you=0`; the key release (`0x1ac`+`0x2e6 held=0`) then gets `0x1b1`/`0x2e7 held=0` with no stats. Sprint end causes over the 10 sprints: key up 6, an attack skill 4.

### Sprint stream
From the **reply to the first `0x63` after the sprint started** until the sprint ends, the server sends **one `0x64` per frame** (no frame missing in 10 sprints) with the current direction ×550/180 and the server's cell, which advances 1.83..1.89 cells per frame. A new `0x63` changes the vector in the next frame's `0x64` (`(127,-127)` → `(388,-388)`, `(0,-180)` → `(0,-550)`; non-axis vectors scale the same way). The bool of the `0x64` stream is `0` every 31st frame (12 of 12 gaps are exactly 31; the phase is not tied to the stream start).

## Scenario sequences

All numbers are from the official captures; `movement_sequences.py scen --scenario <name>` prints them per session.

### WASD start / turn / stop
- Six sessions, ~630 direction inputs: every `0x63` has exactly one `0x64` (at most one input per session, the last of the capture, is unanswered).
- **Start**: `f: 0x63 v` → `f+1: 0x64 v` with the cell where the player is (e.g. `0x63 (-127,-127)` at f856 → `0x64 (-127,-127) cell (284,1141)` at f857). **Turn**: same, new vector. **Stop**: `0x63 (0,0)` → `0x64 (0,0) cell`.
- No other message accompanies WASD. A stance delta (`0x2f9`) is not sent on the first Move in the 2026-10-15/17 sessions (it comes at entry, see [Stance](#combat-stance)).
- The reply cell moves ~1.23 cells per frame between replies; the client turns exactly when it receives `0x64` (frame `t+1`) and puts the entity on the centre of the cell it is given, so the server sends its own simulated cell (as the official one does) and keeps the simulation exact instead of correcting the client.
- While rolling the server still answers each `0x63` immediately with a walking `0x64` (12 inputs in rolls).

### Click-to-move (no real data)
The only Move clicks in the captures are the intro's first click of each session (skill `0x20002909`; its `u16 flags` equals the player's entity id, unlike skills with `0x0400`; every session was WASD input mode): `0x1a5` + `0x1ad` in the same frame → `r+0`: `0x2f9` stance delta (older sessions), the four `0x1d8` skill stats containers + `0x1d3` (skill-list bootstrap), `0x1c8` StartAction **skill `0x2000b7d9` (wake-up)** at the player's cell, `0x1ae` (flags echoed, `x=y=0`), and `0x1b2 (skill, 0)` in the next batch. It is not a walk. A real click walk, a held click and its re-aim are therefore unverified; **from the client code** the lockstep design is: `0x1a5` → `0x1ae` (stored in the Move slot Player +0x22c) + `0x1c8` Move StartAction with the absolute target; held-click repeats (`target flags 0x10`) → `0x1ae` each; re-aim `0x1a8` → `0x1af` (absolute target) which re-targets the running Move through Actor vfunc +0x128; release `0x1ad` → `0x1b2`. See the [gap list](#gap-list).

### Dodge roll
Rolls in the captures: 15 (all started while a WASD direction was held; none standing).
- **Moving** (all 15): the batch above. The roll's direction is the cursor offset from the `0x1a5`; `0x1c8.target = cell + offset` (e.g. `(371,1052) + (15,-35) = (386,1017)`), not clamped to 46 cells (offsets of 19..58 cells seen; the client rolls its fixed 46 cells).
- **Steering**: every `0x1a8` (every 2 frames, key held or not, until the roll ends) → one `0x1af` with `x,y = server cell + offset`. Example (sprint): `0x1a8 (23,-27)` → `0x1af (430,1036)` with the server cell `(407,1063)`.
- **Chained** (second roll 28/34/31/34 frames after the previous one; only taps): each is a full roll batch, the second may be delayed 2 batches (above).
- Our server sends exactly this batch ([implementation](#server-implementation)); it no longer uses `0x1cb` steering.

### Sprint
Hold Space after the roll: the sequences above (`t0+15`, `t0+30`, `t0+35`, stream, end). `0x2e6` is sent once (on down) and once (on up); the client repeats `0x1a5` with target flags `0x13` and flags 0 only in the 2026-10-14 22:23 session (7 repeats; none in the 2026-10-17 session), each acked with `0x1ae` flags 0, target flags `0x17`.
- Sprint with held Space also continues the `0x1a8`/`0x1af` pairs every 2 frames.
- `0x62` aims are not echoed while sprinting (116/116 not echoed) or rolling (150/150).

### Combat stance
`0x2f9` ComponentDelta, component 6 (Player), mask `0x0400` = stance byte. In the captures stance changes **only at entry**: 2026-10-15/17 sessions: `f10: mask 0x0400, stance=1` (9 frames after the client's first `0x62` mode 1); 2026-10-14 sessions: `f10: mask 0x0402` (flag words, stance=1), `f11: mask 0x1400 (stance=0, +0x3e7=1)`. Other Player deltas: `mask 0x0402/0x0002` flag words after the first skill release and new areas, `mask 0x0201` experience after kills, `mask 0x2000 (00)` on level-ups. No stance change while moving, rolling, or sprinting was captured.

### Aim
- C→S `0x62` mode 2 with the **absolute** cell under the cursor, at most one per frame (1530 frames with an input: 1526 have one, 4 have two), about every frame while the mouse moves.
- S→C `0x1dc` kind 0 with the same cell, header `(player,0,0,0)`, at `f+1` (94 %), `f+0` 5 %. Not every input is echoed: **never while rolling or sprinting** (every input, all sessions), mostly not while an attack is running (204/237), and 0.5 % of idle/walking inputs (`5/1114` in the last session; all of session 5's 1432 were echoed).
- Mode 3 (entity, hovered) is sent often (943 in the last session) but only 11 `0x1dc` with kind 1 exist in all sessions: it is not echoed one-to-one.
- Facing is entirely a `0x1dc` effect; there is no `0x1df`.

## Reproducing
```
python3 scripts/re/movement_sequences.py never                                  # opcodes never sent
python3 scripts/re/movement_sequences.py scen --scenario wasd roll sprint stance aim [--max N] [--verbose]
python3 scripts/re/movement_sequences.py events <session> --from 2470 --to 2660 [--aim] [--all]
python3 scripts/re/movement_sequences.py list
cd modules/protocol && cargo run -p poe-proto --example check_capture -- opcodes/0.5.5e <packets.jsonl>
```
The decoders in the script match `poe-proto`'s (`skill.rs` has round-trip tests built from capture bytes). The script needs no dependencies; stat and action names come from `GameFiles/poe-hook-data/` (written by `cargo xtask dll-data`) when present.

## Server implementation
Since WP2.3 the instance server does not compute movement itself. It is thin glue around the `poe-game` World (design: [instance-world.md](../architecture/instance-world.md)):

- **Input.** `servers/instance-server/src/movement.rs` `client_input` maps a client packet, looked up by its name in the opcode table, to a `MoveInput`: `0x63` MoveDirection, `0x1a5` UseSkill, `0x1a8` UpdateSkillTarget, `0x1ac` SkillInputEnd, `0x1ad` SkillReleased, `0x2e6` SprintInput, `0x62` AimTarget (mode 2 `Aim`, mode 3 `AimEntity`, modes 0/1 `AimMode`). `Player::input` queues it in the session's `World`. Nothing is answered at once.
- **Tick.** The session's frame timer (`[instance] tick_ms`) calls `World::tick()`. The events of that frame are encoded by `Codec::encode` (opcodes from `instance.toml`, stat keys from the game's `Stats` table, skill ids and the walking speed from the character class and game data, `[movement] sprint_buff_rows` from config) and sent in order, followed by the `0x2f4` SimulationTick marker. So a reply to an input arrives in the first batch after the input (`r+0`), as on the official server; the batch number is the `World`'s frame.
- **Event to message.** `MoveAck` is `0x64` (header `(entity, entity, 9, 0)`, the unscaled vector, or ×550/180 while sprinting), `ActionStart` is `0x1c8` (flags `0x410` for rolls and attacks, 0 for Move), `ActionEvent` is `0x1ca`, `Stats` is `0x1d3` set 0, `SkillAck` is `0x1ae` (the "confirmed" target flag `0x04` is set here), `TargetAck` `0x1af`, `InputEndAck` `0x1b1`, `ReleasedAck` `0x1b2`, `SprintState` `0x2e7`, `BuffAdded`/`BuffRemoved` `0x1eb`/`0x1ec`, `AimEcho` `0x1dc` kind 0, `StanceChanged` a `0x2f9` Player component delta (header `(entity, 0xffffffff, 0, 0)`), `Teleport` a `0x64` with direction `(1, 0)` and the new cell followed by one with `(0, 0)`. All other headers are `(entity, 0, 0, 0)`.
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
Key and mouse injection for movement tests (`in:`, `inclick:`, `inpress:`, `indown:`/`inup:`/`wait:`, posted keys such as Space) and `movement_log`: [../guides/setup.md](../guides/setup.md#poe-hookkeys-entries), [../architecture/client-dll.md](../architecture/client-dll.md#movement-log). 
## Movement trace
`[instance] movement_trace` writes `runtime/movement-trace.tsv`; `scripts/re/movement_trace.py` lists the frames where the player moved backwards or stalled.

## Gap list
Scenarios and combinations the six official captures **do not cover**, and what is needed. The capture is one read-only official session by the user (DLL in `Official` mode), WASD input mode unless noted; the recording must be kept intact (`packets.jsonl`). Priority marks what WP2.2/2.3 needs.

| # | Missing | Why it matters | What to do in the session |
|---|---|---|---|
| 1 (high) | **Click-to-move**: single click, hold, hold with cursor re-aim, release | The reply set for `0x1a5` Move (`0x1c8` Move, `0x1ae` x/y, target flags, `0x1af`, `0x1b2`) is only known from client code | Set `user_input_mode=mouse_and_keyboard` in `poe2_production_Config.ini` of the **GameFiles** copy (not Steam). Click once on open ground and wait until the player arrives; then hold the left button for ~3 s sweeping the cursor in a circle; release; click while the previous walk is running; click on an unreachable point (wall). 5 s idle between each |
| 2 (high) | **Standing roll** (no WASD held) | Our roll must start from Idle; the roll batch with no WASD state, the `0x64` after the roll ends | Stand still and tap Space (cursor to the left/right/up/down), 5 times; hold Space for 2 s once from standing |
| 3 (high) | **Roll with the key released before / after frame 15** (boundary), taps at frame 5, 10, 14, 15, 16 | exact threshold of the 16/30-frame split | taps of ~0.1, 0.3, 0.45, 0.5 s while walking |
| 4 (high) | **Sprint into roll** (press Space again while sprinting) and **roll into sprint** after a tap-roll when Space is pressed again quickly; **chained rolls inside the 16 frames** | roll cooldown/queue (the 2-batch delay at 12 frames) | roll, release, press again after 0.2, 0.4, 0.55, 0.7 s; while sprinting release and press Space within 0.2 s |
| 5 (high) | **WASD stop during a roll and during a sprint** (`0x63 (0,0)`) | reply and what the stream does when the direction is zero | sprint straight, release all keys while Space is still down; roll and release WASD during the roll |
| 6 (med) | **Stop mid-roll** by other input: a click or an attack during a roll; Escape during a roll | cancel rules; `0x1c8` ordering | attack with the main skill while rolling; open and close the menu during a sprint |
| 7 (med) | **Held click with re-aim while rolling/sprinting** (the Move input is ignored until the roll ends) | queued input | with mouse mode: hold the left button, press Space |
| 8 (med) | **Stance change while walking**; weapon swap; stance off/on | `0x2f9` mask `0x0400` timing relative to movement | in a town and in the field, swap weapon sets or sheathe/unsheathe (the key bound to it) while walking and while standing |
| 9 (med) | **Sprint via other routes**: sprint from a click-walk, sprinting into a wall or against a monster | stream while blocked; `0x64` cell not advancing | sprint into a wall; sprint into a pack |
| 10 (med) | **Exact positions**: the client's real position per frame during a roll/sprint (no position data exists in the official captures, so the roll path and speed profile are only known from our server's measurements) | roll displacement and the `0x64` cell correctness | enable the DLL's read-only position log (`position_sync`/`movement_log` in `Official` mode must only observe) for a session with 5 rolls on open ground and 5 along a wall |
| 11 (low) | **`0x1b0` with a Move/roll**, `0x1aa` outside attacks | entity-target acks are known only for attacks (4 samples) | roll while a monster is hovered |
| 12 (low) | **Stuns and knockback during movement** (`is_light_stunned`, `is_immobilised`, `0x1e0` knockback) | movement refusal while stunned | get hit while walking and while rolling |
| 13 (low) | Buffs with definition-dependent `0x1eb` fields (flag `0x10`, rows with a `u16`/vector from the definition; 6 of 51 `0x1eb` in the captures decode with 2 bytes left) | only the three sprint buffs are fully decoded | (no capture needed: decode the buff definition table) |
| 14 (low) | 0x63 vectors that are not 8-way (e.g. `(75,172)`) | where they come from (mouse hold-move? joystick?) | WASD only, then mouse-assisted |
