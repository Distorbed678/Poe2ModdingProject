//! Wire glue between a session and the `poe-game` movement simulation.
//!
//! Two pure mappings and nothing else, since the `World` lives in the instance actor and serves
//! every session of the instance:
//! - [`client_input`]: a client packet (by its name in the opcode table) to a
//!   [`MoveInput`];
//! - [`Codec::encode`]: a [`MoveEvent`] of an entity to the official server messages (opcodes
//!   from `instance.toml` through the protocol table, stat keys from game data).
//!
//! The mapping table is in docs/architecture/instance-world.md; the sequences behind it in
//! docs/protocol/movement.md.

use std::sync::Arc;

use poe_game::locomotion::{MoveEvent, MoveInput, UseSkill as SimUseSkill};
use poe_net::{DecodeError, Reader, Writer};
use poe_proto::{
    Direction, Message, Protocol,
    entity::{PLAYER_COMPONENT, PlayerDelta},
    instance::{
        AimTarget, ComponentDelta, MoveDirection, MoveDirectionUpdate, SetAimTarget,
        SimulationHeader, SkillReleased, SprintInput, SprintState, StartAction, StatsUpdate,
        UpdateSkillTarget, UseSkill,
    },
    skill::{
        BuffAdded, BuffRemoved, SkillInputEnd, SkillInputEndAck, SkillReleasedAck, SkillUseParams,
        UpdateSkillTargetAck, UseSkillAck,
    },
};
use tracing::{debug, warn};

/// Index of the Actor component in the player's object (Positioned 0 .. Actor 9 .. BuffOrbs 12,
/// as the DLL's component inspector lists them).
const ACTOR_COMPONENT: u8 = 9;
/// Last byte of `0x64` on official servers (unread by the client).
const MOVE_UPDATE_TRAILER: u8 = 0xf1;

/// Maps a stat id (`Stats.Id`) to its network key (`Stats.dat` row + 1).
pub type StatKeys = Arc<dyn Fn(&str) -> Option<u32> + Send + Sync>;

/// `0x1ca`: an action event without a body (with every roll start and when an attack ends a
/// sprint). It has no entry in the protocol table's `instance.toml` yet, so
/// [`Codec::encode`] skips it until `ActionEvent = 0x01ca` is added there (the client only
/// decrements a counter for it, Actor case `0x14214a13e`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActionEvent {
    pub header: SimulationHeader,
}

impl Message for ActionEvent {
    const NAME: &'static str = "ActionEvent";
    const DIRECTION: Direction = Direction::Server;
    fn encode(&self, w: &mut Writer) {
        self.header.encode(w);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            header: SimulationHeader::decode(r)?,
        })
    }
}

/// One message for the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Out {
    /// Message name in the opcode table.
    pub name: &'static str,
    /// Opcode and body.
    pub bytes: Vec<u8>,
    /// Movement trace line (`runtime/movement-trace.tsv`): kind and tab-separated fields.
    pub trace: (&'static str, String),
}

/// Client packet to simulation input. `name` is the packet's name in the client opcode table and
/// `r` is positioned after the opcode. `None`: not a movement packet (nothing consumed).
pub fn client_input(name: &str, r: &mut Reader<'_>) -> Result<Option<MoveInput>, DecodeError> {
    Ok(Some(match name {
        MoveDirection::NAME => {
            let m = MoveDirection::decode(r)?;
            MoveInput::Direction { x: m.x, y: m.y }
        }
        UseSkill::NAME => {
            let m = UseSkill::decode(r)?;
            MoveInput::UseSkill(SimUseSkill {
                skill: m.skill,
                flags: m.flags,
                unk: m.unk_13,
                target_flags: m.target_flags,
                entity: m.target_entity,
                x: m.x,
                y: m.y,
            })
        }
        UpdateSkillTarget::NAME => {
            let m = UpdateSkillTarget::decode(r)?;
            MoveInput::UpdateSkillTarget {
                x: m.x,
                y: m.y,
                skill: m.skill,
            }
        }
        SkillInputEnd::NAME => {
            SkillInputEnd::decode(r)?;
            MoveInput::SkillInputEnd
        }
        SkillReleased::NAME => {
            let m = SkillReleased::decode(r)?;
            MoveInput::SkillReleased {
                skill: m.skill,
                flag: m.unk,
            }
        }
        SprintInput::NAME => MoveInput::SprintInput {
            held: SprintInput::decode(r)?.held,
        },
        AimTarget::NAME => {
            let m = AimTarget::decode(r)?;
            match (m.mode, m.cell, m.entity) {
                (2, Some((x, y)), _) => MoveInput::Aim { x, y },
                (3, _, Some(entity)) => MoveInput::AimEntity { entity },
                (mode, _, _) => MoveInput::AimMode { mode },
            }
        }
        _ => return Ok(None),
    }))
}

