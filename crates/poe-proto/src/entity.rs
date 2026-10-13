//! Entity initial-state blobs: the payload of [`crate::instance::AddObject`].
//!
//! The client reads a blob with `0x141e71120`:
//! `u8 flags, u8 n (records, unsupported), components..., u8 m (keyed entries, unsupported)`.
//! Each serialized component of the object type (in template order) reads its own section
//! with its vtable +0x98 reader. Integers inside a blob are little-endian (copied raw), while
//! the compact varints use the same prefix coding as packets.
//!
//! Section sizes were measured with the DLL's reader trace (all flags and counts zero); see
//! `Documentation/protocol/entities.md`. Field meanings marked `unk` are not known yet.

use poe_net::Writer;

/// One component's section of an initial-state blob.
#[derive(Debug, Clone, PartialEq)]
pub enum Component {
    Positioned(Positioned),
    Stats(Stats),
    /// No active buffs (u16 count 0). Reader `0x141db4f00`.
    Buffs,
    Life(Life),
    /// Flags byte 0 (no animation override). Reader `0x141da6540`.
    Animated,
    Player(Player),
    /// No inventories (two zero counts). Reader `0x141d82a40`.
    Inventories,
    Actor(Actor),
}

impl Component {
    fn encode(&self, w: &mut Writer) {
        match self {
            Self::Positioned(c) => c.encode(w),
            Self::Stats(c) => c.encode(w),
            Self::Buffs => {
                w.u16_le(0);
            }
            Self::Life(c) => c.encode(w),
            Self::Animated => {
                w.u8(0);
            }
            Self::Player(c) => c.encode(w),
            Self::Inventories => {
                w.u8(0).u8(0);
            }
            Self::Actor(c) => c.encode(w),
        }
    }
}

/// Encodes a full initial-state blob from the serialized components, in template order.
pub fn initial_state_blob(components: &[Component]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(0).u8(0);
    for c in components {
        c.encode(&mut w);
    }
    w.u8(0);
    w.into_vec()
}

/// Reader `0x141dd8820`. Without the optional sections (flags 0) it is 17 bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Positioned {
    /// Grid position (+0x444/+0x448), clamped by the client to the area size.
    pub grid_x: i32,
    pub grid_y: i32,
    pub unk_44c: u32,
    pub unk_1e0: u16,
}

impl Positioned {
    fn encode(&self, w: &mut Writer) {
        w.i32_le(self.grid_x)
            .i32_le(self.grid_y)
            .u32_le(self.unk_44c)
            .u16_le(self.unk_1e0)
            // Three flag bytes select optional sections; none are sent.
            .bytes(&[0; 3]);
    }
}

/// Reader `0x142180fe0`. For players the type enables a second section (6 bytes here).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    /// (stat key, value) pairs: varuint key, signed varint value.
    pub stats: Vec<(u32, i32)>,
    pub unk_1b0: u32,
}

impl Stats {
    fn encode(&self, w: &mut Writer) {
        w.varuint(self.stats.len() as u32);
        for &(key, value) in &self.stats {
            w.varuint(key).varint(value);
        }
        // Player section: u8, u8 (0: no lookup), three u8 references, u8 count of stat sets.
        w.bytes(&[0; 6]);
        w.u32_le(self.unk_1b0).u8(0).u8(0);
    }
}

/// A resource pool record of [`Life`] (`0x141d88cf0`, 14 bytes; `0x141d8bd40` adds a u32).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pool {
    /// +0x38 (with `current` equal, the meaning of the two does not matter yet).
    pub maximum: u32,
    pub unk_18: u32,
    pub unk_1c: i16,
    /// +0x30.
    pub current: u32,
    /// +0x48, only in the long form.
    pub unk_48: u32,
}

impl Pool {
    pub fn full(value: u32) -> Self {
        Self {
            maximum: value,
            current: value,
            ..Default::default()
        }
    }

