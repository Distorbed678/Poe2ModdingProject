//! The terrain generator, ported pass by pass (`Documentation/re/terrain.md`, "Port notes").
//! Each pass is checked against the client's generator trace before the next is added.
//!
//! Supported so far: a graph with one room node covering the map (towns): topology sub-seed,
//! room placement draw, room pass, edge cost grid, fill pass, de-repetition. Clearfell
//! Encampment (`G1_town`) comes out identical to the client for every seed tried, tile hash
//! included (`cargo run -p poe-world --example town_gen -- <client export> <seed>`).

use std::collections::HashMap;

use poe_data_tools::file_parsers::{
    FileParser,
    arm::{
        ARMParser,
        types::{ARMFile, Slot},
    },
    gft::GFTParser,
    gt::GTParser,
    tst::TSTParser,
};

use crate::{
    rng::TerrainRng,
    tdt::Tdt,
    tileset::{TileDef, TileKey, TileSet, orient_cell, orientation_mask, pick},
};

#[derive(Debug, thiserror::Error)]
pub enum GenError {
    #[error("{0}: {1}")]
    Data(String, String),
    #[error("{0}")]
    Unsupported(String),
}

fn data_err(path: &str, e: impl std::fmt::Debug) -> GenError {
    GenError::Data(path.to_string(), format!("{e:?}"))
}

/// Reads game files (the generator's only input besides the seed).
pub trait Files {
    fn read(&self, path: &str) -> Result<Vec<u8>, GenError>;
}

impl Files for poe_data::GameFiles {
    fn read(&self, path: &str) -> Result<Vec<u8>, GenError> {
        poe_data::GameFiles::read(self, &path.to_lowercase())
            .map(|b| b.to_vec())
            .map_err(|e| data_err(path, e))
    }
}

/// A tile on the map: which `.tdt`, where its corner is, its orientation and which of its
/// sub-tiles covers this cell (row-major in the tile's own orientation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub path: String,
    pub orientation: u8,
    pub sub: u32,
    pub height: i16,
    /// The cell's offset from the tile's placed corner (record +0x34/+0x35); the tile hash
    /// only covers the corner cell, (0, 0).
    pub offset: (u8, u8),
}

/// A placed tile: corner, size (in its own orientation), orientation and, for tile-set tiles,
/// its index in the tile set (the de-repetition pass re-picks those).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub x: i32,
    pub y: i32,
    pub path: String,
    pub size: (u8, u8),
    pub orientation: u8,
    pub tile: Option<usize>,
}

/// The tile map being generated (one entry per 23×23-cell tile).
#[derive(Debug, Clone)]
pub struct TileMap {
    pub width: u32,
    pub height: u32,
    pub cells: Vec<Option<Cell>>,
    /// Ground type marked on a cell (`.gt` path) for the fill pass.
    pub grounds: Vec<Option<String>>,
    pub pieces: Vec<Piece>,
    /// Piece covering each cell.
    pub piece_at: Vec<Option<usize>>,
    /// Tile corner points, `(width + 1) × (height + 1)` (map +0x50, 3 bytes each).
    pub lattice: Vec<LatticePoint>,
    /// `.gt` flags by ground path (lower case), for grounds the lattice meets; missing grounds
    /// count as the default `0x0f`.
    pub ground_flags: HashMap<String, u8>,
    /// `.et` flags by edge path (lower case): 0x80 wildcard, 4 virtual.
    pub edge_flags: HashMap<String, u8>,
    /// Cells and corners tiles wrote outside the map (a tile may overhang the region by one
    /// cell): the client's region is a window into the full map, so `embed` keeps them.
    spill: Vec<Spill>,
}

#[derive(Debug, Clone)]
enum Spill {
    Cell(i32, i32, usize, Cell),
    Point(i32, i32, Option<String>, i8),
}

/// One tile corner (map +0x50 records: ground index, height, flags).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LatticePoint {
    /// Ground type (`.gt` path, lower case).
    pub ground: Option<String>,
    pub height: i8,
    /// Bit 0: set by a tile or a flood; bit 1: height set; bit 2: set before the post-tile
    /// passes; bit 3: set by `0x1420c0a60`.
    pub flags: u8,
}

impl TileMap {
    pub fn new(width: u32, height: u32) -> Self {
        let n = (width * height) as usize;
        Self {
            width,
            height,
            cells: vec![None; n],
            grounds: vec![None; n],
            pieces: Vec::new(),
            piece_at: vec![None; n],
            lattice: vec![LatticePoint::default(); ((width + 1) * (height + 1)) as usize],
            ground_flags: HashMap::new(),
            edge_flags: HashMap::new(),
            spill: Vec::new(),
        }
    }

