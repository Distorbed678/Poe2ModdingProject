//! Server commands: one registry for chat commands (`!name args`, prefix from `[commands]`)
//! and the client DLL overlay (the debug link sends [`wire`] lines and the overlay builds a
//! form per command), so every command is available in both and they cannot drift apart.
//! Execution is in `lib.rs` (`InstanceServer::run_command`).

/// An argument's type: how the overlay asks for it and how it is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Int,
    /// One word.
    Text,
    /// The rest of the line (messages).
    Rest,
    /// `on`/`off` (also true/false, 1/0).
    Bool,
    Choice(&'static [&'static str]),
    /// `WorldAreas.Id` (the overlay offers its area picker).
    Area,
    /// Terrain seed (`0` or `random`: the server picks one).
    Seed,
    /// 1-based topology of the area (`0` or `natural`: the client's choice).
    Topology,
    /// Region orientation 0..8 (`natural`: the client's choice).
    Orientation,
    /// An instance id (the overlay lists them).
    Instance,
    /// A character name.
    Player,
    Minutes,
}

impl Kind {
    fn wire(self) -> String {
        match self {
            Kind::Int => "int".into(),
            Kind::Text => "text".into(),
            Kind::Rest => "rest".into(),
            Kind::Bool => "bool".into(),
            Kind::Choice(c) => format!("choice={}", c.join("|")),
            Kind::Area => "area".into(),
            Kind::Seed => "seed".into(),
            Kind::Topology => "topology".into(),
            Kind::Orientation => "orientation".into(),
            Kind::Instance => "instance".into(),
            Kind::Player => "player".into(),
            Kind::Minutes => "minutes".into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Arg {
    pub name: &'static str,
    pub kind: Kind,
    pub optional: bool,
}

const fn arg(name: &'static str, kind: Kind) -> Arg {
    Arg { name, kind, optional: false }
}

const fn opt(name: &'static str, kind: Kind) -> Arg {
    Arg { name, kind, optional: true }
}

#[derive(Debug)]
pub struct Spec {
    pub name: &'static str,
    /// Overlay section.
    pub group: &'static str,
    /// Only accounts with the admin role may use it.
    pub admin: bool,
    pub args: &'static [Arg],
    pub help: &'static str,
}

const LAYOUT: [Arg; 4] = [
    arg("area", Kind::Area),
    opt("seed", Kind::Seed),
    opt("topology", Kind::Topology),
    opt("orientation", Kind::Orientation),
];

pub const COMMANDS: &[Spec] = &[
    Spec { name: "help", group: "general", admin: false, args: &[opt("command", Kind::Text)], help: "Lists the commands, or shows one command's arguments." },
    Spec { name: "where", group: "general", admin: false, args: &[], help: "Your instance, area, seed and grid cell." },
    Spec { name: "who", group: "admin", admin: true, args: &[], help: "Online players and their instances." },
    Spec { name: "kick", group: "admin", admin: true, args: &[arg("player", Kind::Player)], help: "Disconnects a player." },
    Spec { name: "mute", group: "admin", admin: true, args: &[arg("player", Kind::Player), opt("minutes", Kind::Minutes)], help: "Stops a player's chat (for good without minutes)." },
    Spec { name: "unmute", group: "admin", admin: true, args: &[arg("player", Kind::Player)], help: "Lets a muted player chat again." },
    Spec { name: "announce", group: "admin", admin: true, args: &[arg("message", Kind::Rest)], help: "A server message to every player." },
    Spec { name: "instances", group: "instances", admin: true, args: &[], help: "Lists the instances (id, area, seed, layout, players)." },
    Spec { name: "instance_new", group: "instances", admin: true, args: &LAYOUT, help: "Creates an instance without entering it." },
    Spec { name: "transfer", group: "instances", admin: true, args: &LAYOUT, help: "Creates an instance and moves you into it." },
    Spec { name: "join", group: "instances", admin: true, args: &[arg("instance", Kind::Instance)], help: "Moves you into an existing instance." },
    Spec { name: "close", group: "instances", admin: true, args: &[arg("instance", Kind::Instance)], help: "Closes an instance (its players are disconnected)." },
    Spec { name: "teleport", group: "player", admin: true, args: &[arg("x", Kind::Int), arg("y", Kind::Int)], help: "Moves you to a grid cell." },
    Spec { name: "terrain", group: "terrain", admin: true, args: &[], help: "Generates this instance's terrain on the server and compares it with the client's." },
    Spec { name: "account", group: "accounts", admin: true, args: &[arg("action", Kind::Choice(&["list", "create", "role", "disable", "enable", "passwd"])), opt("name", Kind::Text), opt("value", Kind::Rest)], help: "Accounts: create <name> <password>, role <name> player|admin, disable|enable <name>, passwd <name> <password>, list." },
    Spec { name: "area_info", group: "terrain", admin: true, args: &[arg("area", Kind::Area), arg("seed", Kind::Seed)], help: "Research: a second AreaInfo on this connection (the client may crash)." },
];

pub fn find(name: &str) -> Option<&'static Spec> {
    COMMANDS.iter().find(|c| c.name.eq_ignore_ascii_case(name))
}

/// `name <arg> [arg]` for messages.
pub fn usage(spec: &Spec) -> String {
    let mut out = spec.name.to_string();
    for a in spec.args {
        if a.optional {
            out.push_str(&format!(" [{}]", a.name));
        } else {
            out.push_str(&format!(" <{}>", a.name));
        }
    }
    out
}

/// The overlay's line for a command: `cmd  name  group  admin  args  help`, args as
/// `name:kind` or `name:kind?` (optional), comma separated.
pub fn wire(spec: &Spec) -> String {
    let args: Vec<String> = spec
        .args
        .iter()
        .map(|a| format!("{}:{}{}", a.name, a.kind.wire(), if a.optional { "?" } else { "" }))
        .collect();
    format!("cmd\t{}\t{}\t{}\t{}\t{}", spec.name, spec.group, spec.admin as u8, args.join(","), spec.help)
}

/// A parsed command line: the command and its arguments (missing optional ones are `None`).
#[derive(Debug)]
pub struct Parsed {
    pub spec: &'static Spec,
    pub args: Vec<Option<String>>,
}

impl Parsed {
    pub fn get(&self, name: &str) -> Option<&str> {
        let i = self.spec.args.iter().position(|a| a.name == name)?;
        self.args.get(i)?.as_deref()
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.get(name)?.parse().ok()
    }

    pub fn bool(&self, name: &str) -> bool {
        matches!(self.get(name), Some("on" | "true" | "1" | "yes"))
    }

    /// Seed: `None` for absent, `0` or `random`.
    pub fn seed(&self) -> Option<u32> {
        self.get("seed").and_then(|v| v.parse().ok()).filter(|&s| s != 0)
    }

    /// Topology: `None` for absent, `0` or `natural`.
    pub fn topology(&self) -> Option<u32> {
        self.get("topology").and_then(|v| v.parse().ok()).filter(|&t| t != 0)
    }

    /// Orientation: `None` for absent or `natural`.
    pub fn orientation(&self) -> Option<u8> {
        self.get("orientation").and_then(|v| v.parse().ok()).filter(|&o| o < 8)
    }
}

/// Parses `name args...` (without the chat prefix) and checks the arguments.
pub fn parse(line: &str) -> Result<Parsed, String> {
    let line = line.trim();
    let (name, mut rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let spec = find(name).ok_or_else(|| format!("unknown command {name:?} (try help)"))?;
    let mut args = Vec::new();
    for a in spec.args {
        rest = rest.trim_start();
        if rest.is_empty() {
            if !a.optional {
                return Err(format!("missing <{}>; usage: {}", a.name, usage(spec)));
            }
            args.push(None);
            continue;
        }
        let word = if a.kind == Kind::Rest {
            let w = rest.to_string();
            rest = "";
            w
        } else {
            let (w, r) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            rest = r;
            w.to_string()
        };
        check(a, &word).map_err(|e| format!("{e}; usage: {}", usage(spec)))?;
        args.push(Some(word));
    }
    if !rest.trim().is_empty() {
        return Err(format!("too many arguments; usage: {}", usage(spec)));
    }
    Ok(Parsed { spec, args })
}

fn check(a: &Arg, w: &str) -> Result<(), String> {
    let int = |w: &str| w.parse::<i64>().is_ok();
    let ok = match a.kind {
        Kind::Int | Kind::Minutes | Kind::Instance => int(w),
        Kind::Bool => matches!(w, "on" | "off" | "true" | "false" | "1" | "0" | "yes" | "no"),
        Kind::Choice(c) => c.contains(&w),
        Kind::Seed => w == "random" || w.parse::<u32>().is_ok(),
        Kind::Topology => w == "natural" || w.parse::<u32>().is_ok(),
        Kind::Orientation => w == "natural" || w.parse::<u8>().is_ok_and(|o| o < 8),
        Kind::Text | Kind::Rest | Kind::Area | Kind::Player => true,
    };
    if ok { Ok(()) } else { Err(format!("bad <{}>: {w:?}", a.name)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_checks() {
        let p = parse("transfer G1_8 1001 natural 3").unwrap();
        assert_eq!(p.get("area"), Some("G1_8"));
        assert_eq!((p.seed(), p.topology(), p.orientation()), (Some(1001), None, Some(3)));
        let p = parse("transfer G1_8").unwrap();
        assert_eq!((p.seed(), p.topology(), p.orientation()), (None, None, None));
        assert!(parse("teleport 1").is_err());
        assert!(parse("teleport 1 x").is_err());
        assert!(parse("teleport 1 2 3").is_err());
        assert_eq!(parse("announce hello there all").unwrap().get("message"), Some("hello there all"));
        assert!(parse("nonsense").is_err());
        assert!(parse("where extra").is_err());
    }

    #[test]
    fn every_command_has_a_wire_line() {
        for c in COMMANDS {
            assert_eq!(wire(c).split('\t').count(), 6, "{}", c.name);
        }
    }
}
