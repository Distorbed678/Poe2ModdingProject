//! How a character is described to clients: the `AddObject` blob of its own client (entry
//! state) and the remote form other clients get (docs/protocol/entities.md, "Local vs remote
//! `AddObject` blob"). Both are built before the session joins the instance actor, which adds
//! the entity id and the current position when it sends them ([`crate::actor`]).

use poe_cluster::TicketCharacter;
use poe_data::{template::ObjectTemplate, views::CharacterClass};
use poe_proto::{
    entity::{self, Actor, Component, Life, Player, Positioned, Skill, Stats},
    player_blob::{ActorSection, AnimationSet, PlayerBlob, PlayerSection, PlayerStats},
};
use tracing::info;

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
/// Bytes of the three empty animation sets in an Actor component (the local encoding writes
/// them as zeros), then the sub-reader and the rest, which the remote blob keeps raw.
const ACTOR_SETS_BYTES: usize = 18;
/// Entity flags and link count in front of the components of `initial_state_blob`.
const BLOB_HEADER_BYTES: usize = 2;

/// What a remote blob is made of. Everything else in the blob is the official default.
pub struct RemoteInit<'a> {
    pub name: &'a str,
    /// `Characters.IntegerId`.
    pub class: u8,
    pub cell: (i32, i32),
    pub team: u16,
    /// Base stats as (network key, value), `level` among them.
    pub stats: Vec<(u32, i32)>,
    pub life: u32,
    pub mana: u32,
    /// Main hand, off hand and default animation set names.
    pub animation_sets: [String; 3],
    pub skills: Vec<Skill>,
}

/// The blob other clients get for a character: the official remote layout without the parts the
/// server has no data for (equipment visuals, links, buffs, stat sets, quest flag words).
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
            sets: Vec::new(),
            unk_1b0: STATS_TRAILER.0,
            unk_1b4: STATS_TRAILER.1,
            unk_1b5: STATS_TRAILER.2,
        },
        buffs: Vec::new(),
        life: Life::character(init.life, init.mana),
        animated_flags: ANIMATED_FLAGS,
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
            flags: 0,
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
        // official extras).
        let local_blob = entity::initial_state_blob(
            entity::ENTITY_IN_WAKE_ZONE,
            &[
                Component::Positioned(Positioned {
                    grid_x: cell.0,
                    grid_y: cell.1,
                    // Team (low 15 bits), from the template's Positioned section.
                    unk_1e0: team,
                    ..Default::default()
                }),
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
            ],
        );
        let remote = remote_blob(&RemoteInit {
            name: &character.name,
            class: class.integer_id as u8,
            cell,
            team,
            stats: network,
            life: pool("maximum_life"),
            mana: pool("maximum_mana"),
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
        }
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
