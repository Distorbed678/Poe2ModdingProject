//! Name tables exported from the game data by `cargo xtask deploy-client` into
//! `poe-hook-data/` next to the exe: `stats.tsv` (stat key → id) and `action_types.tsv`
//! (action type key → id).

use std::{collections::HashMap, sync::OnceLock};

fn load(file: &str) -> HashMap<u32, String> {
    let path = poe_hook_shared::exe_dir().join("poe-hook-data").join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        log::warn!("{} missing (run cargo xtask deploy-client)", path.display());
        String::new()
    });
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('\t')?;
            Some((k.parse().ok()?, v.to_owned()))
        })
        .collect()
}

pub fn stat_name(key: u32) -> Option<&'static str> {
    static STATS: OnceLock<HashMap<u32, String>> = OnceLock::new();
    STATS
        .get_or_init(|| load("stats.tsv"))
        .get(&key)
        .map(String::as_str)
}

pub fn action_name(key: u32) -> Option<&'static str> {
    static ACTIONS: OnceLock<HashMap<u32, String>> = OnceLock::new();
    ACTIONS
        .get_or_init(|| load("action_types.tsv"))
        .get(&key)
        .map(String::as_str)
}
