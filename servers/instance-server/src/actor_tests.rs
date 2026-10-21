//! The instance actor without a game client: fake sessions (an output channel and the encoded
//! packets they would write to the client) against one [`Actor`].
//!
//! [`Fixture`] drives the actor synchronously (`handle`, `tick`), which gives exact frames; the
//! `async` tests at the end run the real task (`Actors::join`, timers, the session's [`Stops`])
//! with paused time.

use std::{sync::Arc, time::Duration};

use poe_cluster::{
    InstanceDirectory, InstanceSpec, LoginSessions, MemoryInstances, MemorySessions,
};
use poe_game::locomotion::MovementConfig;
use poe_net::Reader;
use poe_proto::{
    Direction, Message, Protocol,
    instance::{
        AddObject, ComponentDelta, MoveDirection, MoveDirectionUpdate, SetAimTarget, SetPause,
        SimulationTick, SprintInput, SprintState, StartAction, StatsUpdate, UseSkill,
    },
    player_blob::PlayerBlob,
    remote::{DestroyEntity, EntitySleep, PresenceChanged, RemoveObject},
    skill::{
        BuffAdded, BuffRemoved, SkillInputEndAck, SkillReleasedAck, UpdateSkillTargetAck,
        UseSkillAck,
    },
};
use tokio::sync::{mpsc, oneshot};

use crate::{
    Stop, Stops,
    actor::{Actor, ActorInit, Actors, Appearance, Cmd, Frame, JoinRequest, Joined, SessionId},
    appearance::{RemoteInit, remote_blob},
    live::Live,
    movement::{ActionEvent, Codec, client_input},
    player::Player,
    replay_tests::{ROLL_SKILL, movement_config, protocol, roll_skill, stat_keys},
};

/// One decoded server message.
pub(crate) struct Sent {
    pub name: &'static str,
    pub bytes: Vec<u8>,
}

impl Sent {
    pub fn is<M: Message>(&self) -> bool {
        self.name == M::NAME
    }

    pub fn decode<M: Message>(&self) -> M {
        assert!(self.is::<M>(), "{} is not a {}", self.name, M::NAME);
        let mut r = Reader::new(&self.bytes[2..]);
        let m = M::decode(&mut r).unwrap();
        assert!(r.is_empty(), "trailing bytes in {}", self.name);
        m
    }
}

/// Splits a stream of server messages (the tests know every message type the actor sends).
pub(crate) fn split(protocol: &Protocol, mut bytes: &[u8]) -> Vec<Sent> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let op = u16::from_be_bytes([bytes[0], bytes[1]]);
        let name = protocol
            .instance
            .name(Direction::Server, op)
            .unwrap_or_else(|| panic!("server opcode {op:#06x}"));
        let mut r = Reader::new(&bytes[2..]);
        macro_rules! known {
            ($($t:ty),* $(,)?) => {
                match name {
                    $(<$t>::NAME => {
                        <$t>::decode(&mut r).unwrap();
                        <$t>::NAME
                    })*
                    other => panic!("the tests cannot split {other}"),
                }
            };
        }
        let name = known!(
            MoveDirectionUpdate,
            StartAction,
            ActionEvent,
            StatsUpdate,
            UseSkillAck,
            UpdateSkillTargetAck,
            SkillInputEndAck,
            SkillReleasedAck,
            SprintState,
            BuffAdded,
            BuffRemoved,
            SetAimTarget,
            ComponentDelta,
            AddObject,
            PresenceChanged,
            DestroyEntity,
            EntitySleep,
            RemoveObject,
            SetPause,
            SimulationTick,
        );
        let len = 2 + r.position();
        out.push(Sent {
            name,
            bytes: bytes[..len].to_vec(),
        });
        bytes = &bytes[len..];
    }
    out
}

/// A session without a client: what the actor sends it.
pub(crate) struct Fake {
    pub session: SessionId,
    pub entity: u32,
    pub rx: mpsc::UnboundedReceiver<Frame>,
    pub joined: Joined,
}

/// What one frame holds for a fake session.
pub(crate) struct Got {
    pub head: Vec<Sent>,
    /// The events, without the closing marker.
    pub events: Vec<Sent>,
}

impl Got {
    pub fn names(&self) -> Vec<&'static str> {
        self.events.iter().map(|s| s.name).collect()
    }
}

impl Fake {
    /// The frame the last tick produced (exactly one must be queued).
    pub fn got(&mut self, protocol: &Protocol) -> Got {
        let frame = self.rx.try_recv().expect("a frame was sent");
        assert!(self.rx.try_recv().is_err(), "one frame per tick");
        let head = split(protocol, &frame.head);
        let mut events = split(protocol, &frame.events);
        let marker = events.pop().expect("the tick marker");
        assert!(
            marker.is::<SimulationTick>(),
            "a frame ends with the tick marker"
        );
        Got { head, events }
    }

