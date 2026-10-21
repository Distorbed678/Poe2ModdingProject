//! How a character is described to clients: the `AddObject` blob of its own client (entry
//! state) and the remote form other clients get (docs/protocol/entities.md, "Local vs remote
//! `AddObject` blob"). Both are built before the session joins the instance actor, which adds
//! the entity id and the current position when it sends them ([`crate::actor`]).

use std::sync::Arc;

use poe_cluster::TicketCharacter;
use poe_data::{template::ObjectTemplate, views::CharacterClass};
use poe_game::stats::{self as game_stats, StatRegistry};
use poe_proto::{
    entity::{self, Actor, Component, Life, Player, Pool, Positioned, Skill, Stats},
    player_blob::{ActorSection, AnimationSet, PlayerBlob, PlayerSection, PlayerStats, StatSet},
};
use tracing::{info, warn};

use crate::{
    InstanceServer,
    actor::Appearance,
    player::{OwnedSkill, Player as SessionPlayer},
};

/// Facing (+0x44c), the Animated turn speed (+0x25c, official remote blobs: 0.05 to 0.2) and
/// the other Animated floats of a remote blob.
const ANIMATED_FLAGS: u8 = 0x80;
const TURN_SPEED: f32 = 0.05;
/// `ShapeshiftForms` row + 1 of the three weapon-set references (2 for every official
/// character observed).
const SHAPESHIFT_SLOTS: [u8; 3] = [2, 2, 2];
/// The Stats component's trailing bytes in official remote blobs (`u32 0, u8 0, u8 1`).
const STATS_TRAILER: (u32, u8, u8) = (0, 0, 1);
/// `+0x3e7` of remote players (2; the local player's is 4).
const REMOTE_UNK_3E7: u8 = 2;
/// Player flags of every official remote blob: bit 3 always set; bit 2 is the combat stance,
/// which the instance actor adds while the player is in it ([`crate::actor`]).
pub const REMOTE_PLAYER_FLAGS: u8 = 0x08;
/// Stat set ids of the Stats component: the two weapon sets and the main/off hand weapon.
const SET_WEAPON_1: u8 = 0x40;
const SET_WEAPON_2: u8 = 0x41;
const SET_OFF_HAND: u8 = 0x81;
/// `Life.flags` of every official blob (local and remote).
const LIFE_FLAGS: u8 = 1;
/// Bytes of the three empty animation sets in an Actor component (the local encoding writes
/// them as zeros), then the sub-reader and the rest, which the remote blob keeps raw.
const ACTOR_SETS_BYTES: usize = 18;
/// Entity flags and link count in front of the components of `initial_state_blob`.
const BLOB_HEADER_BYTES: usize = 2;

/// The parts of the official remote layout that can be left out one by one
/// (`[dev] remote_blob_omit`, to find which part the client reacts to).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteParts {
    pub stat_sets: bool,
    pub derived_stats: bool,
    pub life_state: bool,
    pub player_flags: bool,
}

impl Default for RemoteParts {
    fn default() -> Self {
        Self {
            stat_sets: true,
            derived_stats: true,
            life_state: true,
            player_flags: true,
        }
    }
}

impl RemoteParts {
    /// Everything except the named parts; unknown names are returned.
    pub fn omitting(names: &[String]) -> (Self, Vec<String>) {
        let mut parts = Self::default();
        let mut unknown = Vec::new();
        for name in names {
            match name.as_str() {
                "stat_sets" => parts.stat_sets = false,
                "derived_stats" => parts.derived_stats = false,
                "life_state" => parts.life_state = false,
                "player_flags" => parts.player_flags = false,
                _ => unknown.push(name.clone()),
            }
        }
        (parts, unknown)
    }
}

