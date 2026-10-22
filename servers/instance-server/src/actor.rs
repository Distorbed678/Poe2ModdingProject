//! The instance actor: one task per instance that owns the instance's [`World`] and serves
//! every session in it (docs/architecture/instance-world.md).
//!
//! - **One world, one clock.** The actor owns one `poe_game::world::World` with entity ids from
//!   one per-instance counter (`[instance] first_entity_id`, official: players from 12000), runs
//!   one frame timer (`[instance] tick_ms`) and holds the area's grid. Nothing in it is tied to
//!   a connection, an account or a machine: a session is a [`SessionId`] and an output channel.
//! - **Sessions to the actor** are [`Cmd`]s on an unbounded channel: `Join` (with the output
//!   channel), `Input` (movement input of the session's entity), `Leave`, and the dev/admin
//!   operations `Teleport` and `SetPaused`. A session never touches the world.
//! - **The actor to sessions** are encoded packets, one [`Frame`] per tick and session. The
//!   events of a tick are encoded once (`Codec`) and the same bytes go to every session: the
//!   official server relays every entity's simulation messages to every client unchanged (the
//!   remote player's `0x64` carries the same header as the local one,
//!   docs/protocol/entities.md). What differs per session is only the lifecycle part (`head`:
//!   another player's `AddObject`/removal sequence, the pause release) and the entry batch of a
//!   joining session ([`Joined::add_objects`]: the other players in the remote blob form, then
//!   itself in the local form). So encoding stays in the actor, once per frame, and a session
//!   is a pipe that writes `head`, `events` (which already ends with the `SimulationTick`
//!   marker) and does its own bookkeeping with [`Frame::own`].
//! - **Interest management** is the whole instance: the official town sends every player at
//!   entry and removes one only when it leaves the area.
//! - **Pause** only exists for a single player ([`Actor::set_paused`]); see the doc there.
//! - **Lifetime.** [`Actors::join`] creates the actor on the first join. It ends when the
//!   instance is closed in the directory (`close`, idle expiry) or after `[instance]
//!   idle_minutes` without players (also for persistent towns and hideouts: their metadata
//!   stays in the directory, the world is rebuilt by the next join). While nobody is in it the
//!   clock does not tick. Ending drops every output channel, which ends the sessions left.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use poe_cluster::InstanceDirectory;
use poe_game::{
    grid::GridSource,
    locomotion::{MoveInput, MovementConfig},
    movement::Open,
    world::{EntityId, Event, World, WorldConfig},
};
use poe_proto::{
    Message, Protocol,
    instance::{AddObject, SetPause, SimulationHeader, SimulationTick},
    player_blob::PlayerBlob,
    remote::{DestroyEntity, EntitySleep, PresenceChanged, RemoveObject},
};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, info, warn};

use crate::{movement::Codec, spawn};

/// Identifies a session (one client connection) for the actor. Unique per server process; not
/// derived from the account, the character or the client's address.
pub type SessionId = u64;

/// Order key of `AddObject` and `RemoveObject` on official servers: after everything else in
/// the frame.
const AFTER_ALL: u32 = 0xffff_ffff;
/// Player component flag of the combat stance `+0x20c` (official remote blobs: `0x0c`, bit 3
/// being the always-set part the blob itself carries, [`crate::appearance::REMOTE_PLAYER_FLAGS`]).
const PLAYER_FLAG_STANCE: u8 = 0x04;
/// The payload of `PresenceChanged` (official: always `01 01 01 00`).
const PRESENCE: [u8; 4] = [1, 1, 1, 0];

/// What a session needs to know about its own entity after a frame (position for the drift
/// checker, `where`, the saved location and the debug link).
#[derive(Debug, Clone, PartialEq)]
pub struct Own {
    /// The world's frame number of this frame.
    pub frame: u64,
    pub cell: (i32, i32),
    /// Exact position in cells.
    pub position: (f32, f32),
    /// Only for sessions that joined with `want_debug`.
    pub debug: Option<OwnDebug>,
}

