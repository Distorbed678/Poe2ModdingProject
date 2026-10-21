# Client DLLs (`modules/client-hook/`)

The client imports `VERSION.dll` (`GetFileVersionInfoW`, `GetFileVersionInfoSizeW`, `VerQueryValueW`). Since the debug suite (2026-10-14) the client side is two DLLs in one Cargo workspace (`modules/client-hook/Cargo.toml`, the Poe2ClientHook submodule, separate from the server workspace):

| Crate | Built as | Deployed as | Role |
|---|---|---|---|
| `poe-hook` | `poe_hook.dll` | `GameFiles/version.dll` | **Loader**, never unloaded: version.dll proxy, log sink, network guard and read-only latch, Vulkan and window hooks, loads and hot-reloads the core. Under Proton it needs `WINEDLLOVERRIDES=version=n,b`. |
| `poe-hook-core` | `poe_hook_core.dll` | `GameFiles/poe_hook_core.dll` | **Debug core**, hot-reloadable: every feature and the overlay. |
| `poe-hook-shared` | (library) | | Config (`poe-hook.toml`), the loader ↔ core C ABI (`abi.rs`), guarded memory reads, module/pattern helpers, `trust.rs` (`is_trusted`), `login.rs` (addresses and bytes of the login patch). Host-buildable: `cargo test -p poe-hook-shared` in the submodule. |

