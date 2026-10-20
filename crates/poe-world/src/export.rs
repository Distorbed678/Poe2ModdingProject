//! The client DLL's area exports (`<session>/areas/<n>_<hash16>_<seed>/`, see
//! `docs/re/terrain/README.md`): the reference the server's generator is compared with.

use std::path::Path;

use crate::hash::{self, HashedTile};

/// One tile record of `layout.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedTile {
    pub x: u32,
    pub y: u32,
    pub path: String,
    /// Index of the sub-tile within its (multi-tile) tile, in the tile's own orientation.
    pub sub: u32,
    pub height: i16,
    pub orientation: u8,
    /// Not hashed when either is set.
    pub skip: (u8, u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AreaExport {
    pub meta: Vec<(String, String)>,
    pub width: u32,
    pub height: u32,
    /// Row-major.
    pub tiles: Vec<ExportedTile>,
    /// (x, y, angle, path).
    pub doodads: Vec<(i32, i32, f32, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("{0} line {1}: {2}")]
    Format(&'static str, usize, &'static str),
}

impl AreaExport {
    pub fn load(dir: &Path) -> Result<Self, ExportError> {
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name))
                .map_err(|e| ExportError::Io(dir.join(name).display().to_string(), e))
        };
        let meta: Vec<(String, String)> = read("meta.txt")?
            .lines()
            .filter_map(|l| {
                let (k, v) = l.split_once('\t')?;
                Some((k.to_string(), v.to_string()))
            })
            .collect();
        let get = |k: &str| meta.iter().find(|(m, _)| m == k).map(|(_, v)| v.clone());
        let size = get("size_tiles").unwrap_or_default();
        let mut size = size.split('\t').map(|v| v.parse::<u32>().unwrap_or(0));
        let (width, height) = (size.next().unwrap_or(0), size.next().unwrap_or(0));
        let mut tiles = Vec::new();
        for (i, line) in read("layout.tsv")?.lines().enumerate() {
            if line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            let bad = |what| ExportError::Format("layout.tsv", i + 1, what);
            if f.len() < 7 {
                return Err(bad("too few columns"));
            }
            let num = |s: &str| s.parse::<i64>().map_err(|_| bad("bad number"));
            let (a, b) = f[6].split_once(',').ok_or_else(|| bad("bad skip"))?;
            tiles.push(ExportedTile {
                x: num(f[0])? as u32,
                y: num(f[1])? as u32,
                path: f[2].to_string(),
                sub: num(f[3])? as u32,
                height: num(f[4])? as i16,
                orientation: num(f[5])? as u8,
                skip: (num(a)? as u8, num(b)? as u8),
            });
        }
        // The export lists columns (x outer); keep row-major.
        tiles.sort_by_key(|t| (t.y, t.x));
        let mut doodads = Vec::new();
        if let Ok(text) = read("doodads.tsv") {
            for line in text.lines().filter(|l| !l.starts_with('#')) {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() >= 5 {
                    doodads.push((
                        f[0].parse().unwrap_or(0),
                        f[1].parse().unwrap_or(0),
                        f[2].parse().unwrap_or(0.0),
                        f[4].to_string(),
                    ));
                }
            }
        }
        Ok(Self {
            meta,
            width,
            height,
            tiles,
            doodads,
        })
    }

    pub fn meta(&self, key: &str) -> Option<&str> {
        self.meta
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn tile(&self, x: u32, y: u32) -> Option<&ExportedTile> {
        self.tiles.get((y * self.width + x) as usize)
    }

    /// The tile hash of this layout (the client's order and rules).
    pub fn tile_hash(&self) -> u32 {
        let mut order: Vec<&ExportedTile> = self
            .tiles
            .iter()
            .filter(|t| !t.path.is_empty() && t.skip == (0, 0))
            .collect();
        order.sort_by_key(|t| (t.x, t.y));
        hash::tile_hash(order.into_iter().map(|t| HashedTile {
            x: t.x,
            y: t.y,
            path: &t.path,
            height: t.height,
            orientation: t.orientation,
        }))
    }

    pub fn doodad_hash(&self) -> u32 {
        hash::doodad_hash(self.doodads.iter().map(|(x, y, _, p)| (*x, *y, p.as_str())))
    }
}
