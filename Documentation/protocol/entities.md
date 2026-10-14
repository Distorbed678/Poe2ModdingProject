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
| 0 | Positioned | `0x141dd8820` | 17 | `i32 grid x (+0x444), i32 grid y (+0x448), u32 (+0x44c), u16 team (+0x1e0), u8 flags[3]`; a type flag adds a leading 8 bytes for some objects. Flags0: bit 2 `u32, u32` explicit height (+0x440/+0x48c), bit 5 `3 × u32` (+0x474/+0x46c/+0x470), bit 6 `2 × u32` world position (else from the grid), bit 7 → +0x1e5 bit 1. Flags1: bit 0 `3 × u32 + 2 bytes + u32 + 4 bytes`, bit 1 `u8, u8`, bit 2 `4 × u32`, bit 3 `u8, u8`, bit 4 `u32`, bit 6 `u8`, bit 7 `u8`. Flags2: bit 1 sub-reader `0x141ec9ba0` |
| 1 | Stats | `0x142180fe0` | 13 | `varuint n, (varuint stat, varint value)[n]`, player section `u8, u8 (ShapeshiftForms row+1), u8[3] (ShapeshiftForms rows+1), u8 count`, `u32, u8, u8` |
| 2 | Pathfinding | – | 0 | |
| 3 | Buffs | `0x141db4f00` | 2 | `u16 count` |
| 4 | Life | `0x141d8c0e0` | 125 | pools long/short/long/short/long/long (`u32, (u32, i16), u32` + `u32` in the long form), 3 × `(u32, i16)` (count from the type), `(u32, i16)`, `u8 flags` |
| 5 | Animated | `0x141da6540` | 1 | `u8 flags`; bit 0: animated object, as `u32` row (bit 1) or `u32 count + UTF-16LE path` (skipped if equal to the current AO); bit 2 sub-reader `0x141da9e40`; bit 3 `u32 animation, f32, u32` starts an animation (`0x141d439e0`; index into the AO's `.amd`, e.g. `OneRig.amd` has only `idle_01`), then bit 5 `u32, u32`; bit 4 `u32`; bit 6 → +0x18a; bit 7 `5 × u32` (+0x25c..+0x278) |
| 6 | Player | `0x141dcf320` | 39 + 2×name | `u32 len + UTF-16LE name, u8 class (PlayerClass +0x158), u32 (0x141dd11e0), u32 (+0x1d0), u32 (+0x1d4), u16 (+0x200), u16 (+0x202), u16 n + (u8, u64)[n], u16 n + (u8, u64)[n], u8[5], u16, u16 (two Hideouts.dat keys), u8 flags (bit 0: "WashedUp" intro, bit 1, bit 2, bit 3), u8, u8, varuint n + u32[n], u8 n + entries` |
| 7 | PlayerClass | – | 0 | |
| 8 | Inventories | `0x141d82a40` | 2 | `u8 n + (u8 slot, inventory ref 0x1426c3f60)[n], u8 k + u32[k]` |
| 9 | Actor | `0x1416d8090` → `0x142147180` | 48 + skills | partly decoded: 3 × (`u8`, `u8`, `u32` length + UTF-16 string), sub-reader `0x142149390` (`u8` flags, then the [skill list](#actor-skills)), flags (`0x14214c4e0`), 4 × (u8 flags + up to 3 × u32), `u32` (+0x4bc); flags bit 1 wstring, bit 2 `u8` (`0x141e52b90`), bit 3 `u32` (+0x230, then stat 0x77a `actor_scale_+%` is read); if a component's +0x1e0 < 1, `u16 animation, u8, u32` (current animation). Zeros work |

Status: with these sections plus the inventories and InitialState from [instance.md](instance.md) (including its frame length), the player is drawn with the class's model and default stats, in town and in the washed-up intro.

## NPCs (`Metadata/NPC/NPC.ot`)
Serialized components: Positioned, Animated, InteractionAction (`0x141d81300`), NPC (`0x141d966a0`), MinimapIcon, Transitionable (`0x142180310`, `u8`: bits 0-6 state, bit 7), StateMachine (`0x14214cad0`/`0x141dfa320`).

StateMachine: `u8 shared-state count, u8 second count, then per shared state (flag +0x2c in the type) varint value + u8 flags, per second-kind entry u32 + u8`. The counts must equal the number of states the template defines (`define_shared_state`), else "Number of shared states is different on client and server! Delta: N". Example: `Una` defines `visible`, so `[1, 0, 1, 0]`.

Spawning `Una` that way gets through the readers but crashes in the client's script interpreter (`0x1426f2220`, null bytecode). Its states are per-player quest flags, which the server does not send yet. Client-only types (doodads such as `EzomyteCivilianMale01`) are rejected: "Unknown object type serialized by server".

## Actor skills
The Actor component's skill container (`0x14218a850`, read after the sub-reader's flags byte; all integers little-endian):
1. `u8` skill count, then that many skills (`0x14218afb0` → `0x142188bd0`).
2. `u8` count + (`u32` ActiveSkills row, `u32`, `u32`) entries.
3. `u8` count + `u8` entries.
4. `u32` count + (`u32` skill id, `u32 n`, `n` × 4 × `u32`) entries.

One skill:

| Field | Notes |
|---|---|
| `u32` id | skill instance id; use requests (`0x1a5`) and actions (`0x1c8`) refer to it |
| `u8` slot | bits 6–7 and 0–5 select a weapon set / item slot; 0 = the character |
| effect | `u8, u32 GrantedEffectsPerLevel row`, then a stat-set reference (`u8 b, [u8 if b], u32 GrantedEffectStatSetsPerLevel row`) for the base stat set and one per `GrantedEffects.AdditionalStatSets` entry (`0x142683e50`, `0x142683b70`) |
| `u8` flags | bit 0: `u32 parent object, u32` follow |
| `u8` count | supports: (effect, `u32`) each |
| `u32, u32, u32, u8` | |
| stat lists | `varuint count` + (`varuint stat key`, `varint value`) pairs: the base stat set's, then one per additional stat set |
| minion block | only when the granted effect has a `MinionType` |
| `u8` | non-zero: one more stat list |

The table rows are resolved with `poe_data::views::GrantedSkill` (DodgeRollPlayer level 1: GrantedEffectsPerLevel 899, GrantedEffectStatSetsPerLevel 905). Skills that come from the template's `basic_action` list (Move, Interaction, ...) are not listed.
