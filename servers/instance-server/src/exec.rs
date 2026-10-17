//! Running [`crate::commands`] for a session (chat `!` commands and the overlay).

use poe_cluster::{InstanceSpec, Ticket};
use poe_data::views::WorldArea;
use tracing::{info, warn};

use crate::{
    InstanceServer,
    commands::{self, Parsed},
    live::ChatOut,
    player,
};

/// The session a command runs in.
pub struct SessionCtx<'a> {
    pub ticket: &'a Ticket,
    pub account: String,
    pub admin: bool,
    pub instance: Option<u32>,
    pub seed: u32,
    /// The client's tile hash for this layout.
    pub tile_hash: u32,
    pub port: u16,
}

/// What a command produced.
#[derive(Default)]
pub struct Outcome {
    pub text: String,
    pub ok: bool,
    /// Simulation replies for this session's player.
    pub replies: Vec<player::Reply>,
    /// Raw messages for this session's client.
    pub raw: Vec<u8>,
    /// End this session after the reply (the client is being handed to another instance).
    pub leave: bool,
}

impl Outcome {
    fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), ok: true, ..Default::default() }
    }

    fn err(text: impl Into<String>) -> Self {
        Self { text: text.into(), ok: false, ..Default::default() }
    }
}

/// The name server messages are sent under.
pub const SERVER_NAME: &str = "<Server>";

impl InstanceServer {
    /// Parses and runs a command line (without the chat prefix).
    pub(crate) fn run_command(&self, ctx: &SessionCtx<'_>, player: &mut player::Player, line: &str) -> Outcome {
        let parsed = match commands::parse(line) {
            Ok(p) => p,
            Err(e) => return Outcome::err(e),
        };
        if parsed.spec.admin && !ctx.admin {
            return Outcome::err(format!("{} is for admins only", parsed.spec.name));
        }
        let out = match self.execute(ctx, player, &parsed) {
            Ok(o) => o,
            Err(e) => Outcome::err(format!("{e:#}")),
        };
        info!(account = %ctx.account, command = %line, ok = out.ok, result = %out.text, "command");
        if parsed.spec.admin {
            self.audit(&ctx.account, line, &out.text);
        }
        out
    }

    fn execute(&self, ctx: &SessionCtx<'_>, player: &mut player::Player, p: &Parsed) -> anyhow::Result<Outcome> {
        let name = &ctx.ticket.character.name;
        Ok(match p.spec.name {
            "help" => match p.get("command") {
                Some(c) => match commands::find(c) {
                    Some(spec) => Outcome::ok(format!("{}{}: {}", self.config.commands.prefix, commands::usage(spec), spec.help)),
                    None => Outcome::err(format!("no command {c}")),
                },
                None => {
                    let list: Vec<&str> = commands::COMMANDS.iter().filter(|c| ctx.admin || !c.admin).map(|c| c.name).collect();
                    Outcome::ok(format!("commands ({}help <command>): {}", self.config.commands.prefix, list.join(", ")))
                }
            },
            "where" => {
                let (x, y) = player.mover.cell();
                Outcome::ok(format!(
                    "instance {} area {} seed {} cell {x} {y}",
                    ctx.instance.map_or("-".to_string(), |i| i.to_string()),
                    ctx.ticket.area,
                    ctx.seed
                ))
            }
            "who" => {
                let rows: Vec<String> = self
                    .live
                    .online()
                    .into_iter()
                    .map(|o| format!("{} ({}) in {} #{}", o.character, o.account, o.area, o.instance.map_or("-".into(), |i| i.to_string())))
                    .collect();
                Outcome::ok(format!("{} online: {}", rows.len(), rows.join("; ")))
            }
            "kick" => {
                let who = p.get("player").unwrap_or_default();
                if self.live.kick(who) { Outcome::ok(format!("kicked {who}")) } else { Outcome::err(format!("{who} is not online")) }
            }
            "mute" => {
                let who = p.get("player").unwrap_or_default();
                let minutes = p.int("minutes").map(|m| m.max(1) as u64);
                self.live.mute(who, minutes);
                Outcome::ok(match minutes {
                    Some(m) => format!("muted {who} for {m} min"),
                    None => format!("muted {who}"),
                })
            }
            "unmute" => {
                let who = p.get("player").unwrap_or_default();
                if self.live.unmute(who) { Outcome::ok(format!("unmuted {who}")) } else { Outcome::err(format!("{who} was not muted")) }
            }
            "announce" => {
                let text = p.get("message").unwrap_or_default().to_string();
                let n = self.live.broadcast(ChatOut { sender: SERVER_NAME.into(), text }, |_| true);
                Outcome::ok(format!("sent to {n} players"))
            }
            "instances" => {
                let list = self.instances.list();
                let rows: Vec<String> = list
                    .iter()
                    .map(|i| {
                        format!(
                            "#{} {} seed {} topo {} orient {} players [{}]",
                            i.id,
                            i.spec.area,
                            i.spec.seed.map_or("-".into(), |s| s.to_string()),
                            i.spec.topology.map_or("natural".into(), |t| t.to_string()),
                            i.spec.orientation.map_or("natural".into(), |o| o.to_string()),
                            i.players.join(", ")
                        )
                    })
                    .collect();
                Outcome::ok(format!("{} instances: {}", rows.len(), rows.join("; ")))
            }
            "instance_new" => {
                let id = self.new_instance(p)?;
                Outcome::ok(format!("created instance #{id}"))
            }
            "transfer" => {
                let id = self.new_instance(p)?;
                self.move_to(ctx, id)?;
                Outcome { leave: true, ..Outcome::ok(format!("moving to new instance #{id}")) }
            }
            "join" => {
                let id = p.int("instance").unwrap_or(0) as u32;
                if self.instances.get(id).is_none() {
                    return Ok(Outcome::err(format!("no instance #{id}")));
                }
                if Some(id) == ctx.instance {
                    return Ok(Outcome::err("you are already in it"));
                }
                self.move_to(ctx, id)?;
                Outcome { leave: true, ..Outcome::ok(format!("moving to instance #{id}")) }
            }
            "close" => {
                let id = p.int("instance").unwrap_or(0) as u32;
                if self.instances.close(id) { Outcome::ok(format!("closed instance #{id}")) } else { Outcome::err(format!("no instance #{id}")) }
            }
            "teleport" => {
                let (x, y) = (p.int("x").unwrap_or(0) as i32, p.int("y").unwrap_or(0) as i32);
                let mut o = Outcome::ok(format!("teleported to {x} {y}"));
                o.replies = player.teleport((x, y));
                o
            }
            "turn_cell" => {
                player.turn_cell = player::TurnCell::parse(p.get("mode").unwrap_or("natural"));
                Outcome::ok(format!("{:?}", player.turn_cell))
            }
            "turn_timing" => {
                player.turn_timing = p.bool("on");
                Outcome::ok(player.turn_timing.to_string())
            }
            "face_cursor" => {
                player.face_cursor = p.bool("on").then_some(self.config.instance.cursor_to_grid);
                Outcome::ok(player.face_cursor.is_some().to_string())
            }
            "terrain" => {
                self.generate_terrain(&ctx.ticket.area, ctx.seed, ctx.tile_hash, ctx.instance);
                Outcome::ok("generating on the server: the result is in the server log")
            }
            "area_info" => {
                let args = format!("{} {}", p.get("area").unwrap_or_default(), p.get("seed").unwrap_or("1"));
                let mut o = Outcome::ok("sent a second AreaInfo");
                o.raw = self.area_info_command(&args, ctx.ticket)?;
                o
            }
            other => {
                warn!(command = other, name, "command without an implementation");
                Outcome::err(format!("{other} is not implemented"))
            }
        })
    }

