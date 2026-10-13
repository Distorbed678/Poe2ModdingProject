//! Instance channel messages (client 0.5.5e). See `Documentation/protocol/instance.md`.

use poe_net::{DecodeError, Reader, Writer};

use crate::{Direction, Message};

/// First message on a new instance connection (plaintext). Carries the hand-off ticket
/// from `InstanceDetails`. Fields other than the ticket are provisional.
#[derive(Debug, Clone)]
pub struct Hello {
    pub unk0: u32,
    pub unk1: u16,
    pub unk2: [u8; 12],
    /// Ticket from `InstanceDetails` (+0x30).
    pub ticket: u32,
    pub unk3: u32,
    pub unk4: u8,
}

impl Message for Hello {
    const NAME: &'static str = "Hello";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u32(self.unk0)
            .u16(self.unk1)
            .bytes(&self.unk2)
            .u32(self.ticket)
            .u32(self.unk3)
            .u8(self.unk4);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk0: r.u32()?,
            unk1: r.u16()?,
            unk2: r.array()?,
            ticket: r.u32()?,
            unk3: r.u32()?,
            unk4: r.u8()?,
        })
    }
}

/// Server reply to [`Hello`], sent in plaintext. On receipt the client installs the
/// Salsa20 pair built from the `InstanceDetails` key material (handler `0x14212203e`);
/// everything after it is encrypted in both directions.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnableEncryption {
    pub unk: u8,
}

impl Message for EnableEncryption {
    const NAME: &'static str = "EnableEncryption";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self { unk: r.u8()? })
    }
}

/// Area to load (serializer `0x14211c980`, handler in `0x142121f70` case 0x1e). Only the
/// fixed header and empty vectors are supported so far; the optional blocks gated by
/// `flags` are never sent (flags must stay 0 except where noted).
#[derive(Debug, Clone, Default)]
pub struct AreaInfo {
    /// Client +0x10
    pub unk_10: u8,
    /// "Client-Safe Instance ID" (client +0x18).
    pub instance_id: u32,
    /// `WorldAreas.HASH16` of the area (client +0x1c).
    pub area_hash: u16,
    /// Client +0x38 (wide string, meaning TBD).
    pub text_38: String,
    /// Terrain generation seed (client +0x20).
    pub seed: u32,
    /// Client +0x2a, +0x2c, +0x2b, +0x30 in wire order.
    pub bytes_a: [u8; 4],
    /// Client +0x2d..+0x2f: flags gating optional blocks (keep 0).
    pub flags: [u8; 3],
    /// Client +0x28, +0x29, +0x34 in wire order.
    pub bytes_b: [u8; 3],
    /// Client +0x24
    pub unk_24: u32,
}

impl Message for AreaInfo {
    const NAME: &'static str = "AreaInfo";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        assert_eq!(
            self.flags, [0; 3],
            "optional AreaInfo blocks are not implemented"
        );
        w.u8(self.unk_10)
            .u32(self.instance_id)
            .u16(self.area_hash)
            .wstring(&self.text_38)
            .u32(self.seed)
            .bytes(&self.bytes_a)
            .bytes(&self.flags)
            .bytes(&self.bytes_b)
            .u32(self.unk_24);
        // Vectors, all empty: u16-count u32s (+0x70), u16-count u8s (+0x88), u8-count u8s
        // (+0x58), u16-count u16s (+0xe8), u8-count raw 8-byte items (+0x100), u16-count
        // u16s (+0x118).
        w.u16(0).u16(0).u8(0).u16(0).u8(0).u16(0);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let m = Self {
            unk_10: r.u8()?,
            instance_id: r.u32()?,
            area_hash: r.u16()?,
            text_38: r.wstring()?,
            seed: r.u32()?,
            bytes_a: r.array()?,
            flags: r.array()?,
            bytes_b: r.array()?,
            unk_24: r.u32()?,
        };
        if m.flags != [0; 3] {
            return Err(DecodeError::Invalid {
                offset: r.position(),
                what: "AreaInfo optional blocks",
            });
        }
        let n = r.u16()? as usize;
        r.bytes(n * 4)?;
        let n = r.u16()? as usize;
        r.bytes(n)?;
        let n = r.u8()? as usize;
        r.bytes(n)?;
        let n = r.u16()? as usize;
        r.bytes(n * 2)?;
        let n = r.u8()? as usize;
        r.bytes(n * 8)?;
        let n = r.u16()? as usize;
        r.bytes(n * 2)?;
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_matches_capture() {
        // Captured 2026-10-13 (body after the 0x000f opcode).
        let bytes = [
            0, 0, 0, 0, 0x00, 0x0e, 0x94, 0x7c, 0xfc, 0x3f, 0x71, 0xf1, 0xac, 0x02, 0x6a, 0xcd,
            0x77, 0xd8, 0xf6, 0xa9, 0x48, 0xf5, 0, 0, 0, 0, 0x04,
        ];
        let mut r = Reader::new(&bytes);
        let h = Hello::decode(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(h.ticket, 0xf6a9_48f5);
    }
}

/// Client report after generating the area's terrain (provisional layout: u8 + two u32
/// hashes, probably tiles and doodads; compared against the server's own generation).
#[derive(Debug, Clone, Copy)]
pub struct TerrainHashes {
    pub unk: u8,
    pub hash_a: u32,
    pub hash_b: u32,
}

impl Message for TerrainHashes {
    const NAME: &'static str = "TerrainHashes";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk).u32(self.hash_a).u32(self.hash_b);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk: r.u8()?,
            hash_a: r.u32()?,
            hash_b: r.u32()?,
        })
    }
}

/// Local players of this connection with their character names (serializer `0x1421578e0`;
/// provisional meaning).
#[derive(Debug, Clone, Default)]
pub struct LocalPlayers {
    pub unk: u8,
    pub players: Vec<(u8, String)>,
}

impl Message for LocalPlayers {
    const NAME: &'static str = "LocalPlayers";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk).u8(self.players.len() as u8);
        for (index, name) in &self.players {
            w.u8(*index).wstring(name);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let unk = r.u8()?;
        let n = r.u8()?;
        let players = (0..n)
            .map(|_| Ok((r.u8()?, r.wstring()?)))
            .collect::<Result<_, DecodeError>>()?;
        Ok(Self { unk, players })
    }
}

/// Initial state (deserializer `0x142158130`); only the empty form is supported so far.
#[derive(Debug, Clone, Copy, Default)]
pub struct InitialState {
    pub unk_10: u8,
    pub unk_1f8: u32,
    /// Bits 0..2 -> client +0x1fc..+0x1fe.
    pub flags: u8,
}

impl Message for InitialState {
    const NAME: &'static str = "InitialState";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        // Two empty lists (0x70-byte and 0x68-byte entries) follow the header.
        w.u8(self.unk_10)
            .varuint(self.unk_1f8)
            .u8(self.flags)
            .u8(0)
            .u8(0);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let m = Self {
            unk_10: r.u8()?,
            unk_1f8: r.varuint()?,
            flags: r.u8()?,
        };
        if r.u8()? != 0 || r.u8()? != 0 {
            return Err(DecodeError::Invalid {
                offset: r.position(),
                what: "InitialState lists not supported",
            });
        }
        Ok(m)
    }
}
