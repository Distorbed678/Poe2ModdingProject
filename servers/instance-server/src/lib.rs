//! Instance server: hosts areas. The login server hands clients over with a ticket;
//! the instance server validates it, sends the area and runs the simulation.
//!
//! Connection opening (Documentation/protocol/instance.md):
//! 1. client `Hello` (plaintext) carrying the hand-off ticket,
//! 2. server `EnableEncryption` (plaintext), then both directions switch to Salsa20 with the
//!    ticket's key material,
//! 3. server `AreaInfo` (area + seed): the client generates the terrain and loads the area.

use std::{path::PathBuf, sync::Arc, time::Duration};

use poe_cluster::{Ticket, TicketCharacter, TicketRegistry};
use poe_config::Config;
use poe_game::stats::StatRegistry;
use poe_store::Store;

pub mod chat;
pub mod commands;
mod exec;
pub mod live;
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
        AddInventory, AddObject, AimTarget, AreaInfo, CharacterState, ChatJoinRequest, ChatJoined,
        EnableEncryption, ExitReport, FrameReport, Hello, InitialState, Inventory, LeaveGame,
        LocalPlayers, MoveDirection, PauseRequest, PingReply, PlayerEntity, PlayerState, SetPause,
        SimulationHeader, SimulationTick, SkillReleased, SprintInput, StashTab, TerrainHashes,
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
    /// Instances of areas: which one a session is in, its seed and layout choice.
    pub instances: Arc<dyn poe_cluster::InstanceDirectory>,
    /// Sessions in game (who, kick, chat delivery, mutes).
    pub live: Arc<live::Live>,
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
        // Idle instances close after `[instances] idle_minutes` (towns and hideouts stay).
        let idle = self.config.instances.idle_minutes;
        if idle > 0 {
            let instances = self.instances.clone();
            tokio::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_secs(30));
                loop {
                    every.tick().await;
                    for id in instances.expire(Duration::from_secs(idle * 60)) {
                        info!(instance = id, "closed idle instance");
                    }
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

        // Terrain seed sweep: this session generates the next listed layout only.
        let sweep = sweep_list(&self.config.instance.sweep).and_then(|list| {
            let i = SWEEP_NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let entry = list.get(i).cloned()?;
            Some((i, list.len(), entry))
        });
        let (instance, seed) = match &sweep {
            Some((_, _, (area, seed))) => {
                conn.send(&self.area_info_command(&format!("{area} {seed}"), &ticket)?)
                    .await?;
                (None, *seed)
            }
            None => {
                let (instance, seed) = self.send_area(&mut conn, &ticket).await?;
                (Some(instance), seed)
            }
        };

        let hashes = match &sweep {
            // A layout the client cannot generate never gets an answer.
            Some((_, _, (area, seed))) => {
                match tokio::time::timeout(Duration::from_secs(60), self.await_hashes(&mut conn))
                    .await
                {
                    Ok(h) => h?,
                    Err(_) => {
                        self.record_sweep(area, *seed, None);
                        warn!(area, seed, "terrain sweep: no hashes within 60 s");
                        return Ok(());
                    }
                }
            }
            None => self.await_hashes(&mut conn).await?,
        };
        let Some(hashes) = hashes else {
            info!("client disconnected during terrain generation");
            return Ok(());
        };
        if let Some((i, n, (area, seed))) = sweep {
            self.record_sweep(&area, seed, Some(&hashes));
            info!(
                entry = i + 1,
                of = n,
                area,
                seed,
                tile_hash = hashes.hash_a,
                "terrain sweep entry done"
            );
            if i + 1 == n {
                info!("terrain sweep finished");
            }
            return Ok(());
        }
        // hash_a is the tile hash, hash_b the doodad hash (as in the client log).
        info!(
            unk = hashes.unk,
            tile_hash = hashes.hash_a,
            doodad_hash = hashes.hash_b,
            "terrain generated by client"
        );
        if self.config.terrain.generate {
            self.generate_terrain(&ticket.area, seed, hashes.hash_a, instance);
        }
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
        player.turn_cell = player::TurnCell::parse(&self.config.instance.turn_cell);
        player.turn_timing = self.config.instance.turn_timing;
        player.face_cursor = self
            .config
            .instance
            .face_cursor
            .then_some(self.config.instance.cursor_to_grid);
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

        let name = ticket.character.name.clone();
        if let Some(id) = instance {
            self.instances.join(id, &name);
        }
        let account = self
            .store
            .account_by_id(ticket.account_id)
            .ok()
            .flatten()
            .map(|a| a.name)
            .unwrap_or_else(|| format!("account {}", ticket.account_id));
        let ctx = exec::SessionCtx {
            ticket: &ticket,
            admin: self.is_admin(&account),
            account: account.clone(),
            instance,
            seed,
            tile_hash: hashes.hash_a,
            port: client_port,
        };
        let registration = self.live.register(ticket.account_id, &account, &name, instance, &ticket.area, client_port);
        self.send_registry(&ctx);
        let closed = instance.and_then(|id| self.instances.closed(id));
        let result = self
            .run_session(&mut conn, &mut hnc, &ctx, &mut player, closed, registration)
            .await;
        self.live.unregister(&name, client_port);
        if let Some(id) = instance {
            self.instances.leave(id, &name);
        }
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
        ctx: &exec::SessionCtx<'_>,
        player: &mut player::Player,
        mut closed: Option<tokio::sync::watch::Receiver<bool>>,
        registration: live::Registration,
    ) -> anyhow::Result<()> {
        let ticket = ctx.ticket;
        let client_port = ctx.port;
        let live::Registration { mut kicked, chat: mut chat_in } = registration;
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
                () = instance_closed(&mut closed) => {
                    info!("instance closed: disconnecting the player");
                    return Ok(());
                }
                () = was_kicked(&mut kicked) => {
                    info!("kicked: disconnecting the player");
                    return Ok(());
                }
                Some(line) = chat_in.recv() => {
                    conn.send(&self.protocol.instance.encode(&chat_line(&line))?).await?;
                }
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
                    let mut replies = player.tick(self.config.instance.tick_ms as f32 / 1000.0);
                    let mut extra = Vec::new();
                    if self.config.instance.debug_link {
                        // Overlay commands: the same registry as chat commands.
                        for command in self.reports.take_commands(client_port) {
                            if command == "registry" {
                                self.send_registry(ctx);
                                continue;
                            }
                            let out = self.run_command(ctx, player, &command);
                            replies.extend(out.replies);
                            extra.extend(out.raw);
                            let status = if out.ok { "ok" } else { "error" };
                            self.reports.send_debug(
                                client_port,
                                &format!("ack\t{command}\t{status}: {}\n", out.text.replace(['\t', '\n'], " ")),
                            );
                        }
                        if ticks.is_multiple_of(30) {
                            self.send_instances(ctx);
                        }
                        if ticks.is_multiple_of(2) {
                            self.reports.send_debug(
                                client_port,
                                &format!("tick\t{ticks}\n{}", player.debug_state()),
                            );
                        }
                    }
                    let mut out = extra;
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

    /// Server-side terrain (`[terrain] generate`, `terrain` command): generates the layout in
    /// the background and logs whether its tile hash matches the client's.
    pub(crate) fn generate_terrain(&self, area: &str, seed: u32, client_hash: u32, instance: Option<u32>) {
        let data = self.data.clone();
        let area = area.to_string();
        // The instance's forced layout, as sent in AreaInfo.
        let forced = instance
            .and_then(|id| self.instances.get(id))
            .filter(|i| i.spec.topology.is_some() || i.spec.orientation.is_some())
            .map(|i| poe_world::area::Forced { topology: i.spec.topology, orientation: i.spec.orientation });
        tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let mut rng = poe_world::rng::TerrainRng::new(seed);
            match poe_world::area::generate_with(&data, &area, &mut rng, forced) {
                Ok(g) => {
                    let ours = g.map.tile_hash();
                    if ours == client_hash {
                        info!(%area, seed, tile_hash = ours, took = ?started.elapsed(), "server terrain matches the client");
                    } else {
                        warn!(%area, seed, ours, client = client_hash, graph = %g.graph, "server terrain differs from the client");
                    }
                }
                Err(e) => warn!(%area, seed, "server terrain: {e}"),
            }
        });
    }

    /// Waits for the client's TerrainHashes (sent after it generated the area).
    async fn await_hashes(&self, conn: &mut Connection) -> anyhow::Result<Option<TerrainHashes>> {
        let hashes_op = self
            .protocol
            .instance
            .id(TerrainHashes::DIRECTION, TerrainHashes::NAME)?;
        Ok(conn
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
            .await?)
    }

    /// Appends a sweep result line (`area, seed, tile hash, doodad hash`).
    fn record_sweep(&self, area: &str, seed: u32, hashes: Option<&TerrainHashes>) {
        use std::io::Write;
        let path = &self.config.instance.sweep_results;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let line = match hashes {
            Some(h) => format!("{area}\t{seed}\t{}\t{}\n", h.hash_a, h.hash_b),
            None => format!("{area}\t{seed}\tFAILED\t\n"),
        };
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = written {
            warn!(path = %path.display(), "sweep results: {e}");
        }
    }

    /// The `area <id> <seed>` dev command's AreaInfo message.
    fn area_info_command(&self, args: &str, ticket: &Ticket) -> anyhow::Result<Vec<u8>> {
        let mut words = args.split_whitespace();
        let (Some(id), Some(seed)) = (words.next(), words.next()) else {
            anyhow::bail!("usage: area <WorldAreas.Id> <seed>");
        };
        let Some(area) = WorldArea::find(&self.data, id)? else {
            anyhow::bail!("no area {id}");
        };
        let msg = AreaInfo {
            instance_id: rand::random(),
            area_hash: area.hash16,
            league: ticket.character.league.clone(),
            seed: seed.parse()?,
            ..Default::default()
        };
        info!(area = %area.id, seed = msg.seed, "dev: sending area");
        Ok(self.protocol.instance.encode(&msg)?)
    }

    /// Sends the area; returns the terrain seed (a saved location's seed when the character
    /// left this area, so it comes back to the same layout).
    /// A stat's network key (`Stats` row + 1) by id.
    fn stat_key(&self, id: &str) -> anyhow::Result<u16> {
        self.stats.key(id).map(|k| k as u16).ok_or_else(|| anyhow::anyhow!("no stat {id}"))
    }

    /// The instance a ticket enters: its own, else the area's newest, else a new one.
    fn ticket_instance(&self, ticket: &Ticket) -> anyhow::Result<u32> {
        if let Some(id) = ticket.instance
            && self.instances.get(id).is_some()
        {
            return Ok(id);
        }
        if let Some(id) = self.instances.find(&ticket.area) {
            return Ok(id);
        }
        let Some(area) = WorldArea::find(&self.data, &ticket.area)? else {
            anyhow::bail!("ticket area {} not in game data", ticket.area);
        };
        Ok(self.instances.create(
            poe_cluster::InstanceSpec { area: area.id.clone(), seed: area.fixed_seed(), ..Default::default() },
            area.is_town || area.is_hideout,
        ))
    }

    /// AreaInfo for the session's instance; returns the instance and its seed.
    async fn send_area(&self, conn: &mut Connection, ticket: &Ticket) -> anyhow::Result<(u32, u32)> {
        let Some(area) = WorldArea::find(&self.data, &ticket.area)? else {
            anyhow::bail!("ticket area {} not in game data", ticket.area);
        };
        let instance_id = self.ticket_instance(ticket)?;
        let seed = {
            let mut rng = rand::rng();
            // RE experiment: `POE_SEED` pins the layout seed (to reproduce a layout).
            let pinned = std::env::var("POE_SEED").ok().and_then(|v| v.parse().ok());
            let saved = ticket.character.location.map(|(seed, _, _)| seed);
            let spec_seed = self.instances.get(instance_id).and_then(|i| i.spec.seed);
            let seed = pinned
                .or(spec_seed)
                .or(area.fixed_seed())
                .or(saved)
                .or_else(|| self.seeds.pick(&area.id, rng.next_u32()))
                .unwrap_or_else(|| rng.next_u32());
            // The first player decides an instance's seed; later ones share it.
            self.instances.settle_seed(instance_id, seed).unwrap_or(seed)
        };
        // A forced layout (`transfer ... topology orientation`): the generator reads
        // `map_force_topology` (1-based) and `map_force_orientation` (orientation + 1).
        let spec = self.instances.get(instance_id).map(|i| i.spec);
        let mut forced = Vec::new();
        if let Some(t) = spec.as_ref().and_then(|s| s.topology) {
            forced.push((self.stat_key("map_force_topology")?, t as i32));
        }
        if let Some(o) = spec.as_ref().and_then(|s| s.orientation) {
            forced.push((self.stat_key("map_force_orientation")?, o as i32 + 1));
        }
        let msg = AreaInfo {
            instance_id,
            area_hash: area.hash16,
            // Official servers send the character's league here.
            league: ticket.character.league.clone(),
            seed,
            stats: (!forced.is_empty()).then(|| poe_proto::instance::AreaStats { first: forced.clone(), second: Vec::new(), more: Vec::new() }),
            ..Default::default()
        };
        if !forced.is_empty() {
            info!(?forced, "forcing the layout (AreaInfo stats)");
        }
        info!(area = %area.id, seed = msg.seed, instance = msg.instance_id, "sending area");
        conn.send(&self.protocol.instance.encode(&msg)?).await?;
        Ok((instance_id, seed))
    }
}

