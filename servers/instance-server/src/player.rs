//! The player's simulated state in an instance: movement (WASD, click-to-move, dodge roll,
//! sprint) and the skills the server gave the character.
//!
//! The client runs in lockstep (Documentation/protocol/movement.md): it starts a movement only
//! when the server says so, then moves the entity itself every frame. The server answers each
//! input with the message that starts the movement on the client and simulates the same
//! movement here, frame by frame, so it knows where the player is.

use poe_data::views::GrantedSkill;
use poe_game::movement::{Motion, Mover, Open};
use poe_proto::{
    entity::Skill,
    instance::{
        MoveDirection, MoveDirectionUpdate, SimulationHeader, SprintInput, SprintState,
        StartAction, UpdateActionTarget, UpdateSkillTarget, UseSkill,
    },
};
use tracing::{debug, info};

/// A skill the server gave the character.
pub struct OwnedSkill {
    pub id: u32,
    pub granted: GrantedSkill,
    pub id_name: String,
    /// The skill's stats as (network key, value), sent in its Actor entry.
    pub stats: Vec<(u32, i32)>,
}

/// A message for the client, produced by input or by the simulation.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Direction(MoveDirectionUpdate),
    Start(StartAction),
    Sprint(SprintState),
}

/// The action the player is running (started by a StartAction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Current {
    action_id: u16,
    skill: u32,
}

/// Input that arrived during a dodge roll: it starts when the roll ends.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pending {
    Direction(MoveDirection),
    Skill(UseSkill),
}

pub struct Player {
    pub entity_id: u32,
    pub mover: Mover,
    pub skills: Vec<OwnedSkill>,
    /// Skill id of the basic Move action (`0x20000000 | ActionTypes key`).
    pub move_skill: Option<u32>,
    /// While paused the simulation does not advance.
    pub paused: bool,
    /// Sprint speed relative to walking (`sprint_movement_speed_+%`).
    pub sprint_scale: f32,
    next_action: u16,
    current: Option<Current>,
    pending: Option<Pending>,
    /// The WASD direction currently held.
    wasd: (i32, i32),
    /// Held click-to-move: the walk direction last sent, and the last cursor offset.
    held_dir: Option<(i32, i32)>,
    held_target: Option<(i32, i32)>,
    /// The sprint key (Space) is held.
    sprint_held: bool,
    /// The client's latest position report (movement dev bridge, [`crate::sync`]).
    client: Option<crate::sync::ClientReport>,
    /// Reports have arrived in this session: from then on the client's last report is the
    /// player's state, also while reports pause.
    synced: bool,
    /// The last call had a fresh report (for logging).
    fresh: bool,
    /// The client's movement per frame, from its last two reports.
    velocity: (f32, f32),
    /// Client frame of the older of those two reports.
    velocity_from: u32,
    /// The last walk direction sent: until reports show the walk, the client's position is
    /// predicted from it (the reports around a placement do not show the walking speed).
    last_turn: Option<Turn>,
    /// The frame in which a message sent now applies on the client (set by the session:
    /// the tick being built, or the next one for input answered between ticks).
    pub apply_frame: i64,
    pub turn_cell: TurnCell,
    /// Length of a simulation frame in seconds (`tick_ms`).
    pub frame_seconds: f32,
    /// The client's walkability grid, only to keep [`Player::send_cell`] off unwalkable cells
    /// (the server has no collision until M4).
    pub walkable: Option<crate::spawn::Grid>,
    /// Server frames sent minus the client's frame number when its report arrives (the
    /// smallest seen; 0 so far).
    frame_offset: Option<i64>,
    /// A dodge roll is running (from its StartAction until it ends).
    in_roll: bool,
    /// The client has reported the roll action since it started.
    roll_seen: bool,
    roll_frames: u32,
    /// Direction of the running roll (relative cells).
    roll_dir: (f32, f32),
}

