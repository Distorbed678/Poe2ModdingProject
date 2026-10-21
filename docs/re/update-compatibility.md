# Update compatibility: what breaks when GGG ships a new client build

Assessment for WP A1 of [../plans/orchestration-2026-10b.md](../plans/orchestration-2026-10b.md). Analysis only: no code changed. Measured on `GameFiles/PathOfExileSteam.exe` (client 0.5.5e, 2026-10-21). The proposals feed WP A2 (signatures), A3 (`cargo xtask check-build`) and A4 (version plumbing).

## Summary
- **Today the DLL cannot tell which build it runs in.** It checks no version, no hash and no build number. Every address is a fixed Ghidra address of 0.5.5e. Only one feature (`crypto_bypass`) finds its target by a byte pattern.
- **Hooks are installed without checking the target's bytes.** `hooks::install` only saves 16 bytes for the undo (`modules/client-hook/poe-hook-core/src/hooks.rs:45`). The research hooks (`dump_object_types`, `area_export`, packet capture, `crash_log`) are **on by default, also in official read-only mode** (`poe-hook-shared/src/config.rs:125-139`). A stale address crashes the client, also on an official observation session.
- **The login patch check is weak.** It compares 3 bytes. In the 0.5.5e exe, 1,167 functions start with `40 55 53` and 6,590 start with `48 8b c4`. After an exe rebuild, another function may sit at the old address, and it would be overwritten with `xor eax,eax; ret`.
- **The server never learns the client version.** `game.version` in `config/server.toml` is a label. The opcode folder, the DLL's compiled-in tables and `dumps/0.5.5e` all name the version by hand, in separate places.
- **The exe sha256 in the launcher file is correct.** `7157984534da1b146e9b0be342aa9fd24272915b6f478aadef9830fc7873f89f` matches `GameFiles/PathOfExileSteam.exe` (78,335,832 bytes). The five key-file sizes also match. It is still unconfirmed against a real depot download, and `exe_sha256_enforce = false`.
- **Good news for signatures.** Prologue signatures with wildcards over the `rel32`/`disp32` fields and the stack-frame immediates are unique in `.text` for 44 of the 47 addresses I tried (details below). The build also carries a plain `323943` string in `.rdata` (the `Build Revision` that `Client.txt` prints), a cheap version fingerprint.

## How to read the table
Patch kinds, and the likelihood of breakage for each:
- **D** = data-only patch. `Bundles2` changes, the exe is byte-identical (balance, new items, atlas changes).
- **H** = hotfix with a rebuilt exe. Functions move and stack frames change; most function bodies stay the same. 0.5.5**e** is the fifth such letter, so this is the common case.
- **M** = major patch (a new league, 0.6). Messages, components and generator passes change.

Ratings are L (low), M (medium), H (high), written D/H/M. Detection: **easy** = fails at start-up with a log line or a crash; **hard** = the game runs and something is wrong later; **silent** = nothing is logged.

## Ranked table (highest risk first)