/// A ChatMessage for a chat line (layout as in official captures).
fn chat_line(line: &live::ChatOut) -> poe_proto::instance::ChatMessage {
    poe_proto::instance::ChatMessage { unk_10: 1, sender: line.sender.clone(), text: line.text.clone(), unk_a2: 5, ..Default::default() }
}

impl InstanceServer {
    /// The command registry for the overlay (debug link): `cmds` datagrams of `cmd` lines
    /// (`commands::wire`), then whether this account is an admin.
    fn send_registry(&self, ctx: &exec::SessionCtx<'_>) {
        if !self.config.instance.debug_link {
            return;
        }
        let mut chunk = String::from("cmds\tbegin\n");
        for spec in commands::COMMANDS {
            let line = commands::wire(spec);
            if chunk.len() + line.len() > 3000 {
                self.reports.send_debug(ctx.port, &chunk);
                chunk = String::from("cmds\tmore\n");
            }
            chunk.push_str(&line.replace('\n', " "));
            chunk.push('\n');
        }
        chunk.push_str(&format!("you\t{}\t{}\t{}\n", ctx.account, ctx.admin as u8, self.config.commands.prefix));
        self.reports.send_debug(ctx.port, &chunk);
    }

    /// The instance list for the overlay's Instances tab: `instances` datagrams of `inst`
    /// lines (`id, area, seed, topology, orientation, persistent, empty seconds, current,
    /// players`).
    fn send_instances(&self, ctx: &exec::SessionCtx<'_>) {
        let list = self.instances.list();
        let mut chunk = String::from("instances\tbegin\n");
        for i in &list {
            let line = format!(
                "inst\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                i.id,
                i.spec.area,
                i.spec.seed.map_or(String::new(), |s| s.to_string()),
                i.spec.topology.map_or(String::new(), |t| t.to_string()),
                i.spec.orientation.map_or(String::new(), |o| o.to_string()),
                i.persistent as u8,
                i.empty_since.map_or(String::new(), |t| t.elapsed().as_secs().to_string()),
                (Some(i.id) == ctx.instance) as u8,
                i.players.join(",")
            );
            if chunk.len() + line.len() > 3000 {
                self.reports.send_debug(ctx.port, &chunk);
                chunk = String::from("instances\tmore\n");
            }
            chunk.push_str(&line);
        }
        chunk.push_str(&format!("idle\t{}\n", self.config.instances.idle_minutes));
        self.reports.send_debug(ctx.port, &chunk);
    }
}

