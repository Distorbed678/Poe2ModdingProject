//! The dungeon tile pass (`0x14209c4c0`): every placed room's `.arm` slots become tiles
//! (`0x1420983d0`, pass A), and the slots on the one-tile borders rooms share are merged with
//! the neighbour's and tiled afterwards (`0x14209fbc0`, pass B).

use std::collections::HashMap;

use crate::{
    dungeon::Grid,
    generate::{GenError, Master, Room, TileMap, cell_key},
    rng::TerrainRng,
    tileset::{TileKey, orient_cell, pick},
};

fn unsupported(s: impl Into<String>) -> GenError {
    GenError::Unsupported(s.into())
}

/// The edge-type combiner (`.ecf`, master `EdgeCombination`): two edge types meeting on a
/// shared border, with the corner height difference along it, give one edge type.
#[derive(Default)]
pub struct Ecf {
    rules: Vec<(Option<String>, Option<String>, Option<String>, i32)>,
}

impl Ecf {
    pub fn parse(text: &str) -> Self {
        let mut rules = Vec::new();
        for line in text.lines().skip(1) {
            let parts: Vec<&str> = line.split('"').collect();
            if parts.len() < 7 {
                continue;
            }
            let p = |s: &str| (!s.is_empty()).then(|| s.to_ascii_lowercase().replace('\\', "/"));
            let dh = parts[6].trim().parse().unwrap_or(0);
            rules.push((p(parts[1]), p(parts[3]), p(parts[5]), dh));
        }
        Self { rules }
    }

    /// `0x1420a28c0`: the rule for the pair (either order) and height difference; without one,
    /// a lone edge stays and two different edges give none.
    fn combine(&self, a: &Option<String>, b: &Option<String>, dh: i32) -> Option<String> {
        if let Some(r) = self.rules.iter().find(|r| r.3 == dh && ((&r.0 == a && &r.1 == b) || (&r.0 == b && &r.1 == a))) {
            return r.2.clone();
        }
        match (a, b) {
            (None, b) => b.clone(),
            (a, None) => a.clone(),
            _ => None,
        }
    }
}

/// A shared-border slot (`0x142097f80`'s 0x80-byte entry, kept sorted by x·65536 + y).
#[derive(Debug, Clone)]
struct Border {
    key: TileKey,
    /// +0x58.
    height: i32,
    /// +0x5c: bit 0 reachable; the sides its rooms cover: 4 top, 0x10 right, 2 bottom, 8 left.
    seen: u8,
    /// +0x5d: sides both rooms gave.
    merged: u8,
    names: Vec<String>,
}

const CORNER_MASKS: [u8; 4] = [0x0c, 0x14, 0x12, 0x0a];
const EDGE_MASKS: [u8; 4] = [0x04, 0x10, 0x02, 0x08];

/// `0x1420972f0`: `b` (the stored key) with what `a` covers (`fa`) laid over it. Corners `a`
/// covers fully take its ground (none for `.gt` flag 0x10) and height (rebased from `ha` to
/// `hb`); sides only `a` covers take its edge; sides both cover are combined (`0x142097620`).
#[allow(clippy::too_many_arguments)]
fn merge(map: &TileMap, ecf: &Ecf, a: &TileKey, b: &TileKey, ha: i32, hb: i32, fa: u8, fb: u8, merged: &mut u8) -> Result<TileKey, GenError> {
    if a.size != b.size {
        return Err(unsupported("dungeon: cannot combine tile keys of different sizes"));
    }
    let mut out = b.clone();
    for i in 0..4 {
        if fa & CORNER_MASKS[i] != CORNER_MASKS[i] {
            continue;
        }
        out.grounds[i] = a.grounds[i].clone().filter(|g| map.gt_flags(g) & 0x10 == 0);
        out.heights[i] = (a.heights[i] as i32 - hb + ha) as u8;
    }
    for i in 0..4 {
        let m = EDGE_MASKS[i];
        if fa & m == 0 {
            continue;
        }
        if fb & m == 0 {
            out.edges[i] = a.edges[i].clone();
            out.exits[i] = a.exits[i];
            out.virtual_exits[i] = a.virtual_exits[i];
        } else {
            if b.edges[i].is_some() || a.edges[i].is_some() {
                let dh = (out.heights[i] as i8 as i32 - out.heights[(i + 1) & 3] as i8 as i32).abs();
                let c = ecf.combine(&b.edges[i], &a.edges[i], dh);
                let full = 3 * if i % 2 == 0 { out.size.0 } else { out.size.1 };
                match c {
                    None => {
                        out.edges[i] = None;
                        out.exits[i] = full;
                        out.virtual_exits[i] = full;
                    }
                    Some(c) => {
                        out.edges[i] = Some(c);
                        if b.edges[i].is_none() {
                            (out.exits[i], out.virtual_exits[i]) = (a.exits[i], a.virtual_exits[i]);
                        } else if a.edges[i].is_none() {
                            (out.exits[i], out.virtual_exits[i]) = (b.exits[i], b.virtual_exits[i]);
                        } else {
                            out.exits[i] = if b.exits[i] == a.exits[i] { a.exits[i] } else { full >> 1 };
                            out.virtual_exits[i] = if b.virtual_exits[i] == a.virtual_exits[i] { a.virtual_exits[i] } else { full >> 1 };
                        }
                    }
                }
            }
            *merged |= m;
        }
        if out.virtual_exits[i] < out.exits[i] {
            std::mem::swap(&mut out.exits[i], &mut out.virtual_exits[i]);
        }
    }
    Ok(out)
}

