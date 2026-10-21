# Standalone (email/password) login on the Steam exe (0.5.5e)

Static RE (Ghidra) for WP3.1, 2026-10-20. Question: how does `PathOfExileSteam.exe` choose between the Steam login (encrypted app ticket, packet `0x0005`) and the email/password login, what does the latter send, and how does our DLL force it. Addresses are Ghidra addresses (image base `0x140000000`); the new names are in [addresses-0.5.5e.md](addresses-0.5.5e.md#client-platform-and-standalone-login-2026-10-20).

## Answers in short
1. **Platform is decided once, from the exe file name and the command line**, by `ClientPlatform_Detect` (`0x142196ec0`). A name containing `Steam` (case-sensitive) or an argument `--steam` gives platform 1 (Steam). Anything with none of the markers gives platform **0 = standalone**. The result is cached in the global `DAT_144717118` and every consumer reads it through tiny getters. There is no config key, no flag and no Steam probing.
2. **The standalone path sends `0x0003`** (class vtable `0x1435063b8`, serializer `0x1420e3b60`, builder `ClientLoginSession_SendCredentials_0003` `0x1420e7af0`): `u32 mode = 2`, account name (UTF-8 string), `client_hash[32]`, `password_hash[32]`, `machine_hash[32]` and four flag bytes. `password_hash` is plain **SHA-256 of the UTF-16LE password**, or, for "Remember Me" logins, the **32-byte `session_token` that the server returned in the previous `0x0004`** (flag byte 1 = 1). No salt, no wide strings on the wire.
3. **Platform 0 also starts the standalone patcher** (window, GGG patch servers) unless the client is started with `--success-event <int>` (see section 7a). The launcher passes `--success-event 0`; the DLL patches the patcher's entry as a safety net.
4. **Patch: make `ClientPlatform_Detect` return 0** (`40 55 53` at `0x142196ec0` becomes `31 C0 C3`, `xor eax,eax; ret`). It is a pure getter with a cached result, so nothing else changes. With platform 0 the client shows `LoginScreen.ui`, never initialises the Steam API and does not need Steam to be running.
5. **Fallback** (token in the `0x0005` ticket) is workable but strictly worse: it keeps the Steam login screen (no account name), keeps `SteamAPI_Init` (Steam must run) and needs more hooks. Use it only if platform 0 turns out to break something at runtime. The standalone path already has a token channel of its own (the saved 32-byte token), so the launcher does not need the fallback.

Outcome (2026-10-20): the patch gives a working email/password login in game (password and remember-me token). Confidence before the first run was about 85-90 %. Static evidence is complete and consistent (UI choice, flow choice, packet builder, Steam init gate all key on the same getter). The residual risk is runtime behaviour of the other platform-0 consumers that were not read one by one (about 90 getter callers, mostly UI and shop code).

## 1. Platform detection

`ClientPlatform_Detect` (`0x142196ec0`, body to `0x14219731f`), simplified:

```
exe  = basename(argv[0])                     // 0x142195a80 (strips up to the last \ or /)
if VersionInfo ProductName == "Path Of Exile Hotcool Game Client":
    return (exe contains "Steam" or arg == "--steam") ? 3 : 2
if exe contains "Steam"  or any arg == "--steam": return 1
if exe contains "KG"     or any arg == "--kakao": return 4
if exe contains "EGS"    or any arg == "--epic":  return 5
return 0
```

| Value | Platform | Login screen | Credentials flow |
|---|---|---|---|
| 0 | standalone | `LoginScreen.ui` | `0x0003` password / saved token |
| 1 | Steam | `SteamLoginScreen.ui` | `0x0005` encrypted app ticket |
| 2 | Hotcool (Taiwan) | `HotCoolLoginScreen.ui` | `0x0003`, same path as 0 |
| 3 | Hotcool + Steam | `HotCoolSteamLoginScreen.ui` | `0x0005` |
| 4 | Kakao | `KakaoLoginScreen.ui` | `0x140ef4050` |
| 5 | Epic | `EpicLoginScreen.ui` | `0x140ef3a10` |
| 7, 8 | (never returned) | none | `0x0032` / `0x0033` token logins (`0x1420e7620`, `0x1420e7800`), dead code on PC |

Evidence for the `Steam` match: `FUN_140152460(exe, len, 0, L"Steam", 5)` is `wstring::find`, `-1` = not found. The `--steam` loop compares 7 wide characters. The strings are `L"--steam"` at `0x14350cae0`, `L"Steam"` at `0x14350caf0`, `"KG"` at `0x14350ca88`, `"EGS"` at `0x14350caa0`.

The cache: every getter is the MSVC thread-safe-static pattern `if (guard > TLS epoch) { init: DAT_144717118 = ClientPlatform_Detect(); }`. Guard `DAT_144717114`, value `DAT_144717118`. Getters: `ClientPlatform_IsSteam` (`0x140109b20`, 1 or 3), `ClientPlatform_IsHotcool` (`0x140109be0`, 2 or 3), `ClientPlatform_IsTokenLogin7or8` (`0x1401af120`), `ClientPlatform_Equals` (`0x142197330`, compares with `*param`). Callers of `ClientPlatform_Detect` are only these getters and a handful of inlined copies of the same init (each a `[WRITE]` xref to `DAT_144717118`).

## 2. Which screen and which flow

| Decision | Function | Rule |
|---|---|---|
| Login screen | `LoginState_BuildUi_SelectLoginScreen` (`0x140ee8cb0`, branches at `0x140ee9228`..`0x140ee9357`) | platform 2 → Hotcool; Steam (1/3) → Steam/HotCoolSteam; 5 → Epic; 4 → Kakao; else `LoginScreen.ui` |
| Auth start | `LoginState_StartAuthentication` (`0x140ef4c90`), runs once the key exchange is done (session state 2) | 7/8 → `0x140ef3530`; Steam → request the encrypted app ticket, callback `LoginState_OnSteamEncryptedAppTicket` (`0x140ef9ec0`) → `0x0005`; 5 → `0x140ef3a10`; 4 → `0x140ef4050`; **else `LoginState_StandaloneLogin` (`0x140ef4410`)** |
| Steam API init | `FUN_140110960` (`0x140110960`, "Unable to init Steam API") | `if (!ClientPlatform_IsSteam()) return 1;` before `SteamAPI_RestartAppIfNecessary` / `SteamAPI_Init` |

`LoginScreen.ui` (extracted from the bundle, present in 0.5.5e) has `username_textbox`, `password_textbox` (`mode = password`), `remember_login_checkbox`, `gateway_combobox`, `language_combobox`, `login_button`, `new_account_button`, `lost_password_button`. The Steam screen has the gateway combobox and a login button but no account fields, so `gateway_list.txt` works on both.

## 3. The credentials packet: C→S `0x0003`

Built in `ClientLoginSession_SendCredentials_0003` (`0x1420e7af0`), serialised by `Packet0003_Credentials_Serialize` (`0x1420e3b60`). Integers big-endian, as everywhere on the login connection. The session is the login `Tcpstream` of `ClientLoginSession`; the message is sent after the key exchange, like `0x0005`.

| Field | Type | Source / notes |
|---|---|---|
| opcode | u16 | `3` (not part of the Rust `Message` body) |
| mode | u32 | constant `2` (object +0x30). Meaning unknown (auth type / protocol revision); keep `2` |
| name_len, name | u16 + bytes | the account name typed into `username_textbox`, converted wide→UTF-8 (`WideCharToMultiByte(65001)`, `0x141020f20`). **Not** a wstring, no terminator |
| client_hash | 32 bytes | `LoginClientHash_Get32` (`0x1420e7580`, global `DAT_1445d5f50`); same value as in `0x0005` |
| password_hash | 32 bytes | password path: `Sha256_Bytes32` (`0x1421bbd70`) over the UTF-16LE password (`size * 2` bytes), SHA-256 IV at `0x143a0c198`. Saved-login path: the 32 bytes read from the credentials file (section 4) |
| machine_hash | 32 bytes | `0x1421c1340`; same as in `0x0005` |
| flag0 | u8 | password path: the "Remember Me" checkbox (`save_login`); saved-token path: 1 |
| flag1 | u8 | 0 = `password_hash` is a password hash; **1 = it is a saved login token** |
| flag2 | u8 | `DAT_143a0c054 | 0x40`: UI language id (0 = English, 6 / 9 other, see `0x1424a90d0`) with bit 6 set. The same `| 0x40` is `flags_lo` in `0x0005` |
| flag3 | u8 | optional byte of the login state (`+0x529`, valid when `+0x52a`), 0 when unset. `flags_hi` in `0x0005` is the same byte |

Total: 2 + 4 + 2 + len(name) + 96 + 4 bytes. `0x0005` in comparison: `u16 ticket_len, flags_lo, client_hash[32], machine_hash[32], flags_hi, ticket[...]`. The `0x0003` packet is the same identity block (client hash, machine hash, language flag) with an account name and a password hash in place of the Steam ticket. The builder passes `0x15180` (86400) as the second argument of the post-send call `FUN_142197fc0`; the `0x0005` builder passes 0 there (meaning not decoded, probably a response timeout in seconds).

Server reply: the same `0x0004 AuthenticationResult`; failures go through `FUN_140ef63a0` (see section 5).

Rust sketch for `poe-proto` (not added; this task must not touch `modules/`):

```rust
pub struct Credentials {            // opcode 0x0003, C->S
    pub mode: u32,                  // 2
    pub account_name: String,       // u16 length + UTF-8
    pub client_hash: [u8; 32],
    pub password_hash: [u8; 32],    // sha256(utf16le(password)) or the saved token
    pub machine_hash: [u8; 32],
    pub save_login: u8,
    pub token_login: u8,            // 1 = password_hash is a server-issued token
    pub flags_lo: u8,               // language | 0x40
    pub flags_hi: u8,
}
```

## 4. "Remember Me" is a server-issued token

This is the part that matters for accounts and for the launcher.

- On a successful `0x0004` the handler `AuthenticationResultHandler` (`0x140ef5140`) calls `LoginState_SaveLoginAfterAuth` (`0x140ef07d0`) with the **32-byte `session_token` of the packet**. If `save_login` is set in `poe2_production_Config.ini` (`[LOGIN]`) and the token is not all zero, `LoginCredentials_Save32` (`0x1423d0490`) writes the 32 raw bytes to the credentials file and the account name goes into the ini. With `save_login` clear the file is deleted (`FUN_1423cfe30`).
- On the next start `LoginState_StandaloneLogin` takes the **saved-token branch** when: the password box is empty, `[LOGIN] save_login` is true, the credentials file exists (`PathFileExistsW`) and `LoginCredentials_Load32` (`0x1423d03a0`) reads exactly 32 bytes. It then sends `0x0003` with that token as `password_hash` and `flag0 = flag1 = 1`.
- The file: `<Documents>\My Games\Path of Exile 2\` + `<name>`. The name is built by `FUN_141018010` from the configuration display name (default `poe2_production`) and the suffix `_login.credentials` (string at `0x14316a898`), `poe2_production_login.credentials` (seen on disk in the test prefix). `config.json` in the working directory can override it with `GGG_CREDENTIAL_FILE_NAME` (loader `FUN_141018170`). Under our Proton prefix the folder is `third_party/proton-prefix/pfx/drive_c/users/steamuser/Documents/My Games/Path of Exile 2/`, which already holds `poe2_production_Config.ini`.
- ini keys used by the login screen (`[LOGIN]`): `username` (last account name), `save_login`, `gateway_id`, `gateway_auto_select`, `hide_email`. The existing `account_name=` line in our prefix is written by the Steam path.
- Server consequences: on password login issue a random 32-byte token, return it in `AuthenticationResult.session_token`, store a hash of it (revocable, expiry, per cluster); on `flag1 = 1` treat `password_hash` as that token. Never send an all-zero token if the client should not remember anything. A failed login with code `0x0f` makes the client delete the credentials file (`FUN_140ef63a0`), and codes `0x62` / `0x1cf` set `LoginState +0x468` and show a re-enter-your-password message (text id `0x5eb`; the strings "Your saved credentials are out of date. Please re-enter your password." and "You are logging in from a new location..." exist at `0x14351e100` / `0x14351e2c0`). So `0x62` / `0x0f` are the natural codes for "token expired or revoked" (inferred, to be tried).

Launcher auto-login without any packet change: the launcher writes the 32-byte token to the credentials file, sets `[LOGIN] username=<name>` and `save_login=true`, and the DLL (or the user) presses Login with the password box empty. Optionally the DLL hooks `LoginCredentials_Load32` to return a token from `poe-hook.toml` instead of the file, but `PathFileExistsW` runs first, so a file must exist anyway.

## 5. Results and errors

The handler for non-zero results is `FUN_140ef63a0` (`0x140ef63a0`). Relevant parts, all read statically:

- **The `notice` of the `0x0004` packet is shown as the error message when present**; otherwise the client looks the message up by code (`FUN_1425cb780(language, code)`). The login server can therefore send any code with its own text ("Wrong password").
- Codes with special handling: `0x62`, `0x1cf` (flag +0x468, re-enter password message), `0x0f` (delete the credentials file), `0x08` (Steam only: marks the Steam content corrupt, forces a patch recheck), `0x54` (popup), `0x05` (platform 7/8 only).
- A message table for localized texts is at `0x1421ee510`. By its English strings (inferred, not verified at runtime): code `5` "The specified account does not exist.", `6` "The password is not correct for this account.", `9` "Access to the game is not currently enabled", `0x0c` timeout, `0x13` database error, `0x27` "Unable to make contact with server", `0x35` invalid argument, `0x47` banned, `0x49` too many login attempts. Use 5 and 6 for unknown account and wrong password and send a notice as well.
- The success path also contains a client-side report `"System or game DLLs are tampered!"` (`0x14315a128`, sent by `0x1420e9a40` when `LoginState +0x549 != 0 && +0x548 == 0`). `+0x549` is initialised to 0 (`0x140ee7d69`) and the only writes found are the 16-bit stores of `(byte)x` to `+0x548` at `0x140ef5597` / `0x140efa855`, which leave `+0x549` at 0, so the report looks inactive; our current Steam login works with `version.dll` in place (open question 3).

## 6. The patch

**Recommended: patch `ClientPlatform_Detect` to return 0.**

| | |
|---|---|
| Address / RVA | `0x142196ec0` / `0x2196ec0` (function entry) |
| Original bytes | `40 55 53 56 57 41 56 48 8d ac 24 70 fe ff ff 48` (`push rbp; push rbx; push rsi; push rdi; push r14; lea rbp,[rsp-0x190]`...) |
| New bytes | `31 C0 C3` (`xor eax,eax; ret`), rest of the function is dead |
| Safety | Pure function of process state, no side effects needed by others: it only reads argv, the module file name and version resource, and returns the enum. All call sites are the cached getters, so one patch covers every consumer |
| When | At `version.dll` load, before the first getter call (WinMain, after CRT init). `VirtualProtect` the page RWX, write 3 bytes, restore. Verify the original 3 bytes first and refuse (log) otherwise, like `crypto_bypass` does with its pattern. If the guard `*(i32*)0x144717114 != 0` at that moment (something already cached the Steam value), also store 0 into `*(i32*)0x144717118` |
| Config | A `login_mode = "standalone"` key in `poe-hook.toml` (default `"standalone"`; official mode forces `"steam"`); deploy writes it. Do it in `poe-hook` (permanent hooks) because it must be in place before the login state is built |

Why a getter patch and not data: the value is lazily initialised under the thread-safe-static guard, so writing the data early is overwritten and writing late races. Why not a branch: the screen choice, flow choice and Steam init each key on the getters; one change covers them all consistently. A mismatch (standalone screen but Steam flow) would send a `0x0005` without an account.

**Zero-code alternative for a quick test:** start a copy of the exe under a file name without `Steam` (for example `PathOfExile.exe`). `version.dll` loads next to any exe. This must go through `xtask` (never hand-edit `GameFiles/`); useful as an A/B check that the standalone screen works before the DLL patch exists. `--steam` on the command line only forces Steam, never standalone.

### Does the standalone path still need Steam?
- **Steam API init:** no. `FUN_140110960` returns 1 immediately when the platform is not Steam, so `SteamAPI_RestartAppIfNecessary`, `SteamAPI_Init`, the overlay hooks and the Steam Deck settings are skipped. `steam_api64.dll` stays next to the exe (it is imported/mentioned at `0x142eab9c0`) but is not initialised; `steam_appid.txt` becomes unnecessary.
- **Features lost:** Steam achievements/stats ("Steam not loaded. Achievements will not work" paths), Steam microtransaction purchase (`BuyPackSteamAsync`), Steam-specific shop/account linking. None matter for a private server.
- **Entitlement:** there is no local entitlement check on the standalone path; the only gate is the server's `0x0004` result.
- **Other platform-0 behaviour not yet read:** about 90 call sites of the getters (UI, shop, store links, `FUN_1401162d0` error text, `config.json` URLs). They are the same code the standalone client runs.

### Pitfalls
- **No integrity check on the code was found** along these paths; the DLL already patches the exe in memory (`crypto_bypass`), so this is not a new class of risk. The one tamper-related string is the login-time `"System or game DLLs are tampered!"` report (section 5, open question 3).
- **Official servers:** unchanged rule. The patch only changes which login packet is built; the read-only latch and the connection guard still apply. A standalone-mode client must never be pointed at an official server (the official login would reject it anyway).
- **`gateway_list.txt`, `Bundles2/gateway_list.txt`:** loaded by `FUN_140ee79c0` for the gateway combobox, not platform-specific as far as read.
- **Saved-token branch needs an empty password box** and an existing credentials file; with a stale file and a wrong token the server must answer `0x62` / `0x0f` or the user is stuck in a loop of failures.
- **`mode = 2`** in `0x0003` and `flags` should be echoed by tests; the server must not assume other values.

## 7. Fallback: launcher token in the Steam packet

Only if platform 0 is unusable. Facts:

- `0x0005` carries `u16 ticket_len` and an opaque ticket. In `LoginState_OnSteamEncryptedAppTicket` (`0x140ef9ec0`) the ticket is obtained with `ISteamUser` vtable slot `+0xa8` (`GetEncryptedAppTicket`-style call) into a **1024-byte stack buffer** (`local_428[1024]`, size capped at `0x400`), copied into a vector at `LoginState +0x298` and sent by `ClientLoginSession_SendSteamTicket_0005` (`0x1420e7e00`). The wire allows up to 65535 bytes, the client builds at most 1024.
- **The client does not validate the ticket locally** (it cannot: it is encrypted for GGG). The only local checks are Steam success codes (`Steam: Unable to get encrypted app ticket`, `Not connected to steam`, rate limit `0x19`, duplicate `0x1d`).
- So a token could replace the ticket bytes (hook the vector fill or the send), or the `flags_hi` byte could carry a mode bit. Costs: Steam must be running and logged in for `SteamAPI_Init` and the ticket call (or those calls must also be skipped), the login screen has no account field (a token would have to come from the launcher every time), and there is no password path for manual play.

**Decision: force the standalone screen (platform 0).** It is a one-getter patch, needs no Steam, gives a real account screen with Remember Me, and the saved 32-byte token is a ready-made launcher channel. Keep the Steam-ticket token idea documented only as a contingency.

## 7a. Platform 0 starts the standalone patcher (found in the first in-game test, 2026-10-20)

With platform 0 the exe first behaves like the **standalone patcher launcher** (window "Language", news pane, progress bar, LAUNCH button; `Client.txt`: `Started patching (N free bytes available)`). It contacts GGG's patch servers. This must never happen with our client.

**What decides it.** Not a config key and not the platform value alone. The start-up function `Client_Startup` (`0x14011b820`) calls `StandalonePatcher_Run` (`0x1401162d0`, call at `0x14011bf76`; returns `bool`, 0 = quit) and then `CmdLine_ReadSuccessEvent` (`0x140117c40`). `StandalonePatcher_Run`:

1. `ClientPlatform_IsSteam()` true: return 1 (this is why the Steam build never shows the patcher). Platform 5 (Epic): return 1.
2. Otherwise it loads the patch server list from the config singleton (`FUN_14010ac40`, built by `FUN_14010aca0`: `us.patch.pathofexile2.com` `0x142f76fd0`, the Taiwan servers `patch/patch2/patch3.pathofexile2.tw`, `kr-patch.pathofexile2.com`; platform 2 and the `--kakao` option pick other lists) and applies `--patching-server`.
3. **Skip test:** if `--disable-client-switching` is absent **and** `--success-event` is present, it returns 1 without opening anything (code at `0x140116a43`..`0x140116be9`, `bVar3`).
4. Else it opens the patcher window (`MainWindowClass`, `FUN_1428ca5f0`/`FUN_1428cba30`, "Could not create the User Interface for the patcher") and runs `Patcher_RunPatching` (`0x142737620`: "Started patching (...)" at `0x14358d340`; connects to the patch server, `/launcher/patch/start|success|fail`, `/client/record-patching-metrics` in `FUN_1427370c0`). With `--require-launch` the user must press LAUNCH.

The skip test is the real protocol: the patcher/launcher starts the actual client with `--success-event <inherited event handle>` and waits on the event; `CmdLine_ReadSuccessEvent` parses the number (`FUN_1423cd120`, `wistringstream >> int`, so it must be an integer) into `DAT_144431668` (initial value -1). At the end of `Client_Startup` (`0x14011c916`) a handle `!= -1` gets `SetEvent` and `CloseHandle` and is reset to -1. With a dummy value `0` both calls simply fail (`ERROR_INVALID_HANDLE`, nothing is closed). **Do not use a small positive number**: it could be a real handle of the process.

**Command-line table (`CmdLine_BuildOptionTable` `0x140110d60`, parser `CmdLine_ParseArgs` `0x1423cc1a0`).** Registered options (type 1 = flag, 3 = takes a value): `--no-agility-sdk`, `--no-nv`, `--no-lmdb`, `--login-port <n>`, `--patching-server <host>`, `--patching-auth <x>`, `--prev-proc-id <pid>` (waits for that process to exit, 15 s, then "Timed out waiting for patching process to exit."), `--require-launch`, `--fail`, `--nosound`, `--softwareaudio`/`-swa`, `--success-event <handle>`, `--24h2-mt`, `--test-*` (job queues, descriptors, frame graph, ...), `--force-safe-frame-graph`, `--device-resource-manager`, `--disable-static-entities`, `--descriptors-keep-alive`, `--descriptors-validate`, `--flush-descriptors`, `--flush-resources`, `--no-device-recovery`, `--enable-dx12-pso-cache`, `--disable-dx12-pso-cache`, `--nologo` (skips intro video), `--kakao`, `--disable-assert-box`, `-AUTH_PASSWORD` and `-epiclocale` (platform 5 only), `--language`/`-lang` (not Hotcool), `--use-defaults`, `--disable-client-switching`/`--dcs`. Values are given as `--opt value` or `--opt=value`. **There is no `--nopatch` option**; unknown options only log `[CMD][Unrecognized] <arg>` (the table allows unrecognised optional arguments), which is what the earlier test saw.

**Other platform-0 network behaviour.** Everything patch related is reachable only through `StandalonePatcher_Run`: `Patcher_RunPatching` has the single caller `StandalonePatcher_Run`; the metrics sender `FUN_1427370c0` is called only from `FUN_142733af0`, whose only caller is `Patcher_RunPatching`; the `/launcher/patch/*` urls (`FUN_1428cc930`) belong to the patcher UI. `FUN_14011a370` (also reading the config singleton, called early from `Client_Startup`) has no network callee (strings, `LoadStringA`, file name helpers). So skipping `StandalonePatcher_Run` removes every patch-server access that was found. The remaining platform-0 getter callers (about 90, UI/shop/store links) were not read one by one; any URL they open is a browser/shop link, not a patch download.

**Fix, layered (smallest first):**
1. **Launcher flag (primary).** `poe_hook_deploy::launch::client_args` adds `--success-event 0` to the Proton command line when the installed `poe-hook.toml` has `login_mode = "standalone"` and `mode != "official"`. The client then takes the same path as when started by the real patcher: no patcher, no network, no code patch. Manual start: `PathOfExileSteam.exe --success-event 0`.
2. **DLL safety net.** In `login_mode = standalone` the loader checks `GetCommandLineW`: if the client's own test would still run the patcher (`--success-event` missing, or `--disable-client-switching` present) it replaces the first three bytes of `StandalonePatcher_Run` (`48 8B C4`, `mov rax,rsp`) with `B0 01 C3` (`mov al,1; ret`), after verifying them. Failure to patch is logged as an error (`standalone patcher NOT skipped, it would contact GGG patch servers`). Not applied in Steam mode or official mode (the platform patch is also not applied there; the Steam path returns at once anyway).
3. Rejected: returning platform 1 to the patcher only (two answers from one getter needs a caller check and breaks the cache), patching `FUN_14010ac40` data (server names are used again by the network code of the patcher only, but the window would still appear).

Official mode keeps `login_mode = steam` (forced by the config), so none of this runs there.

**Window title (debug core).** `POE_HOOK_WINDOW_TITLE` now renames only windows of class `POEWindowClass` (the game's main window, `Window_CreateMain_POEWindowClass` `0x141050a30`, `RegisterClassW` + `CreateWindowExW`; the patcher uses `MainWindowClass`, message boxes `#32770`). The `CreateWindowExW` hook compares the class name argument, the `SetWindowTextW` hook calls `GetClassNameW` on the window.

## 8. Open questions (runtime; x64dbg only if something misbehaves)
Status 2026-10-21: items 1, 2 and 5 hold (the standalone login with a password and with a remember-me token works in game, `poe2_production_login.credentials` exists in the prefix, and the client starts without the patcher). Items 3, 4 and 6 have not been checked.

No x64dbg session is required to start. Verify these during the first test run with the patched DLL; the debug DLL log and `GameFiles/logs/Client.txt` are normally enough:
1. **Screen and packet:** after the patch, `Client.txt` should load `Metadata/UI/PreGameState/LoginScreen.ui` and the net trace should show a `0x0003` (not `0x0005`) as the first encrypted client message. If the Steam screen still appears, the cache was filled before the patch: breakpoint/log `0x142196ec0` and `0x144717118`.
2. **Credentials file name and location:** after one Remember-Me login, list `Documents/My Games/Path of Exile 2/` in the Proton prefix; expected `poe2_production_login.credentials`, 32 bytes. If it is missing, break on `LoginCredentials_Save32` (`0x1423d0490`), arg is the 32-byte buffer.
3. **Tamper report:** break on `0x1420e9a40` (the "System or game DLLs are tampered!" sender) and `0x140ef5541` (`cmp [rdi+0x549],0`). Expect it never to fire with `version.dll` loaded. If it fires, find the writer of `LoginState +0x548`.
4. **Flag bytes:** what sets `LoginState +0x529/+0x52a` (the optional byte sent as `flag3` / `flags_hi`). Break on `0x140ef4afa` / `0x140ef48a9` and read the value; expected 0.
5. **Patcher skipped:** with the standalone deploy and `cargo xtask run-client` (which passes `--success-event 0`), `Client.txt` must not contain `Started patching`, no patcher window appears, `[CMD] --success-event` is logged, and the debug log says `standalone patcher skipped by the client itself`. No `patch.pathofexile2.com` lookup/connection (check with `ss -tnp` or the DLL network guard log).
6. **Result codes:** confirm that `5` and `6` produce the "account does not exist" / "password is not correct" messages when the notice is empty (send each once from our server).

## Ghidra names added (2026-10-20)
`ClientPlatform_Detect`, `ClientPlatform_IsSteam`, `ClientPlatform_IsHotcool`, `ClientPlatform_IsTokenLogin7or8`, `ClientPlatform_Equals`, `LoginState_BuildUi_SelectLoginScreen`, `LoginState_StartAuthentication`, `LoginState_StandaloneLogin`, `LoginState_SaveLoginAfterAuth`, `LoginState_OnSteamEncryptedAppTicket`, `LoginState_TokenLogin7or8`, `ClientLoginSession_SendCredentials_0003`, `ClientLoginSession_SendSteamTicket_0005`, `Packet0003_Credentials_Serialize`, `Sha256_Bytes32`, `LoginClientHash_Get32`, `LoginCredentials_Load32`, `LoginCredentials_Save32`, `LoginCredentials_GetFilePath`; 2026-10-20 (patcher): `StandalonePatcher_Run`, `Client_Startup`, `CmdLine_ReadSuccessEvent`, `CmdLine_BuildOptionTable`, `CmdLine_ParseArgs`, `Patcher_RunPatching`, `Window_CreateMain_POEWindowClass`.
