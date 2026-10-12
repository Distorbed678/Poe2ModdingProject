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
   This is identical to PoE1 3.x `LOGIN_EPHEMERAL_PUBKEY`. The server is expected to reply with its own `0x0002`, carrying its public value and a signature that the client verifies against a key built into the client. After that, both directions are encrypted. See `re/crypto.md` (in progress).