    /// This map placed at `offset` inside an empty `width × height` map (cells, pieces and
    /// corners move; the outer buffer pass then fills the rest).
    pub fn embed(&self, offset: (i32, i32), width: u32, height: u32) -> TileMap {
        let mut m = TileMap::new(width, height);
        m.ground_flags = self.ground_flags.clone();
        m.edge_flags = self.edge_flags.clone();
        m.pieces = self
            .pieces
            .iter()
            .map(|p| Piece { x: p.x + offset.0, y: p.y + offset.1, ..p.clone() })
            .collect();
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                let i = self.index(x, y).unwrap();
                if let Some(j) = m.index(x + offset.0, y + offset.1) {
                    m.cells[j] = self.cells[i].clone();
                    m.grounds[j] = self.grounds[i].clone();
                    m.piece_at[j] = self.piece_at[i];
                }
            }
        }
        for y in 0..=self.height as i32 {
            for x in 0..=self.width as i32 {
                let i = self.point_index(x, y).unwrap();
                if let Some(j) = m.point_index(x + offset.0, y + offset.1) {
                    m.lattice[j] = self.lattice[i].clone();
                }
            }
        }
        for s in &self.spill {
            match s {
                Spill::Cell(x, y, piece, cell) => {
                    if let Some(j) = m.index(x + offset.0, y + offset.1) {
                        m.cells[j] = Some(cell.clone());
                        m.piece_at[j] = Some(*piece);
                    }
                }
                Spill::Point(x, y, g, h) => {
                    if let Some(j) = m.point_index(x + offset.0, y + offset.1) {
                        let p = &mut m.lattice[j];
                        p.flags |= 3;
                        if g.is_some() {
                            p.ground = g.clone();
                        }
                        p.height = *h;
                    }
                }
            }
        }
        m
    }

    /// Lattice index of corner `(x, y)` (0..=width, 0..=height).
    pub fn point_index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x as u32 <= self.width && y as u32 <= self.height)
            .then(|| (y as u32 * (self.width + 1) + x as u32) as usize)
    }

    /// `0x141d0e130` with a ground: the corner takes it and is set.
    pub fn set_ground(&mut self, x: i32, y: i32, g: &str) {
        if let Some(i) = self.point_index(x, y) {
            self.lattice[i].ground = Some(g.to_ascii_lowercase());
            self.lattice[i].flags |= 1;
        }
    }

    /// `0x141f9e780`: a room's ground overrides on its inside corners.
    pub fn room_overrides(&mut self, room: &Room, origin: (i32, i32), o: u8) {
        let (w, h) = (room.width() as i32, room.height() as i32);
        for v in 1..h {
            for u in 1..w {
                let Some(&k) = room.lattice.get(((v - 1) * (w - 1) + (u - 1)) as usize) else { continue };
                let Some(g) = (k > 0).then(|| room.arm.strings.get(k as usize - 1)).flatten() else { continue };
                let (dx, dy) = crate::tileset::orient_point(o, w, h, u, v);
                let g = g.clone();
                self.set_ground(origin.0 + dx, origin.1 + dy, &g);
            }
        }
    }

    /// `.gt` flags of a ground (`+0x24`).
    pub fn gt_flags(&self, ground: &str) -> u8 {
        self.ground_flags.get(ground).copied().unwrap_or(0x0f)
    }

    pub fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height)
            .then(|| (y as u32 * self.width + x as u32) as usize)
    }

    pub fn get(&self, x: u32, y: u32) -> Option<&Cell> {
        self.cells.get((y * self.width + x) as usize)?.as_ref()
    }

    /// Size of a tile with orientation `o` (odd orientations swap).
    pub fn oriented_size(size: (u8, u8), o: u8) -> (i32, i32) {
        if o & 1 == 1 && o < 8 {
            (size.1 as i32, size.0 as i32)
        } else {
            (size.0 as i32, size.1 as i32)
        }
    }

    /// Takes away the piece covering a cell (`0x141d0c480`).
    pub fn remove_piece_at(&mut self, x: i32, y: i32) {
        let Some(i) = self.index(x, y) else { return };
        let Some(p) = self.piece_at[i] else {
            self.cells[i] = None;
            return;
        };
        for j in 0..self.cells.len() {
            if self.piece_at[j] == Some(p) {
                self.piece_at[j] = None;
                self.cells[j] = None;
            }
        }
    }

    /// Whether every cell of a `w`×`h` rectangle is inside the map and empty (`0x141d0d880`).
    pub fn free(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        (0..h).all(|dy| (0..w).all(|dx| self.index(x + dx, y + dy).is_some_and(|i| self.cells[i].is_none())))
    }

    /// Whether a tile fits at `(x, y)`: inside the map, every covered cell empty.
    fn fits(&self, x: i32, y: i32, size: (u8, u8), o: u8) -> bool {
        let (w, h) = Self::oriented_size(size, o);
        (0..h).all(|dy| {
            (0..w).all(|dx| {
                self.index(x + dx, y + dy)
                    .is_some_and(|i| self.cells[i].is_none())
            })
        })
    }

    pub(crate) fn place(
        &mut self,
        x: i32,
        y: i32,
        path: &str,
        tdt: &Tdt,
        o: u8,
        height: i16,
        tile: Option<usize>,
    ) {
        let size = tdt.size;
        let piece = self.pieces.len();
        self.pieces.push(Piece {
            x,
            y,
            path: path.to_string(),
            size,
            orientation: o,
            tile,
        });
        let (w, h) = (size.0 as i32, size.1 as i32);
        for ly in 0..h {
            for lx in 0..w {
                let (dx, dy) = orient_cell(o, w, h, lx, ly);
                let cell = Cell { path: path.to_string(), orientation: o, sub: (ly * w + lx) as u32, height, offset: (dx as u8, dy as u8) };
                if let Some(i) = self.index(x + dx, y + dy) {
                    self.piece_at[i] = Some(piece);
                    self.cells[i] = Some(cell);
                } else {
                    self.spill.push(Spill::Cell(x + dx, y + dy, piece, cell));
                }
            }
        }
        // Corner points (`0x141d0bad0`): the tile's ground per point, and its height where the
        // ground (old or new) is missing or takes heights (`.gt` flag 1).
        for v in 0..=h {
            for u in 0..=w {
                let k = ((w + 1) * v + u) as usize;
                let g = tdt.point_grounds.get(k).cloned().flatten().map(|g| g.to_ascii_lowercase());
                let (dx, dy) = crate::tileset::orient_point(o, w, h, u, v);
                let Some(i) = self.point_index(x + dx, y + dy) else {
                    let ph = (tdt.point_heights.get(k).copied().unwrap_or(0) as i16 + height) as i8;
                    self.spill.push(Spill::Point(x + dx, y + dy, g, ph));
                    continue;
                };
                let old = self.lattice[i].ground.clone();
                let takes_height = |m: &Self, g: &Option<String>| g.as_ref().is_none_or(|g| m.gt_flags(g) & 1 != 0);
                let set_height = takes_height(self, &g) || takes_height(self, &old);
                let p = &mut self.lattice[i];
                p.flags |= 1;
                if g.is_some() {
                    p.ground = g;
                }
                if set_height {
                    p.height = (tdt.point_heights.get(k).copied().unwrap_or(0) as i16 + height) as i8;
                    p.flags |= 2;
                }
            }
        }
    }
}

