# Instance world

How instances and the simulation are organised: the instance directory, the instance actor that owns one `World` per instance and serves every session in it (WP3.6), and the `World` in `crates/poe-game`. The wire side is in [../protocol/instance.md](../protocol/instance.md), the movement spec in [../protocol/movement.md](../protocol/movement.md), the commands that manage instances in [../guides/server-admin.md](../guides/server-admin.md#instances).

## Current state (instance server)

- **Instance directory** (`poe_cluster::InstanceDirectory`, in-process `MemoryInstances`): holds every instance (area, seed, forced topology and orientation, the players in it, since when it is empty). Find, create, join, leave, close and idle expiry work; towns and hideouts are persistent. It is metadata and the close signal; the running simulation is the [instance actor](#the-instance-actor-wp36).
- **One actor per instance.** `servers/instance-server/src/actor.rs`: a tokio task owns the instance's `World`, the entity counter and the frame clock; every session of the instance (any number, any account) is a join on it. `run_session` (`lib.rs`) is a pipe between the client's connection and the actor.
- **Live registry** of online players (`live.rs`): who, kick, tell, broadcast, mute.
- **Terrain on the server:** the client generates the terrain itself from the area and seed. The server picks the start cell from the layout and cell grid the DLL exports to `dumps/0.5.5e/terrain/{layouts,grids}/` (`[spawn] layout_dir`, `spawn.rs`; a remote client uploads them instead and `terrain_store.rs` caches them there, [../protocol/side-channel.md](../protocol/side-channel.md#terrain-upload)): `[spawn.start_tiles.G1_1] point` is a grid cell in the start tile's own orientation (the Riverbank start is on the shore beside the intro tree, inside the `Intro_Tree_01` tile), and the server finds which of the 8 rotations or mirrors the tile was placed with (`PlacedTile`, from the sub-tile index in the layout export, see [client-dll.md](client-dll.md#layout-export-format)) and maps the point onto the map. Server-side generation (`[terrain] generate`, `!terrain`) is for comparison only: [../status/terrain.md](../status/terrain.md). Until it covers every area the server picks seeds the client is known to accept (`[dev] known_seeds`, from live logs) or a random one.
- **Saved location:** when a session ends the server stores the character's area, terrain seed and cell (`poe_store::Location`, migration `0005_character_location.sql`); the next login into that area gets the same seed, so the same layout, and spawns on the saved cell (or the nearest walkable cell if it is blocked). Characters that have never been in the game start at the area's start position.
- **Entities:** the players (ids from `[instance] first_entity_id`, 12000). NPCs need their StateMachine and quest states ([../protocol/entities.md](../protocol/entities.md)); `POE_NPC=<metadata path>` adds one more object for experiments.

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
- **Entities.** `Entity { id, next_buff, movement, config }`: a struct with its components as fields (no ECS); `config` is an optional per-entity `MovementConfig` (`spawn_player_with_config`) for class-dependent values. `next_buff` is the entity's buff index allocator, shared by every component that adds buffs. `despawn`, `teleport(id, cell)` (reported as `MoveEvent::Teleport`), `set_grid` (the grid can arrive after the world).
- **Counters.** `StartAction` ids come from one counter for the whole world (the official ones are a server-wide counter shared by all entities, 32768+, `first_action_id`).
- **Events.** `tick()` returns `WorldEvent { frame, entity, event }`, ordered by entity id and within an entity in send order. `Event` is `#[non_exhaustive]` (today only `Movement(MoveEvent)`); new components add variants. `frame` is the batch the message belongs to: the session sends every event of a tick before it sends the `SimulationTick` marker.
- **N players.** Everything is per entity; nothing assumes a single player. The [instance actor](#the-instance-actor-wp36) spawns every player of the instance into one world and relays each event to every session.

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

`MovementConfig` holds everything the logic would otherwise hard-code: the skill ids (`SkillIds { move_skill, roll_skill }`), the three sprint buff rows (`SprintConfig.buff_rows`), roll distance, the frame offsets (15, 16, 30, 35), the sprint scale and the wire length of a sprinting direction (550), the sprint stat values and the walking speed. `MovementConfig::new(skills, buff_rows)` fills the official values of the 0.5.5e captures; skill ids and buff rows come from game data when the session builds it. Characters with other speeds use `spawn_player_with_speed`.

### Tests
- Unit tests per state transition in `locomotion.rs`, world behaviour in `world.rs`, grids in `grid.rs`, `Mover` in `movement.rs`.
- **Replay tests** (`crates/poe-game/tests/replay.rs`): input sequences from the official captures go into a `World` and the events are compared per frame (kind, order, field values, frame offsets): WASD start/turn/stop with aim echoes, tap roll while walking, two chained tap rolls (the second waits), sprint hold with release (stream, timeline, scaled and unscaled `MoveAck`, no aim echo), a sprint ended by an attack, and (2026-10-20 capture) standing tap and hold rolls, rolls pressed during a roll, a walk with a tap roll, WASD released during a sprint, an attack waiting for a tap roll, click-to-move (single, held with re-aim) and the input-mode stance toggle; two (held click with rolls, an attack waiting for the sprint end) are `#[ignore]`d with the gap that explains them. Cells are compared with a tolerance (tight for walking, wide during rolls: the roll simulation follows the client's path only within a few cells, [roll path](../protocol/movement.md#roll-path-and-speed-profile)).
- **Fixtures** (`crates/poe-game/tests/fixtures/*.json`) are derived, never whole captures: only the local player's movement and skill messages (semantic events, no names or chat). Regenerate with `python3 scripts/re/movement_fixtures.py build` (needs `dumps/0.5.5e/captures/official/` and `cargo xtask dll-data`; `show <session> --from F --to F` prints a window). The script's header explains the frame model (an input "arrives" one frame before its first reply).

### UNVERIFIED behaviour
The code marks everything the captures do not show with `// UNVERIFIED (gap #n)`, isolated in small branches so a new capture can change it in one place: what is left after the 2026-10-20 capture: the release threshold at +15..18 (#1), click walks that arrive, clicks on walls, WASD over a click walk, the restart trigger of a held click (#2), roll presses while sprinting and whether a released key drops a queued roll (#3), WASD changes during a roll that started standing, a zero WASD vector in a sprint (#4), the pause menu during a roll or sprint (#5), attacks while walking and in click-to-move mode (#6), stance changes while moving (#7), blocking entities and stuns (#8), entity-targeted attack cells (#10). The movement spec's [gap list](../protocol/movement.md#gap-list) says what capture settles each.

## Wiring the movement into the instance server (WP2.3)

The glue is two pure mappings that do not know sessions or the actor, all in `servers/instance-server/src/`:

| File | Role |
|---|---|
| `movement.rs` | `client_input(name, reader)`: client packet (by name in the opcode table) to `MoveInput`. `Codec::encode(entity, &MoveEvent)`: event to the official messages (`Out { name, bytes, trace }`). Opcodes come from `instance.toml` through `poe-proto`; stat keys through a `StatKeys` closure over the game's `Stats` table; no entity or session state. |
| `player.rs` | the session's side: `OwnedSkill`, `sim_config` (builds the `MovementConfig` from game data: Move skill = `0x20000000 | ActionTypes.Move`, the roll skill = the owned skill whose action type is DodgeRoll with its `dodge_roll_base_travel_distance`, walking speed `Pathfinding.base_speed`, `sprint_movement_speed_+%` of the class, `[movement] sprint_buff_rows` and the sprint coasting keys) and `Player`, the handle to the session's entity in the actor. |
| `sync.rs` | the side-channel sessions (HMAC-authenticated UDP datagrams keyed by session: position reports, debug link, [side-channel.md](../protocol/side-channel.md)) and `DriftCheck`, the drift checker ([movement.md](../protocol/movement.md#drift-checker)). Reports never feed the simulation. |
| `terrain_store.rs` | the terrain cache in `[spawn] layout_dir`: authenticated TCP upload of the client's layout and grid (validation, per tile hash), and the wait that `spawn.rs` uses. |
| `actor.rs`, `appearance.rs`, `lib.rs` | the instance actor, the blobs that describe a character, the session loop ([below](#the-instance-actor-wp36)). |

- **Frames.** An input received between ticks is queued and answered by the next tick (`r+0`). The tick's events are sent in order, then the `SimulationTick` marker. The `frame` of a `WorldEvent` is the number of that batch.
- **Header.** `(entity, 0, 0, 0)` for everything except `0x64` (`(entity, entity, 9, 0)`) and the stance delta (`(entity, 0xffffffff, 0, 0)`); official servers use ordering values in `u1a` that the client does not need.
- **Terrain.** The world is created with `Arc<dyn GridSource>`: the client's dump `Grid` (from the DLL's export or upload; `Open` when it is missing), later `poe-world`'s collision grid. The saved or start cell is moved to the nearest walkable cell first.
- **Dev teleport**: `World::teleport`, reported by the next tick as a `0x64` pair.
- **Tests.** `servers/instance-server/src/replay_tests.rs`: the same fixtures as the `poe-game` replay tests go in as encoded client packets through an actor (one fake session) and the batches come out as encoded packets, compared per frame by message name, order and field values; plus glue tests for the roll batch, acks, stance, click-to-move, pause, teleport and `sim_config`.
- **Removed with WP2.3:** the turn-cell/turn-timing/face-cursor workarounds (and the cursor projection), the use of `0x1cb`/`0x1df` for movement and their three server commands; the side channel's position reports are now only read by the opt-in drift checker (`[dev] drift_check`).

## The instance actor (WP3.6)

One task per instance owns everything that is shared. Sessions never touch the `World`.

```
client A ── run_session ──Cmd──►┐                       ┌──Frame──► run_session ── client A
client B ── run_session ──Cmd──►│ instance actor (World)│──Frame──► run_session ── client B
                                └ one clock, one entity counter, one grid
```

### Ownership and lifetime
- `InstanceServer::actors` (`actor::Actors`) maps the instance id to an `ActorHandle` (an unbounded `mpsc` sender, the player count, the area). `poe-cluster`'s `MemoryInstances` stays the directory (what an instance is, who is in it, when it closes): it has no game types, so the handles live in the instance server, and the actor itself writes the directory's player list (`join`/`leave`), so the lists in `instances`, the overlay and the web API come from the actors.
- **Created by the first join** (`Actors::join`; a join retries when the actor ends in between, and fails when the instance is gone from the directory). The first joiner supplies the world's default `MovementConfig` and the grid; every entity carries its own `MovementConfig` (`World::spawn_player_with_config`), so characters of different classes share one world with their own sprint bonus, roll distance and speed.
- **Ends** when the directory closes the instance (`close`, idle expiry; the sessions left are disconnected, as before), or after `[instance] idle_minutes` without players, for towns and hideouts too (their directory entry stays; the next join starts a fresh actor and world). While nobody is in it the clock does not tick. Ending drops every output channel, which ends the sessions left.
- **Entity ids** come from one counter per instance, starting at `[instance] first_entity_id` (12000: the official counter gives players 12000 and up). Ids are never reused.
- **One clock**: a `tokio` interval of `[instance] tick_ms` calls `World::tick()`; commands that arrived before the tick are drained first, so an input received before the tick is answered by it (`r+0`).
- Nothing assumes one session per account or per machine: a session is a `SessionId` (a process-wide counter), and the live registry is keyed by it, not by the client's port. A duplicate login of an account still ends the old session (`LoginSessions::supersede`, official behaviour).

### Messages
| Session to actor (`actor::Cmd`) | |
|---|---|
| `Join { request, reply }` | `JoinRequest`: session id, character name, wanted spawn cell (the actor moves it to a free walkable cell at least `[spawn] player_spacing` from the players there, `spawn::spread`), speed, `MovementConfig`, grid, `Appearance` (class object type key, the local `AddObject` blob as a function of the final cell, the remote `PlayerBlob`), `want_debug`, the session's output channel. Reply: `Joined { entity, add_objects, players, own }`. |
| `Input { session, input }` | a `MoveInput` for the session's entity (queued, answered by the next tick) |
| `Leave { session, reply }` | the session ends (disconnect, kick, transfer, supersede, error); the reply is the cell the entity stood on |
| `Teleport { session, cell }` | dev/admin |
| `SetPaused { session, paused, reply }` | the escape menu; the reply says whether it was applied |

Local chat is not an actor message: `chat::deliver` sends it through `live.rs` to every session whose registered instance id matches, which is the actor's key.

**Actor to session: encoded packets, not semantic events.** A tick's events are encoded once (`Codec`) and the same bytes go to every session in a `Frame { head, events, trace, own }`, because the official server relays each entity's simulation messages unchanged to every client (a remote player's `0x64` has the same header `(A, A, 9, 0)` as the local one, the acks `0x1ae`/`0x1af`/`0x1b1` are relayed with the remote entity id). Encoding per session would repeat the work and let the copies diverge. What differs per session is small and built where it is known:
- `Frame::head`: lifecycle messages for this session in this frame, before the events: another player's `AddObject`, the removal sequence of one that left, the release of a pause.
- `Joined::add_objects`: the entry batch of the joining session (the other players in the remote form, then itself in the local form).
- `Frame::events` itself: shared by all sessions except where an event of the session's own entity has an owner-only stat (`Stats.IsParty` false, e.g. `stun_threshold_+%_final_while_performing_action` in a roll batch): the owner gets the full encoding, everybody else `Codec::encode_for_others` (the official server leaves those stats out of what it relays; [../protocol/entities.md](../protocol/entities.md#remote-roll-check-wp-c2)). The shared bytes are the others' view.
- `Frame::own`: the session's own pose (frame, cell, exact position, optional debug state) for the drift checker, `where`, the debug link and the saved location.
- `Frame::events` ends with the `SimulationTick` marker, so a paused world or an idle frame is still a frame.

### Join
1. The session finishes its terrain handshake, builds the `Appearance` (`appearance.rs`) and sends `Join`.
2. The actor spawns the entity and answers with `Joined`. To the **joining** session: `LocalPlayers`, `PlayerEntity` (`0x24`, the entity id), the `AddInventory`s, then `add_objects` (the players already there as `PlayerBlob` remote `AddObject`s with their current cell and stance, then itself with the local blob, header `(id, 0xffffffff, 0, 0)`), a `SimulationTick`, and `InitialState` with `PlayerState.player_id` = the entity id. To every **other** session: one `AddObject` for the newcomer (remote form) in the head of the next frame, nothing else (no `PlayerEntity`, inventories or `InitialState`).
3. The local player's own blob is the one that always worked (the class's `.ao` path in Animated). The **remote blob** (`appearance::remote_blob`) is the official remote layout from [entities.md](../protocol/entities.md#local-vs-remote-addobject-blob): flags 2, the class object type key, name, class, the `level` and other base stats, the character's skills, life pools, `Animated` with flags `0x80` (no path), Player flags `0x0c` in combat stance and 0 otherwise, `+0x3e7` = 2, the three animation set names from `[player] animation_sets` (official: weapon dependent; the characters have no equipment yet, so unarmed), the four official shapeshift/trailer bytes. Not filled (the server has no data): equipment visuals, link records, buffs, stat sets, quest flag words. The two-client test ([../guides/hosting.md](../guides/hosting.md#local-two-client-test)) shows the other player correctly without them; only its model looks darker than the local one.
4. A session whose client crashes or leaves right after joining is removed by a `Leave` (and a closed output channel counts as one).

### Relay
Every `WorldEvent` of any entity goes to every session in the instance in the same frame (`0x64`, `0x1dc`, `0x1c8`, `0x1ca`, `0x1d3`, `0x2e7`, the acks, buffs, stance). Interest management is the whole instance (the official town shows no player radius). The movement trace lines in a frame name their entity; a session traces only its own.

### Leave
On a disconnect, kick, transfer, supersede, close or error the session's `Player` is dropped (or `Player::leave` called), which sends `Leave`; the actor despawns the entity and puts into the next frame of every remaining session, in one frame and this order: `PresenceChanged` (`01 01 01 00`), `DestroyEntity` and `EntitySleep` at key 0, `RemoveObject` at key `0xffffffff`. The entity's queued inputs never run. Because the drop does it, every exit of `run_session` is covered, including a duplicate-login supersede.

### Pause
In a **town** the client never pauses (the official town capture has no `PauseRequest`), so `InitialState` flags there are `4`, not `7`: bits 0/1 would start the client's world paused with nothing to release it ([instance.md](../protocol/instance.md)). The server also uses `4` in hideouts (unverified there, but the world never starts paused with it). The escape menu pauses the world only for a **single player** (official solo behaviour): `SetPaused` is applied when the session is the only one in the actor, and the session answers its client with `SetPause(paused)`. With other players the request is refused, the world keeps running and the client gets `SetPause(false)` (the official multiplayer reply is not captured: UNVERIFIED). A pause is released when a second player joins (the paused session gets `SetPause(false)` in its next frame) or when the pausing player leaves.

### Admin and the web API
- `instances` (chat command, overlay, `AdminControl`) lists the directory's players, which only the actor writes, with the actor's count.
- `close` closes the instance in the directory: its actor ends and its sessions are disconnected. `transfer`/`join` hand the session a ticket for another instance and end the session (a `Leave` first); the web API's `transfer_player` kicks the live session after handing off, which leaves the same way.

### Tests (`actor_tests.rs`)
Fake sessions (an output channel and the decoded packets) against one `Actor` driven synchronously (`handle`, `tick`) give exact frames; a few `tokio` tests with paused time run the real task. Covered: entity ids from the counter (per instance), the entry batch (others in remote form with their current cell and stance, then itself), one `AddObject` for the newcomer, a walk reaching the other session in the same frame with the header `(A, A, 9, 0)` and identical bytes, a roll batch relayed whole, action ids from one counter, trace lines naming their entity, the leave sequence and the removed entity, a closed channel as a leave, pause for one player only, local chat to both sessions, per-character speed in a shared world, the real task on one clock, drop and `leave` returning the cell, a **supersede** (real `MemorySessions` and `Stops`) and a kick ending in the leave sequence, close ending the actor and its sessions, and the idle end followed by a fresh actor.

### Party and trade (WP C3)
`social.rs`: `Social` (inside `Live`, `live.social`) keeps the open parties and trades and one queue of `SocialOut` messages per session (`register` at session start with the character's `Person`: names, level, class, area hash, instance and entity id; `unregister` at the end). The session loop writes the queue to its client like a chat line (`run_session`, `Some(out) = social_in.recv()`), outside the actor's frames, so it works between instances. `handle_client_message` routes `PartyInvite`, `PartyInviteCancel`, `TradeRequest` and `TradeCancel` to `Social` (a refusal comes back as a server chat line) and **only logs** `PartyAccept` and `PartyDecline` (gate G-PARTY). A trade request sends the two trade-window `AddInventory` (slot `0x79`) before the `TradeState` (without them the client crashes), and a cancel for a closed trade is ignored silently. A session that ends cancels its open parties and trades. Messages and what is verified: [../protocol/entities.md](../protocol/entities.md#party-and-trade-flows-wp-c3). Tests (`social.rs`): the inviter's batch and the cancel against the official bytes (anonymised), the invitee's and target's batches, a trade cancelled by either side, a leaving player ending what it had open, and refusals.

## Not built yet
- **Lockstep with several clients:** all clients follow the instance's frame clock. Two real clients on a LAN worked (relayed walk and roll, chat, leave); how the client treats other entities' actions arriving late over a bad link is not tested.
- **Movement authority:** the server owns every player's position (the DLL drift checker only compares); exactness depends on the server-side terrain.
- **Shared instances by default:** `play_character` finds or creates per area. Towns and hideouts are persistent and areas are already found by spec. Official behaviour is per-party instances for zones and shared towns, which needs parties.
- **Late join, remote blob extras and parties:** a late join was never captured (assumed to be the same `AddObject`); whether the client needs the optional parts of the official remote blob (stat sets, buffs, flag words, equipment visuals) and party/trade accept/decline/timeout (gate G-PARTY; invite and cancel are built, WP C3) are open ([../protocol/entities.md](../protocol/entities.md)). `InitialState` stays local-only (121 bytes in a town with three other players).
- **Interest ranges** (wake zone toggling for objects) when NPCs exist.
