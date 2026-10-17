//! Every area of the game (`poe-hook-data/areas.tsv`, written by `cargo xtask dll-data` or
//! `deploy-client`) for the Instances tab's area picker, and the recently picked ones
//! (`logs/poe-hook/recent_areas.tsv`).

use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone)]
pub struct Area {
    pub id: String,
    pub name: String,
    pub act: i64,
    pub level: i64,
    /// `town`, `hideout`, `map` or `area`.
    pub kind: String,
    /// Topology graph files (file name) and their allowed orientations (bit per
    /// orientation; 0xff = all).
    pub topologies: Vec<(String, u8)>,
}

pub fn all() -> &'static [Area] {
    static AREAS: OnceLock<Vec<Area>> = OnceLock::new();
    AREAS.get_or_init(load)
}

fn load() -> Vec<Area> {
    let path = poe_hook_shared::exe_dir().join("poe-hook-data").join("areas.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        log::warn!("{} missing (run cargo xtask dll-data)", path.display());
        String::new()
    });
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 5 || f[0] == "NULL" {
                return None;
            }
            let topologies = f
                .get(5)
                .unwrap_or(&"")
                .split(',')
                .filter(|t| !t.is_empty())
                .filter_map(|t| {
                    let (graph, mask) = t.rsplit_once(':')?;
                    Some((graph.to_owned(), u8::from_str_radix(mask.trim_start_matches("0x"), 16).unwrap_or(0xff)))
                })
                .collect();
            Some(Area {
                id: f[0].to_owned(),
                name: f[1].to_owned(),
                act: f[2].parse().unwrap_or(0),
                level: f[3].parse().unwrap_or(0),
                kind: f[4].to_owned(),
                topologies,
            })
        })
        .collect()
}

pub fn find(id: &str) -> Option<&'static Area> {
    all().iter().find(|a| a.id.eq_ignore_ascii_case(id))
}

static RECENT: Mutex<Option<Vec<String>>> = Mutex::new(None);
const RECENT_KEEP: usize = 12;

fn recent_path() -> std::path::PathBuf {
    poe_hook_shared::exe_dir().join("logs").join("poe-hook").join("recent_areas.tsv")
}

/// Recently picked area ids, newest first.
pub fn recent() -> Vec<String> {
    RECENT
        .lock()
        .unwrap()
        .get_or_insert_with(|| {
            std::fs::read_to_string(recent_path())
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .clone()
}

pub fn remember(id: &str) {
    let mut list = recent();
    list.retain(|a| !a.eq_ignore_ascii_case(id));
    list.insert(0, id.to_owned());
    list.truncate(RECENT_KEEP);
    let p = recent_path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    if let Err(e) = std::fs::write(&p, list.join("\n")) {
        log::warn!("recent areas: {e}");
    }
    *RECENT.lock().unwrap() = Some(list);
}
