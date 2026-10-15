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
    /// Client +0x38: the character's league (official servers send it).
    pub league: String,
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
    /// Client +0x70: `u16` count + big-endian `u32`s (PoE1 sent a preload list here).
    pub list_70: Vec<u32>,
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
            .wstring(&self.league)
            .u32(self.seed)
            .bytes(&self.bytes_a)
            .bytes(&self.flags)
            .bytes(&self.bytes_b)
            .u32(self.unk_24);
        w.u16(self.list_70.len() as u16);
        for v in &self.list_70 {
            w.u32(*v);
        }
        // Vectors, all empty: u16-count u8s (+0x88), u8-count u8s (+0x58), u16-count u16s
        // (+0xe8), u8-count raw 8-byte items (+0x100), u16-count u16s (+0x118).
        w.u16(0).u8(0).u16(0).u8(0).u16(0);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let m = Self {
            unk_10: r.u8()?,
            instance_id: r.u32()?,
            area_hash: r.u16()?,
            league: r.wstring()?,
            seed: r.u32()?,
            bytes_a: r.array()?,
            flags: r.array()?,
            bytes_b: r.array()?,
            unk_24: r.u32()?,
            list_70: Vec::new(),
        };
        if m.flags != [0; 3] {
            return Err(DecodeError::Invalid {
                offset: r.position(),
                what: "AreaInfo optional blocks",
            });
        }
        let mut m = m;
        let n = r.u16()? as usize;
        for _ in 0..n {
            m.list_70.push(r.u32()?);
        }
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
    fn hnc_bit_order() {
        // Stream bit 0 lands on value bit 9; stream bit 23 on value bit 0.
        assert_eq!(hnc_bits([1, 0, 0, 0]), 1 << 9);
        assert_eq!(hnc_bits([0, 0, 0x80, 0]), 1 << 0);
        assert_eq!(hnc_bits([0, 0, 0, 0x80]), 1 << 8);
        assert_eq!(hnc_bits([0, 0, 0x40, 0]), 1 << 31);
    }

    fn decode_all<M: Message>(bytes: &[u8]) -> M {
        let mut r = Reader::new(bytes);
        let m = M::decode(&mut r).unwrap();
        assert!(r.is_empty(), "trailing bytes");
        m
    }

    #[test]
    fn move_direction_matches_capture() {
        // Captured 2026-10-14 (bodies after the 0x0063 opcode): W, then release; D; S.
        let w: MoveDirection = decode_all(&[0x00, 0x80, 0x7f, 0x80, 0x7f, 0, 0, 0]);
        assert_eq!((w.x, w.y), (127, 127));
        let stop: MoveDirection = decode_all(&[0, 0, 0, 0, 0, 0]);
        assert_eq!((stop.x, stop.y), (0, 0));
        let d: MoveDirection = decode_all(&[0x00, 0x80, 0xb4, 0x00, 0, 0, 0]);
        assert_eq!((d.x, d.y), (180, 0));
        let s: MoveDirection = decode_all(&[0x00, 0xbf, 0x81, 0xbf, 0x81, 0, 0, 0]);
        assert_eq!((s.x, s.y), (-127, -127));
    }

    #[test]
    fn use_skill_matches_capture() {
        // Click-to-move (Move skill), captured 2026-10-14 after the 0x01a5 opcode.
        let u: UseSkill = decode_all(&[
            0x00, 0x20, 0x00, 0x29, 0x09, 0x04, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0x2e, 0xff,
            0xff, 0xff, 0xd3,
        ]);
        assert_eq!(u.skill, 0x2000_2909);
        assert_eq!((u.flags, u.unk_13, u.target_entity), (0x0400, 0xff, None));
        assert_eq!((u.x, u.y), (46, -45));
        let r: SkillReleased = decode_all(&[0x00, 0x20, 0x00, 0x29, 0x09, 0x00]);
        assert_eq!(r.skill, 0x2000_2909);
    }

    #[test]
    fn movement_messages_round_trip() {
        let start = StartAction {
            header: SimulationHeader {
                entity_id: 1,
                ..Default::default()
            },
            flags: 0,
            target_entity: Some(7),
            grid_x: 285,
            grid_y: 1140,
            target_x: 331,
            target_y: -5,
            skill: 1,
            action_id: 3,
            unk_3c: 0xff,
        };
        let mut w = Writer::new();
        start.encode(&mut w);
        assert_eq!(decode_all::<StartAction>(w.as_slice()), start);
        let update = MoveDirectionUpdate {
            header: start.header,
            dir_x: -127,
            dir_y: 127,
            grid_x: 285,
            grid_y: 1140,
            unk_30: 0xf1,
        };
        let mut w = Writer::new();
        update.encode(&mut w);
        assert_eq!(decode_all::<MoveDirectionUpdate>(w.as_slice()), update);
    }

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

