//! The movement component of a player entity: server-authoritative movement for the client's
//! lockstep mode.
//!
//! The client starts nothing on its own. It sends its input (WASD vector, skill use, sprint
//! key, aim), the server answers each input in the next frame batch and simulates the same
//! movement itself so it knows where the player is. [`Locomotion`] is that server side for one
//! entity: [`MoveInput`] in, [`MoveEvent`] out, one [`Locomotion::tick`] per frame. The
//! sequences, frame offsets and counts are the official ones of
//! `docs/protocol/movement.md`; scenarios the captures
//! do not cover are marked `UNVERIFIED (gap #n)` after the gap list there.
//!
//! No wire formats here: [`MoveEvent`] is semantic (the instance server maps it to packets),
//! and skill ids and buff rows come in through [`MovementConfig`].
//!
//! # Frame model
//! `tick` is one server frame (batch). Inside it, in this order:
//! 1. the entity moves one frame (so a movement started by a reply in frame `F` first moves in
//!    `F + 1`, as on the client),
//! 2. the queued inputs are handled in arrival order and answer directly (`r + 0`: the first
//!    tick after an input arrived),
//! 3. timeline events that are due run (roll end, sprint start/stages),
//! 4. the `MoveAck` for a direction change or the sprint stream (one per frame),
//! 5. the sprint end, if the key was released.

use crate::movement::{Motion, Mover, Walkable};

pub type Cell = (i32, i32);

/// Skill ids the component recognises (from game data; everything else is a generic action).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillIds {
    /// Click-to-move (`ActionTypes` basic action Move).
    pub move_skill: u32,
    /// Dodge roll.
    pub roll_skill: u32,
}

/// Dodge roll tuning (official values as defaults).
#[derive(Debug, Clone, PartialEq)]
pub struct RollConfig {
    /// `dodge_roll_travel_distance`, cells.
    pub travel_distance: f32,
    /// Frames the roll action lasts (aim is not echoed, a new roll waits, WASD is acted on
    /// afterwards). Also the frame a roll that became a sprint ends.
    pub action_frames: u32,
    /// Frames after the roll start at which a tap roll (key up before the sprint starts)
    /// clears `is_dodge_rolling`; the action itself goes on until `action_frames`.
    pub tap_end_frames: u32,
    /// Value of the three stun threshold stats while rolling.
    pub stun_threshold: i32,
}

/// Sprint tuning (official values as defaults).
#[derive(Debug, Clone, PartialEq)]
pub struct SprintConfig {
    /// Buff rows added when the sprint starts (indices `n`, `n+1`, `n+2`): from game data.
    pub buff_rows: [u16; 3],
    /// Fade time of the first buff when the roll ends.
    pub buff_fade_secs: f32,
    /// Frames after the roll start at which Space still held starts the sprint.
    pub start_frames: u32,
    /// Frames after the roll start at which `must_sprint` goes off.
    pub must_sprint_end_frames: u32,
    /// Walking speed multiplier while sprinting.
    pub speed_scale: f32,
    /// Direction vectors in sprint replies are the input times `.0 / .1` (integer division).
    pub dir_scale: (i32, i32),
    /// `base_move_speed_+%_from_sprint`.
    pub move_speed_percent: i32,
    /// `movement_velocity_+permyriad`.
    pub velocity_permyriad: i32,
    /// `sprint_blend_time_override_ms`.
    pub blend_time_override_ms: i32,
}

/// Everything the component needs to know about the game (shared by all entities of a world).
#[derive(Debug, Clone, PartialEq)]
pub struct MovementConfig {
    pub skills: SkillIds,
    pub roll: RollConfig,
    pub sprint: SprintConfig,
    /// Characters' walking speed, cells per second.
    pub walk_speed: f32,
    /// Frames between the client entering aim mode (`AimMode(1)`) and the stance change.
    pub stance_delay_frames: u32,
}

impl MovementConfig {
    /// The official values of the 0.5.5e captures; `buff_rows` are the three sprint buff rows
    /// from the buff definitions (game data).
    pub fn new(skills: SkillIds, buff_rows: [u16; 3]) -> Self {
        Self {
            skills,
            roll: RollConfig {
                travel_distance: 46.0,
                action_frames: 30,
                tap_end_frames: 16,
                stun_threshold: 100,
            },
            sprint: SprintConfig {
                buff_rows,
                buff_fade_secs: 0.1,
                start_frames: 15,
                must_sprint_end_frames: 35,
                speed_scale: 1.5,
                dir_scale: (550, 180),
                move_speed_percent: 50,
                velocity_permyriad: 5000,
                blend_time_override_ms: 1,
            },
            walk_speed: 37.0,
            stance_delay_frames: 9,
        }
    }
}

/// A `UseSkill` input (`0x1a5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UseSkill {
    pub skill: u32,
    pub flags: u16,
    pub unk: u8,
    pub target_flags: u8,
    /// Target entity (`target_flags & 8`); then `x`, `y` are absolute, else relative to the
    /// player's cell.
    pub entity: Option<u32>,
    pub x: i32,
    pub y: i32,
}

impl UseSkill {
    /// Bit of the target flags: this is a repeat of a held input.
    pub const HELD_REPEAT: u8 = 0x10;
}

/// What the client sends that moves the player, mirroring its packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveInput {
    /// `0x63` WASD vector (grid axes, length about 180; `(0, 0)` = stop).
    Direction { x: i32, y: i32 },
    /// `0x1a5`.
    UseSkill(UseSkill),
    /// `0x1a8`, `x`, `y` relative to the player's cell.
    UpdateSkillTarget { x: i32, y: i32, skill: u32 },
    /// `0x1ac`: a held skill input ended.
    SkillInputEnd,
    /// `0x1ad`.
    SkillReleased { skill: u32, flag: u8 },
    /// `0x2e6`: the sprint key (Space).
    SprintInput { held: bool },
    /// `0x62` mode 2: the absolute cell under the cursor.
    Aim { x: i32, y: i32 },
    /// `0x62` mode 3 (hovered entity): not echoed.
    AimEntity { entity: u32 },
    /// `0x62` mode 0/1 (once at entry; mode 1 puts the character into combat stance).
    AimMode { mode: u8 },
}

/// Stats the component changes (game stat ids; the session resolves the network keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveStat {
    IsDodgeRolling,
    StunThresholdWhilePerformingAction,
    VirtualHeavyStunThreshold,
    VirtualLightStunThreshold,
    SprintBlendTimeOverrideMs,
    IsSprinting,
    MustSprint,
    AllHitsStunYou,
    VirtualSkillIgnoreNamelockingDynamically,
    BaseMoveSpeedFromSprint,
    MovementVelocityPermyriad,
}

impl MoveStat {
    /// The stat's id in the game data (`Stats.dat` `Id`).
    pub fn id(self) -> &'static str {
        match self {
            Self::IsDodgeRolling => "is_dodge_rolling",
            Self::StunThresholdWhilePerformingAction => {
                "stun_threshold_+%_final_while_performing_action"
            }
            Self::VirtualHeavyStunThreshold => "virtual_heavy_stun_threshold_+%_final",
            Self::VirtualLightStunThreshold => "virtual_light_stun_threshold_+%_final",
            Self::SprintBlendTimeOverrideMs => "sprint_blend_time_override_ms",
            Self::IsSprinting => "is_sprinting",
            Self::MustSprint => "must_sprint",
            Self::AllHitsStunYou => "all_hits_stun_you",
            Self::VirtualSkillIgnoreNamelockingDynamically => {
                "virtual_skill_ignore_namelocking_dynamically"
            }
            Self::BaseMoveSpeedFromSprint => "base_move_speed_+%_from_sprint",
            Self::MovementVelocityPermyriad => "movement_velocity_+permyriad",
        }
    }
}

