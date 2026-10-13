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
5. **C→S `0x009a`** after terrain generation: `00 | 25 cf 91 5d | 49 33 ec 54` (probably u8 + two u32 hashes of the generated terrain: tiles and doodads).

Next: what the server must send for the player to appear (player entity, initial state) and what keeps the client in the area.