/// One `.gft` fill group: `(weight, path, orientation mask)`.
type FillGroup = Vec<(u32, String, u8)>;

/// The data of one area's master (`.tsi`): its tile set and fill groups.
pub struct Master {
    pub tiles: TileSet,
    /// Fill groups by name (`default`, or a ground type's name).
    pub fills: HashMap<String, FillGroup>,
    /// Tile headers by path (lower case), loaded on demand for fill tiles.
    headers: HashMap<String, Tdt>,
    /// Ground type file (lower case) -> its name (first line of the `.gt`).
    ground_names: HashMap<String, String>,
}

/// Reads a `.tdt`, applying an inheriting tile's overrides to its parent chain.
pub fn load_tdt(files: &dyn Files, path: &str) -> Result<Tdt, GenError> {
    let tdt = Tdt::parse(&files.read(path)?).map_err(|e| data_err(path, e))?;
    match &tdt.inherits {
        Some(parent) => Ok(tdt.resolve(&load_tdt(files, parent)?)),
        None => Ok(tdt),
    }
}

impl Master {
    pub fn load(files: &dyn Files, tst: &str, gft: &str) -> Result<Self, GenError> {
        let parsed = TSTParser
            .parse(&files.read(tst)?)
            .map_err(|e| data_err(tst, e))?;
        let mut defs = Vec::new();
        let mut headers = HashMap::new();
        let mut gt_flags: HashMap<String, u8> = HashMap::new();
        for e in &parsed.tdt_files {
            let tdt = load_tdt(files, &e.tdt_file)?;
            headers.insert(e.tdt_file.to_ascii_lowercase(), tdt.clone());
            let mut key = TileKey::from_tdt(&tdt);
            for g in key.grounds.iter_mut() {
                let Some(path) = g.clone() else { continue };
                let lower = path.to_ascii_lowercase();
                if !gt_flags.contains_key(&lower) {
                    let f = crate::rooms::ground_flags(&crate::text::decode_utf16(&files.read(&lower)?));
                    gt_flags.insert(lower.clone(), f);
                }
                if gt_flags[&lower] & 0x10 != 0 {
                    *g = None;
                }
            }
            defs.push(TileDef {
                key,
                path: e.tdt_file.clone(),
                weight: e.weight.unwrap_or(1),
                orientations: tdt.restrict_orientations(orientation_mask(&e.rotations)),
                tdt,
            });
        }
        let gft_file = GFTParser
            .parse(&files.read(gft)?)
            .map_err(|e| data_err(gft, e))?;
        let fills = gft_file
            .sections
            .iter()
            .map(|s| {
                let group = s
                    .files
                    .iter()
                    .map(|f| (f.weight, f.path.clone(), orientation_mask(&f.rotations)))
                    .collect();
                (s.name.to_ascii_lowercase(), group)
            })
            .collect();
        Ok(Self {
            tiles: TileSet::new(defs),
            fills,
            headers,
            ground_names: HashMap::new(),
        })
    }