    /// The events of the last tick (the head must be empty).
    pub fn take(&mut self, protocol: &Protocol) -> Vec<Sent> {
        let got = self.got(protocol);
        assert!(got.head.is_empty(), "unexpected lifecycle messages");
        got.events
    }
}

/// An actor, a directory with its instance and the shared protocol table.
pub(crate) struct Fixture {
    pub actor: Actor,
    pub protocol: Arc<Protocol>,
    pub directory: Arc<MemoryInstances>,
    pub instance: u32,
    next_session: SessionId,
}

pub(crate) const FIRST_ENTITY: u32 = 12000;

pub(crate) fn actor_init(
    protocol: &Arc<Protocol>,
    directory: &Arc<MemoryInstances>,
    instance: u32,
) -> ActorInit {
    ActorInit {
        instance,
        area: "G1_town".into(),
        tick_ms: 33,
        first_entity_id: FIRST_ENTITY,
        movement: movement_config(&[roll_skill(None)]),
        grid: None,
        codec: Codec::new(protocol.clone(), stat_keys()),
        directory: directory.clone(),
        idle: Duration::ZERO,
        trace: true,
        player_spacing: 0,
    }
}

pub(crate) fn join_request(
    session: SessionId,
    name: &str,
    cell: (i32, i32),
    movement: MovementConfig,
) -> (JoinRequest, mpsc::UnboundedReceiver<Frame>) {
    let (out, rx) = mpsc::unbounded_channel();
    let remote = remote_blob(&RemoteInit {
        name,
        class: 2,
        cell,
        team: 1,
        stats: vec![(1, 10)],
        life: 100,
        mana: 50,
        animation_sets: ["UnarmedTown".into(), "UnarmedTown".into(), "default".into()],
        skills: Vec::new(),
        ..Default::default()
    });
    (
        JoinRequest {
            session,
            name: name.to_owned(),
            spawn: cell,
            speed: 37.0,
            movement,
            grid: None,
            appearance: Appearance {
                type_key: 0x1234,
                local_blob: {
                    let blob = format!("local {name}").into_bytes();
                    Arc::new(move |_| blob.clone())
                },
                remote,
            },
            want_debug: true,
            out,
        },
        rx,
    )
}

impl Fixture {
    pub fn new() -> Self {
        Self::with_first_entity(FIRST_ENTITY)
    }

    pub fn with_first_entity(first: u32) -> Self {
        Self::customised(first, |init| init)
    }

    /// A fixture whose actor is created from the standard test init changed by `change`.
    pub fn customised(first: u32, change: impl FnOnce(ActorInit) -> ActorInit) -> Self {
        let protocol = protocol();
        let directory = Arc::new(MemoryInstances::default());
        let instance = directory.create(
            InstanceSpec {
                area: "G1_town".into(),
                ..Default::default()
            },
            true,
        );
        let actor = Actor::new(change(ActorInit {
            first_entity_id: first,
            ..actor_init(&protocol, &directory, instance)
        }))
        .unwrap();
        Self {
            actor,
            protocol,
            directory,
            instance,
            next_session: 1,
        }
    }

    pub fn join(&mut self, name: &str, cell: (i32, i32), movement: MovementConfig) -> Fake {
        self.join_with(name, cell, movement, 37.0)
    }

    fn join_with(
        &mut self,
        name: &str,
        cell: (i32, i32),
        movement: MovementConfig,
        speed: f32,
    ) -> Fake {
        let session = self.next_session;
        self.next_session += 1;
        let (mut request, rx) = join_request(session, name, cell, movement);
        request.speed = speed;
        let (reply, mut joined) = oneshot::channel();
        self.actor.handle(Cmd::Join {
            request: Box::new(request),
            reply,
        });
        Fake {
            session,
            entity: 0,
            rx,
            joined: joined.try_recv().expect("the actor answered the join"),
        }
        .with_entity()
    }

    pub fn join_speed(&mut self, name: &str, cell: (i32, i32), speed: f32) -> Fake {
        self.join_with(name, cell, movement_config(&[roll_skill(None)]), speed)
    }

    /// A character with the default test movement (a dodge roll skill).
    pub fn join_default(&mut self, name: &str, cell: (i32, i32)) -> Fake {
        self.join(name, cell, movement_config(&[roll_skill(None)]))
    }

    pub fn tick(&mut self) {
        self.actor.tick();
    }

    /// A client packet of `who`, through the opcode table and [`client_input`] as the server
    /// does.
    pub fn send<M: Message>(&mut self, who: &Fake, message: &M) {
        let bytes = self.protocol.instance.encode(message).unwrap();
        let op = u16::from_be_bytes([bytes[0], bytes[1]]);
        let name = self
            .protocol
            .instance
            .name(Direction::Client, op)
            .expect("client opcode");
        assert_eq!(name, M::NAME);
        let mut r = Reader::new(&bytes[2..]);
        let input = client_input(name, &mut r)
            .unwrap()
            .unwrap_or_else(|| panic!("{name} is not movement input"));
        assert!(r.is_empty());
        self.actor.handle(Cmd::Input {
            session: who.session,
            input,
        });
    }

