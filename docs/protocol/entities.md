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
- **All other players are in the instance from the first frame.** Nobody joined while the client was there (the capture has no late join), so a join is *inferred* to be the same single `AddObject`, sent when the player enters ([checklist](#what-the-instance-actor-sends)).
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

## Party and trade flows (WP C3)
Built in `modules/server/servers/instance-server/src/social.rs` (`Social`, held by `Live`; architecture: [../architecture/instance-world.md](../architecture/instance-world.md#party-and-trade-wp-c3)). The messages are account-session messages like chat: a session writes them to its client the moment they arrive, outside the actor's frames, and they work across instances (a party invite is by character name; a trade target is an entity id in the requester's instance). Names below are the placeholders of the tests.

**What is implemented**
| Step | Client sends | Server sends to the requester A | Server sends to B | Source |
|---|---|---|---|---|
| Party invite A to B | `0x00b6` `PartyInvite` (name) | `SocialEntry` (A), `PartyUpdate` ×2 (A alone), `SocialEntry` (B), `PartyUpdate` (A leader, B invited) | `SocialEntry` (A), `PartyUpdate` (same list, B's index 1) | A: official bytes (the tests compare them). B: derived, **verified in game** |
| Party cancel by A | `0x00b7` `PartyInviteCancel` (name) | `PartyUpdate` (`state` 1, the remaining members), `SocialRemove` (B's account), `PartyEnded` reason 4 when nobody is left | `PartyEnded` reason 4 | A: official bytes. B: **verified in game** (the popup goes away) |
| Trade request A to B | `0x00db` `TradeRequest` (B's entity) | `AddInventory` ×2 (trade window, slot `0x79`), `TradeState` (`unk_c` `0x31`, B's name) | `SocialEntry` (A), `AddInventory` ×2, `TradeState` (`unk_c` `0x32`, A's name) | A: official bytes. B: derived, **verified in game** |
| Trade cancel by either | `0x00d2` `TradeCancel` (trade id) | the canceller: `NoticeMessage` `0xe1`, `TradeAck`, `TradeEvent` (1, 2), (1, 1) | the other: `TradeAck`, the two events | the canceller: official bytes; the other: verified in game (popup closes; its client answers with its own `TradeCancel`, ignored) |

A session that ends (or is replaced) cancels what it had open: an invitee that leaves is a cancel for the inviter, a leader that leaves ends every invitation, a trade partner that leaves closes the trade. Party ids and trade ids count up from `Social::starting_at` (the tests start at the capture's `0xcfc43` and 1). Refusals (not online, yourself, already invited or trading) come back as a server chat line.

**How the invitee's side was derived (static RE, `docs/re/addresses-0.5.5e.md`)**
- `PartyUpdate` is handled by `Session_HandleAccountMessage` (`0x142106a30`) case `0xc8`. The member roles map to a state: role 2 = leader, 1 = member, 0 = **invited**, other = none (`0x1421063f0`). The client takes its own state from `members[unk_0]` (`unk_0` = the local player's index in the list, `0xff` = none; the capture's inviter is at 0). A new `PartyUpdate` whose own state is "invited" and whose party id is not the current party creates an **invitation record** (`Social_AddPartyInvitation` `0x14210b630`, keyed by party id, kept in a list at session `+0x7620`), which raises the UI event "party invitation received" (`FourSocialPartyReceive`, listener `0x140feb9c0` builds the popup). The record carries the leader's social entry, looked up by name in the session's social list (`+0x27d8`, case-insensitive), so the server sends the leader's `SocialEntry` first; without character data on the entry no popup is shown (`0x14210b740`).
- When the own state becomes "member" (role 1) the record is removed and `FourSocialPartyAccept` is raised; `PartyEnded` for an id that is not the current party removes the record (`0xc9`: the id is compared with session `+0x2278`, the party the player is in). Whether an open popup closes by itself is not known.
- `TradeState` (`0xd9`) copies its fields into the session's trade record (`+0x7a28`) and raises an event; `TradeInvite_OnTradeState` (`0x1402c3cb0`) opens the popup string `TradeInviteFrom;` only if the trade id is not 0, record flag byte (`unk_c`) has **bit 1 set and bit 0 clear**, and the other character's `SocialEntry` is in the social list (looked up by the name in the state). The requester's `0x31` has bit 0 set, so the client does not show a popup for its own request; the target's value `0x32` is verified in game. `TradeAck` (`0xda`) clears the record and raises its event; `0xe0` (the two `TradeEvent`s) is the inventory handler of the client (clear the trade-window inventories 2 and 1).

**Gate G-PARTY: accept and decline are not implemented**
- Party **accept** and **decline** are *decoded from the client's code but never seen on the wire*: C→S `0x00b8` `PartyAccept` and `0x00b9` `PartyDecline`, both `u8 local, u32 party id` (the invite popup's callbacks `PartyInvitePopup_Accept` `0x140c9ef70` and `PartyInvitePopup_Decline` `0x140c9eee0` -> `Social_SendPartyDecline` `0x142110ab0`). They are registered in `instance.toml` and `poe-proto` (`PartyAccept`, `PartyDecline`), the server **only logs** them: `party answer not implemented: needs capture G-PARTY`. The server-side effect of an accept is probably a `PartyUpdate` with the invitee's role 1 (member) for everyone, which the client turns into `FourSocialPartyAccept`; nothing else is known (the `PartyUpdate` `unk_1`, `unk_2`, `state`, what a member's `tail` means, the party list for 3+ members, the reason code for "declined" or "timed out" in `PartyEnded`).
- Trade **accept** and **decline**: not found; the in-game check showed that the popup's 15 s timeout is a client `TradeCancel`, which a decline most likely is too. The trade popup (`TradeInvite_ShowPopup` `0x1402c3fb0`) returns a bool through a promise object (`0x1402c3c30`) and the sender was not traced; a decline may well be `TradeCancel` and an accept a `TradeRequest` back to the requester (PoE1 style), but nothing is known, so the server logs those client messages as unknown (`received` + hex dump, batch skipped) if they are anything else.
- **What the official capture must contain** (a second human answering; the user's client is the one that logs; read-only on the official server, [the capture how-to](../re/captures.md)): play with a friend in the same town and do these in this order, a few seconds apart, **once with the user as the inviter and once with the user as the invitee**:
  1. Friend invites the user to a party; the user **accepts** (popup button). Then leave the party (the social panel or `/leave`), and kick/let the friend leave once if easy.
  2. Friend invites the user; the user **declines**.
  3. User invites the friend; the friend **accepts**, then leaves; user invites again and the friend **declines**.
  4. User invites the friend and nobody answers for about two minutes (timeout; note the time).
  5. Friend requests a trade with the user; the user **accepts** (the trade window opens: put one item in and cancel at the end, or finish the trade). Friend requests again; the user **declines**. User requests; friend accepts and declines once each. One request left unanswered until it times out.
  6. Write down the order and the rough minute of each step, and the (fake) names in use; names and chat are anonymised before anything is committed.
  The capture then shows: the **invitee's** `PartyUpdate`/`SocialEntry`/`PartyEnded` batches (the invitee's side of invite, cancel and trade request is verified with two clients, see the in-game check), the accept/decline opcodes of both flows, the member list with 2+ joined members, and the timeout and decline reasons.

**In-game check (run 2026-10-21, `scripts/test-mp.sh`, two clients in `G1_town`)**
Input: chat text injection (`keys 1 0x0d wait:500 'text:/invite Mp2Ranger' wait:500 0x0d`) worked, so did mouse clicks with `inmove:x,y` + `inclick:x,y` (popup buttons, the menu, the social panel). The Agents rules do not block these, only the W key. Chat commands that send the messages: `/invite <name>` (`0x00b6`), `/kick <name>` (the cancel `0x00b7`), `/tradewith <name>` (`0x00db`); the trade request's Cancel button on the "Waiting for player to accept trade request..." box sends `0x00d2`. The social panel is the menu button (259,652) then "Social Screen" (308,535), tab "Current Party" (230,117).
| Step | Result | Evidence (F8, prefix 1 = client 1, prefix 2 = client 2) |
|---|---|---|
| 1 Party invite | **Works.** Client 2 shows the popup "MP1 sent you a party invite" with Mp1Ranger, level 1 Ranger, Clearfell Encampment, Accept/Decline. Client 1 shows "Mp2Ranger has been invited to your party." and its Current Party tab lists Mp1Ranger as leader and "1 Invitation Sent" with Mp2Ranger and a Cancel button. **Party derivation (role 0, `unk_0` = the invitee's index) verified.** | client 2 `screenshot-0080`, client 1 `screenshot-0077`, panel `screenshot-0087` |
| 2 Cancel (`/kick`) | **Works.** Client 1: "Mp2Ranger was kicked from the party." and "The party was disbanded."; client 2's popup goes away by itself (`PartyEnded` reason 4 is enough: verified). | client 1 `screenshot-0078`, client 2 `screenshot-0081` |
| 3 Trade request | **Works after a fix (below).** Client 1 shows "Waiting for player to accept trade request..." with Cancel, client 2 the popup "MP1 sent you a trade request" (`TradeInviteFrom`). **`TradeState.unk_c = 0x32` verified.** | client 1 `screenshot-0079`, client 2 `screenshot-0082` (second run) |
| 3b Trade cancel by the requester (Cancel button) | **Works.** Both popups close, client 1 prints "Trade cancelled.". | client 1 `screenshot-0081`, client 2 `screenshot-0084` |
| 4 Accept / decline | Party **Accept** (click on the popup): the popup closes in the client, the server logs `party answer not implemented: needs capture G-PARTY ... party=1 accept=true` (as planned; the invitation stays open on the server and the panel keeps "1 Invitation Sent"). The trade popup's buttons were not clicked, but see the next two lines. | server log |
| 4b Trade popup timeout (new) | The invitee's client sends **`TradeCancel` (`0x00d2`, trade id) by itself about 15.25 s after the request** if nobody answers (no input to client 2 at that time; seen twice: trade 1 and 3). A trade decline is therefore very probably the same `TradeCancel`. | server log `trade cancel from=Mp2Ranger trade=1`/`3`, packet log |
| 4c Stale cancel (new) | When the requester cancels, the invitee's client closes its popup on the `TradeAck` and answers with its own `TradeCancel` 0.25 s later (trade 2). The server had said "There is no such trade." in the chat (`screenshot-0084`, client 2). Fixed: a cancel for a closed trade is ignored silently. | `screenshot-0084`, unit test |
| 5 Session leaves | **Works.** Client 2 closed (`close`) with the invitation pending: client 1 shows "Mp2Ranger was kicked from the party." / "The party was disbanded." and its panel falls back to "Create Party". | client 1 `screenshot-0088` |

**Crash found and fixed: "Invalid inventory id" on both clients at the first trade request.** The first run (server before the fix) sent the trade request as `TradeState` (`unk_c` `0x31` to the requester; `SocialEntry` + `0x32` to the target). Both clients threw a C++ `runtime_error` "Invalid inventory id" in the same millisecond and crashed (crash dumps `PoE-1792609716-336.dmp` in both prefixes; the client process stays alive without drawing, F8 no longer works; hook log `1792609716.130 [crash] C++ exception ... "Invalid inventory id"`). Cause: the constructor of the inventory-grid widget (`FUN_14042daa0`, the only thrower of that string) looks its inventory up in the session's inventory registry by `Inventories` slot, and the trade window needs slot `0x79` (121) which an entry batch does not contain (slots 0..120 only). The official server creates it right before the `TradeState`: in the 2026-10-20 capture (`captures/official/2026-10-20_21-51-49_336`, 21:56:17.021) two `AddInventory` precede the `0xd9`, `01 01 00000001 49 79 0c 05 40 00` and the same with id 2 (`InventoryIdKey` `0x49`, slot `0x79`, 12x5, flags `0x40`, empty; ids 1 and 2 are the ones the `TradeEvent`s (1, 2) and (1, 1) clear after a cancel). The second byte is the local player index (1); the entry batch has `unk_10` 1 and index 0 for the same layout. Fix in `social.rs`: `trade_request` now sends these two `AddInventory` first, to the requester and to the target (test `a_trade_request_reaches_both_and_either_side_cancels` compares the capture's bytes). Open: whether the inventory ids follow the trade id (the capture only has trade 1, so ids 1 and 2 are constants here; later trades reuse them, which worked for trades 2 and 3 in game).


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
| `0x00b8` `0x00b9` | C→S | `PartyAccept`, `PartyDecline` | answer an invitation: `u8 local, u32 party id` (from the client's code, never captured: [G-PARTY](#party-and-trade-flows-wp-c3)) |
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

## What the instance actor sends
What other players need to see player **A** (the instance actor implements it, see [Implementation notes](#implementation-notes-wp36)). For client **B** to see player **A** join, move, roll, chat and leave (the first two items are the only new work in the entry batch; everything else reuses the per-entity messages of the local player):
1. **Join (A enters while B is in the instance).** To B: one `AddObject` for A's entity (header `(id, 0xffffffff, 0, 0)`, class type key, a `PlayerBlob` with A's name, class, experience, `level` stat, position, equipment visuals and animation sets, flags 2), and nothing else about A: no `PlayerEntity`, `LocalPlayers`, inventories or `InitialState` entry. Optionally the link record and the stat sets/buffs of the official blob. To A (entering later): the entry batch with `PlayerEntity` for A's own id first, then the `AddObject`s (the other players, then A itself, then the NPCs and objects, as the official town batch does), then `InitialState`. Ids from the instance's counter.
2. **Move.** In the frame the server's world moves A: A's `0x64` (header `(A, A, 9, 0)`) and `0x1dc` (aim echo) to every session, with the same frame numbers as A's own session; plus A's `StatsUpdate`/buffs. Never send B's own messages to A with the wrong entity id.
3. **Roll / sprint.** The whole roll batch for A (`0x2e7`, `0x1c8`, `0x1ca`, `0x1d3`, `0x1ae`, `0x1d3`, later `0x1b1`, `0x2e7`, `0x1ec`/stats) to every session in the frame it is produced (identical to A's own batch).
4. **Interact / skills.** `0x1c8` with the skill and target entity (flags `0x490` for interactions), plus the object's own updates.
5. **Chat.** `ChatMessage` with sender `#A` to every session in the channel; whisper: `ChatMessage` with sender `@name` to both, `NoticeMessage` `0x10e` when the target is not online.
6. **Leave.** Same frame to every remaining session: `PresenceChanged` (`01 01 01 00`), `DestroyEntity` (A and its children) and `EntitySleep` at key 0, `RemoveObject` at key `0xffffffff` (the last alone is enough for the client). Optionally a `SocialEntry` with the new area for friends.
7. **Party and trade** (WP C3, [below](#party-and-trade-flows-wp-c3)): invite, cancel, party and trade state via the account-session messages above; accept, decline and timeout need a new capture with a second human answering (gate G-PARTY).
8. **Frame clock.** One `0x02f4` per tick closes the frame for every session; a session starts receiving a new entity's messages after its `AddObject` in the same or an earlier frame.

Open: the late-join `AddObject` (never captured; the header and blob are assumed to be the same as for entry), whether the client needs the optional blob parts for remote players, party accept/decline/timeout, the meaning of `0x0271`, `0x023d` (`01 55` sent once before a flag-word delta) and the link keys.

## Implementation notes (WP3.6)
Built in `modules/server/servers/instance-server/src/actor.rs` and `appearance.rs` ([../architecture/instance-world.md](../architecture/instance-world.md#the-instance-actor-wp36)); the checklist above is implemented as follows.
- **Join.** The joining session gets `LocalPlayers`, `PlayerEntity`, the inventories, then one `AddObject` per player already present (remote blob, current cell and stance, header `(id, 0xffffffff, 0, 0)`), then its own `AddObject` (the local blob that always worked), a `0x2f4` and `InitialState`. Everyone else gets one remote `AddObject` for the newcomer in the next frame. Entity ids come from the instance counter (`[instance] first_entity_id`, 12000).
- **Spawn cell (WP C2).** The actor, not the session, finalises the cell: the session's spawn (saved cell or area start) is kept when no player is within `[spawn] player_spacing` cells (default 8) of it, else the joiner stands on the nearest walkable cell at least that far from every player in the instance (`spawn::spread`: square rings around the wanted cell on the walkability grid, ties by y then x, so the same input gives the same cell; no grid means every cell is walkable). The joiner's local blob (`Appearance::local_blob` is a function of the cell) and every remote blob use that final cell. UNVERIFIED: only walkability is checked, so on a map with thin walls the free cell can lie behind one; with 8 cells (about a third of a tile) that is rare.
- **`InitialState` `PlayerState.player_id` is the player's entity id** (the id of `0x24`), not the local index: the capture has `0x5b` in both in the first zone and 12013 in the town. The server used 1 for both until WP3.6; with another entity id the client crashed 0.2 s after entry (null read in a component constructor, `0x1404982e0`), because the local player's state was not found.
- **Remote blob** from `PlayerBlob` (`appearance.rs`, field by field in [Remote player look](#remote-player-look-wp-c2)): the class type key, `level` and base stats plus the derived town conditions, the weapon set stat sets `0x40`/`0x41`/`0x81`, the official Life shape, the character's skills (Actor tail as in the solo blob), `Animated` flags `0x80` without a path (the local blob keeps the `.ao` path), Player flags `0x08` plus the stance bit `0x04` while in combat stance, `+0x3e7` = 2, animation sets from `[player] animation_sets`, official shapeshift/trailer bytes; not filled (no data for them): equipment visuals, links, buffs, quest flag words.
- **Relay.** Every entity's events are encoded and sent to every session in the tick's batch, before the `0x2f4`, including the acks (`0x1ae`, `0x1af`, `0x1b1`, `0x1b2`), buffs and stance; interest management is the whole instance. The entity's own session gets the full batch, everybody else the **public view**: `StatsUpdate`s keep only the stats `Stats.IsParty` marks (see [the roll check](#remote-roll-check-wp-c2)); one shared buffer unless the frame has an owner-only stat for that session.
- **Leave.** `0x0271`, `0x01c5` and `0x01c7` at key 0, `0x02fa` at key `0xffffffff`, in one frame to every remaining session.
- **Pause** is for a lone player only; with company the request is refused and the client is told "not paused" (UNVERIFIED reply).

## Remote player look (WP C2)
Why the remote player looked darker and orange in the two-client test, and what the remote blob now contains. Evidence: the decoded blobs of the town capture (`dumps/0.5.5e/captures/official/2026-10-20_21-51-49_336`, entities 12000, 12002, 12009 remote, 12013 and 91 local) against what `remote_blob` produced until WP C2. The anonymised official blobs are `modules/protocol/poe-proto/tests/data/*.hex`; the test `the_remote_blob_has_the_official_parts_and_flags` (`appearance.rs`) checks field presence, flags and constants against them.

**The cause is not proven.** No bisect was run in the game yet (the client was in use). What the diff shows:

| Part | Official (all 3 remote blobs; the local ones where said) | Ours before WP C2 | Now |
|---|---|---|---|
| Life pools, second field `+0x30` (`Pool::current`) | **0 in every pool of all 5 blobs**, local included, while the first field (`+0x38`) is the maximum | `+0x30` = the full value (`Pool::full`) | 0 (remote only) |
| Life `flags` | 1 in all 5 blobs | 0 | 1 (remote only) |
| Life `unk_48` of the life, `+0x240` and last long pool | the energy shield recharge delay as f32 seconds (4.0, 2.0 for the Witch line, whose `energy_shield_delay_-%` is 100); older sessions have negative timers (-63 to -359) in the `+0x240`/`+0x2e0`/`+0x330` pools; the `+0x240` pool's value is `maximum_energy_shield` | 0 | delay in life, `+0x240` and last pool; ES from the stat |
| Player `flags` | `0x0c` in all three (bit 3 always set) | 0 idle, `0x0c` in stance | `0x08` idle, `0x0c` in stance |
| Stat sets | 4: `0x40`, `0x41`, `0x80` (only with a weapon), `0x81` (empty). Set 2 = `weapon_set_index` 2, `weapon_set_inherent_shapeshift_form` 1 (+ gem barrier stats); set 1 = derived totals + `both_hands_are_empty`, `weapon_set_index` 1 | none | `0x40`, `0x41`, `0x81` from the character's stats |
| Base stats | 58-77, all server-derived (armour, evasion, resistances, regeneration, accuracy, attribute-derived, `on_full_life`-style conditions, `cannot_die` in town) and **none of the template-only constants** | 101 stats: the template's constants plus class base values | + the full life/mana/ES conditions, `poise_threshold`, `intermediary_poise_threshold`, the recharge delay, sorted by key; the rest needs a stat calculator |
| Buffs | 0 or 1 hidden permanent buff | none | none. The rows are skill implementation counters (`skill_seals` 1188, `ancestral_slam_big_hit_counter` 1437; `Invisible`, `BuffVisual` row 0): they have no visual and depend on the character's skills |
| Animated | `0x80` + five floats, **no path, also for the local player** | same (but our own local blob sends the `.ao` path) | same; `[dev] remote_animated_path` adds the path (flags `0x81`) |
| Equipment visuals (Inventories) | 6-10 entries on the remote players, 5 on the local one (starter kit): `slot, id, ...`; the first `u16` of each visual is **`ItemVisualIdentity.HASH16`** of the equipped item (`FourBow1` 46436, `FourHelmetInt1` 51364, `FourBootsInt1_` 44805, `FourQuiver1` 13859, ...); the `id` byte is fixed per armour slot (6 -> 25, 8 -> 26, 9 -> 23, 10 -> 24, 11 -> 21, 14/15 -> 6, 16 -> 22, 17 -> 5) | none (also for our local blob, which draws fine) | none: characters have no equipment |
| Links | one record (the object last interacted with) | none | none |
| Flag words | 15-37 and 0-27 entries | none | none: no per-character quest progress |
| Actor | 2 skills: the dodge roll once per weapon set (ids `0x80000040`/`41`, slot byte `0x40`/`0x41`) | the character's skills (id 1) | same as before |

Reading of it: the class model is drawn from the object type (the class `.ao` has the outfit attachments Body, Boots, Gloves and Head built in, so a character without equipment is drawn without any equipment visual); the equipment visuals only replace attachments. The one field that differs from *all five* official blobs in a way that could mark the entity itself is the Life section (`+0x30` full where the official server sends 0, flags 0); our local blob has the same Life section and draws correctly, so either the local entity is not affected by it or the cause is elsewhere (stat sets, derived stats, Player flags, the missing path). The WP C2 blob matches the official one in every part the server has data for. **Bisect switches** (not used, the tint was gone in the re-check): `[dev] remote_blob_omit` leaves the generated parts out one by one (`stat_sets`, `derived_stats`, `life_state`, `player_flags`) and `remote_animated_path` adds the path, so the part the client reacts to shows without a rebuild. If the tint ever comes back, the missing equipment visuals are the next suspect (needs an item model: slot, `ItemVisualIdentity` row, per-slot id byte).

### Remote roll check (WP C2)
The official remote roll batch (10 rolls of one remote player starting at capture frame 7636, and 12013's own 8 rolls): `SprintState` held, `StartAction` (skill `0x80000040` here, flags `0x410`), `ActionEvent`, `StatsUpdate` `is_dodge_rolling = 1`, `UseSkillAck`, `StatsUpdate` stun thresholds, all in one frame, later `SkillInputEndAck` + `SprintState` released; a roll pressed during a roll uses `(self, 5, n)` headers. That is the local batch message for message, with **one difference: the remote stun threshold update has two stats, `virtual_heavy_stun_threshold_+%_final` and `virtual_light_stun_threshold_+%_final`; the local one has three, with `stun_threshold_+%_final_while_performing_action` first.** The same holds for the other roll-related updates (`base_move_speed_+%_from_sprint` and `virtual_skill_ignore_namelocking_dynamically` appear only in the local sprint updates). The rule is in `Stats.dat`: those three stats have `IsParty = false` (the others in the batches have `IsParty = true`), and the server sends other players only the `IsParty` stats.

The server now encodes every movement event twice, for the owner and for the others (`Codec::encode_for_others`, stats filtered with `StatRegistry::is_party`; an update left empty is not sent), so a remote roll carries exactly the official stat lists. The messages, their order and the frame are the same as before (the actor already relayed the whole batch; test `a_roll_batch_reaches_every_session_in_the_frame_it_is_produced`), the new test `a_remote_roll_batch_leaves_out_owner_only_stats` pins the filtering. Verified in game 2026-10-21 ([hosting.md](../../modules/server/docs/hosting.md#local-two-client-test)): client 2 received for entity 12000, in one frame, `SprintState`, `StartAction`, `ActionEvent`, `StatsUpdate` (`is_dodge_rolling`), `UseSkillAck`, `StatsUpdate` with the two `virtual_*_stun_threshold` stats and no `stun_threshold_+%_final_while_performing_action`, later `SprintState` 0, `SkillInputEndAck` and the reset `StatsUpdate`; the steered remote roll animates (tumble pose in the F8 shots, the character ends at the new place). Not checked: the roll distance against the official value.
