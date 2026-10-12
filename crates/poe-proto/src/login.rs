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
    pub account_name: String,
    /// Opaque 32 bytes stored with the logged-in account.
    pub session_token: [u8; 32],
    /// Two bytes of account flags; the client splits them into individual bits.
    pub account_flags: [u8; 2],
    /// Free-form string passed to the client's message/URL handling (empty = none).
    pub message: String,
}

impl Message for AuthenticationResult {
    const NAME: &'static str = "AuthenticationResult";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u32(self.account_id)
            .u16(self.result)
            .wstring(&self.account_name)
            .bytes(&self.session_token)
            .bytes(&self.account_flags)
            .astring(&self.message);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            account_id: r.u32()?,
            result: r.u16()?,
            account_name: r.wstring()?,
            session_token: r.array()?,
            account_flags: r.array()?,
            message: r.astring()?,
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
    /// +0x70
    pub unk_70: u32,
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
        w.u32(self.unk_70).u8(self.characters.len() as u8);
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
        let unk_70 = r.u32()?;
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
            unk_70,
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