    pub fn cell(&self, who: &Fake) -> (i32, i32) {
        self.actor
            .world()
            .entity(poe_game::world::EntityId(who.entity))
            .expect("entity")
            .movement
            .cell()
    }

    pub fn teleport(&mut self, who: &Fake, cell: (i32, i32)) {
        self.actor.handle(Cmd::Teleport {
            session: who.session,
            cell,
        });
    }

    pub fn leave(&mut self, who: &Fake) -> Option<(i32, i32)> {
        let (reply, mut cell) = oneshot::channel();
        self.actor.handle(Cmd::Leave {
            session: who.session,
            reply: Some(reply),
        });
        cell.try_recv().expect("the actor answered the leave")
    }

    pub fn walk(&mut self, who: &Fake, x: i32, y: i32) {
        self.send(
            who,
            &MoveDirection {
                unk_0: 1,
                x,
                y,
                unk_tail: [0; 3],
            },
        );
    }

    pub fn roll(&mut self, who: &Fake, x: i32, y: i32) {
        self.send(
            who,
            &UseSkill {
                player: 1,
                skill: ROLL_SKILL,
                flags: 0x400,
                unk_13: 0xff,
                target_flags: 0,
                target_entity: None,
                x,
                y,
            },
        );
    }
}

impl Fake {
    fn with_entity(mut self) -> Self {
        self.entity = self.joined.entity;
        self
    }

    /// The `AddObject`s of the entry batch, decoded.
    pub fn entry(&self, protocol: &Protocol) -> Vec<AddObject> {
        split(protocol, &self.joined.add_objects)
            .iter()
            .map(|s| s.decode::<AddObject>())
            .collect()
    }
}

#[test]
fn entity_ids_come_from_the_instance_counter() {
    let mut fx = Fixture::new();
    let a = fx.join_default("Alice", (100, 100));
    let b = fx.join_default("Bob", (110, 100));
    assert_eq!((a.entity, b.entity), (FIRST_ENTITY, FIRST_ENTITY + 1));
    // Another instance counts on its own.
    let mut other = Fixture::new();
    assert_eq!(other.join_default("Carol", (1, 1)).entity, FIRST_ENTITY);
}

#[test]
fn the_entry_batch_lists_the_players_already_there_then_the_newcomer() {
    let mut fx = Fixture::new();
    let a = fx.join_default("Alice", (100, 100));
    // Alone: only itself, in the local form.
    let entry = a.entry(&fx.protocol);
    assert_eq!(entry.len(), 1);
    assert_eq!(entry[0].header.entity_id, a.entity);
    assert_eq!(entry[0].header.unk_14, 0xffff_ffff);
    assert_eq!(entry[0].blob, b"local Alice");
    assert_eq!(a.joined.players, 1);

    fx.walk(&a, 180, 0);
    for _ in 0..10 {
        fx.tick();
    }
    let b = fx.join_default("Bob", (300, 300));
    let entry = b.entry(&fx.protocol);
    assert_eq!(entry.len(), 2);
    // Alice first, in the remote form with her current cell; then Bob in the local form.
    assert_eq!(entry[0].header.entity_id, a.entity);
    assert_eq!(entry[0].type_key, 0x1234);
    let blob = PlayerBlob::decode(&entry[0].blob).unwrap();
    assert_eq!(blob.player.name, "Alice");
    assert_eq!((blob.grid_x, blob.grid_y), fx.cell(&a));
    assert!(blob.grid_x > 100, "Alice walked on: {}", blob.grid_x);
    assert_eq!(blob.flags, 2);
    assert_eq!(
        (entry[1].header.entity_id, entry[1].blob.as_slice()),
        (b.entity, &b"local Bob"[..])
    );
    assert_eq!(b.joined.players, 2);
}

#[test]
fn existing_sessions_get_one_add_object_for_the_newcomer() {
    let mut fx = Fixture::new();
    let mut a = fx.join_default("Alice", (100, 100));
    let mut b = fx.join_default("Bob", (140, 90));
    fx.tick();
    let got = a.got(&fx.protocol);
    // One AddObject for Bob (header (id, 0xffffffff, 0, 0)), nothing else about him.
    assert_eq!(got.head.len(), 1);
    let add = got.head[0].decode::<AddObject>();
    assert_eq!(
        (
            add.header.entity_id,
            add.header.unk_14,
            add.header.unk_18,
            add.header.unk_1a
        ),
        (b.entity, 0xffff_ffff, 0, 0)
    );
    let blob = PlayerBlob::decode(&add.blob).unwrap();
    assert_eq!(blob.player.name, "Bob");
    assert_eq!((blob.grid_x, blob.grid_y), (140, 90));
    assert!(got.events.is_empty());
    // The newcomer gets no lifecycle message about itself.
    assert!(b.got(&fx.protocol).head.is_empty());
    // And it is sent once.
    fx.tick();
    assert!(a.got(&fx.protocol).head.is_empty());
}