/// What a remote blob is made of. Everything else in the blob is the official default.
#[derive(Default)]
pub struct RemoteInit<'a> {
    pub name: &'a str,
    /// `Characters.IntegerId`.
    pub class: u8,
    pub cell: (i32, i32),
    pub team: u16,
    /// Base stats as (network key, value), `level` among them.
    pub stats: Vec<(u32, i32)>,
    /// The Stats component's weapon set stat sets, by id (empty: none).
    pub stat_sets: Vec<StatSet>,
    pub life: u32,
    pub mana: u32,
    pub energy_shield: u32,
    /// Energy shield recharge delay in seconds (the official blobs store it in the pools).
    pub recharge_delay: f32,
    pub parts: RemoteParts,
    /// The class's animated object path, when the blob should carry it.
    pub animated_path: Option<String>,
    /// Main hand, off hand and default animation set names.
    pub animation_sets: [String; 3],
    pub skills: Vec<Skill>,
}

/// The pools of an official blob: `current` (+0x30) 0 everywhere (all five blobs of the town
/// capture, local and remote), the life, energy shield and last pool carry the recharge delay as
/// f32 seconds in `unk_48` (4.0 or 2.0 = the `virtual_total_energy_shield_recharge_delay_ms`
/// stat / 1000, in a fresh character; older sessions have negative timers there), flags 1.
fn official_life(init: &RemoteInit<'_>) -> Life {
    let pool = |maximum: u32, unk_48: u32| Pool {
        maximum,
        current: 0,
        unk_48,
        ..Default::default()
    };
    let delay = init.recharge_delay.to_bits();
    Life {
        life: pool(init.life, delay),
        mana: pool(init.mana, 0),
        unk_240: pool(init.energy_shield, delay),
        unk_298: pool(0, 0),
        energy_shield: pool(0, 0),
        unk_330: pool(0, delay),
        unk_380: vec![(0, 0); 3],
        unk_398: (0, 0),
        flags: LIFE_FLAGS,
    }
}

/// The weapon set stat sets of an unarmed character: set 1 carries the derived totals and
/// `both_hands_are_empty`, set 2 only what separates the sets, the off hand set is empty (as in
/// every official blob; the main hand set only exists with a weapon). Ids by name, so a
/// stat the registry does not know is skipped.
pub fn unarmed_stat_sets(stats: &game_stats::Stats, registry: &StatRegistry) -> Vec<StatSet> {
    let list = |entries: Vec<(&str, i32)>| {
        let mut list: Vec<(u32, i32)> = entries
            .into_iter()
            .filter_map(|(id, v)| Some((registry.key(id)?, v)))
            .collect();
        list.sort_unstable();
        list
    };
    let set_1 = list(vec![
        ("maximum_life", stats.get("maximum_life")),
        ("maximum_mana", stats.get("maximum_mana")),
        ("maximum_energy_shield", stats.get("maximum_energy_shield")),
        ("poise_threshold", stats.get("poise_threshold")),
        (
            "intermediary_poise_threshold",
            stats.get("intermediary_poise_threshold"),
        ),
        (
            "virtual_total_energy_shield_recharge_delay_ms",
            stats.get("virtual_total_energy_shield_recharge_delay_ms"),
        ),
        ("both_hands_are_empty", 1),
        ("weapon_set_index", 1),
        ("weapon_set_inherent_shapeshift_form", 1),
    ]);
    let set_2 = list(vec![
        ("weapon_set_index", 2),
        ("weapon_set_inherent_shapeshift_form", 1),
    ]);
    vec![
        StatSet {
            id: SET_WEAPON_1,
            stats: set_1.into_iter().filter(|&(_, v)| v != 0).collect(),
        },
        StatSet {
            id: SET_WEAPON_2,
            stats: set_2,
        },
        StatSet {
            id: SET_OFF_HAND,
            stats: Vec::new(),
        },
    ]
}