    /// `.gt` flags of every ground the tile set's tiles use (for [`TileMap::ground_flags`]).
    pub fn ground_flag_table(&self, files: &dyn Files) -> Result<HashMap<String, u8>, GenError> {
        let mut out = HashMap::new();
        for t in &self.tiles.tiles {
            for g in t.tdt.point_grounds.iter().flatten().chain(t.tdt.grounds.iter().flatten()) {
                let key = g.to_ascii_lowercase();
                if !out.contains_key(&key) {
                    let text = crate::text::decode_utf16(&files.read(&key)?);
                    out.insert(key, crate::rooms::ground_flags(&text));
                }
            }
        }
        Ok(out)
    }

    /// Every `.et` the tile set's tiles use, with the split types of virtual ones.
    pub fn edge_type_table(&self, files: &dyn Files) -> Result<HashMap<String, crate::rooms::EdgeType>, GenError> {
        let mut out = HashMap::new();
        let mut todo: Vec<String> = self.tiles.tiles.iter().flat_map(|t| t.tdt.edges.iter().flatten().map(|e| e.to_ascii_lowercase())).collect();
        while let Some(key) = todo.pop() {
            if key.is_empty() || out.contains_key(&key) {
                continue;
            }
            let et = crate::rooms::EdgeType::parse(&crate::text::decode_utf16(&files.read(&key)?));
            todo.extend(et.splits.iter().map(|(p, _)| p.clone()));
            out.insert(key, et);
        }
        Ok(out)
    }

    /// The first fill tile of a ground's group (`0x141f93cc0`; the `default` group without a
    /// ground or when the ground's name has none), with its orientation (8 = any).
    pub fn first_fill_tile(&mut self, files: &dyn Files, ground: Option<&str>) -> Result<Option<(String, u8)>, GenError> {
        let name = match ground {
            Some(g) => self.ground_name(files, g)?,
            None => "default".into(),
        };
        let group = self.fills.get(&name).or_else(|| self.fills.get("default"));
        Ok(group.and_then(|g| g.iter().find(|(_, p, _)| p.to_ascii_lowercase().ends_with(".tdt"))).map(|(_, p, m)| {
            let o = if *m == 0xff { 8 } else { m.trailing_zeros() as u8 };
            (p.clone(), o)
        }))
    }

    /// `.et` flags (0x80 wildcard, 4 virtual) of every edge type the tile set's tiles use.
    pub fn edge_flag_table(&self, files: &dyn Files) -> Result<HashMap<String, u8>, GenError> {
        let mut out = HashMap::new();
        for t in &self.tiles.tiles {
            for e in t.tdt.edges.iter().flatten() {
                let key = e.to_ascii_lowercase();
                if !out.contains_key(&key) {
                    let et = crate::rooms::EdgeType::parse(&crate::text::decode_utf16(&files.read(&key)?));
                    out.insert(key, (et.wildcard as u8) << 7 | (et.virtual_edge as u8) << 2);
                }
            }
        }
        Ok(out)
    }

    pub(crate) fn header(&mut self, files: &dyn Files, path: &str) -> Result<&Tdt, GenError> {
        let key = path.to_ascii_lowercase();
        if !self.headers.contains_key(&key) {
            let tdt = load_tdt(files, path)?;
            self.headers.insert(key.clone(), tdt);
        }
        Ok(&self.headers[&key])
    }

    pub(crate) fn ground_name(&mut self, files: &dyn Files, gt: &str) -> Result<String, GenError> {
        let key = gt.to_ascii_lowercase();
        if let Some(n) = self.ground_names.get(&key) {
            return Ok(n.clone());
        }
        let parsed = GTParser
            .parse(&files.read(gt)?)
            .map_err(|e| data_err(gt, e))?;
        let name = parsed.name.trim().to_ascii_lowercase();
        self.ground_names.insert(key, name.clone());
        Ok(name)
    }
}

/// A room (`.arm`) as the generator uses it.
pub struct Room {
    pub arm: ARMFile,
    /// Ground overrides at interior lattice points, `(w-1)×(h-1)` row-major, as string-table
    /// indices (1-based, 0 = none). The file's last line; `poe_data_tools` does not parse it
    /// for version 36. Tiles touching an override are not re-picked by the de-repetition pass.
    pub lattice: Vec<u32>,
}

impl Room {
    pub fn width(&self) -> usize {
        self.arm.grid.first().map_or(0, Vec::len)
    }

    pub fn height(&self) -> usize {
        self.arm.grid.len()
    }

    /// Whether interior lattice point `(x, y)` (1..w-1, 1..h-1) has a ground override.
    pub fn overridden(&self, x: usize, y: usize) -> bool {
        let (w, h) = (self.width(), self.height());
        x >= 1
            && y >= 1
            && x < w
            && y < h
            && self
                .lattice
                .get((y - 1) * (w - 1) + (x - 1))
                .is_some_and(|&v| v != 0)
    }
}