#[test]
fn a_late_joiner_sees_the_stance_of_the_player_who_was_there() {
    let mut fx = Fixture::new();
    let a = fx.join_default("Alice", (100, 100));
    fx.send(
        &a,
        &poe_proto::instance::AimTarget {
            local_index: 1,
            mode: 1,
            cell: None,
            entity: None,
        },
    );
    for _ in 0..40 {
        fx.tick();
    }
    let in_stance = fx
        .actor
        .world()
        .entity(poe_game::world::EntityId(a.entity))
        .unwrap()
        .movement
        .in_combat_stance();
    let b = fx.join_default("Bob", (110, 110));
    let blob = PlayerBlob::decode(&b.entry(&fx.protocol)[0].blob).unwrap();
    assert_eq!(blob.player.flags, if in_stance { 0x0c } else { 0 });
}

#[test]
fn a_walk_reaches_the_other_session_in_the_same_frame_with_the_official_header() {
    let mut fx = Fixture::new();
    let mut a = fx.join_default("Alice", (100, 100));
    let mut b = fx.join_default("Bob", (300, 300));
    fx.tick();
    a.got(&fx.protocol);
    b.got(&fx.protocol);
    fx.walk(&a, 180, 0);
    fx.tick();
    let (got_a, got_b) = (a.got(&fx.protocol), b.got(&fx.protocol));
    assert!(got_a.head.is_empty() && got_b.head.is_empty());
    assert_eq!(got_a.names(), ["MoveDirectionUpdate"]);
    assert_eq!(got_b.names(), got_a.names());
    // The same bytes: header (A, A, 9, 0), A's cell. Bob's own session is not involved.
    assert_eq!(got_b.events[0].bytes, got_a.events[0].bytes);
    let m = got_b.events[0].decode::<MoveDirectionUpdate>();
    assert_eq!(
        (
            m.header.entity_id,
            m.header.unk_14,
            m.header.unk_18,
            m.header.unk_1a
        ),
        (a.entity, a.entity, 9, 0)
    );
    assert_eq!((m.dir_x, m.dir_y, m.grid_x, m.grid_y), (180, 0, 100, 100));
    // Bob stands still: nothing about him goes out.
    for _ in 0..5 {
        fx.tick();
        assert!(a.got(&fx.protocol).events.is_empty());
        assert!(b.got(&fx.protocol).events.is_empty());
    }
}

#[test]
fn a_roll_batch_reaches_every_session_in_the_frame_it_is_produced() {
    let mut fx = Fixture::new();
    let mut a = fx.join_default("Alice", (371, 1052));
    let mut b = fx.join_default("Bob", (500, 500));
    fx.tick();
    a.got(&fx.protocol);
    b.got(&fx.protocol);
    fx.send(
        &a,
        &SprintInput {
            player: 1,
            held: true,
        },
    );
    fx.roll(&a, 15, -35);
    fx.tick();
    let (got_a, got_b) = (a.got(&fx.protocol), b.got(&fx.protocol));
    let names = [
        "SprintState",
        "StartAction",
        "ActionEvent",
        "StatsUpdate",
        "UseSkillAck",
        "StatsUpdate",
    ];
    assert_eq!(got_a.names(), names);
    assert_eq!(
        got_b.names(),
        names,
        "the whole batch, with the acks, in one frame"
    );
    for (x, y) in got_a.events.iter().zip(&got_b.events) {
        assert_eq!(x.bytes, y.bytes);
    }
    // All of it is about Alice's entity.
    let start = got_b.events[1].decode::<StartAction>();
    assert_eq!(start.header.entity_id, a.entity);
    assert_eq!((start.skill, start.flags), (ROLL_SKILL, 0x410));
    // Both players roll at once: the action ids come from one counter.
    fx.roll(&b, 0, 40);
    fx.tick();
    let ids: Vec<u16> = a
        .got(&fx.protocol)
        .events
        .iter()
        .filter(|s| s.is::<StartAction>())
        .map(|s| s.decode::<StartAction>().action_id)
        .collect();
    assert_eq!(ids.len(), 1);
    assert!(ids[0] > start.action_id);
}

#[test]
fn the_trace_lines_name_the_entity_they_belong_to() {
    let mut fx = Fixture::new();
    let mut a = fx.join_default("Alice", (100, 100));
    let b = fx.join_default("Bob", (300, 300));
    fx.walk(&b, 0, 180);
    fx.tick();
    let frame = a.rx.try_recv().unwrap();
    assert_eq!(frame.trace.len(), 1);
    assert_eq!(
        (frame.trace[0].entity, frame.trace[0].kind),
        (b.entity, "out-dir")
    );
}

