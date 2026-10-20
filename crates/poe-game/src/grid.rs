//! Walkability grids the simulation runs on.
//!
//! [`GridSource`] is what the [`crate::world::World`] needs from the terrain: walkability per
//! cell ([`Walkable`]) and the size. Implementations:
//! - [`Grid`]: the client's cell grid, exported by the debug DLL to
//!   `terrain_grid_<tile hash>.bin` (the dev bridge until `poe-world` generates collision),
//! - [`VecGrid`]: a plain boolean grid (tests, tools),
//! - [`Open`](crate::movement::Open): no terrain, everything walkable.
//!
//! `poe-world` will implement the trait for its generated collision output.

use crate::movement::{Open, Walkable};

/// The terrain as the simulation sees it.
pub trait GridSource: Walkable + Send + Sync {
    /// Size in cells `(width, height)`; cells outside are not walkable. `(i32::MAX, i32::MAX)`
    /// for an unbounded grid.
    fn size(&self) -> (i32, i32);

    /// Nearest walkable cell to `cell` within `radius` cells (square rings outward, ties by
    /// distance), for spawning on or after a blocked cell.
    fn nearest_walkable(&self, (x, y): (i32, i32), radius: i32) -> Option<(i32, i32)> {
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

impl GridSource for Open {
    fn size(&self) -> (i32, i32) {
        (i32::MAX, i32::MAX)
    }
}

/// A boolean grid, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VecGrid {
    width: i32,
    height: i32,
    cells: Vec<bool>,
}

impl VecGrid {
    /// A grid of `width * height` cells, all walkable.
    pub fn open(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            cells: vec![true; (width * height) as usize],
        }
    }

    /// From rows of text: `#` is blocked, anything else walkable. Rows must have equal length.
    pub fn from_rows(rows: &[&str]) -> Self {
        let width = rows.first().map_or(0, |r| r.chars().count()) as i32;
        let cells = rows
            .iter()
            .flat_map(|r| {
                assert_eq!(r.chars().count() as i32, width, "ragged rows");
                r.chars().map(|c| c != '#')
            })
            .collect();
        Self {
            width,
            height: rows.len() as i32,
            cells,
        }
    }

    pub fn set(&mut self, x: i32, y: i32, walkable: bool) {
        if x >= 0 && y >= 0 && x < self.width && y < self.height {
            self.cells[(y * self.width + x) as usize] = walkable;
        }
    }
}

impl Walkable for VecGrid {
    fn walkable(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < self.width
            && y < self.height
            && self.cells[(y * self.width + x) as usize]
    }
}

impl GridSource for VecGrid {
    fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }
}

/// Cell grids exported by the DLL: `b"PWG1"`, u32 cells_x, cells_y, bytes_per_row, layers,
/// then each layer (4 bits per cell, low nibble first). Layer 0 non-zero = walkable.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// Roomy: layer 0 non-zero on the cell and its 8 neighbours at distance 2. Used to place
    /// spawns away from walls (movement itself uses [`Walkable::walkable`], the cell alone).
    pub fn roomy(&self, x: i32, y: i32) -> bool {
        (-2..=2).step_by(2).all(|dy| {
            (-2..=2)
                .step_by(2)
                .all(|dx| self.cell(0, x + dx, y + dy) != 0)
        })
    }

    /// Nearest [roomy](Self::roomy) cell to `(x, y)` within `radius` cells.
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
                    if self.roomy(p.0, p.1) && best.is_none_or(|(_, bd)| d < bd) {
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
impl Walkable for Grid {
    fn walkable(&self, x: i32, y: i32) -> bool {
        self.cell(0, x, y) != 0
    }

    fn clearance(&self, x: i32, y: i32) -> u8 {
        self.cell(0, x, y)
    }
}

impl GridSource for Grid {
    fn size(&self) -> (i32, i32) {
        (self.cells_x, self.cells_y)
    }

    fn nearest_walkable(&self, cell: (i32, i32), radius: i32) -> Option<(i32, i32)> {
        Grid::nearest_walkable(self, cell, radius)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 12x8 cells, one layer; walkable block at x 4..12, y 0..8.
    fn sample() -> Grid {
        let (cx, cy, bpr) = (12u32, 8u32, 6u32);
        let mut data = b"PWG1".to_vec();
        for v in [cx, cy, bpr, 1] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        for _ in 0..cy {
            data.extend_from_slice(&[0x00, 0x00, 0x55, 0x55, 0x55, 0x55]);
        }
        Grid::parse(&data).unwrap()
    }

    #[test]
    fn parses_the_client_dump() {
        let g = sample();
        assert_eq!(g.size(), (12, 8));
        assert_eq!(g.cell(0, 4, 0), 5);
        assert_eq!(g.cell(0, 3, 0), 0);
        assert!(Walkable::walkable(&g, 4, 0));
        assert!(!Walkable::walkable(&g, 3, 0));
        assert!(!Walkable::walkable(&g, 99, 0));
        assert_eq!(Grid::parse(b"nope"), None);
    }

    #[test]
    fn snaps_to_a_roomy_cell() {
        assert_eq!(sample().nearest_walkable((0, 4), 10), Some((6, 4)));
    }

    #[test]
    fn default_nearest_walkable_uses_the_cell_alone() {
        let g = VecGrid::from_rows(&["##...", "##...", "##..."]);
        assert_eq!(GridSource::nearest_walkable(&g, (0, 1), 5), Some((2, 1)));
        assert_eq!(GridSource::nearest_walkable(&g, (0, 1), 1), None);
        assert_eq!(g.size(), (5, 3));
    }
}
