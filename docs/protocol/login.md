# Login protocol (0.5.5e)

Connection: client to the login host from `gateway_list.txt` (ours: `127.0.0.1:20481`).

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
3. **C→S `0x0005` authentication** (229 bytes in the first capture). It contains a protobuf-like blob starting at +0x45 (`08 02 10 … 2a 90 01 …`), probably the Steam encrypted app ticket. Layout to be decoded.
4. **C→S `0x0001`** every ~10 s: keep-alive (opcode only).

Verified 2026-10-12: the server decrypts the client stream correctly (clean `00 01` keep-alives).

## Message layouts

Encodings: integers big-endian. `wstring` uses the compact per-character encoding (see [framing.md](framing.md)). `astring` = u16 length + bytes. Rust types: `modules/protocol/poe-proto/src/login.rs`. Opcodes: `modules/protocol/opcodes/0.5.5e/login.toml`.

### C→S `0x0005` Authenticate (client serializer `0x1420e34f0`, built in `0x1420e7e00`)
| Field | Type | Notes |
|---|---|---|
| ticket_len | u16 | |
| flags_lo | u8 | `0x40` set plus a client global |
| client_hash | 32 bytes | from `0x1420e7580` |
| machine_hash | 32 bytes | from `0x1421c1340`; stable per machine; the server uses it as the account key |
| flags_hi | u8 | |
| ticket | bytes[ticket_len] | Steam encrypted app ticket (from `GetEncryptedAppTicket`). Encrypted for GGG, so the server cannot read it |

### S→C `0x0004` AuthenticationResult (deserializer `0x14214da60`, handler `0x1420e829d`)
| Field | Type | Notes |
|---|---|---|
| account_id | u32 | stored in the logged-in account record |
| result | u16 | 0 = success; non-zero goes to the error path |
| notice | wstring (+0x38) | shown in a popup after login when non-empty; may hold language-tagged sections (`0x140ef5140`). It used to carry the account name, which caused the "Exile1 / OK" popup |
| session_token | 32 bytes | kept with the account |
| account_flags | 2 bytes | split into booleans in the account entry. **Byte 0:** bit 2 opens the terms-of-use window, bit 3 the cheating warning, bit 4 the RMT warning, bit 7 "update required" (`LoginState::AuthenticationResultHandler` `0x140ef5140`); keep these clear. Setting bits 0, 1, 5 and 6 of byte 0 plus bits 0 and 1 of byte 1 (`[0x63, 0x03]`, config `[accounts] flags`) removes the "Get Full Access" button; which single bit does it is TBD |
| account_name | astring (+0x68) | the account name kept by the login session |

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
| — | C→S | `0x0012` CreateCharacter | `astring name, astring league, u32 (1), u32, u32, wstring start_state, 32 bytes` |
| — | S→C | `0x0013` CharacterCreated | `u16 result (0 = ok), u8 notify, u32 account_id` (deserializer `0x14214ea70`); then a fresh CharacterList. On 0 the create screen (callback `0x140ed2ab0`) plays the creation scene, finds the new character **by name** in the list (`0x140ec8b20`) and sends PlayCharacter (`0x140ecee50`, "character_creation_done"), so a new character goes straight into the game. Before 2026-10-14 the server sent `0x0016`, which is a different message (handler `0x1420e8da6`, signal +0x288, meaning unknown): the first character still worked because the client fell back to the select screen, but a second character left the create screen stuck after Continue |

New characters are stored with their start area (`[player] start_area`).

### CharacterList entry fields (confirmed so far)
| Field | Meaning |
|---|---|
| +0x4e (u8) | class: `Characters.IntegerId` (drives the 3D preview) |
| +0x4a (u16) | current area: `WorldAreas.HASH16` (select screen `0x140f02cef`) |
| +0x48 (u16) | fallback area: `WorldAreas.HASH16` |
| +0x4c (u8) | assumed level |
| +0x4d (u8) | league banner: `LeagueFlag` row + 1 |
| +0x40 (u32) | experience (official: 51 after three 17-xp kills, the same values as the Player component delta's mask `0x0001` field) |
| +0x51 (u8) | flags: bit 3 goes to the select-screen entry (`0x1406b9b90` → `0x1406b8890`); official characters that have played have `0x40` |

Official lists of characters that have played also carry a 91-byte blob (`blob_30`, the same bytes as a component delta of component 8 seen in game, likely the visible gear) and (stat id, value) pairs (`pairs_48`); a fresh character has neither.

### CharacterList trailer
| Field | Meaning |
|---|---|
| +0x29 (u8) | character slot limit. The select screen refuses creation when the count reaches it (`0x140f0d475`) |

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
1. **C→S `0x000c` DeleteCharacter**, sent after the confirmation dialog: `astring name` (`u16` byte length + bytes), then 5 bytes not decoded yet (`01 00 00 00 01` when deleting the first of two characters).
2. **S→C `0x000d` DeleteCharacterResult**: `u16 result, u32` (class `0x143509d60`, shared with 0x0b/0x0f/0x44). The handler signals event +0xb88. The delete screen's callback (`0x140ed8710`) closes on result 0; any other code is looked up and shown as an error text. Until the reply arrives, the dialog shows "Waiting for response...".
3. **S→C `0x0015` CharacterList**: the client keeps its old list after a delete and does not ask for a new one, so the server sends it.

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