## Loader (`modules/client-hook/poe-hook`)
| File | Role |
|---|---|
| `lib.rs` | `DllMain`. Loads the config, creates the session folder `logs/poe-hook/<date_time>_<pid>/`, opens the logs and installs the network guard synchronously (the client's crash-report helper connects out within milliseconds), applies the standalone-login patch (`login_patch.rs`), then starts a thread that hooks Vulkan and loads the core. |
| `proxy.rs` | Forwards all 16 version.dll exports to the system DLL, resolved lazily. |
| `logsink.rs` | Buffered (flushed at once for warnings/errors, otherwise every 250-500 ms; per-line flushes cost frames). One sink for both DLLs: `poe-hook.log` next to the exe (appended per process with a `---- process <pid> ----` header; `run-client` clears it per launch), the session folder's `poe-hook.log`, and a 50 000-record ring the overlay's Log tab reads (it survives core reloads). |
| `guard.rs` | Hooks `connect`/`WSAConnect`: refuses untrusted connections in private mode (`block_external`), records every connection for the core, and sets the **read-only latch** (official mode, or the first untrusted ("external") connection). Trusted = loopback or `trusted_servers` ([below](#login-mode-and-trusted-servers)). |
| `login_patch.rs` | `login_mode = "standalone"`: patches the client's `ClientPlatform_Detect` so it shows the account-name/password screen ([below](#login-mode-and-trusted-servers)). |
| `vulkan.rs` | Redirects `vkGetInstanceProcAddr`/`vkGetDeviceProcAddr` in `vulkan-1.dll`'s **export table** (jump stubs near the module; under Wine both exports forward to `winevulkan`). Inline hooks there hung the game: Wine's exports are tiny thunks. The game (`0x1410262d0`) resolves everything through them, so it gets our wrappers for instance/device/queue/surface/swapchain creation and `vkQueuePresentKHR`. The swapchain gets `COLOR_ATTACHMENT` and `TRANSFER_SRC` usage added when the surface supports them. |
| `window.rs` | Subclasses the game window (from the Vulkan surface) and passes each message to the core first. |
| `core.rs` | Loads `poe_hook_core.dll` from a temp copy (so the file can be replaced while the game runs) and reloads it when the file changes (`[reload] auto`) or the core asks (overlay button). Unload: the core's `stop`, then a write lock that waits for loader → core calls in flight, then `FreeLibrary`. |

## Core (`modules/client-hook/poe-hook-core`)
- `poe_core_start(&HostApi) -> *const CoreApi` is its only export (`poe-hook-shared/src/abi.rs`; `ABI_VERSION` must match).
- `hooks.rs`: every hook goes through `hooks::install`, which saves the target's first 16 bytes; `stop` writes them back, leaks the trampolines (a thread may still be in one), waits until no callback runs (each callback holds `hooks::enter()`), and joins the core's threads (`hooks::spawn`, which loop while `hooks::running()`).
- After a reload the core has missed the entity creations: it adopts the player from its next frame tick.
- The `auto_keys` key timeline only runs on the first load (a reload must not press Enter again).

## Overlay (`[overlay]`)
Dear ImGui 1.92 through [dear-imgui-rs](https://github.com/Latias94/dear-imgui-rs) 0.20 (`dear-imgui-ash` Vulkan backend, `dear-implot` for graphs; chosen 2026-10-14 over the older imgui-rs 0.12), drawn by the core inside the game's `vkQueuePresentKHR`:
- **One overlay thread** owns the imgui context and all overlay Vulkan objects: dear-imgui-rs keeps context and font-atlas state in thread-locals, and the game presents from several threads (a frame built on another thread panicked: "requires explicit managed font-atlas ownership"). The present hook sends the frame to that thread and blocks until it returns the semaphore; the game's present call holds the queue meanwhile.
- `overlay/gpu.rs`: a render pass (kept across swapchain rebuilds while the format is unchanged) that loads (does not clear) the swapchain image and keeps it in `PRESENT_SRC_KHR`; 3 frame slots (command buffer, fence, semaphore). The submit waits on the game's present semaphores and the present then waits on the slot's semaphore. Device functions come from the loader's real `vkGetInstanceProcAddr` (ash `Entry::from_static_fn`).
- `overlay/input.rs`: window messages → imgui. Mouse and keyboard messages, including `WM_INPUT` (the game reads movement and skills through Raw Input), are kept from the game while imgui wants them (pointer over an overlay window, a focused text field). Button releases always reach the game.
- `toggle_key` (default Insert) shows/hides it; `snapshot_key` (default Home) saves a snapshot. While hidden nothing is drawn except the `OFFICIAL: read-only` badge and short notices.
- The backend's `framebuffer_srgb` option is set for the game's `B8G8R8A8_SRGB` swapchain, so colors are right.
- Release builds abort on panic; the core installs a panic hook that logs the message and backtrace first.
- `scale` multiplies the font size (DejaVu Sans Mono 16 px, embedded; license in `poe-hook-core/assets/`). Window layout persists in `logs/poe-hook/imgui.ini`.
- Layout (2026-10-17): one window with a sidebar of groups and pages (HOME: Status, Log, Game log; SERVER: Instances, Commands, Sync state, Desync; NETWORK: Packets, Protocol, Coverage, Notes, Timing; GAME: Player, Stats, Actions, Entities, Watches, Minimap; RE TOOLS: Scanner, Hooks; SETTINGS: Features), a page search box, and the last page remembered (`logs/poe-hook/overlay_page.txt`). The SERVER group is hidden while the read-only latch is on. **Ctrl+P** opens a command palette: type to jump to a page, or type a server command line (`transfer G1_6 random natural natural`) and press Enter to run it; a command that needs arguments opens its form on the Commands page.
- Pages:
  - **Status**: mode, latch, session folder, connections, Vulkan; Reload core / Snapshot / Hide buttons.
  - **Log**: the shared log ring (level and text filters, copy).
  - **Packets**: captured messages (filters, decoded view, hex, copy).
  - **Hidden opcodes**: the Packets page can hide opcodes that are already identified, so new ones stand out: "Hide this opcode" on a selected message, or the "opcodes" list (every opcode seen this session with its count and a visibility checkbox; "Hide all named", "Hide all listed" for the current filter, "Show all"). "show hidden" shows them anyway. Saved in `logs/poe-hook/hidden_opcodes.tsv` (`channel, dir, opcode`, kept across sessions). Only the view is filtered: packet files still get every message.
  - **Notes**: the user's packet notes. In the Packets page a selected message has two editors: a note on its opcode ("what this message does", shown as `*` in the lists) and a note on that one message (`[note]`); the Protocol page edits opcode notes too. Saved in `logs/poe-hook/packet_notes.tsv` (kept across sessions): `kind, channel, dir, opcode, name, session, seq, unix_ms, note, hex` (message notes keep the message's hex, unless redacted). **Check this file when working on a message's layout or meaning.**
  - **Coverage**: every opcode seen per channel and direction, counts on official / our servers (all sessions) and this session, sizes. "Official only" = sent by official servers but never by ours.
  - **Timing** (ImPlot): frame time, messages per 100 ms in each direction, lockstep backlog (world +0x6a8, frames queued).
  - **Game log**: this launch's lines of `logs/Client.txt` (Wine reuses process ids, so lines are taken from the launch time in the session folder name).
  - **Player**: grid, world position, facing (and lock), actions, WASD direction, actor flags.
  - **Entities**: entities created in the area (initial-state reader), type path from the object-type lookup just before, live grid position (Positioned found by its reader `0x141dd8820`). After a reload the player is adopted from its frame tick (type unknown).
  - **Sync state** (our server only; the `server_tab.rs` page): the instance server's simulation state for this session (debug link, refreshed every second tick: frame, server position, movement state, WASD, sprint key, combat stance, pause, skills, and with a fresh report the client's position, action and the drift in cells) and a quick control for the registry's `teleport` command (cell, or "Here").
  - **Commands** and **Instances** (our server only): the server's command registry as forms, and its instances with join/close and a new-instance form: [../guides/server-admin.md](../guides/server-admin.md#overlay-pages-our-server-only-server-group).
  - Our-server-only pages (the SERVER group) are hidden while the read-only latch is on.
  - **Desync** (our server only): the server's predicted position and the last report against the client's own position (ImPlot), with a count of samples over 1.5 cells and the worst; desyncs over 6 cells are logged. The prediction is one frame ahead, so ~1.2 cells while walking is normal.
  - **Minimap**: the client's four cell grids around the player (terrain object found by vtable in the game world; tiles `+0x18`/`+0x20`, 23 cells per tile, 4 bits per cell at `+0xd0`, `+0xe8`, `+0x100`, `+0x118`), shaded by value, with the player and its facing, and the server's position (blue ring, our server). "Export area" writes `terrain_layout_*.tsv` / `terrain_grid_*.bin` on demand (also on official servers, where the automatic export is off).
  - **Stats**: the player's stats by name (`poe-hook-data/stats.tsv`), all stat sets, values that changed in the last 3 s highlighted. Stats component (found by name) `+0x1c8` = set 0 (the entity's own stats), other sets in the 16-byte list `+0x1e8..+0x1f0` (container `+0xd8` = set id); a container is a sorted `(i32 key, i32 value)` array at `+0xf8..+0x100` (setter `0x1426ac920`, StatsUpdate handler `0x142181f20`).
  - **Actions**: starts and ends of the player's Actor actions (`+0x238`, `+0x240`; type at action `+0x120`) with `ActionTypes` names, sampled every simulation frame; also logged ("action start slot 0: DodgeRoll ...").
  - **Entities** also lists the selected entity's components with their names (Positioned, Stats, Pathfinding, Buffs, Life, Animated, Player, PlayerClass, Inventories, Actor, Render, Targetable, BuffOrbs for the player), addresses and vtables; "scan" opens one in the Scanner. Names come from the entity type's component lookup (`entity+8` → `+0x28`: sorted `{char* name, i32 index}` array from `+0x28` to `+0x30`, as used by the get-component-by-name functions such as `0x140150cb0`). The client has no RTTI (the slot before a vtable is a function pointer).
  - **Scanner**: a memory region (watch syntax, up to 8 KB) read every frame; bytes that just changed are red, recently changed orange, differing from the baseline yellow; hex plus u32/f32/pointer columns, tooltips with all interpretations, "rows that changed" filter.
  - **Watches**: read-only memory watches with address expressions (`[ptr]`, `+`/`-`, Ghidra addresses, `player`/`pos`/`actor`/`exe`), saved in `logs/poe-hook/watches.txt`.
  - **Protocol**: per opcode, what the field tracer learned: the client functions that read/wrote it (deserializer or serializer first), the field layouts (offset:size) and, per field over the last 200 samples, constant / range / plausible f32 / text. "Export now" writes `protocol.tsv` and `ghidra_labels.tsv`.
  - **Hooks**: time spent in each of our hook callbacks (calls/s, average, maximum, ms per second). A callback over 50 ms is logged and shown as a notice. Retn hooks exclude the game function they wrap.
  - **Features**: runtime switches (movement log, position sync) and the config values.

## Packet capture (`[packets]`, `packets.rs`)
Plaintext of every login and instance message, read inside the client (no cipher is broken, so it works the same on official servers):
- **Received:** the message reader `0x1421c5950(registry, stream)` (Retn hook). It reads the opcode from the stream's receive ring (decrypted on arrival; ring `+0x298..+0x2a0`, read offset `+0x268`), the registered handlers deserialize the body, and the read offset advances by exactly the message size: the bytes in between are the message. Return value low byte 1 = a whole message was read (0 = not enough data yet). Registry byte 0 set = 1-byte opcodes. The caller gives the channel (`0x1420e6bc0` login, `0x142120b70` instance).
- **Sent:** a JmpBack hook at `0x142198037` inside the stream flush `0x142197fc0`, after its lock and before the cipher (`stream+0x1b8`, slot 1) encrypts in place: pending plaintext `[+0x280] + [+0x260 .. +0x258]`, rbx = stream. Boundaries come from the game-session send `0x14211f880` (stream `session+0x20f8`, opcode `packet+8`), which records where each message starts and ends in the send buffer. Bytes sent outside it (frame reports, login messages) are split with the fixed sizes from `instance.toml` `[client_body_sizes]` and the `poe-proto` decoders, and marked "unframed".
- Streams without a reply yet are classified by their socket (`stream+0`) and the connection's port: ports in `gateway_list.txt` are login, everything else instance.
- The game also has its own plaintext taps (receive `stream+0x150`, send `stream+0x190`, called with `(tap, &ptr, &len)`), probably for packet recording; they are unused here (setting them would be a write).
- Output: the Packets page (`[packets] keep` messages; restored from `packets.jsonl` after a core reload), `packets.jsonl` (one JSON object per message: seq, t, dir, chan, op, name, len, framed, redacted, official, hex) and `packets.txt` in the session folder. Names come from `modules/protocol/opcodes/0.5.5e/*.toml` (embedded at build time), decoded fields from `poe_proto::describe`.
- `redact` (default on) drops the bodies of Authenticate, ResumeSession, AuthenticationResult, InstanceDetails, Hello and client `0x000e` (instance ticket) before anything is stored.

### Field tracer (`protocol.rs`)
While a message is decoded (inside the reader hook) or encoded (inside the session send), the stream primitives are hooked: `0x142197d30` read `(stream, dst, len)` at message offset `stream+0x278`, `0x142197e60` write `(stream, src, len)` at `+0x258` minus the message start. Each record is (offset, size, return address). The function containing the return address (from `.pdata`) is the deserializer/serializer; functions used by more than two opcodes are generic helpers (strings, chars) and are not chosen. Fields are in `packets.jsonl` (`"fields":"offset:size:caller ..."`) and restored after a reload. Messages sent outside the session (frame reports, login) are not traced.
- `ghidra_labels.tsv` suggests names like `Read_S2C_instance_00df_AddInventory` only for functions that serve exactly one opcode.

### Symbols (`symbols.rs`)
Addresses in crash reports, exit traces, logging breakpoints and the Protocol page show as `name+0x..` (names from `assets/symbols.tsv`, generated by `scripts/re/export_symbols.py` from Ghidra's named functions and `docs/re/addresses-0.5.5e.md`) or `FUN_<start>+0x..` (function bounds from the exe's `.pdata`, ~154 500 functions). Regenerate the TSV after documenting new addresses.

### Snapshots
`<session>/snapshots/<time>/`: `screenshot.png` (copied from the presented image in the overlay's command buffer; needs the `TRANSFER_SRC` usage the loader adds), `log.txt`, `packets.txt`, `timings.csv` (the last `snapshot_seconds`) and `state.txt` (mode, connections, Vulkan, player).

### Vulkan capture notes
The game resolves some functions (instance and device creation) before the loader's export patch is in place, so the loader takes the instance from `vkCreateWin32SurfaceKHR`, the physical device from the surface queries, the device from `vkCreateSwapchainKHR` and a present-capable queue family from `vkGetPhysicalDeviceSurfaceSupportKHR`.

## Using it on official servers (read-only)
The steps (`deploy-client --official`, `run-client`, switching back) are in [../guides/setup.md](../guides/setup.md#official-servers-read-only); the rules are in [../../CLAUDE.md](../../CLAUDE.md). In this mode the overlay shows `OFFICIAL: read-only`; packets, logs and snapshots go to the session folder, and the coverage page counts official messages separately. Comparing a capture with one from our server: [../re/captures.md](../re/captures.md).

Risk: the DLL is a third-party module inside the official client, even if it only observes. It can be detected (the client can also load server-delivered code modules, see [../re/crypto.md](../re/crypto.md#other-crypto-in-the-binary)), and it is against the game's terms; there is some risk to the account.

## Debug link with our server (`devlink.rs`)
The DLL's position-report UDP socket also receives sealed `Kind::Debug` frames from the instance server (`key\tvalue` lines; `ack\t<command>\t<result>` for commands, shown as notices) and sends sealed `Kind::Command` frames (the command text). Frames carry the session id, a counter and an HMAC tag made with keys derived from the session's ticket keys ([../protocol/side-channel.md](../protocol/side-channel.md)); the receiver drops anything that does not verify. Server side: `[dev] debug_link` (default on), `sync.rs` remembers each session's UDP address (the source of its latest authentic datagram) and queues its commands, the session tick applies them (`dev_command`) and sends `Player::debug_state`. Works for remote clients. Never while read-only, and never to an address that is not trusted. A test script cannot forge frames: `scripts/re/send_dev.py` writes a `cmd:<command>` entry to `poe-hook.keys` and the DLL sends it.
- Teleport sends two direction updates in one frame: a minimal direction with the target cell, then a stop. The client checks the cell of `0x64` only when the direction is not zero (`0x142149640`); in lockstep it then always places the player on the cell's centre (the 46-cell rule applies only outside lockstep).

## Modes and the read-only latch
`mode` in `poe-hook.toml` (set by `cargo xtask deploy-client [--official]`):
- `private` (default): our server. Crypto bypass on, untrusted ("external") connections refused.
- `official`: official servers, **read-only**. The latch is on from the start: no crypto bypass (the real handshake runs), no position sync, no test input or window-title hooks, no memory writes, no login patch. External connections are allowed. The DLL only observes and logs.
- In either mode the latch switches on by itself at the first untrusted connection that goes through, and features that act (crypto bypass, position sync) check it again when they act.

## Login mode and trusted servers
Two top-level keys of `poe-hook.toml`, written by `deploy-client` (WP3.3):

| Key | Values | Effect |
|---|---|---|
| `login_mode` | `"standalone"` (default in private mode), `"steam"` | `standalone`: the loader patches `ClientPlatform_Detect` (Ghidra `0x142196ec0`, `40 55 53` → `31 C0 C3`, `xor eax,eax; ret`) in `DllMain`, before the client's first platform query. The client then shows `LoginScreen.ui` (account name, password, Remember Me), sends `0x0003` instead of the Steam ticket `0x0005` and does not initialise the Steam API. Evidence and the packet: [../re/standalone-login.md](../re/standalone-login.md). `steam`: no patch at all. **Forced to `steam` in official mode** (deploy writes it, and the config loader overrides the file). |
| `trusted_servers` | `["ip:port", "ip:*", "ip"]` (IPv4 literals) | Servers treated like loopback. Deploy writes `[network] public_host` with the login port and the instance port, resolving names to IPs. Empty in official mode (also enforced when the DLL reads the file). |

Patch safety (`poe-hook/src/login_patch.rs`): applied only for `standalone` and only while the latch is off; the three original bytes are verified first, and on a mismatch (another client build) the patch is skipped with a logged error and the Steam login stays; the page is made writable for the write, restored, and the instruction cache is flushed; if the platform cache guard (`0x144717114`) is already set, the cached value (`0x144717118`) is reset to 0. The addresses and bytes are constants in `poe-hook-shared/src/login.rs` (with the byte logic unit-tested). Log lines start with `login_mode`.

**One trust decision**, `poe_hook_shared::trust::is_trusted(addr)`: loopback (`127.0.0.0/8`, `::1`) is always trusted; otherwise the address must match a `trusted_servers` entry (IPv6 never does). It is used by:

| Where | Use |
|---|---|
| `guard.rs` | `external = !is_trusted`. |
| `abi.rs` `Connection.external` | the same flag, recorded for the core (Status page, position sync target). |
| `crypto_bypass.rs` | the bypass runs unless the latch is on or a recorded connection is untrusted. |
| `ui/mod.rs` `custom_server()` | overlay features that talk to our server need the latch off and every connection trusted. |

How the latch treats each kind of server:

| Server | `block_external` (private) | Read-only latch | Crypto bypass | Position sync / overlay server pages |
|---|---|---|---|---|
| Loopback | allowed | no | yes | yes |
| In `trusted_servers` | allowed | no | yes | yes (reports go to the trusted address) |
| Anything else (official) | refused; allowed if `block_external = false` or in official mode | **yes**, permanently for the process | no (real handshake) | no |

A client with `login_mode = "standalone"` must not be pointed at an official server (the official login rejects `0x0003`); the latch still protects it. Launcher auto-login needs no DLL code: see [../guides/setup.md](../guides/setup.md#remember-me-token-launcher-auto-login).

## Features (`[features]`)
| Key | Default | Description |
|---|---|---|
| `crash_log` | on | Vectored handler. Logs fatal exceptions with Ghidra addresses, registers and game addresses on the stack (bounded by the thread's stack limits). It also logs C++ exceptions (type name and `what()`), and hooks `ExitProcess`/`TerminateProcess` to log exit codes with stack refs (`exit_trace`). |
| `crypto_bypass` | on | Replaces the login verify-and-derive routine (found by its prologue pattern). The session key material becomes SHA-512(server public key), so no signature check or DH is needed. See [re/crypto.md](../re/crypto.md). Off while read-only. |
| `net_trace` | off | Inline hooks on ws2_32 `send`/`WSASend`/`recv`/`WSARecv`. Logs payload prefixes and game addresses on the stack. Connects are logged by the loader's guard. |
| `net_trace_dump_bytes` | 256 | Bytes of payload shown per call. |
| `block_external` | on | Private mode: refuses connections to untrusted addresses (not loopback, not in `trusted_servers`), so the client does not reach official services (shop, telemetry, uploads). Ignored in official mode. |
| `dump_object_types` | on | The research hooks on entities, frames and area loads (below). Position sync needs them. The terrain layout export runs only when not read-only. |
| `entity_trace` | off | Per-entity tracing: each entity's components, the blob span each component reader consumes, terrain dumps on the Positioned reader. **Slow**: on the first official session (2026-10-14) each spawned entity froze the game for ~2.3 s (a 4554-tile dump per Positioned read), and a three-monster spawn froze it for 7 s, after which the server dropped the connection ("Abnormal disconnect"). Research on our server only. |
| `position_sync` | on | Reports the player's position and actions to the instance server every frame (see [Side channel](#side-channel)). Never while read-only. |
| `position_sync_port` | 0 | The instance server's port; only connections to it become the side-channel target (0: the last trusted connection). `deploy-client` sets it from `server.toml`. |
| `movement_log` | off | Per-frame movement log (see [Movement log](#movement-log)). |
| `trace_addresses` | `[]` | "Logging breakpoints": Ghidra addresses whose calls are logged with the argument registers and the game call stack (`trace_calls`, up to `trace_limit` hits each). The `POE_HOOK_TRACE` environment variable (comma-separated) adds addresses without editing `GameFiles/`. Hook only at function starts or instruction boundaries that `ilhook` can relocate: a bad spot crashes the client (it did at `0x140ece810`). |
| `auto_keys` | `[]` | Test input timeline `[[ms, vk], ...]`; tests pass `POE_HOOK_AUTO_KEYS="20000:13"` instead (`scripts/test-run.sh`). `deploy-client` resets it to `[]`. Never while read-only. |

Note: the game's own login/instance TCP traffic does not go through these ws2_32 exports (it is invisible to `net_trace`). The libcurl HTTPS traffic does go through them.

## Game data for the DLL
Deploying also writes name tables from the game data into `GameFiles/poe-hook-data/` ([../guides/setup.md](../guides/setup.md)): `stats.tsv` (network key = `Stats.dat` row + 1, id), `action_types.tsv` (`ActionTypes` key column, id) and `areas.tsv` (the Instances page's area picker).

## Building and deploying
Commands: [../guides/setup.md](../guides/setup.md).
- Hooks use `ilhook` (pure Rust, iced-x86), because `retour`'s libudis86 C dependency does not build under clang-cl.
- The deploy logic is the `poe-hook-deploy` crate in the same submodule (gateway list, `poe-hook.toml` edits, `steam_appid.txt`, DLL install and backup, Proton launch, crash-dump moving); xtask only passes the repo paths (GameFiles, `dumps/<version>`, the Proton prefix) and exports the DLL name tables.
- The opcode tables the Packets page uses are compiled in from Poe2Protocol (`poe_proto::opcodes::v0_5_5e`); `modules/client-hook/protocol` is the nested checkout.
- Mod manifest (`mod.toml`) spec (the launcher reads it; the DLL loader does not yet): `modules/client-hook/docs/mod-manifest.md`.

## Terrain research (`terrain_tools.rs`)
`[features] area_export` (on by default; read-only safe) writes every generated area to `dumps/<version>/terrain/area-exports/` and checks it by recomputing the client's tile and doodad hashes; `[features] generator_trace` / `POE_HOOK_GEN_TRACE=1` adds the generator trace. Formats and the seed sweep: [../re/terrain/README.md](../re/terrain/README.md#tools).

## Entity tracing (`dump_object_types`)
Besides dumping the object-type table, this feature hooks `0x141e71120` (entity initial-state reader). It logs each entity's components (vtable, reader, owner), then wraps each serialized component's reader to log the blob span it consumed. That is how [protocol/entities.md](../protocol/entities.md) was measured: send an all-zero blob, padded, and read the spans.

### Player state probes (`dump_object_types`, 2026-10-13)
Five seconds after the local player entity is created, the DLL logs:
- entity flags +0x8c/+0x8d;
- Positioned scales and positions;
- the world's frame-mode and pause flags;
- the Render component's disabled flag (+0x39a);
- a NaN/inf scan of every component;
- the Animated AO.

It also hooks:
- the frame drain (`0x1421306c0`), logging dt and the caller for frame-mode worlds;
- the frame-mode entity update (`0x141e7d7f0`), logging the player's dt (xmm1);
- the front-end world update (`0x14167ea40`), logging dt;
- the initial-state reader, logging stack references (useful when an object is applied twice).

Setting `POE_HOOK_FORCE_SHOW=1` in the client's environment re-applies the player's render visibility (`0x1416a9810`) as an experiment.

### Output folder (`dump_dir`)
`dump_dir` (top level of `poe-hook.toml`) is where the DLL writes datamined and generated files. `cargo xtask deploy-client` sets it to `<repo>/dumps/<version>` as a Windows path (the client runs under Wine, where `Z:\` is `/`, e.g. `Z:\home\you\Poe2ModdingProject\dumps\0.5.5e`) and creates the folder. The DLL logs `dump_dir <path>` at start. It writes the terrain grids and layouts, area exports (with `generator_trace.tsv`) and the client tables (`object_types.txt`, `terrain_tiles.txt`) below it; which file lands in which folder is in [dumps/README.md](../../dumps/README.md).

Unset, the old locations apply (next to the exe; the session folder's `areas/`).

### Layout export format
`terrain_layout_<tile hash>.tsv` (`terrain/layouts/`): a `size<TAB>w<TAB>h` line, then `x<TAB>y<TAB>tile path<TAB>sub` per sub-tile (23×23 cells).

`sub` is the sub-tile's index in its tile's own orientation, row-major. It comes from the tile record's +0 pointer, which points into the tile object's sub-tile array (0x38 bytes per entry); the lowest such pointer per tile object is the array start. From `sub` and the map position, the server recovers how the tile was rotated or mirrored (`instance-server::spawn::PlacedTile`).

### Send tracing (2026-10-14)
`dump_object_types` also logs, once per opcode, the caller of the game-session send (`0x14211f880`, packet +8 = opcode). It logs the caller of the raw stream writer (`0x142197e60`) the first time `00 1c` is written, plus the HNC cipher object (stream +0xd0). That is how the frame report and the HNC channel were found.

### Input injection for tests (2026-10-14)
`poe-hook.keys` lets a test press keys and move the mouse in the client without touching other windows (never while read-only). The game reads WASD through Raw Input, so the DLL hooks `GetRawInputData`, `GetKeyState`, `GetCursorPos` and, for tests, `GetForegroundWindow` (`POE_HOOK_FAKE_FOCUS`). The entries and environment variables: [../guides/setup.md](../guides/setup.md#poe-hookkeys-entries).

### Movement log
With `dump_object_types`, plus `movement_log = true` or the `POE_HOOK_MOVEMENT_LOG` environment variable (which `scripts/test-run.sh` sets), the DLL logs:
- the player's world position at the start of each of its simulation updates (`frame #n dt world step grid`);
- the Actor's WASD direction (+0x400) when it changes.

It is off by default because it writes about 30 lines a second while moving. See [protocol/movement.md](../protocol/movement.md).

### Facing probes (2026-10-15)
With the movement log on, the DLL logs the player's facing (Positioned +0x44c and the lock bit) when it changes, and every distinct call stack of SetOrientation (`0x141ddc6a0`) for the player.

## Side channel
Code: `side.rs`, `position_sync.rs`, `devlink.rs`. The DLL talks to **our** instance server over an authenticated side channel: position reports, the debug link and the terrain upload. Format, keys and checks: [../protocol/side-channel.md](../protocol/side-channel.md).

- **Keys.** `side.rs` hooks the instance packet-cipher builder (`0x14212aa90`), copies the session's 64-byte key material and derives the session id and the two HMAC keys with `poe_hook_shared::sidechannel` (the code the server runs too). A new instance connection gives a new session. It captures only while the latch is off and every connection is trusted, so there is no session (and nothing is ever sent) on an official server. The log shows `side channel: session <id>` (the id is not secret; the server logs the same id).
- **Gate.** Every send (report, command, upload) asks `side::allowed(ip, port)`, which is `poe_hook_shared::trust::side_channel_allowed(read_only, addr)`: never while the read-only latch is on, and only to loopback or a `trusted_servers` entry. Unit-tested in `trust.rs` (an official or untrusted address gets nothing, latched or not).
- **Position sync.** `position_sync` (on by default) sends one sealed `Kind::Report` UDP frame per player simulation frame: flags, frame counter, x, y, the two action types and the cursor's client-area position with the window size (for facing the cursor). It goes to the game's connection to the instance server (`position_sync_port`, from the connection table: the newest trusted non-external connection to that port); the server listens for UDP on the same port number. The DLL logs the target ("position sync: target ip:port"). No local port is needed any more: reports are keyed by the session id. The connect hook records the connection and restores `WSAGetLastError` afterwards: the game checks it after a non-blocking `connect`, and changing it broke the login. The server simulates movement itself; it reads the reports only for the opt-in drift checker (`[dev] drift_check`), see [../protocol/movement.md](../protocol/movement.md#drift-checker).
- **Terrain upload.** After `export_layout` writes the layout and grid, `side::upload_terrain` sends them to the trusted instance server from a background thread: an offer of the tile hash first (the server may already have it), then the zlib-compressed layout and grid, over a TCP connection to the instance port. The connection goes through the connect hook like the game's own and shows up in the connection table (the report target stays the same address). Local files are still written. The first client to generate an area uploads it; others get "server has it". Hosting: [../guides/hosting.md](../guides/hosting.md).
- **Test entries.** `cmd:<server command>` in `poe-hook.keys` sends that command over the debug link (`scripts/re/send_dev.py`).