/// Turns simulation events into server messages.
#[derive(Clone)]
pub struct Codec {
    pub protocol: Arc<Protocol>,
    pub stat_key: StatKeys,
}

impl Codec {
    pub fn new(protocol: Arc<Protocol>, stat_key: StatKeys) -> Self {
        Self { protocol, stat_key }
    }

    /// The messages for `event` of `entity`, in send order (empty for events the protocol
    /// table cannot express yet).
    pub fn encode(&self, entity: u32, event: &MoveEvent) -> anyhow::Result<Vec<Out>> {
        let plain = SimulationHeader {
            entity_id: entity,
            ..Default::default()
        };
        // Official: the `0x64` is ordered after the entity's own Actor component.
        let actor = SimulationHeader {
            entity_id: entity,
            unk_14: entity,
            unk_18: ACTOR_COMPONENT,
            unk_1a: 0,
        };
        let ack = |dir: (i32, i32), cell: (i32, i32)| MoveDirectionUpdate {
            header: actor,
            dir_x: dir.0,
            dir_y: dir.1,
            grid_x: cell.0,
            grid_y: cell.1,
            unk_30: MOVE_UPDATE_TRAILER,
        };
        let mut out = Vec::new();
        match event {
            MoveEvent::MoveAck { dir, cell } => out.push(self.pack(
                &ack(*dir, *cell),
                "out-dir",
                format!("{}\t{}\t{}\t{}", dir.0, dir.1, cell.0, cell.1),
            )?),
            MoveEvent::ActionStart {
                kind: _,
                skill,
                flags,
                action,
                cell,
                target,
                entity: target_entity,
                unk,
            } => out.push(self.pack(
                &StartAction {
                    header: plain,
                    flags: *flags,
                    target_entity: *target_entity,
                    grid_x: cell.0,
                    grid_y: cell.1,
                    target_x: target.0,
                    target_y: target.1,
                    skill: *skill,
                    action_id: *action,
                    unk_3c: *unk,
                },
                "out-start",
                format!(
                    "{}\t{}\t{}\t{}\t{:#x}\t{}",
                    cell.0, cell.1, target.0, target.1, skill, action
                ),
            )?),
            MoveEvent::ActionEvent => {
                let table = &self.protocol.instance;
                if table.id(ActionEvent::DIRECTION, ActionEvent::NAME).is_ok() {
                    out.push(self.pack(
                        &ActionEvent { header: plain },
                        "out-event",
                        String::new(),
                    )?);
                } else {
                    debug!("ActionEvent (0x1ca) is not in instance.toml: not sent");
                }
            }
            MoveEvent::Stats(stats) => {
                let mut keyed = Vec::with_capacity(stats.len());
                for (stat, value) in stats {
                    match (self.stat_key)(stat.id()) {
                        Some(key) => keyed.push((key, *value)),
                        None => warn!(stat = stat.id(), "movement stat not in Stats.dat: skipped"),
                    }
                }
                let fields = format!("{keyed:?}");
                out.push(self.pack(
                    &StatsUpdate {
                        header: plain,
                        set: 0,
                        stats: keyed,
                    },
                    "out-stats",
                    fields,
                )?);
            }
            MoveEvent::SkillAck {
                skill,
                flags,
                unk,
                target_flags,
                entity: target_entity,
                x,
                y,
            } => out.push(self.pack(
                &UseSkillAck {
                    header: plain,
                    params: SkillUseParams {
                        skill: *skill,
                        flags: *flags,
                        unk_13: *unk,
                        target_flags: target_flags | SkillUseParams::CONFIRMED,
                        target_entity: *target_entity,
                        x: *x,
                        y: *y,
                    },
                },
                "out-skill-ack",
                format!("{skill:#x}\t{target_flags:#x}\t{x}\t{y}"),
            )?),
            MoveEvent::TargetAck { skill, x, y } => out.push(self.pack(
                &UpdateSkillTargetAck {
                    header: plain,
                    x: *x,
                    y: *y,
                    skill: *skill,
                },
                "out-target",
                format!("{x}\t{y}\t{skill:#x}"),
            )?),
            MoveEvent::InputEndAck => out.push(self.pack(
                &SkillInputEndAck { header: plain },
                "out-input-end",
                String::new(),
            )?),
            MoveEvent::ReleasedAck { skill, flag } => out.push(self.pack(
                &SkillReleasedAck {
                    header: plain,
                    skill: *skill,
                    flag: *flag,
                },
                "out-released",
                format!("{skill:#x}\t{flag}"),
            )?),
            MoveEvent::SprintState { held } => out.push(self.pack(
                &SprintState {
                    header: plain,
                    held: *held,
                },
                "out-sprint",
                held.to_string(),
            )?),
            MoveEvent::BuffAdded { index, row, fade } => out.push(
                self.pack(
                    &BuffAdded {
                        header: plain,
                        index: *index,
                        buff: *row,
                        flag_02: fade.map(f32::to_bits),
                        ..Default::default()
                    }
                    .with_flags(),
                    "out-buff-added",
                    format!("{index}\t{row}\t{fade:?}"),
                )?,
            ),
            MoveEvent::BuffRemoved { index } => out.push(self.pack(
                &BuffRemoved {
                    header: plain,
                    index: *index,
                },
                "out-buff-removed",
                index.to_string(),
            )?),
            MoveEvent::AimEcho { cell } => out.push(self.pack(
                &SetAimTarget {
                    header: plain,
                    kind: 0,
                    x: cell.0,
                    y: cell.1,
                },
                "out-aim",
                format!("0\t{}\t{}", cell.0, cell.1),
            )?),
            MoveEvent::StanceChanged { combat } => out.push(
                self.pack(
                    &ComponentDelta {
                        // Official: after all entities (order key 0xffffffff).
                        header: SimulationHeader {
                            entity_id: entity,
                            unk_14: 0xffff_ffff,
                            unk_18: 0,
                            unk_1a: 0,
                        },
                        component: PLAYER_COMPONENT,
                        delta: PlayerDelta {
                            combat_stance: Some(*combat),
                            ..Default::default()
                        }
                        .encode(),
                    },
                    "out-component",
                    format!("stance\t{combat}"),
                )?,
            ),
            // The client checks the cell of a direction update only for a non-zero direction,
            // and in lockstep puts the entity on that cell's centre: a minimal direction with
            // the cell, then a stop, in the same frame.
            MoveEvent::Teleport { cell } => {
                for dir_x in [1, 0] {
                    out.push(self.pack(
                        &ack((dir_x, 0), *cell),
                        "out-dir",
                        format!("{dir_x}\t0\t{}\t{}", cell.0, cell.1),
                    )?);
                }
            }
        }
        Ok(out)
    }

    fn pack<M: Message>(
        &self,
        message: &M,
        kind: &'static str,
        fields: String,
    ) -> anyhow::Result<Out> {
        Ok(Out {
            name: M::NAME,
            bytes: self.protocol.instance.encode(message)?,
            trace: (kind, fields),
        })
    }
}
