//! The player's session state in an instance: the skills the server gave the character and a
//! handle to the movement simulation.
//!
//! Movement is not computed here. The client runs in lockstep (docs/protocol/movement.md): it
//! starts a movement only when the server says so, and the server answers each input with the
//! official messages. That logic is `poe_game::world::World` / `Locomotion`; this file only
//! owns the World for now (one per session) behind [`Sim`], so that it can later move into an
//! instance actor without changing the packet mapping ([`crate::movement`]):
//!
//! - client packet -> [`crate::movement::client_input`] -> [`Player::input`]
//! - the session's frame tick -> [`Player::tick`] -> encoded messages, followed by the
//!   `SimulationTick` marker.

use std::sync::Arc;

use poe_data::views::GrantedSkill;
use poe_game::{
    grid::GridSource,
    locomotion::{MoveInput, MovementConfig as SimConfig, SkillIds},
    world::{EntityId, Event, World, WorldConfig},
};
use poe_proto::entity::Skill;

use crate::movement::{Codec, Out};

/// A skill the server gave the character.
pub struct OwnedSkill {
    pub id: u32,
    pub granted: GrantedSkill,
    pub id_name: String,
    /// The skill's stats as (network key, value), sent in its Actor entry.
    pub stats: Vec<(u32, i32)>,
}

/// Basic-action skill ids are this plus the action type key.
pub const BASIC_ACTION_SKILL: u32 = 0x2000_0000;

/// Roll length when the skill does not list `dodge_roll_base_travel_distance`.
const ROLL_DEFAULT_DISTANCE: f32 = 46.0;

/// Builds the simulation's movement configuration from game data and `[movement]`:
/// - `move_skill`: the Move basic action (`BASIC_ACTION_SKILL | ActionTypes key`);
/// - the roll skill: the owned skill whose action type is `roll_action` (DodgeRoll), with its
///   `dodge_roll_base_travel_distance`;
/// - `speed`: the class's walking speed, `sprint_bonus` its `sprint_movement_speed_+%`.
///
/// Skills that are missing get an id nothing sends (`u32::MAX`), so the component treats
/// everything as a generic action.
pub fn sim_config(
    config: &poe_config::MovementConfig,
    skills: &[OwnedSkill],
    move_skill: Option<u32>,
    roll_action: Option<u16>,
    speed: f32,
    sprint_bonus: Option<f32>,
) -> SimConfig {
    let roll = roll_action.and_then(|t| skills.iter().find(|s| s.granted.action_type == Some(t)));
    let mut cfg = SimConfig::new(
        SkillIds {
            move_skill: move_skill.unwrap_or(u32::MAX),
            roll_skill: roll.map_or(u32::MAX, |s| s.id),
        },
        config.sprint_buff_rows,
    );
    cfg.walk_speed = speed;
    cfg.roll.travel_distance = roll
        .and_then(|s| s.granted.constant_stat("dodge_roll_base_travel_distance"))
        .map_or(ROLL_DEFAULT_DISTANCE, |d| d as f32);
    if let Some(bonus) = sprint_bonus {
        cfg.sprint.speed_scale = 1.0 + bonus / 100.0;
        cfg.sprint.move_speed_percent = bonus.round() as i32;
        cfg.sprint.velocity_permyriad = (bonus * 100.0).round() as i32;
    }
    cfg
}

/// What a [`Player`] needs to be created.
pub struct PlayerInit {
    /// Entity id of the player (the first id of its world).
    pub entity_id: u32,
    pub cell: (i32, i32),
    /// Walking speed, cells per second.
    pub speed: f32,
    pub skills: Vec<OwnedSkill>,
    pub movement: SimConfig,
    /// Milliseconds per simulation frame.
    pub tick_ms: u32,
    pub grid: Arc<dyn GridSource>,
    pub codec: Codec,
}

/// The session's handle to the movement simulation. Today it owns a `World` with the one player
/// in it; an instance actor would own the World and this would send it the inputs and receive
/// this entity's events.
struct Sim {
    world: World,
    entity: EntityId,
}