/// Per-character state block of [`InitialState`] (client entry size 0x70). Consumed by
/// `0x1420edaa0` for the local player whose [`PlayerState`] points at it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CharacterState {
    /// The account's stash tabs (copied to local player +0x3a90; official new accounts: four
    /// tabs named "1".."4").
    pub stash_tabs: Vec<StashTab>,
    /// A second tab list (local player +0x3ac0, sorted by `0x1420ec230`), presumably the guild
    /// stash; empty officially.
    pub guild_tabs: Vec<StashTab>,
    /// (u32, u32) pairs at +0x30.
    pub pairs: Vec<(u32, u32)>,
    /// Wide string at +0x48.
    pub text: String,
    /// Flag bytes at +0x68..+0x6a, copied to the local player by `0x1420edaa0`: byte 0 bits
    /// 0..5 -> +0x3eca..+0x3ed0, bits 6/7 and byte 1 bit 0 -> account mode +0x3ec4; byte 1
    /// bit 1 -> +0x3ed1, bits 2..4 -> +0x3df8..+0x3dfa, bit 5 -> +0x3e32, bit 6 -> +0x3edc
    /// ([`CHARACTER_FULL_ACCESS`]), bit 7 -> +0x3edd; byte 2 bit 0 -> +0x3ede.
    pub flags: [u8; 3],
}

/// A stash tab record (reader `0x141ce2e20`, client entry 0x68 bytes). Optional fields
/// follow when `flags` bit 0x20 (8 bytes, +0x00), 0x10 (`u8`, +0x3c) or 0x40 (4 bytes, +0x3f)
/// is set; those are not supported (keep the bits clear).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StashTab {
    pub unk_28: u32,
    pub unk_2c: u32,
    /// Tab name (+0x08).
    pub name: String,
    /// Byte string at +0x48 (empty officially).
    pub text_48: String,
    /// Tab index (+0x38).
    pub index: u16,
    /// +0x3d.
    pub flags: u8,
    pub unk_3e: u8,
    /// +0x3a (`0xffff` officially: no parent?).
    pub unk_3a: u16,
    /// +0x30, +0x31, +0x32 (a colour?), +0x34.
    pub bytes: [u8; 4],
}

impl StashTab {
    /// A plain tab as official servers send them for a new account.
    pub fn plain(index: u16, name: &str) -> Self {
        Self {
            name: name.into(),
            index,
            unk_3a: 0xffff,
            ..Default::default()
        }
    }

    fn encode(&self, w: &mut Writer) {
        assert_eq!(
            self.flags & 0x70,
            0,
            "optional stash tab fields are not implemented"
        );
        w.u32(self.unk_28)
            .u32(self.unk_2c)
            .wstring(&self.name)
            .astring(&self.text_48)
            .u16(self.index)
            .u8(self.flags)
            .u8(self.unk_3e)
            .u16(self.unk_3a)
            .bytes(&self.bytes);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let tab = Self {
            unk_28: r.u32()?,
            unk_2c: r.u32()?,
            name: r.wstring()?,
            text_48: r.astring()?,
            index: r.u16()?,
            flags: r.u8()?,
            unk_3e: r.u8()?,
            unk_3a: r.u16()?,
            bytes: r.array()?,
        };
        if tab.flags & 0x70 != 0 {
            return Err(DecodeError::Invalid {
                offset: r.position(),
                what: "stash tab optional fields not supported",
            });
        }
        Ok(tab)
    }
}

/// [`CharacterState::flags`] byte 1 bit 6: full game access (the escape menu's "Get Full
/// Access" button is only added when it is clear, `0x1402c4980`).
pub const CHARACTER_FULL_ACCESS: u8 = 0x40;

/// Per-player entry of [`InitialState`] (client entry size 0x68), matched by `player_id`
/// against the id assigned with `0x24`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerState {
    pub player_id: u32,
    /// Low 6 bits: index into [`InitialState::characters`]; bits 6/7 -> +7/+6.
    pub character: u8,
    /// Bits 0..2 -> +8..+10.
    pub flags: u8,
    pub unk_5: u8,
    /// Per weapon set (main, swap, third): row index into
    /// `CharacterShapeshiftBasicSkills.dat` (`0xff`: none, which crashes the weapon-slot UI).
    /// Stored at local player +0x7980.
    pub rows: [u8; 3],
    /// u16 list at +0x40 (the client keeps the first three).
    pub unk_40: Vec<u16>,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            player_id: 0,
            character: 0,
            flags: 0,
            unk_5: 0,
            rows: [0; 3],
            unk_40: Vec::new(),
        }
    }
}

