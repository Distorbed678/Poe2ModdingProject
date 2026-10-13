# Instance protocol (0.5.5e)

Connection: client to `InstanceDetails.host:port` (ours: `127.0.0.1:6112`).
Opcodes: `protocol/0.5.5e/instance.toml`. Rust types: `crates/poe-proto/src/instance.rs`.
The client's incoming factory is `0x142170380` (switch on opcode − 0x0d); the full opcode → class map (with deserializer addresses) is in `../re/instance_factory_s2c.json` (316 classes). The game-session handler is `0x142121f70` (`ClientInstanceGameSession::ProcessPacket`); in-game packets are handled by `0x1420ee310` (`ClientInstanceAccountSession::ProcessPacket`).

## Opening sequence (verified 2026-10-13)
1. **C→S `0x000f` Hello** (plaintext, 29 bytes):
   ```
   u32 (0) | u16 (0x000e) | 12 bytes (unknown) | u32 ticket | u32 (0) | u8 (4)
   ```
   The ticket is `InstanceDetails` +0x30, which the server redeems.
2. **S→C `0x0011` EnableEncryption** (plaintext): `u8`. The handler (`0x14212203e`) builds a client-role Salsa20 pair from the 64-byte key material in `InstanceDetails` and installs it.
3. From here on, **both directions are Salsa20-encrypted**. The key and IV layout is the same as on login (`poe_net::KeyMaterial`).
4. **S→C `0x001e` AreaInfo** (serializer `0x14211c980`):

   | Offset | Field |
   |---|---|
   | +0x10 | u8 |
   | +0x18 | u32 "Client-Safe Instance ID" |
   | +0x1c | u16 area `WorldAreas.HASH16` |
   | +0x38 | wstring |
   | +0x20 | u32 terrain **seed** |
   | +0x2a, +0x2c, +0x2b, +0x30 | u8 |
   | +0x2d | 3 flag bytes |
   | +0x28, +0x29, +0x34 | u8 |
   | +0x24 | u32 |
   | [flags0 & 8] | u16 |
   | +0x70 | u16 count + u32[] |
   | +0x88 | u16 count + u8[] |
   | +0x58 | u8 count + u8[] |
   | +0xe8 | u16 count + u16[] |
   | +0x100 | u8 count + 8-byte[] |
   | +0x118 | u16 count + u16[] |
   | optional | blocks gated by flags1 bits 1/2/4/8 and flags0 bit 7 |

   The client logs "Client-Safe Instance ID = …" and "Generating level N area "X" with seed S", then generates the terrain.
5. **C→S `0x009a` TerrainHashes** after terrain generation: `u8, u32 tile hash, u32 doodad hash`. Confirmed: the client logs "Tile hash: N" and "Doodad hash: N" with the same values. These are the reference values for server-side terrain generation (M4).

## After the area loads (in progress)
- **S→C `0x0022` LocalPlayers** (`u8, u8 n, (u8 index, wstring character name)[n]`): handler `0x142122dc1`. It assigns indices to the *existing* local-player objects by name (+0x181) and does not create players. Sending it alone is harmless: the client stays on the loading screen.
- **S→C `0x0023` InitialState** (`u8, varuint, u8 flags, u8 n, 0x70-byte entries[n], u8 m, 0x68-byte entries[m]`): handler `0x142122eed` passes the header to the world object (vtable calls) and runs `0x1420edaa0` per local session (that function matches entries by the player id at session player +0x184). **Sent before any player entity exists, it makes the client quit cleanly** (main loop returns, `ExitProcess(0)`). It probably has to follow the player-entity messages.
- **S→C `0x0024`** (u8 at +0x18, u32 at +0x1c): sets the player id (+0x184) on the local player whose index matches.
- Entity and simulation messages come from the second factory `0x142186aa0` (map in `../re/instance_factory2_s2c.json`): 0x64, 0x65, 0xa3, 0x1af–0x1fe, 0x222/0x223, 0x26f/0x270, 0x295/0x296, 0x2a4–0x2a6, 0x2ef, **0x2f4 (tick marker, no body)**, 0x2f5, **0x2f7–0x300**, 0x302, 0x30e, 0x316, 0x318, 0x319, 0x31b, 0x324.
  - They share a header (`0x141f4b680`): `u32, u32, u8, u16`.
  - `0x2f7` = header + `u32` + varuint length + blob (probably serialized entity data).
  - No direct opcode comparison was found for these; dispatch is still to be found.

Next steps:
1. Find where factory-2 packets are dispatched (the simulation event queue) and how the entity blob is parsed (object type and component serialization).
2. Build the player entity, then send 0x24/0x23 in the right order.
'''

## Tools
- `tools/re/dump_factory.py`, `find_switches.py`, `case_strings.py` map factories and handlers.
- The DLL's `crash_log` logs C++ exceptions (type and `what()`).
- `exit_trace` logs `ExitProcess` and `TerminateProcess` with stack refs.
- The instance server's post-area messages can be chosen with `POE_EXPERIMENT=players,initial`.

