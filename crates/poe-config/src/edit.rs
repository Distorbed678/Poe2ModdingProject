//! Editing config text without losing its comments or layout (the web admin's config editor),
//! and keeping token-like values out of what the editor shows.
//!
//! The server's config holds no secrets today. The editor still hides any string value whose
//! key looks like one (`token`, `secret`, `password`, ...): [`redact`] replaces it by
//! [`HIDDEN`] before the text leaves the server and [`restore_hidden`] puts the stored value
//! back when the text comes in again with the placeholder still in it.

use std::collections::HashMap;

use toml_edit::{Array, DocumentMut, Item, Table, Value, value};

/// What stands in for a hidden value.
pub const HIDDEN: &str = "<hidden>";

/// A value to write into the config text.
#[derive(Debug, Clone, PartialEq)]
pub enum EditValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<String>),
    /// Remove the key (the default applies again).
    Unset,
}

impl EditValue {
    fn to_item(&self) -> Option<Item> {
        Some(match self {
            Self::Bool(b) => value(*b),
            Self::Int(i) => value(*i),
            Self::Float(f) => value(*f),
            Self::Text(t) => value(t.as_str()),
            Self::List(l) => {
                let mut a = Array::new();
                for x in l {
                    a.push(x.as_str());
                }
                value(a)
            }
            Self::Unset => return None,
        })
    }
}

/// Sets `section.key` (or `a.b.key`) values in `text`, keeping comments and the layout of
/// everything else; a replaced value keeps its trailing comment. Missing tables are created.
pub fn set_values(text: &str, changes: &[(String, EditValue)]) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    for (path, new) in changes {
        let parts: Vec<&str> = path.split('.').collect();
        let (key, tables) = parts.split_last().ok_or("empty key")?;
        if key.is_empty() || tables.iter().any(|t| t.is_empty()) {
            return Err(format!("invalid key {path:?}"));
        }
        if matches!(new, EditValue::Unset) && !exists(doc.as_table(), &parts) {
            continue;
        }
        let mut table = doc.as_table_mut();
        for t in tables {
            if !table.contains_key(t) {
                let mut fresh = Table::new();
                fresh.set_implicit(true);
                table.insert(t, Item::Table(fresh));
            }
            table = table
                .get_mut(t)
                .and_then(Item::as_table_mut)
                .ok_or_else(|| format!("{t:?} in {path:?} is not a table"))?;
        }
        match new.to_item() {
            None => {
                table.remove(key);
            }
            Some(mut item) => {
                if let (Some(Item::Value(old)), Item::Value(fresh)) = (table.get(key), &mut item) {
                    *fresh.decor_mut() = old.decor().clone();
                }
                // In place, so the comment lines above the key stay.
                match table.get_mut(key) {
                    Some(slot) => *slot = item,
                    None => {
                        table.insert(key, item);
                    }
                }
            }
        }
    }
    Ok(doc.to_string())
}

fn exists(root: &Table, parts: &[&str]) -> bool {
    let mut item: Option<&Item> = None;
    let mut table = root;
    for (i, p) in parts.iter().enumerate() {
        match table.get(p) {
            None => return false,
            Some(found) => {
                item = Some(found);
                if i + 1 < parts.len() {
                    match found.as_table() {
                        Some(t) => table = t,
                        None => return false,
                    }
                }
            }
        }
    }
    item.is_some()
}

/// Whether a key looks like it holds a secret.
pub fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "private_key",
        "credential",
    ]
    .iter()
    .any(|w| k.contains(w))
}

type Visit<'a> = dyn FnMut(&str, &mut Item) + 'a;

fn walk(table: &mut Table, path: &mut Vec<String>, f: &mut Visit<'_>) {
    for (key, item) in table.iter_mut() {
        path.push(key.get().to_owned());
        match item {
            Item::Table(t) => walk(t, path, f),
            Item::ArrayOfTables(rows) => {
                for (i, row) in rows.iter_mut().enumerate() {
                    path.push(i.to_string());
                    walk(row, path, f);
                    path.pop();
                }
            }
            leaf => {
                if is_sensitive_key(key.get()) {
                    f(&path.join("."), leaf);
                }
            }
        }
        path.pop();
    }
}

/// `text` with every string value of a token-like key replaced by [`HIDDEN`]. Text that does
/// not parse is returned as it is (nothing can be told apart in it).
pub fn redact(text: &str) -> String {
    let Ok(mut doc) = text.parse::<DocumentMut>() else {
        return text.to_owned();
    };
    let mut changed = false;
    walk(doc.as_table_mut(), &mut Vec::new(), &mut |_, item| {
        if let Item::Value(v) = item
            && v.is_str()
            && v.as_str() != Some(HIDDEN)
        {
            let decor = v.decor().clone();
            let mut fresh = Value::from(HIDDEN);
            *fresh.decor_mut() = decor;
            *item = Item::Value(fresh);
            changed = true;
        }
    });
    if changed {
        doc.to_string()
    } else {
        text.to_owned()
    }
}