/// The simulation state of one entity for the debug link (`key\tvalue` lines in
/// [`crate::player::Player::debug_state`]).
#[derive(Debug, Clone, PartialEq)]
pub struct OwnDebug {
    pub state: String,
    pub wasd: (i32, i32),
    pub sprint_held: bool,
    pub combat_stance: bool,
    pub paused: bool,
}

/// One line of the movement trace for a message of `entity`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceLine {
    pub entity: u32,
    pub kind: &'static str,
    pub fields: String,
}

/// What the actor sends a session for each tick. The session writes `head`, then `events`.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Lifecycle messages for this session in this frame: another player's `AddObject`, the
    /// removal sequence of one that left, the release of a pause. Come first.
    pub head: Vec<u8>,
    /// Every entity's messages of the frame, in event order, then the `SimulationTick` marker.
    /// Shared by all sessions of the instance.
    pub events: Arc<Vec<u8>>,
    /// Trace lines of the movement messages (empty unless tracing is on).
    pub trace: Arc<Vec<TraceLine>>,
    pub own: Own,
}

/// How other players' clients see a character.
#[derive(Clone)]
pub struct Appearance {
    /// The class's object type key (`Metadata/Characters/...`).
    pub type_key: u32,
    /// The `AddObject` blob for the character's own client (entry state), for the cell the
    /// actor puts the character on (it can differ from the requested spawn, see
    /// [`ActorInit::player_spacing`]).
    pub local_blob: Arc<dyn Fn((i32, i32)) -> Vec<u8> + Send + Sync>,
    /// The blob for other clients, built by the session from the character; the actor patches
    /// the current cell and stance into a copy whenever it sends it.
    pub remote: PlayerBlob,
}

/// A session joining the instance (cloneable: a join is retried when the actor ends in
/// between).
#[derive(Clone)]
pub struct JoinRequest {
    pub session: SessionId,
    /// Character name (the instance's player list, logs).
    pub name: String,
    /// The cell the character should start on; the actor moves it to a free cell nearby when
    /// another player stands there.
    pub spawn: (i32, i32),
    /// Walking speed, cells per second.
    pub speed: f32,
    /// This character's movement configuration (class-dependent values).
    pub movement: MovementConfig,
    /// The area's grid, if the session knows it (all sessions of one instance share the
    /// layout).
    pub grid: Option<Arc<dyn GridSource>>,
    pub appearance: Appearance,
    /// Fill [`Own::debug`] in this session's frames.
    pub want_debug: bool,
    /// The session's output channel.
    pub out: mpsc::UnboundedSender<Frame>,
}

/// The actor's answer to a join: the session builds the entry batch from it.
#[derive(Debug)]
pub struct Joined {
    /// The session's entity id (for `PlayerEntity`).
    pub entity: u32,
    /// Encoded `AddObject`s: the players already in the instance (remote form), then the
    /// session's own entity (local form). The session sends them after `PlayerEntity` and the
    /// inventories, followed by a `SimulationTick` marker.
    pub add_objects: Vec<u8>,
    /// Players in the instance, including this one.
    pub players: usize,
    pub own: Own,
}

/// Session to actor.
pub enum Cmd {
    Join {
        request: Box<JoinRequest>,
        reply: oneshot::Sender<Joined>,
    },
    /// Movement input of the session's entity; answered by the next tick.
    Input {
        session: SessionId,
        input: MoveInput,
    },
    /// The session ends (disconnect, kick, transfer, supersede). `reply` gets the cell the
    /// entity was in.
    Leave {
        session: SessionId,
        reply: Option<oneshot::Sender<Option<(i32, i32)>>>,
    },
    /// Dev/admin: put the session's entity on a cell.
    Teleport {
        session: SessionId,
        cell: (i32, i32),
    },
    /// The escape menu; the reply says whether the world was paused or unpaused.
    SetPaused {
        session: SessionId,
        paused: bool,
        reply: oneshot::Sender<bool>,
    },
}

