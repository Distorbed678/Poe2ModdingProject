//! Tile matching: the key a tile or a room slot is matched by, its eight orientations, and the
//! tile-set index the generator looks keys up in (`Documentation/re/terrain.md`).
//!
//! A key (`0x141d09900` from a tile, `0x141d08a40` oriented, 0x4e bytes compared) holds the
//! edge type of each side with its exit and virtual-exit positions, the size, and the ground
//! type and height of each corner. Sides and corners are in file order (the `.tdt` header's and
//! the `.arm` slot's, which agree). The index (`0x141d09ab0`) lists, per feature name and key,
//! `(tile, orientation, weight)` in `.tst` order and orientation order; the generator picks from
//! a list with `below(total weight)`, so this order is part of the layout.

use std::collections::HashMap;

use crate::tdt::Tdt;

/// `I, R90, R180, R270, FI, FR90, FR180, FR270` (`0x142f08780`).
pub const ORIENTATION_NAMES: [&str; 8] =
    ["I", "R90", "R180", "R270", "FI", "FR90", "FR180", "FR270"];

/// Side permutation per orientation (`0x142f08808`): oriented side `i` is source side `T[o][i]`.
const SIDES: [[usize; 4]; 8] = [
    [0, 1, 2, 3],
    [3, 0, 1, 2],
    [2, 3, 0, 1],
    [1, 2, 3, 0],
    [0, 3, 2, 1],
    [1, 0, 3, 2],
    [2, 1, 0, 3],
    [3, 2, 1, 0],
];
/// Corner permutation per orientation (`0x142f08828`).
const CORNERS: [[usize; 4]; 8] = [
    [0, 1, 2, 3],
    [3, 0, 1, 2],
    [2, 3, 0, 1],
    [1, 2, 3, 0],
    [1, 0, 3, 2],
    [2, 1, 0, 3],
    [3, 2, 1, 0],
    [0, 3, 2, 1],
];

/// Cell transform per orientation (`0x142f020c8`): (transpose, x source, y source) where a
/// source 0 mirrors.
const TRANSFORM: [(bool, u8, u8); 8] = [
    (false, 1, 1),
    (true, 0, 1),
    (false, 0, 0),
    (true, 1, 0),
    (false, 0, 1),
    (true, 0, 0),
    (false, 1, 0),
    (true, 1, 1),
];

/// Where local cell `(x, y)` of a `w × h` piece lands with orientation `o` (relative to the
/// placed piece's corner).
pub fn orient_cell(o: u8, w: i32, h: i32, x: i32, y: i32) -> (i32, i32) {
    let (t, xs, ys) = TRANSFORM[(o & 7) as usize];
    let a = [w - 1 - x, x, h - 1 - y, y];
    if t {
        (a[2 + xs as usize], a[ys as usize])
    } else {
        (a[xs as usize], a[2 + ys as usize])
    }
}

/// Paths compare like the client's resource handles: case-insensitively.
fn norm(p: &Option<String>) -> Option<String> {
    p.as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase().replace('\\', "/"))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct TileKey {
    pub edges: [Option<String>; 4],
    pub size: (u8, u8),
    pub grounds: [Option<String>; 4],
    pub heights: [u8; 4],
    pub exits: [u8; 4],
    pub virtual_exits: [u8; 4],
}

impl TileKey {
    pub fn from_tdt(t: &Tdt) -> Self {
        let d = t.edge_distances;
        Self {
            edges: t.edges.clone().map(|e| norm(&e)),
            size: t.size,
            grounds: t.grounds.clone().map(|g| norm(&g)),
            heights: t.unk,
            exits: [d[0], d[1], d[2], d[3]],
            virtual_exits: [d[4], d[5], d[6], d[7]],
        }
    }