pub fn load_room(files: &dyn Files, path: &str) -> Result<Room, GenError> {
    let bytes = files.read(path)?;
    let arm = ARMParser.parse(&bytes).map_err(|e| data_err(path, e))?;
    let (w, h) = (arm.grid.first().map_or(0, Vec::len), arm.grid.len());
    let want = w.saturating_sub(1) * h.saturating_sub(1);
    let text = crate::text::decode_utf16(&bytes);
    let lattice = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| {
            l.split_whitespace()
                .filter_map(|v| v.parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .filter(|v| v.len() == want)
        .unwrap_or_default();
    Ok(Room { arm, lattice })
}

/// The key of a `k` slot (also a room's root slot).
pub fn slot_key(k: &poe_data_tools::file_parsers::arm::types::SlotK) -> TileKey {
    TileKey {
        edges: k
            .edges
            .clone()
            .map(|e| e.edge.map(|s| s.to_ascii_lowercase().replace('\\', "/"))),
        size: (k.width as u8, k.height as u8),
        grounds: k
            .corners
            .clone()
            .map(|c| c.ground.map(|s| s.to_ascii_lowercase().replace('\\', "/"))),
        heights: k.corners.clone().map(|c| c.height as u8),
        exits: k.edges.clone().map(|e| e.exit as u8),
        virtual_exits: k.edges.clone().map(|e| e.virtual_exit as u8),
    }
}

/// The key of an `.arm` grid cell for the room pass, or None when the cell places nothing
/// itself (uniform-ground `k` slots only mark their ground; `n`/`s`/`o` cells).
pub(crate) fn cell_key(slot: &Slot) -> Option<(TileKey, String)> {
    match slot {
        Slot::K(k) => {
            Some((slot_key(k), k.slot_tag.clone().unwrap_or_default()))
        }
        Slot::F { fill: Some(name) } => Some((
            TileKey {
                size: (1, 1),
                exits: [3; 4],
                virtual_exits: [3; 4],
                ..Default::default()
            },
            name.clone(),
        )),
        _ => None,
    }
}

pub(crate) fn uniform_ground(key: &TileKey, feature: &str) -> bool {
    feature.is_empty()
        && key.edges.iter().all(Option::is_none)
        && key.grounds.iter().all(|g| *g == key.grounds[0])
}

/// Room pass (`0x141f8e6c0`) for an unrotated room.
pub fn room_pass(
    map: &mut TileMap,
    master: &Master,
    room: &Room,
    origin: (i32, i32),
    rng: &mut TerrainRng,
) -> Result<(), GenError> {
    room_pass_oriented(map, master, room, origin, 0, false, rng)
}

/// Room pass (`0x141f8e6c0`): every cell of the room grid, x outer, y inner; a slot's key is
/// seen with the room's orientation `o` and lands where its cells land when the room is
/// turned (`origin` is the turned room's corner). With `mark` (its eighth argument; overlays set
/// it, node rooms do not), uniform-ground slots also give their ground to the corners x in
/// `0..w`, y in `0..=h` (`0x141d0e130`).
pub fn room_pass_oriented(
    map: &mut TileMap,
    master: &Master,
    room: &Room,
    origin: (i32, i32),
    o: u8,
    mark: bool,
    rng: &mut TerrainRng,
) -> Result<(), GenError> {
    let (w, h) = (room.width(), room.height());
    let room_full = room;
    let room = &room.arm;
    for x in 0..w {
        for y in 0..h {
            let Some((key, feature)) = cell_key(&room.grid[y][x]) else {
                continue;
            };
            let (sw, sh) = (key.size.0.max(1) as i32, key.size.1.max(1) as i32);
            let key = key.oriented(o);
            let (ax, ay) = orient_cell(o, w as i32, h as i32, x as i32, y as i32);
            let (bx, by) = orient_cell(o, w as i32, h as i32, x as i32 + sw - 1, y as i32 + sh - 1);
            let (mx, my) = (origin.0 + ax.min(bx), origin.1 + ay.min(by));
            if uniform_ground(&key, &feature) {
                // Nothing is placed (the fill pass places a tile here).
                for dy in 0..key.size.1 as i32 {
                    for dx in 0..key.size.0 as i32 {
                        if let Some(i) = map.index(mx + dx, my + dy) {
                            map.grounds[i] = key.grounds[0].clone();
                        }
                    }
                }
                if let (true, Some(g)) = (mark, &key.grounds[0]) {
                    for dy in 0..=key.size.1 as i32 {
                        for dx in 0..key.size.0 as i32 {
                            map.set_ground(mx + dx, my + dy, g);
                        }
                    }
                }
                continue;
            }
            let list = master.tiles.candidates(&feature, &key);
            if list.is_empty() {
                return Err(GenError::Unsupported(format!(
                    "no tile for room cell ({x},{y}) feature {feature:?}"
                )));
            }
            let total = list.iter().map(|c| c.weight).sum();
            let c = pick(list, rng.below(total)).expect("non-empty list");
            let def = &master.tiles.tiles[c.tile];
            map.place(
                mx,
                my,
                &def.path,
                &def.tdt,
                c.orientation,
                0,
                Some(c.tile),
            );
        }
    }
    map.room_overrides(room_full, origin, o);
    Ok(())
}

/// Edge routing cost grid (`0x1420cdc30`): one `below(1024)` per map cell. Routing itself
/// (graphs with edges) is not ported yet.
pub fn edge_cost_grid(map: &TileMap, rng: &mut TerrainRng) -> Vec<u32> {
    (0..map.width * map.height)
        .map(|_| rng.below(1024))
        .collect()
}

/// Fill pass (`0x141f8f930` per empty cell, row by row): a weighted pick from the cell's
/// ground's fill group (`default` without ground), then the orientation (`0x141f90d60`).
pub fn fill_pass(
    map: &mut TileMap,
    master: &mut Master,
    files: &dyn Files,
    rng: &mut TerrainRng,
) -> Result<(), GenError> {
    for y in 0..map.height as i32 {
        for x in 0..map.width as i32 {
            let i = map.index(x, y).unwrap();
            if map.cells[i].is_some() {
                continue;
            }
            let group_name = match map.grounds[i].clone() {
                Some(gt) => master.ground_name(files, &gt)?,
                None => "default".to_string(),
            };
            let mut group: FillGroup =
                master.fills.get(&group_name).cloned().ok_or_else(|| {
                    GenError::Unsupported(format!("no fill group {group_name:?}"))
                })?;
            loop {
                if group.is_empty() {
                    return Err(GenError::Unsupported(format!("no fill fits at ({x},{y})")));
                }
                let total: u32 = group.iter().map(|g| g.0).sum();
                let mut r = rng.below(total);
                let mut index = group.len() - 1;
                for (k, g) in group.iter().enumerate() {
                    if r < g.0 {
                        index = k;
                        break;
                    }
                    r -= g.0;
                }
                let (_, path, _mask) = group[index].clone();
                if !path.to_ascii_lowercase().ends_with(".tdt") {
                    return Err(GenError::Unsupported(format!("fill room {path}")));
                }
                let tdt = master.header(files, &path)?.clone();
                let size = tdt.size;
                // Orientation "any": square tiles draw one of 8 if they fit unrotated.
                if size.0 != size.1 {
                    return Err(GenError::Unsupported(format!("non-square fill {path}")));
                }
                if map.fits(x, y, size, 0) {
                    let o = rng.below(8) as u8;
                    map.place(x, y, &path, &tdt, o, 0, None);
                    break;
                }
                group.remove(index);
            }
        }
    }
    Ok(())
}

/// Mirror orientation across a shared side (`0x141fdd7c0`'s table): `[o][0]` for a left/right
/// neighbour, `[o][1]` for an up/down one.
const MIRROR: [[u8; 2]; 8] = [
    [4, 6],
    [7, 5],
    [6, 4],
    [5, 7],
    [0, 2],
    [3, 1],
    [2, 0],
    [1, 3],
];
/// Neighbour directions in the pass's order: up, right, down, left (in tile-size steps).
const NEIGHBOURS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// A room as the de-repetition pass sees it: where it lies, turned.
pub struct RoomPlacement<'a> {
    pub room: &'a Room,
    /// The turned room's corner.
    pub origin: (i32, i32),
    pub orientation: u8,
}

/// The tile on a cell's origin: tile-set tile (by path), orientation, oriented size.
fn origin_tile(map: &TileMap, x: i32, y: i32) -> Option<(usize, (i32, i32))> {
    let i = map.index(x, y)?;
    let c = map.cells[i].as_ref()?;
    if c.offset != (0, 0) {
        return None;
    }
    let p = map.piece_at[i]?;
    let piece = &map.pieces[p];
    Some((p, TileMap::oriented_size(piece.size, piece.orientation)))
}

/// `0x141fde4b0`: tile-set tile `tile` with orientation `o` at `pos` would mirror a neighbour
/// one tile-size away (up and left only, or all four sides): the same tile in the mirror
/// orientation across the shared side.
fn mirrors(map: &TileMap, master: &Master, pos: (i32, i32), tile: usize, o: u8, all: bool) -> bool {
    let path = &master.tiles.tiles[tile].path;
    let (w, h) = TileMap::oriented_size(master.tiles.tiles[tile].tdt.size, o);
    let dirs: &[usize] = if all { &[0, 1, 2, 3] } else { &[0, 3] };
    dirs.iter().any(|&dir| {
        let (dx, dy) = NEIGHBOURS[dir];
        let Some((p, _)) = origin_tile(map, pos.0 + dx * w, pos.1 + dy * h) else { return false };
        let n = &map.pieces[p];
        n.path.eq_ignore_ascii_case(path) && MIRROR[o as usize][!dir & 1] == n.orientation
    })
}

struct Derepeat<'a> {
    master: &'a Master,
    /// Cells inside a placed room's rectangle.
    in_room: Vec<bool>,
    /// Corners with a room ground override.
    overridden: Vec<bool>,
}

