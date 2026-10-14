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

#[cfg(test)]
mod tests {
    use super::*;

    fn player() -> Player {
        let roll = OwnedSkill {
            id: 1,
            granted: GrantedSkill {
                effect: 25,
                effect_level_row: 899,
                stat_set_rows: vec![905],
                action_type: Some(60748),
                constant_stats: vec![("dodge_roll_base_travel_distance".into(), 46)],
                implicit_stats: Vec::new(),
            },
            id_name: "DodgeRollPlayer".into(),
            stats: Vec::new(),
        };
        Player::new(1, (100, 100), 37.0, None, vec![roll], Some(0x2000_2909))
    }

    fn use_skill(skill: u32, x: i32, y: i32) -> UseSkill {
        UseSkill {
            skill,
            unk_13: 0xff,
            x,
            y,
            ..Default::default()
        }
    }

    #[test]
    fn click_to_move_targets_relative_cells() {
        let mut p = player();
        let a = p.on_use_skill(&use_skill(0x2000_2909, 10, -5)).unwrap();
        assert_eq!((a.grid_x, a.grid_y), (100, 100));
        assert_eq!((a.target_x, a.target_y), (110, 95));
        assert_eq!((a.action_id, a.unk_3c), (1, 0xff));
        for _ in 0..30 {
            p.tick(0.033);
        }
        assert_eq!(p.mover.cell(), (110, 95));
        assert!(!p.mover.is_moving());
        // Action ids increase.
        assert_eq!(
            p.on_use_skill(&use_skill(0x2000_2909, 1, 0))
                .unwrap()
                .action_id,
            2
        );
    }

    #[test]
    fn unknown_skills_are_ignored() {
        assert!(player().on_use_skill(&use_skill(0x1234, 1, 1)).is_none());
    }

    #[test]
    fn roll_uses_the_travel_distance_and_pauses() {
        let mut p = player();
        p.on_use_skill(&use_skill(1, 0, 30)).unwrap();
        assert!(matches!(p.mover.motion, Motion::Roll { remaining, .. } if remaining == 46.0));
        p.paused = true;
        p.tick(0.033);
        assert_eq!(p.mover.cell(), (100, 100));
        p.paused = false;
        while p.mover.is_moving() {
            p.tick(0.033);
        }
        assert_eq!(p.mover.cell(), (100, 146));
    }

    #[test]
    fn wasd_echoes_the_direction_with_the_cell() {
        let mut p = player();
        let r = p.on_move_direction(&MoveDirection {
            x: 180,
            ..Default::default()
        });
        assert_eq!((r.dir_x, r.dir_y, r.grid_x, r.grid_y), (180, 0, 100, 100));
        for _ in 0..10 {
            p.tick(0.033);
        }
        let r = p.on_move_direction(&MoveDirection::default());
        assert_eq!((r.dir_x, r.grid_y), (0, 100));
        assert!(r.grid_x > 110);
        assert!(!p.mover.is_moving());
    }
}
