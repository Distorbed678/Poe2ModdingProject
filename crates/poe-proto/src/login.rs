//! Login channel messages (client 0.5.5e). Layouts were taken from the client's packet
//! (de)serializers; see `Documentation/protocol/login.md`.

use poe_net::{DecodeError, Reader, Writer};

use crate::{Direction, Message};

/// Ephemeral DH public value and signature (both directions use the same layout).
#[derive(Debug, Clone, Default)]
pub struct KeyExchange {
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl KeyExchange {
    fn encode_body(&self, w: &mut Writer) {
        w.u16(self.public_key.len() as u16)
            .bytes(&self.public_key)
            .u16(self.signature.len() as u16)
            .bytes(&self.signature);
    }

    fn decode_body(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let len = r.u16()? as usize;
        let public_key = r.bytes(len)?.to_vec();
        let len = r.u16()? as usize;
        let signature = r.bytes(len)?.to_vec();
        Ok(Self {
            public_key,
            signature,
        })
    }
}

/// Client's key exchange.
#[derive(Debug, Clone)]
pub struct ClientKeyExchange(pub KeyExchange);

impl Message for ClientKeyExchange {
    const NAME: &'static str = "KeyExchange";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        self.0.encode_body(w);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        KeyExchange::decode_body(r).map(Self)
    }
}

/// Server's key exchange reply.
#[derive(Debug, Clone)]
pub struct ServerKeyExchange(pub KeyExchange);

impl Message for ServerKeyExchange {
    const NAME: &'static str = "KeyExchange";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.0.encode_body(w);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        KeyExchange::decode_body(r).map(Self)
    }
}

/// Keep-alive (no body), sent by the client about every 10 seconds.
#[derive(Debug, Clone, Copy)]
pub struct KeepAlive;

impl Message for KeepAlive {
    const NAME: &'static str = "KeepAlive";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, _w: &mut Writer) {}
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self)
    }
}

/// S→C keep-alive reply (no body; the client's login factory maps 0x01 to the empty class
/// `0x143506518`).
#[derive(Debug, Clone, Copy)]
pub struct KeepAliveReply;

impl Message for KeepAliveReply {
    const NAME: &'static str = "KeepAliveReply";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, _w: &mut Writer) {}
    fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self)
    }
}

/// Client authentication (serializer `0x1420e34f0`).
#[derive(Debug, Clone)]
pub struct Authenticate {
    /// Low flag byte (`0x40` set by the client, plus a client-global value).
    pub flags_lo: u8,
    /// 32 bytes from `0x1420e7580` (client-side hash; exact meaning TBD).
    pub client_hash: [u8; 32],
    /// 32 bytes from `0x1421c1340` (stable per machine; used as the account key).
    pub machine_hash: [u8; 32],
    pub flags_hi: u8,
    /// Platform token (Steam encrypted app ticket).
    pub ticket: Vec<u8>,
}

impl Message for Authenticate {
    const NAME: &'static str = "Authenticate";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u16(self.ticket.len() as u16)
            .u8(self.flags_lo)
            .bytes(&self.client_hash)
            .bytes(&self.machine_hash)
            .u8(self.flags_hi)
            .bytes(&self.ticket);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let ticket_len = r.u16()? as usize;
        Ok(Self {
            flags_lo: r.u8()?,
            client_hash: r.array()?,
            machine_hash: r.array()?,
            flags_hi: r.u8()?,
            ticket: r.bytes(ticket_len)?.to_vec(),
        })
    }
}

/// Authentication result (deserializer `0x14214da60`, handler at `0x1420e829d`).
#[derive(Debug, Clone)]
pub struct AuthenticationResult {
    pub account_id: u32,
    /// 0 = success; anything else is shown as an error.
    pub result: u16,
    /// Notice shown in a popup after login when non-empty (+0x38; language-tagged sections
    /// are supported by the client). This slot used to carry the account name, which is why
    /// a popup with the name appeared after every login.
    pub notice: String,
    /// Opaque 32 bytes stored with the logged-in account.
    pub session_token: [u8; 32],
    /// Two bytes of account flags; the client splits them into individual bits.
    pub account_flags: [u8; 2],
    /// Account name (+0x68), kept by the login session.
    pub account_name: String,
}

