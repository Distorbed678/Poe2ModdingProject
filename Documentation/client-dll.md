# Client DLL (`client/poe-hook`)

The client imports `VERSION.dll` (`GetFileVersionInfoW`, `GetFileVersionInfoSizeW`, `VerQueryValueW`). `poe-hook` is built as `poe_hook.dll` and deployed as `GameFiles/version.dll`. Under Proton it needs `WINEDLLOVERRIDES=version=n,b`.

## Structure
| File | Role |
|---|---|
| `src/lib.rs` | `DllMain`. Starts an init thread (no work under the loader lock), then config, log and features. |
| `src/proxy.rs` | Forwards all 16 version.dll exports to the system DLL, resolved lazily. |
| `src/config.rs` | `poe-hook.toml` next to the exe. Falls back to defaults if missing. |
| `src/logger.rs` | `poe-hook.log` next to the exe. |
| `src/module.rs` | Main-module base and size, Ghidra↔runtime address conversion (image base 0x140000000), byte-pattern scanning. |
| `src/features/` | Feature modules, registered in `features/mod.rs`. |

## Features
| Key | Default | Description |
|---|---|---|
| `crash_log` | on | Vectored handler. Logs fatal exceptions with Ghidra addresses, registers and return addresses on the stack. |

Planned: encryption bypass (M1), packet trace, asset overrides.

## Building
`cargo xtask build-dll` runs `Tools/cargo-xwin/bin/cargo-xwin xwin build --target x86_64-pc-windows-msvc --release`, keeping the MSVC SDK cache in `Tools/xwin-cache`. Hooks use `ilhook` (pure Rust, iced-x86), because `retour`'s libudis86 C dependency does not build under clang-cl.
