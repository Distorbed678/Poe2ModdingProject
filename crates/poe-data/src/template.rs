//! Object templates (`.ot`): the per-type component defaults shared by client and server.
//!
//! A template is UTF-16 text:
//!
//! ```text
//! version 3
//! extends "Metadata/Characters/Character"
//! Stats
//! {
//!     level = 1
//!     is_player = 1
//! }
//! Buffs { }
//! client { Render { ... } }
//! ```
//!
//! Only top-level component sections are kept (the `client` block is client-only), as
//! `key = value` pairs in file order; keys may repeat (e.g. `basic_action`). `extends`
//! chains are resolved base first, so later files override earlier ones. Some object types
//! (player classes) have no `.ot` of their own: their `.ao` extends the type that has one.

use std::collections::BTreeMap;

use crate::{DataError, GameData, Result};

/// Merged component defaults of an object type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectTemplate {
    /// Component name -> `key = value` pairs in order (values unquoted).
    pub components: BTreeMap<String, Vec<(String, String)>>,
    /// Template files that were read, base first.
    pub files: Vec<String>,
}

impl ObjectTemplate {
    /// Loads the template of an object type by metadata path (without extension), e.g.
    /// `Metadata/Characters/Dex/DexFour`.
    pub fn load(data: &GameData, metadata_path: &str) -> Result<Self> {
        let mut chain = Vec::new();
        let mut next = Some(metadata_path.to_owned());
        while let Some(path) = next.take() {
            if chain.len() > 16 {
                return Err(DataError::Malformed {
                    table: path,
                    message: "extends loop".into(),
                });
            }
            let (file, text) = match read_text(data, &format!("{path}.ot"))? {
                Some(t) => (format!("{path}.ot"), t),
                // No .ot: follow the .ao's `extends` to the type that has one.
                None => match read_text(data, &format!("{path}.ao"))? {
                    Some(t) => (format!("{path}.ao"), t),
                    None => return Err(DataError::FileNotFound(format!("{path}.ot"))),
                },
            };
            next = extends(&text);
            chain.push((file, text));
        }
        let mut out = Self::default();
        for (file, text) in chain.into_iter().rev() {
            if file.ends_with(".ot") {
                for (component, pairs) in parse_sections(&text) {
                    let slot = out.components.entry(component).or_default();
                    for (k, v) in pairs {
                        // Later templates override single-valued keys.
                        if let Some(existing) = slot.iter_mut().find(|(ek, _)| *ek == k)
                            && k != "basic_action"
                        {
                            existing.1 = v;
                        } else {
                            slot.push((k, v));
                        }
                    }
                }
            }
            out.files.push(file);
        }
        Ok(out)
    }

    /// `key = value` pairs of one component (empty if absent).
    pub fn component(&self, name: &str) -> &[(String, String)] {
        self.components.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
}

fn read_text(data: &GameData, path: &str) -> Result<Option<String>> {
    match data.files().read(&path.to_lowercase()) {
        Ok(bytes) => Ok(Some(decode_utf16(&bytes))),
        Err(DataError::FileNotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

fn decode_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_start_matches('\u{feff}')
        .replace('\r', "")
}

fn extends(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("extends")?.trim();
        let name = rest.trim_matches('"');
        (name != "nothing" && !name.is_empty()).then(|| name.to_owned())
    })
}

/// Top-level `Name { key = value ... }` sections; nested blocks (e.g. `client`) are skipped.
fn parse_sections(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut depth = 0usize;
    let mut pending: Option<String> = None;
    let mut current: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // Tokenize braces so `Buffs { }` and `{` on its own line both work.
        let mut rest = line;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix('{') {
                depth += 1;
                if depth == 1 {
                    let name = pending.take().unwrap_or_default();
                    out.push((name, Vec::new()));
                    current = Some(out.len() - 1);
                }
                rest = after.trim_start();
            } else if let Some(after) = rest.strip_prefix('}') {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    current = None;
                }
                rest = after.trim_start();
            } else {
                let end = rest.find(['{', '}']).unwrap_or(rest.len());
                let token = rest[..end].trim();
                if depth == 0 {
                    if !token.is_empty()
                        && !token.starts_with("version")
                        && !token.starts_with("extends")
                        && token != "abstract"
                    {
                        pending = Some(token.to_owned());
                    }
                } else if depth == 1
                    && let (Some(i), Some((k, v))) = (current, token.split_once('='))
                {
                    out[i]
                        .1
                        .push((k.trim().to_owned(), v.trim().trim_matches('"').to_owned()));
                }
                rest = &rest[end..];
            }
        }
    }
    out.retain(|(name, _)| !name.is_empty() && name != "client");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_and_client_block() {
        let text = "version 3\nabstract\nextends \"nothing\"\nStats\n{\n\tlevel = 1\n\tis_player = 1 // c\n}\nBuffs { }\nPositioned \n{\n\tteam = 1\n}\nclient\n{\n\tRender\n\t{\n\t\tx = 1\n\t}\n}\n";
        let s = parse_sections(text);
        assert_eq!(s[0].0, "Stats");
        assert_eq!(
            s[0].1,
            vec![
                ("level".into(), "1".into()),
                ("is_player".into(), "1".into())
            ]
        );
        assert_eq!(s[1].0, "Buffs");
        assert_eq!(
            s[2],
            ("Positioned".into(), vec![("team".into(), "1".into())])
        );
        assert_eq!(s.len(), 3);
        assert_eq!(extends(text), None);
        assert_eq!(
            extends("extends \"Metadata/Characters/Character\""),
            Some("Metadata/Characters/Character".into())
        );
    }
}
