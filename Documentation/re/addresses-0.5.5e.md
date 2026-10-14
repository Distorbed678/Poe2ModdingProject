# RE findings: PathOfExileSteam.exe 0.5.5e

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
| `0x142b030b0` | Crypto++ Salsa20 key setup | |
| `0x142102c20` | loader for server-delivered encrypted code modules | see re/crypto.md |

## Imports of interest
- `VERSION.dll`: `GetFileVersionInfoW`, `GetFileVersionInfoSizeW`, `VerQueryValueW`. These are proxied by poe-hook.
- `WS2_32.dll`: sockets. `bcrypt.dll` and `CRYPT32.dll` are present. The crypto strings in the binary (CALG_*, TLS_*, RSA) belong to bundled curl/OpenSSL for HTTPS. The game protocol cipher has no string anchors.

## Client log lines (GameFiles/logs/Client.txt)
- `Connected to <host> in Nms.` is the login connection.
- `Got Instance Details from login server` is the hand-off.
- `Connecting to instance server at <ip>:<port>`.
- `Generating level N area "<AreaId>" with seed <u32>`: the client generates terrain itself.

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
| `0x14218a850` | Actor skill container reader; `0x14218afb0` skill list; `0x142188bd0` one skill |
| `0x142683e50` / `0x142683b70` | skill effect reference reader (GrantedEffectsPerLevel row) / stat-set reference (GrantedEffectStatSetsPerLevel row) |
| `0x1419970a0` / `0x141e25d50` | varuint / varint readers used in stat lists |
| `0x141dcd020` | Pathfinding component constructor (+0x550 base speed, +0x55c multiplier) |
| `0x141dcce20` | movement speed getter (base × movement_velocity_+permyriad × virtual_action_speed_+%) |
| `0x14212eb60` | world constructor (+0x661 frame mode; in-game world created with 2 by `0x14100b7e9`) |
| `0x141939e40` | `[NETWORKING] networking_type` option reader |
| `0x142120b70` | game session update: frame reports, auto network-mode switch, world update |
