# Login crypto (0.5.5e)

## Official scheme
**Library:** Crypto++, identified by RTTI names (`DL_GroupParameters_GFP`, `DL_PublicKeyImpl<DL_GroupParameters_DSA>`, `SignatureVerificationFailed`).

**Handshake:**
1. The client sends `KeyExchange` (0x0002): `u16 key_len (128), key, u16 sig_len (0)`. The key is its ephemeral 1024-bit Diffie-Hellman public value.
2. The server replies with `KeyExchange` (0x0002): `u16 key_len, key, u16 sig_len, sig`. The client parses this into a packet object, with the key as a `std::string` at +0x10 and the signature at +0x30.
3. The login packet dispatcher (switch at `0x1420e8193`, opcodes 0x02–0x47) handles opcode 2 at `0x1420e81c0`. It calls the **verify-and-derive** routine `0x14219f010(session, &key, &sig)`, which does three things:
   - Verifies the DSA signature over the server key against GGG's built-in key. On failure it throws **"General Rhoa Failure #3"**.
   - Calls `0x14219ee40`, which runs the Crypto++ `DH2::Agree` unified DH. It combines the static keys at session+0x750/+0x758 with the ephemeral pair. On failure it throws **"General Rhoa Failure #1"**.
   - Hashes the agreed value with SHA-512 into the 64-byte buffer that `session+0x60` points to.
4. The dispatcher then builds the stream cipher with `0x14219f3b0(obj, key_material, is_client=1)` and installs it on the Tcpstream with `0x142198530`.

**Crypto session constructor:** `0x14219dc40`.
- Loads two DH groups from hex strings: a 1024-bit group for the ephemeral keys and a larger one.
- Stores static keys and loads a BER DSA public key, validated at level 3. On failure it throws **"General Rhoa Failure #2"**.

## Stream cipher
`0x14219f3b0` creates two Crypto++ Salsa20 instances (key setup at `0x142b030b0`; vtables `0x14350cca8` and `0x14350d800`). For a client (`is_client = 1`) the key material `m` is split like this:

| Direction | Key | IV |
|---|---|---|
| client→server (client encrypts) | `m[0..32]` | `m[32..40]` |
| server→client (client decrypts) | `m[0..32]` | `m[48..56]` |

The IV offset formula is `((is_client ^ 1) + 2) * 16` for the first instance and `(is_client + 2) * 16` for the second. Both directions are continuous keystreams over the whole connection. The rounds come from the Crypto++ default (20). This is assumed, and confirmed once the server decrypts client traffic.

## Private-server bypass (`poe-hook` feature `crypto_bypass`)
- The DLL finds the verify-and-derive routine by its prologue pattern and replaces it. The replacement writes `SHA-512(server key bytes)` into the key buffer and returns. There is no signature check and no DH.
- The server (`servers/login-server/src/handshake.rs`) sends 128 random bytes as its key with an empty signature, then derives the same key material (`poe_net::KeyMaterial::from_server_public_key`).

## Other crypto in the binary
- `0x14219cde0`: SHA-384(8-byte seed), then Salsa20, applied in place to a buffer.
- It is used by `0x142102c20`, a loader for **server-delivered code modules**:
  - Decrypts the module, checks an FNV-1a hash, and manually maps a PE.
  - Resolves its imports and calls its entry point.
  - Wipes memory with a MurmurHash-based PRNG afterwards.
  - This is anti-cheat style. Our server does not need to send modules unless the client requires one later.
- `0x142bc20e0` / `0x142bc2250`: a ChaCha-based CSPRNG in the memory allocator. Not network related.
- The "HNC Protocol Violated" string (`0x1435130b8`) belongs to the HNC side channel below.

## HNC side channel
The instance EnableEncryption handler (`0x14212203e`) creates two cipher pairs from the 64-byte key material in `InstanceDetails` +0x38:
1. The packet ciphers (`0x14212aa90` → `0x14219f3b0`).
2. The HNC bit queue at stream +0xd0 (`0x14212ab00` → `0x1421c50b0`). Its key material is the session material XORed with a 64-byte table (`0x142f10960`), then six byte pairs are swapped; the Salsa20 key/IV split is the same as for the packets (is_client = 1). The table and swaps are in `protocol/0.5.5e/hnc.toml` (`HncKeys::derive`).

The client's outgoing HNC bytes ride in `0x1c` frame reports, the server's in `0x1d` ping replies (bit layouts in [../protocol/instance.md](../protocol/instance.md)). An idle stream is zero bytes; the server sends keystream bytes (encrypted zeros). Verified 2026-10-14: the client accepts them for minutes, while all-zero bits crash it.