    /// Creates an instance from `area [seed] [topology] [orientation]`, checking the layout
    /// choice against the area's topologies.
    fn new_instance(&self, p: &Parsed) -> anyhow::Result<u32> {
        let id = p.get("area").unwrap_or_default();
        let Some(area) = WorldArea::find(&self.data, id)? else {
            anyhow::bail!("no area {id} in WorldAreas");
        };
        let (topology, orientation) = (p.topology(), p.orientation());
        if topology.is_some() || orientation.is_some() {
            let entries = poe_world::area::topology_entries(&self.data, &area.id, 1).unwrap_or_default();
            if let Some(t) = topology
                && (t as usize > entries.len() || entries.is_empty())
            {
                anyhow::bail!("{} has {} topologies (1..={0})", area.id, entries.len());
            }
            if let (Some(o), Some(t)) = (orientation, topology) {
                let mask = entries[t as usize - 1].1;
                if mask != 0 && mask != 0xff && mask >> o & 1 == 0 {
                    anyhow::bail!("topology {t} of {} does not allow orientation {o} (mask {mask:#04x})", area.id);
                }
            }
        }
        let spec = InstanceSpec { area: area.id.clone(), seed: p.seed().or(area.fixed_seed()), topology, orientation };
        Ok(self.instances.create(spec, false))
    }

    /// Hands this session's client to instance `id` (its login connection sends
    /// InstanceDetails; the client then leaves this instance).
    fn move_to(&self, ctx: &SessionCtx<'_>, id: u32) -> anyhow::Result<()> {
        let Some(target) = self.instances.get(id) else {
            anyhow::bail!("no instance #{id}");
        };
        let mut character = ctx.ticket.character.clone();
        // A saved position belongs to the old layout.
        character.location = None;
        let ticket = self.tickets.issue_for(ctx.ticket.account_id, character, &target.spec.area, Some(id));
        if !self.sessions.transfer(ctx.ticket.account_id, ticket) {
            anyhow::bail!("no login connection for this account (cannot hand off)");
        }
        Ok(())
    }

    /// Appends an admin command to `[commands] audit_log`.
    fn audit(&self, account: &str, command: &str, result: &str) {
        use std::io::Write;
        let path = &self.config.commands.audit_log;
        if path.as_os_str().is_empty() {
            return;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let line = format!("{ms}\t{account}\t{command}\t{result}\n");
        if let Err(e) = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| f.write_all(line.as_bytes())) {
            warn!(path = %path.display(), "audit log: {e}");
        }
    }

    /// Whether an account may use admin commands (`[admin] accounts`).
    pub(crate) fn is_admin(&self, account: &str) -> bool {
        self.config.admin.accounts.iter().any(|a| a.eq_ignore_ascii_case(account))
    }
}
