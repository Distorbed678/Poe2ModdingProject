//! Player start position.
//!
//! The server does not generate terrain yet (M4), so it cannot know where an area's start is.
//! Dev bridge: the client DLL exports the layout it generated
//! (`<layout_dir>/terrain_layout_<tile hash>.tsv`, matched with TerrainHashes: a
//! `size<TAB>w<TAB>h` line, then `x<TAB>y<TAB>tile path` per tile) and the server spawns the
//! player at the configured point of the area's start tile (mapped through the tile's
//! rotation/mirroring in this layout, from the sub-tile indices the DLL exports), or else near
//! the playable tile nearest the map centre. Grid cells per (sub-)tile: 23.
//!
//! The point is then snapped to the nearest walkable cell, using the client's cell grids
//! (`terrain_grid_<tile hash>.bin`, layer 0 non-zero = walkable). Standing on a
//! non-walkable cell inside the playable area leaves the view grey.

use std::{path::Path, time::Duration};

use poe_config::{SpawnConfig, StartTile};
use tracing::{info, warn};

pub const GRID_PER_TILE: i32 = 23;

/// One sub-tile of the layout (23 x 23 cells).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    /// `.tdt` path of the tile this sub-tile belongs to.
    pub path: String,
    /// Index of this sub-tile in the tile's own orientation (row-major), from the DLL export.
    pub sub: u32,
}

pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub tiles: Vec<Tile>,
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
                let mut f = l.split('\t');
                Some(Tile {
                    x: f.next()?.parse().ok()?,
                    y: f.next()?.parse().ok()?,
                    path: f.next()?.to_owned(),
                    sub: f.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                })
            })
            .collect();
        Some(Self {
            width,
            height,
            tiles,
        })
    }

    /// Grid position: the configured point inside the area's start tile (or the tile's
    /// centre), else the centre of the playable tile nearest the map centre.
    pub fn start(&self, start: Option<&StartTile>, filler: &[String]) -> Option<(i32, i32)> {
        if let Some(start) = start {
            let hits: Vec<&Tile> = self
                .tiles
                .iter()
                .filter(|t| t.path.contains(&start.tile))
                .collect();
            if !hits.is_empty() {
                let placed = PlacedTile::fit(&hits);
                let point = match (&placed, start.point) {
                    (Some(p), Some([x, y])) => Some(p.to_map(x, y)),
                    (None, Some(_)) => {
                        warn!(tile = %start.tile, "start tile orientation not found; using its centre");
                        None
                    }
                    _ => None,
                };
                return Some(point.unwrap_or_else(|| {
                    let n = hits.len() as i32;
                    let centre = |sum: i32| sum * GRID_PER_TILE / n + GRID_PER_TILE / 2;
                    (
                        centre(hits.iter().map(|t| t.x).sum()),
                        centre(hits.iter().map(|t| t.y).sum()),
                    )
                }));
            }
        }
        let (cx, cy) = (self.width / 2, self.height / 2);
        let t = self
            .tiles
            .iter()
            .filter(|t| !t.path.is_empty() && !filler.iter().any(|f| t.path.contains(f.as_str())))
            .min_by_key(|t| (t.x - cx).pow(2) + (t.y - cy).pow(2))?;
        let half = GRID_PER_TILE / 2;
        Some((t.x * GRID_PER_TILE + half, t.y * GRID_PER_TILE + half))
    }
}

/// How a multi-sub-tile tile was placed: its corner in sub-tiles, its size on the map, and the
/// rotation/mirroring (one of the 8 that map its sub-tiles' map positions to their canonical
/// indices).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedTile {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub transform: u8,
}

impl PlacedTile {
    /// Map-local (u, v) -> canonical (a, b) for transform `k` (continuous coordinates, map size
    /// `w` x `h`); transforms 4..7 swap the axes.
    fn forward(k: u8, (w, h): (f64, f64), (u, v): (f64, f64)) -> (f64, f64) {
        match k {
            0 => (u, v),
            1 => (w - u, v),
            2 => (u, h - v),
            3 => (w - u, h - v),
            4 => (v, u),
            5 => (h - v, u),
            6 => (v, w - u),
            _ => (h - v, w - u),
        }
    }

    /// Canonical (a, b) -> map-local (u, v).
    fn inverse(k: u8, (w, h): (f64, f64), (a, b): (f64, f64)) -> (f64, f64) {
        match k {
            0 => (a, b),
            1 => (w - a, b),
            2 => (a, h - b),
            3 => (w - a, h - b),
            4 => (b, a),
            5 => (b, h - a),
            6 => (w - b, a),
            _ => (w - b, h - a),
        }
    }

