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