impl Message for AuthenticationResult {
    const NAME: &'static str = "AuthenticationResult";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u32(self.account_id)
            .u16(self.result)
            .wstring(&self.notice)
            .bytes(&self.session_token)
            .bytes(&self.account_flags)
            .astring(&self.account_name);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            account_id: r.u32()?,
            result: r.u16()?,
            notice: r.wstring()?,
            session_token: r.array()?,
            account_flags: r.array()?,
            account_name: r.astring()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticate_matches_captured_length() {
        // First capture: ticket_len 0x9f, total message 229 bytes including the opcode.
        let msg = Authenticate {
            flags_lo: 0x40,
            client_hash: [0; 32],
            machine_hash: [0; 32],
            flags_hi: 0,
            ticket: vec![0; 0x9f],
        };
        let mut w = Writer::new();
        msg.encode(&mut w);
        assert_eq!(w.len() + 2, 229);
        let back = Authenticate::decode(&mut Reader::new(w.as_slice())).unwrap();
        assert_eq!(back.ticket.len(), 0x9f);
    }
}

/// Sent after authentication with the selected account's id (client code `0x140f047d3`).
#[derive(Debug, Clone, Copy)]
pub struct RequestCharacterList {
    pub account_id: u32,
}

impl Message for RequestCharacterList {
    const NAME: &'static str = "RequestCharacterList";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u32(self.account_id);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            account_id: r.u32()?,
        })
    }
}

/// C→S: delete a character (sent after the confirmation dialog). Captured 2026-10-14:
/// `00 0c | 00 08 "deleteme" | 01 00 00 00 01`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteCharacter {
    /// Byte string (`u16` length).
    pub name: String,
    /// Trailing bytes, not decoded yet (`01 00 00 00 01` in the capture).
    pub unk: [u8; 5],
}

impl Message for DeleteCharacter {
    const NAME: &'static str = "DeleteCharacter";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.astring(&self.name).bytes(&self.unk);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            name: r.astring()?,
            unk: r.array()?,
        })
    }
}

/// C→S: resume a session after leaving an instance for character selection. The client sends
/// it on a new login connection, right after the key exchange, with the values the instance
/// server gave in `0x174` ExitSession (instead of authenticating). Captured 2026-10-14:
/// `00 08 | 00 00 00 01 | 88 77 66 55 44 33 22 11` for `u32` 1 and `u64` 0x1122334455667788.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumeSession {
    pub account_id: u32,
    /// Sent raw (little-endian), as the instance server wrote it.
    pub token: u64,
}

impl Message for ResumeSession {
    const NAME: &'static str = "ResumeSession";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u32(self.account_id).bytes(&self.token.to_le_bytes());
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            account_id: r.u32()?,
            token: u64::from_le_bytes(r.array()?),
        })
    }
}

/// S→C reply to [`DeleteCharacter`] (class `0x143509d60`, shared with 0x0b/0x0f/0x44: `u16`
/// at +0x14, then `u32` at +0x10). Signals event +0xb88, which the delete screen
/// (`0x140ed8710`) handles: `result` 0 closes it as deleted, anything else is shown as an
/// error (text looked up by code).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteCharacterResult {
    pub result: u16,
    pub unk: u32,
}

impl Message for DeleteCharacterResult {
    const NAME: &'static str = "DeleteCharacterResult";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u16(self.result).u32(self.unk);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            result: r.u16()?,
            unk: r.u32()?,
        })
    }
}