/// Roll length when the skill does not list `dodge_roll_base_travel_distance`.
const ROLL_DEFAULT_DISTANCE: f32 = 46.0;
/// UseSkill `target_flags` bit 4: the input is held and the use repeats (about every 70 ms,
/// e.g. a held click-to-move).
const TARGET_REPEAT: u8 = 0x10;
/// Which cell a turn is sent with (`[instance] turn_cell`, see [`Player::send_cell`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnCell {
    /// The client's own cell when the turn applies: it only re-aims its walk.
    Own,
    /// A cell one normal step along the new heading: the client is put on its centre.
    Heading,
    /// The cell the client would reach in that frame.
    #[default]
    Natural,
}

impl TurnCell {
    pub fn parse(s: &str) -> Self {
        match s {
            "heading" => Self::Heading,
            "own" => Self::Own,
            _ => Self::Natural,
        }
    }
}

/// A walk direction sent to the client: at the end of `frame` (server frame numbers) the
/// client is at `at` (the centre of the cell sent) and walks on along `dir` (unit, or zero
/// for a stop) at walking speed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Turn {
    frame: i64,
    at: (f32, f32),
    dir: (f32, f32),
}

/// Most frames a position is predicted ahead (beyond that the report is too old to trust).
const MAX_LEAD_FRAMES: i64 = 4;
/// Length of a WASD direction vector (W = (127, 127), D = (180, 0)).
const WASD_LENGTH: f32 = 180.0;
/// A held click closer than this (|dx| + |dy| cells) to the player stops the walk.
const HELD_STOP_CELLS: i32 = 3;

/// Whether two walk directions are within about 8 degrees.
fn same_direction(a: (i32, i32), b: (i32, i32)) -> bool {
    let dot = (a.0 * b.0 + a.1 * b.1) as f32;
    let len = ((a.0 * a.0 + a.1 * a.1) as f32).sqrt() * ((b.0 * b.0 + b.1 * b.1) as f32).sqrt();
    len > 0.0 && dot / len > 0.99
}
/// Basic-action skill ids are this plus the action type key.
pub const BASIC_ACTION_SKILL: u32 = 0x2000_0000;
/// Simulation header order key: apply after all entities have updated in the frame (used for
/// actions that start when a roll ends during that frame).
const AFTER_ENTITIES: u32 = 0xffff_ffff;

impl Player {
    pub fn new(
        entity_id: u32,
        cell: (i32, i32),
        speed: f32,
        skills: Vec<OwnedSkill>,
        move_skill: Option<u32>,
    ) -> Self {
        Self {
            entity_id,
            mover: Mover::at_cell(cell, speed),
            skills,
            move_skill,
            paused: false,
            sprint_scale: 1.5,
            next_action: 0,
            current: None,
            pending: None,
            wasd: (0, 0),
            held_dir: None,
            held_target: None,
            sprint_held: false,
            client: None,
            synced: false,
            fresh: false,
            velocity: (0.0, 0.0),
            velocity_from: 0,
            last_turn: None,
            apply_frame: 0,
            frame_seconds: 0.033,
            turn_cell: TurnCell::Heading,
            walkable: None,
            frame_offset: None,
            in_roll: false,
            roll_seen: false,
            roll_frames: 0,
            roll_dir: (0.0, 0.0),
        }
    }

    /// The client's latest report (or `None` without fresh reports): the server takes the
    /// client's position, so positions it sends match the client's.
    /// Notes that the client's report for its latest frame arrived after `sent` server frames
    /// (lockstep: the client cannot play a frame before it was sent).
    pub fn observe_frames(&mut self, sent: i64) {
        if let Some(r) = self.client.filter(|_| self.fresh) {
            let offset = sent - r.frame as i64;
            if self.frame_offset.is_none_or(|o| offset < o) {
                self.frame_offset = Some(offset);
            }
        }
    }