| # | Item | Break D/H/M | Failure mode | Detection | Proposed fix |
|---|---|---|---|---|---|
| 1 | DLL hooks at fixed addresses, no byte check: `dump_types`, `terrain_tools`, `packets`, `protocol`, `side` (about 45 sites) | L/H/H | Crash on a wrong target, also in official mode; or a hook that never fires | easy for a crash at start-up, hard for crashes at area load | Signature table, resolved once, verified, per-feature disable (S1) |
| 2 | Mid-function hook sites: `FLUSH_PLAINTEXT` `0x142198037`, `ROOM_CANDIDATES` `0x141f8ed45`, `DOODAD_HASH_SITE` `0x141ff3643` | L/H/H | Hook lands inside an instruction, crash; or the registers hold other values (silent bad data) | hard | `site` signatures: anchor + offset + expected bytes, instruction boundary checked (S1) |
| 3 | Login patch: `ClientPlatform_Detect` `0x142196ec0`, `StandalonePatcher_Run` `0x1401162d0`, platform cache `0x144717114/18` | L/H/H | Wrong function patched (3-byte check), or the Steam login screen stays, or a 4-byte write to a wrong global | easy if the screen is wrong; hard if corrupted | Long unique signature + expected bytes; derive the cache addresses from instructions (S1, S3) |
| 4 | Side-channel key hook `0x14212aa90` (`side.rs:29`) and the argument order `(out, material, &is_client)` | L/H/M | Wrong target crashes. Wrong argument register gives wrong keys, so the server drops every report (silent) | hard | Signature; check the 64 bytes against the server's own session key at the first report; log mismatch |
| 5 | Crypto bypass: prologue pattern (`crypto_bypass.rs:23`), key buffer `+0x60` (`:28`) | L/H/M | Pattern fails: logged, login fails (refused). `+0x60` wrong: handshake garbage, connection drops | easy | Keep the pattern, wildcard stack immediates; derive `+0x60` or test it |
| 6 | Opcode tables `modules/protocol/opcodes/0.5.5e/*.toml` (login 22 named, instance 75 named, `client_body_sizes`) | L/M/H | A message id means another message: wrong parse, or "Unable to deserialise packet" (the client drops the link); redaction by name stops working | hard | `check-build` opcode drift report by deserializer fingerprint (S4); per-version opcode dir |
| 7 | Message layouts in `poe-proto` (`login.rs`, `instance.rs`, `skill.rs`, `remote.rs`) | L/M/H | Fields shift: garbage values or a client disconnect; the client does not check a message is fully consumed | hard | Replay tests against official captures per build; fingerprint of each reader function (S4) |
| 8 | `InitialState` / `AddObject` blob layouts and the component order (`entity.rs`, `player_blob.rs`, `docs/protocol/entities.md`) | M/M/H | Components misread; wrong stats, a crash in the client's entity code; component order comes from the `.ot` data | hard | Reader-span self-test with the DLL's `entity_trace` (zero blob, measure spans); order from `.ot` instead of code |
| 9 | Struct offsets read by the DLL (Positioned, Actor, Entity, stream, world, terrain; list in section 2) | L/H/M | `position_sync` reports garbage: the server sees drift (silent); exports wrong | silent / hard | Derive from instruction operands (S2); sanity checks (position inside the terrain) |
| 10 | Hard-coded component indices: Positioned = 0, Actor = 9 (`position_sync.rs:87-90`, `actions.rs:50`, `dump_types.rs:508`) | M/L/M | Reads another component's memory | silent | Look the index up by name with `entities::component_names` once per entity |
| 11 | Data row indices used as keys: `sprint_buff_rows` `[2758, 2737, 2853]` (`config/server.toml:48`, `poe-config/src/lib.rs:444-459`), `Stats` row + 1 (`views.rs:175-179`), `Inventories` row = slot (`views.rs:136-154`), `ActionTypes` 2nd column (`views.rs:182-189`), `Move` `0x20002909` (`poe-game/src/world.rs:258`) | H/M/M | Wrong buff, stat or slot ids: the wrong sprint effects, broken inventories, wrong action names | silent | Store ids not rows; resolve at start; `check-build` anchors (S4) |
| 12 | dat schema `third_party/dat-schema` (rolling `latest`, unpinned, `fetch-tools.sh:15-17`) and table columns | H/M/M | Column names or widths drift. A wider schema only warns (`dat.rs:97-103`); positional access reads wrong data | hard | Pin the schema release per version; `check-build` compares every used column with row width (S4) |
| 13 | No version plumbing: `game.version` is a label; `v0_5_5e` module (`poe-proto/src/opcodes.rs:11`); `0.5.5e` in the login-server tests, `xtask`, config defaults, `dumps/0.5.5e` | M/M/M | Mixed tables from two builds; dumps written into the wrong folder; nobody notices | silent | `versions/<ver>.toml` as the single source (S5); refuse unknown builds |
| 14 | Terrain generator port (`poe-world`, 14,800 lines, 500 address refs) and data formats (`.tgr` version 25, `.dgr`, `.tdt`, `.ot`) | M/M/H | Layout differs from the client's: walls and spawn are wrong. Data patches change tile sets, so layouts change even with an unchanged exe | hard, but the client logs `Tile hash` / `Doodad hash` as an oracle | Re-run vectors per build; `check-build` reports stale `terrain-vectors.tsv` by its `build` column |
| 15 | Terrain export hooks and offsets (`area_export`: world `+0x98/+0x1b4/+0x114/+0x118`, terrain `+0x18/+0x20/+0x28`, tile record `0x38`, grid `+0xd0/+0xe8/+0x100/+0x118`) | L/H/M | Wrong layouts uploaded to the server | easy: the export re-checks the client's own tile hash (`terrain_tools.rs:1050`) | Keep the hash check; make a failed check disable the upload |
| 16 | HNC table: `key_xor` and `swaps` (`opcodes/0.5.5e/hnc.toml`, exe `0x142f10960`, `0x1421c50b0`); Salsa20 key/IV split (`poe-net/src/cipher.rs:40`) | L/L/M | Our side of the HNC channel cannot decode (the client disconnects after `0x1c`) | easy | `check-build` re-extracts the 64 bytes from the exe and compares |
| 17 | Packet capture stream offsets (`packets.rs:333-440`, `protocol.rs:102-123`) and caller ranges `LOGIN_READER`/`INSTANCE_READER` (`packets.rs:41-42`) | L/H/M | Captures show wrong messages or wrong channel; no crash (guarded reads) | hard | Derive offsets from the stream functions' operands; channel from `.pdata` function containing the return address |
| 18 | Packet redaction by opcode name (`packets.rs:198-211`) | L/L/H | After a renumber, secrets are written to `packets.jsonl` | silent | Redact all outgoing login bodies on an unknown build (see surprises) |
| 19 | Launcher `versions/*.toml`: depots, manifests, sha, key sizes (`modules/launcher/versions/0.5.5e.toml`) | L/H/H | Old manifest may be removed by Steam (download fails); `verify` only warns; Play does not check the exe against the hook | easy (verify report) | One file with `status`; Play compares the exe sha/build with the DLL's table (S5) |
| 20 | `symbols.tsv` (585 names) and `addresses-0.5.5e.md` | L/H/M | Misleading names in crash logs and the Protocol page | hard (just confusing) | Embed the table by build id; header with the build; `export_symbols.py` takes a version |
| 21 | `Client.txt` parsing: `Generating level ... with seed`, `Tile hash:`, `Doodad hash:`, `Build Revision:` (`scripts/re/clientlog_terrain.py:13-16`, `clientlog_entries.py:13-19`, DLL `gamelog.rs`) | L/L/M | Scripts give 0 vectors; the Game log page empties | easy (count printed) | Keep; use `Build Revision` as the version key |
| 22 | Command-line options `--success-event`, `--disable-client-switching` (`login.rs:38-41`, `login_patch.rs:64`) and `config.json` / credentials file name | L/L/M | The patcher window appears and talks to GGG patch servers (the guard blocks external connects in private mode) | easy | Signature for `StandalonePatcher_Run` (row 3) is the safety net; check option strings with `check-build` |
| 23 | Data pointer `0x1445d8308` (game window handle) in `auto_keys.rs:473` and `trace_calls` / `POE_HOOK_TRACE` addresses | L/H/L | Test injection only: wrong focus for the aim check. Tests only | easy | Resolve by pattern or drop |
| 24 | Scripts with fixed addresses: `scripts/re/dump_factory.py` arguments (`d 35f 217305c 142173040`), `export_symbols.py` file names | L/H/L | The tool dumps wrong tables | easy | Find the factory by the string `Unable to deserialise packet` (S4) |
| 25 | Win32 and Vulkan hooks (ws2_32 `connect`, `GetRawInputData`, Vulkan loader), `version.dll` proxy exports | L/L/L | None expected | easy | None |
| 26 | `poe-hook-deploy` constants: `PathOfExileSteam.exe`, `gateway_list.txt`, app id `2694490` (`lib.rs:39-51`) | L/L/L | Deploy fails or copies to the wrong file | easy | Take from `versions/<ver>.toml` |

## 1. Fixed Ghidra addresses

