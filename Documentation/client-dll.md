# Client DLL (`client/poe-hook`)

The client imports `VERSION.dll` (`GetFileVersionInfoW`, `GetFileVersionInfoSizeW`, `VerQueryValueW`). `poe-hook` is built as `poe_hook.dll` and deployed as `GameFiles/version.dll`. Under Proton it needs `WINEDLLOVERRIDES=version=n,b`.

## Structure
| File | Role |
|---|---|
| `src/lib.rs` | `DllMain`. Loads the config, opens the log and installs the network hooks synchronously (only file I/O and patching of the already-mapped ws2_32), then starts a thread for the other features. The client's crash-report helper process connects out within milliseconds of starting, so the network guard must be active before the game runs. |
| `src/proxy.rs` | Forwards all 16 version.dll exports to the system DLL, resolved lazily. |
| `src/config.rs` | `poe-hook.toml` next to the exe. Falls back to defaults if missing. |
| `src/logger.rs` | `poe-hook.log` next to the exe. Appends, with a `---- process <pid> ----` header per process, so a client restart (or the crash-report helper) does not erase the previous log. `run-client` clears it per launch. |
| `src/module.rs` | Main-module base and size, Ghidra↔runtime address conversion (image base 0x140000000), byte-pattern scanning. |
| `src/features/` | Feature modules, registered in `features/mod.rs`. |

## Features
| Key | Default | Description |
|---|---|---|
| `crash_log` | on | Vectored handler. Logs fatal exceptions with Ghidra addresses, registers and game addresses on the stack (bounded by the thread's stack limits). It also logs C++ exceptions (type name and `what()`), and hooks `ExitProcess`/`TerminateProcess` to log exit codes with stack refs (`exit_trace`). |
| `crypto_bypass` | on | Replaces the login verify-and-derive routine (found by its prologue pattern). The session key material becomes SHA-512(server public key), so no signature check or DH is needed. See [re/crypto.md](re/crypto.md). |
| `net_trace` | on | Inline hooks on ws2_32 `send`/`WSASend`/`recv`/`WSARecv`/`connect`/`WSAConnect`. Logs payload prefixes and game addresses on the stack. |
| `net_trace_dump_bytes` | 256 | Bytes of payload shown per call. |
| `block_external` | on | Refuses connections to non-loopback addresses, so the client does not reach official services (shop, telemetry, uploads). |

Note: the game's own login/instance TCP traffic does not go through these ws2_32 exports (it is invisible to `net_trace`). The libcurl HTTPS traffic does go through them. The server-side hexdump (`RUST_LOG=poe_net=trace`) shows decrypted game traffic.

Planned: packet trace at the cipher layer, asset overrides.

## Building
`cargo xtask build-dll` runs `Tools/cargo-xwin/bin/cargo-xwin xwin build --target x86_64-pc-windows-msvc --release`, keeping the MSVC SDK cache in `Tools/xwin-cache`. Hooks use `ilhook` (pure Rust, iced-x86), because `retour`'s libudis86 C dependency does not build under clang-cl.

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
