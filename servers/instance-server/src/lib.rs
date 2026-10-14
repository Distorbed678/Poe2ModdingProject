//! Instance server: hosts areas. The login server hands clients over with a ticket;
//! the instance server validates it, sends the area and runs the simulation.
//!
//! Connection opening (Documentation/protocol/instance.md):
//! 1. client `Hello` (plaintext) carrying the hand-off ticket,
//! 2. server `EnableEncryption` (plaintext), then both directions switch to Salsa20 with the
//!    ticket's key material,
//! 3. server `AreaInfo` (area + seed): the client generates the terrain and loads the area.

use std::{sync::Arc, time::Duration};

use poe_cluster::{Ticket, TicketCharacter, TicketRegistry};
use poe_config::Config;
use poe_game::stats::StatRegistry;
use poe_store::Store;

mod player;
pub mod seeds;
mod spawn;
pub mod sync;
use poe_data::{
    GameData,
    template::ObjectTemplate,
    views::{CharacterClass, InventorySlot, WorldArea},
};
use poe_net::{Connection, DecodeError, KeyMaterial, StreamCipher, hexdump::hexdump};
use poe_proto::{
    Message, Protocol, entity,
    instance::{
        AddInventory, AddObject, AreaInfo, CharacterState, ChatJoinRequest, ChatJoined,
        EnableEncryption, ExitReport, FrameReport, Hello, InitialState, Inventory, LeaveGame,
        LocalPlayers, MoveDirection, PauseRequest, PingReply, PlayerEntity, PlayerState, SetPause,
        SimulationHeader, SimulationTick, SkillReleased, SprintInput, TerrainHashes,
        UpdateSkillTarget, UpdateSkillTargetEntity, UseSkill,
    },
};
use rand::RngCore;
use tokio::net::{TcpListener, TcpStream};
use tracing::{Instrument, debug, info, warn};

/// State shared by all instance connections.
pub struct InstanceServer {
    pub config: Arc<Config>,
    pub protocol: Arc<Protocol>,
    pub tickets: Arc<dyn TicketRegistry>,
    pub data: Arc<GameData>,
    /// Stat ids -> network keys (built once from Stats.dat).
    pub stats: Arc<StatRegistry>,
    pub store: Arc<dyn Store>,
    /// Seeds the client is known to generate (`instance.known_seeds`).
    pub seeds: Arc<seeds::KnownSeeds>,
    /// Client position reports (movement dev bridge, see [`sync`]).
    pub reports: Arc<sync::ClientReports>,
    /// Ends the account's login connection when the client leaves the game.
    pub sessions: Arc<dyn poe_cluster::LoginSessions>,
}

impl InstanceServer {
    pub async fn run(self: Arc<Self>) -> anyhow::Result<()> {
        let listener = TcpListener::bind(self.config.instance.bind).await?;
        info!(addr = %self.config.instance.bind, "instance server listening");
        let trace_path = &self.config.instance.movement_trace;
        if !trace_path.as_os_str().is_empty() {
            match sync::MovementTrace::open(trace_path) {
                Ok(t) => {
                    info!(path = %trace_path.display(), "movement trace on");
                    self.reports.set_trace(Arc::new(t));
                }
                Err(e) => warn!(path = %trace_path.display(), "movement trace: {e}"),
            }
        }
        if self.config.instance.position_sync {
            let reports = self.reports.clone();
            let addr = self.config.instance.bind;
            tokio::spawn(async move {
                if let Err(e) = reports.listen(addr).await {
                    warn!("client position reports: {e:#}");
                }
            });
        }
        loop {
            let (stream, peer) = listener.accept().await?;
            let span = tracing::info_span!("instance", %peer);
            let server = self.clone();
            tokio::spawn(
                async move {
                    if let Err(e) = server.handle(stream).await {
                        warn!("connection error: {e:#}");
                    }
                }
                .instrument(span),
            );
        }
    }