pub struct Player {
    pub entity_id: u32,
    pub skills: Vec<OwnedSkill>,
    sim: Sim,
    codec: Codec,
}

impl Player {
    pub fn new(init: PlayerInit) -> Self {
        let mut config = WorldConfig::new(init.movement);
        config.tick_ms = init.tick_ms;
        config.first_entity_id = init.entity_id;
        let mut world = World::new(config, init.grid);
        let entity = world.spawn_player_with_speed(init.cell, init.speed);
        Self {
            entity_id: entity.0,
            skills: init.skills,
            sim: Sim { world, entity },
            codec: init.codec,
        }
    }

    /// The skills to list in the player entity's Actor component.
    pub fn actor_skills(&self) -> Vec<Skill> {
        self.skills
            .iter()
            .map(|s| Skill {
                id: s.id,
                effect_level_row: s.granted.effect_level_row,
                stat_set_rows: s.granted.stat_set_rows.clone(),
                stats: s.stats.clone(),
            })
            .collect()
    }

    /// Client input; answered by the next [`tick`](Self::tick).
    pub fn input(&mut self, input: MoveInput) {
        self.sim.world.input(self.sim.entity, input);
    }

    /// One simulation frame: the messages for the client, in order (the caller sends the
    /// `SimulationTick` marker after them).
    pub fn tick(&mut self) -> anyhow::Result<Vec<Out>> {
        let mut out = Vec::new();
        for e in self.sim.world.tick() {
            if e.entity != self.sim.entity {
                continue;
            }
            let Event::Movement(m) = &e.event else {
                continue;
            };
            out.extend(self.codec.encode(self.entity_id, m)?);
        }
        Ok(out)
    }

    /// Escape menu in a solo game: nothing moves or is answered while paused.
    pub fn set_paused(&mut self, paused: bool) {
        self.sim.world.set_paused(paused);
    }

    /// Dev command: put the player on `cell` (reported by the next tick).
    pub fn teleport(&mut self, cell: (i32, i32)) {
        self.sim.world.teleport(self.sim.entity, cell);
    }

    /// The grid cell the server has the player in.
    pub fn cell(&self) -> (i32, i32) {
        self.loco().cell()
    }

    /// The server's exact position in cells.
    pub fn position(&self) -> (f32, f32) {
        self.loco().position()
    }

    pub fn state(&self) -> &poe_game::locomotion::MoveState {
        self.loco().state()
    }

    /// Frames ticked so far.
    pub fn frame(&self) -> u64 {
        self.sim.world.frame()
    }

    fn loco(&self) -> &poe_game::locomotion::Locomotion {
        &self
            .sim
            .world
            .entity(self.sim.entity)
            .expect("the player entity lives as long as the session")
            .movement
    }

    /// The simulation state for the client DLL's debug link, as `key\tvalue` lines.
    pub fn debug_state(&self, report: Option<&crate::sync::ClientReport>) -> String {
        let l = self.loco();
        let (x, y) = l.position();
        let mut s = String::new();
        let mut kv = |k: &str, v: String| {
            s.push_str(k);
            s.push('\t');
            s.push_str(&v);
            s.push('\n');
        };
        kv("frame", self.frame().to_string());
        kv("server_pos", format!("{x:.3} {y:.3}"));
        kv("state", format!("{:?}", l.state()));
        kv("wasd", format!("{} {}", l.wasd().0, l.wasd().1));
        kv("sprint_held", l.sprint_held().to_string());
        kv("combat_stance", l.in_combat_stance().to_string());
        kv("paused", self.sim.world.paused().to_string());
        kv(
            "skills",
            self.skills
                .iter()
                .map(|s| format!("{}={:#x}", s.id_name, s.id))
                .collect::<Vec<_>>()
                .join(" "),
        );
        if let Some(r) = report {
            kv("report_frame", r.frame.to_string());
            kv("report_pos", format!("{:.3} {:.3}", r.x, r.y));
            kv(
                "report_action",
                format!("{:#06x} {:#06x}", r.action, r.action2),
            );
            kv(
                "drift_cells",
                format!("{:.2}", crate::sync::distance((x, y), (r.x, r.y))),
            );
        }
        s
    }
}