/// One character in [`CharacterList`] (entry reader `0x1427164c0`, 0x58-byte client struct).
/// Field names after `name` are provisional until their meaning is confirmed in the UI code;
/// the comments give the client struct offsets.
#[derive(Debug, Clone, Default)]
pub struct CharacterEntry {
    pub name: String,
    /// +0x20
    pub league: String,
    /// +0x4d
    pub unk_4d: u8,
    /// +0x51
    pub unk_51: u8,
    /// +0x4c
    pub unk_4c: u8,
    /// +0x40
    pub unk_40: u32,
    /// +0x4e
    pub unk_4e: u8,
    /// +0x44
    pub unk_44: u16,
    /// +0x46
    pub unk_46: u16,
    /// +0x48
    pub unk_48: u16,
    /// +0x4a
    pub unk_4a: u16,
    /// +0x4f
    pub unk_4f: u8,
    /// +0x50
    pub unk_50: u8,
}

impl CharacterEntry {
    fn encode(&self, w: &mut Writer) {
        w.wstring(&self.name)
            .wstring(&self.league)
            .u8(self.unk_4d)
            .u8(self.unk_51)
            .u8(self.unk_4c)
            .u32(self.unk_40)
            .u8(self.unk_4e)
            .u16(self.unk_44)
            .u16(self.unk_46)
            .u16(self.unk_48)
            .u16(self.unk_4a)
            .u8(self.unk_4f)
            .u8(self.unk_50);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            name: r.wstring()?,
            league: r.wstring()?,
            unk_4d: r.u8()?,
            unk_51: r.u8()?,
            unk_4c: r.u8()?,
            unk_40: r.u32()?,
            unk_4e: r.u8()?,
            unk_44: r.u16()?,
            unk_46: r.u16()?,
            unk_48: r.u16()?,
            unk_4a: r.u16()?,
            unk_4f: r.u8()?,
            unk_50: r.u8()?,
        })
    }
}

/// Character list (deserializer `0x14214e090`, client struct offsets in comments).
#[derive(Debug, Clone, Default)]
pub struct CharacterList {
    /// Account the list belongs to; the client drops the list unless it matches a
    /// logged-in account (handler `0x1420e8534`). Client +0x70.
    pub account_id: u32,
    pub characters: Vec<CharacterEntry>,
    /// +0x28
    pub unk_28: u8,
    /// +0x29
    pub unk_29: u8,
    /// +0x74
    pub unk_74: u16,
    /// +0x76
    pub unk_76: u16,
    /// +0x30: u16-length byte blob.
    pub blob_30: Vec<u8>,
    /// +0x48: (u16 key, signed varint value) pairs.
    pub pairs_48: Vec<(u16, i32)>,
    /// +0x68
    pub unk_68: u32,
    /// +0x6c
    pub unk_6c: u32,
}

impl Message for CharacterList {
    const NAME: &'static str = "CharacterList";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u32(self.account_id).u8(self.characters.len() as u8);
        for c in &self.characters {
            c.encode(w);
        }
        w.u8(self.unk_28)
            .u8(self.unk_29)
            .u16(self.unk_74)
            .u16(self.unk_76)
            .u16(self.blob_30.len() as u16)
            .bytes(&self.blob_30)
            .varuint(self.pairs_48.len() as u32);
        for &(k, v) in &self.pairs_48 {
            w.u16(k).varint(v);
        }
        w.u32(self.unk_68).u32(self.unk_6c);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let account_id = r.u32()?;
        let count = r.u8()?;
        let characters = (0..count)
            .map(|_| CharacterEntry::decode(r))
            .collect::<Result<_, _>>()?;
        let unk_28 = r.u8()?;
        let unk_29 = r.u8()?;
        let unk_74 = r.u16()?;
        let unk_76 = r.u16()?;
        let blob_len = r.u16()? as usize;
        let blob_30 = r.bytes(blob_len)?.to_vec();
        let pairs = r.varuint()?;
        let pairs_48 = (0..pairs)
            .map(|_| Ok((r.u16()?, r.varint()?)))
            .collect::<Result<_, DecodeError>>()?;
        Ok(Self {
            account_id,
            characters,
            unk_28,
            unk_29,
            unk_74,
            unk_76,
            blob_30,
            pairs_48,
            unk_68: r.u32()?,
            unk_6c: r.u32()?,
        })
    }
}

