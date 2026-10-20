//! The instance world: entities with unique ids, one frame clock, and their components.
//!
//! One [`World`] belongs to one instance. Sessions feed it client input
//! ([`World::input`]), the instance's tick loop calls [`World::tick`] once per frame, and the
//! returned [`WorldEvent`]s are what the clients must be told, in order. The world never
//! reads a clock: the server owns timing and calls `tick` every `tick_ms`.
//!
//! Entities are plain structs with their components as fields. Today the only component is
//! [`Locomotion`] (server-authoritative movement); more (stats, skills, AI) get their own
//! fields and event variants. The world is not tied to one player: every player entity of the
//! instance is spawned into the same world and gets its own movement, which is what
//! multiplayer reuses.

use std::{collections::BTreeMap, sync::Arc};

use crate::{
    grid::GridSource,
    locomotion::{Cell, Locomotion, MoveEvent, MoveInput, MovementConfig, StepCtx},
};

/// An entity id, unique within a world (the protocol's `u32` entity id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub u32);

#[derive(Debug, Clone)]
pub struct WorldConfig {
    /// Milliseconds per frame (`tick_ms`); the simulation advances `tick_ms / 1000` seconds per
    /// tick.
    pub tick_ms: u32,
    /// The first entity id handed out.
    pub first_entity_id: u32,
    /// The first `StartAction` id (one counter for the whole world, shared by all entities).
    pub first_action_id: u16,
    pub movement: MovementConfig,
}

impl WorldConfig {
    /// Official defaults: 33 ms frames, entity ids from 1, action ids from 32768.
    pub fn new(movement: MovementConfig) -> Self {
        Self {
            tick_ms: 33,
            first_entity_id: 1,
            first_action_id: 32768,
            movement,
        }
    }
}

/// An entity and its components.
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: EntityId,
    /// Next free buff index of this entity (shared by all components that add buffs).
    pub next_buff: u16,
    pub movement: Locomotion,
}

/// Something that happened to an entity this frame and that clients must be told.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    Movement(MoveEvent),
}

/// An [`Event`] with the frame (batch) it belongs to and the entity it concerns.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldEvent {
    pub frame: u64,
    pub entity: EntityId,
    pub event: Event,
}

impl WorldEvent {
    pub fn movement(&self) -> Option<&MoveEvent> {
        match &self.event {
            Event::Movement(m) => Some(m),
        }
    }
}

pub struct World {
    config: WorldConfig,
    grid: Arc<dyn GridSource>,
    frame: u64,
    next_entity: u32,
    next_action: u16,
    entities: BTreeMap<EntityId, Entity>,
    paused: bool,
}

impl World {
    pub fn new(config: WorldConfig, grid: Arc<dyn GridSource>) -> Self {
        Self {
            next_entity: config.first_entity_id,
            next_action: config.first_action_id,
            config,
            grid,
            frame: 0,
            entities: BTreeMap::new(),
            paused: false,
        }
    }

    /// Frames ticked so far (the number of the last batch).
    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn tick_ms(&self) -> u32 {
        self.config.tick_ms
    }

    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    pub fn grid(&self) -> &Arc<dyn GridSource> {
        &self.grid
    }

    /// Swaps the terrain (the area's grid arrived after the world was created).
    pub fn set_grid(&mut self, grid: Arc<dyn GridSource>) {
        self.grid = grid;
    }

    /// While paused, frames still count but nothing moves and nothing is answered; inputs
    /// wait. (Solo escape menu.)
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Adds a player entity standing on `cell`, walking at the configured speed.
    pub fn spawn_player(&mut self, cell: Cell) -> EntityId {
        let speed = self.config.movement.walk_speed;
        self.spawn_player_with_speed(cell, speed)
    }

    /// Like [`spawn_player`](Self::spawn_player) with an explicit speed (cells per second,
    /// from the character's stats).
    pub fn spawn_player_with_speed(&mut self, cell: Cell, speed: f32) -> EntityId {
        let id = EntityId(self.next_entity);
        self.next_entity += 1;
        self.entities.insert(
            id,
            Entity {
                id,
                next_buff: 0,
                movement: Locomotion::new(cell, speed),
            },
        );
        id
    }

