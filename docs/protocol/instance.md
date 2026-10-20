# Instance protocol (0.5.5e)

Connection: client to `InstanceDetails.host:port` (ours: `127.0.0.1:6112`).
Opcodes: `modules/protocol/opcodes/0.5.5e/instance.toml`. Rust types: `modules/protocol/poe-proto/src/instance.rs`.
The client's incoming factory is `0x142170380` (switch on opcode − 0x0d); the full opcode → class map (with deserializer addresses) is in `dumps/0.5.5e/ghidra/instance_factory_s2c.json` (316 classes; local and git-ignored, see [dumps/README.md](../../dumps/README.md)). The game-session handler is `0x142121f70` (`ClientInstanceGameSession::ProcessPacket`); in-game packets are handled by `0x1420ee310` (`ClientInstanceAccountSession::ProcessPacket`).

## Opening sequence (verified 2026-10-13)
1. **C→S `0x000f` Hello** (plaintext, 29 bytes):
   ```
   u32 (0) | u16 (0x000e) | 12 bytes (unknown) | u32 ticket | u32 (0) | u8 (4)
   ```
   The ticket is `InstanceDetails` +0x30, which the server redeems.
   Packet capture (2026-10-14) shows this is really two messages: `0x000f` (u32) and `0x000e` (12 bytes: u32 ticket, u32, u8). The server still reads them as one.
2. **S→C `0x0011` EnableEncryption** (plaintext): `u8`. The handler (`0x14212203e`) builds a client-role Salsa20 pair from the 64-byte key material in `InstanceDetails` and installs it.
3. From here on, **both directions are Salsa20-encrypted**. The key and IV layout is the same as on login (`poe_net::KeyMaterial`).
4. **S→C `0x001e` AreaInfo** (serializer `0x14211c980`):

   | Offset | Field |
   |---|---|
   | +0x10 | u8 |
   | +0x18 | u32 "Client-Safe Instance ID" |
   | +0x1c | u16 area `WorldAreas.HASH16` |
   | +0x38 | wstring: the character's **league** (official: "Forbidden Rites") |
   | +0x20 | u32 terrain **seed** |
   | +0x2a, +0x2c, +0x2b, +0x30 | u8 |
   | +0x2d | 3 flag bytes |
   | +0x28, +0x29, +0x34 | u8 |
   | +0x24 | u32 |
   | [flags0 & 8] | u16 |
   | +0x70 | u16 count + big-endian u32[] (`list_70`): object type hashes to preload, sorted (official G1_1: 21, including the AddObject types `0xe113614e`, `0xc8889f16`); sent empty |
   | +0x88 | u16 count + u8[] |
   | +0x58 | u8 count + u8[] |
   | +0xe8 | u16 count + u16[] |
   | +0x100 | u8 count + 8-byte[] |
   | +0x118 | u16 count + u16[] |
   | +0xe8 note | official G1_1: 29 sorted u16 keys (table unknown) |
   | (correction) | the reader `0x14211d1c0` branches on flags0 (+0x2d) bit 3 (`u16` +0x1e, before `list_70`), flags1 bits 0 and 1 (u8-count byte lists +0xa0, +0xb8), bit 2 (`0x14212a8f0`), flags0 bit 7 (two stat records `0x1423baa70` at +0x130/+0x150, then `u8` count + that many records at +0x170; record = varuint count, then `u16` stat key and varint value per stat) and flags1 bit 3 (`0x14212aa00`), in that order. Official G1_1 (flags `94 02 03`): flags1 bit 1 list `[0x11, 0x4f]`, record 1 = {20079 `map_ritual_tower_augment_quantity_+%`, 24795 `area_culture_hash`, 27211 `map_ritual_area_groups_enabled` = 1}, record 2 = {20079, 27211}, no further records. Every official AreaInfo decodes. |
   | optional | flags1 (+0x2e) bit 1: u8 count + u8[] (+0xa0); bit 2: u8 count + u8[] (+0xb8); bit 4: `0x14212a8f0` (+0xd0); flags0 (+0x2d) bit 7: two `0x1423baa70` records + u8 count of them (+0x130/+0x150/+0x170); flags1 bit 8: `0x14212aa00` (+0x188). Reader `0x14211d1c0` |

   Official G1_1 values we do not send yet: +0x2a 1, +0x2c 10, +0x2b 0, +0x30 29, flags `94 02 03`, +0x28 0, +0x29 0, +0x34 56, +0x24 `0x200f76af`, list_58 `[5]`, flags1 bit 2 list `[0x11, 0x4f]` and the flags0 bit 7 records.

   The client logs "Client-Safe Instance ID = …" and "Generating level N area "X" with seed S", then generates the terrain.

   Some seeds fail to generate (`TerrainGenerationException`, e.g. "Mismatching ground types … Cannot place Mud over WideRiverWater" in `G1_1`), and the client then never sends TerrainHashes. The official server generates terrain itself, so it never sends such seeds. Until the server generates terrain for every area, it picks seeds from `instance.known_seeds` (the area/seed pairs from live logs, `scripts/re/clientlog_terrain.py`). Towns and hideouts always use seed 1.