#[test]
fn leaving_tells_the_others_in_one_frame_and_removes_the_entity() {
    let mut fx = Fixture::new();
    let a = fx.join_default("Alice", (100, 100));
    let mut b = fx.join_default("Bob", (300, 300));
    let mut c = fx.join_default("Carol", (320, 300));
    fx.tick();
    b.got(&fx.protocol);
    c.got(&fx.protocol);
    assert_eq!(
        fx.directory.get(fx.instance).unwrap().players,
        ["Alice", "Bob", "Carol"]
    );
    // Alice is mid-walk when she goes; her queued input never runs.
    fx.walk(&a, 180, 0);
    assert_eq!(fx.leave(&a), Some((100, 100)));
    fx.tick();
    for who in [&mut b, &mut c] {
        let got = who.got(&fx.protocol);
        let names: Vec<_> = got.head.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "PresenceChanged",
                "DestroyEntity",
                "EntitySleep",
                "RemoveObject"
            ]
        );
        assert_eq!(got.head[0].decode::<PresenceChanged>().unk, [1, 1, 1, 0]);
        let h = |s: &Sent| match s.name {
            "DestroyEntity" => s.decode::<DestroyEntity>().header,
            "EntitySleep" => s.decode::<EntitySleep>().header,
            _ => s.decode::<RemoveObject>().header,
        };
        for s in &got.head[1..] {
            let header = h(s);
            assert_eq!(header.entity_id, a.entity);
            assert_eq!(
                header.unk_14,
                if s.is::<RemoveObject>() {
                    0xffff_ffff
                } else {
                    0
                }
            );
            assert_eq!((header.unk_18, header.unk_1a), (0, 0));
        }
        assert!(got.events.is_empty(), "nothing more about her");
    }
    assert!(
        fx.actor
            .world()
            .entity(poe_game::world::EntityId(a.entity))
            .is_none()
    );
    assert_eq!(
        fx.directory.get(fx.instance).unwrap().players,
        ["Bob", "Carol"]
    );
    // Gone for good: a second leave and a later input do nothing.
    assert_eq!(fx.leave(&a), None);
    fx.walk(&a, 0, 180);
    fx.tick();
    assert!(b.got(&fx.protocol).head.is_empty());
}

#[test]
fn a_session_whose_channel_closed_is_removed_like_a_leave() {
    let mut fx = Fixture::new();
    let a = fx.join_default("Alice", (100, 100));
    let mut b = fx.join_default("Bob", (300, 300));
    drop(a.rx);
    // The failed send removes Alice after this frame; Bob hears it in the next one.
    fx.tick();
    assert_eq!(fx.actor.player_count(), 1);
    assert!(
        b.got(&fx.protocol)
            .head
            .iter()
            .all(|s| s.name != "RemoveObject")
    );
    fx.tick();
    let names: Vec<_> = b.got(&fx.protocol).head.iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        [
            "PresenceChanged",
            "DestroyEntity",
            "EntitySleep",
            "RemoveObject"
        ]
    );
    assert_eq!(fx.directory.get(fx.instance).unwrap().players, ["Bob"]);
}

#[test]
fn pause_exists_only_for_a_single_player() {
    let mut fx = Fixture::new();
    let mut a = fx.join_default("Alice", (100, 100));
    fx.walk(&a, 180, 0);
    // Alone: the escape menu pauses the world.
    assert!(fx.actor.set_paused(a.session, true));
    fx.tick();
    assert!(a.take(&fx.protocol).is_empty());
    assert!(fx.actor.world().paused());
    // A second player joins: the world runs again and Alice is told with SetPause(false).
    let mut b = fx.join_default("Bob", (300, 300));
    assert!(!fx.actor.world().paused());
    fx.tick();
    let got = a.got(&fx.protocol);
    assert_eq!(
        got.head.iter().map(|s| s.name).collect::<Vec<_>>(),
        ["SetPause", "AddObject"]
    );
    assert!(!got.head[0].decode::<SetPause>().paused);
    assert_eq!(got.names(), ["MoveDirectionUpdate"], "her queued input ran");
    b.got(&fx.protocol);
    // With company the request is refused and the world keeps running.
    assert!(!fx.actor.set_paused(a.session, true));
    assert!(!fx.actor.set_paused(b.session, true));
    assert!(!fx.actor.world().paused());
    // The other player leaving makes it a solo game again.
    fx.leave(&b);
    assert!(fx.actor.set_paused(a.session, true));
    assert!(fx.actor.world().paused());
    // A paused world is released when its player leaves, so the next join starts fresh.
    fx.leave(&a);
    assert!(!fx.actor.world().paused());
}