/// The blob other clients get for a character: the official remote layout without the parts the
/// server has no data for (equipment visuals, links, buffs, quest flag words).
pub fn remote_blob(init: &RemoteInit<'_>) -> PlayerBlob {
    // The Actor component's skill container and rest, as the solo blob encodes it.
    let actor = entity::initial_state_blob(
        0,
        &[Component::Actor(Actor {
            skills: init.skills.clone(),
        })],
    );
    let tail = actor[BLOB_HEADER_BYTES + ACTOR_SETS_BYTES..].to_vec();
    let [main, off, default] = init.animation_sets.clone();
    let set = |name: String| AnimationSet { a: 0, b: 0, name };
    PlayerBlob {
        flags: entity::ENTITY_IN_WAKE_ZONE,
        links: Vec::new(),
        grid_x: init.cell.0,
        grid_y: init.cell.1,
        facing: 0,
        team: init.team,
        stats: PlayerStats {
            base: init.stats.clone(),
            unk_168: 0,
            form: 0,
            slots: SHAPESHIFT_SLOTS,
            sets: if init.parts.stat_sets {
                init.stat_sets.clone()
            } else {
                Vec::new()
            },
            unk_1b0: STATS_TRAILER.0,
            unk_1b4: STATS_TRAILER.1,
            unk_1b5: STATS_TRAILER.2,
        },
        buffs: Vec::new(),
        life: if init.parts.life_state {
            official_life(init)
        } else {
            Life::character(init.life, init.mana)
        },
        animated_flags: ANIMATED_FLAGS | u8::from(init.animated_path.is_some()),
        animated_path: init.animated_path.clone(),
        animated: [TURN_SPEED.to_bits(), 0, 0, 0, 0],
        player: PlayerSection {
            name: init.name.to_owned(),
            class: init.class,
            experience: 0,
            unk_1d0: 0,
            unk_1d4: 0,
            unk_200: 0,
            unk_202: 0,
            flags_a: Vec::new(),
            flags_b: Vec::new(),
            bytes5: [0; 5],
            hideout_a: 0,
            hideout_b: 0,
            flags: if init.parts.player_flags {
                REMOTE_PLAYER_FLAGS
            } else {
                0
            },
            unk_205: 0,
            unk_3e7: REMOTE_UNK_3E7,
            unk_list: Vec::new(),
        },
        equipment: Vec::new(),
        inventory_refs: Vec::new(),
        actor: ActorSection {
            sets: [set(main), set(off), set(default)],
            tail,
        },
    }
}