/// One league in [`LeagueList`] (entry reader `0x142716bd0`, 0xe8-byte client struct).
/// Text fields must not be empty; meanings of the later fields are provisional.
#[derive(Debug, Clone, Default)]
pub struct LeagueEntry {
    /// Internal id, e.g. "Standard" (client +0x10).
    pub id: String,
    /// Display name (client +0x40).
    pub name: String,
    /// client +0x60
    pub text_60: String,
    /// client +0x80
    pub text_80: String,
    /// client +0xa0
    pub text_a0: String,
    /// client +0xd0 (start time?)
    pub time_d0: u64,
    /// client +0xd8 (end time?)
    pub time_d8: u64,
    /// client +0xc8
    pub time_c8: u64,
    /// client +0xe0: bitfield. Byte 0 bit 1 = hardcore (red banner, `0x140ecaed9`);
    /// byte 1 bit 5 and byte 2 bit 2 are also tested by the UI (meaning TBD).
    pub raw_e0: [u8; 3],
    /// client +0xc0
    pub flag_c0: u8,
    /// client +0xc1: league category id compared by the select screen (0x39 = none?).
    pub flag_c1: u8,
    /// client +0xc2
    pub flag_c2: u8,
}

impl LeagueEntry {
    fn encode(&self, w: &mut Writer) {
        w.wstring(&self.id)
            .wstring(&self.name)
            .wstring(&self.text_60)
            .wstring(&self.text_80)
            .wstring(&self.text_a0)
            // u64s are read raw by the client (no byte swap): little-endian.
            .bytes(&self.time_d0.to_le_bytes())
            .bytes(&self.time_d8.to_le_bytes())
            .bytes(&self.time_c8.to_le_bytes())
            .bytes(&self.raw_e0)
            .u8(self.flag_c0)
            .u8(self.flag_c1)
            .u8(self.flag_c2);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            id: r.wstring()?,
            name: r.wstring()?,
            text_60: r.wstring()?,
            text_80: r.wstring()?,
            text_a0: r.wstring()?,
            time_d0: u64::from_le_bytes(r.array()?),
            time_d8: u64::from_le_bytes(r.array()?),
            time_c8: u64::from_le_bytes(r.array()?),
            raw_e0: r.array()?,
            flag_c0: r.u8()?,
            flag_c1: r.u8()?,
            flag_c2: r.u8()?,
        })
    }
}

/// League list (deserializer `0x14214f070`, handler `0x1420e918e`).
#[derive(Debug, Clone, Default)]
pub struct LeagueList {
    /// Server time in seconds since the Unix epoch; the client stores the offset to its clock.
    pub server_time: u64,
    pub leagues: Vec<LeagueEntry>,
    /// Bit 0: stored in the login session (meaning TBD). Bit 1: append to the existing
    /// list instead of replacing it.
    pub flags: u8,
}

impl Message for LeagueList {
    const NAME: &'static str = "LeagueList";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        // The client reads the u64 raw (no byte swap): send it little-endian.
        w.bytes(&self.server_time.to_le_bytes())
            .u32(self.leagues.len() as u32);
        for l in &self.leagues {
            l.encode(w);
        }
        w.u8(self.flags);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let server_time = u64::from_le_bytes(r.array()?);
        let count = r.u32()?;
        let leagues = (0..count)
            .map(|_| LeagueEntry::decode(r))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            server_time,
            leagues,
            flags: r.u8()?,
        })
    }
}