#[test]
fn local_chat_reaches_every_session_in_the_instance() {
    // The chat path is `chat::deliver` over the live registry, keyed by instance id: the
    // sessions of one actor share it.
    let live = Live::default();
    let fx = Fixture::new();
    let id = Some(fx.instance);
    let mut a = live.register(1, "acc-a", "Alice", id, "G1_town", 1);
    let mut b = live.register(2, "acc-b", "Bob", id, "G1_town", 2);
    let mut elsewhere = live.register(3, "acc-c", "Carol", Some(fx.instance + 1), "G1_1", 3);
    assert_eq!(
        crate::chat::deliver(&live, "Alice", id, crate::chat::route("hello town", "!")),
        None
    );
    for rx in [&mut a, &mut b] {
        let line = rx.chat.try_recv().unwrap();
        assert_eq!(
            (line.sender.as_str(), line.text.as_str()),
            ("Alice", "hello town")
        );
    }
    assert!(elsewhere.chat.try_recv().is_err());
}

#[test]
fn player_debug_state_is_in_the_frame_only_when_asked_for() {
    let mut fx = Fixture::new();
    let mut a = fx.join_default("Alice", (100, 100));
    fx.walk(&a, 180, 0);
    fx.tick();
    let frame = a.rx.try_recv().unwrap();
    let debug = frame.own.debug.expect("want_debug");
    assert_eq!(debug.wasd, (180, 0));
    assert!(debug.state.starts_with("Walking"), "{}", debug.state);
    assert_eq!(frame.own.cell, fx.cell(&a));
}

#[test]
fn each_character_keeps_its_own_speed_in_the_shared_world() {
    let mut fx = Fixture::new();
    let slow = fx.join_default("Alice", (100, 100));
    let fast = fx.join_speed("Bob", (100, 300), 74.0);
    fx.walk(&slow, 180, 0);
    fx.walk(&fast, 180, 0);
    for _ in 0..30 {
        fx.tick();
    }
    let (a, b) = (fx.cell(&slow).0 - 100, fx.cell(&fast).0 - 100);
    assert!(a > 20 && b > 40, "{a} {b}");
    assert!((b as f32 / a as f32 - 2.0).abs() < 0.2, "{a} {b}");
}

// --- the real task, with paused time ----------------------------------------------------

/// A session on the real actor: what `handle` does between the join and the end.
struct Live2 {
    player: Player,
    frames: mpsc::UnboundedReceiver<Frame>,
    joined: Joined,
}

struct World2 {
    actors: Arc<Actors>,
    directory: Arc<MemoryInstances>,
    protocol: Arc<Protocol>,
    instance: u32,
    next: SessionId,
}

impl World2 {
    fn new() -> Self {
        let protocol = protocol();
        let directory = Arc::new(MemoryInstances::default());
        let instance = directory.create(
            InstanceSpec {
                area: "G1_town".into(),
                ..Default::default()
            },
            false,
        );
        Self {
            actors: Arc::new(Actors::default()),
            directory,
            protocol,
            instance,
            next: 0,
        }
    }

    async fn join(&mut self, name: &str, cell: (i32, i32), idle: Duration) -> Live2 {
        self.next += 1;
        let (request, frames) =
            join_request(self.next, name, cell, movement_config(&[roll_skill(None)]));
        let (protocol, directory, instance) =
            (self.protocol.clone(), self.directory.clone(), self.instance);
        let (handle, joined) = self
            .actors
            .join(
                &*self.directory,
                self.instance,
                move |_| ActorInit {
                    idle,
                    ..actor_init(&protocol, &directory, instance)
                },
                request,
            )
            .await
            .expect("joined");
        let player = Player::new(
            self.next,
            handle,
            joined.entity,
            Vec::new(),
            joined.own.clone(),
        );
        Live2 {
            player,
            frames,
            joined,
        }
    }
}

impl Live2 {
    /// Waits for the next frame and returns its head (the lifecycle messages).
    async fn head(&mut self, protocol: &Protocol) -> Vec<&'static str> {
        let frame = self.frames.recv().await.expect("a frame");
        split(protocol, &frame.head)
            .iter()
            .map(|s| s.name)
            .collect()
    }
}

#[tokio::test(start_paused = true)]
async fn the_real_actor_serves_two_sessions_on_one_clock() {
    let mut w = World2::new();
    let mut a = w.join("Alice", (100, 100), Duration::ZERO).await;
    let mut b = w.join("Bob", (300, 300), Duration::ZERO).await;
    assert_eq!(w.actors.players(w.instance), Some(2));
    assert_eq!(w.actors.counts(), [(w.instance, 2)]);
    assert_eq!(
        w.directory.get(w.instance).unwrap().players,
        ["Alice", "Bob"]
    );
    assert_eq!(b.joined.players, 2);
    // Alice's walk arrives at both on the shared clock.
    a.player
        .input(poe_game::locomotion::MoveInput::Direction { x: 180, y: 0 });
    let mut seen = [0usize; 2];
    for _ in 0..3 {
        for (i, s) in [&mut a, &mut b].into_iter().enumerate() {
            let frame = s.frames.recv().await.unwrap();
            seen[i] += split(&w.protocol, &frame.events)
                .iter()
                .filter(|m| m.is::<MoveDirectionUpdate>())
                .count();
        }
    }
    assert_eq!(seen, [1, 1]);
    // The escape menu is refused with two players.
    assert!(!b.player.set_paused(true).await);
}

