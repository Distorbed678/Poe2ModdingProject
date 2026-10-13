//! Player start position.
//!
//! The server does not generate terrain yet (M4), so it cannot know where an area's start is.
//! Dev bridge: the client DLL exports the layout it generated
//! (`<layout_dir>/terrain_layout_<tile hash>.tsv`, matched with TerrainHashes: a
//! `size<TAB>w<TAB>h` line, then `x<TAB>y<TAB>tile path` per tile) and the server spawns the
//! player near the centre of the area's configured start feature, or else of the playable
//! tile nearest the map centre. Grid cells per tile: 23.
//!
//! The point is then snapped to the nearest walkable cell, using the client's cell grids
//! (`terrain_grid_<tile hash>.bin`, layer 0 non-zero = walkable). Standing on a
//! non-walkable cell inside the playable area leaves the view grey.

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

/// Cell grids exported by the DLL: `b"PWG1"`, u32 cells_x, cells_y, bytes_per_row, layers,
/// then each layer (4 bits per cell, low nibble first).
pub struct Grid {
    pub cells_x: i32,
    pub cells_y: i32,
    bytes_per_row: usize,
    layers: Vec<Vec<u8>>,
}

impl Grid {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.get(..4)? != b"PWG1" {
            return None;
        }
        let word = |i: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                data.get(4 + i * 4..8 + i * 4)?.try_into().ok()?,
            ))
        };
        let (cells_x, cells_y, bytes_per_row, count) = (word(0)?, word(1)?, word(2)?, word(3)?);
        let size = bytes_per_row as usize * cells_y as usize;
        let layers = (0..count as usize)
            .map(|i| Some(data.get(20 + i * size..20 + (i + 1) * size)?.to_vec()))
            .collect::<Option<_>>()?;
        Some(Self {
            cells_x: cells_x as i32,
            cells_y: cells_y as i32,
            bytes_per_row: bytes_per_row as usize,
            layers,
        })
    }

    pub fn cell(&self, layer: usize, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.cells_x || y >= self.cells_y {
            return 0;
        }
        let b = self.layers[layer][y as usize * self.bytes_per_row + x as usize / 2];
        if x & 1 == 1 { b >> 4 } else { b & 0xf }
    }

    /// Walkable: layer 0 non-zero on the cell and its 8 neighbours at distance 2.
    pub fn walkable(&self, x: i32, y: i32) -> bool {
        (-2..=2).step_by(2).all(|dy| {
            (-2..=2)
                .step_by(2)
                .all(|dx| self.cell(0, x + dx, y + dy) != 0)
        })
    }

    /// Nearest walkable cell to `(x, y)` within `radius` cells (square rings outward).
    pub fn nearest_walkable(&self, (x, y): (i32, i32), radius: i32) -> Option<(i32, i32)> {
        for r in 0..=radius {
            let mut best: Option<((i32, i32), i32)> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let p = (x + dx, y + dy);
                    let d = dx * dx + dy * dy;
                    if self.walkable(p.0, p.1) && best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((p, d));
                    }
                }
            }
            if let Some((p, _)) = best {
                return Some(p);
            }
        }
        None
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
            let target = layout.start(
                config.start_tiles.get(area).map(String::as_str),
                &config.filler,
            )?;
            let grid_path =
                Path::new(&config.layout_dir).join(format!("terrain_grid_{tile_hash}.bin"));
            let grid = std::fs::read(&grid_path).ok().and_then(|d| Grid::parse(&d));
            let start = match &grid {
                Some(g) => g.nearest_walkable(target, 400).or(Some(target)),
                None => {
                    warn!(file = %grid_path.display(), "no cell grid; not snapping to walkable");
                    Some(target)
                }
            };
            info!(
                ?target,
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

    #[test]
    fn snaps_to_walkable() {
        // 12x8 cells, one layer; walkable block at x 4..12, y 0..8.
        let (cx, cy, bpr) = (12u32, 8u32, 6u32);
        let mut data = b"PWG1".to_vec();
        for v in [cx, cy, bpr, 1] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        for _ in 0..cy {
            data.extend_from_slice(&[0x00, 0x00, 0x55, 0x55, 0x55, 0x55]);
        }
        let g = Grid::parse(&data).unwrap();
        assert_eq!(g.cell(0, 4, 0), 5);
        assert_eq!(g.cell(0, 3, 0), 0);
        assert_eq!(g.nearest_walkable((0, 4), 10), Some((6, 4)));
    }
}
