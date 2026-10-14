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
    /// HNC side-channel key derivation (`hnc.toml`).
    pub hnc: HncKeys,
}

/// How the client derives the HNC side channel's key material from the session's.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct HncKeys {
    /// 64 bytes, hex.
    pub key_xor: String,
    /// Byte pairs swapped after the XOR, in order.
    pub swaps: Vec<[usize; 2]>,
}

impl HncKeys {
    /// The HNC key material for a session's key material.
    pub fn derive(&self, session: &[u8; 64]) -> [u8; 64] {
        let mut out = *session;
        let xor = self.key_xor.as_bytes();
        for (i, b) in out.iter_mut().enumerate() {
            let hex = std::str::from_utf8(&xor[i * 2..i * 2 + 2]).unwrap_or("00");
            *b ^= u8::from_str_radix(hex, 16).unwrap_or(0);
        }
        for [a, b] in &self.swaps {
            out.swap(*a, *b);
        }
        out
    }
}

impl Protocol {
    /// Loads `<dir>/login.toml`, `<dir>/instance.toml` and `<dir>/hnc.toml`.
    pub fn load(dir: &Path) -> Result<Self, ProtocolError> {
        let path = dir.join("hnc.toml");
        let text = std::fs::read_to_string(&path).map_err(|source| ProtocolError::Io {
            path: path.clone(),
            source,
        })?;
        let hnc: HncKeys =
            toml::from_str(&text).map_err(|source| ProtocolError::Parse { path, source })?;
        Ok(Self {
            login: load_table(dir, "login")?,
            instance: load_table(dir, "instance")?,
            hnc,
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
        assert_eq!(p.hnc.key_xor.len(), 128);
        // XOR with zeros gives the table, then the swaps apply.
        let k = p.hnc.derive(&[0; 64]);
        assert_eq!(k[0x15], 0xe5);
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let text = "[client]\nA = 1\nB = 1\n";
        let t = OpcodeTable::parse("x", text).unwrap();
        assert!(t.check_duplicates(text).is_err());
    }
}