/// Initial state (deserializer `0x142158130`, handler `0x142122eed`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitialState {
    pub unk_10: u8,
    /// Server frame length in ms (world vfunc +0x178 -> world +0x11c). Every frame marker
    /// (`0x2f4`) records it, and the client advances its simulation by that much per frame:
    /// 0 freezes animations, fades and timers (models never appear).
    pub frame_ms: u32,
    /// Bits 0..2 -> client +0x1fc..+0x1fe.
    pub flags: u8,
    pub characters: Vec<CharacterState>,
    pub players: Vec<PlayerState>,
}

impl Message for InitialState {
    const NAME: &'static str = "InitialState";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk_10)
            .varuint(self.frame_ms)
            .u8(self.flags)
            .u8(self.characters.len() as u8);
        for c in &self.characters {
            for list in [&c.stash_tabs, &c.guild_tabs] {
                w.u16(list.len() as u16);
                for tab in list {
                    tab.encode(w);
                }
            }
            w.u8(c.pairs.len() as u8);
            for (a, b) in &c.pairs {
                w.u32(*a).u32(*b);
            }
            w.wstring(&c.text).bytes(&c.flags);
        }
        w.u8(self.players.len() as u8);
        for p in &self.players {
            w.u32(p.player_id)
                .u8(p.character)
                .u8(p.flags)
                .u8(p.unk_5)
                .bytes(&p.rows)
                .u16(p.unk_40.len() as u16);
            for v in &p.unk_40 {
                w.u16(*v);
            }
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let mut m = Self {
            unk_10: r.u8()?,
            frame_ms: r.varuint()?,
            flags: r.u8()?,
            ..Default::default()
        };
        let tabs = |r: &mut Reader<'_>| -> Result<Vec<StashTab>, DecodeError> {
            (0..r.u16()?).map(|_| StashTab::decode(r)).collect()
        };
        for _ in 0..r.u8()? {
            let stash_tabs = tabs(r)?;
            let guild_tabs = tabs(r)?;
            let pairs = (0..r.u8()?)
                .map(|_| Ok((r.u32()?, r.u32()?)))
                .collect::<Result<_, DecodeError>>()?;
            m.characters.push(CharacterState {
                stash_tabs,
                guild_tabs,
                pairs,
                text: r.wstring()?,
                flags: r.array()?,
                ..Default::default()
            });
        }
        for _ in 0..r.u8()? {
            let mut p = PlayerState {
                player_id: r.u32()?,
                character: r.u8()?,
                flags: r.u8()?,
                unk_5: r.u8()?,
                rows: r.array()?,
                ..Default::default()
            };
            for _ in 0..r.u16()? {
                p.unk_40.push(r.u16()?);
            }
            m.players.push(p);
        }
        Ok(m)
    }
}

/// Assigns the entity a local player controls (handler sets local player +0x184; the entity
/// with that id becomes the controlled character when it is added).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerEntity {
    pub unk: u8,
    /// Local player index (from [`LocalPlayers`]).
    pub local_index: u8,
    pub entity_id: u32,
}

impl Message for PlayerEntity {
    const NAME: &'static str = "PlayerEntity";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk).u8(self.local_index).u32(self.entity_id);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk: r.u8()?,
            local_index: r.u8()?,
            entity_id: r.u32()?,
        })
    }
}

/// Header shared by simulation messages (`0x141f4b680`). The frame processor orders a
/// frame's messages by the last three fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SimulationHeader {
    /// Entity the message applies to.
    pub entity_id: u32,
    pub unk_14: u32,
    pub unk_18: u8,
    pub unk_1a: u16,
}

impl SimulationHeader {
    fn encode(&self, w: &mut Writer) {
        w.u32(self.entity_id)
            .u32(self.unk_14)
            .u8(self.unk_18)
            .u16(self.unk_1a);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            entity_id: r.u32()?,
            unk_14: r.u32()?,
            unk_18: r.u8()?,
            unk_1a: r.u16()?,
        })
    }
}

/// Sets the server frame length in ms (`u8`), applied immediately (not queued) through world
/// vfunc +0x178 -> world +0x11c, like `InitialState::frame_ms`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SimulationFrameValue {
    pub value: u8,
}

impl Message for SimulationFrameValue {
    const NAME: &'static str = "SimulationFrameValue";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.value);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self { value: r.u8()? })
    }
}

/// S→C ping reply to the client's frame report (`0x1c`): `u16, u16, u32` (reader
/// `0x141f48c60`). The handler (`0x142122115`) derives the latency display from the first
/// `u16`. All 32 bits of the `u32` carry the HNC side channel: bits 9..31 then 0..8, packed
/// least significant bit first into bytes that the client decrypts with its HNC cipher. An idle
/// channel decrypts to zero bytes, so the server sends raw keystream ([`hnc_bits`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PingReply {
    pub unk_18: u16,
    pub unk_1a: u16,
    pub hnc: u32,
}