/// What kind of action a [`MoveEvent::ActionStart`] starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Move,
    Roll,
    /// Any other skill (an attack).
    Other,
}

/// What the server tells the client about this entity's movement, in the order it is sent.
/// The instance server maps each variant to its packet (the opcode in the comment names the
/// official message; see docs/protocol/movement.md).
#[derive(Debug, Clone, PartialEq)]
pub enum MoveEvent {
    /// `0x64` MoveDirectionUpdate: direction (scaled while sprinting) and the server's cell.
    MoveAck { dir: Cell, cell: Cell },
    /// `0x1c8` StartAction. `action` comes from the world-wide action counter.
    ActionStart {
        kind: ActionKind,
        skill: u32,
        action: u16,
        cell: Cell,
        target: Cell,
        entity: Option<u32>,
        unk: u8,
    },
    /// `0x1ca`, no body (with every roll start and when an attack ends a sprint).
    ActionEvent,
    /// `0x1d3` StatsUpdate (set 0).
    Stats(Vec<(MoveStat, i32)>),
    /// `0x1ae` UseSkillAck: the request echoed with the absolute target; the session sets the
    /// "confirmed" target flag (`0x04`).
    SkillAck {
        skill: u32,
        flags: u16,
        unk: u8,
        target_flags: u8,
        entity: Option<u32>,
        x: i32,
        y: i32,
    },
    /// `0x1af` UpdateSkillTargetAck: absolute target.
    TargetAck { skill: u32, x: i32, y: i32 },
    /// `0x1b1` SkillInputEndAck.
    InputEndAck,
    /// `0x1b2` SkillReleasedAck.
    ReleasedAck { skill: u32, flag: u8 },
    /// `0x2e7` SprintState: the echo of the sprint key.
    SprintState { held: bool },
    /// `0x1eb` BuffAdded; `fade` is the optional fade time (flag `0x02`).
    BuffAdded {
        index: u16,
        row: u16,
        fade: Option<f32>,
    },
    /// `0x1ec` BuffRemoved.
    BuffRemoved { index: u16 },
    /// `0x1dc` SetAimTarget kind 0: the cursor cell echoed.
    AimEcho { cell: Cell },
    /// `0x2f9` ComponentDelta of the Player component: the combat stance.
    StanceChanged { combat: bool },
    /// The entity was placed by the server (admin command, transfer). The client only snaps
    /// when the distance is 46 cells or more, and only for a `0x64` with a non-zero direction.
    Teleport { cell: Cell },
}

/// How a walk is driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walk {
    /// WASD: keep going along this vector.
    Direction(Cell),
    /// Click-to-move: walk to a cell along an A* path ([`Locomotion::path`]).
    Target { goal: Cell },
}

/// A running sprint (the buffs are added at roll start + 15 frames).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SprintRun {
    /// Frame of the roll that started it (`t0`).
    pub start_frame: u64,
    /// Buff index `n`; the sprint uses `n`, `n + 1`, `n + 2`.
    pub buff_base: u16,
    /// A non-zero WASD vector arrived after the sprint started: one `MoveAck` per frame.
    pub streaming: bool,
}

/// A dodge roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rolling {
    pub start_frame: u64,
    /// The sprint key is down.
    pub held: bool,
    /// Cursor offset the roll runs towards (relative to the cell it started from).
    pub offset: Cell,
    pub action: u16,
    /// The roll turned into a sprint (the key was held at `start_frame + 15`).
    pub sprint: Option<SprintRun>,
    /// A sprint was started during this roll (even if it has ended again).
    pub sprinted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveState {
    Idle,
    Walking(Walk),
    Rolling(Rolling),
    /// The roll is over and the sprint goes on, until the key is released or an attack.
    Sprinting(SprintRun),
}

/// Scheduled timeline events.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Due {
    /// Roll `roll` starts its sprint if the key is still down.
    SprintStart {
        roll: u64,
    },
    /// A tap roll clears `is_dodge_rolling` (skipped when the roll became a sprint).
    RollEndTap {
        roll: u64,
    },
    /// The roll action is over.
    RollEnd {
        roll: u64,
    },
    /// `must_sprint` off; the buff index is in [`Locomotion::must_sprint_off`].
    MustSprintOff,
    /// The reply of the frame after a sprint ended: the unscaled direction.
    UnscaledAck,
    Stance,
}

/// Per-frame facts the component gets from the world.
pub struct StepCtx<'a> {
    pub frame: u64,
    /// Seconds per frame.
    pub dt: f32,
    pub terrain: &'a dyn Walkable,
    pub config: &'a MovementConfig,
    /// World-wide action id counter (`StartAction` ids are shared by all entities).
    pub next_action: &'a mut u16,
    /// The entity's next free buff index.
    pub next_buff: &'a mut u16,
}

/// The movement component.
#[derive(Debug, Clone)]
pub struct Locomotion {
    mover: Mover,
    state: MoveState,
    wasd: Cell,
    held: bool,
    /// A direction input arrived this frame: answer it.
    ack_wanted: bool,
    inbox: Vec<MoveInput>,
    due: Vec<(u64, Due)>,
    /// Buff index of the `must_sprint` buff while it is on.
    must_sprint_off: Option<u16>,
    combat_stance: bool,
    stance_scheduled: bool,
    teleport: Option<Cell>,
    /// A roll asked for while the previous roll's action still runs: it starts when that ends.
    queued_roll: Option<(UseSkill, Cell)>,
    /// Events that follow this frame's `MoveAck`.
    deferred: Vec<MoveEvent>,
}

impl Locomotion {
    /// An idle entity at `cell`, walking `speed` cells per second.
    pub fn new(cell: Cell, speed: f32) -> Self {
        Self {
            mover: Mover::at_cell(cell, speed),
            state: MoveState::Idle,
            wasd: (0, 0),
            held: false,
            ack_wanted: false,
            inbox: Vec::new(),
            due: Vec::new(),
            must_sprint_off: None,
            combat_stance: false,
            stance_scheduled: false,
            teleport: None,
            queued_roll: None,
            deferred: Vec::new(),
        }
    }

    /// Queues an input; it is handled in the next [`tick`](Self::tick).
    pub fn input(&mut self, input: MoveInput) {
        self.inbox.push(input);
    }

    /// Places the entity at `cell` in the next tick and stops it.
    pub fn teleport(&mut self, cell: Cell) {
        self.teleport = Some(cell);
    }

    pub fn state(&self) -> &MoveState {
        &self.state
    }

    /// The grid cell the entity stands in.
    pub fn cell(&self) -> Cell {
        self.mover.cell()
    }

    /// Exact position in cells.
    pub fn position(&self) -> (f32, f32) {
        (self.mover.x, self.mover.y)
    }

    /// The latest WASD vector.
    pub fn wasd(&self) -> Cell {
        self.wasd
    }

    /// The sprint key is down.
    pub fn sprint_held(&self) -> bool {
        self.held
    }

    pub fn in_combat_stance(&self) -> bool {
        self.combat_stance
    }

    /// Waypoints left of a click-to-move walk (cells).
    pub fn path(&self) -> &[Cell] {
        self.mover.path()
    }

    pub fn sprint(&self) -> Option<&SprintRun> {
        match &self.state {
            MoveState::Rolling(r) => r.sprint.as_ref(),
            MoveState::Sprinting(s) => Some(s),
            _ => None,
        }
    }