#[tokio::test(start_paused = true)]
async fn dropping_a_session_leaves_and_leave_returns_the_cell() {
    let mut w = World2::new();
    let a = w.join("Alice", (100, 100), Duration::ZERO).await;
    let mut b = w.join("Bob", (300, 300), Duration::ZERO).await;
    assert_eq!(a.player.leave().await, (100, 100));
    let mut heads = Vec::new();
    while heads.iter().all(|h: &Vec<&str>| h.is_empty()) {
        heads.push(b.head(&w.protocol).await);
    }
    assert_eq!(
        heads.last().unwrap(),
        &[
            "PresenceChanged",
            "DestroyEntity",
            "EntitySleep",
            "RemoveObject"
        ]
    );
    assert_eq!(w.directory.get(w.instance).unwrap().players, ["Bob"]);
    // A dropped player leaves too.
    drop(b.player);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(w.directory.get(w.instance).unwrap().players.is_empty());
}

/// A duplicate login supersedes the first session: the session loop's stop fires, the player
/// is dropped and the other player gets the leave sequence.
#[tokio::test(start_paused = true)]
async fn a_superseded_session_leaves_the_instance() {
    let mut w = World2::new();
    let a = w.join("Alice", (100, 100), Duration::ZERO).await;
    let mut b = w.join("Bob", (300, 300), Duration::ZERO).await;
    let sessions = MemorySessions::default();
    let live = Live::default();
    let registration = live.register(7, "acc-a", "Alice", Some(w.instance), "G1_town", 1);
    let mut stops = Stops {
        closed: w.directory.closed(w.instance),
        superseded: sessions.subscribe_superseded(7),
        kicked: registration.kicked,
    };
    // What `InstanceServer::handle` does around the session loop.
    let ended = tokio::spawn(async move {
        let stop = stops.wait().await;
        let Live2 { player, .. } = a;
        player.leave().await;
        stop
    });
    sessions.supersede(7);
    assert_eq!(ended.await.unwrap(), Stop::Superseded);
    let mut heads = Vec::new();
    while heads.iter().all(|h: &Vec<&str>| h.is_empty()) {
        heads.push(b.head(&w.protocol).await);
    }
    assert_eq!(
        heads.last().unwrap(),
        &[
            "PresenceChanged",
            "DestroyEntity",
            "EntitySleep",
            "RemoveObject"
        ]
    );
    assert_eq!(w.directory.get(w.instance).unwrap().players, ["Bob"]);

    // A kick goes the same way.
    let c = w.join("Carol", (320, 300), Duration::ZERO).await;
    let registration = live.register(8, "acc-c", "Carol", Some(w.instance), "G1_town", 2);
    let mut stops = Stops {
        closed: w.directory.closed(w.instance),
        superseded: sessions.subscribe_superseded(8),
        kicked: registration.kicked,
    };
    assert!(live.kick("Carol"));
    assert_eq!(stops.wait().await, Stop::Kicked);
    drop(c);
}

#[tokio::test(start_paused = true)]
async fn closing_the_instance_ends_the_actor_and_its_sessions() {
    let mut w = World2::new();
    let mut a = w.join("Alice", (100, 100), Duration::ZERO).await;
    let mut stops = Stops {
        closed: w.directory.closed(w.instance),
        superseded: MemorySessions::default().subscribe_superseded(1),
        kicked: Live::default()
            .register(1, "a", "Alice", None, "G1_town", 1)
            .kicked,
    };
    assert!(w.directory.close(w.instance));
    assert_eq!(stops.wait().await, Stop::InstanceClosed);
    // The actor ends: the session's frame channel closes and the registry forgets it.
    while a.frames.recv().await.is_some() {}
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(w.actors.get(w.instance).is_none());
    // Joining a closed instance fails.
    let (request, _rx) = join_request(99, "Late", (1, 1), movement_config(&[roll_skill(None)]));
    let err = w
        .actors
        .join(&*w.directory, w.instance, |_| unreachable!(), request)
        .await
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(err, crate::actor::JoinError::NoInstance(_)));
}