/// What the actor needs to be created.
pub struct ActorInit {
    pub instance: u32,
    pub area: String,
    pub tick_ms: u32,
    pub first_entity_id: u32,
    /// The world's default movement configuration (entities carry their own).
    pub movement: MovementConfig,
    pub grid: Option<Arc<dyn GridSource>>,
    pub codec: Codec,
    pub directory: Arc<dyn InstanceDirectory>,
    /// Empty for this long ends the actor; zero: never.
    pub idle: Duration,
    /// Build [`Frame::trace`].
    pub trace: bool,
    /// `[spawn] player_spacing`: the smallest distance in cells between a joining player's cell
    /// and the players already in the instance (0: spawn where requested).
    pub player_spacing: i32,
}

struct Member {
    session: SessionId,
    entity: EntityId,
    name: String,
    out: mpsc::UnboundedSender<Frame>,
    appearance: Appearance,
    want_debug: bool,
    /// Lifecycle bytes for the next frame.
    head: Vec<u8>,
}

/// The actor's state. [`run`] drives it from a task; tests call [`handle`](Actor::handle) and
/// [`tick`](Actor::tick) directly.
pub struct Actor {
    init: ActorInit,
    world: World,
    /// The grid is the placeholder `Open` (no session had one yet).
    open_grid: bool,
    members: Vec<Member>,
    /// The session that paused the world (only ever a lone player).
    paused_by: Option<SessionId>,
    tick_marker: Vec<u8>,
    /// Lets the registry show the live player count.
    players: Arc<AtomicUsize>,
}

impl Actor {
    pub fn new(init: ActorInit) -> anyhow::Result<Self> {
        let mut config = WorldConfig::new(init.movement.clone());
        config.tick_ms = init.tick_ms;
        config.first_entity_id = init.first_entity_id;
        let open_grid = init.grid.is_none();
        let grid: Arc<dyn GridSource> = init.grid.clone().unwrap_or_else(|| Arc::new(Open));
        let tick_marker = init.codec.protocol.instance.encode(&SimulationTick)?;
        Ok(Self {
            world: World::new(config, grid),
            open_grid,
            members: Vec::new(),
            paused_by: None,
            tick_marker,
            players: Arc::new(AtomicUsize::new(0)),
            init,
        })
    }

    pub fn instance(&self) -> u32 {
        self.init.instance
    }