/// Puts the values of `old` back where `new` still has the [`HIDDEN`] placeholder. Fails when a
/// placeholder has no stored value to restore (a key that was added with the placeholder).
pub fn restore_hidden(new: &str, old: &str) -> Result<String, String> {
    if !new.contains(HIDDEN) {
        return Ok(new.to_owned());
    }
    let mut stored: HashMap<String, Item> = HashMap::new();
    if let Ok(mut old_doc) = old.parse::<DocumentMut>() {
        walk(
            old_doc.as_table_mut(),
            &mut Vec::new(),
            &mut |path, item| {
                stored.insert(path.to_owned(), item.clone());
            },
        );
    }
    let mut doc: DocumentMut = new
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let mut missing: Vec<String> = Vec::new();
    walk(doc.as_table_mut(), &mut Vec::new(), &mut |path, item| {
        if item.as_str() == Some(HIDDEN) {
            match stored.get(path) {
                Some(old_item) => {
                    let mut restored = old_item.clone();
                    if let (Item::Value(r), Item::Value(cur)) = (&mut restored, &*item) {
                        *r.decor_mut() = cur.decor().clone();
                    }
                    *item = restored;
                }
                None => missing.push(path.to_owned()),
            }
        }
    });
    if let Some(path) = missing.first() {
        return Err(format!(
            "{path} is {HIDDEN:?}: a hidden value cannot be set from the editor (the key has no stored value)"
        ));
    }
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "# top comment\n[web]\n# explain\nenabled = true   # trailing\nbind = \"127.0.0.1:8080\"\n\n[server]\nrestart_mode = \"exec\"\n";

    #[test]
    fn set_values_keeps_comments_and_trailing_comments() {
        let out = set_values(
            TEXT,
            &[
                ("web.enabled".into(), EditValue::Bool(false)),
                (
                    "server.motd".into(),
                    EditValue::Text("Welcome \"home\"".into()),
                ),
            ],
        )
        .unwrap();
        assert!(
            out.starts_with("# top comment\n[web]\n# explain\n"),
            "{out}"
        );
        assert!(out.contains("enabled = false   # trailing"), "{out}");
        assert!(out.contains("motd = "), "{out}");
        let parsed: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(parsed["server"]["motd"].as_str(), Some("Welcome \"home\""));
    }

    #[test]
    fn set_values_creates_tables_lists_and_unsets() {
        let out = set_values(
            TEXT,
            &[
                ("metrics.interval_secs".into(), EditValue::Int(10)),
                (
                    "web.cors_origins".into(),
                    EditValue::List(vec!["http://a".into(), "http://b".into()]),
                ),
                ("web.bind".into(), EditValue::Unset),
                ("nothing.here".into(), EditValue::Unset),
            ],
        )
        .unwrap();
        let parsed: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(parsed["metrics"]["interval_secs"].as_integer(), Some(10));
        assert_eq!(parsed["web"]["cors_origins"].as_array().unwrap().len(), 2);
        assert!(parsed["web"].get("bind").is_none());
        assert!(parsed.get("nothing").is_none());
    }

    #[test]
    fn set_values_rejects_bad_paths_and_text() {
        assert!(set_values(TEXT, &[("web.enabled.x".into(), EditValue::Int(1))]).is_err());
        assert!(set_values(TEXT, &[(".x".into(), EditValue::Int(1))]).is_err());
        assert!(set_values("a = [", &[("a.b".into(), EditValue::Int(1))]).is_err());
    }

    #[test]
    fn token_like_strings_are_hidden_and_come_back() {
        let old = "[web]\nenabled = true\nadmin_token = \"abc123\"  # keep\nweb_token_ttl_secs = 60\n[[hooks]]\nsecret = \"s3\"\n";
        let shown = redact(old);
        assert!(
            !shown.contains("abc123") && !shown.contains("s3"),
            "{shown}"
        );
        assert!(
            shown.contains("admin_token = \"<hidden>\"  # keep"),
            "{shown}"
        );
        // Numbers under token-like keys are not secrets.
        assert!(shown.contains("web_token_ttl_secs = 60"), "{shown}");
        // Edited elsewhere, the placeholder still in: the stored values return.
        let edited = shown.replace("enabled = true", "enabled = false");
        let saved = restore_hidden(&edited, old).unwrap();
        assert!(
            saved.contains("enabled = false")
                && saved.contains("abc123")
                && saved.contains("\"s3\""),
            "{saved}"
        );
        // A placeholder with nothing to restore is refused.
        assert!(restore_hidden("[x]\npassword = \"<hidden>\"\n", old).is_err());
        // A real new value is kept.
        let changed = shown.replace("\"<hidden>\"  # keep", "\"new\"  # keep");
        assert!(
            restore_hidden(&changed, old)
                .unwrap()
                .contains("admin_token = \"new\"")
        );
        // Nothing token-like: untouched, byte for byte.
        assert_eq!(redact(TEXT), TEXT);
    }
}