### 1.1 Where they live
| Group | Files and lines | Addresses |
|---|---|---|
| Login patch | `poe-hook-shared/src/login.rs:13-35`, used by `poe-hook/src/login_patch.rs:85,121,141-142` | `ClientPlatform_Detect` `0x142196ec0`, cache guard/value `0x144717114/18`, `StandalonePatcher_Run` `0x1401162d0` |
| Crypto bypass | `poe-hook-core/src/crypto_bypass.rs:23` | pattern only (no address); same function as `0x14219f010` |
| Side channel | `side.rs:29,45-50` | `0x14212aa90` |
| Packet capture | `packets.rs:37-42`, `protocol.rs:31-32` | `0x1421c5950`, `0x142198037` (mid-function), `0x14211f880`, `0x142197d30`, `0x142197e60`, caller ranges `0x1420e6bc0..0x1420e7136`, `0x142120b70..0x1421215b5` |
| Entity and world research | `dump_types.rs:27-62` (installs `:67-95`, calls `:668-696`), `entities.rs:19` | `0x1416936c0`, `0x141e71120`, `0x141e71090`, `0x14010a080`, `0x14167b270`, `0x14212f500`, `0x143385b10` (vtable), `0x141dd8820`, `0x1421306c0`, `0x141e7d7f0`, `0x141ddc6a0`, `0x14167ea40`, `0x140150cb0`, `0x1416a9810`, `0x141de5390`, `0x141d0d420` |
| Terrain export and trace | `terrain_tools.rs:40-121` (installs `:171-200`) | 32 more, including `0x141ffe940`, `0x141ff7770`, `0x141ff3643` (mid-function), and the whole generator trace set |
| Window handle | `auto_keys.rs:473` | data `0x1445d8308` |
| Trace | `trace_calls.rs:29`, `POE_HOOK_TRACE` | any address typed by the user |
| Symbols | `symbols.rs:24` (`assets/symbols.tsv`, 585 lines) | names for Ghidra addresses |
| Docs | `docs/re/addresses-0.5.5e.md` (248 lines), `docs/re/terrain/*.md`, `docs/protocol/*.md` | the knowledge itself |
| Server crates | 500 address mentions in `crates/poe-world` and 139 in `modules/protocol` | comments only: they say where the algorithm came from |

The server crates have no runtime dependency on an address. After a patch, those comments become wrong, but nothing breaks. They are the starting points for re-deriving a changed algorithm.

### 1.2 What the code does today when an address is wrong
- `hooks::install` copies the first 16 bytes and calls `ilhook` (`hooks.rs:45-58`). `ilhook` disassembles the bytes at the target. If the target is not an instruction boundary, it relocates garbage. There is no verification of expected bytes.
- `report()` in `lib.rs:120-130` logs an error when `install` returns `Err`, but `install` returns `Ok` for almost any readable address.
- The login patch is the only place with an expected-bytes check (`login.rs:19,30`). It checks **3 bytes**. Measured: 1,167 functions begin with `40 55 53` and 6,590 with `48 8b c4`. The check proves the code looks like a function start, not that it is this function.
- `login_patch.rs:141-148` reads the cache guard at `0x144717114` and, when it is non-zero, writes 0 to `0x144717118`. If only data moved, this is a stray 4-byte write.
- `crypto_bypass` already uses a 40-byte prologue pattern, requires exactly one match, and fails with a log line. This is the model for the others. Its stack immediates (`48 81 ec 90 02 00 00`) are not wildcarded, so a hotfix that changes the frame size breaks it.

### 1.3 Uniqueness test (offline, no Ghidra)
I disassembled each address in the exe with capstone, wildcarded the `rel32`/`disp32` of calls, jumps and RIP-relative operands, wildcarded `imm` values that look like image addresses, and grew the pattern one instruction at a time until it was unique in `.text`. A second run also wildcarded `sub rsp, imm` and `lea reg, [rsp+disp]`.

| Result | Count |
|---|---|
| Unique with wildcards over displacements | 44 of 47 |
| Unique when stack-frame immediates are also wildcarded | 44 of 47 |
| Not unique within 40 instructions | 3 |

The three that are not unique are template-like clones: `0x1416936c0` (object-type lookup, unique only after 86 instructions, 310 bytes), `0x140150cb0` (Render component by name, unique after 50 instructions), `0x141d0d420` (terrain height; two copies). They need an anchor: a unique caller, a string, or a constant (the `0x30` entry size for the lookup).

Unique patterns for the load-bearing addresses (bytes shown for 0.5.5e; `??` is a wildcard):

| Name | Ghidra | Length | Pattern |
|---|---|---|---|
| `ClientPlatform_Detect` | `0x142196ec0` | 27 | `40 55 53 56 57 41 56 48 8d ac 24 ?? ?? ?? ?? 48 81 ec ?? ?? ?? ?? 48 8d 4c 24 ??` |
| `StandalonePatcher_Run` | `0x1401162d0` | 33 | `48 8b c4 48 89 58 10 4c 89 40 18 48 89 48 08 55 56 57 41 54 41 55 41 56 41 57 48 8d a8 18 fa ff ff` |
| side-channel builder | `0x14212aa90` | 41 | `48 89 5c 24 10 48 89 6c 24 18 48 89 74 24 20 48 89 4c 24 08 57 48 83 ec ?? 49 8b f0 48 8b ea 48 8b d9 33 ff b9 10 00 00 00` |
| verify-and-derive | `0x14219f010` | 29 | `40 55 53 56 57 41 54 41 56 41 57 48 8d ac 24 ?? ?? ?? ?? 48 81 ec ?? ?? ?? ?? 4d 8b f0` |
| message reader | `0x1421c5950` | 29 | `48 8b c4 48 89 50 10 48 89 48 08 53 56 57 41 54 41 55 41 56 41 57 48 81 ec 90 01 00 00` |
| session send | `0x14211f880` | 33 | `48 89 5c 24 10 48 89 6c 24 18 48 89 74 24 20 57 48 83 ec 30 48 8b f2 48 8b e9 48 8b b9 f8 20 00 00` |
| stream read | `0x142197d30` | 29 | `48 89 5c 24 08 48 89 6c 24 10 48 89 74 24 18 57 48 83 ec 60 49 8b f8 48 8b ea 48 8b d9` |
| stream write | `0x142197e60` | 27 | `48 89 5c 24 08 48 89 6c 24 10 48 89 74 24 18 57 48 83 ec 40 48 8d b9 80 02 00 00` |