5. **C→S `0x009a` TerrainHashes** after terrain generation: `u8, u32 tile hash, u32 doodad hash`. Confirmed: the client logs "Tile hash: N" and "Doodad hash: N" with the same values. These are the reference values for server-side terrain generation ([../status/terrain.md](../status/terrain.md)).

## After the area loads (verified 2026-10-13: the player is visible in town and in the Riverbank intro)
The server sends, in this order (`InstanceServer::enter_world`):
1. **S→C `0x0022` LocalPlayers** (`u8, u8 n, (u8 index, wstring character name)[n]`): handler `0x142122dc1`. Assigns indices to the existing local-player objects by name (+0x181).
2. **S→C `0x00df` AddInventory**, one per `Inventories.dat` row except `StashInventoryId`:
   - `u8, u8 local player index, u32 inventory id, inventory`.
   - inventory (reader `0x1425aa550`, serializer `0x1425aa290`): `u8 id key, u8 slot (= Inventories row), u8 width, u8 height, u8 flags, [u64 if flags&8], [4×u32 if flags&0x10], varuint item count, items`.
   - Handled in the per-player processor `0x142106a30` (case 0xdf), registry insert `0x141cf1da0`. The registry also maps slot → inventory id (0x79 slots, except 0x1a). The in-game UI resolves every slot through it, and a missing one throws "Invalid inventory id".
   - Grid sizes are not in the game tables; they are configured in `[inventories]` of `config/server.toml`.