    pub fn apply_report(&mut self, report: Option<crate::sync::ClientReport>) {
        if report.is_some() != self.fresh {
            debug!(cell = ?self.mover.cell(), "client reports {}", if report.is_some() { "arriving" } else { "paused" });
            self.fresh = report.is_some();
        }
        let Some(r) = report else {
            // Keep the last report once synced: the client is where it last said.
            if !self.synced {
                self.client = None;
            }
            return;
        };
        let changed = self
            .client
            .is_none_or(|c| (c.action, c.action2, c.flags) != (r.action, r.action2, r.flags));
        if changed {
            debug!(
                action = format_args!("{:#06x}", r.action),
                action2 = format_args!("{:#06x}", r.action2),
                flags = format_args!("{:#x}", r.flags),
                cell = ?r.cell(),
                "client action"
            );
        }
        self.velocity = match self.client {
            Some(c) if r.frame > c.frame && r.frame - c.frame <= 4 => {
                let n = (r.frame - c.frame) as f32;
                self.velocity_from = c.frame;
                ((r.x - c.x) / n, (r.y - c.y) / n)
            }
            Some(c) if r.frame == c.frame => self.velocity,
            _ => (0.0, 0.0),
        };
        self.mover.x = r.x;
        self.mover.y = r.y;
        self.synced = true;
        self.client = Some(r);
    }