impl Derepeat<'_> {
    /// `0x141fde210`: any cell of the block lies in a room.
    fn in_room(&self, map: &TileMap, pos: (i32, i32), (w, h): (i32, i32)) -> bool {
        (pos.1..pos.1 + h).any(|y| (pos.0..pos.0 + w).any(|x| map.index(x, y).is_some_and(|i| self.in_room[i])))
    }

    /// `0x141fde360`: any corner of the block (far sides included) has a ground override.
    fn touches_override(&self, map: &TileMap, pos: (i32, i32), (w, h): (i32, i32)) -> bool {
        (pos.1..=pos.1 + h).any(|y| (pos.0..=pos.0 + w).any(|x| map.point_index(x, y).is_some_and(|i| self.overridden[i])))
    }

    /// A placed piece's tile-set index key (`0x141d08a40` turned, `0x141d09900`: grounds with
    /// `.gt` flag 0x10 dropped).
    fn key_of(&self, map: &TileMap, p: usize) -> Option<TileKey> {
        let piece = &map.pieces[p];
        Some(self.master.tiles.tiles[piece.tile?].key.oriented(piece.orientation))
    }

    /// `0x141fde880` (its corner ground fix-ups are not ported).
    fn replace(&self, map: &mut TileMap, pos: (i32, i32), tile: usize, o: u8) {
        let def = &self.master.tiles.tiles[tile];
        map.place(pos.0, pos.1, &def.path, &def.tdt, o, 0, Some(tile));
    }

    /// `0x141fdfa60`: re-pick the tile at `pos` (it mirrors the neighbour at `npos` across
    /// side `dir`) from its key's candidates minus mirroring ones; with none, try moving an edge
    /// crossing (`0x141fdef90`).
    fn repick(&self, map: &mut TileMap, pos: (i32, i32), npos: (i32, i32), dir: usize, rng: &mut TerrainRng) {
        let (p, size) = origin_tile(map, pos.0, pos.1).unwrap();
        let Some(key) = self.key_of(map, p) else { return };
        let (np, _) = origin_tile(map, npos.0, npos.1).unwrap();
        let (npath, no) = (map.pieces[np].path.clone(), map.pieces[np].orientation);
        let list: Vec<_> = self
            .master
            .tiles
            .candidates("", &key)
            .iter()
            .filter(|c| {
                !mirrors(map, self.master, pos, c.tile, c.orientation, false)
                    && !(self.master.tiles.tiles[c.tile].path.eq_ignore_ascii_case(&npath)
                        && MIRROR[c.orientation as usize][!dir & 1] == no)
                    && c.weight != 0
            })
            .copied()
            .collect();
        if !list.is_empty() {
            let total = list.iter().map(|c| c.weight).sum();
            let c = pick(&list, rng.below(total)).unwrap();
            self.replace(map, pos, c.tile, c.orientation);
        } else if !self.in_room(map, pos, size) {
            self.shift_edge(map, pos, size, &key, dir, rng);
        }
    }

    /// `0x141fdef90`: on another side with a (non-virtual) edge and a same-size neighbour that
    /// is free to change, every other exit position whose tiles do not mirror up or left is a
    /// candidate; one is picked and kept if the neighbour has a tile with the matching exit.
    fn shift_edge(&self, map: &mut TileMap, pos: (i32, i32), size: (i32, i32), key: &TileKey, dir: usize, rng: &mut TerrainRng) {
        let mut list: Vec<(crate::tileset::Candidate, usize)> = Vec::new();
        for d in 0..4 {
            if d == dir {
                continue;
            }
            let Some(edge) = &key.edges[d] else { continue };
            if map.edge_flags.get(edge).copied().unwrap_or(0) & 4 != 0 {
                continue;
            }
            let n = (pos.0 + NEIGHBOURS[d].0 * size.0, pos.1 + NEIGHBOURS[d].1 * size.1);
            let Some((np, nsize)) = origin_tile(map, n.0, n.1) else { continue };
            if nsize != size {
                continue;
            }
            let Some(nt) = map.pieces[np].tile else { continue };
            if self.master.tiles.tiles[nt].tdt.feature.as_deref().is_some_and(|f| !f.is_empty()) {
                continue;
            }
            if self.touches_override(map, n, nsize) || self.in_room(map, n, nsize) {
                continue;
            }
            let count = if d & 1 == 0 { key.size.0 } else { key.size.1 } as u32 * 3;
            for e in 0..count as u8 {
                if e == key.exits[d] {
                    continue;
                }
                let mut k2 = key.clone();
                k2.exits[d] = e;
                for c in self.master.tiles.candidates("", &k2) {
                    if !mirrors(map, self.master, pos, c.tile, c.orientation, false) && c.weight != 0 {
                        list.push((*c, d));
                    }
                }
            }
        }
        let mut total: u32 = list.iter().map(|(c, _)| c.weight).sum();
        while !list.is_empty() {
            let mut r = rng.below(total);
            let mut k = list.len() - 1;
            for (i, (c, _)) in list.iter().enumerate() {
                if r < c.weight {
                    k = i;
                    break;
                }
                r -= c.weight;
            }
            let (c, d) = list[k];
            let new_key = TileKey::from_tdt(&self.master.tiles.tiles[c.tile].tdt).oriented(c.orientation);
            let span = if d & 1 == 0 { new_key.size.0 } else { new_key.size.1 };
            let n = (pos.0 + NEIGHBOURS[d].0 * size.0, pos.1 + NEIGHBOURS[d].1 * size.1);
            let (np, _) = origin_tile(map, n.0, n.1).unwrap();
            if let Some(mut nk) = self.key_of(map, np) {
                nk.exits[(d + 2) & 3] = span.wrapping_mul(3).wrapping_sub(new_key.exits[d]).wrapping_sub(1);
                let cpath = &self.master.tiles.tiles[c.tile].path;
                let list2: Vec<_> = self
                    .master
                    .tiles
                    .candidates("", &nk)
                    .iter()
                    .filter(|c2| {
                        !mirrors(map, self.master, n, c2.tile, c2.orientation, true)
                            && !(self.master.tiles.tiles[c2.tile].path == *cpath
                                && MIRROR[c.orientation as usize][!d & 1] == c2.orientation)
                            && c2.weight != 0
                    })
                    .copied()
                    .collect();
                if !list2.is_empty() {
                    self.replace(map, pos, c.tile, c.orientation);
                    let t2 = list2.iter().map(|c| c.weight).sum();
                    let c2 = pick(&list2, rng.below(t2)).unwrap();
                    self.replace(map, n, c2.tile, c2.orientation);
                    return;
                }
            }
            let i = list.iter().position(|(e, ed)| e.tile == c.tile && e.orientation == c.orientation && *ed == d).unwrap();
            total -= list[i].0.weight;
            list.remove(i);
        }
    }
}

