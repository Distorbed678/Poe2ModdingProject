# Wire format

Status: **assumptions from PoE1 research, not yet verified for 0.5.5e.** Each item is marked once confirmed.

- **Transport:** TCP. Login and instance are separate connections.
- **Framing:** no length prefix. A message is `opcode` followed by fields, so the receiver must know every layout to split the stream. An unknown opcode is fatal on the client ("Unable to deserialise packet"). *(unverified)*
- **Opcode:** u16 big-endian. *(confirmed on login: first message `00 02`)*
- **Integers:** big-endian. *(confirmed for u16 length fields on login)*
- **`wstring`** *(confirmed, reader `0x141cfb3c0`)*: a u16 character count, then one entry per character:
  - `b < 0x80`: one byte, ASCII;
  - `0x80` followed by u16 little-endian: a BMP code unit (not a surrogate);
  - `0xc0` followed by u32 little-endian: a code point (supplementary characters become a surrogate pair).

  Any other lead byte raises "Protocol violation reading wstring from the network". A stream flag (+0x254) switches to plain UTF-16 (2 bytes per character); this is not seen in practice.
- **`astring`:** u16 byte length, then bytes. *(confirmed, `0x14087cd70`)*
- **Compact integers:** 1–4 byte prefix-coded varint. *(unverified)*
- **Encryption:** Salsa20 per direction after the login key exchange. *(confirmed; see [../re/crypto.md](../re/crypto.md))*

Implemented in `modules/protocol/poe-net/src/codec.rs` (`Reader`/`Writer`).
