# Wire format

Status: **assumptions from PoE1 research, not yet verified for 0.5.5e.** Each item is marked once confirmed.

- **Transport:** TCP. Login and instance are separate connections.
- **Framing:** no length prefix. A message is `opcode` followed by fields, so the receiver must know every layout to split the stream. An unknown opcode is fatal on the client ("Unable to deserialise packet"). *(unverified)*
- **Opcode:** u16 big-endian. *(confirmed on login: first message `00 02`)*
- **Integers:** big-endian. *(confirmed for u16 length fields on login)*
- **`wstring`:** u16 count of UTF-16 code units, then UTF-16LE data. *(unverified)*
- **`astring`:** u16 byte length, then ASCII. *(unverified)*
- **Compact integers:** 1–4 byte prefix-coded varint. *(unverified)*
- **Encryption:** Salsa20-family stream cipher per direction, with keys from a signed Diffie-Hellman exchange at login. The instance keys are delivered in the login hand-off. *(PoE1 2018; the 0.5.5e scheme is under investigation, see re/crypto.md once written.)*

Implemented in `crates/poe-net/src/codec.rs` (`Reader`/`Writer`).
