# Login protocol (0.5.5e)

Connection: client to the login host from `gateway_list.txt` (ours: `[network] public_host` and `[network.login]` port, written by `cargo xtask deploy-client`).

## Observed sequence
1. **C→S `0x0002` ephemeral public key** (the first message, unencrypted). Captured 2026-10-12:
   ```
   00 02            opcode
   00 80            u16 key length = 128
   <128 bytes>      client DH public value (1024-bit)
   00 00            u16 signature length = 0 (the client does not sign)
   ```
   This is identical to PoE1 3.x `LOGIN_EPHEMERAL_PUBKEY`.
2. **S→C `0x0002`** with the same layout: the server's public value and a DSA signature. Our server sends 128 random bytes and an empty signature; the DLL's `crypto_bypass` derives the keys. **From here on, both directions are Salsa20-encrypted** (see [../re/crypto.md](../re/crypto.md)).
3. **C→S authentication.** On the Steam platform (the exe file name contains `Steam`): `0x0005` (229 bytes in the first capture, a protobuf-like blob from +0x45 = the Steam encrypted app ticket). On the standalone platform: `0x0003` with account name and password hash, see [below](#cs-0x0003-credentials-standalone-platform). Which one the client uses is decided by the platform getter and can be forced with a DLL patch: [../re/standalone-login.md](../re/standalone-login.md).
4. **C→S `0x0001`** every ~10 s: keep-alive (opcode only).

Verified 2026-10-12: the server decrypts the client stream correctly (clean `00 01` keep-alives).

## Message layouts

Encodings: integers big-endian. `wstring` uses the compact per-character encoding (see [framing.md](framing.md)). `astring` = u16 length + bytes. Rust types: `modules/protocol/poe-proto/src/login.rs`. Opcodes: `modules/protocol/opcodes/0.5.5e/login.toml`.

### C→S `0x0005` Authenticate (client serializer `0x1420e34f0`, built in `0x1420e7e00`)
| Field | Type | Notes |
|---|---|---|
| ticket_len | u16 | |
| flags_lo | u8 | UI language id (global `DAT_143a0c054`, 0 = English) `\| 0x40` |
| client_hash | 32 bytes | `LoginClientHash_Get32` `0x1420e7580`: a hash (SHA-256 family, `0x1421bbdf0`) of **the file named by argv[0], the client executable**, computed once on a worker thread at start (`0x1420eb470`; the getter waits until it is done). Confirmed in game (2026-10-21, standalone `0x0003` of 0.5.5e): the value the server saw is the exe's SHA-256, `7157984534da1b146e9b0be342aa9fd24272915b6f478aadef9830fc7873f89f`. Since WP A4 the login server judges it against `[client] hashes` of `versions/<version>.toml` and refuses an unknown build with code 8 (`InvalidProtocolVersion`) once `hashes_confirmed` is set ([../guides/new-game-version.md](../guides/new-game-version.md#what-the-server-does-with-the-clients-version)); it logs the value while unconfirmed (only for a hash that is not in the list). `hashes_confirmed = true` for 0.5.5e; the refusal was seen in game: the popup "There has been a patch that you need to update to. Please restart Path of Exile." and our notice in the log pane |
| machine_hash | 32 bytes | `0x1421c1340`: cached 32-byte machine id built by `0x1421c0d20` (fallback `0x1421c0c20` when all zero); stable per machine; only the dev machine-hash login (`[accounts] dev_machine_hash_login`) uses it as the account key |
| flags_hi | u8 | optional login-state byte (`LoginState +0x529`, valid when `+0x52a`), 0 when unset |
| ticket | bytes[ticket_len] | Steam encrypted app ticket (from `GetEncryptedAppTicket`). Encrypted for GGG, so the server cannot read it |

### C→S `0x0003` Credentials (standalone platform)
Sent instead of `0x0005` when the client platform is 0 (email/password screen). Serializer `0x1420e3b60`, builder `0x1420e7af0`; analysis and the patch that forces this path on the Steam exe: [../re/standalone-login.md](../re/standalone-login.md).
Opcode in `login.toml` (`Credentials`), Rust type `poe_proto::login::Credentials`; handled by `servers/login-server/src/auth.rs`.
| Field | Type | Notes |
|---|---|---|
| mode | u32 | constant `2` (object +0x30 of the packet class). The only value the client ever stores; meaning still unknown (auth type or protocol revision). What would settle it: the official `0x0003`, which only exists on the standalone platform and is not captured. Keep `2` and ignore it |
| account_name | u16 length + UTF-8 bytes | not a wstring |
| client_hash | 32 bytes | as in `0x0005` |
| password_hash | 32 bytes | SHA-256 of the UTF-16LE password; with `token_login = 1` the saved 32-byte token that an earlier `0x0004` `session_token` handed out |
| machine_hash | 32 bytes | as in `0x0005` |
| save_login | u8 | the Remember Me checkbox (1 on the token path) |
| token_login | u8 | 1 = `password_hash` is a saved login token |
| flags_lo | u8 | UI language id `\| 0x40` (same as `0x0005`) |
| flags_hi | u8 | optional login-state byte, 0 when unset |

Answer with the same `0x0004`. With `save_login = 1` (or a token login) a successful answer carries a fresh 32-byte `session_token`, which the client saves for Remember Me; without it the token is all zeros. `password_hash` is checked against the argon2id hash of the digest ([../architecture/accounts.md](../architecture/accounts.md)). Failures are listed under [Authentication results](#authentication-results).

### Authentication results
**The client has one code table: `Data/Balance/BackendErrors.dat`** (715 rows, `Id` and `Text`; the text of code *n* is row *n − 1*, so row 0 is code 1). `BackendError_GetText` (`0x1425cb780`) looks it up; the language picks a translated copy of the table under `data/balance/<language>/`. Every result code on the login connection goes through it: `0x0004` `result`, `0x0013` CharacterCreated `result`, `0x000d` DeleteCharacterResult `result`. An unknown row prints `bad-error-code`. The table is game data: read it with `poe_data_tools ... extract <dir> 'data/balance/backenderrors.datc64'` (`docs/guides/setup.md`) and let the server name codes by the `Id` column instead of hardcoding texts.

(The 16-entry table at `0x1421ee510`, which an earlier version of this page took for the login table, belongs to the standalone patcher only. Its caller is `StandalonePatcher_Run`.)

Codes the login server uses (`servers/login-server/src/auth.rs`, `auth::code`, which take the numbers from `poe_proto::backend_error`: one named list of the table rows the server uses, each with its `Id`; the server checks them against the real table at start and warns on a mismatch, `login-server/src/codes.rs`). `*` = special handling in `LoginState_HandleAuthenticationError` `0x140ef63a0`:

| Code | `Id` | Text (English table) | Client effect |
|---|---|---|---|
| `0` | | accepted | continues (see [flags](#authenticationresult-flags-and-the-suspicious-activity-popup)) |
| `5` | AccountDoesNotExist | The specified account does not exist. | error popup |
| `6` | InvalidPassword | The password is not correct for this account. | error popup |
| `8` * | InvalidProtocolVersion | There has been a patch that you need to update to. Please restart Path of Exile. | Steam only: marks the content corrupt |
| `9` | NotAllowedGameLogin | Access to the game is not currently enabled on this account. | error popup |
| `0x0f` * | UnexpectedDisconnect | An unexpected disconnection occurred. | **deletes the saved credentials file** (not on Steam); in game (WP B2 check) our notice shows only as a line in the status log at the lower left, no modal popup |
| `0x13` | DatabaseError | An unexpected database error occurred. Try again. | error popup |
| `0x47` (71) | AccountBanned | Your account has been banned by an administrator. | error popup |
| `0x49` (73) | LoginRateLimited | You have attempted to log in too often within a short time period. Try again later. | error popup with the table text; our notice (the wait) shows only in the status log |
| `0x4a` (74) | AccountLocked | Your account has been locked. Please contact support. | error popup |
| `0x4c` (76) | TemporaryLockout | Your account has been temporarily locked while an administrator works with it. | error popup |
| `0x54` (84) * | LeagueUnavailableToNonBeta | This league is only available to Beta players. | popup path of its own |
| `0x60` (96) | SavedCredentialsNewLocation | You are logging in from a new location. Please re-enter your password. | error popup only |
| `0x61` (97) | SavedCredentialsBad | Your saved credentials are out of date. Please re-enter your password. | error popup only (file kept) |
| `0x62` (98) * | LoginRequiresUnlockCode | Your account has been locked until you can provide an unlock code from your email. | **opens the Unlock window** ("Your account has been locked because it was accessed from another location. An unlock code has been sent to your email address...", code box, Unlock / Resend Email). Our `notice` is not shown |
| `0x1cf` (463) * | LoginRequiresUnlockCodeStrict | ... (strict) | the same Unlock window with the red "We have detected suspicious activity on your account. You will be required to enter login unlock codes more often until you change your password." in front |
| `0x208` (520) | EmailLoginDisabled | This login method has been disabled for this account. | error popup |
| `0x230` (560) | AccountNotVerified | Account is not verified. | error popup |
| other | | table text, or `bad-error-code` | error popup |

If `notice` is non-empty it replaces the table text in the popup (language-tagged, see the `notice` row of the `0x0004` table below); it does not change what the code does. Seen in game (WP B2 check, 2026-10-21): for `0x0f` and `0x49` the notice appears only as a line in the status log at the lower left, and the `0x49` popup keeps the table text.

**Close the connection after a refusal.** The client starts a new attempt (Log In button, Enter) only once the old login connection is gone. The server closes it right after a refused `0x0003` (`login-server/src/session.rs`); the client then also logs "An unexpected disconnection occurred." in the status log. Before that fix the client stayed on the login screen and sent nothing more after a wrong password.

**What our server sends (since WP B2).** `5` no such account, `6` wrong password, `9` Steam login while `dev_machine_hash_login` is off, `0x13` store error, `0x47` disabled account, **`0x49` too many password attempts** ([rate limits](#password-attempt-limits)), **`0x0f` for every saved login that cannot be used** (expired, revoked, unknown, of another cluster, or for another user name): the client deletes its credentials file and shows our notice ("Your saved credentials are out of date. Please re-enter your password."), so the next start shows the password screen. Never `0x62` (it opens the email Unlock window) and never `0x61` (it keeps the file, so the dead token would be retried at every start). The notice is our own wording for `0x0f`, `0x47`, `0x13`, `0x49` (it carries the wait) and the same words as the table for `5`/`6`; with an empty notice the client prints the table text (the unchecked part: [standalone-login.md](../re/standalone-login.md) open item 6). Only `0` has been seen at runtime; the rest is read from the client code and the table, and the in-game check of WP B2 covers it.

### Password attempt limits
`[security]` in `config/server.toml` (`crates/poe-accounts/src/ratelimit.rs`, one `LoginLimiter` shared by the login server and the web API). Failed password attempts (wrong password, or an unknown name) are counted per account name (case-insensitive, existing or not) and per client IP (IPv6 per /64) in a sliding window `login_failure_window_secs`. At `login_max_failures_per_account` (5) or `login_max_failures_per_ip` (20) the key is locked for `login_lockout_secs`; attempts while locked are answered `0x49` before the password is checked and do not extend the lock. A right password clears the account's count (not the IP's). Token logins are not counted or refused (a token cannot be guessed). `0` turns a limit off. The `0x49` reply carries the wait in the notice ("... Try again in 10 minutes."), because the table text has none. The Steam dev path has no password and is not limited. The web API answers the same condition with HTTP 429 ([web-api.md](../architecture/web-api.md)).

### S→C `0x0004` AuthenticationResult (deserializer `0x14214da60`, handler `0x1420e829d`)
| Field | Type | Notes |
|---|---|---|
| account_id | u32 | stored in the logged-in account record (account entry +0x20, 0x78 bytes each, vector at login session +0xef0) |
| result | u16 | 0 = success; non-zero goes to the error path ([codes](#authentication-results)) |
| notice | wstring (+0x38) | **on success** (result 0, no flag below set): a non-empty notice is shown in a popup after login. **On an error** it replaces the table text. Language-tagged: the first part (up to the first NUL) is the default text; each later part is `NUL, <language id as one wchar>, text`. With language 0 (English) the first part is used; for another language the part whose id matches is used (`0x140ef5140`, `0x140ef63a0`). It used to carry the account name, which caused the "Exile1 / OK" popup. Official: empty (2 bytes) |
| session_token | 32 bytes | kept with the account; on the standalone path it is the Remember Me token the client saves (all zero = nothing to save) |
| account_flags | 2 bytes | split into booleans in the account entry, see [flags](#authenticationresult-flags-and-the-suspicious-activity-popup). **Not captured**: the official `0x0004` is redacted in every capture (63 bytes = 2 opcode + 4 account id + 2 result + 2 empty notice + 32 token + 2 flags + 2 name length + 17 name) |
| account_name | astring (+0x68) | the account name kept by the login session (official: 17 bytes, the `Name#1234` form). The short `0x0004` that answers a resume is 46 bytes: the name is empty |

### AuthenticationResult flags and the suspicious-activity popup
`0x1420e8391`..`0x1420e8427` (case 4 of `ClientLoginSession_ProcessPacket`) splits the two flag bytes into ten booleans of the account entry. `LoginState_AuthenticationResultHandler` (`0x140ef5140`) and the screens read them:

| Wire bit | Entry field | Meaning (client code) | Official / ours |
|---|---|---|---|
| byte 0 bit 0 | +0x30 | no reader found in the login, select or create code (unused or read through an accessor) | ours 1 |
| byte 0 bit 1 | +0x31 | "early league access": a league whose start time has not come is shown as started (`0x1406c19a0` picks the start time by this flag and by league flag +0xe0 bit 5); also shows a widget on the login screen (`0x140effac0`) | ours 1 |
| byte 0 bit 2 | +0x32 | **terms of use**: opens the ToU window after login and holds the login | keep 0 |
| byte 0 bit 3 | +0x33 | **cheating warning** popup (`LoginStateWarnAboutHacks`) | keep 0 |
| byte 0 bit 4 | +0x34 | **RMT warning** popup (`LoginStateWarnAboutRMT`) | keep 0 |
| byte 0 bit 5 | +0x37 | strict-unlock banner: the banner is only built when this is set on every account entry (`0x140f0f6c6` ff.), and a later update hides it again while it is set (`0x140f0f690`) | ours 1 (default `0x23`) |
| byte 0 bit 6 | +0x38 | **strict unlocks enabled** (see below): the character-select screen shows the red "suspicious activity" banner | ours 0 since WP B2 (it was 1, the cause of the popup; default `0x23`) |
| byte 0 bit 7 | +0x35 | **update required** popup (`LoginErrorGameUpdateRequired`, "There has been a patch that you need to update to...", title "Warning"); checked after the ToU, cheating and RMT flags | keep 0 |
| byte 1 bit 0 | +0x36 | per-row hook in the select screen (`0x140f060de`, calls a list widget method for characters of the account) | ours 1, effect not seen |
| byte 1 bit 1 | +0x39 | hides a select-screen widget (+0x6e0 of the select screen object, `0x1406c0b64`): the widget is visible only while this bit is clear. Probably an upsell button | ours 1 |

Order of the success path with no error and `+0x468 == 0` (the "unlock window is open" flag): save the login, then ToU (bit 2) before everything else, else cheating (bit 3), else RMT (bit 4), else update required (bit 7), else the notice if it is non-empty, else continue to the character list (`0x140ef62c0`). If the Unlock window was open (`+0x468`) a success shows "Account Unlocked / Account successfully unlocked. You can now attempt to log in again." instead.

**The suspicious-activity popup.** ClientStrings row `StrictUnlocksEnabledMessage` (row 4307) is the red text "We have detected suspicious activity on your account. You will be required to enter login unlock codes more often until you change your password." It is put on the character-select screen by `SelectScreen_UpdateStrictUnlockBanner` (`0x140f0f600`, widget names `strict_unlock_warning_frame` / `strict_unlock_warning_label`) and made visible by `SelectScreen_ShowWidgets` (`0x140f05b20`) when:
1. the platform is **not Steam** (`ClientPlatform_IsSteam()` false: this is why it appeared only after our standalone patch; on the Steam login the code is skipped), and
2. at least one account entry has **+0x38** set (`AuthenticationResult` account_flags byte 0 **bit 6**, mask `0x40`), and
3. the select screen's flag `+0x4b0` is clear (set once the user has unlocked the account in this session), and
4. for the frame to exist at all, no entry has +0x37 clear (byte 0 bit 5, mask `0x20`).

Until WP B2 the default `[accounts] flags = [0x63, 0x03]` set both bit 5 and bit 6. **The default is now `[0x23, 0x03]`** (`poe-config` default and both config files): bit 6 clear removes the banner. Bit 5 stays set; clear it too (`0x03` as byte 0) if the banner still shows. Confidence: the chain is read from the code and the string; it has not been run (the in-game check of WP B2 confirms it; no capture is needed).

Related but different: the error code `0x1cf` (`LoginRequiresUnlockCodeStrict`) opens the Unlock window with the same red text in front. The server never sends it.

**"Get Full Access" is not an account flag.** The escape-menu button comes from the in-game InitialState flag at `+0x3edc` (`0x1420edbd6`, `[accounts] full_access`), not from `0x0004`. The earlier note that `[0x63, 0x03]` removes it was wrong: bits 0, 1, 5, 6 only added the select-screen widgets above.

## Incoming packet factory
The client's login factory is `0x14214fa60` (opcode → class, then vtable slot 2 = deserialize). Classes by opcode (vtable):

| Opcode(s) | Vtable | Size |
|---|---|---|
| 0x01 | `0x143506518` | 0x10 |
| 0x02 | `0x143506228` | 0x50 |
| 0x04 | `0x143509d40` | 0x90 |
| 0x0b, 0x0d, 0x0f, 0x44 | `0x143509d60` (u32 + u16) | 0x18 |
| 0x11 | `0x143509ea0` | 0x60 |
| 0x13 | `0x143509d00` | 0x18 |
| 0x14 | `0x143509e20` | 0x88 |
| 0x15 | `0x143509e80` | 0x78 |
| 0x16 | `0x143509e00` | 0x18 |
| 0x17, 0x1e | `0x143509d80` (u16) | 0x18 |
| 0x1a | `0x143509da0` | 0x30 |
| 0x1b | `0x143509e60` | 0x38 |
| 0x1f, 0x20 | `0x143506248` (u32) | 0x18 |
| 0x24, 0x25, 0x26, 0x28, 0x29 | `0x143509ce0` (string) | 0x30 |
| 0x3c | `0x143509dc0` | 0x20 |
| 0x3e | `0x143509e40` | 0x18 |
| 0x41 | `0x143509ec0` | 0x30 |
| 0x43 | `0x143509de0` | 0x38 |
| 0x47 | `0x143509d20` | 0x30 |

Vtable slots: 0 = destructor, 1 = serialize, 2 = deserialize, 3 = ?. The dispatcher is the switch at `0x1420e8193`, inside a function starting at `0x1420e8130`.

## Login flow after authentication (verified 2026-10-12)
| Step | Direction | Message | Notes |
|---|---|---|---|
| 1 | C→S | `0x0018` RequestCharacterList | `u32 account_id` |
| 2 | S→C | `0x0015` CharacterList | must carry the same `account_id` (the client drops it otherwise) |
| 3 | S→C | `0x001b` LeagueList | sent by our server right after the character list |
| 4 | C→S | `0x0010` RequestCharacterDetails | `u32 account_id, u8 index`; sent automatically for the selected character |
| 5 | S→C | `0x0011` CharacterDetails | same tail layout as the list; empty blob/pairs are accepted |
| — | C→S | `0x0012` CreateCharacter | `astring name, astring league, u32 account_id, u32 0, u32 0, wstring start_state, 32 bytes` (official capture: the first `u32` is the account id, the next two are 0; `start_state` is `<class>Default`-style text such as the metadata name plus `Default`; sender not decoded yet) |
| — | S→C | `0x0013` CharacterCreated | `u16 result (0 = ok), u8 notify, u32 account_id` (deserializer `0x14214ea70`); then a fresh CharacterList. **Result codes: see [Create and delete result codes](#create-and-delete-result-codes).** On 0 the create screen (callback `CreateCharacterState_OnCharacterCreated` `0x140ed2ab0`) plays the creation scene, finds the new character **by name** in the list (`0x140ec8b20`) and sends PlayCharacter (`0x140ecee50`, "character_creation_done"), so a new character goes straight into the game. Before 2026-10-14 the server sent `0x0016`, which is a different message (handler `0x1420e8da6`, signal +0x288, meaning unknown): the first character still worked because the client fell back to the select screen, but a second character left the create screen stuck after Continue |

New characters are stored with their start area (`[player] start_area`).

### CharacterList entry fields
Wire order of one entry (reader `CharacterEntries_Read` `0x1427164c0`, 0x58-byte client struct): `wstring name (+0x00), wstring league (+0x20), u8 +0x4d, u8 +0x51, u8 +0x4c, u32 +0x40, u8 +0x4e, u16 +0x44, u16 +0x46, u16 +0x48, u16 +0x4a, u8 +0x4f, u8 +0x50`. Consumers: `SelectScreen_SetEntry` (`0x1406b9b90`) fills one select-screen row from it.

| Field | Meaning | Official values seen |
|---|---|---|
| +0x4e (u8) | **bits 0-4: class** (`Characters.IntegerId`; drives the 3D preview and the icon `Art/2DArt/UIImages/Common/Icon<class>`); **bits 5-7: ascendancy index** (0 = none, up to 6; adds a suffix to the icon name, `0x1425ff110`) | `2` (no ascendancy) |
| +0x4a (u16) | current area: `WorldAreas.HASH16`. Looked up first (`0x1406b8e90`); if the area is not found the row falls back to +0x48 | **0 for a brand-new character and after logging out in a town**; the zone's hash after logging out in a field zone (the intro zone, 3860) |
| +0x48 (u16) | fallback area: `WorldAreas.HASH16`; if the row is not a town the client shows its parent town (`WorldAreas` +0x70) | the intro zone (3860) for a new character; the town's hash after logging out in a town |
| +0x4c (u8) | level | 1, 2 |
| +0x4d (u8) | league banner: `LeagueFlag` row + 1 | `0x9b` |
| +0x40 (u32) | experience (51 after three 17-xp kills, the same values as the Player component delta's mask `0x0001` field) | |
| +0x51 (u8) | flags: bit 3 goes to the select-screen entry (`0x1406b9b90` → `0x1406b8890`); official characters that have played have `0x40` | `0x40` (`0x00` while the character is brand new) |
| +0x44 (u16) | **MTX portrait frame id**: 0 = `DefaultIconFrame`; non-zero = `Art/2DArt/UIImages/Common/MTX/SupporterIconFrame<id>` (`SelectScreen_SetEntryPortraitFrame` `0x1406b8460`) | 0 |
| +0x46 (u16) | **portrait variation row**: 0 = the class icon; non-zero = `Art/2DArt/UIImages/Common/PortraitVariations/<name of the row>` (`0x1425ff110`) | 0 |
| +0x50 (u8) | **`MiniQuestStates.dat` row** of the character's quest state, used for the "Act N" label of a row whose two area hashes find no area (`SelectScreen_SetEntryLocation` `0x1406b9010`, text "Act {0}" with the row's act number). If both hashes are 0 the row shows a fixed text (string id 205) instead | 0 |
| +0x4f (u8) | stored, no reader found in the select or create screens (read through an accessor, or unused) | 0 |

Official lists of characters that have played also carry a blob (`blob_30`, 91 to 131 bytes) and (stat id, value) pairs (`pairs_48`); a fresh character has neither.

### CharacterList trailer (and the CharacterDetails tail)
`0x0015` handler: case at `0x1420e8534`; `0x0011` CharacterDetails has the same tail. The tail describes **one character, the one the select screen previews**; the handler builds an event with it (`0x1420e8660`..`0x1420e8852`) that reaches `SelectScreen_OnCharacterDetails` (`0x140f0c090`), which caches it per character index.

| Field | Meaning | Official |
|---|---|---|
| +0x28 (u8) | **index of the character the tail belongs to** (in the list). The handler builds the preview event only when the list has more entries than this value (`0x1420e8655`: count > +0x28), with `0xffffffff` otherwise. Inferred from the handler, not verified at runtime | 0 |
| +0x29 (u8) | character slot limit. The select screen refuses creation when the count reaches it (`0x140f0d475`; the create button popup `CreateCharacterButtonMaxCharactersReachedPopup`) | `0x18` (24) |
| +0x74, +0x76 (u16, u16) | passed on to the cached preview (`0x140f0c090` reads them at event +0xf88 / +0xf8a); meaning unknown | 0, 0 |
| +0x30 blob (u16 length + bytes) | the **31 visible equipment slots** (a decoded array of 31 items of 128 bytes each, `0x1420e5a90`) | 91 to 131 bytes |
| +0x48 pairs (varuint count, `u16 key, varint value`) | stat (id, value) pairs, copied into the preview event as 8-byte entries | about 33 pairs |
| +0x68 (u32) | kept in the preview event (+0xfb0); meaning unknown | 0 |
| +0x6c (u32) | **`ShapeshiftForms.dat` row** (the form the preview shows, Druid); looked up by `FUN_1423e55b0` against that table (`0x140495730`) | 0 |

Unknown pieces and what would settle them: +0x74/+0x76 and +0x68 (always 0 in every official capture, so any value works; a capture of a character with a non-default form or a supporter pack, or the consumer `FUN_140ec6670`, would name them), +0x4f (same), account flag +0x30 (no reader found).

### LeagueEntry (0xe8 bytes, reader `0x142716bd0`)
Wire order: five wstrings (+0x10 id, +0x40, +0x60 display name used by the select screen, +0x80, +0xa0), three u64 (+0xd0 start, +0xd8, +0xc8 end), 3 flag bytes (+0xe0), u8 +0xc0, u8 +0xc1, u8 +0xc2. After the list: a flags byte (bit 0 → +0x30, bit 1 → +0x31).

| Bit | Meaning |
|---|---|
| +0xe0 bit 1 | hardcore (red text and banner) |
| +0xe0 bit 2 | timed event (Events panel `0x1408b3dd0`; bits 3/4/5 gate on the start/end times) |
| +0xe0 bit 7 | **permanent league**: gets a banner at character creation and is not offered in the character-select "Join" list. Set for Standard and Hardcore (`[[leagues]] permanent`) |
| +0xe1 bit 1 | **variant** of the league named at +0x80 (Hardcore of Standard): no banner of its own and not in the "Join" list (`0x1406bee00`); copied to characters (+0x3ab). Config `[[leagues]] variant_of` |
| +0xe1 bit 4 | entry faded (0.5 alpha) |
| +0xe1 bits 5, 6 | extra confirmation warnings at character creation (`0x140ece3e0`) |
| +0xe2 bit 2 | "green" variant / creation warning |

- The character-select **"Join" list** (lower right) has one entry per league unless it has +0xe0 bit 7 or +0xe1 bit 1, or its end time has passed.
- **Character creation** (second and later characters; the first goes straight to the class step) starts with a league step: "Choose a league by clicking on a banner below, then click OK". The banners are built by `0x140e7ca80`: one per entry with +0xe0 bit 7 and without +0xe1 bit 1. Variants are matched to their base banner by +0x80 = the base league's id (the Hardcore checkbox; +0xa0, probably SSF, and +0x60 are compared too). Creation needs a selected league (`0x140ece3e0`, screen +0x320 / +0x318). Until 2026-10-14 the server set +0xe1 bit 1 on both leagues, so there were no banners and the class step could not be reached.
- The **league label** under a character is grey (`0xff676767`) for every non-hardcore league and dark red for hardcore (`0x1406b95f0`). Grey is normal.
- The **league banner** comes from CharacterEntry **+0x4d = `LeagueFlag.dat` row + 1** (`0x140c7e460`). 0 or 1 give the grey event banner. The row's IsHC/IsSSF pick red or blue and the green variant. Config: `[[leagues]] flag = "StandardFlag"`.

## Deleting a character (verified 2026-10-14)
1. **C→S `0x000c` DeleteCharacter**, sent after the confirmation dialog (builder `0x140ed8149`, delete screen state): `astring name` (`u16` byte length + bytes), `u8 slot`, `u32 account_id`. In the official capture the byte is 0 and the `u32` is the account id; the older local note `01 00 00 00 01` is the same layout with other values. The byte is the low byte of a `u32` at screen +0x214 that the select screen stores when the delete screen opens (`0xffffffff` when unset; probably the list index or the local player); our server identifies the character by name and may ignore it.
2. **S→C `0x000d` DeleteCharacterResult**: `u16 result, u32 account_id` (class `0x143509d60`, shared with 0x0b/0x0f/0x44; official: `00 00` + the account id). The handler signals event +0xb88. The delete screen's callback (`0x140ed8710`) closes on result 0; any other code is looked up in `BackendErrors.dat` and shown as an error text. Until the reply arrives, the dialog shows "Waiting for response...".
3. **S→C `0x0015` CharacterList**: the client keeps its old list after a delete and does not ask for a new one, so the server sends it.

## Create and delete result codes
Both results go through `BackendError_GetText` (`0x1425cb780`) as described under [result codes](#authentication-results): the client shows the table text of the code in the create screen (`0x140ec96b0`, a message line) or the delete screen. **The packets carry no text**, so the server picks the code and the client prints the string. Official captures only contain result 0 for both (create and delete of a test character, several times); the failure codes below are read from the table and the callbacks, not captured.

| Situation | Code | `Id` | Text |
|---|---|---|---|
| name already used (global) | 21 (`0x15`) | CharacterNameAlreadyExists | A character with this name already exists. |
| bad characters in the name | 22 | CharacterNameInvalid | The character name contains invalid characters. Valid names only contain letters and underscores. |
| name too short | 23 | CharacterNameTooShort | The character name is too short. |
| name too long | 24 | CharacterNameTooLong | The character name is too long. |
| unknown class / start state | 25 | CharacterInvalidClass | Invalid character class. |
| unknown league | 26 | LeagueDoesNotExist | This league does not exist. |
| bad flags (hardcore, SSF, ...) | 27 | InvalidCharacterFlags | Invalid character flags. |
| no free slot | 49 (`0x31`) | TooManyCharacters | You have no character slots left. |
| league over | 55 / 140 | LeagueHasEnded / LeagueAlreadyEnded | The league has ended. You cannot create characters in it anymore. / This league has already ended. |
| league closed | 56 | LeagueClosedForEntry | Entry to this league has closed. |
| league not started | 139 | LeagueNotYetStarted | The league has not yet started and so you cannot move. ... |
| league password | 79 | LeaguePasswordInvalid | The password is not correct for this league. |
| delete: no such character | 16 (`0x10`) | CharacterDoesNotExist | The specified character does not exist. |
| delete: not the owner | 17 | NotCharacterOwner | An attempt was made to retreive a character that this account is not the owner of. |
| delete: cosmetics still on items | 94 | CantDeleteWithMicrotransactions | Please remove all cosmetic effects from items on this character first. ... |
| any: database failure | 19 | DatabaseError | An unexpected database error occurred. Try again. |

Until WP B2 the server sent `1` for every create failure and for a failed delete (row 1 is `AccountNameAlreadyExists`, the wrong text for both). Now (`login-server/src/session.rs`): a name under 3 characters `23`, over 23 `24`, other characters than letters, digits and `_` `22` (the client's text says letters and underscores only; the server accepts digits, but the client's name box already drops digits, spaces and other characters), unknown league `26`, no free slot (`[accounts] max_characters`) `49`, unknown start state `25`, name taken `21`, store error `19` (seen in game: name taken shows "A character with this name already exists."; `22`, `23` and `24` cannot be reached from the stock client, which filters characters, keeps Continue disabled under 3 characters and caps the box at 23, so only the server tests cover them); flags `27` is named but cannot occur (the create packet has no flag field). Delete: no such character `16`, a character of another account `17`, store error `19`, and the reply carries the account id in its `u32`. The `notify` byte of CharacterCreated must stay 0: when it is non-zero the client builds a request to `https://<web host>/api/tracking-conversions/` (`0x1420e89c1`..`0x1420e8ac0`), a marketing call to the official website.

## Keep-alive and resuming (verified 2026-10-14)
- The client keeps its login connection open while in game (unless the server sends `0x0017`, below) and sends **`0x0001` KeepAlive** (no body) every 10 seconds. The server answers with an empty `0x0001` (the factory maps it to the empty class `0x143506518`).
- **C→S `0x0008` ResumeSession**: `u32 account id, u64 token` (raw). Sent on a new login connection right after the key exchange, after the instance server's `0x174` ExitSession ("Exit to character selection"; see [instance.md](instance.md#leaving-the-game-2026-10-14)). The server redeems the ticket (`TicketRegistry::redeem_resume`, single use) and answers like a login (AuthenticationResult). The client then requests the character list.

## Hand-off to the instance server
| Message | Layout |
|---|---|
| C→S `0x0e` PlayCharacter | `u8 flags, varuint count, (u32 character_index, u32 account_id)[count]` (one entry per local split-screen player) |
| S→C `0x14` InstanceDetails | `astring host, u16 port (+0x38), u32 (+0x30), u16 area HASH16 (+0x3a, must exist in WorldAreas), u32 (+0x34), 64-byte key material (+0x3c), 8 bytes (+0x7c), u8 (+0x84)` |

- **S→C `0x0017`** (`u16` reason): the client closes its login connection (handler case in `0x1420e8130`, `ClientLoginSession_ProcessPacket` → `0x1420e7140` → `0x1420e71d0`: `shutdown()`). Official servers send `0x0017 0x0014` right after InstanceDetails; we do not (in-game transfers use the open connection, see [server-admin.md](../guides/server-admin.md#transfers)).
- InstanceDetails while the client is in an instance is received and ignored; at character selection it is followed even unasked.

Evidence: deserializer `0x14214ece0`; handler `0x1420e8ef8`; the instance connect (`0x14211dbb0`) logs host = the string and port = struct +0x80 (packet +0x38).

Unknown parts of `InstanceDetails`: the connect routine (`0x14211dbb0`) uses only the host, port `+0x38` and area `+0x3a` plus the key material; the byte **`+0x84`** is compared with 7 and the result is kept in the instance session (`+0x2184`, meaning unknown). Official `0x0014` is redacted in the captures, so the values of `+0x30`, `+0x34`, the 8 bytes at `+0x7c` and `+0x84` are not known from the wire. What would settle them: an unredacted capture of one hand-off, or the readers of session `+0x2184`.

## Still unknown (login protocol)
| Item | Where | What would settle it |
|---|---|---|
| `0x0003` `mode` (always 2) | above | official standalone `0x0003` capture; harmless |
| `0x0004` byte-0 bit 0 (`+0x30`), byte-1 bit 0 (`+0x36`) effect | flags table | a client run with the bit toggled (static search found no or only a vague reader) |
| `0x0004` official flag values | flags table | an unredacted `0x0004` capture (the DLL redacts the auth packets); not needed to remove the popup |
| `CharacterList` +0x74, +0x76, +0x68, entry +0x4f | tail table | always 0 officially; a capture of a Druid in a shapeshift form or with MTX portrait settings |
| `CharacterList` +0x28 as "preview index" | tail table | a capture of a list with two characters (official lists seen have one) |
| `InstanceDetails` `+0x30`, `+0x34`, `+0x7c`, `+0x84` | hand-off | unredacted capture |
| `CreateCharacter` the two zero `u32` and the trailing 32 bytes | login flow table | decode the sender (not found by the opcode searches); official values are in `dumps/0.5.5e/captures/official/*/packets.jsonl` |
| Create/delete failure codes at runtime | result codes | **gate G-LOGIN** (see below) only if the table mapping must be checked at runtime |

**Gate G-LOGIN is not needed to implement WP B2.** The code-to-text mapping comes from the client's own table (`BackendErrors.dat`) and the callbacks that print it. A read-only official capture of a failed create (taken or invalid name) and a delete of a missing character would only confirm which code official servers pick for each case (`21`, `22` and `16` are the expected ones); ask the user for it if B2 wants to match them exactly.

## Implementation notes for WP B2 (done 2026-10-21)
WP B1 findings and what WP B2 did with them (`servers/login-server`, `poe-proto`, `poe-config`):

| # | WP B1 finding | Status |
|---|---|---|
| 1 | account flags `[0x23, 0x03]`: bit 6 clear removes the strict-unlock banner | done: default in `poe-config`, `config/server.toml` and the example; comment fixed. In-game check pending |
| 2 | stale, revoked and unknown tokens: `0x0f` + the "out of date" notice instead of `0x62` | done (`auth::code::TOKEN_INVALID`; `auth::code::TOKEN_STALE` removed). In-game check pending |
| 3 | create failures use the table codes | done, see [Create and delete result codes](#create-and-delete-result-codes) |
| 4 | failed delete `16`/`17`/`19`, account id in the reply | done (`DeleteCharacterResult.account_id`) |
| 5 | `CharacterCreated.notify` stays 0 | kept; a test checks the bytes |
| 6 | `session_token` all zero unless Remember Me | unchanged |
| 7 | resume answer carries the account name | unchanged |
| 8 | texts from `BackendErrors.dat` | partly: the codes are named once in `poe_proto::backend_error` (each with its `Id`; code = row + 1) and verified against the table at start (`codes::verify`, also a test that runs when `GameFiles/` exists); the client prints the table text for create/delete (no notice exists there). Login refusals keep short own notices (see above) |
| 9 | CharacterEntry field names | done: `portrait_frame` (+0x44), `portrait_variation` (+0x46), `quest_state` (+0x50); `DeleteCharacterResult.unk` is `account_id` |
| new | password attempt limits | done, [above](#password-attempt-limits) |
