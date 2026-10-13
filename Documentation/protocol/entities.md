# Entity initial state (0.5.5e)

The blob in `AddObject` (S→C `0x2f7`) is read by `0x141e71120(entity, stream)`:

```
u8  flags          bit 0 -> entity +0x8d |= 1, bit 1 -> |= 4
u8  n              then n × (u32, u8, data via 0x141e7e9d0)   (0x141e71420)
... component sections, in object-template order (vtable +0x98 of each)
u8  m              then m × (u16 LE key, data via 0x141e7dbe0)
```

- Integers inside the blob are **little-endian** (copied raw).
- Compact integers use the packet varint coding (`0x1419970a0` unsigned, `0x141e25d50` signed).
- Only the first *k* components are serialized, where *k* = object type +0x28 → +0x78. Client-only components (Render, Targetable, BuffOrbs) follow them.
- The client does not check that the blob is fully consumed.

## Player character (`Metadata/Characters/Character.ot`)
Components in order, with their readers. Sizes are measured with every flag and count at zero, using the DLL reader trace. The encoder is `poe_proto::entity`.

| # | Component | Reader | Bytes | Layout (zero flags) |
|---|---|---|---|---|
| 0 | Positioned | `0x141dd8820` | 17 | `i32 grid x, i32 grid y, u32, u16, u8 flags[3]` (flags select optional sections; a type flag adds a leading 8 bytes for some objects) |
| 1 | Stats | `0x142180fe0` | 13 | `varuint n, (varuint stat, varint value)[n]`, player section `u8, u8 (ShapeshiftForms row+1), u8[3] (ShapeshiftForms rows+1), u8 count`, `u32, u8, u8` |
| 2 | Pathfinding | – | 0 | |
| 3 | Buffs | `0x141db4f00` | 2 | `u16 count` |
| 4 | Life | `0x141d8c0e0` | 125 | pools long/short/long/short/long/long (`u32, (u32, i16), u32` + `u32` in the long form), 3 × `(u32, i16)` (count from the type), `(u32, i16)`, `u8 flags` |
| 5 | Animated | `0x141da6540` | 1 | `u8 flags` |
| 6 | Player | `0x141dcf320` | 39 + 2×name | `u32 len + UTF-16LE name, u8, u32, u32, u32, u16, u16, u16 n + (u8, u64)[n], u16 n + (u8, u64)[n], u8[5], u16, u16, u8 flags (bit 0: "WashedUp" intro), u8, u8, varuint n + u32[n], u8 n + entries` |
| 7 | PlayerClass | – | 0 | |
| 8 | Inventories | `0x141d82a40` | 2 | `u8 n + (u8 slot, inventory ref 0x1426c3f60)[n], u8 k + u32[k]` |
| 9 | Actor | `0x1416d8090` → `0x142147180` | 40–48 | not decoded; zeros work (length depends on state) |

Status: with these sections (name, life and mana from `Characters.BaseMaxLife/BaseMaxMana`), plus the inventories and InitialState from [instance.md](instance.md), the client reaches the game screen with HUD. The player still spawns at grid (0, 0), outside the map.
