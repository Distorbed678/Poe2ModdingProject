//! The player's simulated state in an instance: movement (WASD, click-to-move, dodge roll)
//! and the skills the server gave the character.
//!
//! The client runs in lockstep (Documentation/protocol/movement.md): it starts a movement only
//! when the server says so, then moves the entity itself every frame. The server answers each
//! input with the message that starts the movement on the client and simulates the same
//! movement here, frame by frame, so it knows where the player is.

use poe_data::views::GrantedSkill;
use poe_game::movement::{Motion, Mover, Open, Walkable};
use poe_proto::{
    entity::Skill,
    instance::{MoveDirection, MoveDirectionUpdate, SimulationHeader, StartAction, UseSkill},
};
use tracing::{debug, info};

use crate::spawn::Grid;

/// A skill the server gave the character.
pub struct OwnedSkill {
    pub id: u32,
    pub granted: GrantedSkill,
    pub id_name: String,
    /// The skill's stats as (network key, value), sent in its Actor entry.
    pub stats: Vec<(u32, i32)>,
}

pub struct Player {
    pub entity_id: u32,
    pub mover: Mover,
    /// The client's walkability grid (dev bridge until the server has terrain, M4).
    pub grid: Option<Grid>,
    pub skills: Vec<OwnedSkill>,
    /// Skill id of the basic Move action (`0x20000000 | ActionTypes key`).
    pub move_skill: Option<u32>,
    /// While paused the simulation does not advance.
    pub paused: bool,
    next_action: u16,
}

/// Roll length when the skill does not list `dodge_roll_base_travel_distance`.
const ROLL_DEFAULT_DISTANCE: f32 = 46.0;
/// Basic-action skill ids are this plus the action type key.
pub const BASIC_ACTION_SKILL: u32 = 0x2000_0000;

impl Player {
    pub fn new(
        entity_id: u32,
        cell: (i32, i32),
        speed: f32,
        grid: Option<Grid>,
        skills: Vec<OwnedSkill>,
        move_skill: Option<u32>,
    ) -> Self {
        Self {
            entity_id,
            mover: Mover::at_cell(cell, speed),
            grid,
            skills,
            move_skill,
            paused: false,
            next_action: 0,
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

    /// One simulation frame.
    pub fn tick(&mut self, dt: f32) {
        if self.paused {
            return;
        }
        let was = self.mover.motion;
        let terrain: &dyn Walkable = match &self.grid {
            Some(g) => g,
            None => &Open,
        };
        self.mover.advance(dt, terrain);
        if was != Motion::Idle && self.mover.motion == Motion::Idle {
            debug!(cell = ?self.mover.cell(), "movement ended");
        }
    }

    fn header(&self) -> SimulationHeader {
        SimulationHeader {
            entity_id: self.entity_id,
            ..Default::default()
        }
    }

    fn next_action_id(&mut self) -> u16 {
        // Server action ids are positive 15-bit values (negative ones are client predictions).
        self.next_action = self.next_action % 0x7fff + 1;
        self.next_action
    }

    /// WASD: the direction, echoed with the server's position of the player.
    pub fn on_move_direction(&mut self, req: &MoveDirection) -> MoveDirectionUpdate {
        self.mover.start(if req.x == 0 && req.y == 0 {
            Motion::Idle
        } else {
            Motion::Direction {
                x: req.x as f32,
                y: req.y as f32,
            }
        });
        let (grid_x, grid_y) = self.mover.cell();
        debug!(dir = ?(req.x, req.y), cell = ?(grid_x, grid_y), "move direction");
        MoveDirectionUpdate {
            header: self.header(),
            dir_x: req.x,
            dir_y: req.y,
            grid_x,
            grid_y,
            unk_30: false,
        }
    }

    /// A skill use: Move (click-to-move) or one of the character's skills (dodge roll).
    /// Returns the action to start on the client, or `None` if the skill is not known.
    pub fn on_use_skill(&mut self, req: &UseSkill) -> Option<StartAction> {
        let (cx, cy) = self.mover.cell();
        // Without a target entity the location is relative to the player's cell.
        let (tx, ty) = match req.target_entity {
            Some(_) => (req.x, req.y),
            None => (cx + req.x, cy + req.y),
        };
        if Some(req.skill) == self.move_skill {
            self.mover.start(Motion::MoveTo {
                x: tx as f32 + 0.5,
                y: ty as f32 + 0.5,
            });
        } else if let Some(skill) = self.skills.iter().find(|s| s.id == req.skill) {
            let distance = skill
                .granted
                .constant_stat("dodge_roll_base_travel_distance")
                .map_or(ROLL_DEFAULT_DISTANCE, |d| d as f32);
            info!(skill = %skill.id_name, target = ?(tx, ty), "skill used");
            let (dx, dy) = ((tx - cx) as f32, (ty - cy) as f32);
            self.mover.start(if dx == 0.0 && dy == 0.0 {
                Motion::Idle
            } else {
                Motion::Roll {
                    x: dx,
                    y: dy,
                    remaining: distance,
                    elapsed: 0.0,
                }
            });
        } else {
            return None;
        }
        Some(StartAction {
            header: self.header(),
            flags: 0,
            target_entity: req.target_entity,
            grid_x: cx,
            grid_y: cy,
            target_x: tx,
            target_y: ty,
            skill: req.skill,
            action_id: self.next_action_id(),
            unk_3c: req.unk_13,
        })
    }
}