    pub fn despawn(&mut self, id: EntityId) -> Option<Entity> {
        self.entities.remove(&id)
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(&id)
    }

    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    /// Queues client input for an entity; it is answered in the next tick. Returns whether the
    /// entity exists.
    pub fn input(&mut self, id: EntityId, input: MoveInput) -> bool {
        match self.entities.get_mut(&id) {
            Some(e) => {
                e.movement.input(input);
                true
            }
            None => false,
        }
    }

    /// Places an entity (the next tick reports it with [`MoveEvent::Teleport`]).
    pub fn teleport(&mut self, id: EntityId, cell: Cell) -> bool {
        match self.entities.get_mut(&id) {
            Some(e) => {
                e.movement.teleport(cell);
                true
            }
            None => false,
        }
    }

    /// Advances one frame and returns what happened, ordered by entity id and within an entity
    /// in the order the messages are sent.
    pub fn tick(&mut self) -> Vec<WorldEvent> {
        self.frame += 1;
        if self.paused {
            return Vec::new();
        }
        let dt = self.config.tick_ms as f32 / 1000.0;
        let mut out = Vec::new();
        for e in self.entities.values_mut() {
            let mut ctx = StepCtx {
                frame: self.frame,
                dt,
                terrain: &*self.grid,
                config: &self.config.movement,
                next_action: &mut self.next_action,
                next_buff: &mut e.next_buff,
            };
            for event in e.movement.tick(&mut ctx) {
                out.push(WorldEvent {
                    frame: self.frame,
                    entity: e.id,
                    event: Event::Movement(event),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        grid::VecGrid,
        locomotion::{SkillIds, UseSkill},
        movement::Open,
    };

    const ROLL: u32 = 0x8000_0040;

    fn world() -> World {
        let cfg = MovementConfig::new(
            SkillIds {
                move_skill: 0x2000_2909,
                roll_skill: ROLL,
            },
            [1, 2, 3],
        );
        World::new(WorldConfig::new(cfg), Arc::new(Open))
    }

    fn roll() -> MoveInput {
        MoveInput::UseSkill(UseSkill {
            skill: ROLL,
            flags: 0x400,
            unk: 0xff,
            target_flags: 0,
            entity: None,
            x: 20,
            y: 0,
        })
    }

    #[test]
    fn entity_ids_are_unique_per_world_and_start_where_configured() {
        let mut w = world();
        let a = w.spawn_player((0, 0));
        let b = w.spawn_player((5, 5));
        assert_eq!((a, b), (EntityId(1), EntityId(2)));
        w.despawn(a);
        // Ids are not reused.
        assert_eq!(w.spawn_player((1, 1)), EntityId(3));
        assert_eq!(w.entities().count(), 2);
        assert!(!w.input(a, MoveInput::SkillInputEnd));
        let mut cfg = world().config.clone();
        cfg.first_entity_id = 100;
        let mut other = World::new(cfg, Arc::new(Open));
        assert_eq!(other.spawn_player((0, 0)), EntityId(100));
    }

    #[test]
    fn the_frame_clock_is_owned_by_the_caller() {
        let mut w = world();
        let p = w.spawn_player((10, 10));
        assert_eq!(w.frame(), 0);
        assert!(w.tick().is_empty());
        assert_eq!(w.frame(), 1);
        w.input(p, MoveInput::Direction { x: 180, y: 0 });
        // Input is answered by the next tick, stamped with its frame.
        let ev = w.tick();
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].frame, ev[0].entity), (2, p));
        assert!(matches!(
            ev[0].movement(),
            Some(MoveEvent::MoveAck {
                dir: (180, 0),
                cell: (10, 10)
            })
        ));
        // `tick_ms` sets the step.
        let x0 = w.entity(p).unwrap().movement.position().0;
        w.tick();
        let step = w.entity(p).unwrap().movement.position().0 - x0;
        assert!((step - 37.0 * 0.033).abs() < 1e-3);
    }

    #[test]
    fn tick_ms_is_configurable() {
        let mut w = world();
        w.config.tick_ms = 50;
        let p = w.spawn_player((10, 10));
        w.input(p, MoveInput::Direction { x: 180, y: 0 });
        w.tick();
        let x0 = w.entity(p).unwrap().movement.position().0;
        w.tick();
        let step = w.entity(p).unwrap().movement.position().0 - x0;
        assert!((step - 37.0 * 0.05).abs() < 1e-3, "{step}");
    }

    #[test]
    fn players_move_independently_and_events_name_their_entity() {
        let mut w = world();
        let a = w.spawn_player((10, 10));
        let b = w.spawn_player((100, 100));
        w.input(a, MoveInput::Direction { x: 0, y: 180 });
        w.input(b, roll());
        let ev = w.tick();
        assert!(
            ev.iter()
                .any(|e| e.entity == a && matches!(e.movement(), Some(MoveEvent::MoveAck { .. })))
        );
        assert!(
            ev.iter()
                .any(|e| e.entity == b
                    && matches!(e.movement(), Some(MoveEvent::ActionStart { .. })))
        );
        // Events come entity by entity.
        let order: Vec<_> = ev.iter().map(|e| e.entity).collect();
        assert!(order.windows(2).all(|p| p[0] <= p[1]));
        for _ in 0..10 {
            w.tick();
        }
        assert_eq!(w.entity(a).unwrap().movement.cell().0, 10);
        assert!(w.entity(a).unwrap().movement.cell().1 > 14);
        assert!(w.entity(b).unwrap().movement.cell().0 > 110);
        assert_eq!(w.entity(b).unwrap().movement.cell().1, 100);
    }

    #[test]
    fn action_ids_are_one_counter_for_the_world() {
        let mut w = world();
        let a = w.spawn_player((0, 0));
        let b = w.spawn_player((50, 50));
        w.input(a, roll());
        w.input(b, roll());
        let ids: Vec<u16> = w
            .tick()
            .iter()
            .filter_map(|e| match e.movement() {
                Some(MoveEvent::ActionStart { action, .. }) => Some(*action),
                _ => None,
            })
            .collect();
        assert_eq!(ids, [32768, 32769]);
    }

    #[test]
    fn each_player_has_own_buff_indices() {
        let mut w = world();
        let a = w.spawn_player((0, 0));
        let b = w.spawn_player((50, 50));
        for p in [a, b] {
            w.input(p, MoveInput::SprintInput { held: true });
            w.input(p, roll());
        }
        let mut rows = Vec::new();
        for _ in 0..16 {
            for e in w.tick() {
                if let Some(MoveEvent::BuffAdded { index, row, .. }) = e.movement() {
                    rows.push((e.entity, *index, *row));
                }
            }
        }
        assert_eq!(
            rows,
            [
                (a, 0, 1),
                (a, 1, 2),
                (a, 2, 3),
                (b, 0, 1),
                (b, 1, 2),
                (b, 2, 3)
            ]
        );
    }

    #[test]
    fn pause_freezes_the_simulation_but_not_the_clock() {
        let mut w = world();
        let p = w.spawn_player((10, 10));
        w.input(p, MoveInput::Direction { x: 180, y: 0 });
        w.set_paused(true);
        assert!(w.tick().is_empty());
        assert_eq!(w.frame(), 1);
        w.set_paused(false);
        assert_eq!(w.tick().len(), 1, "the input waited");
    }

    #[test]
    fn teleport_and_grid_swap() {
        let mut w = world();
        let p = w.spawn_player((10, 10));
        assert!(w.teleport(p, (40, 40)));
        let ev = w.tick();
        assert!(matches!(
            ev[0].movement(),
            Some(MoveEvent::Teleport { cell: (40, 40) })
        ));
        // A grid that arrives later blocks movement.
        let mut grid = VecGrid::open(100, 100);
        for y in 0..100 {
            grid.set(45, y, false);
        }
        w.set_grid(Arc::new(grid));
        w.input(p, MoveInput::Direction { x: 180, y: 0 });
        for _ in 0..30 {
            w.tick();
        }
        assert!(w.entity(p).unwrap().movement.cell().0 < 45);
    }
}