    /// Finds the placement from the sub-tiles of one tile occurrence.
    pub fn fit(tiles: &[&Tile]) -> Option<Self> {
        let x = tiles.iter().map(|t| t.x).min()?;
        let y = tiles.iter().map(|t| t.y).min()?;
        let width = tiles.iter().map(|t| t.x).max()? - x + 1;
        let height = tiles.iter().map(|t| t.y).max()? - y + 1;
        let size = (width as f64, height as f64);
        (0..8u8)
            .find(|&k| {
                let canonical_width = if k < 4 { width } else { height };
                tiles.iter().all(|t| {
                    let centre = ((t.x - x) as f64 + 0.5, (t.y - y) as f64 + 0.5);
                    let (a, b) = Self::forward(k, size, centre);
                    (b as i32) * canonical_width + a as i32 == t.sub as i32
                })
            })
            .map(|transform| Self {
                x,
                y,
                width,
                height,
                transform,
            })
    }

    /// A point given in the tile's own orientation (grid cells from its corner) on the map.
    pub fn to_map(&self, x: i32, y: i32) -> (i32, i32) {
        let g = GRID_PER_TILE as f64;
        let (u, v) = Self::inverse(
            self.transform,
            (self.width as f64, self.height as f64),
            (x as f64 / g, y as f64 / g),
        );
        (
            ((self.x as f64 + u) * g).round() as i32,
            ((self.y as f64 + v) * g).round() as i32,
        )
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

/// Movement uses layer 0 of the client's grid directly (non-zero = walkable).
impl poe_game::movement::Walkable for Grid {
    fn walkable(&self, x: i32, y: i32) -> bool {
        self.cell(0, x, y) != 0
    }
}

/// What the server knows about the client's terrain: the walkability grid, if exported.
pub fn client_grid(config: &SpawnConfig, tile_hash: u32) -> Option<Grid> {
    if config.layout_dir.is_empty() {
        return None;
    }
    let path = Path::new(&config.layout_dir).join(format!("terrain_grid_{tile_hash}.bin"));
    std::fs::read(path).ok().and_then(|d| Grid::parse(&d))
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
            let target = layout.start(config.start_tiles.get(area), &config.filler)?;
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
        let text = "size\t4\t4\n0\t0\tMetadata/a/fill.tdt\t0\n1\t1\tMetadata/a/Intro_Tree_01.tdt\t0\n2\t1\tMetadata/a/Intro_Tree_01.tdt\t1\n2\t2\tMetadata/a/ground.tdt\t0\n3\t3\tMetadata/a/blank.tdt\t0\n";
        let layout = Layout::parse(text).unwrap();
        let filler = vec!["fill".to_owned(), "blank".to_owned()];
        let tree = StartTile {
            tile: "Intro_Tree".into(),
            point: None,
        };
        // Centre of tiles (1,1) and (2,1).
        assert_eq!(layout.start(Some(&tree), &filler), Some((45, 34)));
        assert_eq!(layout.start(None, &filler), Some((57, 57)));
        // A point in the tile's own orientation; identity placement here.
        let tree = StartTile {
            point: Some([30, 5]),
            ..tree
        };
        assert_eq!(layout.start(Some(&tree), &filler), Some((53, 28)));
    }

    /// The Riverbank intro tree (6 x 6 sub-tiles) as placed for seed 3357350429: rows of
    /// canonical indices run bottom-up (vertical mirror), see Documentation.
    #[test]
    fn fits_mirrored_tile() {
        let tiles: Vec<Tile> = (0..6)
            .flat_map(|v| {
                (0..6).map(move |u| Tile {
                    x: 12 + u,
                    y: 45 + v,
                    path: "Intro_Tree_01.tdt".into(),
                    sub: ((5 - v) * 6 + u) as u32,
                })
            })
            .collect();
        let refs: Vec<&Tile> = tiles.iter().collect();
        let p = PlacedTile::fit(&refs).unwrap();
        assert_eq!(p.transform, 2);
        assert_eq!(p.to_map(9, 33), (285, 1140));
        // A rotated placement (90 degrees: canonical (a, b) at map (5 - b, a)).
        let rotated: Vec<Tile> = (0..6)
            .flat_map(|v| {
                (0..6).map(move |u| Tile {
                    x: u,
                    y: v,
                    path: String::new(),
                    sub: ((5 - u) * 6 + v) as u32,
                })
            })
            .collect();
        let refs: Vec<&Tile> = rotated.iter().collect();
        let p = PlacedTile::fit(&refs).unwrap();
        let (a, b) = (9.0 / 23.0, 33.0 / 23.0);
        let expect = (
            ((6.0 - b) * 23.0_f64).round() as i32,
            (a * 23.0_f64).round() as i32,
        );
        assert_eq!(p.to_map(9, 33), expect);
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