/// De-repetition (`0x141fdd7c0`): cells row by row; a tile-set tile bigger than 1×1 without a
/// feature, not of uniform ground, not touching a room ground override, that mirrors a
/// neighbour one tile-size away (same tile, mirror orientation across the shared side; sides
/// up, right, down, left) is re-picked (`0x141fdfa60`).
pub fn derepeat_pass(map: &mut TileMap, master: &Master, rooms: &[RoomPlacement], rng: &mut TerrainRng) {
    let mut in_room = vec![false; map.cells.len()];
    let mut overridden = vec![false; map.lattice.len()];
    for r in rooms {
        let (w, h) = (r.room.width() as i32, r.room.height() as i32);
        let (ow, oh) = TileMap::oriented_size((w as u8, h as u8), r.orientation);
        for y in r.origin.1..r.origin.1 + oh {
            for x in r.origin.0..r.origin.0 + ow {
                if let Some(i) = map.index(x, y) {
                    in_room[i] = true;
                }
            }
        }
        for v in 1..h {
            for u in 1..w {
                if r.room.lattice.get(((v - 1) * (w - 1) + (u - 1)) as usize).is_some_and(|&k| k != 0) {
                    let (dx, dy) = crate::tileset::orient_point(r.orientation, w, h, u, v);
                    if let Some(i) = map.point_index(r.origin.0 + dx, r.origin.1 + dy) {
                        overridden[i] = true;
                    }
                }
            }
        }
    }
    let d = Derepeat { master, in_room, overridden };
    for y in 0..map.height as i32 {
        for x in 0..map.width as i32 {
            let Some((p, size)) = origin_tile(map, x, y) else { continue };
            if size == (1, 1) {
                continue;
            }
            let Some(tile) = map.pieces[p].tile else { continue };
            let def = &master.tiles.tiles[tile];
            if def.tdt.feature.as_deref().is_some_and(|f| !f.is_empty()) {
                continue;
            }
            let key = TileKey::from_tdt(&def.tdt).oriented(map.pieces[p].orientation);
            if uniform_ground(&key, "") || d.touches_override(map, (x, y), size) {
                continue;
            }
            let (path, o) = (map.pieces[p].path.clone(), map.pieces[p].orientation);
            for (dir, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
                let n = (x + dx * size.0, y + dy * size.1);
                let Some((np, _)) = origin_tile(map, n.0, n.1) else { continue };
                let nb = &map.pieces[np];
                if nb.path.eq_ignore_ascii_case(&path) && nb.orientation == MIRROR[o as usize][!dir & 1] {
                    d.repick(map, (x, y), n, dir, rng);
                    break;
                }
            }
        }
    }
}

impl TileMap {
    /// The client's tile hash of this layout (column by column).
    pub fn tile_hash(&self) -> u32 {
        let mut tiles = Vec::new();
        for x in 0..self.width {
            for y in 0..self.height {
                if let Some(c) = self.get(x, y).filter(|c| c.offset == (0, 0)) {
                    tiles.push(crate::hash::HashedTile {
                        x,
                        y,
                        path: &c.path,
                        height: c.height,
                        orientation: c.orientation,
                    });
                }
            }
        }
        crate::hash::tile_hash(tiles)
    }
}