    async fn handle(&self, stream: TcpStream) -> anyhow::Result<()> {
        let client_port = stream.peer_addr()?.port();
        let mut conn = Connection::new(stream)?;
        info!("client connected");
        let hello_op = self.protocol.instance.id(Hello::DIRECTION, Hello::NAME)?;
        let Some(hello) = conn
            .decode(|r| {
                let at = r.position();
                if r.u16()? != hello_op {
                    return Err(DecodeError::Invalid {
                        offset: at,
                        what: "expected Hello",
                    });
                }
                Hello::decode(r)
            })
            .await?
        else {
            return Ok(());
        };
        let Some(ticket) = self.tickets.redeem(hello.ticket) else {
            warn!(
                ticket = format_args!("{:#010x}", hello.ticket),
                "unknown or expired ticket"
            );
            return Ok(());
        };
        info!(
            ticket = format_args!("{:#010x}", ticket.id),
            account = ticket.account_id,
            character = ticket.character.id,
            area = %ticket.area,
            "ticket redeemed"
        );

        conn.send(
            &self
                .protocol
                .instance
                .encode(&EnableEncryption::default())?,
        )
        .await?;
        let keys = KeyMaterial(ticket.keys);
        conn.set_ciphers(
            Box::new(keys.client_to_server()),
            Box::new(keys.server_to_client()),
        );
        // HNC side channel (see PingReply): server-to-client keystream for idle bytes.
        let mut hnc = KeyMaterial(self.protocol.hnc.derive(&ticket.keys)).server_to_client();

        let seed = self.send_area(&mut conn, &ticket).await?;

        let hashes_op = self
            .protocol
            .instance
            .id(TerrainHashes::DIRECTION, TerrainHashes::NAME)?;
        let hashes = conn
            .decode(|r| {
                let at = r.position();
                if r.u16()? != hashes_op {
                    return Err(DecodeError::Invalid {
                        offset: at,
                        what: "expected TerrainHashes",
                    });
                }
                TerrainHashes::decode(r)
            })
            .await?;
        let Some(hashes) = hashes else {
            info!("client disconnected during terrain generation");
            return Ok(());
        };
        // hash_a is the tile hash, hash_b the doodad hash (as in the client log).
        info!(
            unk = hashes.unk,
            tile_hash = hashes.hash_a,
            doodad_hash = hashes.hash_b,
            "terrain generated by client"
        );
        // Where the character left this layout, if it did; else the area's start.
        let saved = ticket
            .character
            .location
            .filter(|&(s, _, _)| s == seed)
            .map(|(_, x, y)| (x, y));
        let spawn = match spawn_override().or(saved) {
            Some(p) => Some(p),
            None => {
                spawn::from_client_layout(&self.config.spawn, &ticket.area, hashes.hash_a).await
            }
        }
        .unwrap_or((0, 0));
        let mut player = self.new_player(&ticket, spawn)?;
        player.walkable = spawn::client_grid(&self.config.spawn, hashes.hash_a);
        player.frame_seconds = self.config.instance.tick_ms as f32 / 1000.0;
        if saved.is_some() {
            // Same layout, so the saved cell is normally walkable; otherwise use the nearest
            // open spot.
            if let Some(grid) = &player.walkable
                && !poe_game::movement::Walkable::walkable(grid, spawn.0, spawn.1)
                && let Some(cell) = grid.nearest_walkable(spawn, 50)
            {
                player.mover.place(cell);
            }
            info!(cell = ?player.mover.cell(), "restoring saved position");
        }
        self.enter_world(&mut conn, &ticket, &player).await?;

        let result = self
            .run_session(&mut conn, &mut hnc, &ticket, &mut player, client_port)
            .await;
        if let Some(r) = self.reports.fresh(client_port) {
            // Where the client really is.
            player.apply_report(Some(r));
        }
        self.reports.forget(client_port);
        if let Some(t) = self.reports.trace() {
            t.flush();
        }
        // Save where the character is, however the session ended.
        let (x, y) = player.mover.cell();
        let location = poe_store::Location { seed, x, y };
        match self
            .store
            .save_location(ticket.character.id, &ticket.area, Some(location))
        {
            Ok(()) => info!(area = %ticket.area, seed, cell = ?(x, y), "saved character location"),
            Err(e) => warn!("saving character location: {e}"),
        }
        result
    }