    /// Client position reports have arrived in this session.
    pub fn synced(&self) -> bool {
        self.synced
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

    fn rolling(&self) -> bool {
        self.in_roll
    }

    /// `ActionTypes` key of the roll skill's action (DodgeRoll).
    fn roll_action(&self) -> Option<u16> {
        self.current
            .and_then(|c| self.skills.iter().find(|s| s.id == c.skill))
            .and_then(|s| s.granted.action_type)
    }

    /// Whether the roll is over: with client reports, when the client's roll action ended
    /// (or never showed up); without, when the server's simulated roll ended.
    fn roll_over(&mut self) -> bool {
        self.roll_frames += 1;
        match (self.client, self.roll_action()) {
            (Some(c), Some(t)) => {
                let client_rolling = c.action == t || c.action2 == t;
                if client_rolling {
                    self.roll_seen = true;
                }
                (self.roll_seen && !client_rolling) || (!self.roll_seen && self.roll_frames > 15)
            }
            _ => !matches!(self.mover.motion, Motion::Roll { .. }),
        }
    }

    fn sprinting(&self) -> bool {
        matches!(self.mover.motion, Motion::Sprint { .. })
    }

    /// One simulation frame; returns messages that start in this frame (actions waiting for a
    /// roll to end).
    pub fn tick(&mut self, dt: f32) -> Vec<Reply> {
        if self.paused {
            return Vec::new();
        }
        let was = self.mover.motion;
        // No collision until the server has the terrain (M4): the simulation moves in straight
        // lines. Without terrain it would walk through walls, so once reports have arrived the
        // player is only ever where the client last reported it.
        if !self.synced {
            self.mover.advance(dt, &Open);
        }
        let mut out = Vec::new();
        if self.in_roll {
            if self.roll_over() {
                self.in_roll = false;
                if matches!(self.mover.motion, Motion::Roll { .. }) {
                    self.mover.start(Motion::Idle);
                }
                // Holding the key turns the roll into a sprint (the client decides the same,
                // `0x141b76c20`). With client reports the client moves itself; the server
                // simulates the sprint only without them.
                if self.sprint_held && self.client.is_none() {
                    debug!(cell = ?self.mover.cell(), "roll ended, sprinting");
                    let (sx, sy) = if self.wasd != (0, 0) {
                        (self.wasd.0 as f32, self.wasd.1 as f32)
                    } else {
                        self.roll_dir
                    };
                    self.mover.start(Motion::Sprint {
                        x: sx,
                        y: sy,
                        scale: self.sprint_scale,
                    });
                } else {
                    debug!(cell = ?self.mover.cell(), "roll ended");
                    self.current = None;
                    // With reports the client's roll already ended in an earlier frame, so the
                    // next action applies at the start of the frame; otherwise after the roll's
                    // last update in this frame.
                    let order = if self.client.is_some() {
                        0
                    } else {
                        AFTER_ENTITIES
                    };
                    out.extend(self.resume(order));
                }
            }
        } else if was != Motion::Idle && self.mover.motion == Motion::Idle {
            debug!(cell = ?self.mover.cell(), "movement ended");
            self.current = None;
        }
        out
    }

    /// Starts what was waiting for a roll to end, or the held WASD direction. With client
    /// reports, a held WASD direction is not sent again: the client resumes walking by itself
    /// after a roll (and sprints if the key is held), and only the server's simulation needs
    /// to follow.
    fn resume(&mut self, order: u32) -> Vec<Reply> {
        let wasd = (self.wasd != (0, 0)).then_some(MoveDirection {
            x: self.wasd.0,
            y: self.wasd.1,
            ..Default::default()
        });
        if self.client.is_some() {
            match self.pending.take() {
                Some(Pending::Skill(req)) => {
                    if let Some(mut a) = self.start_skill(&req) {
                        a.header.unk_14 = order;
                        return vec![Reply::Start(a)];
                    }
                }
                _ => {
                    if let Some(req) = wasd {
                        self.start_direction(&req);
                    }
                }
            }
            return Vec::new();
        }
        let pending = self.pending.take().or(wasd.map(Pending::Direction));
        let mut out = Vec::new();
        match pending {
            Some(Pending::Direction(req)) => {
                let mut r = self.start_direction(&req);
                r.header.unk_14 = order;
                out.push(Reply::Direction(r));
            }
            Some(Pending::Skill(req)) => {
                if let Some(mut a) = self.start_skill(&req) {
                    a.header.unk_14 = order;
                    out.push(Reply::Start(a));
                }
            }
            None => {}
        }
        out
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

    /// WASD: the direction, echoed with the server's position of the player. During a roll it
    /// waits for the roll to end; during a sprint it steers the sprint.
    pub fn on_move_direction(&mut self, req: &MoveDirection) -> Option<MoveDirectionUpdate> {
        self.wasd = (req.x, req.y);
        if self.rolling() {
            if self.client.is_some() {
                // The client keeps WASD input during a roll and walks (or sprints) on when the
                // roll ends, so it gets the direction now, with its own cell.
                let (grid_x, grid_y) = self.send_cell(Some((req.x as f32, req.y as f32)));
                return Some(MoveDirectionUpdate {
                    header: self.header(),
                    dir_x: req.x,
                    dir_y: req.y,
                    grid_x,
                    grid_y,
                    unk_30: false,
                });
            }
            self.pending = Some(Pending::Direction(*req));
            return None;
        }
        if self.sprinting() {
            if req.x != 0 || req.y != 0 {
                self.mover.motion = Motion::Sprint {
                    x: req.x as f32,
                    y: req.y as f32,
                    scale: self.sprint_scale,
                };
            }
            return None;
        }
        Some(self.start_direction(req))
    }

    /// The player's cell for a message that starts a movement heading `dir`.
    ///
    /// In the frame the message applies (`apply_frame`), the client puts the player on the
    /// centre of the given cell (`0x142149640` / `0x141e41ae0`: a cell other than its own is set
    /// as its position), then walks on from there in the next frame. So that frame's step is
    /// from where the client is when the frame begins to that centre. The cell is chosen to make
    /// it look like a normal step: about the client's speed long and along the new heading.
    /// A cell from the last report pulled the player back at every turn; the nearest cell made
    /// steps up to 0.7 cells too long or short.
    fn send_cell(&self, dir: Option<(f32, f32)>) -> (i32, i32) {
        if self.client.is_none() {
            return self.mover.cell();
        }
        // Where the client is when the frame begins, and where it would be at its end.
        let (start, (vx, vy)) = self.predict(self.apply_frame - 1);
        let natural = ((start.0 + vx).floor() as i32, (start.1 + vy).floor() as i32);
        match self.turn_cell {
            TurnCell::Own => return (start.0.floor() as i32, start.1.floor() as i32),
            TurnCell::Natural => return natural,
            TurnCell::Heading => {}
        }
        // Standing still, the first frame would only go to the centre of the player's own cell
        // (a dead frame): step to a neighbouring cell at walking speed instead.
        let speed = match vx.hypot(vy) {
            s if s > 0.3 => s,
            _ => self.mover.speed * self.frame_seconds,
        };
        let Some((dx, dy)) = dir.filter(|d| d.0.hypot(d.1) > 0.0) else {
            return natural;
        };
        let len = dx.hypot(dy);
        let (dx, dy) = (dx / len, dy / len);
        let cost = |(cx, cy): (i32, i32)| {
            let (sx, sy) = (cx as f32 + 0.5 - start.0, cy as f32 + 0.5 - start.1);
            let step = sx.hypot(sy);
            if step < 1e-3 {
                return f32::MAX;
            }
            (step - speed).abs() + speed * (1.0 - (sx * dx + sy * dy) / step)
        };
        let (bx, by) = (start.0.floor() as i32, start.1.floor() as i32);
        let mut best = (natural, cost(natural));
        for cy in by - 2..=by + 2 {
            for cx in bx - 2..=bx + 2 {
                let walkable = self
                    .walkable
                    .as_ref()
                    .is_none_or(|g| poe_game::movement::Walkable::walkable(g, cx, cy));
                let c = cost((cx, cy));
                if walkable && c < best.1 {
                    best = ((cx, cy), c);
                }
            }
        }
        best.0
    }

    /// The client's position at the end of server frame `f` and its velocity then: from the
    /// last walk direction sent while the reports do not yet show it, else from the latest
    /// report and the measured velocity.
    fn predict(&self, f: i64) -> ((f32, f32), (f32, f32)) {
        let Some(r) = self.client else {
            return ((self.mover.x, self.mover.y), (0.0, 0.0));
        };
        let offset = self.frame_offset.unwrap_or(0);
        let report_frame = r.frame as i64 + offset;
        if let Some(t) = self.last_turn
            && f >= t.frame
            && (self.velocity_from as i64 + offset) < t.frame
        {
            let walk = self.mover.speed * self.frame_seconds;
            let v = (t.dir.0 * walk, t.dir.1 * walk);
            let n = (f - t.frame) as f32;
            return ((t.at.0 + v.0 * n, t.at.1 + v.1 * n), v);
        }
        let n = (f - report_frame).clamp(0, MAX_LEAD_FRAMES) as f32;
        let v = self.velocity;
        ((self.mover.x + v.0 * n, self.mover.y + v.1 * n), v)
    }

    fn start_direction(&mut self, req: &MoveDirection) -> MoveDirectionUpdate {
        self.current = None;
        self.mover.start(if req.x == 0 && req.y == 0 {
            Motion::Idle
        } else {
            Motion::Direction {
                x: req.x as f32,
                y: req.y as f32,
            }
        });
        let (grid_x, grid_y) = self.send_cell(Some((req.x as f32, req.y as f32)));
        debug!(dir = ?(req.x, req.y), cell = ?(grid_x, grid_y), "move direction");
        if self.client.is_some() {
            let len = (req.x as f32).hypot(req.y as f32);
            self.last_turn = Some(if len > 0.0 {
                Turn {
                    frame: self.apply_frame,
                    at: (grid_x as f32 + 0.5, grid_y as f32 + 0.5),
                    dir: (req.x as f32 / len, req.y as f32 / len),
                }
            } else {
                // A stop has no position: the client halts about where it is.
                let (at, _) = self.predict(self.apply_frame);
                Turn {
                    frame: self.apply_frame,
                    at,
                    dir: (0.0, 0.0),
                }
            });
        }
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
    /// Returns the action to start on the client; `None` if the skill is not known or the use
    /// waits for a roll to end.
    /// A skill use from the client. A held click-to-move repeats the Move use about every
    /// 70 ms (target flag `0x10`). The client re-aims only the action in its first slot with
    /// `0x1cb`, not a Move, and a new StartAction restarts the Move and costs most of a
    /// frame's step. So while the button is held the player walks like WASD (`0x64` toward
    /// the cursor, sent when the direction changes), and on release the Move goes to the last
    /// cursor point ([`Player::on_skill_released`]).
    pub fn on_skill_input(&mut self, req: &UseSkill) -> Vec<Reply> {
        let repeat = req.target_flags & TARGET_REPEAT != 0;
        if repeat && Some(req.skill) == self.move_skill && !self.rolling() {
            self.held_target = Some((req.x, req.y));
            // The cursor on the player: stop walking.
            if req.x.abs() + req.y.abs() < HELD_STOP_CELLS {
                return match self.held_dir.take() {
                    Some(_) => vec![Reply::Direction(
                        self.start_direction(&MoveDirection::default()),
                    )],
                    None => Vec::new(),
                };
            }
            let len = ((req.x * req.x + req.y * req.y) as f32).sqrt();
            let dir = (
                (req.x as f32 * WASD_LENGTH / len).round() as i32,
                (req.y as f32 * WASD_LENGTH / len).round() as i32,
            );
            let client_idle = self.client.is_some_and(|c| c.action == 0 && c.action2 == 0);
            if let Some(prev) = self.held_dir
                && !client_idle
                && same_direction(prev, dir)
            {
                return Vec::new();
            }
            self.held_dir = Some(dir);
            let walk = MoveDirection {
                x: dir.0,
                y: dir.1,
                ..Default::default()
            };
            return vec![Reply::Direction(self.start_direction(&walk))];
        }
        self.held_dir = None;
        self.held_target = None;
        self.on_use_skill(req)
            .map(Reply::Start)
            .into_iter()
            .collect()
    }

    /// The skill's input was released: a held click-to-move stops walking and goes to the last
    /// cursor point (the stop also clears the client's walk direction, which it would
    /// otherwise resume after the Move).
    pub fn on_skill_released(&mut self, skill: u32) -> Vec<Reply> {
        let target = self.held_target.take();
        if Some(skill) != self.move_skill || self.held_dir.take().is_none() {
            return Vec::new();
        }
        let mut out = vec![Reply::Direction(
            self.start_direction(&MoveDirection::default()),
        )];
        if let Some((x, y)) = target
            && x.abs() + y.abs() >= HELD_STOP_CELLS
        {
            let release = UseSkill {
                skill,
                unk_13: 0xff,
                x,
                y,
                ..Default::default()
            };
            out.extend(self.start_skill(&release).map(Reply::Start));
        }
        out
    }

    pub fn on_use_skill(&mut self, req: &UseSkill) -> Option<StartAction> {
        if self.rolling() {
            // Moves wait for the roll to end (the latest one wins); a second roll is ignored.
            if Some(req.skill) == self.move_skill {
                self.pending = Some(Pending::Skill(*req));
            }
            return None;
        }
        self.start_skill(req)
    }

    fn start_skill(&mut self, req: &UseSkill) -> Option<StartAction> {
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
            self.in_roll = dx != 0.0 || dy != 0.0;
            self.roll_seen = false;
            self.roll_frames = 0;
            self.roll_dir = (dx, dy);
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
            self.pending = None;
        } else {
            return None;
        }
        let action_id = self.next_action_id();
        self.current = Some(Current {
            action_id,
            skill: req.skill,
        });
        let (cx, cy) = self.send_cell(Some(((tx - cx) as f32, (ty - cy) as f32)));
        // Actions (click-to-move, rolls) move differently: predict from reports again.
        self.last_turn = None;
        Some(StartAction {
            header: self.header(),
            flags: 0,
            target_entity: req.target_entity,
            grid_x: cx,
            grid_y: cy,
            target_x: tx,
            target_y: ty,
            skill: req.skill,
            action_id,
            unk_3c: req.unk_13,
        })
    }

    /// New target for the running skill (roll or sprint steering, a held click): the server
    /// moves its own target and tells the client (in lockstep it does not apply it itself).
    pub fn on_update_target(&mut self, req: &UpdateSkillTarget) -> Option<UpdateActionTarget> {
        let current = self.current.filter(|c| c.skill == req.skill)?;
        Some(self.retarget(current, req.x, req.y))
    }

    /// Re-aims the running action at a location relative to the player's cell.
    fn retarget(&mut self, current: Current, rx: i32, ry: i32) -> UpdateActionTarget {
        let (cx, cy) = self.mover.cell();
        let (tx, ty) = (cx + rx, cy + ry);
        match &mut self.mover.motion {
            Motion::Roll { x, y, .. } | Motion::Sprint { x, y, .. } if rx != 0 || ry != 0 => {
                *x = rx as f32;
                *y = ry as f32;
                self.roll_dir = (*x, *y);
            }
            Motion::MoveTo { .. } => {
                self.mover.start(Motion::MoveTo {
                    x: tx as f32 + 0.5,
                    y: ty as f32 + 0.5,
                });
            }
            _ => {}
        }
        UpdateActionTarget {
            header: self.header(),
            action_id: current.action_id,
            x: tx,
            y: ty,
        }
    }

    /// The sprint key: echoed to the client. Releasing it ends a sprint.
    pub fn on_sprint_input(&mut self, req: &SprintInput) -> Vec<Reply> {
        self.sprint_held = req.held;
        let mut out = vec![Reply::Sprint(SprintState {
            header: self.header(),
            held: req.held,
        })];
        if !req.held && self.sprinting() {
            debug!(cell = ?self.mover.cell(), "sprint ended");
            self.mover.start(Motion::Idle);
            self.current = None;
            out.extend(self.resume(0));
        }
        out
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
        Player::new(1, (100, 100), 37.0, vec![roll], Some(0x2000_2909))
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

    fn run_until_idle(p: &mut Player) -> Vec<Reply> {
        let mut out = Vec::new();
        for _ in 0..200 {
            out.extend(p.tick(0.033));
            if !p.mover.is_moving() {
                break;
            }
        }
        out
    }

    #[test]
    fn held_click_walks_like_wasd_and_ends_at_the_cursor() {
        let mut p = player();
        let first = p.on_skill_input(&use_skill(0x2000_2909, 30, 0));
        assert!(matches!(&first[..], [Reply::Start(_)]));
        let held = |x, y| UseSkill {
            target_flags: 0x10,
            ..use_skill(0x2000_2909, x, y)
        };
        // Repeats walk toward the cursor (direction scaled like WASD)...
        let r = p.on_skill_input(&held(30, 0));
        assert!(matches!(&r[..], [Reply::Direction(d)] if (d.dir_x, d.dir_y) == (180, 0)));
        // ...but only when the direction changes.
        assert!(p.on_skill_input(&held(31, 1)).is_empty());
        let r = p.on_skill_input(&held(0, 20));
        assert!(matches!(&r[..], [Reply::Direction(d)] if (d.dir_x, d.dir_y) == (0, 180)));
        // Release: stop the walk, then move to the last cursor point.
        let r = p.on_skill_released(0x2000_2909);
        assert!(matches!(&r[..],
            [Reply::Direction(d), Reply::Start(a)]
                if (d.dir_x, d.dir_y) == (0, 0) && (a.target_x, a.target_y) == (100, 120)));
        // A plain click release does nothing.
        p.on_skill_input(&use_skill(0x2000_2909, 5, 5));
        assert!(p.on_skill_released(0x2000_2909).is_empty());
    }

    #[test]
    fn click_to_move_targets_relative_cells() {
        let mut p = player();
        let a = p.on_use_skill(&use_skill(0x2000_2909, 10, -5)).unwrap();
        assert_eq!((a.grid_x, a.grid_y), (100, 100));
        assert_eq!((a.target_x, a.target_y), (110, 95));
        assert_eq!((a.action_id, a.unk_3c), (1, 0xff));
        run_until_idle(&mut p);
        assert_eq!(p.mover.cell(), (110, 95));
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
        run_until_idle(&mut p);
        assert_eq!(p.mover.cell(), (100, 146));
    }

    #[test]
    fn input_during_a_roll_waits_for_its_end() {
        let mut p = player();
        p.on_use_skill(&use_skill(1, 0, 30)).unwrap();
        p.tick(0.033);
        // A click while rolling does not interrupt the roll; it starts when the roll ends.
        assert!(p.on_use_skill(&use_skill(0x2000_2909, 20, 0)).is_none());
        let out = run_until_idle(&mut p);
        assert!(matches!(&out[..], [Reply::Start(a)]
            if a.header.unk_14 == AFTER_ENTITIES && (a.grid_x, a.grid_y) == (100, 146)
                && (a.target_x, a.target_y) == (120, 146)));
        run_until_idle(&mut p);
        assert_eq!(p.mover.cell(), (120, 146));
    }

    #[test]
    fn roll_steers_and_turns_into_a_sprint_while_held() {
        let mut p = player();
        let start = p.on_use_skill(&use_skill(1, 0, 30)).unwrap();
        let held = p.on_sprint_input(&SprintInput {
            player: 0,
            held: true,
        });
        assert!(matches!(&held[..], [Reply::Sprint(s)] if s.held));
        // Steering towards +x.
        let t = p
            .on_update_target(&UpdateSkillTarget {
                x: 30,
                y: 0,
                skill: 1,
                ..Default::default()
            })
            .unwrap();
        assert_eq!((t.action_id, t.x, t.y), (start.action_id, 130, 100));
        for _ in 0..40 {
            p.tick(0.033);
        }
        assert!(p.sprinting());
        let (x, _) = p.mover.cell();
        let out = p.on_sprint_input(&SprintInput {
            player: 0,
            held: false,
        });
        assert!(matches!(&out[..], [Reply::Sprint(s)] if !s.held));
        assert!(!p.mover.is_moving());
        assert!(x > 140);
    }

    fn report(x: f32, y: f32, action: u16) -> Option<crate::sync::ClientReport> {
        Some(crate::sync::ClientReport {
            frame: 0,
            x,
            y,
            action,
            action2: 0,
            flags: 0,
            at: std::time::Instant::now(),
        })
    }

    #[test]
    fn with_reports_the_roll_ends_with_the_clients() {
        let mut p = player();
        p.on_use_skill(&use_skill(1, 0, 30)).unwrap();
        assert!(p.on_use_skill(&use_skill(0x2000_2909, 20, 0)).is_none());
        // The client rolls for 40 frames (longer than the server's simulation) and ends up
        // elsewhere than the server's straight roll.
        for i in 0..40 {
            p.apply_report(report(100.5, 100.5 + i as f32, 0xed4c));
            assert!(p.tick(0.033).is_empty(), "frame {i}");
        }
        p.apply_report(report(103.5, 139.5, 0));
        let out = p.tick(0.033);
        // The waiting click starts at the start of the next frame, from the client's position:
        // standing still, its first step goes to the neighbouring cell toward the target.
        assert!(matches!(&out[..], [Reply::Start(a)]
            if a.header.unk_14 == 0 && (a.grid_x, a.grid_y) == (104, 139)
                && (a.target_x, a.target_y) == (123, 139)));
    }

    #[test]
    fn wasd_echoes_the_direction_with_the_cell() {
        let mut p = player();
        let r = p
            .on_move_direction(&MoveDirection {
                x: 180,
                ..Default::default()
            })
            .unwrap();
        assert_eq!((r.dir_x, r.dir_y, r.grid_x, r.grid_y), (180, 0, 100, 100));
        for _ in 0..10 {
            p.tick(0.033);
        }
        let r = p.on_move_direction(&MoveDirection::default()).unwrap();
        assert_eq!((r.dir_x, r.grid_y), (0, 100));
        assert!(r.grid_x > 110);
        assert!(!p.mover.is_moving());
    }
}
