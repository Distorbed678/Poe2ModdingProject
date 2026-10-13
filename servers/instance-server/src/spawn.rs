//! Player start position.
//!
//! The server does not generate terrain yet (M4), so it cannot know where an area's start is.
//! Dev bridge: the client DLL exports the layout it generated
//! (`<layout_dir>/terrain_layout_<tile hash>.tsv`, matched with TerrainHashes: a
//! `size<TAB>w<TAB>h` line, then `x<TAB>y<TAB>tile path` per tile) and the server spawns the
//! player at the centre of the area's configured start tile, or else at the playable tile
//! nearest the map centre. Grid cells per tile: 23.

use std::{path::Path, time::Duration};

use poe_config::SpawnConfig;
use tracing::{info, warn};

pub const GRID_PER_TILE: i32 = 23;

pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub tiles: Vec<(i32, i32, String)>,
}

impl Layout {
    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let mut size = lines.next()?.split('\t');
        if size.next()? != "size" {
            return None;
        }
        let (width, height) = (size.next()?.parse().ok()?, size.next()?.parse().ok()?);
        let tiles = lines
            .filter_map(|l| {
                let mut f = l.splitn(3, '\t');
                Some((
                    f.next()?.parse().ok()?,
                    f.next()?.parse().ok()?,
                    f.next()?.to_owned(),
                ))
            })
            .collect();
        Some(Self {
            width,
            height,
            tiles,
        })
    }

    /// Grid position: the centre of the configured start feature (features span several
    /// tiles), else the centre of the playable tile nearest the map centre.
    pub fn start(&self, start_tile: Option<&str>, filler: &[String]) -> Option<(i32, i32)> {
        let half = GRID_PER_TILE / 2;
        if let Some(name) = start_tile {
            let hits: Vec<_> = self.tiles.iter().filter(|t| t.2.contains(name)).collect();
            let n = hits.len() as i32;
            if n > 0 {
                let centre = |sum: i32| sum * GRID_PER_TILE / n + half;
                return Some((
                    centre(hits.iter().map(|t| t.0).sum()),
                    centre(hits.iter().map(|t| t.1).sum()),
                ));
            }
        }
        let (cx, cy) = (self.width / 2, self.height / 2);
        let (x, y, _) = self
            .tiles
            .iter()
            .filter(|t| !t.2.is_empty() && !filler.iter().any(|f| t.2.contains(f.as_str())))
            .min_by_key(|t| (t.0 - cx).pow(2) + (t.1 - cy).pow(2))?;
        Some((x * GRID_PER_TILE + half, y * GRID_PER_TILE + half))
    }
}

/// Waits briefly for the layout the client exported for this terrain (by tile hash) and
/// picks the start position.
pub async fn from_client_layout(
    config: &SpawnConfig,
    area: &str,
    tile_hash: u32,
) -> Option<(i32, i32)> {
    if config.layout_dir.is_empty() {
        return None;
    }
    let path = Path::new(&config.layout_dir).join(format!("terrain_layout_{tile_hash}.tsv"));
    for _ in 0..30 {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let layout = Layout::parse(&text)?;
            let start = layout.start(
                config.start_tiles.get(area).map(String::as_str),
                &config.filler,
            );
            info!(
                ?start,
                width = layout.width,
                height = layout.height,
                "spawn from client layout"
            );
            return start;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    warn!(file = %path.display(), "no client layout; spawning at the origin");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_configured_tile_then_playable() {
        let text = "size\t4\t4\n0\t0\tMetadata/a/fill.tdt\n1\t1\tMetadata/a/Intro_Tree_01.tdt\n2\t1\tMetadata/a/Intro_Tree_01.tdt\n2\t2\tMetadata/a/ground.tdt\n3\t3\tMetadata/a/blank.tdt\n";
        let layout = Layout::parse(text).unwrap();
        let filler = vec!["fill".to_owned(), "blank".to_owned()];
        // Centre of tiles (1,1) and (2,1).
        assert_eq!(layout.start(Some("Intro_Tree"), &filler), Some((45, 34)));
        assert_eq!(layout.start(None, &filler), Some((57, 57)));
    }
}