/// Create a character (client serializer `0x1420e4b40`, built in `0x1420e9b60`).
#[derive(Debug, Clone)]
pub struct CreateCharacter {
    pub name: String,
    pub league: String,
    /// Client +0x50 (observed 1; meaning TBD).
    pub unk_50: u32,
    /// Client +0x54.
    pub unk_54: u32,
    /// Client +0x58.
    pub unk_58: u32,
    /// `CharacterStartStates.Id`, e.g. "DexFourDefault".
    pub start_state: String,
    /// Client +0x5c: hash of an optional input (zero when unused).
    pub hash: [u8; 32],
}

impl Message for CreateCharacter {
    const NAME: &'static str = "CreateCharacter";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.astring(&self.name)
            .astring(&self.league)
            .u32(self.unk_50)
            .u32(self.unk_54)
            .u32(self.unk_58)
            .wstring(&self.start_state)
            .bytes(&self.hash);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            name: r.astring()?,
            league: r.astring()?,
            unk_50: r.u32()?,
            unk_54: r.u32()?,
            unk_58: r.u32()?,
            start_state: r.wstring()?,
            hash: r.array()?,
        })
    }
}

/// Character creation result (deserializer `0x14214dd00`, handler `0x1420e8da6`, which
/// forwards all three fields to its listeners). Provisional: the meaning of the fields is
/// inferred, see `Documentation/protocol/login.md`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CharacterCreated {
    pub account_id: u32,
    /// 0 = success.
    pub result: u16,
    pub unk_16: u16,
}

impl Message for CharacterCreated {
    const NAME: &'static str = "CharacterCreated";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u32(self.account_id).u16(self.result).u16(self.unk_16);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            account_id: r.u32()?,
            result: r.u16()?,
            unk_16: r.u16()?,
        })
    }
}

#[cfg(test)]
mod create_tests {
    use super::*;

    #[test]
    fn create_character_matches_capture() {
        // Captured 2026-10-12 (Ranger "test" in Standard).
        let mut bytes = vec![0x00, 0x04];
        bytes.extend_from_slice(b"test");
        bytes.extend_from_slice(&[0x00, 0x08]);
        bytes.extend_from_slice(b"Standard");
        bytes.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x0e]);
        bytes.extend_from_slice(b"DexFourDefault");
        bytes.extend_from_slice(&[0; 32]);
        let mut r = Reader::new(&bytes);
        let c = CreateCharacter::decode(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!((c.name.as_str(), c.league.as_str()), ("test", "Standard"));
        assert_eq!(c.start_state, "DexFourDefault");
        assert_eq!(c.unk_50, 1);
    }
}

/// Sent by the client for the selected character on the select screen (account id +
/// list index); answered with [`CharacterDetails`].
#[derive(Debug, Clone, Copy)]
pub struct RequestCharacterDetails {
    pub account_id: u32,
    pub index: u8,
}

impl Message for RequestCharacterDetails {
    const NAME: &'static str = "RequestCharacterDetails";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u32(self.account_id).u8(self.index);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            account_id: r.u32()?,
            index: r.u8()?,
        })
    }
}

/// Details of one character (deserializer `0x14214e650`). Same trailing layout as
/// [`CharacterList`]; the blob and pairs probably carry equipment/appearance (TBD).
#[derive(Debug, Clone, Default)]
pub struct CharacterDetails {
    /// Client +0x48
    pub account_id: u32,
    /// Client +0x4c
    pub index: u8,
    /// Client +0x4e
    pub unk_4e: u16,
    /// Client +0x50
    pub unk_50: u16,
    /// Client +0x10: u16-length byte blob.
    pub blob_10: Vec<u8>,
    /// Client +0x28: (u16 key, signed varint value) pairs.
    pub pairs_28: Vec<(u16, i32)>,
    /// Client +0x54
    pub unk_54: u32,
    /// Client +0x58
    pub unk_58: u32,
}