/// Resolves when the session is kicked.
async fn was_kicked(rx: &mut tokio::sync::watch::Receiver<bool>) {
    if rx.wait_for(|k| *k).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Resolves when the session's instance is closed; never without an instance.
async fn instance_closed(rx: &mut Option<tokio::sync::watch::Receiver<bool>>) {
    if let Some(rx) = rx {
        // A closed instance's sender is dropped: an error also means closed.
        let _ = rx.wait_for(|c| *c).await;
        return;
    }
    std::future::pending().await
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
            player::Reply::Orientation(m) => self.trace(
                port,
                "out-facing",
                format_args!("{:.3}\t{}", m.angle, m.locked),
            ),
            player::Reply::Aim(m) => self.trace(
                port,
                "out-aim",
                format_args!("{}\t{}\t{}", m.kind, m.x, m.y),
            ),
            player::Reply::Component(m) => self.trace(
                port,
                "out-component",
                format_args!("{}\t{:02x?}", m.component, m.delta),
            ),
        }
        let table = &self.protocol.instance;
        Ok(match reply {
            player::Reply::Direction(m) => table.encode(m)?,
            player::Reply::Start(m) => table.encode(m)?,
            player::Reply::Sprint(m) => table.encode(m)?,
            player::Reply::Orientation(m) => table.encode(m)?,
            player::Reply::Aim(m) => table.encode(m)?,
            player::Reply::Component(m) => table.encode(m)?,
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
            // The cursor: echoed as SetAimTarget in the next frame (player facing).
            AimTarget::NAME => {
                let req = AimTarget::decode(r)?;
                self.trace(
                    port,
                    "in-aim",
                    format_args!("{}\t{:?}\t{:?}", req.mode, req.cell, req.entity),
                );
                player.on_aim_target(&req);
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
                if let Some(name) = self.live.name_on_port(port) {
                    self.live.set_channel(&name, req.kind != 0, channel);
                }
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
                    // (M8), each `..._base_...` stat is also sent under its name without
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
        // Local player 1 (as on official servers: the client copies the index into its own
        // messages, e.g. `0062 01 ..`).
        let players = LocalPlayers {
            unk: 0,
            players: vec![(LOCAL_PLAYER, character.name.clone())],
        };
        conn.send(&table.encode(&players)?).await?;
        // Official order: the player's entity id, then its inventories.
        conn.send(&table.encode(&PlayerEntity {
            unk: LOCAL_PLAYER,
            local_index: LOCAL_PLAYER,
            entity_id: player.entity_id,
        })?)
        .await?;

        // The inventories an official server creates (protocol/<version>/inventories.tsv),
        // else one empty inventory per Inventories.dat slot.
        let mut out = Vec::new();
        let official = load_inventories(&self.config.game.protocol_dir.join("inventories.tsv"));
        if !official.is_empty() {
            for (id, inventory) in official {
                out.extend(table.encode(&AddInventory {
                    unk_10: LOCAL_PLAYER,
                    inventory_id: id,
                    inventory,
                    ..Default::default()
                })?);
            }
        } else {
            let cfg = &self.config.inventories;
            for slot in InventorySlot::all(&self.data)? {
                if cfg.skip.contains(&slot.id) {
                    continue;
                }
                let [width, height] = cfg.size(&slot.id);
                out.extend(table.encode(&AddInventory {
                    unk_10: LOCAL_PLAYER,
                    inventory_id: slot.index as u32 + 1,
                    inventory: Inventory {
                        id_key: slot.id_key as u8,
                        slot: slot.index,
                        width,
                        height,
                        flags: 0,
                    },
                    ..Default::default()
                })?);
            }
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
        let mut out = Vec::new();
        out.extend(table.encode(&AddObject {
            header: SimulationHeader {
                entity_id,
                // Ordering within the frame: +0x14 = 0 (before entity updates), component
                // +0x18 = 0xff. InitialState starts the world, which applies every queued packet
                // without advancing the queue cursor; a nested flush during that apply (starting
                // the WashedUp action from the Player reader) applies queued packets whose
                // ordering matches the current position (0, 0, 0). With 0 here the player was
                // added twice ("Replacing awake object") and the client crashed. Official servers
                // send (0xffffffff, 0, 0): after everything else in the frame.
                unk_14: 0xffff_ffff,
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
        // Flag values not understood yet are the official ones (official-comparison.md).
        let initial = InitialState {
            frame_ms: self.config.instance.tick_ms as u32,
            flags: 0x07,
            characters: vec![CharacterState {
                stash_tabs: (self.config.accounts.stash_tabs.iter().enumerate())
                    .map(|(i, name)| StashTab::plain(i as u16, name))
                    .collect(),
                flags: [
                    // Bits 3 and 5 (+0x3ece, +0x3ed0); bit 6: account mode 0 (else 3).
                    0x68,
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
                // Character 0, bit 6 (entry +7).
                character: 0x40,
                flags: 0x07,
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

/// Next sweep entry (shared by all sessions of this server process).
static SWEEP_NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The sweep list (`<WorldAreas.Id> <seed> [count]` lines, `#` comments); `POE_SWEEP`
/// overrides the configured path. None when off or unreadable.
fn sweep_list(configured: &std::path::Path) -> Option<Vec<(String, u32)>> {
    let path = std::env::var_os("POE_SWEEP")
        .map(PathBuf::from)
        .unwrap_or_else(|| configured.to_path_buf());
    if path.as_os_str().is_empty() {
        return None;
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            warn!(path = %path.display(), "sweep list: {e}");
            return None;
        }
    };
    let mut list = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let w: Vec<&str> = line.split_whitespace().collect();
        let (Some(area), Some(Ok(seed))) = (w.first(), w.get(1).map(|s| s.parse::<u32>())) else {
            warn!(line, "sweep list: bad line");
            continue;
        };
        let count = w.get(2).and_then(|c| c.parse::<u32>().ok()).unwrap_or(1);
        list.extend((0..count).map(|i| (area.to_string(), seed.wrapping_add(i))));
    }
    Some(list)
}

/// The local player index the server gives the character (official servers use 1).
const LOCAL_PLAYER: u8 = 1;

/// `inventories.tsv`: `id, id_key, slot, width, height, flags` per line (`#` comments, a
/// header line). Empty when the file is missing.
fn load_inventories(path: &std::path::Path) -> Vec<(u32, Inventory)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let num = |s: &str| -> Option<u32> {
        match s.strip_prefix("0x") {
            Some(h) => u32::from_str_radix(h, 16).ok(),
            None => s.parse().ok(),
        }
    };
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("id\t"))
        .filter_map(|l| {
            let f: Vec<u32> = l.split('\t').map(num).collect::<Option<_>>()?;
            let [id, id_key, slot, width, height, flags] = f[..] else {
                return None;
            };
            Some((
                id,
                Inventory {
                    id_key: id_key as u8,
                    slot: slot as u8,
                    width: width as u8,
                    height: height as u8,
                    flags: flags as u8,
                },
            ))
        })
        .collect()
}