    fn sprint_mut(&mut self) -> Option<&mut SprintRun> {
        match &mut self.state {
            MoveState::Rolling(r) => r.sprint.as_mut(),
            MoveState::Sprinting(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_rolling(&self) -> bool {
        matches!(self.state, MoveState::Rolling(_))
    }

    /// One server frame; see the module docs for the order.
    pub fn tick(&mut self, ctx: &mut StepCtx) -> Vec<MoveEvent> {
        let mut out = Vec::new();
        // 1. Move.
        self.mover.advance(ctx.dt, ctx.terrain);
        if matches!(self.state, MoveState::Walking(Walk::Target { .. })) && !self.mover.is_moving()
        {
            self.state = MoveState::Idle;
        }
        if let Some(cell) = self.teleport.take() {
            self.mover.place(cell);
            self.state = MoveState::Idle;
            self.wasd = (0, 0);
            self.ack_wanted = false;
            out.push(MoveEvent::Teleport { cell });
        }
        // 2. Inputs.
        let mut end_sprint = false;
        for input in std::mem::take(&mut self.inbox) {
            self.handle(input, ctx, &mut out, &mut end_sprint);
        }
        // 3. Timeline.
        let mut unscaled_ack = false;
        let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.due)
            .into_iter()
            .partition(|(f, _)| *f <= ctx.frame);
        self.due = later;
        for (_, due) in now {
            self.fire(due, ctx, &mut out, &mut unscaled_ack);
        }
        // 4. MoveAck, then what follows it.
        self.move_ack(unscaled_ack, ctx, &mut out);
        out.append(&mut self.deferred);
        // 5. Sprint end.
        if end_sprint {
            self.end_sprint(ctx.frame, &mut out);
        }
        out
    }

    // ---- inputs

    fn handle(
        &mut self,
        input: MoveInput,
        ctx: &mut StepCtx,
        out: &mut Vec<MoveEvent>,
        end_sprint: &mut bool,
    ) {
        match input {
            MoveInput::Direction { x, y } => self.on_direction((x, y), ctx),
            MoveInput::UseSkill(req) => self.on_use_skill(req, ctx, out),
            MoveInput::UpdateSkillTarget { x, y, skill } => {
                let (cx, cy) = self.mover.cell();
                let abs = (cx + x, cy + y);
                self.steer(skill, (x, y), abs, ctx);
                out.push(MoveEvent::TargetAck {
                    skill,
                    x: abs.0,
                    y: abs.1,
                });
            }
            MoveInput::SkillInputEnd => out.push(MoveEvent::InputEndAck),
            MoveInput::SkillReleased { skill, flag } => {
                out.push(MoveEvent::ReleasedAck { skill, flag });
            }
            MoveInput::SprintInput { held } => {
                self.held = held;
                if let MoveState::Rolling(r) = &mut self.state {
                    r.held = held;
                }
                out.push(MoveEvent::SprintState { held });
                if !held && self.sprint().is_some() {
                    *end_sprint = true;
                }
            }
            MoveInput::Aim { x, y } => {
                // Never echoed while rolling or sprinting (all sessions).
                if !self.is_rolling() && self.sprint().is_none() {
                    out.push(MoveEvent::AimEcho { cell: (x, y) });
                }
            }
            MoveInput::AimEntity { .. } => {}
            MoveInput::AimMode { mode } => {
                // UNVERIFIED (gap #8): the captures show the stance change once at entry, 9
                // frames after the first mode 1; nothing about stance changes while moving.
                if mode == 1 && !self.combat_stance && !self.stance_scheduled {
                    self.stance_scheduled = true;
                    self.due.push((
                        ctx.frame + ctx.config.stance_delay_frames as u64,
                        Due::Stance,
                    ));
                }
            }
        }
    }

    fn on_direction(&mut self, dir: Cell, ctx: &StepCtx) {
        self.wasd = dir;
        self.ack_wanted = true;
        let moving = dir != (0, 0);
        if let Some(run) = self.sprint_mut() {
            // The stream starts with the first vector after the sprint started. A zero vector
            // is answered once. UNVERIFIED (gap #5): what the stream does when the direction
            // goes to zero during a sprint.
            run.streaming = moving;
        }
        match self.state {
            // Acted on when the roll ends.
            MoveState::Rolling(_) => {}
            MoveState::Sprinting(_) => self.mover.start(if moving {
                Motion::Sprint {
                    x: dir.0 as f32,
                    y: dir.1 as f32,
                    scale: ctx.config.sprint.speed_scale,
                }
            } else {
                Motion::Idle
            }),
            MoveState::Idle | MoveState::Walking(Walk::Direction(_)) => {
                if moving {
                    self.state = MoveState::Walking(Walk::Direction(dir));
                    self.mover.start(Motion::Direction {
                        x: dir.0 as f32,
                        y: dir.1 as f32,
                    });
                } else {
                    self.state = MoveState::Idle;
                    self.mover.start(Motion::Idle);
                }
            }
            MoveState::Walking(Walk::Target { .. }) => {
                // UNVERIFIED (gap #1): WASD takes over a click walk; a stop leaves it running.
                if moving {
                    self.state = MoveState::Walking(Walk::Direction(dir));
                    self.mover.start(Motion::Direction {
                        x: dir.0 as f32,
                        y: dir.1 as f32,
                    });
                }
            }
        }
    }

    fn target_of(&self, req: &UseSkill) -> Cell {
        match req.entity {
            Some(_) => (req.x, req.y),
            None => (self.mover.cell().0 + req.x, self.mover.cell().1 + req.y),
        }
    }

    fn skill_ack(req: &UseSkill, target: Cell) -> MoveEvent {
        MoveEvent::SkillAck {
            skill: req.skill,
            flags: req.flags,
            unk: req.unk,
            target_flags: req.target_flags,
            entity: req.entity,
            x: target.0,
            y: target.1,
        }
    }

    fn on_use_skill(&mut self, req: UseSkill, ctx: &mut StepCtx, out: &mut Vec<MoveEvent>) {
        let target = self.target_of(&req);
        let busy = matches!(self.state, MoveState::Rolling(_) | MoveState::Sprinting(_));
        if req.skill == ctx.config.skills.roll_skill {
            match self.state {
                // One capture (28 frames after the previous roll started, while its action
                // still ran): acked at once, the roll itself starts when the previous action
                // ends. UNVERIFIED (gap #4): the rule is generalised to any new press during
                // a roll; the other three chained rolls (31+ frames) started at once.
                MoveState::Rolling(_) if req.target_flags & UseSkill::HELD_REPEAT == 0 => {
                    out.push(Self::skill_ack(&req, target));
                    self.queued_roll = Some((req, target));
                }
                // Held-key repeats of the roll skill (target flags 0x13) are acked and nothing
                // else happens. UNVERIFIED (gap #4): a roll while sprinting is treated alike.
                MoveState::Rolling(_) | MoveState::Sprinting(_) => {
                    out.push(Self::skill_ack(&req, target));
                }
                _ => self.start_roll(req, target, true, ctx, out),
            }
        } else if req.skill == ctx.config.skills.move_skill {
            if busy {
                // UNVERIFIED (gap #7): Move input while rolling or sprinting is ignored.
                out.push(Self::skill_ack(&req, target));
            } else {
                self.start_walk(req, target, ctx, out);
            }
        } else {
            self.start_other(req, target, ctx, out);
        }
    }

    fn alloc_action(ctx: &mut StepCtx) -> u16 {
        let id = *ctx.next_action;
        *ctx.next_action = ctx.next_action.wrapping_add(1);
        id
    }

    /// Starts a roll towards the absolute `target`. `ack` is false for a queued roll, whose
    /// `UseSkillAck` was sent when it was asked for.
    fn start_roll(
        &mut self,
        req: UseSkill,
        target: Cell,
        ack: bool,
        ctx: &mut StepCtx,
        out: &mut Vec<MoveEvent>,
    ) {
        let cell = self.mover.cell();
        let offset = (target.0 - cell.0, target.1 - cell.1);
        let action = Self::alloc_action(ctx);
        let cfg = ctx.config;
        // UNVERIFIED (gap #2): a standing roll (no WASD held) is assumed to get the same batch
        // and to end in Idle with no `MoveAck`.
        self.mover.start(if offset == (0, 0) {
            Motion::Idle
        } else {
            Motion::Roll {
                x: offset.0 as f32,
                y: offset.1 as f32,
                remaining: cfg.roll.travel_distance,
                elapsed: 0.0,
            }
        });
        let t0 = ctx.frame;
        self.state = MoveState::Rolling(Rolling {
            start_frame: t0,
            held: self.held,
            offset,
            action,
            sprint: None,
            sprinted: false,
        });
        out.push(MoveEvent::ActionStart {
            kind: ActionKind::Roll,
            skill: req.skill,
            action,
            cell,
            target,
            entity: req.entity,
            unk: req.unk,
        });
        out.push(MoveEvent::ActionEvent);
        out.push(MoveEvent::Stats(vec![(MoveStat::IsDodgeRolling, 1)]));
        if ack {
            out.push(Self::skill_ack(&req, target));
        }
        out.push(MoveEvent::Stats(stun_stats(cfg.roll.stun_threshold)));
        let at = |frames: u32| t0 + frames as u64;
        self.due
            .push((at(cfg.sprint.start_frames), Due::SprintStart { roll: t0 }));
        self.due
            .push((at(cfg.roll.tap_end_frames), Due::RollEndTap { roll: t0 }));
        self.due
            .push((at(cfg.roll.action_frames), Due::RollEnd { roll: t0 }));
    }

    fn start_walk(
        &mut self,
        req: UseSkill,
        target: Cell,
        ctx: &mut StepCtx,
        out: &mut Vec<MoveEvent>,
    ) {
        // UNVERIFIED (gap #1): click-to-move replies are derived from the client code (lockstep
        // design): `StartAction` Move with the absolute target and a `UseSkillAck`; held-click
        // repeats are acked and re-aim the walk.
        let repeat = req.target_flags & UseSkill::HELD_REPEAT != 0
            && matches!(self.state, MoveState::Walking(Walk::Target { .. }));
        let cell = self.mover.cell();
        self.state = MoveState::Walking(Walk::Target { goal: target });
        self.mover.start(Motion::MoveTo {
            x: target.0 as f32 + 0.5,
            y: target.1 as f32 + 0.5,
        });
        if !repeat {
            out.push(MoveEvent::ActionStart {
                kind: ActionKind::Move,
                skill: req.skill,
                action: Self::alloc_action(ctx),
                cell,
                target,
                entity: req.entity,
                unk: req.unk,
            });
        }
        out.push(Self::skill_ack(&req, target));
    }

    fn start_other(
        &mut self,
        req: UseSkill,
        target: Cell,
        ctx: &mut StepCtx,
        out: &mut Vec<MoveEvent>,
    ) {
        // UNVERIFIED (gap #6): attacks only end a running sprint (the one sequence in the
        // captures); they do not stop walking, and an attack during a roll is just acked.
        if self.is_rolling() {
            out.push(Self::skill_ack(&req, target));
            return;
        }
        let cell = self.mover.cell();
        out.push(MoveEvent::ActionStart {
            kind: ActionKind::Other,
            skill: req.skill,
            action: Self::alloc_action(ctx),
            cell,
            target,
            entity: req.entity,
            unk: req.unk,
        });
        let Some(run) = self.sprint().copied() else {
            out.push(Self::skill_ack(&req, target));
            return;
        };
        // The attack ends the sprint: (`must_sprint` off, if it still runs,) `0x1c8`, the first
        // half of the sprint-off, `0x1ca`, the ack, then after the frame's `MoveAck` (unscaled)
        // the second half.
        if let Some(n) = self.must_sprint_off.take() {
            let start = out.pop().expect("ActionStart");
            out.push(MoveEvent::BuffRemoved { index: n });
            out.push(MoveEvent::Stats(vec![(MoveStat::MustSprint, 0)]));
            out.push(start);
        }
        out.push(MoveEvent::BuffRemoved {
            index: run.buff_base + 1,
        });
        out.push(MoveEvent::Stats(sprint_off_stats()));
        out.push(MoveEvent::ActionEvent);
        out.push(Self::skill_ack(&req, target));
        self.deferred.push(MoveEvent::BuffRemoved {
            index: run.buff_base + 2,
        });
        self.deferred
            .push(MoveEvent::Stats(vec![(MoveStat::AllHitsStunYou, 0)]));
        self.ack_wanted |= run.streaming && self.wasd != (0, 0);
        self.finish_sprint();
    }

    /// Re-aims the running action after a `UpdateSkillTarget`.
    fn steer(&mut self, skill: u32, offset: Cell, abs: Cell, ctx: &StepCtx) {
        match &mut self.state {
            MoveState::Rolling(r) if skill == ctx.config.skills.roll_skill => {
                if offset != (0, 0) {
                    r.offset = offset;
                    if let Motion::Roll { x, y, .. } = &mut self.mover.motion {
                        *x = offset.0 as f32;
                        *y = offset.1 as f32;
                    }
                }
            }
            MoveState::Walking(Walk::Target { goal }) if skill == ctx.config.skills.move_skill => {
                *goal = abs;
                self.mover.start(Motion::MoveTo {
                    x: abs.0 as f32 + 0.5,
                    y: abs.1 as f32 + 0.5,
                });
            }
            _ => {}
        }
    }

    // ---- timeline

    fn fire(&mut self, due: Due, ctx: &mut StepCtx, out: &mut Vec<MoveEvent>, unscaled: &mut bool) {
        let cfg = ctx.config;
        match due {
            Due::SprintStart { roll } => {
                let MoveState::Rolling(r) = &mut self.state else {
                    return;
                };
                if r.start_frame != roll || !r.held || r.sprint.is_some() {
                    return;
                }
                let n = *ctx.next_buff;
                *ctx.next_buff += 3;
                r.sprint = Some(SprintRun {
                    start_frame: roll,
                    buff_base: n,
                    streaming: false,
                });
                r.sprinted = true;
                for (i, row) in cfg.sprint.buff_rows.iter().enumerate() {
                    out.push(MoveEvent::BuffAdded {
                        index: n + i as u16,
                        row: *row,
                        fade: None,
                    });
                }
                out.push(MoveEvent::Stats(vec![
                    (
                        MoveStat::SprintBlendTimeOverrideMs,
                        cfg.sprint.blend_time_override_ms,
                    ),
                    (MoveStat::IsSprinting, 1),
                    (MoveStat::MustSprint, 1),
                    (MoveStat::AllHitsStunYou, 1),
                    (MoveStat::VirtualSkillIgnoreNamelockingDynamically, 1),
                ]));
                self.must_sprint_off = Some(n);
                self.due.push((
                    roll + cfg.sprint.must_sprint_end_frames as u64,
                    Due::MustSprintOff,
                ));
            }
            Due::RollEndTap { roll } => {
                if let MoveState::Rolling(r) = self.state
                    && r.start_frame == roll
                    && !r.sprinted
                {
                    // `is_dodge_rolling` goes off; the action itself runs on to `action_frames`.
                    roll_end_stats(out);
                }
            }
            Due::RollEnd { roll } => {
                let MoveState::Rolling(r) = self.state else {
                    return;
                };
                if r.start_frame != roll {
                    return;
                }
                // A tap roll cleared its stats earlier; one that sprinted does it now.
                if r.sprinted {
                    roll_end_stats(out);
                }
                if let Some(run) = r.sprint {
                    out.push(MoveEvent::Stats(vec![
                        (
                            MoveStat::BaseMoveSpeedFromSprint,
                            cfg.sprint.move_speed_percent,
                        ),
                        (
                            MoveStat::MovementVelocityPermyriad,
                            cfg.sprint.velocity_permyriad,
                        ),
                    ]));
                    out.push(MoveEvent::BuffAdded {
                        index: run.buff_base + 1,
                        row: cfg.sprint.buff_rows[1],
                        fade: None,
                    });
                    out.push(MoveEvent::BuffAdded {
                        index: run.buff_base,
                        row: cfg.sprint.buff_rows[0],
                        fade: Some(cfg.sprint.buff_fade_secs),
                    });
                    self.state = MoveState::Sprinting(run);
                    self.start_sprint_motion(ctx);
                } else {
                    // The client resumes walking by itself after a roll; no message.
                    // (UNVERIFIED (gap #3) if the sprint was cut short by the key going up
                    // between +15 and +30.)
                    self.resume_walking();
                }
                if let Some((req, target)) = self.queued_roll.take() {
                    self.start_roll(req, target, false, ctx, out);
                }
            }
            Due::MustSprintOff => {
                if let Some(n) = self.must_sprint_off.take() {
                    out.push(MoveEvent::BuffRemoved { index: n });
                    out.push(MoveEvent::Stats(vec![(MoveStat::MustSprint, 0)]));
                }
            }
            Due::UnscaledAck => *unscaled = true,
            Due::Stance => {
                self.combat_stance = true;
                out.push(MoveEvent::StanceChanged { combat: true });
            }
        }
    }

    /// Walks in the held direction, or stops.
    fn resume_walking(&mut self) {
        if self.wasd != (0, 0) {
            self.state = MoveState::Walking(Walk::Direction(self.wasd));
            self.mover.start(Motion::Direction {
                x: self.wasd.0 as f32,
                y: self.wasd.1 as f32,
            });
        } else {
            self.state = MoveState::Idle;
            self.mover.start(Motion::Idle);
        }
    }

    /// The roll that became a sprint is over: sprint in the held direction.
    fn start_sprint_motion(&mut self, ctx: &StepCtx) {
        self.mover.start(if self.wasd != (0, 0) {
            Motion::Sprint {
                x: self.wasd.0 as f32,
                y: self.wasd.1 as f32,
                scale: ctx.config.sprint.speed_scale,
            }
        } else {
            Motion::Idle
        });
    }

    // ---- replies of the frame

    fn move_ack(&mut self, unscaled_due: bool, ctx: &StepCtx, out: &mut Vec<MoveEvent>) {
        let cell = self.mover.cell();
        let wasd = self.wasd;
        let (num, den) = ctx.config.sprint.dir_scale;
        let scaled = (wasd.0 * num / den, wasd.1 * num / den);
        match self.sprint() {
            Some(run) if run.streaming => out.push(MoveEvent::MoveAck { dir: scaled, cell }),
            _ if self.ack_wanted || unscaled_due => {
                out.push(MoveEvent::MoveAck { dir: wasd, cell })
            }
            _ => {}
        }
        self.ack_wanted = false;
    }

    /// The sprint key went up: sprint-off buffs and stats after this frame's `MoveAck`, then
    /// back to walking. The stream's last message was scaled and the next frame sends the
    /// unscaled direction.
    fn end_sprint(&mut self, frame: u64, out: &mut Vec<MoveEvent>) {
        let Some(run) = self.sprint().copied() else {
            return;
        };
        out.push(MoveEvent::BuffRemoved {
            index: run.buff_base + 1,
        });
        out.push(MoveEvent::Stats(sprint_off_stats()));
        out.push(MoveEvent::BuffRemoved {
            index: run.buff_base + 2,
        });
        out.push(MoveEvent::Stats(vec![(MoveStat::AllHitsStunYou, 0)]));
        if run.streaming && self.wasd != (0, 0) {
            self.due.push((frame + 1, Due::UnscaledAck));
        }
        self.finish_sprint();
    }

    /// Leaves the sprint: a sprinting entity walks on; a rolling one finishes its roll.
    fn finish_sprint(&mut self) {
        match &mut self.state {
            MoveState::Rolling(r) => r.sprint = None,
            MoveState::Sprinting(_) => self.resume_walking(),
            _ => {}
        }
    }
}

fn stun_stats(value: i32) -> Vec<(MoveStat, i32)> {
    vec![
        (MoveStat::StunThresholdWhilePerformingAction, value),
        (MoveStat::VirtualHeavyStunThreshold, value),
        (MoveStat::VirtualLightStunThreshold, value),
    ]
}

fn roll_end_stats(out: &mut Vec<MoveEvent>) {
    let mut stats = stun_stats(0);
    stats.push((MoveStat::IsDodgeRolling, 0));
    out.push(MoveEvent::Stats(stats));
}

fn sprint_off_stats() -> Vec<(MoveStat, i32)> {
    vec![
        (MoveStat::BaseMoveSpeedFromSprint, 0),
        (MoveStat::MovementVelocityPermyriad, 0),
        (MoveStat::SprintBlendTimeOverrideMs, 0),
        (MoveStat::IsSprinting, 0),
        (MoveStat::VirtualSkillIgnoreNamelockingDynamically, 0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{grid::VecGrid, movement::Open};

    const MOVE: u32 = 0x2000_2909;
    const ROLL: u32 = 0x8000_0040;
    const ATTACK: u32 = 0x8040_0040;

    /// One entity and its frame clock.
    struct Rig {
        loco: Locomotion,
        cfg: MovementConfig,
        grid: Box<dyn Walkable>,
        frame: u64,
        next_action: u16,
        next_buff: u16,
    }

    impl Rig {
        fn new(cell: Cell) -> Self {
            Self::on(cell, Box::new(Open))
        }

        fn on(cell: Cell, grid: Box<dyn Walkable>) -> Self {
            let cfg = MovementConfig::new(
                SkillIds {
                    move_skill: MOVE,
                    roll_skill: ROLL,
                },
                [100, 101, 102],
            );
            Self {
                loco: Locomotion::new(cell, cfg.walk_speed),
                cfg,
                grid,
                frame: 0,
                next_action: 5000,
                next_buff: 0,
            }
        }

        /// Submits `inputs` and ticks once.
        fn tick(&mut self, inputs: &[MoveInput]) -> Vec<MoveEvent> {
            for i in inputs {
                self.loco.input(*i);
            }
            self.frame += 1;
            let mut ctx = StepCtx {
                frame: self.frame,
                dt: 0.033,
                terrain: &*self.grid,
                config: &self.cfg,
                next_action: &mut self.next_action,
                next_buff: &mut self.next_buff,
            };
            self.loco.tick(&mut ctx)
        }

        /// Ticks `n` frames without input, collecting `(frame, event)`.
        fn run(&mut self, n: u64) -> Vec<(u64, MoveEvent)> {
            let mut all = Vec::new();
            for _ in 0..n {
                let ev = self.tick(&[]);
                all.extend(ev.into_iter().map(|e| (self.frame, e)));
            }
            all
        }
    }

    fn dir(x: i32, y: i32) -> MoveInput {
        MoveInput::Direction { x, y }
    }

    fn skill(skill: u32, x: i32, y: i32) -> UseSkill {
        UseSkill {
            skill,
            flags: 0x400,
            unk: 0xff,
            target_flags: 0,
            entity: None,
            x,
            y,
        }
    }

    fn roll(x: i32, y: i32) -> MoveInput {
        MoveInput::UseSkill(skill(ROLL, x, y))
    }

    fn space(held: bool) -> MoveInput {
        MoveInput::SprintInput { held }
    }

    fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
    }

    #[test]
    fn wasd_is_acked_in_the_next_frame_and_moves_from_the_one_after() {
        let mut r = Rig::new((100, 100));
        let start = r.loco.position();
        assert_eq!(
            r.tick(&[dir(180, 0)]),
            vec![MoveEvent::MoveAck {
                dir: (180, 0),
                cell: (100, 100)
            }]
        );
        assert_eq!(
            r.loco.state(),
            &MoveState::Walking(Walk::Direction((180, 0)))
        );
        // The client turns when it receives the reply; the server moves from the next frame.
        assert_eq!(r.loco.position(), start);
        r.tick(&[]);
        assert!((dist(r.loco.position(), start) - 37.0 * 0.033).abs() < 1e-3);
        // Same vector again: acked again, state kept.
        assert_eq!(r.tick(&[dir(180, 0)]).len(), 1);
        // Turn.
        let ev = r.tick(&[dir(0, -180)]);
        assert!(matches!(
            &ev[..],
            [MoveEvent::MoveAck { dir: (0, -180), .. }]
        ));
    }

    #[test]
    fn wasd_stop_goes_idle() {
        let mut r = Rig::new((10, 10));
        r.tick(&[dir(127, 127)]);
        r.run(5);
        let ev = r.tick(&[dir(0, 0)]);
        assert_eq!(
            ev,
            vec![MoveEvent::MoveAck {
                dir: (0, 0),
                cell: r.loco.cell()
            }]
        );
        assert_eq!(r.loco.state(), &MoveState::Idle);
        let at = r.loco.position();
        r.run(5);
        assert_eq!(r.loco.position(), at);
        assert!(r.run(5).is_empty());
    }

    #[test]
    fn walking_stops_at_a_wall() {
        let grid = VecGrid::from_rows(&["..........", "..........", "##########"]);
        let mut r = Rig::on((2, 1), Box::new(grid));
        r.tick(&[dir(0, 180)]);
        r.run(20);
        assert_eq!(r.loco.cell().1, 1);
    }

    #[test]
    fn standing_roll_batch_and_timeline() {
        let mut r = Rig::new((50, 50));
        let ev = r.tick(&[space(true), roll(15, -35)]);
        let kinds: Vec<_> = ev
            .iter()
            .map(|e| match e {
                MoveEvent::SprintState { .. } => "sprint",
                MoveEvent::ActionStart { .. } => "start",
                MoveEvent::ActionEvent => "event",
                MoveEvent::Stats(_) => "stats",
                MoveEvent::SkillAck { .. } => "ack",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["sprint", "start", "event", "stats", "ack", "stats"]);
        let MoveEvent::ActionStart {
            cell,
            target,
            kind,
            action,
            skill,
            ..
        } = &ev[1]
        else {
            panic!()
        };
        assert_eq!(
            (*cell, *target, *kind, *skill),
            ((50, 50), (65, 15), ActionKind::Roll, ROLL)
        );
        assert_eq!(*action, 5000);
        assert!(matches!(
            &ev[4],
            MoveEvent::SkillAck {
                x: 65,
                y: 15,
                flags: 0x400,
                unk: 0xff,
                ..
            }
        ));
        assert!(r.loco.is_rolling());
        // Space is still down at +15: the sprint starts there.
        let later = r.run(15);
        let at15: Vec<_> = later.iter().filter(|(f, _)| *f == 16).collect();
        assert_eq!(at15.len(), 4, "{at15:?}");
        assert!(matches!(
            at15[0].1,
            MoveEvent::BuffAdded {
                index: 0,
                row: 100,
                fade: None
            }
        ));
        assert!(matches!(
            at15[2].1,
            MoveEvent::BuffAdded {
                index: 2,
                row: 102,
                ..
            }
        ));
        assert!(
            matches!(&at15[3].1, MoveEvent::Stats(s) if s.contains(&(MoveStat::IsSprinting, 1)))
        );
        // Nothing at +16 (tap stats are skipped once the roll became a sprint).
        assert!(r.run(1).is_empty());
    }

    #[test]
    fn tap_roll_ends_its_stats_at_16_and_the_action_at_30() {
        let mut r = Rig::new((50, 50));
        r.tick(&[roll(30, 0)]);
        r.tick(&[MoveInput::SkillInputEnd, space(false)]);
        let ev = r.run(40);
        let frames: Vec<u64> = ev.iter().map(|(f, _)| *f).collect();
        // Roll batch at frame 1: tap stats at 1 + 16, nothing else.
        assert_eq!(frames, [17]);
        assert!(matches!(&ev[0].1, MoveEvent::Stats(s)
            if s.len() == 4 && s[3] == (MoveStat::IsDodgeRolling, 0)));
        assert_eq!(r.loco.state(), &MoveState::Idle);
        // About 46 cells to the right (the speed profile may stop it a little short).
        assert!(r.loco.position().0 > 50.0 + 40.0, "{:?}", r.loco.position());
        assert!((r.loco.position().1 - 50.5).abs() < 0.1);
    }

    #[test]
    fn roll_state_lasts_30_frames_then_walking_resumes() {
        let mut r = Rig::new((50, 50));
        r.tick(&[dir(0, 180)]);
        r.tick(&[roll(30, 0)]);
        // A WASD change during the roll is acked at once and acted on after the roll.
        let ev = r.tick(&[dir(180, 0)]);
        assert!(matches!(
            &ev[..],
            [MoveEvent::MoveAck { dir: (180, 0), .. }]
        ));
        assert!(r.loco.is_rolling());
        r.run(28);
        assert!(r.loco.is_rolling(), "t0 + 29");
        r.run(1);
        assert_eq!(
            r.loco.state(),
            &MoveState::Walking(Walk::Direction((180, 0)))
        );
    }

    #[test]
    fn roll_steering_moves_the_target_and_is_acked() {
        let mut r = Rig::new((50, 50));
        r.tick(&[roll(30, 0)]);
        let ev = r.tick(&[MoveInput::UpdateSkillTarget {
            x: 0,
            y: 30,
            skill: ROLL,
        }]);
        // Relative to the server's cell at the time.
        let cell = r.loco.cell();
        assert_eq!(
            ev,
            vec![MoveEvent::TargetAck {
                skill: ROLL,
                x: cell.0,
                y: cell.1 + 30
            }]
        );
        r.run(30);
        let (x, y) = r.loco.position();
        assert!(y > 50.0 + 20.0 && x < 50.0 + 20.0, "{x} {y}");
    }

    #[test]
    fn no_aim_echo_while_rolling_or_sprinting() {
        let mut r = Rig::new((50, 50));
        let aim = MoveInput::Aim { x: 60, y: 61 };
        assert_eq!(r.tick(&[aim]), vec![MoveEvent::AimEcho { cell: (60, 61) }]);
        assert!(r.tick(&[AimEntityInput()]).is_empty());
        r.tick(&[space(true), roll(10, 0)]);
        assert!(r.tick(&[aim]).is_empty());
        // Sprint, after the roll action: still no echo.
        r.run(30);
        assert!(r.loco.sprint().is_some());
        assert!(r.tick(&[aim]).is_empty());
        r.tick(&[MoveInput::SkillInputEnd, space(false)]);
        assert_eq!(r.tick(&[aim]), vec![MoveEvent::AimEcho { cell: (60, 61) }]);
    }

    #[allow(non_snake_case)]
    fn AimEntityInput() -> MoveInput {
        MoveInput::AimEntity { entity: 7 }
    }

    /// Roll with Space held until the sprint runs and a vector is set.
    fn sprinting(wasd: Cell) -> Rig {
        let mut r = Rig::new((50, 50));
        r.tick(&[space(true), roll(10, 0)]);
        r.run(36); // past `must_sprint` off at t0 + 35
        assert!(matches!(r.loco.state(), MoveState::Sprinting(_)));
        if wasd != (0, 0) {
            r.tick(&[dir(wasd.0, wasd.1)]);
        }
        r
    }

    #[test]
    fn sprint_stream_is_one_scaled_move_ack_per_frame() {
        let mut r = sprinting((127, -127));
        // The reply to the vector is the stream's first message, scaled with integer division.
        // (`sprinting` consumed it; the next frames follow.)
        for _ in 0..5 {
            let ev = r.tick(&[]);
            assert!(
                matches!(
                    &ev[..],
                    [MoveEvent::MoveAck {
                        dir: (388, -388),
                        ..
                    }]
                ),
                "{ev:?}"
            );
        }
        // A new vector changes the next message.
        let ev = r.tick(&[dir(-53, 172)]);
        assert!(matches!(
            &ev[..],
            [MoveEvent::MoveAck {
                dir: (-161, 525),
                ..
            }]
        ));
        // Speed: 1.5 x walking.
        let p = r.loco.position();
        r.tick(&[]);
        let step = dist(p, r.loco.position());
        assert!((step - 37.0 * 1.5 * 0.033).abs() < 0.01, "{step}");
    }

    #[test]
    fn a_vector_during_the_roll_before_the_sprint_is_not_scaled() {
        let mut r = Rig::new((50, 50));
        r.tick(&[space(true), roll(10, 0)]);
        let ev = r.tick(&[dir(127, 127)]);
        assert!(matches!(
            &ev[..],
            [MoveEvent::MoveAck {
                dir: (127, 127),
                ..
            }]
        ));
        // No stream before the sprint starts (t0 + 15).
        assert!(r.run(13).is_empty());
        // The first vector after the start begins it, still during the roll.
        r.run(1);
        let ev = r.tick(&[dir(127, 127)]);
        assert!(matches!(
            &ev[..],
            [MoveEvent::MoveAck {
                dir: (388, 388),
                ..
            }]
        ));
        assert!(r.loco.is_rolling());
        assert_eq!(r.tick(&[]).len(), 1);
    }

    #[test]
    fn sprint_timeline_t0_15_30_35() {
        let mut r = Rig::new((50, 50));
        r.tick(&[space(true), roll(10, 0)]);
        let ev = r.run(40);
        let at = |f: u64| {
            ev.iter()
                .filter(|(g, _)| *g == f)
                .map(|(_, e)| e)
                .collect::<Vec<_>>()
        };
        assert_eq!(at(16).len(), 4);
        assert!(at(30).is_empty() && at(31).len() == 4, "{:?}", at(31));
        // t0 + 30: roll-end stats, sprint speed stats, buff n + 1 again, buff n with a fade.
        assert!(matches!(
            at(31)[2],
            MoveEvent::BuffAdded {
                index: 1,
                row: 101,
                fade: None
            }
        ));
        assert!(
            matches!(at(31)[3], MoveEvent::BuffAdded { index: 0, row: 100, fade: Some(f) } if (*f - 0.1).abs() < 1e-6)
        );
        // t0 + 35: `must_sprint` off.
        assert!(matches!(
            at(36)[..],
            [MoveEvent::BuffRemoved { index: 0 }, MoveEvent::Stats(_)]
        ));
    }

    #[test]
    fn sprint_release_sequence_and_unscaled_follow_up() {
        let mut r = sprinting((0, -180));
        r.run(3);
        let ev = r.tick(&[MoveInput::SkillInputEnd, space(false)]);
        let names: Vec<&str> = ev
            .iter()
            .map(|e| match e {
                MoveEvent::InputEndAck => "1b1",
                MoveEvent::SprintState { held: false } => "2e7",
                MoveEvent::MoveAck { dir: (0, -550), .. } => "64 scaled",
                MoveEvent::BuffRemoved { index: 1 } => "1ec n+1",
                MoveEvent::BuffRemoved { index: 2 } => "1ec n+2",
                MoveEvent::Stats(s) if s.len() == 5 => "off",
                MoveEvent::Stats(s) if s == &[(MoveStat::AllHitsStunYou, 0)] => "all_hits",
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            names,
            [
                "1b1",
                "2e7",
                "64 scaled",
                "1ec n+1",
                "off",
                "1ec n+2",
                "all_hits"
            ]
        );
        assert_eq!(
            r.loco.state(),
            &MoveState::Walking(Walk::Direction((0, -180)))
        );
        let ev = r.tick(&[]);
        assert!(matches!(
            &ev[..],
            [MoveEvent::MoveAck { dir: (0, -180), .. }]
        ));
        assert!(r.run(5).is_empty());
    }

    #[test]
    fn a_zero_vector_during_the_sprint_stops_the_stream() {
        // UNVERIFIED (gap #5).
        let mut r = sprinting((0, -180));
        let ev = r.tick(&[dir(0, 0)]);
        assert!(matches!(&ev[..], [MoveEvent::MoveAck { dir: (0, 0), .. }]));
        assert!(r.run(3).is_empty());
        assert!(matches!(r.loco.state(), MoveState::Sprinting(_)));
        // Releasing now ends the sprint without a `MoveAck`.
        let ev = r.tick(&[MoveInput::SkillInputEnd, space(false)]);
        assert!(!ev.iter().any(|e| matches!(e, MoveEvent::MoveAck { .. })));
        assert_eq!(r.loco.state(), &MoveState::Idle);
    }

    #[test]
    fn an_attack_ends_the_sprint() {
        let mut r = sprinting((0, -180));
        r.run(5);
        let ev = r.tick(&[MoveInput::UseSkill(skill(ATTACK, 5, 5))]);
        let names: Vec<&str> = ev
            .iter()
            .map(|e| match e {
                MoveEvent::ActionStart {
                    kind: ActionKind::Other,
                    ..
                } => "1c8",
                MoveEvent::BuffRemoved { index: 1 } => "1ec n+1",
                MoveEvent::BuffRemoved { index: 2 } => "1ec n+2",
                MoveEvent::Stats(s) if s.len() == 5 => "off",
                MoveEvent::Stats(_) => "all_hits",
                MoveEvent::ActionEvent => "1ca",
                MoveEvent::SkillAck { .. } => "1ae",
                MoveEvent::MoveAck { dir: (0, -180), .. } => "64 unscaled",
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            names,
            [
                "1c8",
                "1ec n+1",
                "off",
                "1ca",
                "1ae",
                "64 unscaled",
                "1ec n+2",
                "all_hits"
            ]
        );
        assert_eq!(
            r.loco.state(),
            &MoveState::Walking(Walk::Direction((0, -180)))
        );
        // The key release now only gets its echo.
        let ev = r.tick(&[MoveInput::SkillInputEnd, space(false)]);
        assert_eq!(
            ev,
            vec![
                MoveEvent::InputEndAck,
                MoveEvent::SprintState { held: false }
            ]
        );
    }

    #[test]
    fn an_attack_just_after_the_roll_also_ends_must_sprint() {
        let mut r = Rig::new((50, 50));
        r.tick(&[space(true), roll(10, 0)]);
        r.run(31); // frame 32 = t0 + 31: sprinting, `must_sprint` still on
        assert!(matches!(r.loco.state(), MoveState::Sprinting(_)));
        let ev = r.tick(&[MoveInput::UseSkill(skill(ATTACK, 5, 5))]);
        assert!(matches!(
            &ev[..3],
            [
                MoveEvent::BuffRemoved { index: 0 },
                MoveEvent::Stats(_),
                MoveEvent::ActionStart { .. }
            ]
        ));
        // The t0 + 35 timer finds nothing left to do.
        assert!(r.run(5).is_empty());
    }

    #[test]
    fn a_roll_asked_for_during_a_roll_waits_for_its_end() {
        // UNVERIFIED (gap #4): one capture (second roll 27 frames after the first started).
        let mut r = Rig::new((50, 50));
        r.tick(&[roll(30, 0)]);
        r.run(26);
        let ev = r.tick(&[roll(0, 30)]);
        assert!(matches!(
            &ev[..],
            [MoveEvent::SkillAck {
                skill: ROLL,
                y: 80,
                ..
            }]
        ));
        let ev = r.run(3);
        assert_eq!(ev.len(), 4, "{ev:?}");
        assert_eq!(ev[0].0, 31, "t0 + 30");
        assert!(matches!(
            ev[0].1,
            MoveEvent::ActionStart {
                target: (_, 80),
                ..
            }
        ));
        assert!(matches!(ev[1].1, MoveEvent::ActionEvent));
        assert!(r.loco.is_rolling());
    }

    #[test]
    fn held_repeats_of_the_roll_skill_are_only_acked() {
        let mut r = Rig::new((50, 50));
        r.tick(&[space(true), roll(10, 0)]);
        r.run(31);
        let mut again = skill(ROLL, 10, 0);
        again.target_flags = 0x13;
        again.flags = 0;
        let ev = r.tick(&[MoveInput::UseSkill(again)]);
        assert!(matches!(
            &ev[..],
            [MoveEvent::SkillAck {
                flags: 0,
                target_flags: 0x13,
                ..
            }]
        ));
        assert!(matches!(r.loco.state(), MoveState::Sprinting(_)));
    }

    #[test]
    fn click_to_move_walks_around_a_wall_and_stops() {
        // UNVERIFIED (gap #1) for the replies; the walk itself uses the A*.
        let grid = VecGrid::from_rows(&[
            "....#....",
            "....#....",
            "....#....",
            "....#....",
            ".........",
        ]);
        let mut r = Rig::on((1, 0), Box::new(grid));
        let ev = r.tick(&[MoveInput::UseSkill(skill(MOVE, 6, 0))]);
        assert!(matches!(
            &ev[..],
            [
                MoveEvent::ActionStart {
                    kind: ActionKind::Move,
                    cell: (1, 0),
                    target: (7, 0),
                    ..
                },
                MoveEvent::SkillAck { x: 7, y: 0, .. }
            ]
        ));
        assert_eq!(
            r.loco.state(),
            &MoveState::Walking(Walk::Target { goal: (7, 0) })
        );
        let mut went_down = false;
        for _ in 0..60 {
            r.tick(&[]);
            went_down |= r.loco.cell().1 >= 3;
            assert_ne!(r.loco.cell(), (4, 0), "through the wall");
        }
        assert!(went_down);
        assert_eq!(r.loco.cell(), (7, 0));
        assert_eq!(r.loco.state(), &MoveState::Idle);
    }

    #[test]
    fn held_click_repeats_and_reaims() {
        let mut r = Rig::new((10, 10));
        r.tick(&[MoveInput::UseSkill(skill(MOVE, 20, 0))]);
        let mut held = skill(MOVE, 0, 20);
        held.target_flags = UseSkill::HELD_REPEAT;
        let ev = r.tick(&[MoveInput::UseSkill(held)]);
        // No second StartAction, only the ack with the new absolute target.
        assert!(
            matches!(&ev[..], [MoveEvent::SkillAck { y: 30, .. }]),
            "{ev:?}"
        );
        let ev = r.tick(&[MoveInput::UpdateSkillTarget {
            x: 5,
            y: 5,
            skill: MOVE,
        }]);
        let cell = r.loco.cell();
        assert_eq!(
            ev,
            vec![MoveEvent::TargetAck {
                skill: MOVE,
                x: cell.0 + 5,
                y: cell.1 + 5
            }]
        );
        assert!(
            matches!(r.loco.state(), MoveState::Walking(Walk::Target { goal }) if *goal == (cell.0 + 5, cell.1 + 5))
        );
        assert_eq!(
            r.tick(&[
                MoveInput::SkillReleased {
                    skill: MOVE,
                    flag: 0
                },
                MoveInput::SkillInputEnd
            ]),
            vec![
                MoveEvent::ReleasedAck {
                    skill: MOVE,
                    flag: 0
                },
                MoveEvent::InputEndAck
            ]
        );
    }

    #[test]
    fn wasd_takes_over_a_click_walk() {
        let mut r = Rig::new((10, 10));
        r.tick(&[MoveInput::UseSkill(skill(MOVE, 20, 0))]);
        r.tick(&[dir(0, 180)]);
        assert_eq!(
            r.loco.state(),
            &MoveState::Walking(Walk::Direction((0, 180)))
        );
    }

    #[test]
    fn entity_target_skills_use_absolute_cells() {
        let mut r = Rig::new((10, 10));
        let mut s = skill(ATTACK, 40, 41);
        s.entity = Some(77);
        s.target_flags = 8;
        let ev = r.tick(&[MoveInput::UseSkill(s)]);
        assert!(matches!(
            &ev[..],
            [
                MoveEvent::ActionStart {
                    kind: ActionKind::Other,
                    target: (40, 41),
                    entity: Some(77),
                    ..
                },
                MoveEvent::SkillAck {
                    x: 40,
                    y: 41,
                    entity: Some(77),
                    target_flags: 8,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn stance_changes_9_frames_after_aim_mode_one() {
        let mut r = Rig::new((10, 10));
        r.tick(&[MoveInput::AimMode { mode: 0 }]);
        r.tick(&[MoveInput::AimMode { mode: 1 }]);
        let ev = r.run(12);
        assert_eq!(ev, vec![(11, MoveEvent::StanceChanged { combat: true })]);
        assert!(r.loco.in_combat_stance());
        // Once only.
        r.tick(&[MoveInput::AimMode { mode: 1 }]);
        assert!(r.run(12).is_empty());
    }

    #[test]
    fn teleport_places_and_reports() {
        let mut r = Rig::new((10, 10));
        r.tick(&[dir(180, 0)]);
        r.loco.teleport((200, 300));
        let ev = r.tick(&[]);
        assert_eq!(ev, vec![MoveEvent::Teleport { cell: (200, 300) }]);
        assert_eq!(r.loco.cell(), (200, 300));
        assert_eq!(r.loco.state(), &MoveState::Idle);
    }

    #[test]
    fn sprint_buffs_come_from_the_config() {
        let mut r = Rig::new((0, 0));
        r.cfg.sprint.buff_rows = [7, 8, 9];
        r.next_buff = 4;
        r.tick(&[space(true), roll(10, 0)]);
        let ev = r.run(15);
        let rows: Vec<_> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                MoveEvent::BuffAdded { index, row, .. } => Some((*index, *row)),
                _ => None,
            })
            .collect();
        assert_eq!(rows, [(4, 7), (5, 8), (6, 9)]);
        assert_eq!(r.next_buff, 7);
    }
}
