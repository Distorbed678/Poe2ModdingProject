# Client DLLs (`client/`)

The client imports `VERSION.dll` (`GetFileVersionInfoW`, `GetFileVersionInfoSizeW`, `VerQueryValueW`). Since the debug suite (M4, 2026-10-14) the client side is two DLLs in one Cargo workspace (`client/Cargo.toml`, separate from the server workspace):

| Crate | Built as | Deployed as | Role |
|---|---|---|---|
| `poe-hook` | `poe_hook.dll` | `GameFiles/version.dll` | **Loader**, never unloaded: version.dll proxy, log sink, network guard and read-only latch, Vulkan and window hooks, loads and hot-reloads the core. Under Proton it needs `WINEDLLOVERRIDES=version=n,b`. |
| `poe-hook-core` | `poe_hook_core.dll` | `GameFiles/poe_hook_core.dll` | **Debug core**, hot-reloadable: every feature and the overlay. |
| `poe-hook-shared` | (library) | | Config (`poe-hook.toml`), the loader ↔ core C ABI (`abi.rs`), guarded memory reads, module/pattern helpers. |

## Loader (`client/poe-hook`)
| File | Role |
|---|---|
| `lib.rs` | `DllMain`. Loads the config, creates the session folder `logs/poe-hook/<date_time>_<pid>/`, opens the logs and installs the network guard synchronously (the client's crash-report helper connects out within milliseconds), then starts a thread that hooks Vulkan and loads the core. |
| `proxy.rs` | Forwards all 16 version.dll exports to the system DLL, resolved lazily. |
| `logsink.rs` | One sink for both DLLs: `poe-hook.log` next to the exe (appended per process with a `---- process <pid> ----` header; `run-client` clears it per launch), the session folder's `poe-hook.log`, and a 50 000-record ring the overlay's Log tab reads (it survives core reloads). |
| `guard.rs` | Hooks `connect`/`WSAConnect`: refuses non-loopback connections in private mode (`block_external`), records every connection for the core, and sets the **read-only latch** (official mode, or the first external connection). |
| `vulkan.rs` | Redirects `vkGetInstanceProcAddr`/`vkGetDeviceProcAddr` in `vulkan-1.dll`'s **export table** (jump stubs near the module; under Wine both exports forward to `winevulkan`). Inline hooks there hung the game: Wine's exports are tiny thunks. The game (`0x1410262d0`) resolves everything through them, so it gets our wrappers for instance/device/queue/surface/swapchain creation and `vkQueuePresentKHR`. The swapchain gets `COLOR_ATTACHMENT` and `TRANSFER_SRC` usage added when the surface supports them. |
| `window.rs` | Subclasses the game window (from the Vulkan surface) and passes each message to the core first. |
| `core.rs` | Loads `poe_hook_core.dll` from a temp copy (so the file can be replaced while the game runs) and reloads it when the file changes (`[reload] auto`) or the core asks (overlay button). Unload: the core's `stop`, then a write lock that waits for loader → core calls in flight, then `FreeLibrary`. |

## Core (`client/poe-hook-core`)
- `poe_core_start(&HostApi) -> *const CoreApi` is its only export (`poe-hook-shared/src/abi.rs`; `ABI_VERSION` must match).
- `hooks.rs`: every hook goes through `hooks::install`, which saves the target's first 16 bytes; `stop` writes them back, leaks the trampolines (a thread may still be in one), waits until no callback runs (each callback holds `hooks::enter()`), and joins the core's threads (`hooks::spawn`, which loop while `hooks::running()`).
- After a reload the core has missed the entity creations: it adopts the player (entity id 1 on our server) from its next frame tick.
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
- Tabs:
  - **Status**: mode, latch, session folder, connections, Vulkan; Reload core / Snapshot / Hide buttons.
  - **Log**: the shared log ring (level and text filters, copy).
  - **Packets**: captured messages (filters, decoded view, hex, copy).
  - **Coverage**: every opcode seen per channel and direction, counts on official / our servers (all sessions) and this session, sizes. "Official only" = sent by official servers but never by ours.
  - **Timing** (ImPlot): frame time, messages per 100 ms in each direction, lockstep backlog (world +0x6a8, frames queued).
  - **Game log**: this launch's lines of `logs/Client.txt` (Wine reuses process ids, so lines are taken from the launch time in the session folder name).
  - **Player**: grid, world position, facing (and lock), actions, WASD direction, actor flags.
  - **Entities**: entities created in the area (initial-state reader), type path from the object-type lookup just before, live grid position (Positioned found by its reader `0x141dd8820`). After a reload the player is adopted from its frame tick (type unknown).
  - **Watches**: read-only memory watches with address expressions (`[ptr]`, `+`/`-`, Ghidra addresses, `player`/`pos`/`actor`/`exe`), saved in `logs/poe-hook/watches.txt`.
  - **Features**: runtime switches (movement log, position sync) and the config values.

## Packet capture (`[packets]`, `packets.rs`)
Plaintext of every login and instance message, read inside the client (no cipher is broken, so it works the same on official servers):
- **Received:** the message reader `0x1421c5950(registry, stream)` (Retn hook). It reads the opcode from the stream's receive ring (decrypted on arrival; ring `+0x298..+0x2a0`, read offset `+0x268`), the registered handlers deserialize the body, and the read offset advances by exactly the message size: the bytes in between are the message. Return value low byte 1 = a whole message was read (0 = not enough data yet). Registry byte 0 set = 1-byte opcodes. The caller gives the channel (`0x1420e6bc0` login, `0x142120b70` instance).
- **Sent:** a JmpBack hook at `0x142198037` inside the stream flush `0x142197fc0`, after its lock and before the cipher (`stream+0x1b8`, slot 1) encrypts in place: pending plaintext `[+0x280] + [+0x260 .. +0x258]`, rbx = stream. Boundaries come from the game-session send `0x14211f880` (stream `session+0x20f8`, opcode `packet+8`), which records where each message starts and ends in the send buffer. Bytes sent outside it (frame reports, login messages) are split with the fixed sizes from `instance.toml` `[client_body_sizes]` and the `poe-proto` decoders, and marked "unframed".
- Streams without a reply yet are classified by their socket (`stream+0`) and the connection's port: ports in `gateway_list.txt` are login, everything else instance.
- The game also has its own plaintext taps (receive `stream+0x150`, send `stream+0x190`, called with `(tap, &ptr, &len)`), probably for packet recording; they are unused here (setting them would be a write).
- Output: the Packets tab (`[packets] keep` messages; restored from `packets.jsonl` after a core reload), `packets.jsonl` (one JSON object per message: seq, t, dir, chan, op, name, len, framed, redacted, official, hex) and `packets.txt` in the session folder. Names come from `protocol/0.5.5e/*.toml` (embedded at build time), decoded fields from `poe_proto::describe`.
- `redact` (default on) drops the bodies of Authenticate, ResumeSession, AuthenticationResult, InstanceDetails, Hello and client `0x000e` (instance ticket) before anything is stored.
- Finding (2026-10-14): the instance opening "Hello" is two messages, `0x000f` (u32) and `0x000e` (12 bytes, u32 ticket, u32, u8); the server still reads them as one (left as is for now).

### Snapshots
`<session>/snapshots/<time>/`: `screenshot.png` (copied from the presented image in the overlay's command buffer; needs the `TRANSFER_SRC` usage the loader adds), `log.txt`, `packets.txt`, `timings.csv` (the last `snapshot_seconds`) and `state.txt` (mode, connections, Vulkan, player).

### Vulkan capture notes
The game resolves some functions (instance and device creation) before the loader's export patch is in place, so the loader takes the instance from `vkCreateWin32SurfaceKHR`, the physical device from the surface queries, the device from `vkCreateSwapchainKHR` and a present-capable queue family from `vkGetPhysicalDeviceSurfaceSupportKHR`.

## Modes and the read-only latch
`mode` in `poe-hook.toml` (set by `cargo xtask deploy-client [--official]`):
- `private` (default): our server. Crypto bypass on, external connections refused.
- `official`: official servers, **read-only**. The latch is on from the start: no crypto bypass (the real handshake runs), no position sync, no test input or window-title hooks, no memory writes. External connections are allowed. The DLL only observes and logs.
- In either mode the latch switches on by itself at the first external connection that goes through, and features that act (crypto bypass, position sync) check it again when they act.

## Features (`[features]`)
| Key | Default | Description |
|---|---|---|
| `crash_log` | on | Vectored handler. Logs fatal exceptions with Ghidra addresses, registers and game addresses on the stack (bounded by the thread's stack limits). It also logs C++ exceptions (type name and `what()`), and hooks `ExitProcess`/`TerminateProcess` to log exit codes with stack refs (`exit_trace`). |
| `crypto_bypass` | on | Replaces the login verify-and-derive routine (found by its prologue pattern). The session key material becomes SHA-512(server public key), so no signature check or DH is needed. See [re/crypto.md](re/crypto.md). Off while read-only. |
| `net_trace` | off | Inline hooks on ws2_32 `send`/`WSASend`/`recv`/`WSARecv`. Logs payload prefixes and game addresses on the stack. Connects are logged by the loader's guard. |
| `net_trace_dump_bytes` | 256 | Bytes of payload shown per call. |
| `block_external` | on | Private mode: refuses connections to non-loopback addresses, so the client does not reach official services (shop, telemetry, uploads). Ignored in official mode. |
| `dump_object_types` | on | The research hooks on entities, frames and area loads (below). Position sync needs them. |
| `position_sync` | on | Reports the player's position and actions to the instance server every frame (see [Position sync](#position-sync)). Never while read-only. |
| `position_sync_port` | 0 | The instance server's port; only connections to it become the report target (0: the last loopback connection). `deploy-client` sets it from `server.toml`. |
| `movement_log` | off | Per-frame movement log (see [Movement log](#movement-log)). |
| `trace_addresses` | `[]` | "Logging breakpoints": Ghidra addresses whose calls are logged with the argument registers and the game call stack (`trace_calls`, up to `trace_limit` hits each). The `POE_HOOK_TRACE` environment variable (comma-separated) adds addresses without editing `GameFiles/`. Hook only at function starts or instruction boundaries that `ilhook` can relocate: a bad spot crashes the client (it did at `0x140ece810`). |
| `auto_keys` | `[]` | Test input timeline `[[ms, vk], ...]`; tests pass `POE_HOOK_AUTO_KEYS="20000:13"` instead (`tools/test-run.sh`). `deploy-client` resets it to `[]`. Never while read-only. |

Note: the game's own login/instance TCP traffic does not go through these ws2_32 exports (it is invisible to `net_trace`). The libcurl HTTPS traffic does go through them.

## Building and deploying
- `cargo xtask build-dll` runs `Tools/cargo-xwin/bin/cargo-xwin xwin build --workspace --target x86_64-pc-windows-msvc --release` in `client/`, keeping the MSVC SDK cache in `Tools/xwin-cache`. Hooks use `ilhook` (pure Rust, iced-x86), because `retour`'s libudis86 C dependency does not build under clang-cl.
- `cargo xtask deploy-client [--official] [--no-build]` installs both DLLs, the config and the gateway list (ours, or the original for `--official`).
- `cargo xtask deploy-core [--no-build]` installs only the core; a running client reloads it within about a second.

## Crash reports
After a crash the client writes `PoE-<time>-<pid>.dmp` (plus `.txt`, `.log.txt` and `.poecap` side files) under `My Games/Path of Exile 2/poe2_production` in the prefix. A helper process uploads them, both right after the crash and on the next launch. With `block_external` installed from `DllMain`, the upload connections are refused. `cargo xtask run-client` also moves pending dumps to `data/crash-dumps/` (git-ignored) before launching, so nothing is left to upload and the dumps stay available for debugging.

## Entity tracing (`dump_object_types`)
Besides dumping the object-type table, this feature hooks `0x141e71120` (entity initial-state reader). It logs each entity's components (vtable, reader, owner), then wraps each serialized component's reader to log the blob span it consumed. That is how [protocol/entities.md](protocol/entities.md) was measured: send an all-zero blob, padded, and read the spans.

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

### Layout export format
`terrain_layout_<tile hash>.tsv`: a `size<TAB>w<TAB>h` line, then `x<TAB>y<TAB>tile path<TAB>sub` per sub-tile (23×23 cells).

`sub` is the sub-tile's index in its tile's own orientation, row-major. It comes from the tile record's +0 pointer, which points into the tile object's sub-tile array (0x38 bytes per entry); the lowest such pointer per tile object is the array start. From `sub` and the map position, the server recovers how the tile was rotated or mirrored (`instance-server::spawn::PlacedTile`).

### Send tracing (2026-10-14)
`dump_object_types` also logs, once per opcode, the caller of the game-session send (`0x14211f880`, packet +8 = opcode). It logs the caller of the raw stream writer (`0x142197e60`) the first time `00 1c` is written, plus the HNC cipher object (stream +0xd0). That is how the frame report and the HNC channel were found.

`poe-hook.keys` also accepts `close`, which posts `WM_CLOSE` to the game window (like its X button).

### Input injection for movement tests (2026-10-14)
- The game reads WASD through Raw Input, so posted key messages do not move the player.
- **`in:<vk>[:ms]`** posts `WM_INPUT` with a marker handle. The DLL hooks `GetRawInputData` (user32) to return a synthetic keyboard record for that handle, and `GetKeyState` reports the key as held meanwhile.
- **`inmove:x,y` / `inclick:x,y`** make the hooked `GetCursorPos` return that client position, then post the mouse messages.
- **Plain keys (`0x20`)** are still posted messages. Space (dodge roll) is read that way.
- **`text:<chars>`** posts one `WM_CHAR` per character, for text boxes such as the character name.
- **`inpress:x,y` / `inrelease:x,y`** hold and release the left button at a client position (held click-to-move); the hooked `GetKeyState` reports `VK_LBUTTON` held meanwhile.
- Before each batch the DLL posts `WM_ACTIVATEAPP(1)`: the game ignores input while it thinks it is in the background, and this changes only its own flag, not the real focus.
- **`0x77` (F8)** takes a game screenshot into the test prefix's `My Games/Path of Exile 2/Screenshots`, which shows the game even when its window is not visible.
- Nothing goes to other windows. `SendInput` is not used, because it types into whatever window has focus.

### Movement log
With `dump_object_types`, plus `movement_log = true` or the `POE_HOOK_MOVEMENT_LOG` environment variable (which `tools/test-run.sh` sets), the DLL logs:
- the player's world position at the start of each of its simulation updates (`frame #n dt world step grid`);
- the Actor's WASD direction (+0x400) when it changes.

It is off by default because it writes about 30 lines a second while moving. See [protocol/movement.md](protocol/movement.md).

### Facing probes (2026-10-15)
With the movement log on, the DLL logs the player's facing (Positioned +0x44c and the lock bit) when it changes, and every distinct call stack of SetOrientation (`0x141ddc6a0`) for the player.

## Position sync
`position_sync` (on by default) sends one UDP datagram per player simulation frame (format `PSY2`: the PSY1 fields plus the cursor's client-area position and the window size, for facing the cursor). It goes to the game's loopback TCP connection to the instance server (`position_sync_port`); the server listens for UDP on the same port. The local port is read with `getsockname` after the non-blocking connect and again when reporting while it is still 0. The DLL logs the target ("position sync: target port …"). So reports only ever go to a local server. Payload, 24 bytes little-endian: `b"PSY1", u16 local TCP port of that connection, u16 Actor +0x2a2 flags, u32 frame counter, f32 grid x, f32 grid y, u16 action type at Actor +0x238, u16 action type at +0x240`. The server keys reports by the TCP port. The connect hook records the connection and restores `WSAGetLastError` afterwards: the game checks it after a non-blocking `connect`, and changing it broke the login. This is a dev bridge for M3.5 until the server simulates movement exactly (M5); see [protocol/movement.md](protocol/movement.md#lag-m35).

## Test window title
With `POE_HOOK_WINDOW_TITLE=<title>` in the environment (set by `tools/test-run.sh`), the DLL hooks `CreateWindowExW`/`SetWindowTextW` so the game window gets that title. A desktop rule can then match the test client, for example a KWin rule that moves the window to another virtual desktop without focus, so unattended tests do not cover the screen. No rule is installed by the project.
