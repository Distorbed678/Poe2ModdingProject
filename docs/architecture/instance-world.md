# Instance world

How instances and the simulation are organised: what runs today, the `World` in `crates/poe-game` that replaces the per-connection movement (WP2.2, built; WP2.3 wires it into the instance server), and the shared-world plan. The wire side is in [../protocol/instance.md](../protocol/instance.md), the movement spec in [../protocol/movement.md](../protocol/movement.md), the commands that manage instances in [../guides/server-admin.md](../guides/server-admin.md#instances).

## Current state (instance server, until WP2.3)

- **Instance directory** (`poe_cluster::InstanceDirectory`, in-process `MemoryInstances`): holds every instance (area, seed, forced topology and orientation, the players in it, since when it is empty). Find, create, join, leave, close and idle expiry work; towns and hideouts are persistent. An "instance" is metadata today: area, seed, layout and player count.
- **One simulation per connection.** `run_session` (`servers/instance-server/src/lib.rs`) owns one `Player` (`player.rs`) with `PLAYER_ENTITY` = 1, runs its own frame clock (`SimulationTick` every `tick_ms`) and sends only that player's AddObject. Two players in the "same" instance share only chat and the player count; they cannot see each other. The movement logic in `player.rs` (`TurnCell`, `turn_timing`, `face_cursor`, `0x1cb`/`0x1df`) is what WP2.3 replaces.
- **Live registry** of online players (`live.rs`): who, kick, tell, broadcast, mute.
- **Terrain on the server:** the client generates the terrain itself from the area and seed. The server picks the start cell from the layout and cell grid the DLL exports to `dumps/0.5.5e/terrain/{layouts,grids}/` (`[spawn] layout_dir`, `spawn.rs`): `[spawn.start_tiles.G1_1] point` is a grid cell in the start tile's own orientation (the Riverbank start is on the shore beside the intro tree, inside the `Intro_Tree_01` tile), and the server finds which of the 8 rotations or mirrors the tile was placed with (`PlacedTile`, from the sub-tile index in the layout export, see [client-dll.md](client-dll.md#layout-export-format)) and maps the point onto the map. Server-side generation (`[terrain] generate`, `!terrain`) is for comparison only: [../status/terrain.md](../status/terrain.md). Until it covers every area the server picks seeds the client is known to accept (`instance.known_seeds`, from live logs) or a random one.
- **Saved location:** when a session ends the server stores the character's area, terrain seed and cell (`poe_store::Location`, migration `0005_character_location.sql`); the next login into that area gets the same seed, so the same layout, and spawns on the saved cell (or the nearest walkable cell if it is blocked). Characters that have never been in the game start at the area's start position.
- **Entities:** only the player (entity 1). NPCs need their StateMachine and quest states ([../protocol/entities.md](../protocol/entities.md)); `POE_NPC=<metadata path>` adds one more object for experiments.

## The World (`crates/poe-game`)

One `World` per instance. It knows nothing about sockets, packets or opcodes: sessions feed it typed input, the instance's tick loop calls `tick()`, and it returns typed events. The pieces:

| Module | Contents |
|---|---|
| `world.rs` | `World`, `WorldConfig`, `Entity`, `EntityId`, `Event`, `WorldEvent` |
| `locomotion.rs` | the movement component: `Locomotion`, `MoveInput`, `MoveEvent`, `MoveState`, `MovementConfig` |
| `grid.rs` | `GridSource` (terrain for the simulation), `Grid` (the client's dump), `VecGrid` |
| `movement.rs` | `Walkable`, `Mover` (advance, slide, roll speed profile, `Motion`) |
| `pathfinding.rs` | A* (`find_path`) for click-to-move and WASD planning |

### World
```rust
let mut world = World::new(WorldConfig::new(movement_config), Arc::new(grid)); // Arc<dyn GridSource>
let me = world.spawn_player((x, y));            // EntityId, ids unique per world (first_entity_id)
world.input(me, MoveInput::Direction { x: 180, y: 0 });   // queued; answered by the next tick
let events: Vec<WorldEvent> = world.tick();     // one frame; the caller owns the timing
```
- **Frame clock.** `frame()` counts ticks; `tick()` advances exactly one frame of `tick_ms` (default 33; the simulation steps `tick_ms / 1000` seconds). The world never reads a clock: the instance's timer calls `tick()`, so replays, tests and a paused instance (`set_paused`) need no special cases.
- **Entities.** `Entity { id, next_buff, movement }`: a struct with its components as fields (no ECS). `next_buff` is the entity's buff index allocator, shared by every component that adds buffs. `despawn`, `teleport(id, cell)` (reported as `MoveEvent::Teleport`), `set_grid` (the grid can arrive after the world).
- **Counters.** `StartAction` ids come from one counter for the whole world (the official ones are a server-wide counter shared by all entities, 32768+, `first_action_id`).
- **Events.** `tick()` returns `WorldEvent { frame, entity, event }`, ordered by entity id and within an entity in send order. `Event` is `#[non_exhaustive]` (today only `Movement(MoveEvent)`); new components add variants. `frame` is the batch the message belongs to: the session sends every event of a tick before it sends the `SimulationTick` marker.
- **N players.** Everything is per entity; nothing assumes a single player. Multiplayer (phase 3 of [the plan](../plans/orchestration-2026-10.md)) spawns every player into the instance's world and routes each event to the right clients.

### GridSource
```rust
pub trait GridSource: Walkable + Send + Sync {
    fn size(&self) -> (i32, i32);
    fn nearest_walkable(&self, cell, radius) -> Option<(i32, i32)>;   // default impl
}
```
`Walkable` (per cell `walkable(x, y)`, optional `clearance`) is the trait `Mover` and the A* already used. Implementations: `Grid` (the DLL's `terrain_grid_<tile hash>.bin`; parsing moved here from `instance-server/src/spawn.rs`, which re-exports it), `VecGrid` (tests, tools), `Open` (no terrain). `poe-world` will implement it for its generated collision output; the world only sees the trait.

### Movement component (`Locomotion`)
Server-authoritative movement for the client's lockstep mode, built from [../protocol/movement.md](../protocol/movement.md). Inputs mirror the client packets; the output events are semantic and in the order the official server sends the messages.

| `MoveInput` | Client packet |
|---|---|
| `Direction { x, y }` | `0x63` MoveDirection (WASD vector) |
| `UseSkill(UseSkill)` | `0x1a5` (skill, flags, target flags, optional entity, x, y) |
| `UpdateSkillTarget { x, y, skill }` | `0x1a8` |
| `SkillInputEnd` | `0x1ac` |
| `SkillReleased { skill, flag }` | `0x1ad` |
| `SprintInput { held }` | `0x2e6` |
| `Aim { x, y }` / `AimEntity` / `AimMode { mode }` | `0x62` mode 2 / 3 / 0 and 1 |

| `MoveEvent` | Server message |
|---|---|
| `MoveAck { dir, cell }` | `0x64` (dir already scaled while sprinting) |
| `ActionStart { kind, skill, action, cell, target, entity, unk }` | `0x1c8` (flags `0x410` for Roll/Other, 0 for Move) |
| `ActionEvent` | `0x1ca` |
| `Stats(Vec<(MoveStat, i32)>)` | `0x1d3` set 0; `MoveStat::id()` is the game stat id, the session resolves the network key from `poe-data` |
| `SkillAck { skill, flags, unk, target_flags, entity, x, y }` | `0x1ae`; the session ORs the "confirmed" target flag `0x04` |
| `TargetAck { skill, x, y }` | `0x1af` |
| `InputEndAck` / `ReleasedAck { skill, flag }` | `0x1b1` / `0x1b2` |
| `SprintState { held }` | `0x2e7` |
| `BuffAdded { index, row, fade }` / `BuffRemoved { index }` | `0x1eb` (flag `0x02` when `fade` is set) / `0x1ec` |
| `AimEcho { cell }` | `0x1dc` kind 0 |
| `StanceChanged { combat }` | `0x2f9` Player component, stance byte |
| `Teleport { cell }` | a `0x64` with a non-zero direction and the new cell (the client snaps at 46+ cells), or a StartAction |

State machine (`MoveState`): `Idle`, `Walking(Direction(v) | Target { goal })`, `Rolling(Rolling { start_frame, held, offset, action, sprint, sprinted })`, `Sprinting(SprintRun)`. A sprint starts at `t0 + 15` inside the roll (`Rolling.sprint`) and becomes `Sprinting` when the roll action ends at `t0 + 30`.

Frame order inside `Locomotion::tick`: (1) the entity moves one frame, (2) queued inputs are handled in arrival order and answer directly, (3) timeline events that are due run (roll stats at +16, sprint start +15, roll end +30, `must_sprint` off +35, stance), (4) the frame's `MoveAck` (an unscaled one after a direction change, a scaled one per frame while the sprint stream runs), then events that follow it, (5) a sprint end by key release. A movement started by a reply in frame `F` first moves in `F + 1`, as on the client. `Mover` does the walking (A* path, wall sliding, roll speed profile, sprint scale).

`MovementConfig` holds everything the logic would otherwise hard-code: the skill ids (`SkillIds { move_skill, roll_skill }`), the three sprint buff rows (`SprintConfig.buff_rows`), roll distance, the frame offsets (15, 16, 30, 35), the sprint scale and wire scale (550/180), the sprint stat values and the walking speed. `MovementConfig::new(skills, buff_rows)` fills the official values of the 0.5.5e captures; skill ids and buff rows come from game data when the session builds it. Characters with other speeds use `spawn_player_with_speed`.

### Tests
- Unit tests per state transition in `locomotion.rs`, world behaviour in `world.rs`, grids in `grid.rs`, `Mover` in `movement.rs`.
- **Replay tests** (`crates/poe-game/tests/replay.rs`): input sequences from the official captures go into a `World` and the events are compared per frame (kind, order, field values, frame offsets): WASD start/turn/stop with aim echoes, tap roll while walking, two chained tap rolls (the second waits), sprint hold with release (stream, timeline, scaled and unscaled `MoveAck`, no aim echo), a sprint ended by an attack. Cells are compared with a tolerance (tight for walking, ignored during rolls: the official server's cell during a roll is not reproducible, [gap #10](../protocol/movement.md#gap-list)).
- **Fixtures** (`crates/poe-game/tests/fixtures/*.json`) are derived, never whole captures: only the local player's movement and skill messages (semantic events, no names or chat). Regenerate with `python3 scripts/re/movement_fixtures.py build` (needs `dumps/0.5.5e/captures/official/` and `cargo xtask dll-data`; `show <session> --from F --to F` prints a window). The script's header explains the frame model (an input "arrives" one frame before its first reply).

### UNVERIFIED behaviour
The code marks everything the captures do not show with `// UNVERIFIED (gap #n)`, isolated in small branches so a new capture can change it in one place: click-to-move replies and held-click re-aim (#1), standing roll (#2), a roll cut short by the key going up between +15 and +30 (#3), roll presses during a roll or sprint (queued roll generalised from one capture, sprint into roll) (#4), a zero WASD vector during a sprint (#5), attacks other than ending a sprint (#6), Move input while rolling (#7), stance changes (#8). The movement spec's [gap list](../protocol/movement.md#gap-list) says what capture settles each.

## Wiring it into the instance server (WP2.3)

- **One world per instance.** The instance actor owns the `World` and one tick timer (`tick_ms` from `config/server.toml`). `run_session` stops owning movement: it decodes the client packet into a `MoveInput` (the packet structs are in `poe-proto`), calls `world.input(entity, input)`, and forwards the events the tick returns for its client.
- **Event → packet mapping** is a single function in the instance server (table above), using `poe-proto` structs and opcodes from `modules/protocol/opcodes/<version>/instance.toml`. The simulation header is `(entity, 0, 0, 0)`; sending zeros is valid. Stat keys come from the game data stat table by `MoveStat::id()`; skill ids and the three sprint buff rows come from `poe-data` when `MovementConfig` is built.
- **Frames.** Each `tick()`'s events are sent in order, then the `SimulationTick` marker. The `frame` of a `WorldEvent` is the number of that batch.
- **Terrain.** The world is created with `Arc<dyn GridSource>`: the client's dump `Grid` while the terrain bridge exists, later `poe-world`'s collision grid.
- **Removed with WP2.3:** `TurnCell`, `turn_timing`, `face_cursor`, `cursor_to_grid`, the use of `0x1cb`/`0x1df` for movement; PSY2 (`sync.rs`, `position_sync.rs`) becomes an opt-in drift checker that reads `Locomotion::position()`. Existing `player.rs` tests that still hold are ported to the world; the pure movement ones already live in `movement.rs`.
- **Pause** (`0x1ff`): `World::set_paused`.

## Planned: shared world

- **Instance actor:** one task per instance owning the world: entities with unique IDs, one frame clock and the tick loop. Sessions forward client input to it and receive per-client output.
- **Entity IDs per instance:** `0x0024 PlayerEntity` already tells each client which entity is its own (`u8 local player index, u32 entity id`), so the protocol supports this. InitialState's PlayerState is matched by player id, which still needs checking for several players.
- **Remote players:** send an AddObject for other players' characters (class object type plus a blob as seen by others) when they enter or come into range, remove them when they leave, and forward their actions (StartAction, movement updates, aim, skill use) with their entity ID. We do not know yet how the official server encodes a *remote* player (which components, and whether the Player component carries less). **This needs an official capture with other players nearby** (town is enough, read-only as always).
- **Visibility / interest management:** which entities each client gets (whole instance for small areas, range-based later). Official servers send removes when you move away.
- **Lockstep:** the client runs in lockstep with the server's frames. With several clients, all must follow the instance's frame clock. Each client's inputs arrive on that clock and are echoed to the others in the right frame. The `WorldEvent`s of other entities are the data for that; the open question is how the client treats other entities' actions arriving late.
- **Movement authority:** with the world, the server owns every player's position (the DLL drift checker only compares). Exactness depends on the server-side terrain.
- **Shared instances by default:** `play_character` finds or creates per area. Towns and hideouts are persistent and areas are already found by spec. Official behaviour is per-party instances for zones and shared towns, which needs parties.

Open questions: what a remote player's AddObject blob contains compared with the local one; how the client handles InitialState and PlayerState with several players.
