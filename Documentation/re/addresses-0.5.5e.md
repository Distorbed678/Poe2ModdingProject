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
| `0x1421306c0` | world frame drain (`world, f32 dt`): applies queued simulation packets, updates entities |
| `0x1421370f0` | frame-mode entity update loop (component vfunc +0x50, then `0x141e7d7f0`) |
| `0x1421340e0` | simulation packet queue (`0x2f4` frame marker, `0x2f5` → world vfunc +0x178) |
| `0x140150cb0` | entity → Render component (by name) |
| `0x1416a9810` | Render: apply visibility to the AO scene node (+0x39a disabled flag) |
| `0x141923f80` | GEAL `EnableRendering` lambda body |
| `0x141d439e0` | animation controller: play animation (index, …) |
| `0x141dfa320` | StateMachine reader (shared-state count check) |
| `0x1426f2220` | GEAL script interpreter loop |
| `0x140286850` | `Data/Balance/Hideouts.dat` table getter |
