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

Class and default stats (character panel, Ranger level 1): Str 7, Dex 15, Int 7, Life 28, Mana 34, Evasion 5%, mana regeneration 1.4/s. The object type is the class (`Characters.Id`). Stats come from the merged object template (`Character.ot`) plus the class's base attributes and per-level stats (`poe_game::stats`); the Player component sends the class byte, and Animated the class's `.ao`.

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

# Other players: add, actions, remove, party and trade (WP3.5)

Source: the read-only official town capture `dumps/0.5.5e/captures/official/2026-10-20_21-51-49_336/` (the first zone, then Clearfell Encampment with three other players for about 150 seconds; session row in [../re/captures.md](../re/captures.md#town-capture-with-other-players-2026-10-20)). Character names and chat text never go into the repository: this page calls the other players `<player-1>` (a Ranger-line class, class byte 2, level 10), `<player-2>` (Witch-line, class 7, level 10) and `<player-3>` (class 11, level lower) with entity ids 12000, 12002 and 12009; the local player is 12013. Decoders: `modules/protocol/poe-proto/src/player_blob.rs` (blobs) and `remote.rs` (messages), tests with the real bytes re-encoded under placeholder names (`poe-proto/tests/data/*.hex` are anonymised blobs).

## Findings in one list
- **A remote player is one `AddObject`** (`0x02f7`) with the same header, the same component list and the same blob layout as the local player. There is **no** `LocalPlayers`, `PlayerEntity`, `AddInventory` or `InitialState` `PlayerState` for it (the town `InitialState` is 121 bytes, as in a solo zone). Which entity is "mine" is only decided by the session's `PlayerEntity` id.
- **All other players are in the instance from the first frame.** Nobody joined while the client was there (the capture has no late join), so a join is *inferred* to be the same single `AddObject`, sent when the player enters ([checklist](#what-wp36-must-send)).
- **No interest management for players.** The three players were added at distances of 183, 201 and 210 grid cells from the local player and removed while 46 to 61 cells away. Entities are created for the whole instance; the only range mechanism is the **wake zone** for objects (`0x01c6`/`0x01c7`), which the server toggles per object, with the waking player as order key.
- **A player leaves** with `0x0271`, then `0x01c5` (destroy) + `0x01c7` (sleep) at key 0 and `0x02fa` (remove) at key `0xffffffff`, all in one frame (its pet gets `0x01c5` and `0x02fa` in the same frame). Two of three left right after clicking the waypoint, one of them moving to another area (the social entry shows the new area hash).
- **Remote actions are relayed as the same simulation messages the local player gets**, with the remote entity id, in the frame the server's simulation produces them: `0x64`, `0x1dc`, `0x1c8`, `0x1ca`, `0x1d3`, `0x2e7`, the skill acks `0x1ae`/`0x1af`/`0x1b1`, buffs, component deltas. Only `0x1b2` (release ack of click-to-move) was never seen for others.
- **Party and trade** are separate account-session messages (`0x00b6`..`0x00e0`), not simulation messages; the capture contains an invite, a cancel and a cancelled trade request, but no decline or timeout from the other side ([below](#party-and-trade)).

## Local vs remote `AddObject` blob
Both use the layout of the first half of this page; the decoder is `PlayerBlob::decode`/`encode` (round-trips four real blobs byte for byte). What the official server fills for another player, in blob order (all integers little-endian, stat lists with the packet varint coding):

| Part | Layout (remote player) | Local player in the same capture |
|---|---|---|
| header | `AddObject` header `(entity, 0xffffffff, 0, 0)`, type key = the class's `Metadata/Characters/...` path (`Dex/DexFour`, `Int/IntFourb`, `StrInt/StrIntFourb`) | same |
| `u8 flags` | 2 (wake zone) | 2 |
| link records | `u8 n`, each `u32 key, u8 tag, u8 count, [u8 kind, count × u32]` read by `0x141e71420`/`0x141e7e9d0`. Kind 1 = **entity ids** (forward references are queued), kind `0x0b` = plain integers. Observed: key `0xb350ac3a`, tag 1, kind 1, one id = the object the player last **interacted with** (35 = an NPC, 83 = the stash); `<player-3>`'s pet adds key `0x951dd509` (tag 0, kind `0x0b`, the pet entity 12010) first. A pet's own blob has `0x40f0fc9c` (tag 0, kind 1, its owner) and `0xcd30e41f` (tag 1, kind `0x0b`, 1) | none (`n = 0`) |
| Positioned | `i32 x, i32 y` (grid), `f32` facing (+0x44c), `u16` team 1, `u8[3]` 0 (no optional sections) | same layout |
| Stats | 58-77 base stats (key 1 = `level`: remote 10; also `maximum_life`, resistances, accuracy, `cannot_die` = 1 in town, ...), `u8 0, u8 0, u8[3] = 2, 2, 2`, then **4 stat sets** `(u8 id, stat list)` with ids `0x40`/`0x41` (weapon sets 1/2) and `0x80`/`0x81` (main/off hand weapon; `0x81` empty), then `u32 0, u8 0, u8 1` | 73 base stats, the same four sets |
| Buffs | `u16 n`; each: `u16 id, u8, u16 buff row, u16, u32 duration f32 bits (0x7f800000 = infinite), u32 1.0f, u16, u32 owner entity (the player), u16, u16, u32, u32, varuint, u8, i32 k, k × u32`. A town player has one hidden permanent buff (row `0x4a4`); the list at the end is non-empty for `<player-3>`. Definition-dependent fields (see `BuffAdded`) are assumed absent | 0 buffs at entry |
| Life | 125 bytes, the pool layout above, real values | same |
| Animated | `u8 0x80` + 5 × `u32` f32 bits (+0x25c..+0x278: 0.05..0.2, 0, 0|6.2, facing, facing). No path: the model comes from the class | same |
| Player | `u32 len + UTF-16LE name, u8 class, u32 experience, u32 +0x1d0, u32 +0x1d4` (random-looking ids), `u16, u16`, **two flag-word lists** `u16 n + (u8 key, u64)[n]` (15-37 entries in the first, 0-27 in the second: quest/progress flag words, same format as `PlayerDelta::flag_words`), `u8[5]`, `u16, u16` (hideout keys), `u8 flags`, `u8`, `u8 +0x3e7`, `varuint n + u32[n]`, `u8 0` | experience 898, 15 words in list 1, **flags 0**, `+0x3e7` = 4 |
| Player `flags` | **0x0c**: bit 2 = combat stance (+0x20c), bit 3 set. Level is not in this component: it is the `level` stat | 0 at entry (stance comes with the first move) |
| Inventories | `u8 n` equipment visuals `(u8 slot, u8 id, [id != 0: (u16, u32 MTX row, u8) × 2, u8, u16, u8 × 3])`, then `u8 k + u32[k]` (0). Slots seen: 0, 1 (hands), 6, 8, 9, 10, 11, 14-17 (armour, jewellery), 6-10 entries | 5 entries (0, 1, 8, 10, 11) |
| Actor | 3 × `(u8, u8, u32 len + UTF-16)` **animation sets** (`BowTown`/`WandSpiritShieldTown`/`TalismanTown`, `UnarmedTown`, `default`; the two bytes of the first are the ids of slot 0 and slot 1 above), skill container with **2 skills** (`0x80000040` dodge roll, `0x80000041`), the rest as in a solo blob, `u8 0` | 9 skills (the character's) |

So a remote player costs the server nothing new except real data: name, class, experience and the `level` stat, the equipment visuals (what the viewer's client draws) and the animation sets (the weapon pose; `BowTown` etc. come from the main/off-hand item class). Whether the client needs the stat sets, buffs, flag words or the link record to draw and animate the player is **not verified** (our solo blob omits all of them and works); the official layout is what the decoder produces.

Entity ids: area objects come from a per-instance counter (the town's NPCs have 6-100, players and their children 12000+; the pet of `<player-3>` is 12010 and its `ServerEffect` entities 12014-12016, all with links to the player). A player entity and its children are removed together.

## Lifecycle
1. **Add.** The entry batch (frame 0 of the area) is `LocalPlayers`, `PlayerEntity` (`0x24`), the `AddInventory`s, then the `AddObject`s: the **three remote players first, then the local player, then the 45 NPCs and objects** (in the solo first zone the local player is first). `InitialState` follows in the next frame.
2. **Wake / sleep.** `0x01c6` EntityWake (`entity |= +0x8d 4`, enters the wake zone) has the *waking player's entity* as its second header field (`(object, player, 0, 0)`); `0x01c7` EntitySleep is `(object, 0xffffffff, 0, 0)` for objects. Players are added with flag bit 1 already set and need neither. Monsters woke at about 180 cells from the player in the four samples (not enough to give a rule).
3. **Remove.** In one frame, in stream order: `0x0271` PresenceChanged (`01 01 01 00`, also sent once at area entry), `0x01c5` DestroyEntity (`Entity_Destroy(entity, 0)`: sets the destroyed flag, destroys the children, queues the delete) at key 0 for the player and its pet, `0x01c7` EntitySleep (key 0), then `0x02fa` RemoveObject at key `0xffffffff` (a no-op for an entity already flagged destroyed; **alone it is a complete removal**, as it is for chests and markers). The three removals happened in capture frames 6344, 8030 and 8172 (the area was entered in 4527).
4. **Causes seen.** `<player-3>` and `<player-2>` interacted with the **waypoint** (`StartAction` Interaction, target entity 7, flags `0x490`) 324 and 120 frames earlier; for `<player-2>` the next message after the removal is a social entry (`0x00c6`) with the new area hash `0xe911` (`G1_7`). `<player-1>` left without an interaction we saw (portal or logout). Leaving the area is therefore the only removal cause observed; walking away never removes a player (they stayed visible at 183-210 cells).

## Remote actions (relay rules)
Every remote player's input is turned into the same simulation messages as the local player's and **broadcast to all clients in the frame the server's simulation produces them** (before that frame's `0x02f4`); there is no delay or batching difference for other players. Headers have the same shapes: `0x64` is `(self, self, 9, 0)` for local and remote players alike; `StartAction` `(0, 0, 0)` or `(self, 5, 8)`; `0x1dc`/`0x2e7`/`0x1ae`/`0x1af`/`0x1b1` `(0, 0, n)`; stat updates `(self, 5, n)`. The client applies them to the remote entity like online play (the lockstep restriction in [movement.md](movement.md) is for the local player). Message counts in the capture (local 12013 / the three remote players together):

| Opcode | Message | Local | Remote | Notes |
|---|---|---|---|---|
| `0x0064` | MoveDirectionUpdate | 387 | 742 | one per frame while moving or turning: direction vector (length about 180, scaled while sprinting), the entity's grid cell, `0xf1` |
| `0x01dc` | SetAimTarget | 1153 | 811 | cursor cell of the remote player (the echo of its `0x62`), about one per frame while the cursor moves in combat stance |
| `0x01c8` | StartAction | 168 | 22 | remote: rolls (`0x80000040`, flags `0x410`, plus `(self, 5, 8)` restarts) and **interactions** (`0x2000c266`, flags `0x490` with a target entity: waypoint, stash, NPCs); **no `Move` (`0x20002909`) actions**: the three players walked with WASD, the walk is the `0x64` stream |
| `0x01ca` | ActionEvent | 9 | 22 | with every roll start |
| `0x02e7` | SprintState | 16 | 56 | held 1/0 |
| `0x01d3` | StatsUpdate | 39 | 80 | roll/sprint stats, same sets as for the local player |
| `0x01ae`/`0x01af`/`0x01b1` | UseSkillAck, UpdateSkillTargetAck, SkillInputEndAck | 205/202/10 | 40/12/52 | broadcast with the roll batch |
| `0x01b2` | SkillReleasedAck | 8 | 0 | click-to-move only, not relayed (no other player used it) |
| `0x01eb`/`0x01ec` | BuffAdded/Removed | 15/9 | 10/8 | sprint buffs (rows 2758, 2737, 2853) |
| `0x02f9` | ComponentDelta | 7 | 4 | Player flag words (`0x0002`, 299-362 bytes: `<player-3>` 6020, `<player-2>` 8052, shortly before they left) and an Actor delta |
| `0x01d8` | SkillStatsUpdate | | 12 | ten for one equipment change of `<player-2>` (frame 5201) |

- **A remote roll batch** is the local `r+0` batch: `0x2e7 held=1`, `0x1c8` roll, `0x1ca`, `0x1d3`, `0x1ae`, `0x1d3`, all in one frame, then `0x1b1` + `0x2e7 held=0` when the key is released ([movement.md](movement.md#roll-batch-r0-identical-in-all-45-rolls)).
- **Interaction**: `0x1c8` with skill `0x2000c266`, flags `0x490`, target entity, grid cell of the player and of the target. The object's own state changes come as their own messages.
- **Pets** move with `Move` actions headed `(pet, owner, 9, n)`: the order key is the owner entity, so the pet's action runs right after its owner in the frame.
- **Relation to our ticks.** A frame is whatever precedes the next `0x02f4` in the stream, so a remote entity's messages interleave with ours in one stream: the server decides the frame, the client just plays frames in order. We cannot see the remote player's own input frame; for the local player the reply comes in `f+1` (see movement.md). In the instance actor (WP3.6) the rule is: simulate all entities in one world, and send every entity's events to every session in the frame they are produced.
- **Chat**: lines from other players in the same chat channels arrive as `ChatMessage` (`0x0018`) with the channel prefix and name in the sender (`#` global); our own whisper (`@name text` in `ChatSend`) is echoed as a `ChatMessage` whose sender is `@name`. A `NoticeMessage` (`0x0019`) follows a whisper that could not be delivered.

## Party and trade
Opcodes and decoders: `remote.rs`; all frames below are in the capture's frame numbers; "+n" is the delay of the reply. Names and accounts are anonymised.

**Party invite** (C→S `0x00b6` `PartyInvite { local, wstring name }`):
1. The client sends it for the context menu, the social panel and `/invite` (chat command id `0x16`, sender `Social_SendPartyInvite`).
2. About 8-9 frames later (an asynchronous social service) the server sends, in one frame: `0x00c6` SocialEntry (yourself), `0x00c8` PartyUpdate ×2 (one member: you, role 2), `0x00c6` (the invitee: level, class, area hash, league), `0x00c8` PartyUpdate with **two members** (you role 2, the invitee role 0 = invited). A Player `ComponentDelta` with the flag words follows in the same or the next frame.
3. `PartyUpdate`: `u8 local, u32 party id, u16, u16, u32 party id again, u8 0, u8 1, u8 0, u8 state, u8 member count`, each member `u8 role, wstring character, wstring account, u8 0xff`. The party id is new per party (`0xcfc43`, then `0xcfc55` for the second invite).
4. **Cancel**: C→S `0x00b7` `PartyInviteCancel { local, name }` (chat command id `0x1a`). Reply 2 frames later: `0x00c8` (one member, `state` = 1), `0x00c7` SocialRemove (invitee's account), then `0x00c9` PartyEnded `{ local, party id, reason 4 }`; a new invite creates a new party id.
5. **Not captured:** the invitee accepting or declining, and any timeout. In the capture the first invitation stayed pending until the sender cancelled it about 770 frames (25 s) later, so a decline by the other player is not seen; the opcodes for it are unknown. The server sends `0x00c4` SetPartyInvitationCode (`u8 local, u32 0`; the client logs `InstanceClientSetSelfPartyInvitationSecurityCode = 0`) in every entry batch; nothing else in `AreaInfo`/`InstanceDetails` carries a party or instance-ownership bit that differs from a solo zone (the capture's `AreaInfo` of the town is 197 bytes, the zone's 217).

**`SocialEntry` (`0x00c6`)** = one list entry (the client keeps `0x210`-byte records at session `+0x27d8`): `u8, wstring account, wstring (empty), u64, u8 flags (5 = bits 0 and 2; 4 = yourself in an instance), wstring character, u8 level, u8 class, u16 area hash, u16 area hash 2, wstring league, u8, [u16, u16 if flags bit 0], [u16, u16 if bit 1], u8`. Re-sent when the area changes (player moves from `G1_town` `0x3f70` to `G1_7` `0xe911`).

**Trade** (a request, then cancelled by the sender):
1. C→S `0x00db` `TradeRequest { local, u32 target entity }` (the other player's character entity, here 12002).
2. +1 frame, S→C `0x00d9` `TradeState { local, u32 trade id (1), u32 2, u32 1, u8 0x31, wstring other character, u32 0xffffffff, u16 0x6a }`.
3. The sender cancels 125 frames later: C→S `0x00d2` `TradeCancel { local, u32 trade id, u8 0 }`.
4. One frame later S→C: `0x0019` NoticeMessage (id `0x00e1`), `0x00da` TradeAck (echo of the cancel), `0x00e0` TradeEvent `{ local, kind 1, value 2 }` and `{ 1, 1 }`.
5. Not captured: the other player declining, accepting or a timeout (the other player's client is silent for us until a decline would arrive). Whisper delivery failures use `NoticeMessage` id `0x010e`.

## Opcode table
| Opcode | Dir | Name (`remote.rs` unless noted) | Role |
|---|---|---|---|
| `0x02f7` | S→C | `AddObject` (`instance.rs`), blob `player_blob.rs` | add a player (local or remote), NPC, effect |
| `0x01c5` | S→C | `DestroyEntity` | destroy the entity and its children (key 0) |
| `0x01c6` | S→C | `EntityWake` | object enters the wake zone, key = waking player |
| `0x01c7` | S→C | `EntitySleep` | object leaves the wake zone |
| `0x02fa` | S→C | `RemoveObject` | remove an entity (key `0xffffffff`) |
| `0x0271` | S→C | `PresenceChanged` | `01 01 01 00` at entry and before a player's removal; meaning unknown |
| `0x0064` `0x01c8` `0x01ca` `0x01dc` `0x02e7` `0x01d3` `0x01ae` `0x01af` `0x01b1` `0x01eb` `0x01ec` `0x02f9` `0x01d8` | S→C | `instance.rs`, `skill.rs` | relayed as for the local player |
| `0x00c4` | S→C | `SetPartyInvitationCode` | `u8 local, u32 code`, 0 at every entry |
| `0x00b6` | C→S | `PartyInvite` | invite a character by name |
| `0x00b7` | C→S | `PartyInviteCancel` | cancel the invitation |
| `0x00c6` | S→C | `SocialEntry` | social list entry (self, invitee, area changes) |
| `0x00c7` | S→C | `SocialRemove` | social entry removed (account) |
| `0x00c8` | S→C | `PartyUpdate` | party members and state |
| `0x00c9` | S→C | `PartyEnded` | party id, reason (4 after cancel) |
| `0x00db` | C→S | `TradeRequest` | target entity id |
| `0x00d9` | S→C | `TradeState` | pending trade with the other character |
| `0x00d2` | C→S | `TradeCancel` | cancel by trade id |
| `0x00da` | S→C | `TradeAck` | trade closed |
| `0x00e0` | S→C | `TradeEvent` | trade events (kind, value) |
| `0x0019` | S→C | `NoticeMessage` | system notice id (`0x10e` whisper failed, `0xe1` trade cancelled) |
| `0x0018` / `0x0016` | both | `ChatMessage` / `ChatSend` (`instance.rs`) | chat, whispers |

Decoder gaps found while checking this capture (not fixed here): `SetAimTarget` kind 1 (aiming at an entity) carries a `u32 entity` after `x, y` (2 messages, e.g. the stash), `BuffAdded` flag `0x10` and `AddInventory` items are still unsupported.

## What WP3.6 must send
For client **B** to see player **A** join, move, roll, chat and leave (the first two items are the only new work in the entry batch; everything else reuses the per-entity messages of the local player):
1. **Join (A enters while B is in the instance).** To B: one `AddObject` for A's entity (header `(id, 0xffffffff, 0, 0)`, class type key, a `PlayerBlob` with A's name, class, experience, `level` stat, position, equipment visuals and animation sets, flags 2), and nothing else about A: no `PlayerEntity`, `LocalPlayers`, inventories or `InitialState` entry. Optionally the link record and the stat sets/buffs of the official blob. To A (entering later): the entry batch with `PlayerEntity` for A's own id first, then the `AddObject`s (the other players, then A itself, then the NPCs and objects, as the official town batch does), then `InitialState`. Ids from the instance's counter.
2. **Move.** In the frame the server's world moves A: A's `0x64` (header `(A, A, 9, 0)`) and `0x1dc` (aim echo) to every session, with the same frame numbers as A's own session; plus A's `StatsUpdate`/buffs. Never send B's own messages to A with the wrong entity id.
3. **Roll / sprint.** The whole roll batch for A (`0x2e7`, `0x1c8`, `0x1ca`, `0x1d3`, `0x1ae`, `0x1d3`, later `0x1b1`, `0x2e7`, `0x1ec`/stats) to every session in the frame it is produced (identical to A's own batch).
4. **Interact / skills.** `0x1c8` with the skill and target entity (flags `0x490` for interactions), plus the object's own updates.
5. **Chat.** `ChatMessage` with sender `#A` to every session in the channel; whisper: `ChatMessage` with sender `@name` to both, `NoticeMessage` `0x10e` when the target is not online.
6. **Leave.** Same frame to every remaining session: `PresenceChanged` (`01 01 01 00`), `DestroyEntity` (A and its children) and `EntitySleep` at key 0, `RemoveObject` at key `0xffffffff` (the last alone is enough for the client). Optionally a `SocialEntry` with the new area for friends.
7. **Party and trade** (later): invite, cancel, party and trade state via the account-session messages above; accept, decline and timeout need a new capture with a second human answering.
8. **Frame clock.** One `0x02f4` per tick closes the frame for every session; a session starts receiving a new entity's messages after its `AddObject` in the same or an earlier frame.

Open: the late-join `AddObject` (never captured; the header and blob are assumed to be the same as for entry), whether the client needs the optional blob parts for remote players, party accept/decline/timeout, the meaning of `0x0271`, `0x023d` (`01 55` sent once before a flag-word delta) and the link keys.
