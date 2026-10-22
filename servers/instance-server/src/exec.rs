//! Running [`crate::commands`] for a session (chat `!` commands and the overlay).

use poe_cluster::{InstanceSpec, Ticket};
use poe_data::views::WorldArea;
use poe_mod_host::CommandAction;
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
    /// Side-channel session id (position reports, debug link; `sync::ClientReports`).
    pub side: u64,
}

/// What a command produced.
#[derive(Default)]
pub struct Outcome {
    pub text: String,
    pub ok: bool,
    /// Raw messages for this session's client.
    pub raw: Vec<u8>,
    /// End this session after the reply (the client is being handed to another instance).
    pub leave: bool,
}

impl Outcome {
    pub(crate) fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), ok: true, ..Default::default() }
    }

    pub(crate) fn err(text: impl Into<String>) -> Self {
        Self { text: text.into(), ok: false, ..Default::default() }
    }
}

/// How many aliases of mod commands may call each other.
const MAX_ALIAS_DEPTH: u8 = 4;

/// The name server messages are sent under.
pub const SERVER_NAME: &str = "<Server>";

impl InstanceServer {
    /// Parses and runs a command line (without the chat prefix).
    pub(crate) fn run_command(&self, ctx: &SessionCtx<'_>, player: &mut player::Player, line: &str) -> Outcome {
        self.run_command_depth(ctx, player, line, 0)
    }

    /// [`run_command`](Self::run_command) for an alias of a mod: `depth` counts the aliases
    /// followed so far.
    fn run_command_depth(&self, ctx: &SessionCtx<'_>, player: &mut player::Player, line: &str, depth: u8) -> Outcome {
        let parsed = match commands::parse(line) {
            Ok(p) => p,
            Err(e) => return Outcome::err(e),
        };
        // The role is checked at the time of the command (a demoted admin loses it at once).
        if parsed.spec.admin && !self.is_admin(&ctx.account) {
            return Outcome::err(format!("{} is for admins only", parsed.spec.name));
        }
        let out = match commands::extra(parsed.spec.name) {
            Some(extra) => self.execute_mod(ctx, player, &parsed, extra, depth),
            None => self.execute(ctx, player, &parsed),
        };
        self.finish(&ctx.account, &parsed, line, out)
    }

    /// A command that a mod added: a fixed reply, an alias of another command line, or the
    /// mod's own code (run by the instance actor; the answer arrives as the chat lines the mod
    /// submits, so this outcome has no text).
    fn execute_mod(
        &self,
        ctx: &SessionCtx<'_>,
        player: &mut player::Player,
        p: &Parsed,
        extra: &commands::Extra,
        depth: u8,
    ) -> anyhow::Result<Outcome> {
        let args = p.get("args").unwrap_or_default();
        let fill = |text: &str| commands::fill(text, &ctx.ticket.character.name, args);
        Ok(match &extra.action {
            CommandAction::Reply(text) => Outcome::ok(fill(text)),
            CommandAction::Alias(line) => {
                if depth >= MAX_ALIAS_DEPTH {
                    return Ok(Outcome::err(format!("{}: aliases nest too deep", p.spec.name)));
                }
                // The target command is checked like a typed one (its role, its arguments).
                return Ok(self.run_command_depth(ctx, player, fill(line).trim(), depth + 1));
            }
            CommandAction::Native => {
                if player.mod_command(&extra.mod_id, p.spec.name, args) {
                    Outcome::ok("")
                } else {
                    Outcome::err("the instance is not running")
                }
            }
        })
    }

    /// Runs a command that needs no game session of its own (the web API's `AdminControl`):
    /// the same registry, role check, log and audit as chat commands. `account` is the label
    /// logged and audited.
    pub(crate) fn run_global(&self, account: &str, actor: &str, line: &str) -> Outcome {
        let parsed = match commands::parse(line) {
            Ok(p) => p,
            Err(e) => return Outcome::err(e),
        };
        if parsed.spec.admin && !self.is_admin(actor) {
            return Outcome::err(format!("{} is for admins only", parsed.spec.name));
        }
        let out = self.execute_global(actor, &parsed).unwrap_or_else(|| {
            Ok(Outcome::err(format!("{} needs your own game session", parsed.spec.name)))
        });
        self.finish(account, &parsed, line, out)
    }

    /// Logs a command's result and appends admin commands to the audit file. Passwords never
    /// reach either.
    pub(crate) fn finish(&self, account: &str, parsed: &Parsed, line: &str, result: anyhow::Result<Outcome>) -> Outcome {
        let shown = match parsed.spec.name {
            "account" => poe_accounts::redact(parsed.get("action").unwrap_or_default(), line),
            _ => line.to_string(),
        };
        let out = match result {
            Ok(o) => o,
            Err(e) => Outcome::err(format!("{e:#}")),
        };
        info!(account = %account, command = %shown, ok = out.ok, result = %out.text, "command");
        if parsed.spec.admin {
            self.audit(account, &shown, &out.text);
        }
        out
    }