3. **S→C `0x0024` PlayerEntity**: `u8, u8 local player index, u32 entity id`. Sets the local player's entity id (+0x184).
4. **S→C `0x02f7` AddObject** for the player entity, then a **`0x02f4`** frame marker (see below and [entities.md](entities.md)). The header's component byte (+0x18) is `0xff`: see "Frame ordering" below.
5. **S→C `0x0023` InitialState** (deserializer `0x142158130`, handler `0x142122eed`):
   - `u8, varuint frame_ms, u8 flags (3 bits), u8 n, CharacterState[n], u8 m, PlayerState[m]`.
   - The handler calls world vfuncs: +0x178 (`frame_ms` → world +0x11c), +0x200 (flags bits 0/1: pause, world +0x868/+0x869), +0x60 (flags bit 2 → world +0xc4), then +0x1f0 (start the world: applies every queued simulation packet, `0x14212fc10`).
   - **`frame_ms` is the length of a simulation frame.** Every frame marker records it, and the world update (`0x1421302c0`) plays the queued frames in real time, giving each frame's entity update `dt = frame_ms / 1000`. With 0, all client time stops: no animations, no fade-ins, so no models are drawn (only labels and lights), and the camera breaks on a fresh client (grey view with a black bar). The server sends `tick_ms` (33).
   - CharacterState (0x70): `u16 n + stash tabs, u16 m + stash tabs (guild?), u8 k + (u32, u32)[k], wstring, 3 flag bytes`. A stash tab (`0x141ce2e20`, 0x68 bytes; local player +0x3a90 / +0x3ac0): `u32, u32, wstring name, astring, u16 index, u8 flags (0x20: +8 bytes, 0x10: +u8, 0x40: +4 bytes), u8, u16 (0xffff), u8 x4`. Official new accounts get four tabs "1".."4" (`[accounts] stash_tabs`).
   - Official values (sent by our server too, meanings partly unknown): flags `7` (pause bits 0/1 and bit 2; the client still unpauses through PauseRequest/SetPause), CharacterState flags byte 0 `0x68` (bits 3/5 → local player +0x3ece/+0x3ed0, bit 6 → account mode +0x3ec4 = 0 instead of 3), PlayerState character byte `0x40` and flags `7`. CharacterState flags byte 1 bit 6 (`[accounts] full_access`) removes the escape menu's "Get Full Access" button.
   - PlayerState (0x68): `u32 player id, u8 (bits 0-5: CharacterState index, 6, 7), u8 flags, u8, u8 rows[3], u16 n + u16[n]`. `rows` are `CharacterShapeshiftBasicSkills.dat` row indices, one per weapon set (local player +0x7980); 0xff (none) crashes the weapon-slot UI.
   - Entries are matched by player id, so InitialState must follow `0x24` and the player entity. Sent first, the client quits (`ExitProcess(0)`).

### Simulation packets (factory 2, `0x142186aa0`; map in `dumps/0.5.5e/ghidra/instance_factory2_s2c.json`)
1. The stream reader `0x1421c5950` tries the registered packet handlers. Factory 2's handler (`0x1421c5810`) builds the packet and passes it to `0x1421340e0`.
2. `0x1421340e0` **queues** the packet in the world's frame queue (world+0x6b0):
   - `0x2f4` (tick marker, no body) closes a frame;
   - `0x2f5` (`u8`) is applied immediately: world vfunc +0x178, the frame length like InitialState's `frame_ms`.
3. Each frame, `0x1421306c0` drains the queue and applies every packet with `0x142134240` (switch on opcode). They share a header (`0x141f4b680`, `u32 entity id, u32, u8, u16`), and fields +0x14/+0x18/+0x1a order the packets inside a frame.

### Frame ordering
The header fields after the entity id schedule a packet within its frame: +0x14 is an entity order key (0 = before entity updates, an entity id = before that entity, `0xffffffff` = after all), +0x18 a component index (`0xff` = after all components), +0x1a a sequence number. During an entity's update the client flushes the packets scheduled at the current position (`0x1421301e0`: world +0x678/+0x67c/+0x67e).

The world start (InitialState) applies every queued packet without advancing the queue cursor. If the player's AddObject is scheduled at (0, 0, 0), a flush nested inside its own apply matches it again. Starting the WashedUp action from the Player reader causes such a flush, so the client adds the player twice ("Replacing awake object with id 1") and crashes. The server therefore sends the player with +0x18 = `0xff`.

### S→C `0x2f7` AddObject (apply case `0x142134d47`)
| Field | Meaning |
|---|---|
| header | `u32 entity id, u32, u8, u16` |
| `u32` | **object type key = MurmurHash2(ASCII metadata path, seed 0)** (`poe_data::hash::object_type_key`), e.g. `Metadata/Characters/Dex/DexFour` = `0xc393f3d5`. The class path is `Characters.Id`. An unknown key throws "Unknown object type serialized by server" |
| `varuint len, blob` | initial state, read by `0x141e71120`: see [entities.md](entities.md) |

If the entity id equals a local player's id (+0x184, set by `0x24`), `0x14210bb30` makes it that player's controlled entity.