impl Message for PingReply {
    const NAME: &'static str = "PingReply";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u16(self.unk_18).u16(self.unk_1a).u32(self.hnc);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk_18: r.u16()?,
            unk_1a: r.u16()?,
            hnc: r.u32()?,
        })
    }
}

/// C→S frame report, about every 100 ms (sender `0x1421211e4`, serializer `0x141f486f0`):
/// frames processed since the last report (world +0x680, capped at 255), and 32 HNC bits
/// (value bits 14..31, then 0..13).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameReport {
    pub frames: u8,
    pub hnc: u32,
}

impl Message for FrameReport {
    const NAME: &'static str = "FrameReport";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.frames).u32(self.hnc);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            frames: r.u8()?,
            hnc: r.u32()?,
        })
    }
}

/// C→S request to join a chat channel, sent for global and trade chat after entering an
/// area: `u8, u16 channel (0xffff = any), u8 kind (0 global, 1 trade), u8, u8`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChatJoinRequest {
    pub unk_0: u8,
    pub channel: u16,
    pub kind: u8,
    pub unk_4: u8,
    pub unk_5: u8,
}

impl Message for ChatJoinRequest {
    const NAME: &'static str = "ChatJoinRequest";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk_0)
            .u16(self.channel)
            .u8(self.kind)
            .u8(self.unk_4)
            .u8(self.unk_5);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk_0: r.u8()?,
            channel: r.u16()?,
            kind: r.u8()?,
            unk_4: r.u8()?,
            unk_5: r.u8()?,
        })
    }
}

/// S→C chat channel joined (constructor `0x14215cb50`, reader `0x14215cc30`, handler
/// `0x142124bca`): `u8 (+0x10), u16 channel (+0x1a), u8 trade (+0x18), u8 (+0x1c)`. Stores the
/// channel in the session (trade +0x2738, global +0x2740) and ends the pending join; without
/// it the client reports "Timeout while joining … chat".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChatJoined {
    pub unk_10: u8,
    pub channel: u16,
    pub trade: bool,
    pub unk_1c: u8,
}

impl Message for ChatJoined {
    const NAME: &'static str = "ChatJoined";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk_10)
            .u16(self.channel)
            .u8(self.trade as u8)
            .u8(self.unk_1c);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk_10: r.u8()?,
            channel: r.u16()?,
            trade: r.u8()? != 0,
            unk_1c: r.u8()?,
        })
    }
}

/// Packs 4 HNC stream bytes into the `u32` of [`PingReply`] (client bit order: value bits
/// 9..31, then 0..8; each byte filled from its least significant bit).
pub fn hnc_bits(bytes: [u8; 4]) -> u32 {
    let stream = u32::from_le_bytes(bytes);
    // Stream bit k (k < 23) -> value bit 9 + k; stream bit 23 + k (k < 9) -> value bit k.
    ((stream & 0x7f_ffff) << 9) | (stream >> 23)
}

/// S→C reply to the client's exit request (`0x173`, "Exit to character selection"):
/// `u8, u32, u64` (constructor `0x14215df40`, reader `0x14215e010`). The handler broadcasts
/// (`u32`, `u64`) to listeners (`0x1402f5e10`) and sends the exit telemetry (`0x355`). The
/// client then reconnects to the login server and sends them back in login `0x08`
/// ResumeSession (with zeros it sends nothing and waits): `unk_20` = account id,
/// `unk_18` = resume token.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExitSession {
    pub unk_10: u8,
    pub unk_20: u32,
    pub unk_18: u64,
}

impl Message for ExitSession {
    const NAME: &'static str = "ExitSession";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        // The u64 is read raw (no byte swap).
        w.u8(self.unk_10)
            .u32(self.unk_20)
            .bytes(&self.unk_18.to_le_bytes());
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk_10: r.u8()?,
            unk_20: r.u32()?,
            unk_18: u64::from_le_bytes(r.array()?),
        })
    }
}

/// S→C simulation message: world pause state (reader `0x1421842a0`, applied by
/// `0x142134240` → world vfunc +0x200 `0x14168aaf0`): bit 0 -> world +0x868 (paused: pausable
/// entities update with dt 0), bit 1 -> +0x869; with both set the client also applies its
/// "Pause" audio state. Reply to the client's [`PauseRequest`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetPause {
    pub header: SimulationHeader,
    pub paused: bool,
    pub flag_869: bool,
}

impl Message for SetPause {
    const NAME: &'static str = "SetPause";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.u8(self.paused as u8 | (self.flag_869 as u8) << 1);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let header = SimulationHeader::decode(r)?;
        let b = r.u8()?;
        Ok(Self {
            header,
            paused: b & 1 != 0,
            flag_869: b & 2 != 0,
        })
    }
}