    /// The commands that work without a game session. `None`: the command needs one.
    pub(crate) fn execute_global(&self, actor: &str, p: &Parsed) -> Option<anyhow::Result<Outcome>> {
        Some(Ok(match p.spec.name {
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
                        // The player list is written by the instance's actor.
                        format!(
                            "#{} {} seed {} topo {} orient {} players {} [{}]",
                            i.id,
                            i.spec.area,
                            i.spec.seed.map_or("-".into(), |s| s.to_string()),
                            i.spec.topology.map_or("natural".into(), |t| t.to_string()),
                            i.spec.orientation.map_or("natural".into(), |o| o.to_string()),
                            self.actors.players(i.id).unwrap_or(i.players.len()),
                            i.players.join(", ")
                        )
                    })
                    .collect();
                Outcome::ok(format!("{} instances: {}", rows.len(), rows.join("; ")))
            }
            "instance_new" => match self.new_instance(p) {
                Ok(id) => Outcome::ok(format!("created instance #{id}")),
                Err(e) => return Some(Err(e)),
            },
            "close" => {
                let id = p.int("instance").unwrap_or(0) as u32;
                if self.instances.close(id) { Outcome::ok(format!("closed instance #{id}")) } else { Outcome::err(format!("no instance #{id}")) }
            }
            "account" => match poe_accounts::run_account_command(
                &*self.accounts,
                Some(actor),
                p.get("action").unwrap_or_default(),
                p.get("name"),
                p.get("value"),
            ) {
                Ok(done) => {
                    if let Some(id) = done.kick_account {
                        // Ends its login connection and its game session.
                        self.sessions.supersede(id);
                    }
                    Outcome::ok(done.text)
                }
                Err(e) => Outcome::err(e),
            },
            _ => return None,
        }))
    }

    fn execute(&self, ctx: &SessionCtx<'_>, player: &mut player::Player, p: &Parsed) -> anyhow::Result<Outcome> {
        if let Some(done) = self.execute_global(&ctx.account, p) {
            return done;
        }
        let name = &ctx.ticket.character.name;
        Ok(match p.spec.name {
            "help" => match p.get("command") {
                Some(c) => match commands::find(c) {
                    Some(spec) => Outcome::ok(format!("{}{}: {}", self.config.commands.prefix, commands::usage(spec), spec.help)),
                    None => Outcome::err(format!("no command {c}")),
                },
                None => {
                    let list: Vec<&str> = commands::all().filter(|c| ctx.admin || !c.admin).map(|c| c.name).collect();
                    Outcome::ok(format!("commands ({}help <command>): {}", self.config.commands.prefix, list.join(", ")))
                }
            },
            "where" => {
                let (x, y) = player.cell();
                Outcome::ok(format!(
                    "instance {} area {} seed {} cell {x} {y}",
                    ctx.instance.map_or("-".to_string(), |i| i.to_string()),
                    ctx.ticket.area,
                    ctx.seed
                ))
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
            "teleport" => {
                let (x, y) = (p.int("x").unwrap_or(0) as i32, p.int("y").unwrap_or(0) as i32);
                player.teleport((x, y));
                Outcome::ok(format!("teleported to {x} {y}"))
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
    pub(crate) fn new_instance(&self, p: &Parsed) -> anyhow::Result<u32> {
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
        self.hand_off(ctx.ticket.account_id, ctx.ticket.character.clone(), id)
    }

    /// Issues a ticket for instance `id` and gives it to the account's login connection.
    pub(crate) fn hand_off(&self, account_id: u32, mut character: poe_cluster::TicketCharacter, id: u32) -> anyhow::Result<()> {
        let Some(target) = self.instances.get(id) else {
            anyhow::bail!("no instance #{id}");
        };
        // A saved position belongs to the old layout.
        character.location = None;
        let ticket = self.tickets.issue_for(account_id, character, &target.spec.area, Some(id));
        if !self.sessions.transfer(account_id, ticket) {
            anyhow::bail!("no login connection for this account (cannot hand off)");
        }
        Ok(())
    }

    /// Appends an admin command to `[commands] audit_log`.
    pub(crate) fn audit(&self, account: &str, command: &str, result: &str) {
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

    /// Whether an account may use admin commands: its role is admin and it is not disabled.
    pub(crate) fn is_admin(&self, account: &str) -> bool {
        match self.accounts.find(account) {
            Ok(found) => found.is_some_and(|a| a.role == poe_accounts::Role::Admin && !a.disabled),
            Err(e) => {
                warn!(account, "role lookup failed: {e}");
                false
            }
        }
    }
}
