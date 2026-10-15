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
    /// Reader `0x141da6540`. With a path, flags bit 0 is set and the animated object
    /// (model) is loaded from it; players need it (`Characters.AOFile`).
    Animated(Option<String>),
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
            Self::Animated(path) => match path {
                Some(p) => {
                    // Bit 0: animated object by path.
                    w.u8(0x01).utf16_le(p);
                }
                None => {
                    w.u8(0);
                }
            },
            Self::Player(c) => c.encode(w),
            Self::Inventories => {
                w.u8(0).u8(0);
            }
            Self::Actor(c) => c.encode(w),
        }
    }
}

/// Entity flags (first blob byte): bit 0 -> entity +0x8d |= 1; bit 1 -> +0x8d |= 4, the
/// object starts in the **wake zone** (`ObjectInWakeZone`, `0x141e73bf0`): awake objects
/// are updated and drawn. A player created without it stays invisible (and so does its
/// light, leaving the scene dark).
pub const ENTITY_IN_WAKE_ZONE: u8 = 0x02;

/// Encodes a full initial-state blob from the entity flags and the serialized components,
/// in template order.
pub fn initial_state_blob(flags: u8, components: &[Component]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(flags).u8(0);
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
    /// Facing (+0x44c, radians as f32 bits).
    pub unk_44c: u32,
    /// +0x1e0: team in the low 15 bits (players: 1), bit 15 a flag.
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
    /// +0x1a8: life (the life orb shows it).
    pub life: Pool,
    /// +0x200: mana (with the value in +0x240 instead, the mana orb stayed empty).
    pub mana: Pool,
    pub unk_240: Pool,
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
            mana: Pool::full(mana),
            unk_240: Pool::default(),
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
        self.mana.encode(w, false);
        self.unk_240.encode(w, true);
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
    /// Written to the PlayerClass component (+0x158), which rebuilds when it changes:
    /// the class (`Characters.IntegerId`).
    pub class: u8,
    pub unk_u32: u32,
    pub unk_1d0: u32,
    pub unk_1d4: u32,
    /// Flags bit 0: the "WashedUp" opening animation (new characters on the Riverbank).
    pub washed_up: bool,
}

impl Player {
    fn encode(&self, w: &mut Writer) {
        w.utf16_le(&self.name)
            .u8(self.class)
            .u32_le(self.unk_u32)
            .u32_le(self.unk_1d0)
            .u32_le(self.unk_1d4)
            // u16 +0x200, u16 +0x202, two empty (u8, u64) lists, 5 bytes, two u16 data keys.
            .bytes(&[0; 2 + 2 + 2 + 2 + 5 + 2 + 2])
            // Flags (bit 0: washed up).
            .u8(self.washed_up as u8)
            // u8, u8, varuint count of u32s, u8 count.
            .bytes(&[0; 4]);
    }
}

/// A Player component delta (component vfunc +0xb0, `0x141dd0a10`): a `u16` field mask
/// (raw, little-endian), then the fields whose bit is set, in the reader's order. Only the
/// fields seen in official traffic are supported:
/// - `0x0002` flag words: `u16` count (raw), then (`u8` key, `u64` bits) entries. Official
///   servers send the whole set (5 at entry, key 0x08 added after the first Move).
/// - `0x0400` combat stance (`u8`, +0x20c): the component's vfunc +0x138 (`0x141dd47e0`)
///   sets +0x20d from it. While +0x20d is set the client reports its cursor cell
///   (`AimTarget` mode 2, `0x14016ba40`) and the player turns toward it. Official servers
///   send 1 at entry, 0 right after, and 1 again with the first Move.
/// - `0x1000` a `u8` (+0x3e7, unknown; official: 1 right after entry).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerDelta {
    pub flag_words: Option<Vec<(u8, u64)>>,
    pub combat_stance: Option<bool>,
    pub unk_3e7: Option<u8>,
}

/// Player component index in the player template (the official `ComponentDelta`s use 6).
pub const PLAYER_COMPONENT: u8 = 6;

impl PlayerDelta {
    pub fn encode(&self) -> Vec<u8> {
        let mut mask = 0u16;
        mask |= if self.flag_words.is_some() { 0x0002 } else { 0 };
        mask |= if self.combat_stance.is_some() {
            0x0400
        } else {
            0
        };
        mask |= if self.unk_3e7.is_some() { 0x1000 } else { 0 };
        let mut w = Writer::new();
        w.u16_le(mask);
        if let Some(words) = &self.flag_words {
            w.u16_le(words.len() as u16);
            for (key, bits) in words {
                w.u8(*key).bytes(&bits.to_le_bytes());
            }
        }
        if let Some(on) = self.combat_stance {
            w.u8(on as u8);
        }
        if let Some(v) = self.unk_3e7 {
            w.u8(v);
        }
        w.into_vec()
    }
}

/// Reader `0x1416d8090` (body `0x142147180`):
/// - 3 × (`u8`, `u8`, `u32` length + UTF-16 string) (18 bytes when empty);
/// - sub-reader `0x142149390`: `u8` flags, then (initial state) the skill container
///   (`0x14218a850`): `u8` skill count + [`Skill`]s, `u8` count + 12-byte entries, `u8` count
///   + bytes, `u32` count + entries;
/// - the rest (flags, action state) is sent as zeros (22 bytes).
///
/// Skills the template's `basic_action`s grant (Move, Interaction...) exist on the client
/// without being sent; granted effects such as dodge roll have to be listed here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Actor {
    pub skills: Vec<Skill>,
}