    pub fn player_count(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    fn protocol(&self) -> &Protocol {
        &self.init.codec.protocol
    }

    /// Appends the encoding of `message` to `out` (a message the table cannot encode is
    /// logged and skipped).
    fn put<M: Message>(&self, out: &mut Vec<u8>, message: &M) {
        match self.protocol().instance.encode(message) {
            Ok(bytes) => out.extend(bytes),
            Err(e) => warn!(message = M::NAME, "cannot encode: {e}"),
        }
    }

    fn member(&self, session: SessionId) -> Option<&Member> {
        self.members.iter().find(|m| m.session == session)
    }

    /// The `AddObject` of a player as other clients see it, with its current cell and stance.
    fn add_remote(&self, m: &Member, out: &mut Vec<u8>) {
        let mut blob = m.appearance.remote.clone();
        if let Some(e) = self.world.entity(m.entity) {
            let (x, y) = e.movement.cell();
            blob.grid_x = x;
            blob.grid_y = y;
            blob.player.flags = if e.movement.in_combat_stance() {
                blob.player.flags | PLAYER_FLAG_STANCE
            } else {
                blob.player.flags & !PLAYER_FLAG_STANCE
            };
        }
        self.put(
            out,
            &AddObject {
                header: SimulationHeader {
                    entity_id: m.entity.0,
                    unk_14: AFTER_ALL,
                    ..Default::default()
                },
                type_key: m.appearance.type_key,
                blob: blob.encode(),
            },
        );
    }

    fn own(&self, m: &Member) -> Own {
        let e = self.world.entity(m.entity);
        let movement = e.map(|e| &e.movement);
        Own {
            frame: self.world.frame(),
            cell: movement.map_or((0, 0), |l| l.cell()),
            position: movement.map_or((0.0, 0.0), |l| l.position()),
            debug: movement.filter(|_| m.want_debug).map(|l| OwnDebug {
                state: format!("{:?}", l.state()),
                wasd: l.wasd(),
                sprint_held: l.sprint_held(),
                combat_stance: l.in_combat_stance(),
                paused: self.world.paused(),
            }),
        }
    }

    pub fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Join { request, reply } => self.join(*request, reply),
            Cmd::Input { session, input } => {
                if let Some(m) = self.member(session) {
                    self.world.input(m.entity, input);
                }
            }
            Cmd::Leave { session, reply } => {
                let cell = self.leave(session);
                if let Some(reply) = reply {
                    let _ = reply.send(cell);
                }
            }
            Cmd::Teleport { session, cell } => {
                if let Some(m) = self.member(session) {
                    self.world.teleport(m.entity, cell);
                }
            }
            Cmd::SetPaused {
                session,
                paused,
                reply,
            } => {
                let _ = reply.send(self.set_paused(session, paused));
            }
        }
    }

    /// Pause means one thing in a solo game and nothing else: the escape menu stops the world
    /// when the session is the only player in the instance (official behaviour). With other
    /// players the request is refused (`false`): the world keeps running and the session must
    /// answer its client with "not paused". A world paused by a lone player is released when
    /// a second player joins (the paused session gets `SetPause(false)` in its next frame) or
    /// when that player leaves. Returns whether the request was applied.
    pub fn set_paused(&mut self, session: SessionId, paused: bool) -> bool {
        if self.member(session).is_none() {
            return false;
        }
        if paused {
            if self.members.len() != 1 {
                info!(
                    players = self.members.len(),
                    "pause refused: other players are in the instance"
                );
                return false;
            }
            self.world.set_paused(true);
            self.paused_by = Some(session);
            true
        } else {
            if self.paused_by == Some(session) {
                self.world.set_paused(false);
                self.paused_by = None;
            }
            true
        }
    }

    /// Releases a pause held by `session` (a player joined or the pauser left) and tells it
    /// unless it is gone.
    fn release_pause(&mut self) {
        let Some(session) = self.paused_by.take() else {
            return;
        };
        self.world.set_paused(false);
        let mut bytes = Vec::new();
        self.put(
            &mut bytes,
            &SetPause {
                paused: false,
                flag_869: false,
                ..Default::default()
            },
        );
        if let Some(m) = self.members.iter_mut().find(|m| m.session == session) {
            m.head.extend(bytes);
        }
    }

    fn join(&mut self, req: JoinRequest, reply: oneshot::Sender<Joined>) {
        if self.member(req.session).is_some() {
            warn!(session = req.session, "session joined twice: ignored");
            return;
        }
        // The first session with a grid supplies the area's; they all have the same layout.
        if self.open_grid
            && let Some(grid) = req.grid.clone()
        {
            self.world.set_grid(grid);
            self.open_grid = false;
        }
        // A paused solo world runs again with company.
        if !self.members.is_empty() {
            self.release_pause();
        }
        // A second player must not stand on the first: take a free walkable cell nearby.
        let occupied: Vec<(i32, i32)> = self
            .members
            .iter()
            .filter_map(|m| self.world.entity(m.entity))
            .map(|e| e.movement.cell())
            .collect();
        let cell = spawn::spread(
            self.world.grid().as_ref(),
            req.spawn,
            &occupied,
            self.init.player_spacing,
        );
        if cell != req.spawn {
            info!(
                player = %req.name,
                wanted = ?req.spawn,
                ?cell,
                "spawn cell taken: standing on a free cell nearby"
            );
        }
        let entity = self
            .world
            .spawn_player_with_config(cell, req.speed, Some(req.movement));
        let member = Member {
            session: req.session,
            entity,
            name: req.name,
            out: req.out,
            appearance: req.appearance,
            want_debug: req.want_debug,
            head: Vec::new(),
        };
        // The newcomer's entry batch: the players already here (remote form), then itself.
        let mut add_objects = Vec::new();
        for other in &self.members {
            self.add_remote(other, &mut add_objects);
        }
        self.put(
            &mut add_objects,
            &AddObject {
                header: SimulationHeader {
                    entity_id: entity.0,
                    // Official servers send (0xffffffff, 0, 0): after everything else in the
                    // frame (a smaller key made the client add the player twice).
                    unk_14: AFTER_ALL,
                    ..Default::default()
                },
                type_key: member.appearance.type_key,
                blob: (member.appearance.local_blob)(cell),
            },
        );
        // Everyone already here gets one AddObject for the newcomer in the next frame.
        let mut remote = Vec::new();
        self.add_remote(&member, &mut remote);
        for other in &mut self.members {
            other.head.extend_from_slice(&remote);
        }
        self.init.directory.join(self.init.instance, &member.name);
        info!(
            instance = self.init.instance,
            session = member.session,
            player = %member.name,
            entity = entity.0,
            players = self.members.len() + 1,
            "player joined the instance"
        );
        if self.members.is_empty() {
            // Fresh clock for the first player (the timer does not run while empty).
            self.world.set_paused(false);
        }
        let own = self.own(&member);
        self.members.push(member);
        self.players.store(self.members.len(), Ordering::Relaxed);
        let joined = Joined {
            entity: entity.0,
            add_objects,
            players: self.members.len(),
            own,
        };
        if reply.send(joined).is_err() {
            // The session gave up while waiting.
            self.leave(req.session);
        }
    }

    /// Removes a session's entity and tells the others in one frame: `PresenceChanged`,
    /// `DestroyEntity` and `EntitySleep` at key 0, `RemoveObject` at key `0xffffffff`
    /// (docs/protocol/entities.md). Returns the cell it was in.
    fn leave(&mut self, session: SessionId) -> Option<(i32, i32)> {
        let i = self.members.iter().position(|m| m.session == session)?;
        let gone = self.members.remove(i);
        if self.paused_by == Some(session) {
            self.paused_by = None;
            self.world.set_paused(false);
        }
        let cell = self.world.despawn(gone.entity).map(|e| e.movement.cell());
        let plain = SimulationHeader {
            entity_id: gone.entity.0,
            ..Default::default()
        };
        let mut seq = Vec::new();
        self.put(&mut seq, &PresenceChanged { unk: PRESENCE });
        self.put(&mut seq, &DestroyEntity { header: plain });
        self.put(&mut seq, &EntitySleep { header: plain });
        self.put(
            &mut seq,
            &RemoveObject {
                header: SimulationHeader {
                    unk_14: AFTER_ALL,
                    ..plain
                },
            },
        );
        for other in &mut self.members {
            other.head.extend_from_slice(&seq);
        }
        self.init.directory.leave(self.init.instance, &gone.name);
        self.players.store(self.members.len(), Ordering::Relaxed);
        info!(
            instance = self.init.instance,
            session = gone.session,
            player = %gone.name,
            entity = gone.entity.0,
            players = self.members.len(),
            "player left the instance"
        );
        cell
    }

    /// One frame: advances the world, encodes its events once and gives every session its
    /// [`Frame`]. Sessions whose channel is closed are removed (as a `Leave`).
    pub fn tick(&mut self) {
        let events = self.world.tick();
        // Per event: the entity, the bytes for its own client and the bytes for everybody else
        // (they differ for stat updates that carry owner-only stats).
        let mut segments: Vec<(u32, Vec<u8>, Vec<u8>)> = Vec::new();
        let mut trace = Vec::new();
        for e in &events {
            let Event::Movement(m) = &e.event else {
                continue;
            };
            let entity = e.entity.0;
            let (owner, others) = match (
                self.init.codec.encode(entity, m),
                self.init.codec.encode_for_others(entity, m),
            ) {
                (Ok(owner), Ok(others)) => (owner, others),
                (Err(err), _) | (_, Err(err)) => {
                    warn!(entity, "cannot encode a movement event: {err:#}");
                    continue;
                }
            };
            let join = |outs: &[crate::movement::Out]| -> Vec<u8> {
                outs.iter().flat_map(|o| o.bytes.iter().copied()).collect()
            };
            segments.push((entity, join(&owner), join(&others)));
            if self.init.trace {
                trace.extend(owner.into_iter().map(|o| TraceLine {
                    entity,
                    kind: o.trace.0,
                    fields: o.trace.1,
                }));
            }
        }
        // What everyone but the owner gets, and the owner's own version where it differs.
        let mut shared = Vec::new();
        for (_, _, others) in &segments {
            shared.extend_from_slice(others);
        }
        shared.extend_from_slice(&self.tick_marker);
        let shared = Arc::new(shared);
        let trace = Arc::new(trace);
        let mut dead = Vec::new();
        for i in 0..self.members.len() {
            let own = self.own(&self.members[i]);
            let m = &mut self.members[i];
            let entity = m.entity.0;
            let differs = segments
                .iter()
                .any(|(e, full, others)| *e == entity && full != others);
            let events = if differs {
                let mut bytes = Vec::new();
                for (e, full, others) in &segments {
                    bytes.extend_from_slice(if *e == entity { full } else { others });
                }
                bytes.extend_from_slice(&self.tick_marker);
                Arc::new(bytes)
            } else {
                shared.clone()
            };
            let frame = Frame {
                head: std::mem::take(&mut m.head),
                events,
                trace: trace.clone(),
                own,
            };
            if m.out.send(frame).is_err() {
                dead.push(m.session);
            }
        }
        for session in dead {
            debug!(session, "session channel closed: leaving");
            self.leave(session);
        }
    }
}