    fn encode(&self, w: &mut Writer, long: bool) {
        w.u32_le(self.maximum)
            .u32_le(self.unk_18)
            .i16_le(self.unk_1c)
            .u32_le(self.current);
        if long {
            w.u32_le(self.unk_48);
        }
    }
}

/// Reader `0x141d8c0e0`: six pools (long, short, long, short, long, long), a type-defined
/// number of (u32, i16) entries (3 for characters), one more (u32, i16) and a flags byte.
#[derive(Debug, Clone, PartialEq)]
pub struct Life {
    /// +0x1a8, assumed to be life.
    pub life: Pool,
    pub unk_200: Pool,
    /// +0x240, assumed to be mana.
    pub mana: Pool,
    pub unk_298: Pool,
    /// +0x2e0, assumed to be energy shield.
    pub energy_shield: Pool,
    pub unk_330: Pool,
    pub unk_380: Vec<(u32, i16)>,
    pub unk_398: (u32, i16),
    pub flags: u8,
}

impl Life {
    /// Character defaults: full life and mana, three `unk_380` entries.
    pub fn character(life: u32, mana: u32) -> Self {
        Self {
            life: Pool::full(life),
            unk_200: Pool::default(),
            mana: Pool::full(mana),
            unk_298: Pool::default(),
            energy_shield: Pool::default(),
            unk_330: Pool::default(),
            unk_380: vec![(0, 0); 3],
            unk_398: (0, 0),
            flags: 0,
        }
    }

    fn encode(&self, w: &mut Writer) {
        self.life.encode(w, true);
        self.unk_200.encode(w, false);
        self.mana.encode(w, true);
        self.unk_298.encode(w, false);
        self.energy_shield.encode(w, true);
        self.unk_330.encode(w, true);
        for &(a, b) in self.unk_380.iter().chain([&self.unk_398]) {
            w.u32_le(a).i16_le(b);
        }
        w.u8(self.flags);
    }
}

/// Reader `0x141dcf320`. 35 bytes plus the name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Player {
    pub name: String,
    pub unk_c: u8,
    pub unk_u32: u32,
    pub unk_1d0: u32,
    pub unk_1d4: u32,
}

impl Player {
    fn encode(&self, w: &mut Writer) {
        w.utf16_le(&self.name)
            .u8(self.unk_c)
            .u32_le(self.unk_u32)
            .u32_le(self.unk_1d0)
            .u32_le(self.unk_1d4)
            // u16 +0x200, u16 +0x202, two empty (u8, u64) lists, 5 bytes, two u16 data keys,
            // flags (bit 0: "WashedUp" intro), u8, u8, varuint count of u32s, u8 count.
            .bytes(&[0; 2 + 2 + 2 + 2 + 5 + 2 + 2 + 1 + 1 + 1 + 1 + 1]);
    }
}

/// Reader `0x1416d8090` (body `0x142147180`). Only the all-zero form (48 bytes) is known.
#[derive(Debug, Clone, PartialEq)]
pub struct Actor {
    pub raw: Vec<u8>,
}

impl Default for Actor {
    fn default() -> Self {
        Self { raw: vec![0; 48] }
    }
}

impl Actor {
    fn encode(&self, w: &mut Writer) {
        w.bytes(&self.raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(c: &Component) -> usize {
        let mut w = Writer::new();
        c.encode(&mut w);
        w.len()
    }

    /// Spans measured in the client with an all-zero blob (DLL reader trace).
    #[test]
    fn zero_sections_match_client_spans() {
        assert_eq!(size(&Component::Positioned(Positioned::default())), 17);
        assert_eq!(size(&Component::Stats(Stats::default())), 13);
        assert_eq!(size(&Component::Buffs), 2);
        let mut life = Life::character(0, 0);
        life.life = Pool::default();
        assert_eq!(size(&Component::Life(life)), 125);
        assert_eq!(size(&Component::Animated), 1);
        assert_eq!(size(&Component::Player(Player::default())), 39);
        assert_eq!(size(&Component::Inventories), 2);
        assert_eq!(size(&Component::Actor(Actor::default())), 48);
    }
}