    /// The key seen with orientation `o` (`0x141d08a40`).
    pub fn oriented(&self, o: u8) -> Self {
        let o = (o & 7) as usize;
        let mut k = Self {
            size: if o & 1 == 1 {
                (self.size.1, self.size.0)
            } else {
                self.size
            },
            ..Default::default()
        };
        for i in 0..4 {
            let (g, e) = (CORNERS[o][i], SIDES[o][i]);
            k.grounds[i] = self.grounds[g].clone();
            k.heights[i] = self.heights[g];
            k.edges[i] = self.edges[e].clone();
            k.exits[i] = self.exits[e];
            k.virtual_exits[i] = self.virtual_exits[e];
        }
        if o >= 4 {
            for i in 0..4 {
                let full = if i % 2 == 0 { k.size.0 } else { k.size.1 }.wrapping_mul(3);
                if k.exits[i] != full {
                    let flipped = full.wrapping_sub(k.exits[i]).wrapping_sub(1);
                    k.exits[i] = flipped;
                    if k.virtual_exits[i] != full {
                        k.exits[i] = full.wrapping_sub(k.virtual_exits[i]).wrapping_sub(1);
                        k.virtual_exits[i] = flipped;
                    }
                }
            }
        }
        k
    }
}

/// One `.tst` entry with its parsed header.
#[derive(Debug, Clone)]
pub struct TileDef {
    pub path: String,
    pub tdt: Tdt,
    pub weight: u32,
    /// Allowed orientations (bit per orientation).
    pub orientations: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub tile: usize,
    pub orientation: u8,
    pub weight: u32,
}

#[derive(Debug, Default)]
pub struct TileSet {
    pub tiles: Vec<TileDef>,
    index: HashMap<(String, TileKey), Vec<Candidate>>,
}

/// Orientation mask of a `.tst` rotation list (empty: all eight).
pub fn orientation_mask(rotations: &[String]) -> u8 {
    if rotations.is_empty() {
        return 0xff;
    }
    rotations.iter().fold(0u8, |m, r| {
        ORIENTATION_NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(r))
            .map_or(m, |o| m | 1 << o)
    })
}

impl TileSet {
    /// Builds the index from tiles in `.tst` order.
    pub fn new(tiles: Vec<TileDef>) -> Self {
        let mut index: HashMap<(String, TileKey), Vec<Candidate>> = HashMap::new();
        for (i, t) in tiles.iter().enumerate() {
            if t.weight == 0 {
                continue;
            }
            let feature = t
                .tdt
                .feature
                .clone()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let key = TileKey::from_tdt(&t.tdt);
            for o in 0..8u8 {
                if t.orientations & (1 << o) == 0 {
                    continue;
                }
                index
                    .entry((feature.clone(), key.oriented(o)))
                    .or_default()
                    .push(Candidate {
                        tile: i,
                        orientation: o,
                        weight: t.weight,
                    });
            }
        }
        Self { tiles, index }
    }

    pub fn candidates(&self, feature: &str, key: &TileKey) -> &[Candidate] {
        self.index
            .get(&(feature.to_ascii_lowercase(), key.clone()))
            .map_or(&[], Vec::as_slice)
    }
}

/// The weighted pick the generator does (`0x1415d5960`): `r` in `0..total`.
pub fn pick(list: &[Candidate], mut r: u32) -> Option<Candidate> {
    for c in list {
        if r < c.weight {
            return Some(*c);
        }
        r -= c.weight;
    }
    list.last().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientations_round_trip() {
        // A 3×2 piece: identity keeps cells, R180 maps the corners onto each other.
        assert_eq!(orient_cell(0, 3, 2, 0, 0), (0, 0));
        assert_eq!(orient_cell(2, 3, 2, 0, 0), (2, 1));
        // Odd orientations transpose: the result fits a 2×3 box.
        for o in [1u8, 3, 5, 7] {
            let (x, y) = orient_cell(o, 3, 2, 2, 1);
            assert!(x < 2 && y < 3, "o={o}: ({x},{y})");
        }
    }

    #[test]
    fn masks() {
        assert_eq!(orientation_mask(&[]), 0xff);
        assert_eq!(orientation_mask(&["FI".into(), "R270".into()]), 0b0001_1000);
    }
}