/// C→S: the client asks to pause (escape menu opened, solo game) or resume: `u8, u8 paused`
/// (class `0x1434e50f8`, fields +0x10 and +0x18). Sent as `00 00` after entering an area.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PauseRequest {
    pub unk: u8,
    pub paused: bool,
}

impl Message for PauseRequest {
    const NAME: &'static str = "PauseRequest";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk).u8(self.paused as u8);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk: r.u8()?,
            paused: r.u8()? != 0,
        })
    }
}

/// C→S `0x1b`: the client leaves the game ("Exit to log in screen" or quitting; logout
/// routine `0x14211f540` with 0 or 2): `u8, u8` (both 0 so far, same class as PauseRequest).
/// Sent after [`ExitReport`], then the client closes the connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LeaveGame {
    pub unk_0: u8,
    pub unk_1: u8,
}

impl Message for LeaveGame {
    const NAME: &'static str = "LeaveGame";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk_0).u8(self.unk_1);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk_0: r.u8()?,
            unk_1: r.u8()?,
        })
    }
}

/// C→S `0x355`: client statistics sent when it leaves an instance (`0x14211fad0`;
/// serializer `0x141f479b0`): `u8, u16 length, length bytes` (a little-endian `u16` 6 and
/// a 0x360-byte block, not decoded).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExitReport {
    pub unk: u8,
    pub data: Vec<u8>,
}

impl Message for ExitReport {
    const NAME: &'static str = "ExitReport";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk).u16(self.data.len() as u16).bytes(&self.data);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let unk = r.u8()?;
        let len = r.u16()? as usize;
        Ok(Self {
            unk,
            data: r.bytes(len)?.to_vec(),
        })
    }
}

/// C→S: WASD movement input, sent when the pressed direction changes:
/// `u8, varint x, varint y, u8[3]`. The vector is in grid axes with a length of about 180
/// (W = (127, 127), D = (180, 0), S = (-127, -127)...); (0, 0) means stop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MoveDirection {
    pub unk_0: u8,
    pub x: i32,
    pub y: i32,
    pub unk_tail: [u8; 3],
}

impl Message for MoveDirection {
    const NAME: &'static str = "MoveDirection";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk_0)
            .varint(self.x)
            .varint(self.y)
            .bytes(&self.unk_tail);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let unk_0 = r.u8()?;
        let x = r.varint()?;
        let y = r.varint()?;
        let tail = r.bytes(3)?;
        Ok(Self {
            unk_0,
            x,
            y,
            unk_tail: [tail[0], tail[1], tail[2]],
        })
    }
}

/// S→C simulation message: an entity's WASD movement (reader `0x141f6b0e0`, applied by the
/// Actor component `0x142149640`): header, `varint dir x, varint dir y, varint grid x,
/// varint grid y, u8`. With a non-zero direction the client first moves the entity to the
/// given grid cell if it is elsewhere (snaps, or blends when close), then moves it in that
/// direction every frame until a zero direction arrives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MoveDirectionUpdate {
    pub header: SimulationHeader,
    pub dir_x: i32,
    pub dir_y: i32,
    pub grid_x: i32,
    pub grid_y: i32,
    /// Official servers send `0xf1` in nearly all of these (0 in a few answers to
    /// single-axis input); the client's handler does not read it (`0x142149640`).
    pub unk_30: u8,
}

impl Message for MoveDirectionUpdate {
    const NAME: &'static str = "MoveDirectionUpdate";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.varint(self.dir_x)
            .varint(self.dir_y)
            .varint(self.grid_x)
            .varint(self.grid_y)
            .u8(self.unk_30);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            header: SimulationHeader::decode(r)?,
            dir_x: r.varint()?,
            dir_y: r.varint()?,
            grid_x: r.varint()?,
            grid_y: r.varint()?,
            unk_30: r.u8()?,
        })
    }
}

/// C→S: use a skill (class vtable `0x1434e5b40`, built by `0x14210f920`, body `0x141ecdd00`):
/// `u8 local player, u32 skill id, u16 flags, u8, u8 target flags` (bit 3: a target entity
/// follows), `[u32 target entity]`, `i32 x, i32 y`. Without a target entity the location is
/// relative to the player's grid cell. Left click in click-to-move mode uses the Move skill
/// (`0x20002909`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UseSkill {
    pub player: u8,
    pub skill: u32,
    pub flags: u16,
    pub unk_13: u8,
    pub target_flags: u8,
    pub target_entity: Option<u32>,
    pub x: i32,
    pub y: i32,
}