/// `0x142097aa0`: a key with no edges whose corners share one ground (`.gt` flag 0x10 grounds
/// count as none; a height only matters on grounds with flag 1).
fn uniform(map: &TileMap, key: &TileKey) -> bool {
    let g = |i: usize| key.grounds[i].clone().filter(|g| map.gt_flags(g) & 0x10 == 0);
    let g0 = g(0);
    (0..4).all(|i| {
        let flat = key.heights[i] == 0 || key.grounds[i].as_ref().is_some_and(|gr| map.gt_flags(gr) & 1 == 0);
        flat && key.edges[i].is_none() && (i == 0 || g(i) == g0)
    })
}

/// A ground-only cell for the fill (`0x142097cb0`'s 0x18-byte record at context +0x58).
#[derive(Debug, Clone)]
pub struct GroundCell {
    pub pos: (i32, i32),
    pub ground: Option<String>,
    pub accessible: bool,
    pub height: i32,
}

/// `0x142097cb0`: a ground-only slot. Its corners take the height where none is set yet, and
/// each cell is listed with its ground for the fill.
fn mark_ground(map: &mut TileMap, cells: &mut Vec<GroundCell>, pos: (i32, i32), key: &TileKey, height: i32, accessible: bool) {
    let (w, h) = (key.size.0 as i32, key.size.1 as i32);
    for y in pos.1..=pos.1 + h {
        for x in pos.0..=pos.0 + w {
            if let Some(i) = map.point_index(x, y) {
                if map.lattice[i].flags & 2 == 0 {
                    map.lattice[i].height = height as i8;
                    map.lattice[i].flags |= 2;
                }
            }
        }
    }
    for y in pos.1..pos.1 + h {
        for x in pos.0..pos.0 + w {
            if let Some(i) = map.index(x, y) {
                map.grounds[i] = key.grounds[0].clone();
            }
            cells.push(GroundCell { pos: (x, y), ground: key.grounds[0].clone(), accessible, height });
        }
    }
}

/// Door bookkeeping (only with `door` tiles, tile pass +0x38): exit markers on room borders
/// (`0x142096ad0`), groups of 1×1 passage tiles at shared exits and larger tiles there
/// (`0x142097060`).
#[derive(Default)]
struct Doors {
    on: bool,
    markers: HashMap<(i32, i32), u8>,
    groups: Vec<Vec<(i32, i32)>>,
    group_of: HashMap<(i32, i32), usize>,
    large: Vec<(i32, i32)>,
}

