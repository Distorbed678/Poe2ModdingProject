# RE findings: PathOfExileSteam.exe 0.5.5e

> **Since WP A2 (2026-10-21) the DLL no longer uses the addresses on this page.** Every hook address, patch site and struct offset it needs is found by byte signature from `modules/client-hook/signatures/0.5.5e.toml` (85 signatures, 51 fields; each has its 0.5.5e Ghidra address in a `ghidra` key and the offsets their 0.5.5e `value`). That table is the source for what the DLL does; this page stays the knowledge base (what the functions are, evidence, names) and the starting points for a new build. When the two differ, the table was checked against the exe (`cargo run --release -p poe-hook-sig --example resolve -- GameFiles/PathOfExileSteam.exe`) and this page may be stale. See [../architecture/client-dll.md](../architecture/client-dll.md#signature-table-signaturesversiontoml) and [update-compatibility.md](update-compatibility.md).

Ghidra project: `GhidraProject/Poe2ModdingProject`, program `/PathOfExileSteam.exe`, image base `0x140000000`, 115,673 functions after auto-analysis. Addresses are Ghidra addresses. The DLL converts them with `Module::from_ghidra`.

| Address | Name / what | Evidence |
|---|---|---|
| `0x140ee79c0` | gateway list loader | references `"gateway_list.txt"` (0x143158510) and `"Bundles2/gateway_list.txt"` (0x143158558) |
| `0x14211dbb0` | instance server connect | references `"Connecting to instance server at {}:{} "` (0x143507aa0) |
| `0x1435130b8` | string `"HNC Protocol Violated"` | no direct code references found yet |
| `0x1420e8193` | login packet dispatcher switch (`ClientLoginSession`?), opcodes 0x02–0x47; opcode 2 case at `0x1420e81c0` | jump table `0x1420e98e4` / index `0x1420e9954` |
| `0x14219f010` | crypto: verify server signature + derive keys ("General Rhoa Failure #3") | hooked by `crypto_bypass` |
| `0x14219ee40` | crypto: DH2 agree + SHA-512 → 64-byte key material ("#1") | |
| `0x14219dc40` | crypto session constructor (DH groups, DSA key, "#2") | |
| `0x14219f3b0` | builds the two-direction Salsa20 cipher from the key material | |
| `0x142198530` | installs a cipher on a Tcpstream | |
| `0x14212aa90` | instance packet-cipher builder `(out, InstanceDetails + 0x38, &is_client)`, only caller `0x142122062` in `0x142121f70` | hooked by `side` for the side-channel keys ([crypto.md](crypto.md#side-channel-keys)) |
| `0x142b030b0` | Crypto++ Salsa20 key setup | |
| `0x142102c20` | loader for server-delivered encrypted code modules | see [crypto.md](crypto.md) |

## Imports of interest
- `VERSION.dll`: `GetFileVersionInfoW`, `GetFileVersionInfoSizeW`, `VerQueryValueW`. These are proxied by poe-hook.
- `WS2_32.dll`: sockets. `bcrypt.dll` and `CRYPT32.dll` are present. The crypto strings in the binary (CALG_*, TLS_*, RSA) belong to bundled curl/OpenSSL for HTTPS. The game protocol cipher has no string anchors.

## Entities and rendering (2026-10-13)
| Address | What |
|---|---|
| `0x1421302c0` | game world update in frame mode: plays queued frames in real time, dt = frame ms / 1000 |
| `0x1421306c0` | world frame drain (`world, f32 dt`): applies queued simulation packets, updates entities |
| `0x14212fc10` | world start (InitialState vfunc +0x1f0): applies all queued packets |
| `0x1421301e0` | flush packets scheduled at the current (entity, component, sequence) position |
| `0x141e72880` | object registry insert ("Replacing awake/asleep/deleted object") |
| `0x1421370f0` | frame-mode entity update loop (component vfunc +0x50, then `0x141e7d7f0`) |
| `0x1421340e0` | simulation packet queue (`0x2f4` frame marker, `0x2f5` → world vfunc +0x178) |
| `0x140150cb0` | entity → Render component (by name) |
| `0x1416a9810` | Render: apply visibility to the AO scene node (+0x39a disabled flag) |
| `0x141923f80` | GEAL `EnableRendering` lambda body |
| `0x141d439e0` | animation controller: play animation (index, …) |
| `0x141dfa320` | StateMachine reader (shared-state count check) |
| `0x1426f2220` | GEAL script interpreter loop |
| `0x140286850` | `Data/Balance/Hideouts.dat` table getter |

## Session, chat, pause (2026-10-14)
| Address | What |
|---|---|
| `0x14211f540` | logout routine ("FourLogOut"): 0 / 2 = exit to login / game, 1 = exit to character selection (sends `0x173`) |
| `0x14215df40` / `0x14215e010` | S→C `0x174` ExitSession constructor / reader |
| `0x1421842a0` | S→C `0x1fe` SetPause reader; applied via world vfunc +0x200 `0x14168aaf0` |
| `0x14215cb50` / `0x14215cc30` / `0x142124bca` | S→C `0x14d` ChatJoined constructor / reader / handler |
| `0x1401b6940` | chat join timeout message ("Timeout while joining {0} chat") |
| `0x1402c4980` | escape menu construction ("Get Full Access" button when +0x3edc is clear) |
| `0x1420edaa0` | InitialState CharacterState flags → local player (+0x3eca..+0x3ede) |
| `0x140ed8710` | delete-character screen result callback |
| `0x1421211e4` | `0x1c` frame report sender; `0x141f486f0` its serializer |
| `0x141f48c60` / `0x142122115` | S→C `0x1d` ping reply reader / handler |
| `0x1421c50b0` | HNC bit queue: key material XOR table `0x142f10960` + swaps, Salsa20 pair |
| `0x1421c5360` | HNC incoming byte parser ("HNC Protocol Violated") |

## Movement and skills (2026-10-14)
| Address | What |
|---|---|
| `0x14211f880` | game-session send (`packet->vtable[1](packet, stream)`); senders build the packet on the stack (opcode at +8) |
| `0x142149640` | Actor component simulation handler: `0x64` WASD, `0x65`, `0x1c8` start action, `0x1c9`..`0x1ce`, `0x1d8`/`0x1d9` skill stats, `0x1f9`..`0x1fc`, `0x2e7`, `0x30e` action update, `0x316`..`0x324` |
| `0x141f6b0e0` | S→C `0x64` MoveDirectionUpdate reader |
| `0x141f696b0` | S→C `0x1c8` StartAction reader |
| `0x141e4f9c0` | start WASD walking: a Move action (skill `0x20002909`) toward cell + input vector |
| `0x141e89130` | create an action from an actor skill |
| `0x14210f920` | C→S `0x1a5` UseSkill sender (vtable `0x1434e5b40`, body `0x141ecdd00`); `0x1a7`..`0x1ad` senders follow |
| `0x14016e780` | C→S `0x2e6` sender (`u8 player, u8`) |
| `0x14016ba40` | `AimReport_Tick`: per frame, AimTarget mode 2/3 via `0x14016b820` (33 ms) only while mode 1 was sent (+0x418, WASD input mode), `GetForegroundWindow() == *[0x1445d8308]`, the Player component (`local player +0x188`) has combat stance (+0x20d), and UI checks pass |
| `0x141dcf320` / `0x141dd0a10` | Player component reader (full, vtable `0x1433a7708` +0x98) / delta reader (+0xb0, `u16` raw mask; see `PlayerDelta`) |
| `0x141dd47e0` | Player vfunc +0x138: set combat stance (+0x20d); +0x20c is the networked byte |
| `0x141ffe940` / `0x141fefa20` | `TerrainGen_Run` (generator driver) / `TerrainGen_Construct`; phases, RNG and hashes in [terrain/README.md](terrain/README.md) |
| `0x14167b270` / `0x14212f500` | `ClientWorld_LoadArea` (in-game world) / `HeadlessWorld_LoadArea` (server-side world class in the client) |
| `0x141f9c020` / `0x140a58a70` / `0x141f0d7f0` | terrain RNG seed (TLS +0x20190) / `std::mt19937` next / MSVC `uniform_int` draw |
| `0x141d141d0` / `0x141ff5550` / `0x141ff3310` | tile hash / doodad pass (doodad hash → world +0x118) / per-tile doodads and entities |
| `0x1408719a0` | stream varuint reader (the `0x2f9` ComponentDelta length) |
| `0x14218a850` | Actor skill container reader; `0x14218afb0` skill list; `0x142188bd0` one skill |
| `0x142683e50` / `0x142683b70` | skill effect reference reader (GrantedEffectsPerLevel row) / stat-set reference (GrantedEffectStatSetsPerLevel row) |
| `0x1419970a0` / `0x141e25d50` | varuint / varint readers used in stat lists |
| `0x141dcd020` | Pathfinding component constructor (+0x550 base speed, +0x55c multiplier) |
| `0x141dcce20` | movement speed getter (base × movement_velocity_+permyriad × virtual_action_speed_+%) |
| `0x14212eb60` | world constructor (+0x661 frame mode; in-game world created with 2 by `0x14100b7e9`) |
| `0x141939e40` | `[NETWORKING] networking_type` option reader |
| `0x142120b70` | game session update: frame reports, auto network-mode switch, world update |

## Roll, sprint and steering (2026-10-14)
| Address | What |
|---|---|
| `0x141b76c20` | end of a dodge roll: decides whether it carries on as a sprint |
| `0x141e5d490` | Actor "is sprinting": Actor +0x915 or the stat `is_sprinting` |
| `0x141e4de20` | movement reset / stat listener: stops Move when Actor +0x2a1 bit 2 is set |
| `0x141aa90d0` | `dodge_roll_travel_distance` getter |
| Actor +0x238 / +0x240 | current action (roll) / Move action slot (WASD walk); action type at action +0x120 |
| Actor +0x2a2 bit 5 | sprint key held (set by S→C `0x2e7`) |

## Character creation (2026-10-14)
| Address | What |
|---|---|
| `0x14214fa60` | login incoming-packet factory (opcode → class); `0x13` CharacterCreated class `0x143509d00`, deserializer `0x14214ea70` |
| `0x1420e81a5` | login dispatch switch (opcode − 2; byte table `0x1420e9954`, targets `0x1420e98e4`); `0x13` case `0x1420e895e` signals connection +0xb08 |
| `0x140ec82e0` | create screen: connects to the login connection's signals (+0xb08 → `0x140ed2ab0`, +0x188 → league list `0x140ed49b0`) |
| `0x140ece3e0` | create screen Continue/Create button (needs a selected league: +0x320, +0x318) |
| `0x140ece810` | sends CreateCharacter (`0x1420e9b60`); do not hook its entry, the trace hook crashes |
| `0x140ed2ab0` | create result callback (u16 result): 0 → +0x392, creation scene (`0x140ececc0`, sets +0x390); else error text |
| `0x140ecf5d0` | create screen per-frame update: scene done with +0x390 and +0x392 → `0x140ecee50` |
| `0x140ecee50` | "character_creation_done": finds the new character by name (`0x140ec8b20`), sends PlayCharacter (`0x1420e9e70`) |
| `0x140ed4e50` | create screen key handler (Space skips the creation scene) |
| `0x140e7ca80` | league banners at character creation (entries with +0xe0 bit 7, not variants; variants matched by +0x80) |

## Frame playback, snapping and leaving the game (2026-10-14)
| Address | What |
|---|---|
| `0x1421302c0` | lockstep frame playback: speed factor world +0x670 (1.01× with 1 spare frame, 1.5× with 2–29, 2× with 30–59, 5× with ≥60, back to 1.0 when drained) |
| `0x141ddb570` | SetLocationForcedNoCallbacks (moves an entity to a cell) |
| `0x142149640` (`0x1421497ae`) | `0x64` handler: snaps only when the server cell is ≥46 cells away (distance² ≥ `0x844`), else bends the walk direction |
| `0x141e41ae0` (`0x141e41ba0`) | StartAction apply: same 46-cell snap threshold |
| `0x1402c76b0` | escape menu buttons: +0x2f0 exit to log in screen (logout 0), +0x2f8 exit game (`0x140eac930`, logout 2 + WM_CLOSE), +0x300 exit to character selection (logout 1) |
| `0x14211fad0` | sends `0x355` ExitReport (once per session) |
| `0x141f479b0` | `0x355` serializer: `u8, u16 len, bytes` |

## Facing (2026-10-15)
| Address | What |
|---|---|
| `0x141ddc6a0` | Positioned::SetOrientation(angle): writes +0x44c (radians, `atan2(dy, dx) + π/2`) unless locked (+0x1e5 bit 1) |
| `0x141dda640` | Positioned simulation handler: `0x1dd`/`0x1de` position, **`0x1df` SetOrientation (f32, u8 locked)**, `0x1e0` knockback-like move, `0x1e7` +0x440/+0x48c, `0x2ef` flags |
| `0x141f6af00` | `0x1df` reader |
| `0x141de3020` | movement step: turns the entity toward its path (orientation mode +0x1e3 or the request's; 2 = backwards) |
| `0x141dd8820` | Positioned initial data (facing +0x44c, flags0 bit 7 → lock) |
| `0x141e534b0` | Actor RecalculateStance ("Aiming", "AimingUpwards", "Ranged", "Mounted", "Unarmed") |
| `0x14214a2ea` | Actor `0x1cc` (u16 action id, u8) → Actor vfunc +0x130 (end action?) |
| `0x14214a400` | Actor `0x1fc` (f32, u16) → Actor vfunc +0x100 |
| `0x14210fbb0` | skill target update sender: `0x1a8` (lockstep, relative), `0x1a9` (absolute), `0x1aa` (entity) |
| `0x14210ee60` | chat sender (`0x16`, wstring; "@whisper") |

## Streams and packet capture (2026-10-14, debug DLL)
| Address | What | Notes |
|---|---|---|
| `0x1421c5950` | message reader `(registry, stream)` | reads the opcode, lets the registered handlers deserialize, advances the receive ring (+0x268) by the message size; low byte of the result 1 = a whole message was read |
| `0x1420e6bc0` | login session receive loop | caller of the message reader on the login connection |
| `0x142120b70` | instance session receive loop | caller of the message reader on the instance connection |
| `0x142197d30` | stream read `(stream, dst, len)` | copies from the receive ring at +0x268 + +0x278 (bytes read in this message); sets +0x252 on underflow |
| `0x142197e60` | stream write `(stream, src, len)` | appends to the send buffer (+0x280, length +0x258); flushes when full |
| `0x142197fc0` | stream flush `(stream, err, force)` | plaintext tap (+0x190), cipher (+0x1b8 slot 1, in place), then `send` on the socket (+0) |
| `0x142198037` | inside the stream flush | after the lock, before the cipher: pending plaintext `[+0x280] + [+0x260 .. +0x258]`, rbx = stream |
| `0x1410262d0` | Vulkan loader | `LoadLibrary("vulkan-1.dll")` and `GetProcAddress("vkGetInstanceProcAddr")` |
| `0x140150cb0` | get Render component by name | entity `+8` → type, `+0x28` → component lookup: sorted `{char* name, i32 index}` (16 bytes) from `+0x28` to `+0x30`; the index selects `entity+0x10[index]` |
| `0x142181f20` | Stats component message handler | `0x1d3` StatsUpdate: set 0 → container at component `+0x1c8`, other sets in the list `+0x1e8..+0x1f0` (container `+0xd8` = set id); `0x1d4` removes a set |
| `0x1426ac920` | stat container set `(container, key, value)` | sorted `(i32 key, i32 value)` array at `+0xf8..+0x100` |

## Login hand-off, chat, AreaInfo stats, terrain passes (2026-10-16/17)
| Address | Function | Notes |
|---|---|---|
| `0x1420e8130` | `ClientLoginSession_ProcessPacket` (named in Ghidra) | login message switch (opcode − 2); case `0x14` InstanceDetails (+0x84 byte kept), case `0x17` → `0x1420e7140` |
| `0x1420e7140` / `0x1420e71d0` | login disconnect with reason | `0x1420e71d0` calls `shutdown()` on the login socket: login `0x0017 <u16>` makes the client close its login connection |
| `0x142150ad0` / `0x142150a60` | S→C `0x18` ChatMessage read / write | `u8`, then the chat struct (`0x1421a73d0` read, `0x1421a71d0` write): 3 wstrings (`0x141cfb1b0`), flags, class, item links |
| `0x14211d1c0` | AreaInfo reader | optional blocks by flags0/flags1 bits; flags0 bit 7: stat records `0x1423baa70` (varuint count, `u16` key, varint value via `0x141cfbfe0`) |
| `0x1420a1f70` | dungeon exit wiggle (after the fill) | sorted position set at tile pass +0xb0, filled by pass A (`0x142098b18`) and the fill listener `0x14209a2c0`; hashed exits `0x1421969e0` |
| `0x14209f050` | dungeon border tile placement | fallbacks: middle exits `0x14209fa10`, height flattening `0x14209fb00`, neighbour repair `0x14209ec70` |
| `0x14209d180` | dungeon door groups | door sizes per edge set `0x141d09700` (sets built by `0x141d09ab0`) |
| `0x141ff7a10` / `0x141ff05f0` | per-graph outer fill + buffer / subgraph generation | subgraphs run the whole pipeline in a window (`0x141f9e130`) of the main map |
| `0x141fd5ff0` / `0x141fd71c0` | outer fill with height repair / one levelling round | see [terrain/dungeon.md](terrain/dungeon.md) |

## Skill acknowledgements, buffs, sprint (WP2.1, 2026-10-20)
Names below were set in Ghidra on 2026-10-20.
Layouts and sequences: [../protocol/movement.md](../protocol/movement.md). Structs: `modules/protocol/poe-proto/src/skill.rs`.

| Address | Ghidra name | What |
|---|---|---|
| `0x142186800` | `UseSkillAck_1ae_Read` | S→C `0x1ae` reader: sim header (`SimMsg_ReadHeader`, `0x141f4b680`) then the skill block |
| `0x141ecde00` / `0x141ecdd00` | `SkillUseParams_Read` / `_Write` | shared block: `u32 skill, u16 flags, u8, u8 target flags, [u32 entity if flags & 8], i32 x, i32 y` (C→S `0x1a5` writes it after `u8 player`) |
| `0x141f6d520` | `SkillTargetAck_1af_Read` | S→C `0x1af`: header, `i32 x, i32 y, u32 skill` |
| `0x1421868e0` | | S→C `0x1b0`: header, `u32 entity, i32 x, i32 y, u32 skill` |
| `0x141f4b680` | `SimMsg_ReadHeader` | `0x1b1` (and `0x1c6`, `0x1c7`, `0x1ca`) have no body past the header; vtable `0x14350bea8` |
| class vtable `0x14350c0a8` | | S→C `0x1b2`: header, `u32 skill, u8` (ctor `0x142187ec5`) |
| `0x14210fd40` | `Session_SendSkillInputEnd_1ac` | C→S `0x1ac` (`u8 local player`); in lockstep (world +0x661 == 2) the client only waits for `0x1b1`, otherwise it calls Player vfunc +0x118 itself |
| `0x14210fdc0` | `Session_SendSkillReleased_1ad` | C→S `0x1ad` (`u8, u32 skill, u8`); the local fallback is Player vfunc +0x120 |
| `0x14210f920` | `Session_SendUseSkill_1a5` | builds the packet with the cursor relative to the player (`x -= Positioned +0x444`, `y -= +0x448`; absolute when a target entity is given). Outside lockstep it ORs `0x04` into the local copy and calls Player vfunc +0x108 |
| `0x14210fbb0` | | C→S `0x1a8` / `0x1a9` / `0x1aa` target updates (already listed above) |
| `0x142134240` (`0x142134bef`, `0x142135b99`) | | frame processor: `0x64`/`0x65` go straight to the Actor handler `0x142149640`; the opcode switch at `0x142135b99` (base `0x1af`) sends `0x1ae`..`0x1b2`, `0x26f`, `0x270` to `Entity_GetPlayerComponent` + vfunc +0xc0 |
| `0x14171cc70` | `Player_SimulationHandler` | Player component sim handler (vtable `0x1433a7708` +0xc0); jump table over opcode - `0x1ae`: `0x1ae` → `0x14171d205`, `0x1af` → `0x14171d23d`, `0x1b0` → `0x14171d27c`, `0x1b1` → `0x14171d2b5`, `0x1b2` → `0x14171d2e2`, `0x26f`/`0x270` → `0x14171ccd0`/`0x14171cdaf`. Each of `0x1ae`..`0x1b2` is skipped when `[Player +0x430] != 0` and the world vfunc +0x1d8 returns 1 (predictive); in lockstep they always run |
| `0x141dd4270` | `Player_ApplyServerSkillUse` | Player vfunc +0x108 (`0x1ae`): copies the block (24 bytes) into the pending-skill slot (+0x214, or +0x22c when the skill is Move `0x20002909`), marks it valid. Target flags bit `0x04` set → stop. Else, if the player may act (vfunc +0x128) and bit `0x01` is clear, starts the skill locally (`0x141ecd8e0`); bit `0x02` with a running action of the same skill cancels it |
| `0x141dd43d0` | `Player_ApplyServerSkillTarget` | Player vfunc +0x110 (`0x1af`, `0x1b0`): stores the position (+0x21c / +0x234) and entity (+0x218 / +0x230) of the matching pending slot, then Actor vfunc +0x128 `(pos, skill, entity)` re-targets that skill's running action |
| `0x141dd4520` | `Player_ClearHeldSkill` | Player vfunc +0x118 (`0x1b1`): clears both pending slots (+0x214, +0x22c), then ends the held action unless Player flag +0x226 bit 2 |
| `0x141dd4610` | `Player_ApplyServerSkillRelease` | Player vfunc +0x120 (`0x1b2`, `u32 skill, u8`): clears the slot of that skill (Move → +0x22c) and runs +0x118 for the main slot; a running Actor action (+0x248) of that skill is ended (except a Move with the `u8` clear); with the `u8` set and the Move slot (+0x240) matching, the walk is finished (`0x141e45c20`) |
| `0x14014fe20` / `0x14014e870` | `Entity_GetPlayerComponent` / `Entity_GetActorComponent` | component lookup by name (the name is a static string pointer; Player at `0x1434d58d4`) |
| `0x141f6bbf0` | `BuffAdded_1eb_Read` | S→C `0x1eb`: header, `u16 index, u16 buff row, u32, u16, u16 flags`, optional fields by flag (`0x01` varuint,u32,u8,u16; `0x02` f32; `0x80` f32; `0x04` u16; `0x08` u16,u32,u8; `0x10` u16 buff-definition lookup; `0x20` u8), a definition-dependent `u16` (definition +0xfe) and `u32` list (definition +0x7c), then `u32 n` + `n` x `u32` |
| `0x141f6c1a0` | `BuffRemoved_1ec_Read` | S→C `0x1ec`: header, `u16 index`; the Actor-side opcodes `0x1eb`..`0x1ed` share the handler at `0x142135bf8` |
| `0x14214a13e` | | Actor case `0x1ca` (header only): decrements `Actor +0x348`; in predictive mode also restarts the action; seen with every roll start and sprint end by an attack |
| `0x141b76c20` | | end of a roll: whether it becomes a sprint (already listed above); official servers decide server-side: `is_sprinting` is set 15 frames into a held roll |

## Walk speed factor, roll heading, sprint end (WP C1, 2026-10-21)
Names set in Ghidra on 2026-10-21. Findings and evidence: [../protocol/movement.md](../protocol/movement.md#coasting-after-a-sprint).

| Address | Ghidra name | What |
|---|---|---|
| `0x141dcce20` | `Pathfinding_GetSpeed` | already listed above: `base_speed` (+0x550) x (1 + stat `0xec` / 10000) x (1 + stat `0xc39` / 100) x multiplier (+0x55c), capped by stat `0x44a`; +0x554 < 0 means "computed" |
| `0x141dccfb0` | `Pathfinding_SetSpeedMultiplier` | writes the multiplier +0x55c (clamped at 0) and re-issues the running step (`0x141dcbf90`) |
| `0x141e41ec0` | `Actor_ApplyActionSpeedFactor` | from the running action: vfunc +0xe8 = 0 sets the factor at once (non-Move actions: Actor +0x440 = vfunc +0xe0), else Actor +0x444 = rate and +0x448 = target (vfunc +0xe0 clamped 0..1); the Actor tick (inside `0x141e42880`, the multiplier written at `0x141e44123`) moves +0x440 to the target at that rate per second and multiplies it by Actor +0x438 into Pathfinding +0x55c |
| `0x14197e990` | `Action_GetSpeedAccelerationPerSecond` | action vfunc +0xe8: stat `0x4777` (`movement_speed_acceleration_+%_per_second_while_performing_action`) / 100 when the action is a performing one (vfunc +0x58); `0x1419b0bd0`, `0x1419b7390` are variants |
| `0x141e4f9c0` | `Actor_StartWasdWalk` | start of a WASD walk: creates the Move action (skill `0x20002909`) towards cell + input vector; returns without starting when Actor +0x400 and +0x404 (the held input axes) are both 0, or while Actor +0x2a1 bits 1/2 or +0x2a0 bit 6 say no |
| `0x141dcbdb0` / `0x141dcc520` / `0x141dcbf90` | `Pathfinding_FollowPath` / (path request) / `Pathfinding_StepAlongPath` | the path request clamps the first leg to 126 cells (`0x42fc0000`), step issues the waypoint move with the speed getter |
| `0x141de3020` | `Positioned_MoveToWaypoint` | the entity's own move towards a waypoint cell with a speed (`"Tried to issue a MoveTo with infinite speed"`) |
| `0x141aa8380` | `DodgeRoll_OnStart` | the roll action's start: stats `0x5121`, `0x478c`, travel distance (`0x141aa90d0`), builds the movement helper and calls `0x141b8bc20` |
| `0x141b8bc20` | `DodgeRoll_InitHeading` | roll heading (action +0x40, radians, `atan2 + pi/2` wrapped to 0..2pi): the held input vector when Actor +0x400/+0x404 are not both 0 and the action's flag +0x5e is set; else the direction from the entity's position (Positioned +0x444/+0x448) to the action's target (+0x150/+0x154); the entity's facing (+0x44c) when target = position; action +0x3f = 1 when the heading differs from the facing by 90 degrees or more (Actor +0x8fa) |
| `0x141e4de20` | | (already listed) reset of the Actor's walk on stat changes: ends the Move action; the coasting after a sprint's end was not found in code, only measured |
| `0x141e4d9e0`, `0x1416e1f70` | | sprint animation transition: `sprint_blend_time_override_ms` (`0x5735`) / 1000 as the blend time (-1 = default), `virtual_sprint_timeline_variation` (`0x6996`) |
| client log `action start/end slot N: <Action>` (`poe-hook.log`, `poe_hook_core::actions`) | | slot 0 = the performing action (DodgeRoll, Melee), slot 1 = Move; the end of the Move at a sprint's end and its restart by a `0x64` are read from it |

## Client platform and standalone login (2026-10-20)
Details and the patch: [standalone-login.md](standalone-login.md).

| Address | Name | What |
|---|---|---|
| `0x142196ec0` | `ClientPlatform_Detect` | platform enum from the exe file name and command line: 0 standalone, 1 Steam (`Steam` in the name or `--steam`), 2 Hotcool, 3 Hotcool+Steam, 4 Kakao (`KG` / `--kakao`), 5 Epic (`EGS` / `--epic`). Patch `40 55 53` → `31 C0 C3` forces 0 |
| `0x144717118` / `0x144717114` | | cached platform value / its thread-safe-static guard |
| `0x140109b20` / `0x140109be0` / `0x1401af120` / `0x142197330` | `ClientPlatform_IsSteam` / `_IsHotcool` / `_IsTokenLogin7or8` / `_Equals` | cached-value getters (about 90 callers) |
| `0x140ee8cb0` | `LoginState_BuildUi_SelectLoginScreen` | picks `LoginScreen.ui` / `SteamLoginScreen.ui` / Hotcool / Epic / Kakao by platform (branches `0x140ee9228`..`0x140ee9357`) |
| `0x140ef4c90` | `LoginState_StartAuthentication` | after the key exchange: Steam → ticket request, 5 → `0x140ef3a10`, 4 → `0x140ef4050`, else `0x140ef4410` |
| `0x140ef4410` | `LoginState_StandaloneLogin` | email/password or saved-token login; sends `0x0003` |
| `0x140ef9ec0` | `LoginState_OnSteamEncryptedAppTicket` | Steam callback: ticket (≤1024 bytes) → `0x0005` |
| `0x140ef3530` | `LoginState_TokenLogin7or8` | platform 7/8 only: `0x0032` / `0x0033` (`0x1420e7620`, `0x1420e7800`), dead on PC |
| `0x1420e7af0` | `ClientLoginSession_SendCredentials_0003` | builds and sends `0x0003` (class vtable `0x1435063b8`, serializer `0x1420e3b60` `Packet0003_Credentials_Serialize`) |
| `0x1420e7e00` | `ClientLoginSession_SendSteamTicket_0005` | builds and sends `0x0005` (serializer `0x1420e34f0`) |
| `0x1421bbd70` | `Sha256_Bytes32` | SHA-256 (IV at `0x143a0c198`) used for the password hash: UTF-16LE bytes, no salt |
| `0x1420e7580` | `LoginClientHash_Get32` | the 32-byte client hash (global `0x1445d5f50`), a hash of the file at argv[0] computed on a worker thread by `0x1420eb470` (`0x1410261a0` returns argv[0], `0x1421bbdf0` hashes the file) |
| `0x140ef07d0` | `LoginState_SaveLoginAfterAuth` | after success: ini `[LOGIN]`, writes or deletes the credentials file with the `0x0004` `session_token` |
| `0x1423d0210` / `0x1423d03a0` / `0x1423d0490` | `LoginCredentials_GetFilePath` / `_Load32` / `_Save32` | `Documents\My Games\Path of Exile 2\<name>_login.credentials`, exactly 32 raw bytes |
| `0x140ef63a0` | `LoginState_HandleAuthenticationError` | non-zero `0x0004` result handler: notice text if present, else the `BackendErrors.dat` text; special codes `0x62` / `0x1cf` (Unlock window), `0x0f` (delete the saved credentials), `0x08`, `0x54` |
| `0x141018170` | | `config.json` loader (`GGG_*`, `STEAM_APP_ID`, `GGG_CREDENTIAL_FILE_NAME`, `GGG_LOGIN_SERVER_PORT`, ...); defaults at `0x1446fc340` |
| `0x140110960` | | Steam API init, returns 1 without touching Steam when the platform is not Steam |
| `0x1421ee510` | `StandalonePatcher_GetErrorText` | the **patcher's** own 16-entry error table (only caller `StandalonePatcher_Run`); the login, create and delete texts come from `BackendErrors.dat` via `0x1425cb780` |
| `0x14011b820` | `Client_Startup` | start-up function (WinMain-level): single-instance check, config, `StandalonePatcher_Run` (call `0x14011bf76`), `CmdLine_ReadSuccessEvent`, then engine/state init; `SetEvent`+`CloseHandle` of `DAT_144431668` at `0x14011c916` |
| `0x1401162d0` | `StandalonePatcher_Run` | returns bool; Steam/Epic return 1 at once; skipped (returns 1) when `--success-event` is given and `--disable-client-switching` is not; else the patcher window + patch-server connection. First bytes `48 8B C4`; the DLL safety net writes `B0 01 C3` |
| `0x142737620` | `Patcher_RunPatching` | patching run ("Started patching ({} free bytes available)" `0x14358d340`, "No patching required" `0x14358d100`); only caller is `StandalonePatcher_Run` |
| `0x1427370c0` / `0x142733af0` | | `/client/record-patching-metrics` sender / its only caller (inside the patcher) |
| `0x1428ca5f0` / `0x1428cba30` / `0x1428cc930` | | patcher window: create (`MainWindowClass` `0x1435ac7f8`), class registration, `/launcher/patch/start` |
| `0x140117c40` | `CmdLine_ReadSuccessEvent` | parses `--success-event <int>` into `DAT_144431668` (-1 initially) unless `--disable-client-switching` |
| `0x140110d60` | `CmdLine_BuildOptionTable` | registers every command-line option (`--patching-server`, `--require-launch`, `--success-event`, `--nologo`, `--language`, ...); no `--nopatch` |
| `0x1423cc1a0` | `CmdLine_ParseArgs` | parser: `--opt value` / `--opt=value`; unknown args only log `[CMD][Unrecognized]` (`0x143522628`) |
| `0x1423cd120` | | reads an integer option value (`wistringstream`), throws "value is not an integer" |
| `0x14010ac40` / `0x14010aca0` | | config singleton / its default builder (patch servers `us.patch.pathofexile2.com` `0x142f76fd0`, `patch.pathofexile2.tw`, `patch2/3.pathofexile2.tw`, `kr-patch.pathofexile2.com`) |
| `0x141050a30` | `Window_CreateMain_POEWindowClass` | registers and creates the game window (class `POEWindowClass`, string `0x142f797b8`); `FUN_14011ac30` uses the same class for the single-instance check |

## Remote players, entity lifetime, party and trade (WP3.5, 2026-10-20)
Layouts: [../protocol/entities.md](../protocol/entities.md#other-players-add-actions-remove-party-and-trade-wp35). Structs: `modules/protocol/poe-proto/src/player_blob.rs`, `remote.rs`.

| Address | Ghidra name | What |
|---|---|---|
| `0x141e71120` | `Entity_ReadInitialState` | AddObject blob reader: `u8 flags`, link records, components in template order, `u8 m` keyed entries (`0x141e7dbe0`) |
| `0x141e71420` | `Entity_ReadLinkRecords` | `u8 n`, each `u32 key, u8 tag`, values by `0x141e7e9d0`; stored in the entity's property map (`entity +0x28`: `0x1421f12b0` / `0x1421f0c50`) |
| `0x141e7e9d0` | `LinkRecord_ReadValues` | `u8 count`, then (count != 0) `u8 kind` and `count` values: kind 1 = `u32` entity ids (resolved with `0x141e72030`, unresolved ones queued by `0x141e73210`), other kinds through the variant reader table at `0x1434db750` (kind `0x0a` `0x141e80dc0`, `0x0b` = `u32` `0x141e80eb0`, `0x0c`-`0x0e` `0x141e80fb0`/`0x141e810d0`/`0x141e811e0`) |
| `0x141e71620` | `Entity_Destroy` | `(entity, bool destroy parent chain)`: sets `+0x8c |= 1`, recurses into the children (`+0xd0..+0xd8`), appends the entity to the world's delete list (`world +0xb8`), calls vfunc `+0xa8` when id < `0x40000000`; logs "Attempting to delete an object that is flagged as CannotBeDeleted" for flag `+0x8c & 0x10`. Called by `0x1c5` (parent flag 0) and `0x2fa` (1) |
| `0x142135b9b` (case), `0x142135baa`, `0x142135bc5`, `0x142136099` | | frame processor cases (`0x142134240`): `0x1c5` → `Entity_Destroy(e, 0)`; `0x1c6` → `+0x8d |= 4`, `ObjectInWakeZone` (`0x141e73bf0`); `0x1c7` → `+0x8d &= ~4`, `Entity_LeaveWakeZone` (`0x141e73f20`); `0x2fa` → skipped when `+0x8c & 1`, deferred when `+0x8c & 4`, `+0x8d & 4` and the type flag `+0x29` is clear, else `Entity_Destroy(e, 1)` |
| `0x141db4f00` | `BuffsComponent_Read` | `u16 n`, each `u16, u8`, body `Buff_ReadBody`, then `0x141db5940` (add the buff to the component) |
| `0x141d72910` | `Buff_ReadBody` | buff record: `u16 row, u16 buff-definition index (Buffs table stride 0x137), u32 +0x28, u32 +0x2c, u16 +0x38, u32 +0x34, u16 +0x3a, u16 +0x48, u32 +0x44, u32 +0x4c, varuint +0x3c, u8 +0xb8`, `[u16 +0x64 if definition +0xfe]`, `[u32 × k if definition +0x7c]`, `i32 n`, `n × u32` |
| `0x142180fe0` | `StatsComponent_Read` | stat list (`0x1419970a0`/`0x141e25d50`, applied by `0x1426ac920`); player types (type +0x158 → +0x20): `u8 +0x168`, `u8` shapeshift form (row + 1 of `FUN_1423e55b0`'s table), `u8 × 3` weapon-set forms (`+0x180..+0x1b0`), `u8 k` + k × (`u8 set id` via `0x142181ca0`, stat list), then `u32 +0x1b0, u8 +0x1b4, u8 +0x1b5` |
| `0x141dcf320` | `PlayerComponent_Read` | already listed; the first `u32` after the class is experience (`0x141dd11e0`), `flags` bit 2 = combat stance (`+0x20c`) |
| `0x141d82a40` | `InventoriesComponent_Read` | `u8 n` × (`u8 slot`, `EquipmentVisual_Read`), `u8 k` × `u32` (applied by `0x141d84830` per slot) |
| `0x1426c3f60` | `EquipmentVisual_Read` | `u8 id`; `0` = empty, else `0x1426c3b40` and `0x1426c3d10` (`u16` row, `u32` row of `Data/Balance/MTXTypeGameSpecific.dat`, `u8`) twice, `u8` (row of a 0x11-byte table), `u16` row, `u8 × 3` |
| `0x14210d630` | `Chat_ProcessCommand` | `/command` dispatcher (called by the chat sender `0x14210ee60` for lines starting with `/`); case `0x16` sends `0x00b6` through `Social_SendPartyInvite`, case `0x1a` sends `0x00b7`, `0x28`/`0x29` join a chat channel by name (`0x142128f60`, global/trade) |
| `0x142110810` | `Social_SendPartyInvite` | sends `PartyInvite` `0x00b6` (class `0x1434e51f0`, `u8 local player` from session `+0x181`, wstring name) and raises the UI event `FourSocialPartySend` (string `0x143507418`) |
| class vtable `0x1434e51f0` | | generic `u8 + wstring` packet class (ctor `0x140855780`, serializer `0x141f37db0`): `0xb6`, `0xb7`, `0xc7` and others (the chat sender also builds `0xba` from it); the opcode is stored at `+8` by the sender |
| `0x141f35de0` | `SocialEntryPacket_Read` | S→C `0x00c6` (flags byte: bit 2 character data, bits 0/1 extra `u16` pairs); handler `0x142123f1a` keeps `0x210`-byte entries in the session at `+0x27d8` (add/update by account) |
| `0x1420ee915` | | account-session case for `0x00c4`: logs "InstanceClientSetSelfPartyInvitationSecurityCode = " (`0x143506870`) and stores the `u32` at session `+0x3ec0` |
| `0x141f45090` | | deserializer of `0x0271` (`u8 × 4`); handler not found |
| `0x142156320` | | `0x00c8` PartyUpdate reader (`0x1421560b0` ctor); `0x1421574b0` `0x00d9` TradeState reader; `0x141f43e80` `0x00c9`; `0x141f38d80` `0x00da`; `0x141f38ce0` serializer of `0x00d2`/`0x00c9`; `0x141f3cd50` `0x00e0`; `0x142150da0` `0x0019` |

### Party invitation and trade request, receiving side (WP C3, 2026-10-21)
Layouts and the flow: [../protocol/entities.md](../protocol/entities.md#party-and-trade-flows-wp-c3). Renamed in Ghidra.

| Address | Ghidra name | What |
|---|---|---|
| `0x142106a30` | `Session_HandleAccountMessage` | the big message handler of the account session (switch on the opcode); case `0xc8` PartyUpdate (member roles: 2 leader, 1 member, 0 invited; own state from `members[+0x76]`), `0xc9` PartyEnded (current party id at session `+0x2278`, else the invitation record is erased), `0xd9` TradeState, `0xda` TradeAck, `0xdd`/`0xe0` inventory events |
| `0x1421063f0` | | member role byte to state: 0 → 1 (invited), 1 → 2 (member), 2 → 0 (leader), else 3 |
| `0x14210b630` | `Social_AddPartyInvitation` | adds an invitation record (`0xa0` bytes + flag `+0x2b0`, keyed by party id, vector at session `+0x7620`); raises the event list at `+0x5850` when the leader's social entry was found (flag set by `0x14210b740`) |
| `0x140feb9c0` | `PartyInvite_OnReceived_ShowPopup` | listener of that list: builds the popup (`0x140c75310`) and raises `FourSocialPartyReceive`; registered in `0x140fd7800` |
| `0x140febbe0` | | listener of the list at `+0x58d0` (member joined): raises `FourSocialPartyAccept` |
| `0x140c9ef70` | `PartyInvitePopup_Accept` | the popup's accept button: sends C→S `0x00b8` (`u8 local` from session `+0x181`, `u32` the popup's party id) with the generic `u8 + u32` class `0x1430a2fa0` |
| `0x140c9eee0` | `PartyInvitePopup_Decline` | decline button: raises `FourSocialPartyDecline`, calls `Social_SendPartyDecline` |
| `0x142110ab0` | `Social_SendPartyDecline` | sends C→S `0x00b9` (same class) unless the id is the party already joined with 2+ members |
| `0x1402c3fb0` | `TradeInvite_ShowPopup` | builds the popup with the string `TradeInviteFrom;` (`0x142fa5b00`) and the trade record; its result callback is `0x1402c3c30` (writes a bool into a promise) |
| `0x1402c3cb0` | `TradeInvite_OnTradeState` | listener of the `0xd9` event: popup only if trade id ≠ 0, record flags `+0x30` bit 1 set and bit 0 clear, and the character is in the social list (`0x142129580`, lookup by character name, case-insensitive) |

## Login results, flags, create and delete (WP B1, 2026-10-21)
Details and decoded fields: [../protocol/login.md](../protocol/login.md). Names are set in Ghidra.

| Address | Name | What |
|---|---|---|
| `0x1420e8391`..`0x1420e8427` | | case 4 of the login dispatcher: splits the two `0x0004` flag bytes into the ten booleans of the account entry (`+0x30`..`+0x39`, entry size `0x78`, vector at login session `+0xef0`) |
| `0x140ef5140` | `LoginState_AuthenticationResultHandler` | success path: ToU (entry `+0x32`), cheating (`+0x33`), RMT (`+0x34`), update required (`+0x35`), notice popup, or continue (`0x140ef62c0`) |
| `0x1425cb780` | `BackendError_GetText` | result code to text: `Data/Balance/BackendErrors.dat` row `code - 1` (language `0xb` is read directly, others through a language path); `bad-error-code` for an unknown row |
| `0x140f0f600` | `SelectScreen_UpdateStrictUnlockBanner` | builds/updates `strict_unlock_warning_frame` / `_label` (text `StrictUnlocksEnabledMessage`, id `0x10d4`); needs entry `+0x38` (flag byte 0 bit 6) |
| `0x140f05b20` | `SelectScreen_ShowWidgets` | select-screen enter: on a non-Steam platform shows the strict-unlock banner when an entry has `+0x38` and screen `+0x4b0` is clear |
| `0x140ec7700` | | (create-screen range) after a successful unlock: deletes the credentials file, sets screen `+0x4b0 = 1` when an entry has `+0x38` |
| `0x140effac0` | | login-screen widget shown when an entry has `+0x31` |
| `0x1406c19a0` | | league start time choice, uses entry `+0x31` and league `+0xe0` bit 5 |
| `0x1406c08b0` / `0x1406c7ae0` | | select-screen refresh: hides widget `+0x6e0` when entry `+0x39` is set |
| `0x1427164c0` | `CharacterEntries_Read` | reads the character entries of `0x0015` (`0x58`-byte struct, order in login.md) |
| `0x1406b9b90` | `SelectScreen_SetEntry` | fills one select-screen row from an entry; calls the next three |
| `0x1406b8460` | `SelectScreen_SetEntryPortraitFrame` | entry `+0x44`: `DefaultIconFrame` or `MTX/SupporterIconFrame<id>` |
| `0x1425ff110` | `CharacterPortrait_GetIconPath` | entry `+0x4e` bits 0-4 class, bits 5-7 ascendancy; entry `+0x46` portrait variation |
| `0x1406b9010` / `0x1406b8e90` | `SelectScreen_SetEntryLocation` / | entry `+0x50` (MiniQuestStates row), areas `+0x48` / `+0x4a` (WorldAreas lookup) |
| `0x140f0c090` | `SelectScreen_OnCharacterDetails` | caches the preview event (31 gear slots, two `u16`, pairs, shapeshift form) per character index |
| `0x1420e8534`..`0x1420e8936` | | `0x0015` handler: stores the list in the account entry (`+0x40` vector, `+0x58`/`+0x59` bytes), builds the preview event; signal at login session `+0xd08` |
| `0x14214ea70` / `0x1420e895e` | | `0x0013` CharacterCreated reader / handler (signal `+0xb08` with account id and result; the `notify` byte makes `0x1420e89c1` build an `/api/tracking-conversions/` request) |
| `0x140ed2ab0` | `CreateCharacterState_OnCharacterCreated` | create-screen callback: result 0 starts the creation scene, else `BackendError_GetText` into the message line `0x140ec96b0` |
| `0x140ed8149` | | `0x000c` DeleteCharacter builder (name, `u8` from screen `+0x214`, `u32` account id from `+0x210`) |


## Pathfinder (2026-10-21)
Algorithm and evidence: [pathfinding.md](pathfinding.md). Signature `path_plan` (feature `path_log`) in `modules/client-hook/signatures/0.5.5e.toml`.

| Address | Name | What |
|---|---|---|
| `0x141ed0390` | `Pathfinder_Plan` | the grid search (bounded jump point search, ellipse window of 272 x 272 cells, MSVC heap, line-of-sight thinning); `(goal*, entity, params*, ..., out path*, out count*, out flag*)`, hooked by `path_log` |
| `0x141ece050` | | the A*-style fallback it calls when the main search is skipped (monsters/pets, or when its window test fails) |
| `0x141ed28a0` / `0x141ed21b0` | | window setup: marks cells with dynamic obstacles (other entities) as walls |
| `0x141ed2c70` | | classifies a window cell: 0 free, 1 wall (`level >= value`), 2 outside the terrain or the ellipse |
| `0x141ed2f20` / `0x141ed3390` | | straight / diagonal jump with the jump limit `min(cap, 3 + 5 * alignment + 0.2 * excess)` |
| `0x141ed2e30` | | whether the neighbour in a direction is a wall (forced-neighbour test) |
| `0x141ecdef0` | | `h` of a cell: distance adjusted by min/max/radius plus `0.001 * |cross|` |
| `0x141edc7f0` / `0x141edc560` | | the line-of-sight tests of the thinning passes (Bresenham over the grid) |
| `0x141ed3760` | | whether a cell is walkable and not marked a wall in the window |
| `0x141d0ddd0` | | reads a cell's 4-bit value of layer `n` (value 5 reads as 255) |
| `0x141dc7b80` | | the entity's pathing level from Positioned `+0x1e8` (sizes `+0x1f8/+0x1f9`, extra bytes `+0x1fc..+0x200`) |
| `0x141b76f90` / `0x141b77220` | | sprint/roll action start sets / end clears the larger pathing size (level 4 for characters) |
| `0x142193120` | | world vector to cells: `floor(v * 0.092)` (cell = 250/23 = 10.87 world units) |
| `0x141ec8520` / `0x141ec8b30` / `0x141ec8160` | | the client's own line (digital line) from cell to cell that trails the position by up to half a cell; steps it by the frame's distance |
| `0x1419c5810` / `0x1419c56a0` / `0x1419c5900` | | open-list push / pop (MSVC `push_heap`/`pop_heap` on `f`) and the node array `0x1445d8920` (16-byte nodes, 0x110 x 0x110) |
| `0x14212eb60`, vfunc `+0x1d8` | | world constructor; vfunc `+0x1d8` returns 2 for frame mode 2 (lockstep) |
| `0x1421302c0` | | frame playback: speed factor 1.01 with one spare frame queued (the client's steady walking step is 1.2333 = 1.01 x 1.221; short steps when it waits for the next frame) |

## In-client batch generation (`gen_batch`, 2026-10-22)
Details, field meanings and the exception handling: [terrain/batch.md](terrain/batch.md). Signature entries (feature `gen_batch`) in `modules/client-hook/signatures/0.5.5e.toml`.

| Address | Name | What |
|---|---|---|
| `0x14167d2f0` | `world_construct_client` | `ClientWorld` constructor `(this, u8 frame mode, u8 flag)`, object size `0x17e8`; vtable `0x14339cf18` |
| `0x14212eb60` | | base world constructor (called by the ClientWorld one only); vtable `0x1435082f0`, vfunc `+0x1e0` is `HeadlessWorld_LoadArea` |
| `0x141fefa20` / `0x141fefe40` | `terrain_gen_construct` / `terrain_gen_destroy` | base generator constructor `(generator, world)` / destructor; generator `+0xe` (u8, 1 after construction) enables the tile map validation pass, which `ClientWorld_LoadArea` clears |
| `0x141ff9280` | `terrain_topology_pick` | generator vtable `+0`: reads stats `0xb92` (forced topology, 1-based, `<= row +0x60`) and `0xb93` (forced orientation + 1), world `+0xb8` (league mode); `0x141ff90f0` picks the graph |
| `0x141ffdab0` / `0x141f9df00` / `0x141f9d9c0` | | validation pass (only when generator `+0xe`) / driver / "Invalid TileMap: ..." checks |
| `0x1423ba400` / `0x1423ba2a0` / `0x1423ba350` | `flat_map_set` / get / contains | sorted `(i32 key, i32 value)` map at `container +8 .. +0x10`; the world stats at world `+0x128` |
| `0x140223710` | `world_areas_table` | `Data/Balance/WorldAreas.dat` table getter `(out)` -> counted reference; table `+0x28` -> `(begin, end)`, 442 rows of `0x2e0`, row `+0` id string pointer, `+0x2a` hash16, `+0x60` / `+0x68` topologies, `+0x1b4` kind |
| `0x1421d6e30` | `resource_wait_loaded` | blocks until resource state `+0x40` is 2 |
| `0x142bba3c0` / `0x142bba2a0` | `game_alloc` / `game_free` | the game's allocator (per-thread heaps, free from any thread) |
| `0x142c14288` | `vcrt_getptd` | the statically linked C++ runtime's per-thread data; `+0x58` chain of catch-block frame infos (`_IsExceptionObjectToBeDestroyed` `0x142c13a00`) |
| `0x141eff290` | | Incursion temple room object constructor: reads the `WorldAreas` table from world `+0xa0` |
| world `+0x98` / `+0xa0` / `+0xb8` | `world.area` / `world.area_table` / `world.league_mode` | area row / counted table reference / league mode |
| world `+0x4b0` / `+0x4b4` / `+0x663` / `+0x8d0` | `world.gen_state` / `world.gen_seed` / `world.gen_flag` / terrain | generation state (u32 first LoadArea argument; seed) / headless flag / the embedded terrain object |
