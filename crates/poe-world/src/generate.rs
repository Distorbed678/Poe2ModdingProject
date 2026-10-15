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
        }
    }

    /// Lattice index of corner `(x, y)` (0..=width, 0..=height).
    pub fn point_index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x as u32 <= self.width && y as u32 <= self.height)
            .then(|| (y as u32 * (self.width + 1) + x as u32) as usize)
    }

    /// `.gt` flags of a ground (`+0x24`).
    pub fn gt_flags(&self, ground: &str) -> u8 {
        self.ground_flags.get(ground).copied().unwrap_or(0x0f)
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height)
            .then(|| (y as u32 * self.width + x as u32) as usize)
    }

    pub fn get(&self, x: u32, y: u32) -> Option<&Cell> {
        self.cells.get((y * self.width + x) as usize)?.as_ref()
    }

    /// Size of a tile with orientation `o` (odd orientations swap).
    fn oriented_size(size: (u8, u8), o: u8) -> (i32, i32) {
        if o & 1 == 1 && o < 8 {
            (size.1 as i32, size.0 as i32)
        } else {
            (size.0 as i32, size.1 as i32)
        }
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
                if let Some(i) = self.index(x + dx, y + dy) {
                    self.piece_at[i] = Some(piece);
                    self.cells[i] = Some(Cell {
                        path: path.to_string(),
                        orientation: o,
                        sub: (ly * w + lx) as u32,
                        height,
                        offset: (dx as u8, dy as u8),
                    });
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
                let Some(i) = self.point_index(x + dx, y + dy) else { continue };
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

impl Master {
    pub fn load(files: &dyn Files, tst: &str, gft: &str) -> Result<Self, GenError> {
        let parsed = TSTParser
            .parse(&files.read(tst)?)
            .map_err(|e| data_err(tst, e))?;
        let mut defs = Vec::new();
        let mut headers = HashMap::new();
        for e in &parsed.tdt_files {
            let tdt =
                Tdt::parse(&files.read(&e.tdt_file)?).map_err(|x| data_err(&e.tdt_file, x))?;
            headers.insert(e.tdt_file.to_ascii_lowercase(), tdt.clone());
            defs.push(TileDef {
                path: e.tdt_file.clone(),
                tdt,
                weight: e.weight.unwrap_or(1),
                orientations: orientation_mask(&e.rotations),
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

    fn header(&mut self, files: &dyn Files, path: &str) -> Result<&Tdt, GenError> {
        let key = path.to_ascii_lowercase();
        if !self.headers.contains_key(&key) {
            let tdt = Tdt::parse(&files.read(path)?).map_err(|e| data_err(path, e))?;
            self.headers.insert(key.clone(), tdt);
        }
        Ok(&self.headers[&key])
    }

    fn ground_name(&mut self, files: &dyn Files, gt: &str) -> Result<String, GenError> {
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
fn cell_key(slot: &Slot) -> Option<(TileKey, String)> {
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

fn uniform_ground(key: &TileKey, feature: &str) -> bool {
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
    room_pass_oriented(map, master, room, origin, 0, rng)
}

/// Room pass (`0x141f8e6c0`): every cell of the room grid, x outer, y inner; a slot's key is
/// seen with the room's orientation `o` and lands where its cells land when the room is
/// turned (`origin` is the turned room's corner).
pub fn room_pass_oriented(
    map: &mut TileMap,
    master: &Master,
    room: &Room,
    origin: (i32, i32),
    o: u8,
    rng: &mut TerrainRng,
) -> Result<(), GenError> {
    let (w, h) = (room.width(), room.height());
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
                // Only the ground is marked (the fill pass places a tile here).
                for dy in 0..key.size.1 as i32 {
                    for dx in 0..key.size.0 as i32 {
                        if let Some(i) = map.index(mx + dx, my + dy) {
                            map.grounds[i] = key.grounds[0].clone();
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

/// `0x141fde4b0` (last argument 0): whether `tile` with orientation `o` at `piece`'s corner
/// would mirror its up or left neighbour (the same tile in the mirror orientation one tile-size
/// away).
fn mirrors_up_or_left(map: &TileMap, master: &Master, piece: &Piece, tile: usize, o: u8) -> bool {
    let size = master.tiles.tiles[tile].tdt.size;
    let (w, h) = TileMap::oriented_size(size, o);
    [(0usize, (0, -1)), (3usize, (-1, 0))]
        .iter()
        .any(|&(dir, (dx, dy))| {
            let Some(i) = map.index(piece.x + dx * w, piece.y + dy * h) else {
                return false;
            };
            let Some(n) = map.piece_at[i] else {
                return false;
            };
            let n = &map.pieces[n];
            n.tile == Some(tile) && MIRROR[o as usize][!dir & 1] == n.orientation
        })
}

/// De-repetition (`0x141fdd7c0` → `0x141fdfa60`): row by row over placed tile-set pieces
/// bigger than 1×1, if a neighbour one tile-size away is the same tile in the mirror
/// orientation across the shared side, the piece is re-picked from its key's candidates
/// minus those that would mirror that neighbour (`below(total)`).
pub fn derepeat_pass(
    map: &mut TileMap,
    master: &Master,
    rooms: &[(&Room, (i32, i32))],
    rng: &mut TerrainRng,
) {
    // Lattice points (tile corners, (w+1)×(h+1)) with a room ground override.
    let lw = map.width as i32 + 1;
    let mut marked = vec![false; (lw * (map.height as i32 + 1)) as usize];
    for (room, (ox, oy)) in rooms {
        for y in 1..room.height() {
            for x in 1..room.width() {
                let (px, py) = (ox + x as i32, oy + y as i32);
                if room.overridden(x, y) && px < lw && py <= map.height as i32 {
                    marked[(py * lw + px) as usize] = true;
                }
            }
        }
    }
    let mut origins: Vec<usize> = (0..map.pieces.len())
        .filter(|&p| map.pieces[p].tile.is_some() && map.pieces[p].size != (1, 1))
        .collect();
    origins.sort_by_key(|&p| (map.pieces[p].y, map.pieces[p].x));
    for p in origins {
        let piece = map.pieces[p].clone();
        let tile = piece.tile.unwrap();
        let def = &master.tiles.tiles[tile];
        if def.tdt.feature.as_deref().is_some_and(|f| !f.is_empty()) {
            continue;
        }
        let key = TileKey::from_tdt(&def.tdt).oriented(piece.orientation);
        if uniform_ground(&key, "") {
            continue;
        }
        let (w, h) = TileMap::oriented_size(piece.size, piece.orientation);
        let touches_override = (piece.y..=piece.y + h).any(|py| {
            (piece.x..=piece.x + w).any(|px| {
                px >= 0 && py >= 0 && px < lw && marked.get((py * lw + px) as usize) == Some(&true)
            })
        });
        if touches_override {
            continue;
        }
        for (dir, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
            let Some(ni) = map.index(piece.x + dx * w, piece.y + dy * h) else {
                continue;
            };
            let Some(n) = map.piece_at[ni] else { continue };
            let neighbour = &map.pieces[n];
            if neighbour.tile != Some(tile)
                || neighbour.orientation != MIRROR[piece.orientation as usize][!dir & 1]
            {
                continue;
            }
            let axis = (!dir & 1) as usize;
            let neighbour_o = neighbour.orientation;
            let list: Vec<_> = master
                .tiles
                .candidates("", &key)
                .iter()
                .filter(|c| {
                    !mirrors_up_or_left(map, master, &piece, c.tile, c.orientation)
                        && !(c.tile == tile && MIRROR[c.orientation as usize][axis] == neighbour_o)
                })
                .copied()
                .collect();
            if !list.is_empty() {
                let total = list.iter().map(|c| c.weight).sum();
                let c = pick(&list, rng.below(total)).unwrap();
                let def = &master.tiles.tiles[c.tile];
                map.place(
                    piece.x,
                    piece.y,
                    &def.path,
                    &def.tdt,
                    c.orientation,
                    0,
                    Some(c.tile),
                );
            }
            break;
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