impl Message for UseSkill {
    const NAME: &'static str = "UseSkill";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.player)
            .u32(self.skill)
            .u16(self.flags)
            .u8(self.unk_13);
        match self.target_entity {
            Some(id) => w.u8(self.target_flags | 8).u32(id),
            None => w.u8(self.target_flags & !8),
        };
        w.i32(self.x).i32(self.y);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let player = r.u8()?;
        let skill = r.u32()?;
        let flags = r.u16()?;
        let unk_13 = r.u8()?;
        let target_flags = r.u8()?;
        let target_entity = if target_flags & 8 != 0 {
            Some(r.u32()?)
        } else {
            None
        };
        Ok(Self {
            player,
            skill,
            flags,
            unk_13,
            target_flags,
            target_entity,
            x: r.i32()?,
            y: r.i32()?,
        })
    }
}

/// C→S: the skill's input was released (e.g. the mouse button after a click-to-move):
/// `u8 local player, u32 skill id, u8`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkillReleased {
    pub player: u8,
    pub skill: u32,
    pub unk: u8,
}

impl Message for SkillReleased {
    const NAME: &'static str = "SkillReleased";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.player).u32(self.skill).u8(self.unk);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            player: r.u8()?,
            skill: r.u32()?,
            unk: r.u8()?,
        })
    }
}

/// S→C simulation message: an entity starts an action (reader `0x141f696b0`, applied by the
/// Actor component `0x142149640`): header, `u16 flags` (bit 7: a `u32` target entity follows),
/// `varint x, varint y` (the entity's grid cell when it starts), `varint target x, varint
/// target y` (grid), `u32 skill id, u16 action id, u8`. The client creates the action from the
/// actor's skill with that id (Move: walks to the target, DodgeRoll: rolls towards it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartAction {
    pub header: SimulationHeader,
    pub flags: u16,
    pub target_entity: Option<u32>,
    pub grid_x: i32,
    pub grid_y: i32,
    pub target_x: i32,
    pub target_y: i32,
    pub skill: u32,
    pub action_id: u16,
    pub unk_3c: u8,
}

impl Message for StartAction {
    const NAME: &'static str = "StartAction";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        match self.target_entity {
            Some(id) => w.u16(self.flags | 0x80).u32(id),
            None => w.u16(self.flags & !0x80),
        };
        w.varint(self.grid_x)
            .varint(self.grid_y)
            .varint(self.target_x)
            .varint(self.target_y)
            .u32(self.skill)
            .u16(self.action_id)
            .u8(self.unk_3c);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let header = SimulationHeader::decode(r)?;
        let flags = r.u16()?;
        let target_entity = if flags & 0x80 != 0 {
            Some(r.u32()?)
        } else {
            None
        };
        Ok(Self {
            header,
            flags: flags & !0x80,
            target_entity,
            grid_x: r.varint()?,
            grid_y: r.varint()?,
            target_x: r.varint()?,
            target_y: r.varint()?,
            skill: r.u32()?,
            action_id: r.u16()?,
            unk_3c: r.u8()?,
        })
    }
}

/// C→S: new target for a running skill, relative to the player's cell (sender `0x14210fbb0`):
/// `u8 local player, i32 x, i32 y, u32 skill id`. Sent about every 70 ms while a skill's input
/// is held, e.g. a dodge roll or sprint steering towards the cursor. In lockstep the client
/// does not apply it itself; the server answers with [`UpdateActionTarget`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateSkillTarget {
    pub player: u8,
    pub x: i32,
    pub y: i32,
    pub skill: u32,
}

impl Message for UpdateSkillTarget {
    const NAME: &'static str = "UpdateSkillTarget";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.player).i32(self.x).i32(self.y).u32(self.skill);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            player: r.u8()?,
            x: r.i32()?,
            y: r.i32()?,
            skill: r.u32()?,
        })
    }
}

/// C→S: new target entity for a running skill: `u8 local player, u32 entity, u32 skill id`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateSkillTargetEntity {
    pub player: u8,
    pub entity: u32,
    pub skill: u32,
}

impl Message for UpdateSkillTargetEntity {
    const NAME: &'static str = "UpdateSkillTargetEntity";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.player).u32(self.entity).u32(self.skill);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            player: r.u8()?,
            entity: r.u32()?,
            skill: r.u32()?,
        })
    }
}

/// S→C simulation message: new target location of an entity's current action (reader
/// `0x141f69c60`, Actor handler case `0x1cb`): header, `u16 action id, i32 x, i32 y`
/// (absolute grid cell). Applied when the id matches the running action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateActionTarget {
    pub header: SimulationHeader,
    pub action_id: u16,
    pub x: i32,
    pub y: i32,
}

impl Message for UpdateActionTarget {
    const NAME: &'static str = "UpdateActionTarget";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.u16(self.action_id).i32(self.x).i32(self.y);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            header: SimulationHeader::decode(r)?,
            action_id: r.u16()?,
            x: r.i32()?,
            y: r.i32()?,
        })
    }
}