impl Doors {
    /// `0x142096ad0`: the middle cells (3, or 1 below 5 tiles) of every open exit's unit on the
    /// room's border line, flagged with the side (4 top, 0x10 right, 2 bottom, 8 left).
    fn mark(&mut self, p: &crate::dungeon::PlacedRoom, unit: i32, overlap: i32) {
        let step = unit - overlap;
        let base = (p.origin.0 * step, p.origin.1 * step);
        let (w, h) = (p.size.0 as i32, p.size.1 as i32);
        let (rw, rh) = (step * w + overlap, step * h + overlap);
        for (u, e) in p.exits.iter().enumerate() {
            if e.door.is_none() {
                continue;
            }
            let u = u as i32;
            let count = |len: i32| if len < 5 || unit < 5 { 1 } else { 3 };
            let (cells, flag): (Vec<(i32, i32)>, u8) = if u < w {
                let c = count(rw);
                ((0..c).map(|k| (base.0 + step * u - c / 2 + unit / 2 + k, base.1)).collect(), 4)
            } else if u - w < h {
                let c = count(rh);
                ((0..c).map(|k| (base.0 + rw - 1, base.1 + step * (u - w) - c / 2 + unit / 2 + k)).collect(), 0x10)
            } else if u - w - h < w {
                let c = count(rw);
                ((0..c).map(|k| (base.0 + step * (2 * w + h - 1 - u) - c / 2 + unit / 2 + k, base.1 + rh - 1)).collect(), 2)
            } else {
                let c = count(rh);
                ((0..c).map(|k| (base.0, base.1 + step * (2 * (w + h) - 1 - u) - c / 2 + unit / 2 + k)).collect(), 8)
            };
            for c in cells {
                *self.markers.entry(c).or_default() |= flag;
            }
        }
    }

    /// `0x142097060`.
    fn add(&mut self, pos: (i32, i32), size: (u8, u8)) {
        if size != (1, 1) {
            self.large.push(pos);
            return;
        }
        let id = |d: (i32, i32)| self.group_of.get(&(pos.0 + d.0, pos.1 + d.1)).copied();
        let g = id((0, -1)).or(id((0, 1))).or(id((-1, 0))).or(id((1, 0)));
        match g {
            Some(g) => {
                self.groups[g].push(pos);
                self.group_of.insert(pos, g);
            }
            None => {
                self.groups.push(vec![pos]);
                self.group_of.insert(pos, self.groups.len() - 1);
            }
        }
    }

    /// Pass B after a border tile is placed (`0x14209fbc0`): 3×3 tiles touching a shared exit,
    /// and straight passages across one, become door spots.
    fn after_border(&mut self, pos: (i32, i32), key: &TileKey) {
        if !self.on {
            return;
        }
        let both = |m: u8| m & 6 == 6 || m & 0x18 == 0x18;
        if key.size == (3, 3) {
            if (0..3).any(|i| (0..3).any(|j| self.markers.get(&(pos.0 + i, pos.1 + j)).is_some_and(|&m| both(m)))) {
                self.add(pos, key.size);
            }
            return;
        }
        let Some(&m) = self.markers.get(&pos) else { return };
        let (e, g, h) = (&key.edges, &key.grounds, &key.heights);
        let across = m & 0x18 == 0x18 && e[0] == e[2] && e[0].is_some() && e[1].is_none() && e[3].is_none() && g[0] == g[3] && g[1] == g[2] && h[0] == h[3] && h[1] == h[2];
        let down = m & 6 == 6 && e[1] == e[3] && e[1].is_some() && e[0].is_none() && e[2].is_none() && g[1] == g[0] && g[2] == g[3] && h[1] == h[0] && h[2] == h[3];
        if across || down {
            for dy in 0..key.size.1 as i32 {
                for dx in 0..key.size.0 as i32 {
                    self.add((pos.0 + dx, pos.1 + dy), (1, 1));
                }
            }
        }
    }
}

pub struct TilePass<'a> {
    pub master: &'a Master,
    pub ecf: &'a Ecf,
    /// Tiles per room unit (master +0x149) and the shared border (+0x14b).
    pub unit: u32,
    pub overlap: u32,
}

