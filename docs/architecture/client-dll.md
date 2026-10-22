# Client DLLs (`modules/client-hook/`)

The client imports `VERSION.dll` (`GetFileVersionInfoW`, `GetFileVersionInfoSizeW`, `VerQueryValueW`). Since the debug suite (2026-10-14) the client side is two DLLs in one Cargo workspace (`modules/client-hook/Cargo.toml`, the Poe2ClientHook submodule, separate from the server workspace):

| Crate | Built as | Deployed as | Role |
|---|---|---|---|
| `poe-hook` | `poe_hook.dll` | `GameFiles/version.dll` | **Loader**, never unloaded: version.dll proxy, log sink, network guard and read-only latch, Vulkan and window hooks, loads and hot-reloads the core. Under Proton it needs `WINEDLLOVERRIDES=version=n,b`. |
| `poe-hook-core` | `poe_hook_core.dll` | `GameFiles/poe_hook_core.dll` | **Debug core**, hot-reloadable: every feature and the overlay. |
| `poe-hook-shared` | (library) | | Config (`poe-hook.toml`), the loader ↔ core C ABI (`abi.rs`), guarded memory reads, module helpers, `trust.rs` (`is_trusted`), `login.rs` (byte logic of the login patch), `sigs.rs` (the signature table in a running process), `offsets.rs` (the resolved struct offsets), `redact.rs` (which captured messages carry secrets). Host-buildable: `cargo test -p poe-hook-shared` in the submodule. |
| `poe-hook-mod-api` | (library) | | The **mod ABI**: `HookApi` and the export names of a mod DLL, no dependencies. See [Mod loader](#mod-loader). |
| `poe-hook-mods` | (library) | | Host-buildable logic of the mod loader: `mod.toml` and `state.toml` parsing, the load plan, the ABI check, the official-mode refusal. `examples/sample-mod` is a tiny mod DLL (`sample_mod.dll`). |
| `poe-hook-sig` | (library) | | The **signature table** matcher: pattern parser, PE reader, scanner, resolver, per-feature status. Pure Rust without Windows dependencies, so `cargo xtask check-build` (WP A3) can use it on the exe file. `signatures/<version>.toml` is embedded by `include_str!`. See [Signature table](#signature-table-signaturesversiontoml). |

## Distribution

The DLLs are released from the Poe2ClientHook repository: a tag `hook-v<version>` runs `.github/workflows/release.yml` there, which builds `poe_hook.dll` and `poe_hook_core.dll` for `x86_64-pc-windows-msvc` and publishes `poe-hook-v<version>-game-<game versions>-windows-x86_64.zip` with a `.sha256`, `SHA256SUMS` and a `release.json` inside (hook version, game versions = the signature tables shipped, mod ABI, SHA-256 of every file); how: [../guides/releasing-hook.md](../guides/releasing-hook.md). The launcher (`modules/launcher/src-tauri/src/hook.rs`, `hook_ops.rs`) downloads the newest stable release that supports the selected game version, checks the archive and every file, keeps it in `<data>/hook/packages/<version>/` and deploys from there; its *Hook DLL directory* setting stays as an override for developers' own builds ([../guides/launcher.md](../guides/launcher.md#client-hook)). `cargo xtask build-dll` and `deploy-client` are unchanged (they use the local build).

## Loader (`modules/client-hook/poe-hook`)
| File | Role |
|---|---|
| `lib.rs` | `DllMain`. Loads the config, creates the session folder `logs/poe-hook/<date_time>_<pid>/`, opens the logs and installs the network guard synchronously (the client's crash-report helper connects out within milliseconds), applies the standalone-login patch (`login_patch.rs`), then starts a thread that hooks Vulkan and loads the core. |
| `proxy.rs` | Forwards all 16 version.dll exports to the system DLL, resolved lazily. |
| `logsink.rs` | Buffered (flushed at once for warnings/errors, otherwise every 250-500 ms; per-line flushes cost frames). One sink for both DLLs: `poe-hook.log` next to the exe (appended per process with a `---- process <pid> ----` header; `run-client` clears it per launch), the session folder's `poe-hook.log`, and a 50 000-record ring the overlay's Log tab reads (it survives core reloads). |
| `guard.rs` | Hooks every Winsock connect API (`connect`, `WSAConnect`, `WSAConnectByNameW/A`, `WSAConnectByList`, and `ConnectEx` through `WSAIoctl`; [below](#network-guard-connect-apis)): refuses untrusted connections in private mode (`block_external`), records every connection for the core, and sets the **read-only latch** (official mode, or the first untrusted ("external") connection). Trusted = loopback or `trusted_servers` ([below](#login-mode-and-trusted-servers)). |
| `mods.rs` | The **mod loader** ([below](#mod-loader)): maps the enabled mods' DLLs and calls `poe_mod_init`. Nothing in official mode. |
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
Observe-only use of the official servers; the launcher never enables it. The user guide (what it does and does not do, the risk, the steps `deploy-client --official`, `run-client`, switching back) is [../guides/official-mode.md](../guides/official-mode.md); the project's rules are in [../../CLAUDE.md](../../CLAUDE.md). In this mode the overlay shows `OFFICIAL: read-only`; packets, logs and snapshots go to the session folder, and the coverage page counts official messages separately. Comparing a capture with one from our server: [../re/captures.md](../re/captures.md).

**Read-only means read-only for the game's state, not for the process.** The latch stops everything that sends, alters or injects (crypto bypass, position sync, side channel, mods, login patch, key injection all check it), but the observers still patch game code in memory: inline detours on game functions, a Vulkan hook, and the `version.dll` proxy that loads the DLL at all. The DLL is a third-party module inside the official client, even if it only observes. It can be detected (the client can also load server-delivered code modules, see [../re/crypto.md](../re/crypto.md#other-crypto-in-the-binary)), and it is against the game's terms; there is some risk to the account.

## Debug link with our server (`devlink.rs`)
The DLL's position-report UDP socket also receives sealed `Kind::Debug` frames from the instance server (`key\tvalue` lines; `ack\t<command>\t<result>` for commands, shown as notices) and sends sealed `Kind::Command` frames (the command text). Frames carry the session id, a counter and an HMAC tag made with keys derived from the session's ticket keys ([../protocol/side-channel.md](../protocol/side-channel.md)); the receiver drops anything that does not verify. Server side: `[dev] debug_link` (default on in the code and the example config; off in the committed `modules/server/config/server.toml`, on in the developer config), `sync.rs` remembers each session's UDP address (the source of its latest authentic datagram) and queues its commands, the session tick applies them (`dev_command`) and sends `Player::debug_state`. Works for remote clients. Never while read-only, and never to an address that is not trusted. A test script cannot forge frames: `scripts/re/send_dev.py` writes a `cmd:<command>` entry to `poe-hook.keys` and the DLL sends it.
- Teleport sends two direction updates in one frame: a minimal direction with the target cell, then a stop. The client checks the cell of `0x64` only when the direction is not zero (`0x142149640`); in lockstep it then always places the player on the cell's centre (the 46-cell rule applies only outside lockstep).

## Modes and the read-only latch
`mode` in `poe-hook.toml` (set by `cargo xtask deploy-client [--official]`):
- `private` (default): our server. Crypto bypass on, untrusted ("external") connections refused.
- `official`: official servers, **read-only**. The latch is on from the start: no crypto bypass (the real handshake runs), no position sync, no test input or window-title hooks, no memory writes, no login patch. External connections are allowed. The DLL only observes and logs.
- No mod loads in `official` mode, or when the latch is already on at start ([Mod loader](#mod-loader)).
- In either mode the latch switches on by itself at the first untrusted connection that goes through, and features that act (crypto bypass, position sync) check it again when they act.

## Mod loader
Mods are folders `poe-hook-mods/<id>/` next to the exe (`mod.toml`, an optional entry DLL, data) and `poe-hook-mods/state.toml` (`enabled = [...]`, the load order), the layout the launcher writes ([../guides/launcher.md](../guides/launcher.md#mods)); `cargo xtask deploy-mod` installs one into `GameFiles/` the same way ([../guides/modding.md](../guides/modding.md)). Before the core starts (it hooks function starts, which a later resolution would no longer match) the loader thread resolves the signature table for the mods (only when `state.toml` exists, ~0.3 s); after the core is up it plans the load (`poe-hook-mods`: valid manifest, folder name = id, ABI check, DLL present), then for each mod in order checks dependencies and conflicts against the mods that really loaded, maps the DLL (`LoadLibraryExW`) and calls `poe_mod_init(&HookApi)`. `HookApi` (ABI version 1, `poe-hook-mod-api`): the mod's id and folders, the game version/revision and whether the build is the verified one, `log` (tagged `mod:<id>`), `read_only`, `sig_addr`, `feature_ok`, `field`.
- **Read-only guarantee.** In official mode (and when the latch is already on) the loader loads no mod and does not even read `poe-hook-mods/`; it logs `mods: official mode is read-only: no mod is loaded`. Mods are code inside the client, so the official client only ever gets the observing DLL. If the latch comes on later (an untrusted connection), loaded mods see it through `read_only()`; they cannot be unloaded.
- **Never crashes the client at load time.** A mod with another ABI (checked on the manifest before mapping, and on the optional `poe_mod_abi_version` export after), a missing dependency, a broken manifest or `state.toml`, a DLL that does not map, lacks `poe_mod_init` or returns non-zero is logged (`mods: skipped <id>: <why>`) and skipped; the DLL of a manifest-level skip is never mapped. What a mod does after `poe_mod_init` returns is its own responsibility (`crash_log` still reports a fault in it).
- **Report to a private server.** The loader records every mod that loaded (id, version, SHA-256 of `mod.toml` and of the entry DLL) and hands the encoded list to the core through `HostApi::mod_report` (core ABI 3; `-1` until the loader is done); the core sends it over the side channel ([Side channel](#side-channel)). Details and the server's check: [mod-host.md](mod-host.md#the-login-check-of-the-clients-mods).
- Not yet: `[data]` overlays, `poe_mod_shutdown`, unloading or hot-reloading a mod.

## Signature table (`signatures/<version>.toml`)
Since WP A2 (2026-10-21) the DLLs hold **no** game addresses and no game struct offsets in code. Every hook address, patch site and the struct offsets the DLL reads come from `modules/client-hook/signatures/0.5.5e.toml`, resolved once when a DLL loads. The matcher is the crate `poe-hook-sig`; the process glue is `poe-hook-shared/src/sigs.rs` (the core uses it through `poe-hook-core/src/sigs.rs`: `gate`, `addr`, `off`).

**Where it lives and why embedded.** The tables are compiled into both DLLs (`include_str!`), not read from disk: the table must match the code it was built with, a file next to the exe could be stale or edited to point a hook at the wrong bytes, and the launcher deploys the DLL and its table together anyway. The build is chosen by the image's PE timestamp and `SizeOfImage`; the ASCII build revision (`323943`, a string in `.rdata` next to `%S.poecap`, also printed in `Client.txt`) is a third check. Any difference makes the result **unverified** (logged; see below). The generator bootstraps a table for a new build: [../re/update-compatibility.md](../re/update-compatibility.md#what-wp-a2-did).

**Entries** (85 signatures, 51 fields in 0.5.5e: 74 `function`, 3 `site`, 8 `ref`):

| Kind | Resolves to | Rule |
|---|---|---|
| `function` (default) | the match start plus `offset` | `pattern` (wildcards `??`, captures `<name:N>`, `<name:rip4+K>`) matches **exactly once** in the code sections; optional `pdata = true` (must be a `.pdata` function start) |
| `site` | an address inside the function of `anchor` | found by `pattern` inside the anchor function (exactly once) or `anchor_offset`; `expect` bytes required; the site must be an **instruction boundary** (linear decode from the function start) |
| `ref` | the target of a RIP-relative operand (call target, global, vtable) | `pattern` with a `rip4` capture, searched in the anchor function (or image-wide) exactly once; used for the three clone-like functions (`object_type_lookup`, `entity_get_render`, `terrain_sample`), found through a unique caller, and for globals |
| `[[field]]` | a struct offset | `pattern` inside the `anchor` function with an integer capture over a displacement; optional `min`/`max`/`align`; `agrees_with` names a second field that reads the same member (both fail on a mismatch); `value` is the 0.5.5e value (a different resolved value is logged as `changed`, not an error) |

Common fields: `features` (the DLL features that are **off** when the entry does not resolve), `ghidra` (the 0.5.5e address: logging and sanity, never used), `expect` (bytes, wildcards allowed, that must be at the resolved address; for a patch exactly the bytes replaced), `patch` (bytes written, only after `expect` matched), `note`. Patterns wildcard everything a rebuild moves (call and jump targets, RIP-relative displacements, stack-frame sizes and offsets) and keep struct displacements and opcodes.

**Resolution rules.**
1. Resolved once per DLL load, in one pass over the code sections (about 0.2 s). The loader resolves only `login_patch`, `login_cache`, `patcher_skip` (before it patches them, so the patched bytes never meet a pattern); the core resolves everything else.
2. A signature with 0 or more than 1 matches, a failed anchor, wrong bytes at the address, a site that is not an instruction boundary, a field outside its limits, or two partners that disagree: the entry has **failed**, every feature that lists it is **off**, and the log says which entry and why (`signatures: feature X: OFF (sig Y: matched 0 times)`). Other features continue. Nothing is hooked or patched on a guess, and there is no fallback to the 0.5.5e address.
3. A feature's hooks are all resolved first; `hooks::install` runs only when all of them are there. The research hooks that are on by default (`dump_object_types`, `area_export`, packet capture) fail closed the same way. Checked in game on 2026-10-21 with one pattern byte of `read_initial_state` broken: `poe-hook.log` listed `dump_object_types`, `area_export` and `position_sync` as `OFF` (`sig read_initial_state: matched 0 times; 7 entries need the failed anchor`), every other feature stayed `ok`, the client did not crash and entered the world (the server then spawned at the origin: no layout export); with the table restored all features are `ok` again.
4. **Unverified build** (timestamp, size or revision differ from the table, or no table matches): the newest table is tried as a candidate, results are logged as `NOT verified`, and with the read-only latch on (official servers) only the pure observers start (`packets` without the field tracer, `crash_log`, `exit_trace`); `dump_object_types`, `area_export`, `entity_probe` and the rest stay off. In private mode the features that resolved start.
5. The packet capture redacts **every** login message on an unverified build, so a renumbered opcode cannot write credentials to `packets.jsonl` (`poe-hook-shared/src/redact.rs`; the 0.5.5e rule is by message name and includes `Credentials` `0x0003`).
6. One log block at start: `signatures resolved in N ms`, the build check line, one line per feature (`ok (N entries)` or `OFF (...)`), and a line for every entry that moved or changed value.

**Features in the table:** `login_patch`, `login_cache`, `patcher_skip` (loader); `crypto_bypass`, `side_channel`, `packets`, `dump_object_types`, `area_export`, `generator_trace`, `position_sync`, `entity_probe`, `entity_trace`, `inspect`, `auto_keys_focus` (core). `position_sync` also needs `dump_object_types` (its tick hook). `inspect` (player inspector, stats, entity component names, movement-log fields) and `entity_probe` (the 5-second probe after the player spawned, which calls four game functions directly) are research features: when off, the reads/calls are skipped.

**Struct offsets in the table** (`poe-hook-shared/src/offsets.rs`, unit-tested against the table): entity (`type`, `components_begin/end`, `id`, `world`, `tick_base`), type/lookup (`lookup`, `serialized_count`, `names_begin/end`), component (`owner`, `vt_reader` = vtable slot of the initial-state reader), Positioned (`grid_x/y`, `world_x/y`, `facing`, `facing_lock`), Actor (`action0/1`, `dir_x/y`, `sprint_flags`, action `type`), world (`frame_mode`, `pause`, `frame_queue`, `area`, area `kind`, `tile_hash`, `doodad_hash`, vtable slot `vt_terrain`, game-update accumulator), generator (`world`, `terrain`, `state`, state `seed`), terrain (`width`, `height`, `tiles`), network stream (`error`, `msg_read`, `recv_base/end/head`, `send_len/buf/sent`), session `stream`, crypto session `key_buffer`. **Still literal in the code** (listed so nobody assumes otherwise): the four terrain cell-grid vectors (`+0xd0/+0xe8/+0x100/+0x118`; the export checks each vector's size against the grid and writes nothing on a mismatch), the tile record internals (`0x38` record, fields `+0x30..+0x37`; the export recomputes the client's tile hash from them and flags a mismatch), the object-type table entry layout (`0x30`, path at `+8`), area `hash16` (`+0x2a`) and world `+0x4b0/+0x4b4` (export metadata), the packet object's opcode (`+8`), the stream socket (`+0`), the component indices Positioned `0` and Actor `9`, Stats component and stat-set containers (`inspect.rs`), the generator-trace object layouts (graph, node, edge, router, region, room; research, off by default) and the diagnostic-only reads of the movement log and the entity probe (Render `+0x39a`, action flags `+0x144`, Actor `+0x220..+0x250`, `+0x2a0/+0x2a1/+0x915/+0x916`, Positioned `+0x46c..+0x49c`, world `+0x11c/+0x84a`, vtable slots `0x178/0x200/0x60/0x1f0`). `trace_addresses` / `POE_HOOK_TRACE` take addresses typed by the user and are unchanged.

**`check-build` and the version file.** `cargo xtask check-build <exe>` resolves this table against an exe with the same crate (`poe-hook-sig`), reports every entry as found, ambiguous or missing with its new address and the delta from `ghidra`, and exits 2 when an entry of a feature listed in `modules/protocol/versions/<version>.toml` `[client_hook] required_features` fails (1 for the research features). The table's `[build]` must agree with the version file's `[exe]`/`build_revision` (the check reports a disagreement). A new build: [../guides/new-game-version.md](../guides/new-game-version.md).

**Tools.** `cargo run --release -p poe-hook-sig --example resolve -- <exe> [table.toml] [--quiet]` (in `modules/client-hook`) resolves the table against an exe file and compares every result with the table's `ghidra` addresses and `value`s (exit 1 on any difference). `tests/real_exe.rs` does the same against `GameFiles/PathOfExileSteam.exe` and also mutates bytes (the affected entries report missing, their features go off, the others stay on); it skips when the exe is not there. `scripts/re/sig_gen.py scripts/re/sig_spec_0.5.5e.py` regenerates the table from the spec of Ghidra addresses (third_party/pyenv: capstone, pefile).

## Login mode and trusted servers
Two top-level keys of `poe-hook.toml`, written by `deploy-client` (WP3.3):

| Key | Values | Effect |
|---|---|---|
| `login_mode` | `"standalone"` (default in private mode), `"steam"` | `standalone`: the loader patches `ClientPlatform_Detect` (Ghidra `0x142196ec0`, `40 55 53` → `31 C0 C3`, `xor eax,eax; ret`) in `DllMain`, before the client's first platform query. The client then shows `LoginScreen.ui` (account name, password, Remember Me), sends `0x0003` instead of the Steam ticket `0x0005` and does not initialise the Steam API. Evidence and the packet: [../re/standalone-login.md](../re/standalone-login.md). `steam`: no patch at all. **Forced to `steam` in official mode** (deploy writes it, and the config loader overrides the file). |
| `trusted_servers` | `["ip:port", "ip:*", "ip"]` (IPv4 literals) | Servers treated like loopback. Deploy writes `[network] public_host` with the login port and the instance port, resolving names to IPs; a name under an official game domain (`pathofexile.com`, `pathofexile2.com`, `grindinggear.com`) is refused with a warning (`poe_hook_deploy::trusted::OFFICIAL_DOMAINS`). Empty in official mode (also enforced when the DLL reads the file). |

Patch safety (`poe-hook/src/login_patch.rs`): applied only for `standalone` and only while the latch is off; the function is found by a byte signature that must match exactly once (`client_platform_detect`), its first bytes are verified against the table's `expect` bytes (at resolution and again right before the write), and on a mismatch or a missing signature (another client build) the patch is skipped with a logged error and the Steam login stays; the page is made writable for the write, restored, and the instruction cache is flushed; if the platform cache guard (`platform_cache_guard`, 0.5.5e `0x144717114`) is already set, the cached value (`platform_cache_value`, `0x144717118`) is reset to 0 (without these two signatures the patch still applies and the reset is skipped with a warning). The patcher skip (`standalone_patcher_run`) works the same way. The byte logic is in `poe-hook-shared/src/login.rs` (unit-tested); addresses and bytes are in the signature table. Log lines start with `login_mode` and `signatures:`.

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

### Network guard: connect APIs
Which APIs the game really uses (static imports of `PathOfExileSteam.exe`, checked again at runtime): **only `connect`** (5 call sites; `WSAConnect` is not imported). `WSAIoctl` is imported but called with `SIO_KEEPALIVE_VALS` and one other control code, never `SIO_GET_EXTENSION_FUNCTION_POINTER`; there is no `ConnectEx` GUID, no `mswsock`/`WinHTTP`/`WinINet` import and no `WSAConnectByName`/`List`. Details and addresses: [../re/network-apis.md](../re/network-apis.md). In game the log shows `api=connect` for every connection and never a `ConnectEx ... hooked` line.

The guard nevertheless covers the other ways to open a TCP connection, so a client build or a dynamically loaded module that used one could not slip past (audit S-10):
- `connect`, `WSAConnect`, `WSAConnectByNameW/A`, `WSAConnectByList` are inline-hooked in `ws2_32`; the last three are optional (Wine/Proton's `ws2_32` does not export `WSAConnectByList`; the guard logs a warning and goes on). `connect` and `WSAConnect` are required (no guard without them).
- `WSAIoctl` is hooked. When a call asks for `SIO_GET_EXTENSION_FUNCTION_POINTER` with `WSAID_CONNECTEX` and succeeds, the returned function is itself inline-hooked (once per address; log `network guard: ConnectEx at ... hooked`) with the same rules. If that hook cannot be installed the ioctl is failed (`WSAEOPNOTSUPP`, pointer zeroed) so the game cannot reach an unguarded `ConnectEx`; with the latch already on (official play, observing only) the pointer is left alone and a warning logged.
- Same rule everywhere (`poe_hook_shared::netguard`, unit-tested on the host): trusted (loopback, `trusted_servers`) is allowed and never latches; external is refused in private mode with `block_external` and otherwise latches. A host name is never resolved (loader context), so `WSAConnectByName` is trusted only for a numeric IPv4 address and a decimal port that are trusted, and `WSAConnectByList` only when every entry is. Refused calls fail with `WSAECONNREFUSED`. The log line is `connect #n ... api=<name>` / `blocked <name> ...`.
- Not covered: a socket opened by raw system calls (`NtDeviceIoControlFile`/AFD) or by code that bypasses ws2_32. There is no send/receive fail-closed rule: the loader hooks no `send`/`recv` (only the optional `net_trace` feature of the core does, and the core cannot set the latch).

Verified without the game: a probe program under Wine with the loader in private mode (ConnectEx to a TEST-NET address blocked with 10061, loopback allowed and logged, `WSAConnectByNameA` to a host name blocked, a numeric loopback target allowed). In game (private mode, standalone login, `scripts/test-run.sh`): login, world entry, terrain export and upload unchanged; blocked official-service connections (shop, telemetry) as before.

On start the DLL logs the effective guard state (`network guard: official=... block_external=... latch_at_start=... hooks=...`).

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
| `path_log` | off | One `PATHPLAN` line per path the client plans for the local player (goal, start, search parameters, pathing level, waypoints); also `POE_HOOK_PATH_LOG=1`. Hooks the pathfinder (signature `path_plan`, read-only), [../re/pathfinding.md](../re/pathfinding.md). |
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
- Mod manifest (`mod.toml`), the mod ABI and the loader's rules: `modules/client-hook/docs/mod-manifest.md` (summary [below](#mod-loader)).

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
- **Terrain upload.** After `export_layout` writes the layout and grid, `side::upload_terrain` sends them to the trusted instance server from a background thread: an offer of the tile hash first (the server may already have it), then the zlib-compressed layout and grid, over a TCP connection to the instance port. The connection goes through the connect hook like the game's own and shows up in the connection table (the report target stays the same address). Local files are still written. The first client to generate an area uploads it; others get "server has it". Hosting: [../guides/hosting.md](../../modules/server/docs/hosting.md).
- **Mod report.** When the session's keys are captured, `side::report_mods` starts a background thread that waits for the loader's list of loaded client mods (`host::mod_report`) and the trusted server connection (`position_sync::server_addr`), then sends one sealed `Kind::ModReport` frame over TCP to the instance port and logs `mod report to the server: stored`. The server compares it with the client mods it requires at login ([mod-host.md](mod-host.md#the-login-check-of-the-clients-mods), protocol: [../protocol/side-channel.md](../protocol/side-channel.md#mod-report)). Same gate as every send: nothing without a session, so nothing in official mode or while read-only.
- **Test entries.** `cmd:<server command>` in `poe-hook.keys` sends that command over the debug link (`scripts/re/send_dev.py`).