## Tools
- `scripts/re/dump_factory.py`, `find_switches.py`, `case_strings.py` map factories and handlers.
- The DLL's `crash_log` logs C++ exceptions (type and `what()`).
- `exit_trace` logs `ExitProcess` and `TerminateProcess` with stack refs.
- Other RE overrides of the instance server (environment variables):
  - `POE_AREA` (login server): the area to send;
  - `POE_SEED`: the terrain seed;
  - `POE_SPAWN=x,y`: the grid spawn cell;
  - `POE_NPC=<metadata path>`: also add that object as entity 2 next to the player (zero blob; e.g. `Metadata/MiscellaneousObjects/Waypoint`).
- The DLL's `dump_object_types` feature also logs, per spawned entity, its components and the blob span each component reader consumes.


## Frame reports and ping replies (2026-10-14)
- **C→S `0x001c` FrameReport**, every 100 ms (500 ms while idle, `0x1421211e4`): `u8 frames` (world +0x680, the frames processed since the last report, capped at 255), then `u32`. All 32 bits of the `u32` are HNC side-channel bits (serializer `0x141f486f0`: value bits 14..31, then 0..13).
- **S→C `0x001d` PingReply**: `u16, u16, u32` (reader `0x141f48c60`). The handler (`0x142122115`) updates the latency estimate (session +0x2260) from the first `u16`. The `u32` carries 32 HNC bits: value bits 9..31, then 0..8, filling bytes least significant bit first.
- **HNC side channel**: both directions carry a byte stream encrypted with a second Salsa20 pair (see [../re/crypto.md](../re/crypto.md#hnc-side-channel)). Received bytes are decrypted and parsed (`0x1421c5360`): 0 = idle, 0xda..0xdd start a message, anything else throws ("HNC Protocol Violated"). The server answers every report with a PingReply whose bits are raw keystream, so they decrypt to idle zero bytes. Zero bits instead crash the client.

## Leaving the game (2026-10-14)
- **Exit to character selection**:
  1. The client sends **C→S `0x0173`** (no body) and waits.
  2. The server issues a resume ticket (`TicketRegistry::issue_resume`) and replies **S→C `0x0174` ExitSession**: `u8, u32 account id, u64 token` (the `u64` raw/little-endian; constructor `0x14215df40`, reader `0x14215e010`).
  3. The client sends its exit telemetry (`0x355`), opens a new login connection and, after the key exchange, sends login **`0x0008` ResumeSession** with the same `u32`/`u64` instead of authenticating (see [login.md](login.md)).
  4. The login server checks the ticket and answers with an AuthenticationResult. The client asks for the character list and shows character selection cleanly.

  If the server only closes the connection, the client still reaches character selection, but logs "An unexpected disconnection occurred". With a zero ticket it opens the login connection and then waits.
- **Exit to log in screen** and **Exit Path of Exile** (escape menu `0x1402c76b0`; logout routine `0x14211f540`, "FourLogOut", with 0 or 2): the client sends **C→S `0x355` ExitReport** (`u8, u16 length, length bytes` of statistics; serializer `0x141f479b0`), then **C→S `0x1b` LeaveGame** (`u8, u8`, both 0), and closes the connection itself. "Exit Path of Exile" then closes the window ("Closing game gracefully").
  - The client keeps its **login connection** open during play (the official log shows later instance hand-offs arriving on it). After leaving the game it shows the log-in screen only if that connection is gone; otherwise it asks for the character list on it and shows character selection.
  - So on `0x1b` the instance server ends the account's login connection (`poe_cluster::LoginSessions`), and the client returns to the log-in screen. Verified 2026-10-14: log-in screen, logging in again, and exit to character selection afterwards all work. Before this, `0x355` was not decoded, so the `0x1b` behind it in the same batch was never seen and "Exit to log in screen" led to character selection.

## Pause (2026-10-14)
- **C→S `0x01ff` PauseRequest**: `u8, u8 paused` (class `0x1434e50f8`). Sent when the escape menu opens or closes in a solo game, and once (`00 00`) after entering an area.
- **S→C `0x01fe` SetPause** (simulation message, reader `0x1421842a0`): simulation header, then `u8` (bit 0 → world +0x868 paused, bit 1 → +0x869). Applied by `0x142134240` through world vfunc +0x200 (`0x14168aaf0`). While +0x868 is set, pausable entities update with dt 0. With both bits set the client also applies its "Pause" audio state and shows "GAME PAUSED". The server answers every request with both bits equal to the request, and pauses its own simulation while paused (see [movement.md](movement.md#pause)).

## Chat channels (2026-10-14)
- **C→S `0x014c` ChatJoinRequest**: `u8, u16 channel (0xffff = any), u8 kind (0 global, 1 trade), u8, u8`. Sent for global and trade chat after entering an area.
- **S→C `0x014d` ChatJoined** (constructor `0x14215cb50`, reader `0x14215cc30`, handler `0x142124bca`): `u8, u16 channel, u8 trade, u8`. Stores the channel in the session (trade +0x2738, global +0x2740) and ends the pending join.
- **S→C `0x0018` ChatMessage** (class `0x14350abb8`, reader `0x142150ad0` → `0x1421a73d0`, writer `0x1421a71d0`): `u8` (1), then the chat line: wstring sender (channel prefix + character name: `#` global, `$` trade; none for local), wstring text (an item link shows as `_`), wstring (empty in captures), 2 raw flag bytes (+0xa3, +0xa4; +0xa4 bit 3 adds `u16` length + bytes), `u8` +0xa1 (0..8: probably the sender's class), `u8` +0xa2 (always 5), `u64` +0x98, `u8` +0xa0, `u8` item count then per item `u32` + the item (`0x1421ecbc0`). All 55 official ones decode (`check_capture`).
- **C→S `0x0016` ChatSend**: `u8` (1), wstring text (as typed, prefixes included), `u8` (0). From our own session (`!where`, `!help`).
- Without it the chat box shows "Timeout while joining {global|trade} chat" (ClientStrings `ChatTimeoutMessage`, formatted by `0x1401b6940`). The server answers a join with the requested channel number (`[chat] default_channel` for "any") and remembers it for routing ([server-admin.md](../guides/server-admin.md#chat)).

## Splitting client messages
Client messages have no length prefix. The instance server decodes the known messages one after another (`InstanceServer::handle_client_message`). Fixed-size messages that are not decoded yet are listed by name in `[client_body_sizes]` of `modules/protocol/opcodes/0.5.5e/instance.toml` and skipped. Anything else ends the batch, and the rest is logged.

## Aim (cursor) and inventories (2026-10-14, from official captures)
- **C→S `0x0062` AimTarget**: `u8 local player, u8 mode`, then mode 2 `i32 x, i32 y` (the cursor's cell), mode 3 `u32` (an entity, probably hovered), modes 0/1 nothing. Sent while the cursor moves once the client reports aims (see [captures.md](../re/captures.md)).
- **The client only sends AimTarget mode 2 while** it is in WASD input mode (`user_input_mode=wasd`; mode 1 is sent first), its window is in the foreground, and **the Player component is in combat stance** (`0x14016ba40`). Official servers set the stance with a `0x02f9` delta on the first Move (and 1 then 0 at entry); our server sets it on the player's first movement or skill.
- **S→C `0x02f9` ComponentDelta**: header (official: entity, `0xffffffff`, 0, 0), varuint length, then `u8` component index (template order, Player = 6) and the component's own delta (component vfunc +0xb0). Player delta (`0x141dd0a10`): `u16` raw little-endian mask; `0x0002` flag words (`u16` count, (`u8` key, `u64`) entries), `0x0400` combat stance `u8`, `0x1000` `u8` (+0x3e7); other bits in `entity::PlayerDelta`'s reader notes.
- **S→C `0x01dc` SetAimTarget** (Actor handler `0x142149640` case `0x1dc`): header, `u8 kind` (0 = cell, 1 = entity), `i32 x, i32 y`. Official servers echo each AimTarget cell one frame later; the client turns the player toward it.
- **AddInventory**: the byte after width/height is a flags byte (official values in `modules/protocol/opcodes/0.5.5e/inventories.tsv`), then the item count. The server sends the official inventory set; the client needs all of it before it starts the world.
- **Local player index** is 1 on official servers (LocalPlayers, PlayerEntity, AddInventory, and the client's own messages).

## Why the player model was not drawn (resolved 2026-10-13)
**Cause:** InitialState's `frame_ms` (world +0x11c, see InitialState above) was 0, so every entity update ran with dt = 0: models stayed invisible while labels and lights showed. A server-sent waypoint behaved the same, so it was not player-specific. How it was found: the frame drain's dt for the game world was always 0 (DLL hook on `0x1421306c0`), dt comes from the frame queue values, and those are recorded from world +0x11c, which InitialState overwrote with 0. With `frame_ms = 33` the player, NPC models and the town render.

Other symptoms that had the same cause:
- With the same server, spawn cell and DLL, the first area entry of a client process showed grey with a black vertical bar; after a restart of the server (the client returns to login) and a re-entry, the town rendered. Delaying the spawn by 8 s did not help.
- Opening the character panel turned a good view grey (camera re-target): the camera's player target was degenerate.
- Right after the player is created the client runs six more area loads (`0x14167b270`), probably world-map scenes.
- A different grey view with a black bar has another cause: the player standing on a non-walkable spot ([../re/terrain/README.md](../re/terrain/README.md#client-side-facts-offline-analysis-2026-10-13)).

State of the player entity in town (DLL `dump_object_types`, 5 s after spawn), kept for reference:
- Awake: entity +0x8c = +0x8d = 0x0c (bit 2: in the wake/update set).
- Updated every simulation frame. The world runs in **frame mode** (world +0x661 = 2): the frame drain `0x1421306c0` applies queued `0x2f7`/`0x2f4` packets, and `0x1421370f0` updates entities inline: component vfunc +0x50 per component, then `0x141e7d7f0(entity + 0x58, dt)`. It does not go through `0x141e71090`. The player gets about 30 updates per second. Tick markers are queued only while world +0x661 and +0x86a are set; each marker records world +0x11c, which `0x2f5` (u8, world vfunc +0x178) sets.
- Not paused: world +0x868 = 0.
- Rendering enabled: Render component (getter `0x140150cb0`, looked up by name) +0x39a = 0. GEAL `EnableRendering` (lambda `0x141923f80`) clears +0x39a, then `0x1416a9810` shows (vfunc +0x60) or hides (+0x58) the AO's scene node, depending on +0x39a and entity +0x8d bit 0. Calling `0x1416a9810` from the DLL changes nothing.
- No NaN or inf floats in any of the 13 components; `0x7fffffff` sentinels only.
- The Animated AO exists. Animation index 0 (`idle_01`, the only entry in `OneRig.amd`) can be started with Animated bit 3 (`POE_ANIM=0`), with no visible effect.
- Not checked: what the AO's skinned mesh and its attachments (`DexFour.ao`: RangerBody/Boots/Gloves/Hair via `AttachedAnimatedObject`) need to load, and whether resources for server-created objects need a preload list (AreaInfo +0x70 is a `u16`-count `u32` vector; in PoE1 it was the preload list).

## Client messages after spawn
About 0.8 s after the spawn the client sends `0x351` (twice), `0x357`, `0x17a`, `0xbc`, `0x2e8`, `0x14c`, `0x13d` and `0x221` (UI and settings; only `0x14c` is answered, see Chat channels). Seen once each elsewhere: `0x23d`. `0x1c` is the 10 Hz frame report ([above](#frame-reports-and-ping-replies-2026-10-14)); `0xbc` carries bytes.

## Client packet classes
`scripts/re/packet_layout.py <opcode…>` lists serializer and deserializer write/read calls for classes whose constructor stores the opcode at +8. Mapping vtables to opcodes over all of `.rdata` gives 306 serializer-shaped vtables and 185 opcodes: `dumps/0.5.5e/ghidra/packet_classes_0.5.5e.json` (local, git-ignored; opcode → vtables, both directions; slot 1 serialize, slot 2 deserialize). More RE scripts: [../re/README.md](../re/README.md).