/// Runs the actor until the instance is closed, it idles out or every handle is gone.
async fn run(
    mut actor: Actor,
    mut rx: mpsc::UnboundedReceiver<Cmd>,
    mut closed: Option<watch::Receiver<bool>>,
    ticks: Arc<poe_cluster::TickStats>,
) {
    let mut ticker =
        tokio::time::interval(Duration::from_millis(u64::from(actor.init.tick_ms.max(1))));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let idle = actor.init.idle;
    let mut empty_since = Some(tokio::time::Instant::now());
    loop {
        let idle_at = empty_since.filter(|_| !idle.is_zero()).map(|t| t + idle);
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                let was_empty = actor.is_empty();
                actor.handle(cmd);
                if was_empty && !actor.is_empty() {
                    ticker.reset();
                }
            }
            _ = ticker.tick(), if !actor.is_empty() => {
                // Inputs that arrived before the tick are answered by it.
                while let Ok(cmd) = rx.try_recv() {
                    actor.handle(cmd);
                }
                let started = Instant::now();
                actor.tick();
                let took = started.elapsed();
                ticks.record(took);
                if took.as_millis() as u32 > actor.init.tick_ms / 2 {
                    warn!(instance = actor.instance(), ?took, "slow simulation frame");
                }
            }
            () = instance_closed(&mut closed) => {
                info!(instance = actor.instance(), "instance closed: ending its actor");
                break;
            }
            () = idle_deadline(idle_at) => {
                info!(instance = actor.instance(), "no players for a while: ending the instance actor");
                break;
            }
        }
        empty_since = if actor.is_empty() {
            empty_since.or_else(|| Some(tokio::time::Instant::now()))
        } else {
            None
        };
    }
    // Members drop with the actor: their output channels close and their sessions end.
    let left: Vec<String> = actor.members.iter().map(|m| m.name.clone()).collect();
    for name in left {
        actor.init.directory.leave(actor.init.instance, &name);
    }
    actor.players.store(0, Ordering::Relaxed);
}