/// S→C simulation message for the Positioned component (`0x1df`, reader `0x141f6af00`,
/// handler `0x141dda640`): header, `f32 angle, u8 locked`. The client sets the entity's facing
/// (Positioned +0x44c, radians: `atan2(dy, dx) + π/2` of a grid direction) unless it is locked,
/// then sets or clears the lock (+0x1e5 bit 1); while locked, movement does not turn it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SetOrientation {
    pub header: SimulationHeader,
    pub angle: f32,
    pub locked: bool,
}

impl Message for SetOrientation {
    const NAME: &'static str = "SetOrientation";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.u32(self.angle.to_bits()).u8(self.locked as u8);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            header: SimulationHeader::decode(r)?,
            angle: f32::from_bits(r.u32()?),
            locked: r.u8()? != 0,
        })
    }
}

/// C→S: the sprint/dodge key (Space) went down or up (sender `0x14016e780`):
/// `u8 local player, u8 held`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SprintInput {
    pub player: u8,
    pub held: bool,
}

impl Message for SprintInput {
    const NAME: &'static str = "SprintInput";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.player).u8(self.held as u8);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            player: r.u8()?,
            held: r.u8()? != 0,
        })
    }
}

/// S→C simulation message: whether the entity's sprint key is held (reader `0x141dcb170`,
/// Actor vfunc +0x3d8 `0x141e5c070`: Actor +0x2a2 bit 5). A dodge roll that ends while it is
/// set turns into a sprint (`0x141b76c20`); clearing it ends the sprint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SprintState {
    pub header: SimulationHeader,
    pub held: bool,
}

impl Message for SprintState {
    const NAME: &'static str = "SprintState";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.u8(self.held as u8);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            header: SimulationHeader::decode(r)?,
            held: r.u8()? != 0,
        })
    }
}

/// S→C simulation message: an entity's stats change (reader `0x141f6dce0`, Stats component
/// handler `0x142181f20` case `0x1d3`): header, `u8 set` (0 = the entity's own stats), varuint
/// count, then (varuint stat key, varint value) pairs. Keys are `Stats.dat` row + 1.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatsUpdate {
    pub header: SimulationHeader,
    pub set: u8,
    pub stats: Vec<(u32, i32)>,
}

impl Message for StatsUpdate {
    const NAME: &'static str = "StatsUpdate";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.u8(self.set).varuint(self.stats.len() as u32);
        for &(key, value) in &self.stats {
            w.varuint(key).varint(value);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let header = SimulationHeader::decode(r)?;
        let set = r.u8()?;
        let n = r.varuint()?;
        let mut stats = Vec::new();
        for _ in 0..n {
            stats.push((r.varuint()?, r.varint()?));
        }
        Ok(Self { header, set, stats })
    }
}

/// S→C simulation message: stats of one of an actor's skills change (reader `0x141f6aa80`,
/// Actor handler case `0x1d8`): header, `u32 skill id, u8 stat container` (0 = the skill's own
/// stats), varuint count, then (varuint stat key, varint value) pairs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillStatsUpdate {
    pub header: SimulationHeader,
    pub skill: u32,
    pub container: u8,
    pub stats: Vec<(u32, i32)>,
}

impl Message for SkillStatsUpdate {
    const NAME: &'static str = "SkillStatsUpdate";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.u32(self.skill)
            .u8(self.container)
            .varuint(self.stats.len() as u32);
        for &(key, value) in &self.stats {
            w.varuint(key).varint(value);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let header = SimulationHeader::decode(r)?;
        let skill = r.u32()?;
        let container = r.u8()?;
        let n = r.varuint()?;
        let mut stats = Vec::new();
        for _ in 0..n {
            stats.push((r.varuint()?, r.varint()?));
        }
        Ok(Self {
            header,
            skill,
            container,
            stats,
        })
    }
}

/// Ends a simulation frame (no body).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SimulationTick;

impl Message for SimulationTick {
    const NAME: &'static str = "SimulationTick";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, _: &mut Writer) {}
    fn decode(_: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self)
    }
}

/// Spawns an entity: object type key (MurmurHash2 of its metadata path) and its initial
/// state blob (read by `0x141e71120`; layout in `Documentation/protocol/entities.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddObject {
    pub header: SimulationHeader,
    pub type_key: u32,
    pub blob: Vec<u8>,
}

impl Message for AddObject {
    const NAME: &'static str = "AddObject";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.u32(self.type_key)
            .varuint(self.blob.len() as u32)
            .bytes(&self.blob);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let header = SimulationHeader::decode(r)?;
        let type_key = r.u32()?;
        let len = r.varuint()? as usize;
        Ok(Self {
            header,
            type_key,
            blob: r.bytes(len)?.to_vec(),
        })
    }
}