Cautions the table hides:
- **Neighbours with the same prologue.** `0x14212ab00` (the HNC builder) starts with the same `mov [rsp+0x10], rbx` as `0x14212aa90`. Only the later instructions (`b9 10 00 00 00`, the `call`) separate them. Short prefixes are not enough. Always check uniqueness, and always report the match count.
- **Short patterns are lucky.** `SET_ORIENTATION` is unique at 12 bytes (`40 53 48 81 ec 60 01 00 00 0f 28 c1`) and `FRAME_DRAIN` at 11 bytes. They hold only if the stack frame stays the same. Require at least 24 bytes, or include body instructions.
- **Mid-function sites** (`0x142198037`, `0x141f8ed45`, `0x141ff3643`) are unique at 11 to 21 bytes because I started the pattern at the site itself. That is accidental. Anchor them to the enclosing function's signature plus an offset, and check the expected bytes at the site (for example `48 8b 93 60 02 00 00 48 8b bb 58 02 00 00` for the flush site).
- **String anchors survive a rebuild better than prologues.** Good ones: `"General Rhoa Failure #3"` (verify-and-derive), `"HNC Protocol Violated"`, `"Generating level"`, `"Tile hash"`, `"Doodad hash"`, `"Unable to deserialise packet"` (message reader and factory), `"Connecting to instance server at {}:{} "`. A `string_xref` signature kind (find the string in `.rdata`, find the `lea` that references it, take the function start from `.pdata`) should be the fallback when a prologue is ambiguous.

### 1.4 Order of work for A2
The login patch, the side channel and the crypto bypass act on our own sessions. Packet capture, entity and terrain research also run on official sessions, where a bad hook costs the observation. Suggested order: login patch pair, crypto, side channel, then packet capture, then the entity and terrain research hooks.

## 2. Struct offsets