async fn instance_closed(rx: &mut Option<watch::Receiver<bool>>) {
    match rx {
        // A closed instance's sender is dropped: an error also means closed.
        Some(rx) => {
            let _ = rx.wait_for(|c| *c).await;
        }
        None => std::future::pending().await,
    }
}

async fn idle_deadline(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// A running actor, as the registry and sessions see it.
#[derive(Clone)]
pub struct ActorHandle {
    tx: mpsc::UnboundedSender<Cmd>,
    generation: u64,
    pub instance: u32,
    pub area: String,
    players: Arc<AtomicUsize>,
}

impl ActorHandle {
    /// Sends a command; `false` when the actor has ended.
    pub fn send(&self, cmd: Cmd) -> bool {
        self.tx.send(cmd).is_ok()
    }

    pub fn players(&self) -> usize {
        self.players.load(Ordering::Relaxed)
    }

    pub fn is_running(&self) -> bool {
        !self.tx.is_closed()
    }
}

/// The running actors by instance id. (`poe-cluster`'s `MemoryInstances` stays the directory:
/// what an instance is, who is in it, when it closes; it has no game types, so the actor
/// handles live here and the actor itself keeps the directory's player list.)
#[derive(Default)]
pub struct Actors {
    map: Mutex<HashMap<u32, ActorHandle>>,
    generation: AtomicU64,
    sessions: AtomicU64,
    /// Frame times of every actor, read by the metrics sampler (web admin dashboard).
    pub ticks: Arc<poe_cluster::TickStats>,
}

/// Why a join failed.
#[derive(Debug)]
pub enum JoinError {
    /// The instance is not in the directory (closed meanwhile).
    NoInstance(u32),
    /// The actor ended while joining (retried a few times first).
    ActorGone,
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInstance(id) => write!(f, "instance #{id} is closed"),
            Self::ActorGone => write!(f, "the instance ended while joining"),
        }
    }
}

