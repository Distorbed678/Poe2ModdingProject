# Instance world

How instances and the simulation are organised today, and the shared-world plan. The wire side is in
[../protocol/instance.md](../protocol/instance.md), movement in [../protocol/movement.md](../protocol/movement.md), the
commands that manage instances in [../guides/server-admin.md](../guides/server-admin.md#instances).

## Current state

- **Instance directory** (`poe_cluster::InstanceDirectory`, in-process `MemoryInstances`): holds every instance (area, seed, forced topology and orientation, the players in it, since when it is empty). Find, create, join, leave, close and idle expiry work; towns and hideouts are persistent. An "instance" is metadata today: area, seed, layout and player count.
- **One simulation per connection.** `run_session` (`servers/instance-server/src/lib.rs`) owns one `Player` with `PLAYER_ENTITY` = 1, runs its own frame clock (`SimulationTick` every `tick_ms`) and sends only that player's AddObject. Two players in the "same" instance share only chat and the player count; they cannot see each other.
- **Live registry** of online players (`live.rs`): who, kick, tell, broadcast, mute.
- **Terrain on the server:** the client generates the terrain itself from the area and seed. The server picks the start cell from the layout and cell grid the DLL exports to `dumps/0.5.5e/terrain/{layouts,grids}/` (`[spawn] layout_dir`, `spawn.rs`): `[spawn.start_tiles.G1_1] point` is a grid cell in the start tile's own orientation (the Riverbank start is on the shore beside the intro tree, inside the `Intro_Tree_01` tile), and the server finds which of the 8 rotations or mirrors the tile was placed with (`PlacedTile`, from the sub-tile index in the layout export, see [client-dll.md](client-dll.md#layout-export-format)) and maps the point onto the map. Server-side generation (`[terrain] generate`, `!terrain`) is for comparison only: [../status/terrain.md](../status/terrain.md). Until it covers every area the server picks seeds the client is known to accept (`instance.known_seeds`, from live logs) or a random one.
- **Saved location:** when a session ends the server stores the character's area, terrain seed and cell (`poe_store::Location`, migration `0005_character_location.sql`); the next login into that area gets the same seed, so the same layout, and spawns on the saved cell (or the nearest walkable cell if it is blocked). Characters that have never been in the game start at the area's start position.
- **No server collision.** Movement is straight lines that follow the client's position reports ([../protocol/movement.md](../protocol/movement.md)).
- **Entities:** only the player (entity 1). NPCs need their StateMachine and quest states ([../protocol/entities.md](../protocol/entities.md)); `POE_NPC=<metadata path>` adds one more object for experiments.

## Planned: shared world

- **Instance actor:** one task per instance owning the world: entities with unique IDs, one frame clock and the tick loop. Sessions forward client input to it and receive per-client output. The current per-session `Player` logic (`player.rs`) moves into the actor as one entity among many.
- **Entity IDs per instance:** `0x0024 PlayerEntity` already tells each client which entity is its own (`u8 local player index, u32 entity id`), so the protocol supports this. InitialState's PlayerState is matched by player id, which still needs checking for several players.
- **Remote players:** send an AddObject for other players' characters (class object type plus a blob as seen by others) when they enter or come into range, remove them when they leave, and forward their actions (StartAction, movement updates, aim, skill use) with their entity ID. We do not know yet how the official server encodes a *remote* player (which components, and whether the Player component carries less). **This needs an official capture with other players nearby** (town is enough, read-only as always).
- **Visibility / interest management:** which entities each client gets (whole instance for small areas, range-based later). Official servers send removes when you move away.
- **Lockstep:** the client runs in lockstep with the server's frames. With several clients, all must follow the instance's frame clock. Each client's inputs arrive on that clock and are echoed to the others in the right frame. Our movement replies are already frame-based; the open question is how the client treats other entities' actions arriving late.
- **Movement authority:** with the position-sync bridge the server trusts each client's reported position and others see whatever that client reports. That is acceptable for friends, not for anti-cheat. Exact server-side movement needs the server-side terrain.
- **Shared instances by default:** `play_character` finds or creates per area. Towns and hideouts are persistent and areas are already found by spec. Official behaviour is per-party instances for zones and shared towns, which needs parties.

Open questions: what a remote player's AddObject blob contains compared with the local one; how the client handles InitialState and PlayerState with several players.