| Struct and field | Offset | Used in | Production or research | Derivable from an instruction? |
|---|---|---|---|---|
| Entity: component vector | `+0x10`/`+0x18` | `entities.rs:75,167`, `position_sync.rs:87`, `actions.rs:50`, `inspect.rs:29` | `position_sync` is production | Yes: the `GetComponent` by name function `0x140150cb0` and the entity update loop read them |
| Entity: id, type, world | `+0x88`, `+8`, `+0x78` | `entities.rs:118,143`, `dump_types.rs:212,637` | research | Yes: `Entity_ReadInitialState` and `Entity_Destroy` use `+0x88` |
| Entity: type's component lookup | `+8` → `+0x28`, array `+0x28..+0x30` of 16-byte `{char*, i32}` | `entities.rs:143-168` | research | Yes: from `0x140150cb0` |
| Positioned: grid | `+0x444`, `+0x448` | `entities.rs:133`, `inspect.rs:41`, `dump_types.rs:487` | production (`dump_types` feeds position sync) | Yes: `lea rbx,[rsi+0x444]` at `0x141dd8922` in the reader |
| Positioned: world | `+0x490`, `+0x494` | `position_sync.rs:94-95`, `entities.rs:134` | **production** | Yes: `mov [rsi+0x490], ecx` at `0x141dd8c19` |
| Positioned: facing, lock | `+0x44c`, `+0x1e5` bit 1 | `entities.rs:135`, `inspect.rs:43-44` | research | **Yes, best example:** `SetOrientation` has `f6 81 <D1:4> 02` then later `c7 81 <D2:4> 00 00 00 00`; unique as a 37-byte pattern (tested) |
| Actor: WASD direction | `+0x400`, `+0x404` | `inspect.rs:48-49`, `dump_types.rs:520` | research | Probably: stores in the `0x64` case of `0x142149640`; to be proved in A2 |
| Actor: action slots, action type | `+0x238`, `+0x240`; action `+0x120` | `position_sync.rs:99`, `actions.rs:50-58` | **production** | Probably: `0x14210f920` / `0x141e4f9c0` |
| Actor: sprint key bit | `+0x2a2` bit 5 | `position_sync.rs:103` | **production** | Probably: the `0x2e7` handler |
| Player pending-skill slots | `+0x214` (and `+0x22c`) | docs only (`docs/protocol/movement.md`, `addresses-0.5.5e.md:181`) | none at run time | n/a, no code reads it |
| Stats component, set containers | `+0x1c8`, `+0x1e8/+0x1f0`, container `+0xd8/+0xf8/+0x100` | `inspect.rs:95-119` | research | Yes: `Stats` handler `0x142181f20` |
| World: frame mode, pause, frame queue | `+0x661`, `+0x868/+0x869/+0x86a`, `+0x6a8` | `dump_types.rs:169-199,651-661`, `timing.rs` | research | Yes: frame drain `0x1421306c0` |
| World / area: area row, kind, tile hash, doodad hash | `+0x98` → `+0x1b4`, `+0x114`, `+0x118` | `dump_types.rs:345-349`, `terrain_tools.rs:262` | **production** (`area_export`) | Yes: `ClientWorld_LoadArea` / `TerrainGen_Run` store them |
| Generator object | `+0x28`, `+0x30` (seed at `+4`), `+0x38` | `terrain_tools.rs:255-256` | production | Yes: `TerrainGen_Run` |
| Terrain: size, tile array, tile record | `+0x18`, `+0x20`, `+0x28`, record `0x38` | `dump_types.rs:353-385`, `minimap.rs:24-25` | production | Yes |
| Terrain: cell grids | `+0xd0`, `+0xe8`, `+0x100`, `+0x118` | `dump_types.rs:435` | production | Yes, from the tile reader |
| Network stream | `+0x252`, `+0x258`, `+0x260`, `+0x268`, `+0x278`, `+0x280`, `+0x298`, `+0x2a0`, session `+0x20f8` | `packets.rs:333-440`, `protocol.rs:102-123` | production (capture) | Yes: stream read/write functions use them directly (the session-send signature above already contains `+0x20f8` as `f8 20 00 00`) |
| Crypto session: key buffer pointer | `+0x60` | `crypto_bypass.rs:28` | **production** | Partly: found at the SHA-512 output copy inside the routine; verify with a test (the server's key must agree) |
| Render component: disabled flag | `+0x39a` | `dump_types.rs:670` | research (probe) | Yes |

How to derive an operand safely (A2):
1. The signature has a named capture over the displacement bytes, for example `c7 81 <facing:4> 00 00 00 00`.
2. The table says the capture must be in `0x100..0x2000` and 4-byte aligned.
3. Where the same field appears twice (grid in the reader and the setter), require that both captures agree. Disagreement disables the feature.
4. A runtime sanity check where possible. For Positioned: after the first player frame, the world position divided by the cell size must be near the grid position (`position_sync.rs:92`: `CELL = 250/23`). If not, stop reporting.

**Component indices** (Positioned 0, Actor 9) are not struct offsets. They come from the object template (`.ot`), so they change with data patches even when the exe is the same. `entities::component_names` already reads the names from the type's lookup table. Use the names once per entity and cache the index.

## 3. Opcodes and message layouts

### 3.1 Opcodes
- Files: `modules/protocol/opcodes/0.5.5e/login.toml` (35 lines), `instance.toml` (143 lines, 75 named opcodes), `hnc.toml`, `inventories.tsv`.
- Loaded by the servers from `[game] protocol_dir` (`config/server.toml:11`) and compiled into the DLL by `poe-proto/src/opcodes.rs:11-19` (`v0_5_5e`, hard-coded module name and `include_str!` paths).
- The client's factories are about 316 server-to-client instance opcodes and 90 more in the second factory (`dumps/0.5.5e/ghidra/instance_factory_s2c.json`, `instance_factory2_s2c.json`). We name 75.
- A wrong opcode table is a hard failure only for opcodes the client reads with a different class. An unknown opcode is fatal on the client (`docs/protocol/framing.md`).
- Opcode numbers are likely to move on a major patch (messages are inserted in the middle of the switch) and unlikely to move on a hotfix.

**Drift detection (A3).** `scripts/re/dump_factory.py` already maps each opcode to its class vtable and deserializer address. Extend it with a function fingerprint: the bytes of the deserializer with `rel32`/`disp32` wildcarded, hashed. Store `opcode, name, fingerprint` per version. On a new build: dump the factory, hash each deserializer, and match by fingerprint. Report: same opcode and fingerprint (fine), fingerprint found under another opcode (renumbered, with the new number), fingerprint missing (layout changed, needs RE). The factory itself must be found automatically. Today its arguments are typed by hand (`d 35f 217305c 142173040`, `dump_factory.py:5`). Find it from the string `Unable to deserialise packet` instead.

### 3.2 Layouts
- Rust types: `poe-proto/src/login.rs` (934 lines), `instance.rs` (1,669), `entity.rs`, `player_blob.rs`, `remote.rs`, `skill.rs`. The doc comments carry the reader addresses, so a changed layout is traceable.
- The client does not check that a message or a blob was fully consumed. A layout error does not fail loudly: later fields read garbage.
- **Oracle:** the replay tests (`servers/instance-server/src/replay_tests.rs`, `crates/poe-game/tests/replay.rs`) compare against official captures of 0.5.5e. They protect the current build only. For a new build they need new captures, which are the user's (Gate: official capture).
- **Cheaper oracle for blobs:** the DLL's reader-span trace (`entity_trace`) measures how many bytes each component reader consumes. Keep the 0.5.5e spans as a table (`docs/protocol/entities.md`); on a new build, run one session and diff the spans.
- `InitialState` and the character-state flags reader (`0x1420edaa0`) are the most delicate: they feed a lot of client state with few checks.
- `client_body_sizes` in `instance.toml:129-143` are sizes of messages we do not decode. They are in the same drift class as opcodes.

### 3.3 Crypto and framing
- Salsa20 split of the 64-byte key material (`cipher.rs:40`: key `[0..32]`, IVs `[32..40]` and `[48..56]`) is verified by the live handshake. A change would show at once as a login failure.
- HNC: `key_xor` and the six `swaps` are data (`hnc.toml`). I checked that the 64 `key_xor` bytes occur in the 0.5.5e exe (file offset `0x2f0f560`, Ghidra `0x142f10960`). A3 can search for the table by its value, and fall back to the rip-relative operand in `0x1421c50b0`.
- `wstring`/`astring`/varint codecs (`poe-net/src/codec.rs`) are core serialization code and change rarely. A test on a captured message detects a change.

## 4. Game data: dat-schema, tables, row indices

- `poe-data` addresses columns by name (`lib.rs:5-6`), which is good. The schema comes from `third_party/dat-schema/schema-poe2.min.json`, downloaded from the rolling release `latest` (`third_party/fetch-tools.sh:15-17`). It is not pinned: two clones a week apart can differ, and a schema made for a newer game than ours can be wrong for our files. `poe_data_tools` is pinned (`Cargo.toml:26`, `4.0.1`), but the bundle format can change without notice.
- Width mismatches are only `tracing::warn!` (schema wider than the file, `dat.rs:97-103`) or `debug!` (narrower). The server starts and some columns read the wrong bytes.
- Tables and columns in use: `WorldAreas` (`Id`, `Name`, `Act`, `AreaLevel`, `IsTown`, `IsHideout`, `HASH16`), `Characters`, `CharacterStartStates`, `Inventories`, `LeagueFlag`, `Stats`, `ActionTypes`, `GrantedEffects`, `GrantedEffectsPerLevel`, `GrantedEffectStatSets`, `GrantedEffectStatSetsPerLevel`, `ActiveSkills` and others (`poe-data/src/views.rs`), the `.ot` templates (`template.rs`), `.tgr`/`.dgr`/`.tdt` (`poe-world`).
- Positional access (fragile): `get_index(1)` for the `ActionTypes` key (`views.rs:189`, `:287`).
- **Row index as a key** (each one breaks if a row is inserted before it):
  - network key of a stat = `Stats` row + 1 (`views.rs:175-179`; DLL `stats.tsv` from `xtask/src/main.rs:353-364`);
  - inventory slot = `Inventories` row (`views.rs:136-154`; 121 rows in 0.5.5e; `opcodes/0.5.5e/inventories.tsv` has 123 lines; `config/server.toml:151-158`);
  - `sprint_buff_rows = [2758, 2737, 2853]` (`config/server.toml:48`);
  - the `Move` skill id `0x20002909` is `0x20000000 | ActionTypes key (10505)`. The constant is also written in `poe-game/src/world.rs:258` (test config), `poe-game/src/locomotion.rs:1374`, `poe-game/tests/replay.rs:20`, `instance-server/src/replay_tests.rs:34`. Production code takes it from data (`views.rs:182-189`), the tests do not.
- `InstanceDetails` carries an area `HASH16` that must exist in `WorldAreas` (`docs/protocol/login.md:164`). A data patch that removes an area breaks any character saved there.
- Config values copied from captures: `first_entity_id = 12000`, `animation_sets` (`config/server.toml:42,215`), the two account flag bytes `[0x23, 0x03]` (`config/server.toml`, default in `poe-config`). They come from official behaviour and may change with the client.
- Bundle paths: `Bundles2/_.index.bin`, `data/balance/<table>.datc64` (`lib.rs:29,70`). `gateway_list.txt` at the game root is written by deploy (`poe-hook-deploy/src/gateway.rs`); the client also looks at `Bundles2/gateway_list.txt` (`addresses-0.5.5e.md:7`).

**Fixes:**
- Store ids in config and code, resolve rows at start (`sprint_buffs = ["<BuffDefinitions id>", ...]`; the ids can be read from the 0.5.5e rows now).
- Pin the schema: record the release tag or file sha256 in `versions/<ver>.toml`, and have `fetch-tools.sh` download that.
- `check-build` (A3) walks every `(table, column)` used by `poe-data` and the xtask exports. For each table: the column exists, the schema row width equals the file row width, the row count is plausible. Then the anchors: `Stats` has `actor_scale_+%`, `ActionTypes` has `Move` with key 10505, `Inventories` has 121 rows and `MainInventory` at the expected index, and the three sprint buffs exist by id.
- A width mismatch in a table the server uses should be an error at start, not a warning.

## 5. Version strings and where the version lives

| Place | What | Lines |
|---|---|---|
| `config/server.toml`, `config/server.example.toml` | `version`, `protocol_dir`, `layout_dir`, `known_seeds`, `sweep_results` all contain `0.5.5e` | `:5,11,54,72,175` |
| `crates/poe-config/src/lib.rs` | `[game] version` is a plain `String`; the default sweep path contains `0.5.5e` | `:329`, `:523` |
| `xtask/src/main.rs` | `dump_dir = dumps/<config.game.version>` | `:274` |
| `xtask/src/terrain_matrix.rs` | `VECTORS_DIR = "dumps/0.5.5e/terrain/vectors"` | `:30` |
| `modules/protocol/poe-proto/src/opcodes.rs` | module `v0_5_5e` and `include_str!` paths | `:11-19`, tests `:236-260` |
| DLL | `packets.rs:164,168` uses `v0_5_5e` | |
| `servers/login-server/src/auth.rs` | test path `opcodes/0.5.5e` | `:134` |
| `servers/instance-server/src/replay_tests.rs` | same | `:41,602` |
| `modules/launcher/versions/0.5.5e.toml`, `src-tauri/src/version.rs:7` | `EMBEDDED` list; `settings.rs:157`, `paths.rs:160-178` use `0.5.5e` in test paths | |
| `modules/launcher/src-tauri/src/mods.rs` | mod `versions = ["0.5.5e"]` compatibility check (`compat`) | `:930-1019` |
| Web API | `GET /public` shows `game.version` (`poe-web-api/src/public.rs:74`) | |
| Python tools | `dumps/0.5.5e/...`, `docs/re/addresses-0.5.5e.md` | `export_symbols.py:6-39`, `movement_sequences.py:41` |

Verified values (2026-10-21):
- exe sha256 `7157984534da1b146e9b0be342aa9fd24272915b6f478aadef9830fc7873f89f`: matches the launcher file.
- Sizes match: exe 78,335,832; `Bundles2/_.index.bin` 115,154,591; `steam_api64.dll` 262,944; `fmod.dll` 1,913,344; `bink2w64.dll` 466,560.
- Extra identifiers I found: PE `TimeDateStamp` `0x6ac302e0`, `SizeOfImage` `0x4ca0000`, and the string `323943` in `.rdata` (file offset `0x350b509`, next to the crash-report strings and `LatestClient.txt`). `Client.txt` of today's sessions prints `[ENGINE] Build Revision: 323943`; a session in May printed `304532`.
- Depot manifests `7366357142270402355` (content `2694491`) and `246779027001700092` (exe `2694492`) cannot be checked offline. The launcher file says they were not confirmed by a depot download. DepotDownloader can fetch an old manifest only while Steam keeps it.
- `exe_sha256_enforce = false` and Play does not run `verify` at all (`install_verify` is a separate command, `commands.rs:125`). Nothing stops the DLL from being deployed onto another build.

## 6. Terrain generator constants and data formats
- The port reproduces the client's generator: MSVC `mt19937` and `uniform_int` (stable library behaviour), MurmurHash2 string and tile hashes (`hash.rs`, `terrain_tools.rs:1016-1050`), and long algorithms (rooms, routing, smoothing). Constants (`0x38` records, 23-cell tiles, `250/23` cell size, `0x2000` steps in `smoothing.rs`) are stable per release. The algorithms change with game updates that touch levels. Tile sets and `.tgr`/`.dgr` come from the bundles, so a **data** patch changes layouts with no exe change.
- Format versions: `.tgr` is version 25 (`poe-world/src/graph.rs:1,111`); the parser reads the version number and branches on `> 22` / `> 23` (`graph.rs:133-134`), but it has no upper bound: a version 26 file would be parsed as 25. It should fail on a version it does not know.
- **Oracle.** The client logs `Generating level L area "A" with seed S`, `Tile hash` and `Doodad hash` for every area, and the DLL recomputes the hashes on export. `terrain-vectors.tsv` has a `build` column (from `Build Revision`). So stale vectors are detectable: a vector whose `build` differs from the current one is stale.
- On a new build: re-export a few areas from a play session, run `cargo xtask terrain-matrix` and `check_exports`. A hash mismatch is the signal. This is the slowest check and needs the client to run, so it is not part of `check-build`, only its report ("vectors from build X, current Y").
- The terrain status doc already lists unported systems. After a patch, expect the number of failing seeds to grow for new area types first.

## 7. `Client.txt` parsing in scripts and the DLL
- `scripts/re/clientlog_terrain.py:13-16`: four regexes, the `Build Revision` one is already there.
- `scripts/re/clientlog_entries.py:13-19`: a line regex (`^\S+ \S+ (\d+) \S+ \[(\w+) Client \d+\] (.*)$`) and the `Generating level` regex.
- `scripts/re/sweep_drive.py`, `scripts/test-run.sh`, `scripts/test-mp.sh`: they read the **server** log (`authenticated`, `listening`), not `Client.txt`. They depend on our own log text only.
- DLL `gamelog.rs` and the Game log page read `logs/Client.txt` lines of this launch.
- Risk is low: the log format has been stable, and these tools print counts. Add the build revision to every output header. If the count is 0, the tool should say "no lines matched; the log format may have changed".

## 8. Other items checked
- `poe-hook` and the core use `ws2_32`, `user32` raw input, Vulkan loader (`vkGetInstanceProcAddr` found by export, `vulkan.rs:4`) and `version.dll` forwarding. These are OS APIs and do not move.
- `trace_calls.rs:29` takes arbitrary addresses from the user: any new build needs new addresses. No fix needed. Print the build revision next to each hit in the log so old traces are not misread.
- `auto_keys.rs:473` reads the pointer at `0x1445d8308` (the game window). Test input only, never in official mode.
- `login_patch.rs` falls back to a log line on mismatch. The patcher skip logs "would contact GGG patch servers" (`:64-80`); in private mode the guard still refuses external connects.
- `poe-hook-deploy/src/lib.rs:39-51`: `version.dll`, `steam_appid.txt` (`2694490`), `gateway_list.txt`, `PathOfExileSteam.exe`. The Steam app id and exe name rarely change.
- `config.json` keys (`GGG_*`, `STEAM_APP_ID`, ...) from `0x141018170` are used by `docs/guides/setup.md` for the credentials file name (`login_token.rs:14`).

## 9. Proposal: the signature table (`modules/client-hook/signatures/<version>.toml`)

Goal: one file per build, embedded in the DLL, used by the DLL and by `check-build`.

```toml
# modules/client-hook/signatures/0.5.5e.toml
[build]
version        = "0.5.5e"
build_revision = "323943"                 # string in .rdata, also in Client.txt
pe_timestamp   = 0x6ac302e0               # IMAGE_FILE_HEADER.TimeDateStamp
image_size     = 0x04ca0000               # SizeOfImage
exe_sha256     = "7157984534da1b146e9b0be342aa9fd24272915b6f478aadef9830fc7873f89f"   # used by check-build only

# kind "function": the resolved address is the function start (+ offset).
[[sig]]
name     = "client_platform_detect"
features = ["login_patch"]               # which feature needs it
required = true                          # missing => that feature is disabled, others continue
kind     = "function"
pattern  = "40 55 53 56 57 41 56 48 8d ac 24 ?? ?? ?? ?? 48 81 ec ?? ?? ?? ?? 48 8d 4c 24 ??"
offset   = 0                             # bytes from the match start to the address
ghidra   = 0x142196ec0                   # the 0.5.5e address (docs, symbols, sanity check)
expect   = "40 55 53"                    # bytes at the resolved address; required for any patch
patch    = "31 c0 c3"                    # optional: written only after expect matched

# kind "site": inside a function. Anchored to another signature.
[[sig]]
name     = "packet_flush_plaintext"
features = ["packets"]
kind     = "site"
anchor   = "stream_flush"                # a function signature
anchor_offset = 0x...                    # from the function start to the site
expect   = "48 8b 93 60 02 00 00 48 8b bb 58 02 00 00"
boundary = true                          # the loader decodes from the anchor and requires an instruction boundary

# kind "data": resolve a global through a RIP-relative operand.
[[sig]]
name     = "platform_cache_guard"
kind     = "data"
anchor   = "client_platform_get"         # cached getter, 0x140109b20
pattern  = "80 3d <guard:rip4+1> 00 75 ??"   # absolute address of the operand's target
```

**Pattern syntax:** `xx` = byte, `??` = any byte, `<name:N>` = capture N bytes as a little-endian integer, `<name:rip4+K>` = a RIP-relative 4-byte displacement; the resolved value is the absolute address (end of the instruction = after the 4 bytes plus `K` more bytes, usually 0 for `call`, `jmp` and `lea`).

**Other kinds:**
- `string_xref`: `string = "General Rhoa Failure #3"`, `utf16 = false`, `xref = 1`. Find the string in `.rdata`, find the `lea reg,[rip+rel32]` that targets it, take the enclosing function from `.pdata` (the DLL already parses `.pdata`, `symbols.rs`).
- `caller_of`: find the unique function that calls the unique function `anchor` at the `n`-th call, for the template-clone lookups (`0x1416936c0`, `0x140150cb0`).

**Fields that give offsets** (from section 2):

```toml
[[field]]
name   = "positioned.facing"
sig    = "positioned_set_orientation"
capture = "facing"        # the <facing:4> capture in that signature's pattern
min    = 0x100
max    = 0x2000
align  = 4
check  = "positioned.facing_lock"   # optional second field that must agree with the first
```

Example pattern for `positioned_set_orientation` (tested, unique in 0.5.5e): `f6 81 <lock:4> 02 0f 85 ?? ?? ?? ?? e8 ?? ?? ?? ?? 84 c0 0f 84 ?? ?? ?? ?? c7 81 <facing:4> 00 00 00 00`. The capture `lock` gives `+0x1e5` (the `00 00` high bytes are part of the displacement), the capture `facing` gives `+0x44c`.

**Resolution rules (A2):**
1. Select the table by `(pe_timestamp, image_size)` read from the PE header (cheap, available in `DllMain`; a SHA-256 of 78 MB costs time, so only `check-build` does that), then confirm with `build_revision`. Unknown build: use the newest table as a candidate, but flag every result as "unverified" (and see rule 6).
2. Scan `.text` only, one pass for all patterns (the existing `Module::scan` runs a naive `windows()` over the whole `SizeOfImage`, `module.rs:49-61`; fine for one pattern, slow for 50).
3. A signature must give **exactly one** match. Zero or several: the feature that needs it is disabled and logged with the signature name and the count. Other features continue.
4. If `ghidra` is set and the match is at another address, log it as a note (the exe was rebuilt but the signature held).
5. A patch is written only when the resolved address holds the `expect` bytes. If it already holds `patch`, do nothing. The existing read-only rules stay: never in official mode.
6. Unknown build in official mode: install only features that read memory with guarded reads (`packets` without the field tracer, `crash_log`). Do not install `dump_object_types` or `area_export` until their signatures resolved uniquely.
7. Log one line per feature at start: `feature X: on (3 signatures)` or `feature X: OFF (sig Y matched 0 times)`. Show the same list on the overlay Status page.
8. Struct-offset captures are checked against `min/max/align` and any `check` partner; a failed check disables the dependent feature (position sync especially).

**Which addresses get a signature:** all 47 I tested except three need an anchor (section 1.3). In priority: login patch pair, crypto, side channel, packets (3 + 2 stream functions), `Entity_ReadInitialState`, `Positioned` reader, `SetOrientation`, area load pair, frame drain, terrain driver and its two mid-function sites, then the research trace set (generator trace is off by default and can stay "best effort").

**A generator for signatures:** the algorithm I used (decode instruction by instruction with capstone, wildcard `rel32`, RIP-relative `disp32`, image-address immediates and stack-frame immediates, grow until unique in `.text`) is about 60 lines of Python. A3 should ship it as `cargo xtask sig-gen <exe> <addr>…` or a `scripts/re/` tool, so a new table can be bootstrapped from a Ghidra export of the old one (`symbols.tsv`).

## 10. Proposal: `versions/<ver>.toml` as the single source

Today the version appears in about 15 places (section 5). Proposed: one file per supported build.

```toml
# versions/0.5.5e.toml
version        = "0.5.5e"
build_revision = "323943"
status         = "supported"            # supported | experimental | legacy
app_id         = 2694490

[exe]
name        = "PathOfExileSteam.exe"
sha256      = "7157984534da1b146e9b0be342aa9fd24272915b6f478aadef9830fc7873f89f"
size        = 78335832
pe_timestamp = 0x6ac302e0
image_size   = 0x04ca0000
sha256_enforce = true                   # after a real depot install has matched

[[depots]]
id = 2694491
manifest = "7366357142270402355"
role = "content"
[[depots]]
id = 2694492
manifest = "246779027001700092"
role = "windows exe"

[verify]                                 # as today: min_files, min_bytes, [[verify.files]]

[protocol]
opcodes = "opcodes/0.5.5e"               # relative to the protocol repo

[client_hook]
signatures = "signatures/0.5.5e.toml"
symbols    = "symbols/0.5.5e.tsv"

[data]
schema_release = "<dat-schema tag or sha256>"
schema_sha256  = "..."
dumps          = "dumps/0.5.5e"
```

Placement: the version file is needed by the server, xtask, the launcher and (through its embedded copy of the protocol repo) the DLL. Poe2Protocol is the repo all of them already depend on (`opcodes/<ver>/` lives there, and the DLL and the launcher embed it). Put the canonical files in `modules/protocol/versions/<ver>.toml` and let the launcher embed them (it already `include_str!`s its own copy, `version.rs:7`). The signature table stays in Poe2ClientHook as the plan says, referenced by name. The alternative (repo root) works for the server and xtask but the launcher and the DLL cannot read it at build time.

Consumers:
- **Server:** `[game] version` is looked up in `versions/`; startup fails for an unknown version, loads `opcodes` from the file instead of `protocol_dir`. The `Protocol::load` check for duplicates stays.
- **xtask:** `dump_dir`, deploy paths and `check-build` read the file.
- **Launcher:** the `EMBEDDED` list is generated from the folder; Play compares the installed exe (cached sha from the install, or at least size and PE timestamp) with the file and refuses to deploy the hook on a mismatch unless the user overrides.
- **DLL:** the signature table and opcode tables are embedded by build; the table is selected by `pe_timestamp` and `image_size`.
- **`status`:** `experimental` means "signatures resolve but layouts are unchecked". `legacy` means "kept for old captures only".

**Letting the server know the client's version.** The client sends no version string. Two options for A4:
1. The DLL knows the build (header fields and `build_revision`). Private mode needs the DLL anyway. Have the DLL say so in the side-channel hello, and make the server drop a session from an unsupported build with a clear log line. The side channel starts after login, so this protects the instance part only.
2. Check whether `client_hash` (32 bytes in `0x0003`/`0x0005`, `LoginClientHash_Get32` `0x1420e7580`, global `0x1445d5f50`) is stable per build. It is filled asynchronously, which suggests a hash of the executable or a build signature. If it is, the login server can map it to a version before any message is answered. I did not verify this. It needs one `0x0003` log of the 0.5.5e value (the server does not log it today; no code reads `client_hash`), then the same on the next build.

## 11. Proposed `cargo xtask check-build` report (A3 input)
Each block reports found / ambiguous / missing (or OK / drift / fail):

| Block | Checks | Source |
|---|---|---|
| Identity | sha256, size, PE timestamp, image size, `build_revision` string vs the version file | exe |
| Signatures | every `[[sig]]`, match count, new address, delta from `ghidra` | exe |
| Captures | every `[[field]]`, value, agreement of partners | exe |
| Patch sites | `expect` bytes at each patch address | exe |
| Opcodes | factory dump, deserializer fingerprints, renumbered or missing opcodes | exe |
| HNC table | `key_xor` found by value, swaps from `0x1421c50b0` | exe |
| Strings | `--success-event`, `--disable-client-switching`, `gateway_list.txt`, `Bundles2/gateway_list.txt`, `LatestClient.txt` | exe |
| Data | each used table and column, row width, row-count sanity, the anchors in section 4, the sprint buff ids | `Bundles2` |
| Formats | `.tgr` / `.dgr` / `.tdt` / `.ot` versions seen in the bundles against the versions the parsers accept | `Bundles2` |
| Vectors | `build` column of `terrain-vectors.tsv`, vector count for this build | dumps |
| Launcher | exe sha vs `versions/*.toml`, depot manifests listed | files |

Exit code: 0 all fine, 1 warnings (a research signature missing), 2 a required feature missing.

## Surprises
- The packet capture's redaction list is by name and **does not contain `Credentials` (`0x0003`)** (`packets.rs:198-211`: `Authenticate`, `ResumeSession`, `AuthenticationResult`, `InstanceDetails`, `Hello`, instance `0x000e`). On a standalone login, `packets.jsonl` stores the account name and the SHA-256 password hash (and the saved login token on a token login). This is not an update issue, but it is the same weakness: redaction depends on the opcode table. Both are small fixes: add `Credentials` and redact **every** outgoing login-channel body when the build is unknown. Check `dumps/0.5.5e/captures/local/` before the repos go public (WP D1).
- The build revision is a plain ASCII string in `.rdata`. It is the easiest version fingerprint, and it is also what `Client.txt` prints.
- Research hooks run by default on official sessions. A signature failure there is a crash risk, not just a lost feature.
- `Play` never verifies the exe. `exe_sha256_enforce = false`, and `verify` is a separate button.
- `dump_factory.py` and `export_symbols.py` are tied to 0.5.5e by name; the symbols file is a source of misleading names after a patch.