    /// The in-game loop: client messages and simulation frames until the session ends.
    async fn run_session(
        &self,
        conn: &mut Connection,
        hnc: &mut poe_net::Salsa20Stream,
        ticket: &Ticket,
        player: &mut player::Player,
        client_port: u16,
    ) -> anyhow::Result<()> {
        // Simulation frames: each tick marker closes a frame of `tick_ms` (InitialState).
        let tick = self.protocol.instance.encode(&SimulationTick)?;
        let mut ticker =
            tokio::time::interval(Duration::from_millis(self.config.instance.tick_ms.max(1)));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Without position reports the server only has its own simulation, which has no
        // terrain yet: positions drift and the client snaps when they are 46+ cells apart.
        let report_deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut warned = !self.config.instance.position_sync;
        let mut ticks: u64 = 0;
        // Not decoded yet: log the client's decrypted messages (heartbeat 0x1c is skipped).
        loop {
            tokio::select! {
                n = conn.read_more() => {
                    if n? == 0 {
                        info!("client disconnected");
                        return Ok(());
                    }
                    let data = conn.buffer().to_vec();
                    conn.consume(data.len());
                    // Input positions are relative to where the client is now: take its
                    // latest report rather than the one from the last tick. Replies apply in
                    // the frame after the last tick marker sent.
                    player.apply_report(self.reports.fresh(client_port));
                    player.observe_frames(ticks as i64);
                    player.apply_frame = ticks as i64 + 1;
                    // Client messages have no length prefix: decode the known ones in turn;
                    // anything else ends the batch (logged).
                    let mut rest = &data[..];
                    while rest.len() >= 2 {
                        let opcode = u16::from_be_bytes([rest[0], rest[1]]);
                        let mut r = poe_net::Reader::new(&rest[2..]);
                        match self.handle_client_message(conn, hnc, player, ticket.account_id, client_port, opcode, &mut r).await? {
                            Handled::Continue => rest = &rest[2 + r.position()..],
                            Handled::Unknown => {
                                info!(len = rest.len(), "received\n{}", hexdump(rest));
                                break;
                            }
                            Handled::Exit => return Ok(()),
                        }
                    }
                }
                _ = ticker.tick() => {
                    let frame = std::time::Instant::now();
                    ticks += 1;
                    if let Some(t) = self.reports.trace() {
                        t.line(client_port, "tick", format_args!("{ticks}"));
                        if ticks.is_multiple_of(30) {
                            t.flush();
                        }
                    }
                    player.apply_report(self.reports.fresh(client_port));
                    // `ticks - 1` markers are out; this tick's replies apply in frame `ticks`.
                    player.observe_frames(ticks as i64 - 1);
                    player.apply_frame = ticks as i64;
                    if !warned && !player.synced() && frame > report_deadline {
                        warned = true;
                        warn!(
                            client_port,
                            "no position reports from the client (poe-hook position_sync): \
                             movement uses the server simulation and can snap"
                        );
                    }
                    let replies = player.tick(self.config.instance.tick_ms as f32 / 1000.0);
                    let mut out = Vec::new();
                    for reply in &replies {
                        out.extend(self.encode_reply(client_port, reply)?);
                    }
                    out.extend_from_slice(&tick);
                    conn.send(&out).await?;
                    let took = frame.elapsed();
                    if took.as_millis() as u64 > self.config.instance.tick_ms / 2 {
                        warn!(?took, "slow simulation frame");
                    }
                }
            }
        }
    }

    /// Sends the area; returns the terrain seed (a saved location's seed when the character
    /// left this area, so it comes back to the same layout).
    async fn send_area(&self, conn: &mut Connection, ticket: &Ticket) -> anyhow::Result<u32> {
        let Some(area) = WorldArea::find(&self.data, &ticket.area)? else {
            anyhow::bail!("ticket area {} not in game data", ticket.area);
        };
        let (instance_id, seed) = {
            let mut rng = rand::rng();
            // RE experiment: `POE_SEED` pins the layout seed (to reproduce a layout).
            let pinned = std::env::var("POE_SEED").ok().and_then(|v| v.parse().ok());
            let saved = ticket.character.location.map(|(seed, _, _)| seed);
            let seed = pinned
                .or(area.fixed_seed())
                .or(saved)
                .or_else(|| self.seeds.pick(&area.id, rng.next_u32()))
                .unwrap_or_else(|| rng.next_u32());
            (rng.next_u32(), seed)
        };
        let msg = AreaInfo {
            instance_id,
            area_hash: area.hash16,
            text_38: String::new(),
            seed,
            ..Default::default()
        };
        info!(area = %area.id, seed = msg.seed, instance = msg.instance_id, "sending area");
        conn.send(&self.protocol.instance.encode(&msg)?).await?;
        Ok(seed)
    }
}

/// Entity id of the (only) player in an instance.
const PLAYER_ENTITY: u32 = 1;