impl InstanceServer {
    /// Both blobs of a character standing on `cell`, with the class's object type key.
    pub(crate) fn appearance(
        &self,
        character: &TicketCharacter,
        class: &CharacterClass,
        cell: (i32, i32),
        skills: &[OwnedSkill],
        washed_up: bool,
    ) -> anyhow::Result<Appearance> {
        let template = ObjectTemplate::load(&self.data, &class.metadata_id)?;
        let stats = poe_game::stats::character_defaults(&template, class, character.level);
        let team = template
            .component("Positioned")
            .iter()
            .find(|(k, _)| k == "team")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let pool = |id: &str| stats.get(id).max(1) as u32;
        info!(
            class = %class.name,
            level = character.level,
            maximum_life = stats.get("maximum_life"),
            maximum_mana = stats.get("maximum_mana"),
            stats = stats.0.len(),
            "player stats"
        );
        let network = stats.to_network(&self.stats);
        let actor_skills = SessionPlayer::actor_skills(skills);
        // The character's own client: the class's model path in Animated (it works without the
        // official extras). The blob is built for the cell the instance actor finally puts the
        // character on.
        let local_components = [
            Component::Stats(Stats {
                stats: network.clone(),
                ..Default::default()
            }),
            Component::Buffs,
            Component::Life(Life::character(pool("maximum_life"), pool("maximum_mana"))),
            Component::Animated(Some(class.ao_file.clone())),
            Component::Player(Player {
                name: character.name.clone(),
                class: class.integer_id as u8,
                washed_up,
                ..Default::default()
            }),
            Component::Inventories,
            Component::Actor(Actor {
                skills: actor_skills.clone(),
            }),
        ];
        let local_blob = Arc::new(move |cell: (i32, i32)| {
            let mut components = vec![Component::Positioned(Positioned {
                grid_x: cell.0,
                grid_y: cell.1,
                // Team (low 15 bits), from the template's Positioned section.
                unk_1e0: team,
                ..Default::default()
            })];
            components.extend(local_components.iter().cloned());
            entity::initial_state_blob(entity::ENTITY_IN_WAKE_ZONE, &components)
        });
        // Other clients: the official remote layout with what the character's data gives.
        let (parts, unknown) = RemoteParts::omitting(&self.config.dev.remote_blob_omit);
        if !unknown.is_empty() {
            warn!(
                ?unknown,
                "[dev] remote_blob_omit: unknown part names (stat_sets, derived_stats, life_state, player_flags)"
            );
        }
        let mut remote_stats = stats.clone();
        if parts.derived_stats {
            game_stats::add_resting_state(&mut remote_stats);
        }
        let mut remote_network = remote_stats.to_network(&self.stats);
        remote_network.sort_unstable();
        let remote = remote_blob(&RemoteInit {
            name: &character.name,
            class: class.integer_id as u8,
            cell,
            team,
            stats: remote_network,
            stat_sets: unarmed_stat_sets(&remote_stats, &self.stats),
            life: pool("maximum_life"),
            mana: pool("maximum_mana"),
            energy_shield: remote_stats.get("maximum_energy_shield").max(0) as u32,
            recharge_delay: remote_stats.get("virtual_total_energy_shield_recharge_delay_ms")
                as f32
                / 1000.0,
            parts,
            animated_path: self
                .config
                .dev
                .remote_animated_path
                .then(|| class.ao_file.clone()),
            animation_sets: self.config.player.animation_sets.clone(),
            skills: actor_skills,
        });
        Ok(Appearance {
            type_key: class.object_type_key(),
            local_blob,
            remote,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init(skills: Vec<Skill>) -> RemoteInit<'static> {
        RemoteInit {
            name: "Alice",
            class: 2,
            cell: (314, 246),
            team: 1,
            stats: vec![(1, 10), (2, 100)],
            life: 120,
            mana: 60,
            animation_sets: ["UnarmedTown".into(), "UnarmedTown".into(), "default".into()],
            skills,
            ..Default::default()
        }
    }

    /// Anonymised blob of an official remote player (town capture 2026-10-20); the decoder's
    /// own fixtures.
    const OFFICIAL_REMOTE: &str =
        include_str!("../../../modules/protocol/poe-proto/tests/data/remote_dex_blob.hex");
    const OFFICIAL_REMOTE_2: &str =
        include_str!("../../../modules/protocol/poe-proto/tests/data/remote_int_blob.hex");

    fn official(text: &str) -> PlayerBlob {
        let bytes: Vec<u8> = text
            .trim()
            .as_bytes()
            .chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect();
        PlayerBlob::decode(&bytes).unwrap()
    }

    /// Real `Stats.dat` keys of the stats the weapon sets use (rows + 1).
    fn registry() -> StatRegistry {
        StatRegistry::from_rows(&[
            ("maximum_life", 239, true),
            ("maximum_mana", 240, true),
            ("maximum_energy_shield", 241, true),
            ("poise_threshold", 17375, true),
            ("intermediary_poise_threshold", 18619, true),
            ("virtual_total_energy_shield_recharge_delay_ms", 21909, true),
            ("both_hands_are_empty", 22307, true),
            ("weapon_set_index", 23513, true),
            ("weapon_set_inherent_shapeshift_form", 24174, true),
        ])
    }

    /// A remote blob built the way `appearance` does it for a level 10 Ranger-line character.
    fn generated(recharge_delay: f32) -> PlayerBlob {
        let mut stats = game_stats::Stats::default();
        stats.set("level", 10);
        stats.set("maximum_life", 188);
        stats.set("maximum_mana", 209);
        game_stats::add_resting_state(&mut stats);
        let registry = registry();
        let mut init = init(Vec::new());
        init.stat_sets = unarmed_stat_sets(&stats, &registry);
        init.stats = stats.to_network(&registry);
        init.life = 188;
        init.mana = 209;
        init.recharge_delay = recharge_delay;
        remote_blob(&init)
    }

    /// The generated remote blob against the decoded official one: the same parts are present
    /// with the same flags and constants; what the server has no data for is absent.
    #[test]
    fn the_remote_blob_has_the_official_parts_and_flags() {
        for text in [OFFICIAL_REMOTE, OFFICIAL_REMOTE_2] {
            let off = official(text);
            // The recharge delay in the life pool is the stat of that name in seconds.
            let delay_ms = off.stats.base.iter().find(|&&(k, _)| k == 21909).unwrap().1;
            assert_eq!(
                f32::from_bits(off.life.life.unk_48),
                delay_ms as f32 / 1000.0
            );
            let ours = generated(delay_ms as f32 / 1000.0);
            // Header and constants.
            assert_eq!(ours.flags, off.flags);
            assert_eq!(ours.team, off.team);
            assert_eq!(ours.stats.slots, off.stats.slots);
            assert_eq!(
                (ours.stats.unk_168, ours.stats.form),
                (off.stats.unk_168, off.stats.form)
            );
            assert_eq!(
                (ours.stats.unk_1b0, ours.stats.unk_1b4, ours.stats.unk_1b5),
                (off.stats.unk_1b0, off.stats.unk_1b4, off.stats.unk_1b5)
            );
            // Animated: flags 0x80 with five floats, no path; turn speed within the official range.
            assert_eq!(ours.animated_flags, off.animated_flags);
            assert_eq!(ours.animated_path, None);
            let turn = f32::from_bits(ours.animated[0]);
            assert!((0.05..=0.2).contains(&turn));
            // Player: bit 3 always set, +0x3e7, the class byte's position.
            assert_ne!(ours.player.flags & 0x08, 0);
            assert_eq!(off.player.flags & 0x08, 0x08);
            assert_eq!(ours.player.unk_3e7, off.player.unk_3e7);
            // Life: flags 1, nothing in the second field of any pool (+0x30), as in all five
            // official blobs; the recharge delay sits in the life pool.
            assert_eq!(ours.life.flags, off.life.flags);
            for (a, b) in [
                (ours.life.life, off.life.life),
                (ours.life.mana, off.life.mana),
                (ours.life.unk_240, off.life.unk_240),
                (ours.life.energy_shield, off.life.energy_shield),
            ] {
                assert_eq!(a.current, b.current);
            }
            assert_eq!(ours.life.life.unk_48, off.life.life.unk_48);
            assert_eq!(ours.life.unk_380, off.life.unk_380);
            // Stat sets: ids 0x40, 0x41 and the empty 0x81 (0x80 only exists with a weapon).
            let ids = |b: &PlayerBlob| b.stats.sets.iter().map(|s| s.id).collect::<Vec<_>>();
            assert_eq!(ids(&ours), [0x40, 0x41, 0x81]);
            assert!(ids(&off).starts_with(&[0x40, 0x41]));
            let set = |b: &PlayerBlob, id: u8| {
                b.stats
                    .sets
                    .iter()
                    .find(|s| s.id == id)
                    .unwrap()
                    .stats
                    .clone()
            };
            assert!(set(&ours, 0x81).is_empty() && set(&off, 0x81).is_empty());
            // Every stat key of our weapon sets is one the official sets or base stats carry.
            let known: Vec<u32> = off
                .stats
                .sets
                .iter()
                .flat_map(|s| s.stats.iter().map(|&(k, _)| k))
                .chain(off.stats.base.iter().map(|&(k, _)| k))
                .collect();
            for id in [0x40, 0x41] {
                for (key, _) in set(&ours, id) {
                    assert!(known.contains(&key), "set {id:#x}: key {key}");
                }
            }
            // Set 2 is exactly the official's two index stats.
            let two = set(&ours, 0x41);
            let off_two = set(&off, 0x41);
            for pair in &two {
                assert!(off_two.contains(pair), "{pair:?}");
            }
            // Not generated: no data for them.
            assert!(ours.links.is_empty() && ours.buffs.is_empty() && ours.equipment.is_empty());
            assert!(ours.player.flags_a.is_empty() && ours.player.flags_b.is_empty());
            assert!(!off.links.is_empty() && !off.equipment.is_empty());
        }
    }

    #[test]
    fn weapon_set_stats_of_an_unarmed_character() {
        let mut stats = game_stats::Stats::default();
        stats.set("maximum_life", 188);
        stats.set("maximum_mana", 209);
        game_stats::add_resting_state(&mut stats);
        let sets = unarmed_stat_sets(&stats, &registry());
        let first = &sets[0].stats;
        // Ascending by key, zero values dropped (no energy shield).
        assert!(first.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(!first.iter().any(|&(k, _)| k == 241));
        assert!(first.contains(&(239, 188)) && first.contains(&(240, 209)));
        assert!(first.contains(&(22307, 1)), "both_hands_are_empty");
        assert!(first.contains(&(23513, 1)) && first.contains(&(24174, 1)));
        assert!(first.contains(&(21909, 4000)));
        assert_eq!(sets[1].stats, [(23513, 2), (24174, 1)]);
    }

    #[test]
    fn parts_can_be_left_out_one_by_one() {
        let mut init = init(Vec::new());
        init.stat_sets = unarmed_stat_sets(&game_stats::Stats::default(), &registry());
        let full = remote_blob(&init);
        assert!(!full.stats.sets.is_empty());
        assert_eq!(full.life.flags, 1);
        assert_eq!(full.player.flags, REMOTE_PLAYER_FLAGS);
        let (parts, unknown) = RemoteParts::omitting(&[
            "stat_sets".into(),
            "life_state".into(),
            "player_flags".into(),
            "nonsense".into(),
        ]);
        assert_eq!(unknown, ["nonsense"]);
        init.parts = parts;
        let bare = remote_blob(&init);
        assert!(bare.stats.sets.is_empty());
        // The solo blob's life: full pools, flags 0.
        assert_eq!((bare.life.flags, bare.life.life.current), (0, 120));
        assert_eq!(bare.player.flags, 0);
        assert_eq!(RemoteParts::default(), RemoteParts::omitting(&[]).0);
    }

    #[test]
    fn the_animated_path_is_optional() {
        let mut init = init(Vec::new());
        assert_eq!(remote_blob(&init).animated_path, None);
        init.animated_path = Some("Metadata/Characters/Dex/DexFour.ao".into());
        let blob = remote_blob(&init);
        let back = PlayerBlob::decode(&blob.encode()).unwrap();
        assert_eq!(
            back.animated_path.as_deref(),
            Some("Metadata/Characters/Dex/DexFour.ao")
        );
        assert_eq!(back, blob);
    }

    #[test]
    fn the_remote_blob_decodes_as_an_official_player_blob() {
        let skill = Skill {
            id: 1,
            effect_level_row: 899,
            stat_set_rows: vec![905],
            stats: vec![(7, 46)],
        };
        let blob = remote_blob(&init(vec![skill.clone()]));
        let bytes = blob.encode();
        let back = PlayerBlob::decode(&bytes).unwrap();
        assert_eq!(back, blob);
        assert_eq!((back.player.name.as_str(), back.player.class), ("Alice", 2));
        assert_eq!(
            (back.grid_x, back.grid_y, back.team, back.flags),
            (314, 246, 1, 2)
        );
        assert_eq!(back.level(), Some(10));
        assert_eq!(back.actor.sets[0].name, "UnarmedTown");
        assert_eq!(back.life.life.maximum, 120);
        // The skill container follows the three animation sets and is the solo encoding.
        let solo = entity::initial_state_blob(
            0,
            &[Component::Actor(Actor {
                skills: vec![skill],
            })],
        );
        assert_eq!(
            back.actor.tail,
            solo[BLOB_HEADER_BYTES + ACTOR_SETS_BYTES..]
        );
    }

    #[test]
    fn the_actor_tail_matches_the_solo_actor_component() {
        // A solo Actor component is the 18 bytes of empty sets plus the tail.
        let solo = entity::initial_state_blob(0, &[Component::Actor(Actor::default())]);
        assert_eq!(solo.len(), BLOB_HEADER_BYTES + 48 + 1);
        assert!(
            solo[BLOB_HEADER_BYTES..BLOB_HEADER_BYTES + ACTOR_SETS_BYTES]
                .iter()
                .all(|&b| b == 0)
        );
    }
}
