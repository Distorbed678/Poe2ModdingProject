//! Player start position.
//!
//! The server's own generator (`poe-world`) does not cover every area yet and is not used for
//! spawns, so the server learns where an area's start is from the client: the DLL exports the
//! layout it generated
//! (`<layout_dir>/layouts/terrain_layout_<tile hash>.tsv`, matched with TerrainHashes: a
//! `size<TAB>w<TAB>h` line, then `x<TAB>y<TAB>tile path` per tile; a remote client uploads it
//! instead, see `terrain_store`) and the server spawns the
//! player at the configured point of the area's start tile (mapped through the tile's
//! rotation/mirroring in this layout, from the sub-tile indices the DLL exports), or else near
//! the playable tile nearest the map centre. Grid cells per (sub-)tile: 23.
//!
//! The point is then snapped to the nearest walkable cell, using the client's cell grids
//! (`<layout_dir>/grids/terrain_grid_<tile hash>.bin`, layer 0 non-zero = walkable). Standing on a
//! non-walkable cell inside the playable area leaves the view grey.

use std::path::Path;

use poe_config::{SpawnConfig, StartTile};
use poe_game::movement::Walkable;
use tracing::{info, warn};

use crate::terrain_store::TerrainStore;

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

/// The client's cell grid (exported by the DLL); lives in `poe-game` so the simulation can use it.
pub use poe_game::grid::Grid;

/// What the server knows about the client's terrain: the walkability grid, if exported.
pub fn client_grid(config: &SpawnConfig, tile_hash: u32) -> Option<Grid> {
    if config.layout_dir.is_empty() {
        return None;
    }
    let path = Path::new(&config.layout_dir)
        .join("grids")
        .join(format!("terrain_grid_{tile_hash}.bin"));
    std::fs::read(path).ok().and_then(|d| Grid::parse(&d))
}

/// Where a player joining an instance stands: `wanted` when no player is within `spacing`
/// cells of it, else the walkable cell nearest to `wanted` that is at least `spacing` cells from
/// every occupied cell. Deterministic: square rings of growing radius around `wanted`, ties
/// broken by (y, x). Nothing found within the search radius (no free walkable cell nearby)
/// leaves `wanted`. `spacing <= 0` turns spreading off.
///
/// UNVERIFIED: only the walkability grid is used, so the chosen cell can lie behind a wall from
/// the first player; the spacing is small (a few cells) so that is rare on open ground.
pub fn spread(
    grid: &dyn Walkable,
    wanted: (i32, i32),
    occupied: &[(i32, i32)],
    spacing: i32,
) -> (i32, i32) {
    if spacing <= 0 || occupied.is_empty() {
        return wanted;
    }
    let limit = i64::from(spacing) * i64::from(spacing);
    let free = |(x, y): (i32, i32)| {
        grid.walkable(x, y)
            && occupied.iter().all(|&(ox, oy)| {
                let (dx, dy) = (i64::from(x - ox), i64::from(y - oy));
                dx * dx + dy * dy >= limit
            })
    };
    if free(wanted) {
        return wanted;
    }
    let reach = spacing.saturating_mul(2 + occupied.len().min(64) as i32);
    let mut best: Option<(i64, i32, i32)> = None;
    for r in 1..=reach {
        // A ring's nearest cells are r away: once the best is closer, no later ring beats it.
        if best.is_some_and(|(d, _, _)| i64::from(r) * i64::from(r) > d) {
            break;
        }
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                let p = (wanted.0 + dx, wanted.1 + dy);
                if !free(p) {
                    continue;
                }
                let d = i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy);
                let key = (d, p.1, p.0);
                if best.is_none_or(|b| key < b) {
                    best = Some(key);
                }
            }
        }
    }
    best.map_or(wanted, |(_, y, x)| (x, y))
}

/// Waits for the layout of this terrain (by tile hash) in the store (written by a local DLL
/// or uploaded by a remote one, [`crate::terrain_store`]) and picks the start position.
pub async fn from_client_layout(
    store: &TerrainStore,
    config: &SpawnConfig,
    area: &str,
    tile_hash: u32,
    session: u64,
) -> Option<(i32, i32)> {
    let path = store.layout_path(tile_hash)?;
    let Some(text) = store.wait_layout(session, tile_hash).await else {
        warn!(file = %path.display(), "no client layout; spawning at the origin");
        return None;
    };
    let layout = Layout::parse(&text)?;
    let target = layout.start(config.start_tiles.get(area), &config.filler)?;
    let grid_path = store.grid_path(tile_hash)?;
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
    start
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
    /// canonical indices run bottom-up (vertical mirror), see docs/architecture/client-dll.md (layout export format).
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

    /// Walkable except for the cells `blocked` says no to.
    struct Floor<F: Fn(i32, i32) -> bool>(F);

    impl<F: Fn(i32, i32) -> bool> Walkable for Floor<F> {
        fn walkable(&self, x: i32, y: i32) -> bool {
            (self.0)(x, y)
        }
    }

    fn dist2(a: (i32, i32), b: (i32, i32)) -> i32 {
        (a.0 - b.0).pow(2) + (a.1 - b.1).pow(2)
    }

    #[test]
    fn spreading_leaves_a_free_spawn_alone() {
        let open = Floor(|_, _| true);
        assert_eq!(spread(&open, (50, 50), &[], 8), (50, 50));
        // Far enough from the other player, or spreading off.
        assert_eq!(spread(&open, (50, 50), &[(50, 58)], 8), (50, 50));
        assert_eq!(spread(&open, (50, 50), &[(50, 50)], 0), (50, 50));
    }

    #[test]
    fn a_second_player_gets_the_nearest_free_cell() {
        let open = Floor(|_, _| true);
        let cell = spread(&open, (50, 50), &[(50, 50)], 8);
        assert_ne!(cell, (50, 50));
        assert!(dist2(cell, (50, 50)) >= 64, "{cell:?}");
        // The nearest such cell: exactly 8 away on an axis (ties go to the smaller y, then x).
        assert_eq!(cell, (50, 42));
        // The same input always gives the same cell.
        assert_eq!(cell, spread(&open, (50, 50), &[(50, 50)], 8));
        // A third player keeps clear of both.
        let third = spread(&open, (50, 50), &[(50, 50), cell], 8);
        assert!(
            dist2(third, (50, 50)) >= 64 && dist2(third, cell) >= 64,
            "{third:?}"
        );
        assert_ne!(third, cell);
    }

    #[test]
    fn spreading_only_picks_walkable_cells() {
        // A strip one cell wide along y = 50 (a corridor): the only free cells are on it.
        let corridor = Floor(|_, y| y == 50);
        let cell = spread(&corridor, (50, 50), &[(50, 50)], 8);
        assert_eq!(cell, (42, 50));
        // A wall on the left side pushes the spot to the right.
        let wall = Floor(|x, _| x >= 50);
        let cell = spread(&wall, (50, 50), &[(50, 50)], 8);
        assert!(cell.0 >= 50 && dist2(cell, (50, 50)) >= 64, "{cell:?}");
        // Nothing walkable nearby: the wanted cell stays.
        let nowhere = Floor(|x, y| (x, y) == (50, 50));
        assert_eq!(spread(&nowhere, (50, 50), &[(50, 50)], 8), (50, 50));
    }
}