/// RE experiment: `POE_NPC=<metadata path>` adds that object as entity 2 next to the player
/// with a zero blob (Positioned first). Objects with a StateMachine need their shared states
/// (see Documentation/protocol/entities.md), so this suits simple types such as
/// `Metadata/MiscellaneousObjects/Waypoint`.
fn npc_experiment(
    table: &poe_proto::OpcodeTable,
    path: &str,
    (x, y): (i32, i32),
) -> anyhow::Result<Vec<u8>> {
    let mut blob = vec![entity::ENTITY_IN_WAKE_ZONE, 0];
    blob.extend((x + 15).to_le_bytes());
    blob.extend(y.to_le_bytes());
    blob.extend([0u8; 1024]);
    info!(%path, "adding experiment object");
    Ok(table.encode(&AddObject {
        header: SimulationHeader {
            entity_id: 2,
            ..Default::default()
        },
        type_key: poe_data::hash::object_type_key(path),
        blob,
    })?)
}

/// RE experiment: `POE_SPAWN=x,y` forces the grid position (the client clamps it to the map).
fn spawn_override() -> Option<(i32, i32)> {
    let v = std::env::var("POE_SPAWN").ok()?;
    let (x, y) = v.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

impl InstanceServer {
    fn trace(&self, port: u16, kind: &str, fields: std::fmt::Arguments<'_>) {
        if let Some(t) = self.reports.trace() {
            t.line(port, kind, fields);
        }
    }

    fn encode_reply(&self, port: u16, reply: &player::Reply) -> anyhow::Result<Vec<u8>> {
        match reply {
            player::Reply::Direction(m) => self.trace(
                port,
                "out-dir",
                format_args!("{}\t{}\t{}\t{}", m.dir_x, m.dir_y, m.grid_x, m.grid_y),
            ),
            player::Reply::Start(m) => self.trace(
                port,
                "out-start",
                format_args!(
                    "{}\t{}\t{}\t{}\t{:#x}\t{}",
                    m.grid_x, m.grid_y, m.target_x, m.target_y, m.skill, m.action_id
                ),
            ),
            player::Reply::Sprint(m) => self.trace(port, "out-sprint", format_args!("{}", m.held)),
        }
        let table = &self.protocol.instance;
        Ok(match reply {
            player::Reply::Direction(m) => table.encode(m)?,
            player::Reply::Start(m) => table.encode(m)?,
            player::Reply::Sprint(m) => table.encode(m)?,
        })
    }
}

/// Outcome of handling one client message.
enum Handled {
    Continue,
    /// Not a message the server decodes yet: the rest of the batch is skipped.
    Unknown,
    /// The session ends.
    Exit,
}

impl InstanceServer {
    /// One client message after the player entered the world (`r` is after the opcode).
    #[allow(clippy::too_many_arguments)]
    async fn handle_client_message(
        &self,
        conn: &mut Connection,
        hnc: &mut poe_net::Salsa20Stream,
        player: &mut player::Player,
        account_id: u32,
        port: u16,
        opcode: u16,
        r: &mut poe_net::Reader<'_>,
    ) -> anyhow::Result<Handled> {
        let table = &self.protocol.instance;
        let Some(name) = table.name(poe_proto::Direction::Client, opcode) else {
            return Ok(Handled::Unknown);
        };
        match name {
            // Frame report: answer with a ping reply carrying idle HNC bytes.
            FrameReport::NAME => {
                FrameReport::decode(r)?;
                let mut bytes = [0u8; 4];
                hnc.apply(&mut bytes);
                let reply = PingReply {
                    hnc: poe_proto::instance::hnc_bits(bytes),
                    ..Default::default()
                };
                conn.send(&table.encode(&reply)?).await?;
            }
            // WASD: the server echoes the direction with its position of the player; the client
            // then moves the player itself until the next change.
            MoveDirection::NAME => {
                let req = MoveDirection::decode(r)?;
                self.trace(port, "in-dir", format_args!("{}\t{}", req.x, req.y));
                if let Some(reply) = player.on_move_direction(&req) {
                    let reply = player::Reply::Direction(reply);
                    conn.send(&self.encode_reply(port, &reply)?).await?;
                }
            }
            // Steering a running skill (roll/sprint towards the cursor, held click).
            UpdateSkillTarget::NAME => {
                let req = UpdateSkillTarget::decode(r)?;
                self.trace(
                    port,
                    "in-target",
                    format_args!("{}\t{}\t{:#x}", req.x, req.y, req.skill),
                );
                if let Some(reply) = player.on_update_target(&req) {
                    self.trace(
                        port,
                        "out-target",
                        format_args!("{}\t{}\t{}", reply.action_id, reply.x, reply.y),
                    );
                    conn.send(&table.encode(&reply)?).await?;
                }
            }
            UpdateSkillTargetEntity::NAME => {
                let req = UpdateSkillTargetEntity::decode(r)?;
                debug!(?req, "skill target entity (no entities yet)");
            }
            // The sprint key, echoed; a roll that ends while it is held becomes a sprint.
            SprintInput::NAME => {
                let req = SprintInput::decode(r)?;
                self.trace(port, "in-sprint", format_args!("{}", req.held));
                let mut out = Vec::new();
                for reply in player.on_sprint_input(&req) {
                    out.extend(self.encode_reply(port, &reply)?);
                }
                conn.send(&out).await?;
            }
            // Skill use (click-to-move, dodge roll): start the action on the client.
            UseSkill::NAME => {
                let req = UseSkill::decode(r)?;
                self.trace(
                    port,
                    "in-skill",
                    format_args!(
                        "{:#x}\t{}\t{}\t{}",
                        req.skill, req.target_flags, req.x, req.y
                    ),
                );
                let replies = player.on_skill_input(&req);
                debug!(?req, ?replies, "skill use");
                let mut out = Vec::new();
                for reply in &replies {
                    out.extend(self.encode_reply(port, reply)?);
                }
                if !out.is_empty() {
                    conn.send(&out).await?;
                }
            }
            SkillReleased::NAME => {
                let req = SkillReleased::decode(r)?;
                self.trace(port, "in-release", format_args!("{:#x}", req.skill));
                let replies = player.on_skill_released(req.skill);
                debug!(
                    skill = format_args!("{:#010x}", req.skill),
                    ?replies,
                    "skill released"
                );
                let mut out = Vec::new();
                for reply in &replies {
                    out.extend(self.encode_reply(port, reply)?);
                }
                if !out.is_empty() {
                    conn.send(&out).await?;
                }
            }
            // Escape menu in a solo game: confirm with SetPause, which pauses the client's
            // world; the server simulation pauses with it.
            PauseRequest::NAME => {
                let req = PauseRequest::decode(r)?;
                info!(paused = req.paused, "pause request");
                player.paused = req.paused;
                let reply = SetPause {
                    paused: req.paused,
                    flag_869: req.paused,
                    ..Default::default()
                };
                conn.send(&table.encode(&reply)?).await?;
            }
            // Chat: every request joins channel 1 of the requested kind (no chat server yet).
            ChatJoinRequest::NAME => {
                let req = ChatJoinRequest::decode(r)?;
                let channel = if req.channel == 0xffff {
                    self.config.chat.default_channel
                } else {
                    req.channel
                };
                info!(kind = req.kind, channel, "chat join");
                let reply = ChatJoined {
                    unk_10: req.unk_0,
                    channel,
                    trade: req.kind != 0,
                    unk_1c: req.unk_4,
                };
                conn.send(&table.encode(&reply)?).await?;
            }
            // Exit to character selection: hand the client a resume ticket (0x174). It then
            // reconnects to the login server and presents it (login 0x08) instead of
            // authenticating; the login server answers like a normal login.
            "ExitRequest" => {
                let resume = self.tickets.issue_resume(account_id);
                info!("client exits to character selection");
                let reply = poe_proto::instance::ExitSession {
                    unk_10: 0,
                    unk_20: resume.account_id,
                    unk_18: resume.token,
                };
                conn.send(&table.encode(&reply)?).await?;
                return Ok(Handled::Exit);
            }
            ExitReport::NAME => {
                let report = ExitReport::decode(r)?;
                debug!(len = report.data.len(), "exit report");
            }
            // "Exit to log in screen" or quitting: the client closes the connection next. Its
            // login connection must close too, or it returns to character selection on it.
            LeaveGame::NAME => {
                LeaveGame::decode(r)?;
                info!("client leaves the game");
                self.sessions.logout(account_id);
            }
            _ => match table.client_body_size(opcode) {
                // Known size, not decoded yet: skip it.
                Some(size) => {
                    let body = r.bytes(size)?;
                    debug!(opcode = format_args!("{opcode:#06x}"), ?body, "skipped");
                }
                None => return Ok(Handled::Unknown),
            },
        }
        Ok(Handled::Continue)
    }

    /// The player's server-side state: position, walking speed (class template
    /// `Pathfinding.base_speed`, cells per second) and the innate skills
    /// (`[player] innate_skills`).
    fn new_player(&self, ticket: &Ticket, spawn: (i32, i32)) -> anyhow::Result<player::Player> {
        let Some(class) = CharacterClass::by_integer_id(&self.data, ticket.character.class_id)?
        else {
            anyhow::bail!("unknown class id {}", ticket.character.class_id);
        };
        let template = ObjectTemplate::load(&self.data, &class.metadata_id)?;
        let speed = template
            .component("Pathfinding")
            .iter()
            .find(|(k, _)| k == "base_speed")
            .and_then(|(_, v)| v.parse::<f32>().ok())
            .unwrap_or(37.0);
        let mut skills = Vec::new();
        for (i, id) in self.config.player.innate_skills.iter().enumerate() {
            match poe_data::views::GrantedSkill::find(&self.data, id, 1)? {
                Some(granted) => {
                    // The stat set's constant stats, and its implicit (flag) stats as 1. The
                    // client reads computed stats (e.g. `dodge_roll_travel_distance`), which
                    // the server derives from the base ones; until there is a stat calculator
                    // (M7), each `..._base_...` stat is also sent under its name without
                    // `base_` when that stat exists.
                    let mut named: Vec<(String, i32)> = granted
                        .constant_stats
                        .iter()
                        .map(|(k, v)| (k.clone(), *v as i32))
                        .chain(granted.implicit_stats.iter().map(|k| (k.clone(), 1)))
                        .collect();
                    let derived: Vec<(String, i32)> = named
                        .iter()
                        .filter(|(k, _)| k.contains("base_"))
                        .map(|(k, v)| (k.replacen("base_", "", 1), *v))
                        .collect();
                    named.extend(derived);
                    let mut stats = Vec::new();
                    for (stat, value) in &named {
                        if let Some(key) = poe_data::views::stat_key(&self.data, stat)? {
                            stats.push((key, *value));
                        } else if !stat.contains("base_") {
                            debug!(%stat, "skill stat not in Stats.dat");
                        }
                    }
                    skills.push(player::OwnedSkill {
                        // Small ids, distinct from basic actions (0x2000xxxx) and item skills.
                        id: i as u32 + 1,
                        granted,
                        id_name: id.clone(),
                        stats,
                    })
                }
                None => warn!(skill = %id, "innate skill not in game data"),
            }
        }
        let move_skill = poe_data::views::action_type_key(&self.data, "Move")?
            .map(|k| player::BASIC_ACTION_SKILL | k as u32);
        info!(speed, skills = skills.len(), "player state");
        let mut player = player::Player::new(PLAYER_ENTITY, spawn, speed, skills, move_skill);
        // Sprint speed (dodge roll held), from the class template.
        if let Some(bonus) = template
            .component("Stats")
            .iter()
            .find(|(k, _)| k == "sprint_movement_speed_+%")
            .and_then(|(_, v)| v.parse::<f32>().ok())
        {
            player.sprint_scale = 1.0 + bonus / 100.0;
        }
        Ok(player)
    }

    /// After the client generated the area: local player, inventories, the player entity,
    /// then InitialState (see Documentation/protocol/instance.md).
    async fn enter_world(
        &self,
        conn: &mut Connection,
        ticket: &Ticket,
        player: &player::Player,
    ) -> anyhow::Result<()> {
        let table = &self.protocol.instance;
        let character = &ticket.character;
        let spawn = player.mover.cell();
        let players = LocalPlayers {
            unk: 0,
            players: vec![(0, character.name.clone())],
        };
        conn.send(&table.encode(&players)?).await?;

        // One empty inventory per slot; the in-game UI resolves every slot it shows.
        let cfg = &self.config.inventories;
        let mut out = Vec::new();
        for slot in InventorySlot::all(&self.data)? {
            if cfg.skip.contains(&slot.id) {
                continue;
            }
            let [width, height] = cfg.size(&slot.id);
            out.extend(table.encode(&AddInventory {
                inventory_id: slot.index as u32 + 1,
                inventory: Inventory {
                    id_key: slot.id_key as u8,
                    slot: slot.index,
                    width,
                    height,
                },
                ..Default::default()
            })?);
        }
        conn.send(&out).await?;

        // Local player 0 controls entity 1, which is added with the class's object type and
        // its initial state (layout: poe_proto::entity).
        let Some(class) = CharacterClass::by_integer_id(&self.data, character.class_id)? else {
            anyhow::bail!("unknown class id {}", character.class_id);
        };
        // New characters wash up on their first visit to an intro area.
        let washed_up =
            !character.intro_done && self.config.spawn.intro_areas.contains(&ticket.area);
        if washed_up {
            self.store.set_intro_done(character.id)?;
        }
        let entity_id = player.entity_id;
        let mut out = table.encode(&PlayerEntity {
            entity_id,
            ..Default::default()
        })?;
        out.extend(table.encode(&AddObject {
            header: SimulationHeader {
                entity_id,
                // Ordering within the frame: +0x14 = 0 (before entity updates), component
                // +0x18 = 0xff. InitialState starts the world, which applies every queued packet
                // without advancing the queue cursor; a nested flush during that apply (starting
                // the WashedUp action from the Player reader) applies queued packets whose
                // ordering matches the current position (0, 0, 0). With 0 here the player was
                // added twice ("Replacing awake object") and the client crashed.
                unk_18: 0xff,
                ..Default::default()
            },
            type_key: class.object_type_key(),
            blob: self.player_blob(character, &class, player, washed_up)?,
        })?);
        if let Ok(path) = std::env::var("POE_NPC") {
            out.extend(npc_experiment(table, &path, spawn)?);
        }
        out.extend(table.encode(&SimulationTick)?);
        conn.send(&out).await?;

        // InitialState must follow 0x24 and the player entity (its handler uses the entity).
        // `frame_ms` is the length of each simulation frame.
        let initial = InitialState {
            frame_ms: self.config.instance.tick_ms as u32,
            characters: vec![CharacterState {
                flags: [
                    0,
                    if self.config.accounts.full_access {
                        poe_proto::instance::CHARACTER_FULL_ACCESS
                    } else {
                        0
                    },
                    0,
                ],
                ..Default::default()
            }],
            players: vec![PlayerState {
                player_id: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        conn.send(&table.encode(&initial)?).await?;
        info!(
            class = %class.metadata_id,
            ?spawn,
            washed_up,
            "player entered the world"
        );
        Ok(())
    }

    /// Initial state of the player character (components of its object template): the
    /// class's object type with the template's default stats, class base stats and level.
    fn player_blob(
        &self,
        character: &TicketCharacter,
        class: &CharacterClass,
        player: &player::Player,
        washed_up: bool,
    ) -> anyhow::Result<Vec<u8>> {
        use poe_proto::entity::{Actor, Component, Life, Player, Positioned, Stats};
        let (grid_x, grid_y) = player.mover.cell();
        let template = ObjectTemplate::load(&self.data, &class.metadata_id)?;
        let stats = poe_game::stats::character_defaults(&template, class, character.level);
        let team = template
            .component("Positioned")
            .iter()
            .find(|(k, _)| k == "team")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let pool = |id: &str| stats.get(id).max(1) as u32;
        info!(
            class = %class.name,
            level = character.level,
            maximum_life = stats.get("maximum_life"),
            maximum_mana = stats.get("maximum_mana"),
            stats = stats.0.len(),
            "player stats"
        );
        Ok(entity::initial_state_blob(
            entity::ENTITY_IN_WAKE_ZONE,
            &[
                Component::Positioned(Positioned {
                    grid_x,
                    grid_y,
                    // Team (low 15 bits), from the template's Positioned section.
                    unk_1e0: team,
                    ..Default::default()
                }),
                Component::Stats(Stats {
                    stats: stats.to_network(&self.stats),
                    ..Default::default()
                }),
                Component::Buffs,
                Component::Life(Life::character(pool("maximum_life"), pool("maximum_mana"))),
                Component::Animated(Some(class.ao_file.clone())),
                Component::Player(Player {
                    name: character.name.clone(),
                    class: class.integer_id as u8,
                    washed_up,
                    ..Default::default()
                }),
                Component::Inventories,
                Component::Actor(Actor {
                    skills: player.actor_skills(),
                }),
            ],
        ))
    }
}