impl std::error::Error for JoinError {}

impl Actors {
    /// A new session id (unique in this process, never reused).
    pub fn next_session(&self) -> SessionId {
        self.sessions.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn get(&self, instance: u32) -> Option<ActorHandle> {
        self.map
            .lock()
            .unwrap()
            .get(&instance)
            .filter(|h| h.is_running())
            .cloned()
    }

    /// Players in an instance's actor; `None` when it has none running.
    pub fn players(&self, instance: u32) -> Option<usize> {
        self.get(instance).map(|h| h.players())
    }

    /// Running actors and their player counts.
    pub fn counts(&self) -> Vec<(u32, usize)> {
        let mut v: Vec<_> = self
            .map
            .lock()
            .unwrap()
            .values()
            .filter(|h| h.is_running())
            .map(|h| (h.instance, h.players()))
            .collect();
        v.sort_unstable();
        v
    }

    /// The instance's actor, started with `init` if none runs. `closed` ends it with the
    /// instance. Must be called inside a tokio runtime.
    pub fn get_or_spawn(
        self: &Arc<Self>,
        init: impl FnOnce() -> ActorInit,
        instance: u32,
        closed: Option<watch::Receiver<bool>>,
    ) -> anyhow::Result<ActorHandle> {
        let mut map = self.map.lock().unwrap();
        if let Some(h) = map.get(&instance).filter(|h| h.is_running()) {
            return Ok(h.clone());
        }
        let init = init();
        let actor = Actor::new(init)?;
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = ActorHandle {
            tx,
            generation: self.generation.fetch_add(1, Ordering::Relaxed),
            instance,
            area: actor.init.area.clone(),
            players: actor.players.clone(),
        };
        map.insert(instance, handle.clone());
        info!(instance, area = %handle.area, "instance actor started");
        let registry = self.clone();
        let generation = handle.generation;
        tokio::spawn(async move {
            run(actor, rx, closed, registry.ticks.clone()).await;
            let mut map = registry.map.lock().unwrap();
            if map
                .get(&instance)
                .is_some_and(|h| h.generation == generation)
            {
                map.remove(&instance);
            }
            info!(instance, "instance actor ended");
        });
        Ok(handle)
    }

    /// Joins a session to the instance's actor, starting it when needed (`make` builds the
    /// actor's init from the first join). The session's frames arrive on `req.out`.
    pub async fn join(
        self: &Arc<Self>,
        directory: &dyn InstanceDirectory,
        instance: u32,
        make: impl Fn(&JoinRequest) -> ActorInit,
        req: JoinRequest,
    ) -> Result<(ActorHandle, Joined), JoinError> {
        for _ in 0..3 {
            // The instance must still exist (it may have been closed since the ticket).
            let Some(closed) = directory.closed(instance) else {
                return Err(JoinError::NoInstance(instance));
            };
            let handle = match self.get_or_spawn(|| make(&req), instance, Some(closed)) {
                Ok(h) => h,
                Err(e) => {
                    warn!(instance, "cannot start the instance actor: {e:#}");
                    return Err(JoinError::ActorGone);
                }
            };
            let (reply, joined) = oneshot::channel();
            let cmd = Cmd::Join {
                request: Box::new(req.clone()),
                reply,
            };
            if !handle.send(cmd) {
                continue;
            }
            if let Ok(joined) = joined.await {
                return Ok((handle, joined));
            }
        }
        Err(JoinError::ActorGone)
    }
}