impl Message for CharacterDetails {
    const NAME: &'static str = "CharacterDetails";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u32(self.account_id)
            .u8(self.index)
            .u16(self.unk_4e)
            .u16(self.unk_50)
            .u16(self.blob_10.len() as u16)
            .bytes(&self.blob_10)
            .varuint(self.pairs_28.len() as u32);
        for &(k, v) in &self.pairs_28 {
            w.u16(k).varint(v);
        }
        w.u32(self.unk_54).u32(self.unk_58);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let account_id = r.u32()?;
        let index = r.u8()?;
        let unk_4e = r.u16()?;
        let unk_50 = r.u16()?;
        let blob_len = r.u16()? as usize;
        let blob_10 = r.bytes(blob_len)?.to_vec();
        let pairs = r.varuint()?;
        let pairs_28 = (0..pairs)
            .map(|_| Ok((r.u16()?, r.varint()?)))
            .collect::<Result<_, DecodeError>>()?;
        Ok(Self {
            account_id,
            index,
            unk_4e,
            unk_50,
            blob_10,
            pairs_28,
            unk_54: r.u32()?,
            unk_58: r.u32()?,
        })
    }
}

/// "Play" on the character select screen (client serializer `0x1420e4660`). One entry per
/// local player (split-screen on consoles): `(character index, account id)`.
#[derive(Debug, Clone, Default)]
pub struct PlayCharacter {
    /// Bit 0..6 from client +0x10, bit 7 from client +0x11 (meaning TBD).
    pub flags: u8,
    pub selections: Vec<(u32, u32)>,
}

impl Message for PlayCharacter {
    const NAME: &'static str = "PlayCharacter";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.flags).varuint(self.selections.len() as u32);
        for &(index, account) in &self.selections {
            w.u32(index).u32(account);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let flags = r.u8()?;
        let count = r.varuint()?;
        let selections = (0..count)
            .map(|_| Ok((r.u32()?, r.u32()?)))
            .collect::<Result<_, DecodeError>>()?;
        Ok(Self { flags, selections })
    }
}

/// Hand-off to an instance server (deserializer `0x14214ece0`, handler `0x1420e8ef8`,
/// connect `0x14211dbb0`).
#[derive(Debug, Clone)]
pub struct InstanceDetails {
    /// Host the client connects to (client +0x10).
    pub host: String,
    /// TCP port (client +0x38).
    pub port: u16,
    /// Client +0x30 (provisional: ticket).
    pub unk_30: u32,
    /// Target area, `WorldAreas.HASH16` (client +0x3a; looked up, the hand-off fails if
    /// unknown).
    pub area_hash: u16,
    /// Client +0x34.
    pub unk_34: u32,
    /// 64 bytes of stream key material for the instance connection (client +0x3c).
    pub keys: [u8; 64],
    /// Client +0x7c.
    pub unk_7c: [u8; 8],
    /// Client +0x84.
    pub unk_84: u8,
}

impl Message for InstanceDetails {
    const NAME: &'static str = "InstanceDetails";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.astring(&self.host)
            .u16(self.port)
            .u32(self.unk_30)
            .u16(self.area_hash)
            .u32(self.unk_34)
            .bytes(&self.keys)
            .bytes(&self.unk_7c)
            .u8(self.unk_84);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            host: r.astring()?,
            port: r.u16()?,
            unk_30: r.u32()?,
            area_hash: r.u16()?,
            unk_34: r.u32()?,
            keys: r.array()?,
            unk_7c: r.array()?,
            unk_84: r.u8()?,
        })
    }
}

#[cfg(test)]
mod play_tests {
    use super::*;

    #[test]
    fn play_character_matches_capture() {
        // Captured 2026-10-12: index 0, account 1.
        let bytes = [0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 1];
        let mut r = Reader::new(&bytes);
        let p = PlayCharacter::decode(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(p.selections, vec![(0, 1)]);
    }
}