#[tokio::test(start_paused = true)]
async fn an_empty_actor_ends_after_the_idle_time_and_the_next_join_starts_a_new_one() {
    let idle = Duration::from_secs(60);
    let mut w = World2::new();
    let a = w.join("Alice", (100, 100), idle).await;
    let first = w.actors.get(w.instance).unwrap();
    // Not while a player is in it.
    tokio::time::sleep(idle * 2).await;
    assert!(first.is_running());
    a.player.leave().await;
    tokio::time::sleep(idle - Duration::from_secs(1)).await;
    assert!(w.actors.get(w.instance).is_some());
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        w.actors.get(w.instance).is_none(),
        "ended after idle_minutes without players"
    );
    // The directory still has the instance: the next join creates a fresh actor and world.
    let b = w.join("Bob", (5, 5), idle).await;
    assert_eq!(b.joined.entity, FIRST_ENTITY);
    assert!(w.actors.get(w.instance).is_some());
}

/// The other players' view of a roll leaves out the stats only the owner is told about (official
/// remote roll batches carry two of the three stun threshold stats, the local one all three).
#[test]
fn a_remote_roll_batch_leaves_out_owner_only_stats() {
    use crate::replay_tests::stat_keys;
    let owner_only = "stun_threshold_+%_final_while_performing_action";
    let mut fx = Fixture::customised(FIRST_ENTITY, |mut init| {
        init.codec = Codec::new(init.codec.protocol.clone(), stat_keys())
            .with_stat_visibility(Arc::new(move |id: &str| id != owner_only));
        init
    });
    let mut a = fx.join_default("Alice", (371, 1052));
    let mut b = fx.join_default("Bob", (500, 500));
    fx.tick();
    a.got(&fx.protocol);
    b.got(&fx.protocol);
    fx.roll(&a, 15, -35);
    fx.tick();
    let (own, other) = (a.got(&fx.protocol), b.got(&fx.protocol));
    let key = |id: &str| stat_keys()(id).unwrap();
    let stats = |got: &Got| -> Vec<Vec<u32>> {
        got.events
            .iter()
            .filter(|s| s.is::<StatsUpdate>())
            .map(|s| {
                s.decode::<StatsUpdate>()
                    .stats
                    .iter()
                    .map(|&(k, _)| k)
                    .collect()
            })
            .collect()
    };
    // Alice's own client: all three stun threshold stats; Bob's: without the owner-only one.
    assert_eq!(
        stats(&own),
        [
            vec![key("is_dodge_rolling")],
            vec![
                key(owner_only),
                key("virtual_heavy_stun_threshold_+%_final"),
                key("virtual_light_stun_threshold_+%_final")
            ]
        ]
    );
    assert_eq!(
        stats(&other),
        [
            vec![key("is_dodge_rolling")],
            vec![
                key("virtual_heavy_stun_threshold_+%_final"),
                key("virtual_light_stun_threshold_+%_final")
            ]
        ]
    );
    // Everything else is the same messages in the same order.
    assert_eq!(own.names(), other.names());
}

/// A second player never stands on the first: the actor moves the joiner to a free walkable
/// cell and builds its own blob and its entity for that cell; others see the new cell too.
#[test]
fn a_second_player_does_not_spawn_on_the_first() {
    let mut fx = Fixture::customised(FIRST_ENTITY, |mut init| {
        init.player_spacing = 8;
        init
    });
    let mut a = fx.join_default("Alice", (100, 100));
    // Bob's own blob is built for the cell the actor chose, not the requested one.
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (mut request, rx) =
        join_request(2, "Bob", (100, 100), movement_config(&[roll_skill(None)]));
    let log = seen.clone();
    request.appearance.local_blob = Arc::new(move |cell| {
        log.lock().unwrap().push(cell);
        b"local Bob".to_vec()
    });
    let (reply, mut joined) = oneshot::channel();
    fx.actor.handle(Cmd::Join {
        request: Box::new(request),
        reply,
    });
    let b = Fake {
        session: 2,
        entity: 0,
        rx,
        joined: joined.try_recv().expect("joined"),
    }
    .with_entity();
    fx.next_session = 3;
    let c = fx.join_default("Carol", (101, 99));
    let (ca, cb, cc) = (fx.cell(&a), fx.cell(&b), fx.cell(&c));
    assert_eq!(ca, (100, 100), "the first player keeps the requested cell");
    let apart = |p: (i32, i32), q: (i32, i32)| (p.0 - q.0).pow(2) + (p.1 - q.1).pow(2) >= 64;
    assert!(
        apart(ca, cb) && apart(ca, cc) && apart(cb, cc),
        "{ca:?} {cb:?} {cc:?}"
    );
    // Bob's own entry batch (the local blob is a stand-in text) and the remote form Alice
    // gets carry the final cell, not the requested one.
    fx.tick();
    let got = a.got(&fx.protocol);
    let remote: Vec<_> = got
        .head
        .iter()
        .map(|s| PlayerBlob::decode(&s.decode::<AddObject>().blob).unwrap())
        .map(|blob| (blob.grid_x, blob.grid_y))
        .collect();
    assert_eq!(remote, [cb, cc]);
    assert_eq!(b.joined.own.cell, cb);
    assert_eq!(*seen.lock().unwrap(), [cb]);
}
