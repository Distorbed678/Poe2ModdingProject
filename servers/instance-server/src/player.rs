//! The session's side of its player: the skills the server gave the character and a handle to
//! the instance actor that owns its entity.
//!
//! Movement is not computed here. The client runs in lockstep (docs/protocol/movement.md): it
//! starts a movement only when the server says so, and the server answers each input with the
//! official messages. That logic is `poe_game::world::World` / `Locomotion`, owned by the
//! instance's actor ([`crate::actor`]) together with every other player of the instance:
//!
//! - client packet -> [`crate::movement::client_input`] -> [`Player::input`] -> actor
//! - actor frame -> [`crate::actor::Frame`] -> the session writes it -> [`Player::apply`] keeps
//!   the latest [`Own`] pose for `cell`/`position`/`debug_state`.
//!
//! Dropping a `Player` leaves the instance (so every way a session can end removes the entity
//! and tells the others); [`Player::leave`] does the same and returns the final cell.

use poe_data::views::GrantedSkill;
use poe_game::locomotion::{MoveInput, MovementConfig as SimConfig, SkillIds};
use poe_proto::entity::Skill;
use tokio::sync::oneshot;

use crate::actor::{ActorHandle, Cmd, Own, SessionId};

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
    cfg.sprint.coast_start_scale = config.sprint_coast_start_scale;
    cfg.sprint.coast_frames = config.sprint_coast_frames;
    cfg.sprint.coast_delay_frames = config.sprint_coast_delay_frames;
    cfg.sprint.restart_delay_frames = config.sprint_restart_delay_frames;
    cfg.sprint.release_window_frames = config.sprint_release_window_frames;
    cfg.sprint.cursor_match_degrees = config.sprint_cursor_match_degrees;
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

/// The session's handle to its entity in the instance actor.
pub struct Player {
    pub entity_id: u32,
    pub skills: Vec<OwnedSkill>,
    session: SessionId,
    actor: ActorHandle,
    own: Own,
    left: bool,
}

impl Player {
    /// `own` is the pose the actor reported with the join.
    pub fn new(
        session: SessionId,
        actor: ActorHandle,
        entity_id: u32,
        skills: Vec<OwnedSkill>,
        own: Own,
    ) -> Self {
        Self {
            entity_id,
            skills,
            session,
            actor,
            own,
            left: false,
        }
    }

    /// The skills to list in the player entity's Actor component.
    pub fn actor_skills(skills: &[OwnedSkill]) -> Vec<Skill> {
        skills
            .iter()
            .map(|s| Skill {
                id: s.id,
                effect_level_row: s.granted.effect_level_row,
                stat_set_rows: s.granted.stat_set_rows.clone(),
                stats: s.stats.clone(),
            })
            .collect()
    }

    /// Client input; answered by the actor's next tick.
    pub fn input(&self, input: MoveInput) {
        self.actor.send(Cmd::Input {
            session: self.session,
            input,
        });
    }

    /// The escape menu: `true` when the world is now paused (or running again after
    /// `paused = false`), `false` when the request was refused (other players in the instance;
    /// see [`crate::actor::Actor::set_paused`]) or the actor is gone.
    pub async fn set_paused(&self, paused: bool) -> bool {
        let (reply, answer) = oneshot::channel();
        let sent = self.actor.send(Cmd::SetPaused {
            session: self.session,
            paused,
            reply,
        });
        sent && answer.await.unwrap_or(false)
    }

    /// Dev command: put the player on `cell` (reported by the next frame).
    pub fn teleport(&self, cell: (i32, i32)) {
        self.actor.send(Cmd::Teleport {
            session: self.session,
            cell,
        });
    }

    /// Takes the pose of the latest frame.
    pub fn apply(&mut self, own: Own) {
        self.own = own;
    }

    /// The grid cell of the latest frame.
    pub fn cell(&self) -> (i32, i32) {
        self.own.cell
    }

    /// The server's exact position in cells at the latest frame.
    pub fn position(&self) -> (f32, f32) {
        self.own.position
    }

    /// `Debug` of the movement state at the latest frame (empty without `want_debug`).
    pub fn state(&self) -> &str {
        self.own.debug.as_ref().map_or("", |d| d.state.as_str())
    }

    /// Leaves the instance and returns the cell the entity was in (the latest frame's when the
    /// actor is gone).
    pub async fn leave(mut self) -> (i32, i32) {
        self.left = true;
        let (reply, cell) = oneshot::channel();
        let sent = self.actor.send(Cmd::Leave {
            session: self.session,
            reply: Some(reply),
        });
        if sent && let Ok(Some(cell)) = cell.await {
            return cell;
        }
        self.own.cell
    }

    /// The simulation state for the client DLL's debug link, as `key\tvalue` lines.
    pub fn debug_state(&self, report: Option<&crate::sync::ClientReport>) -> String {
        let (x, y) = self.own.position;
        let mut s = String::new();
        let mut kv = |k: &str, v: String| {
            s.push_str(k);
            s.push('\t');
            s.push_str(&v);
            s.push('\n');
        };
        kv("frame", self.own.frame.to_string());
        kv("server_pos", format!("{x:.3} {y:.3}"));
        if let Some(d) = &self.own.debug {
            kv("state", d.state.clone());
            kv("wasd", format!("{} {}", d.wasd.0, d.wasd.1));
            kv("sprint_held", d.sprint_held.to_string());
            kv("combat_stance", d.combat_stance.to_string());
            kv("paused", d.paused.to_string());
        }
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

impl Drop for Player {
    fn drop(&mut self) {
        if !self.left {
            self.actor.send(Cmd::Leave {
                session: self.session,
                reply: None,
            });
        }
    }
}