impl TilePass<'_> {
    /// Passes A and B; returns the ground-only cells in fill order (`0x1420d5060`: stable, by
    /// x then y).
    pub fn run(&self, grid: &Grid, rooms: &HashMap<String, Room>, map: &mut TileMap, rng: &mut TerrainRng) -> Result<Vec<GroundCell>, GenError> {
        // `0x142099c80`: door bookkeeping only with `door` tiles.
        let mut doors = Doors { on: self.door_tiles().next().is_some(), ..Default::default() };
        let mut cells: Vec<GroundCell> = Vec::new();
        let mut borders: Vec<((i32, i32), Border)> = Vec::new();
        for p in &grid.rooms {
            if p.path.is_empty() {
                continue;
            }
            let room = rooms.get(&p.path).ok_or_else(|| unsupported(format!("dungeon: room {} not loaded", p.path)))?;
            if doors.on && self.overlap != 0 {
                doors.mark(p, self.unit as i32, self.overlap as i32);
            }
            self.room(p, room, map, &mut borders, &mut cells, rng)?;
        }
        self.borders(borders, map, &mut cells, &mut doors, rng)?;
        if doors.on {
            self.check_doors(&doors, map)?;
        }
        // `0x141f9e780`: the rooms' ground overrides.
        let step = (self.unit - self.overlap) as i32;
        for p in &grid.rooms {
            if let Some(room) = rooms.get(&p.path) {
                map.room_overrides(room, (p.origin.0 * step, p.origin.1 * step), p.orientation);
            }
        }
        cells.sort_by_key(|c| c.pos);
        Ok(cells)
    }

    /// `0x1420983d0`: a room's slots row by row.
    fn room(&self, p: &crate::dungeon::PlacedRoom, room: &Room, map: &mut TileMap, borders: &mut Vec<((i32, i32), Border)>, cells: &mut Vec<GroundCell>, rng: &mut TerrainRng) -> Result<(), GenError> {
        let o = p.orientation;
        let step = (self.unit - self.overlap) as i32;
        let base = (p.origin.0 * step, p.origin.1 * step);
        let (w, h) = (room.width() as i32, room.height() as i32);
        let (tw, th) = if o & 1 == 1 { (h, w) } else { (w, h) };
        for y in 0..h {
            for x in 0..w {
                let slot = &room.arm.grid[y as usize][x as usize];
                if std::env::var("POE_DBG_SLOTS").is_ok() {
                    let (ax, ay) = orient_cell(o, w, h, x, y);
                    eprintln!("DBG slot {} ({x},{y}) -> ({},{}) {:?}", p.path.rsplit('/').next().unwrap_or(""), base.0 + ax, base.1 + ay, match slot { poe_data_tools::file_parsers::arm::types::Slot::K(_) => "K".to_string(), other => format!("{other:?}") });
                }
                // `s` slots (and `f` without a fill name) are ground-only cells without a ground.
                let blank = matches!(slot, poe_data_tools::file_parsers::arm::types::Slot::F { fill: None } | poe_data_tools::file_parsers::arm::types::Slot::S);
                let Some((key, feature)) = cell_key(slot).or_else(|| blank.then(|| (TileKey { size: (1, 1), exits: [3; 4], virtual_exits: [3; 4], ..Default::default() }, String::new()))) else { continue };
                let (sw, sh) = (key.size.0.max(1) as i32, key.size.1.max(1) as i32);
                // Slot +0x73: the corner the slot is listed at (SW, SE, NE, NW).
                let anchor = match slot {
                    poe_data_tools::file_parsers::arm::types::Slot::K(k) => {
                        use poe_data_tools::file_parsers::arm::types::Direction::*;
                        match k.origin {
                            SE => 1,
                            NE => 2,
                            NW => 3,
                            _ => 0,
                        }
                    }
                    _ => 0,
                };
                let x0 = if anchor % 3 != 0 { x - (sw - 1) } else { x };
                let y0 = if anchor > 1 { y - (sh - 1) } else { y };
                let (ax, ay) = orient_cell(o, w, h, x0, y0);
                let (bx, by) = orient_cell(o, w, h, x0 + sw - 1, y0 + sh - 1);
                let (lx, ly) = (ax.min(bx), ay.min(by));
                let mut key = key.oriented(o);
                // Slot +0x72: the lowest corner's height; the corners are kept above it.
                let low = key.heights.iter().map(|&v| v as i8).min().unwrap_or(0);
                for v in &mut key.heights {
                    *v = (*v as i8 - low) as u8;
                }
                let (kw, kh) = (key.size.0 as i32, key.size.1 as i32);
                let xflag = if lx < 1 { 1 } else if lx + kw >= tw { 2 } else { 0 };
                let yflag = if ly < 1 { 1 } else if ly + kh >= th { 2 } else { 0 };
                let pos = (base.0 + lx, base.1 + ly);
                let height = p.height + low as i32;
                let flat = feature.is_empty() && key.heights.iter().all(|&v| v == 0) && key.edges.iter().all(Option::is_none) && key.grounds.iter().all(|g| *g == key.grounds[0]);
                if flat {
                    mark_ground(map, cells, pos, &key, height, p.accessible);
                    continue;
                }
                if self.on_border(&feature, &key, (xflag, yflag), (lx, ly), (tw, th)) {
                    let mut flags = p.accessible as u8;
                    if xflag != 2 {
                        flags |= 0x10;
                    }
                    if xflag != 1 {
                        flags |= 8;
                    }
                    if yflag != 2 {
                        flags |= 2;
                    }
                    if yflag != 1 {
                        flags |= 4;
                    }
                    self.add_border(map, borders, pos, key, height, flags, feature, (xflag, yflag))?;
                    continue;
                }
                let list = self.master.tiles.candidates(&feature, &key);
                if list.is_empty() {
                    return Err(unsupported(format!("dungeon: no tile for {} slot ({x},{y}) at {pos:?} feature {feature:?} key {key:?}", p.path)));
                }
                let total = list.iter().map(|c| c.weight).sum();
                if std::env::var("POE_DBG_KEYS").is_ok() {
                    eprintln!("DBG key {pos:?} total {total} {} slot ({x},{y}) o{o} {key:?}", p.path.rsplit('/').next().unwrap_or(""));
                }
                let c = pick(list, rng.below(total)).expect("non-empty list");
                let def = &self.master.tiles.tiles[c.tile];
                map.place(pos.0, pos.1, &def.path, &def.tdt, c.orientation, height as i16, Some(c.tile));
                map.set_heights(pos.0, pos.1, &def.tdt, height as i16);
            }
        }
        Ok(())
    }

    /// `0x142099b90`: an odd-sized slot centred on the room's border line (with a shared
    /// border).
    fn on_border(&self, feature: &str, key: &TileKey, (xf, yf): (i32, i32), (lx, ly): (i32, i32), (tw, th): (i32, i32)) -> bool {
        if self.overlap == 0 || feature.eq_ignore_ascii_case("missiontile") {
            return false;
        }
        let (kw, kh) = (key.size.0 as i32, key.size.1 as i32);
        if (xf == 1 && kw & 1 == 1 && (kw >> 1) + lx == 0) || (xf == 2 && kw & 1 == 1 && (kw >> 1) + lx == tw - 1) {
            return true;
        }
        match yf {
            1 => kh & 1 == 1 && (kh >> 1) + ly == 0,
            2 => kh & 1 == 1 && (kh >> 1) + ly == th - 1,
            _ => false,
        }
    }

    /// `0x142097f80`.
    #[allow(clippy::too_many_arguments)]
    fn add_border(&self, map: &TileMap, borders: &mut Vec<((i32, i32), Border)>, pos: (i32, i32), key: TileKey, height: i32, flags: u8, feature: String, (xf, yf): (i32, i32)) -> Result<(), GenError> {
        let at = borders.binary_search_by_key(&pos, |b| b.0);
        let e = match at {
            Err(i) => {
                borders.insert(i, (pos, Border { key, height, seen: flags, merged: 0, names: Vec::new() }));
                &mut borders[i].1
            }
            Ok(i) => {
                let e = &mut borders[i].1;
                let mut merged = e.merged;
                let mut out = merge(map, self.ecf, &key, &e.key, height, e.height, flags, e.seen, &mut merged)?;
                let m = out.heights.iter().map(|&v| v as i8).min().unwrap_or(0);
                for v in &mut out.heights {
                    *v = (*v as i8 - m) as u8;
                }
                e.key = out;
                e.height += m as i32;
                e.merged = merged;
                e.seen |= flags;
                e
            }
        };
        if !feature.is_empty() && !e.names.contains(&feature) {
            e.names.push(feature);
        }
        e.merged |= match (xf, yf) {
            (1, 0) => 0x10,
            (2, 0) => 8,
            (0, 1) => 2,
            (0, 2) => 4,
            _ => 0,
        };
        Ok(())
    }

    /// `0x14209fbc0`: the shared-border slots in position order.
    fn borders(&self, borders: Vec<((i32, i32), Border)>, map: &mut TileMap, cells: &mut Vec<GroundCell>, doors: &mut Doors, rng: &mut TerrainRng) -> Result<(), GenError> {
        for (pos, mut e) in borders {
            if e.names.is_empty() && uniform(map, &e.key) {
                mark_ground(map, cells, pos, &e.key, e.height, e.seen & 1 != 0);
                continue;
            }
            let (w, h) = (e.key.size.0 as i32, e.key.size.1 as i32);
            let taken = (0..h).flat_map(|dy| (0..w).map(move |dx| (dx, dy))).filter(|&(dx, dy)| map.index(pos.0 + dx, pos.1 + dy).is_some_and(|i| map.piece_at[i].is_some())).count() as i32;
            if taken != 0 {
                return Err(unsupported(format!("dungeon: border tile at {pos:?} over a placed tile (not ported)")));
            }
            if e.merged & 0x1e != 0x1e {
                // Sides only one room gave: their `.gt` flag 0x10 grounds are dropped.
                let only = !e.merged & e.seen & 0x1e;
                let mut a = e.key.clone();
                a.edges = Default::default();
                let mut merged = e.merged;
                e.key = merge(map, self.ecf, &a, &e.key, e.height, e.height, only, e.seen, &mut merged)?;
                e.merged = merged;
            }
            if !e.names.is_empty() {
                return Err(unsupported(format!("dungeon: named border tile at {pos:?} (not ported)")));
            }
            if uniform(map, &e.key) {
                mark_ground(map, cells, pos, &e.key, e.height, e.seen & 1 != 0);
                continue;
            }
            // `0x14209f050`.
            let list = self.master.tiles.candidates("", &e.key);
            if list.is_empty() {
                return Err(unsupported(format!("dungeon: no border tile at {pos:?} for {:?}", e.key)));
            }
            let total = list.iter().map(|c| c.weight).sum();
            let c = pick(list, rng.below(total)).expect("non-empty list");
            let def = &self.master.tiles.tiles[c.tile];
            map.place(pos.0, pos.1, &def.path, &def.tdt, c.orientation, e.height as i16, Some(c.tile));
            map.set_heights(pos.0, pos.1, &def.tdt, e.height as i16);
            doors.after_border(pos, &e.key);
        }
        Ok(())
    }

    fn door_tiles(&self) -> impl Iterator<Item = &crate::tileset::TileDef> {
        self.master.tiles.tiles.iter().filter(|t| t.weight != 0 && t.tdt.feature.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("door")))
    }

    /// The oriented key of the tile-set tile covering `pos` (None for fill tiles).
    fn key_at(&self, map: &TileMap, pos: (i32, i32)) -> Option<(TileKey, bool)> {
        let piece = &map.pieces[map.piece_at[map.index(pos.0, pos.1)?]?];
        let def = &self.master.tiles.tiles[piece.tile?];
        Some((def.key.oriented(piece.orientation), def.tdt.feature.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("door"))))
    }

    /// `0x14209d180` / `0x14209dc90` are not ported: fail when a door tile could go on a door
    /// spot (`0x141d09700`: a `door` tile with the spot tile's edges, grounds and heights).
    fn check_doors(&self, doors: &Doors, map: &TileMap) -> Result<(), GenError> {
        let fits = |k: &TileKey| {
            self.door_tiles().any(|t| (0..8u8).filter(|o| t.orientations >> o & 1 != 0).any(|o| {
                let d = t.key.oriented(o);
                d.edges == k.edges && d.grounds == k.grounds && d.heights == k.heights
            }))
        };
        for g in &doors.groups {
            if g.iter().any(|&p| self.key_at(map, p).is_some_and(|k| k.1)) {
                continue;
            }
            if g.iter().filter_map(|&p| self.key_at(map, p)).any(|k| fits(&k.0)) {
                return Err(unsupported(format!("dungeon: door group at {:?} (0x14209d180 not ported)", g[0])));
            }
        }
        for &p in &doors.large {
            if self.key_at(map, p).is_some_and(|k| !k.1 && self.master.tiles.candidates("door", &k.0).iter().next().is_some()) {
                return Err(unsupported(format!("dungeon: door tile at {p:?} (0x14209dc90 not ported)")));
            }
        }
        Ok(())
    }
}
