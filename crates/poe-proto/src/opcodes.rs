use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{channel}: duplicate {direction:?} opcode {id:#06x} ({a} and {b})")]
    Duplicate {
        channel: String,
        direction: Direction,
        id: u16,
        a: String,
        b: String,
    },
    #[error("{channel}: no {direction:?} message named {name}")]
    UnknownName {
        channel: String,
        direction: Direction,
        name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Sent by the client.
    Client,
    /// Sent by the server.
    Server,
}

#[derive(Debug, Default, Deserialize)]
struct RawTable {
    #[serde(default)]
    client: HashMap<String, u16>,
    #[serde(default)]
    server: HashMap<String, u16>,
}

#[derive(Debug, Default)]
struct Map {
    by_name: HashMap<String, u16>,
    by_id: HashMap<u16, String>,
}

/// Opcodes of one channel (login or instance) in both directions.
#[derive(Debug)]
pub struct OpcodeTable {
    channel: String,
    client: Map,
    server: Map,
}

impl OpcodeTable {
    pub fn parse(channel: &str, text: &str) -> Result<Self, toml::de::Error> {
        let raw: RawTable = toml::from_str(text)?;
        let build = |entries: HashMap<String, u16>| {
            let mut map = Map::default();
            for (name, id) in entries {
                map.by_id.insert(id, name.clone());
                map.by_name.insert(name, id);
            }
            map
        };
        Ok(Self {
            channel: channel.to_owned(),
            client: build(raw.client),
            server: build(raw.server),
        })
    }

    fn map(&self, direction: Direction) -> &Map {
        match direction {
            Direction::Client => &self.client,
            Direction::Server => &self.server,
        }
    }

    /// Opcode of a named message.
    pub fn id(&self, direction: Direction, name: &str) -> Result<u16, ProtocolError> {
        self.map(direction)
            .by_name
            .get(name)
            .copied()
            .ok_or_else(|| ProtocolError::UnknownName {
                channel: self.channel.clone(),
                direction,
                name: name.to_owned(),
            })
    }

    /// Name of an opcode, if known.
    pub fn name(&self, direction: Direction, id: u16) -> Option<&str> {
        self.map(direction).by_id.get(&id).map(String::as_str)
    }

    fn check_duplicates(&self, raw_text: &str) -> Result<(), ProtocolError> {
        // HashMap construction silently keeps one name per id; detect collisions.
        let raw: RawTable = toml::from_str(raw_text).unwrap_or_default();
        for (direction, entries) in [
            (Direction::Client, raw.client),
            (Direction::Server, raw.server),
        ] {
            let mut seen: HashMap<u16, String> = HashMap::new();
            for (name, id) in entries {
                if let Some(other) = seen.insert(id, name.clone()) {
                    return Err(ProtocolError::Duplicate {
                        channel: self.channel.clone(),
                        direction,
                        id,
                        a: other,
                        b: name,
                    });
                }
            }
        }
        Ok(())
    }
}

/// All opcode tables of one client version.
#[derive(Debug)]
pub struct Protocol {
    pub login: OpcodeTable,
    pub instance: OpcodeTable,
}

impl Protocol {
    /// Loads `<dir>/login.toml` and `<dir>/instance.toml`.
    pub fn load(dir: &Path) -> Result<Self, ProtocolError> {
        Ok(Self {
            login: load_table(dir, "login")?,
            instance: load_table(dir, "instance")?,
        })
    }
}

fn load_table(dir: &Path, channel: &str) -> Result<OpcodeTable, ProtocolError> {
    let path = dir.join(format!("{channel}.toml"));
    let text = std::fs::read_to_string(&path).map_err(|source| ProtocolError::Io {
        path: path.clone(),
        source,
    })?;
    let table = OpcodeTable::parse(channel, &text).map_err(|source| ProtocolError::Parse {
        path: path.clone(),
        source,
    })?;
    table.check_duplicates(&text)?;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_protocol_loads() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol/0.5.5e");
        let p = Protocol::load(&dir).unwrap();
        let id = p.login.id(Direction::Client, "KeyExchange").unwrap();
        assert_eq!(p.login.name(Direction::Client, id), Some("KeyExchange"));
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let text = "[client]\nA = 1\nB = 1\n";
        let t = OpcodeTable::parse("x", text).unwrap();
        assert!(t.check_duplicates(text).is_err());
    }
}