impl Actor {
    fn encode(&self, w: &mut Writer) {
        w.bytes(&[0; 18]);
        // Sub-reader flags, then the skill container.
        w.u8(0);
        w.u8(self.skills.len() as u8);
        for skill in &self.skills {
            skill.encode(w);
        }
        w.u8(0).u8(0).u32_le(0);
        w.bytes(&[0; 22]);
    }
}

/// One skill of an actor (reader `0x142188bd0`; all integers little-endian):
/// `u32 id, u8 slot, effect, u8 flags (bit 0: u32 parent object, u32), u8 supports count +
/// (effect, u32)[], u32, u32, u32, u8, stat list` (varuint count + (varuint stat, varint
/// value)[]), one more stat list per additional stat set, a minion block when the granted
/// effect has a MinionType, then `u8` (non-zero: one more stat list).
///
/// The effect (`0x142683e50`): `u8, u32 GrantedEffectsPerLevel row`, then a stat-set reference
/// (`0x142683b70`: `u8 b, [u8 if b], u32 GrantedEffectStatSetsPerLevel row`) for the base stat
/// set and one per additional stat set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skill {
    /// Skill instance id: the id use requests (`0x1a5`) and actions (`0x1c8`) refer to.
    /// Basic actions use `0x20000000 | action type`.
    pub id: u32,
    /// `GrantedEffectsPerLevel` row.
    pub effect_level_row: u32,
    /// `GrantedEffectStatSetsPerLevel` rows: the base stat set, then the additional ones.
    pub stat_set_rows: Vec<u32>,
    /// The base stat set's stats as (network key, value). The client takes skill stats from
    /// here, not from the tables: without `dodge_roll_base_travel_distance` a dodge roll logs
    /// "Trying to do pathfinding action movement with base travel distance <= 0" and stays.
    pub stats: Vec<(u32, i32)>,
}

impl Skill {
    fn encode(&self, w: &mut Writer) {
        w.u32_le(self.id).u8(0);
        w.u8(0).u32_le(self.effect_level_row);
        for &row in &self.stat_set_rows {
            w.u8(0).u32_le(row);
        }
        // Flags, supports, three u32s and a u8.
        w.u8(0).u8(0).u32_le(0).u32_le(0).u32_le(0).u8(0);
        // Stat lists: the base one, then one per additional stat set (empty).
        w.varuint(self.stats.len() as u32);
        for &(key, value) in &self.stats {
            w.varuint(key).varint(value);
        }
        for _ in 1..self.stat_set_rows.len() {
            w.varuint(0);
        }
        w.u8(0);
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
        assert_eq!(size(&Component::Animated(None)), 1);
        assert_eq!(size(&Component::Player(Player::default())), 39);
        assert_eq!(size(&Component::Inventories), 2);
        assert_eq!(size(&Component::Actor(Actor::default())), 48);
    }

    /// Official deltas (comp 6): `0014 00 01` and the entry's flag words + stance.
    #[test]
    fn player_delta_matches_official() {
        let off = PlayerDelta {
            combat_stance: Some(false),
            unk_3e7: Some(1),
            ..Default::default()
        };
        assert_eq!(off.encode(), [0x00, 0x14, 0x00, 0x01]);
        let on = PlayerDelta {
            flag_words: Some(vec![(0x1f, 0x2000_0000), (0x08, 0x40_0000)]),
            combat_stance: Some(true),
            ..Default::default()
        };
        let mut expect = vec![0x02, 0x04, 0x02, 0x00];
        expect.extend([0x1f, 0, 0, 0, 0x20, 0, 0, 0, 0]);
        expect.extend([0x08, 0, 0, 0x40, 0, 0, 0, 0, 0]);
        expect.push(1);
        assert_eq!(on.encode(), expect);
    }

    #[test]
    fn actor_skill_layout() {
        let skill = Skill {
            id: 1,
            effect_level_row: 899,
            stat_set_rows: vec![905],
            stats: vec![(0x457d, 46)],
        };
        let mut w = Writer::new();
        skill.encode(&mut w);
        let mut expect = vec![1, 0, 0, 0, 0]; // id, slot
        expect.extend([0, 0x83, 0x03, 0, 0]); // effect: u8, GrantedEffectsPerLevel 899
        expect.extend([0, 0x89, 0x03, 0, 0]); // stat set: u8, row 905
        expect.extend([0; 2 + 12 + 1]); // flags, supports, 3 x u32, u8
        expect.extend([1, 0xc0, 0x45, 0x7d, 46]); // stats: count, key (varuint), value
        expect.push(0);
        assert_eq!(w.as_slice(), expect);
        // The skill count sits after the 18-byte prefix and the sub-reader flags.
        let actor = Actor {
            skills: vec![skill],
        };
        let mut w = Writer::new();
        actor.encode(&mut w);
        assert_eq!(w.as_slice()[19], 1);
        assert_eq!(w.len(), 48 + expect.len());
    }
}