/// An inventory (client class vtable `0x143573a28`; serializer `0x1425aa290`, reader
/// `0x1425aa550`). Items are not supported yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Inventory {
    /// +0x140 (provisional: `Inventories.InventoryIdKey`).
    pub id_key: u8,
    /// +0x144: `Inventories.dat` row index.
    pub slot: u8,
    pub width: u8,
    pub height: u8,
    /// Official servers send 0x00/0x01/0x03/0x04/0x60/0x61/0x65 here
    /// (`protocol/<version>/inventories.tsv`); none of these add bytes to the message.
    pub flags: u8,
}

impl Inventory {
    fn encode(&self, w: &mut Writer) {
        w.u8(self.id_key)
            .u8(self.slot)
            .u8(self.width)
            .u8(self.height)
            .u8(self.flags)
            // Item count.
            .varuint(0);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let inv = Self {
            id_key: r.u8()?,
            slot: r.u8()?,
            width: r.u8()?,
            height: r.u8()?,
            flags: r.u8()?,
        };
        if r.varuint()? != 0 {
            return Err(DecodeError::Invalid {
                offset: r.position(),
                what: "inventory items not supported",
            });
        }
        Ok(inv)
    }
}

/// Creates (or replaces) a server inventory for a local player (handler in the per-player
/// processor, registry insert `0x141cf1da0`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AddInventory {
    pub unk_10: u8,
    pub local_index: u8,
    /// Server inventory id (the registry key).
    pub inventory_id: u32,
    pub inventory: Inventory,
}

impl Message for AddInventory {
    const NAME: &'static str = "AddInventory";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.unk_10)
            .u8(self.local_index)
            .u32(self.inventory_id);
        self.inventory.encode(w);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unk_10: r.u8()?,
            local_index: r.u8()?,
            inventory_id: r.u32()?,
            inventory: Inventory::decode(r)?,
        })
    }
}

/// C→S `0x0062`: where the player aims (the cursor), reported by the client. `u8 local player`,
/// `u8 mode`, then by mode: 2 = a ground cell (`i32 x, i32 y`), 3 = an entity (`u32`), 0/1 =
/// nothing (seen once each around spawning and before a skill use). Official servers echo mode 2
/// as [`SetAimTarget`] one frame later, which turns the player toward the cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AimTarget {
    pub local_index: u8,
    pub mode: u8,
    pub cell: Option<(i32, i32)>,
    pub entity: Option<u32>,
}

impl Message for AimTarget {
    const NAME: &'static str = "AimTarget";
    const DIRECTION: Direction = Direction::Client;
    fn encode(&self, w: &mut Writer) {
        w.u8(self.local_index).u8(self.mode);
        if let Some((x, y)) = self.cell {
            w.i32(x).i32(y);
        } else if let Some(e) = self.entity {
            w.u32(e);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let local_index = r.u8()?;
        let mode = r.u8()?;
        let (cell, entity) = match mode {
            2 => (Some((r.i32()?, r.i32()?)), None),
            3 => (None, Some(r.u32()?)),
            _ => (None, None),
        };
        Ok(Self {
            local_index,
            mode,
            cell,
            entity,
        })
    }
}

/// S→C `0x01dc` (Actor message, handler `0x142149640` case `0x1dc`): turn the entity toward a
/// cell (`kind` 0) or an entity (`kind` 1, its id in `entity`). Header `(0, 0, 0)` on official
/// servers. Official servers send it for the player's [`AimTarget`] positions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetAimTarget {
    pub header: SimulationHeader,
    pub kind: u8,
    pub x: i32,
    pub y: i32,
}

impl Message for SetAimTarget {
    const NAME: &'static str = "SetAimTarget";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.u8(self.kind).i32(self.x).i32(self.y);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            header: SimulationHeader::decode(r)?,
            kind: r.u8()?,
            x: r.i32()?,
            y: r.i32()?,
        })
    }
}

/// S→C component delta for one entity (reader `0x141f6c7a2`): the header (official: entity,
/// `0xffffffff`, 0, 0), a varuint length and that many bytes, which are the component index
/// (the entity template's order) followed by the component's own delta (component vfunc
/// +0xb0). See [`crate::entity::PlayerDelta`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentDelta {
    pub header: SimulationHeader,
    pub component: u8,
    pub delta: Vec<u8>,
}

impl Message for ComponentDelta {
    const NAME: &'static str = "ComponentDelta";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
        w.varuint(1 + self.delta.len() as u32)
            .u8(self.component)
            .bytes(&self.delta);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let header = SimulationHeader::decode(r)?;
        let len = r.varuint()? as usize;
        let blob = r.bytes(len)?;
        let (&component, delta) = blob.split_first().ok_or(DecodeError::Incomplete {
            offset: r.position(),
            needed: 1,
        })?;
        Ok(Self {
            header,
            component,
            delta: delta.to_vec(),
        })
    }
}
