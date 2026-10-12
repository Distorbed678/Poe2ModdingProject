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

Encodings: integers big-endian. `wstring` uses the compact per-character encoding (see [framing.md](framing.md)). `astring` = u16 length + bytes. Rust types: `crates/poe-proto/src/login.rs`. Opcodes: `protocol/0.5.5e/login.toml`.

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
| account_name | wstring | |
| session_token | 32 bytes | kept with the account |
| account_flags | 2 bytes | the client splits them into individual flag bits (meaning TBD) |
| message | astring | passed to a message/URL routine; empty = none |

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
