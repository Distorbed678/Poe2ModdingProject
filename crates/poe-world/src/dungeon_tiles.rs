//! The dungeon tile pass (`0x14209c4c0`): every placed room's `.arm` slots become tiles
//! (`0x1420983d0`, pass A), and the slots on the one-tile borders rooms share are merged with
//! the neighbour's and tiled afterwards (`0x14209fbc0`, pass B).

use std::collections::HashMap;

use crate::{
    dungeon::Grid,
    generate::{GenError, Master, Room, TileMap, cell_key},
    rng::TerrainRng,
    tileset::{Candidate, TileKey, orient_cell, pick},
};

fn unsupported(s: impl Into<String>) -> GenError {
    GenError::Unsupported(s.into())
}

/// The edge-type combiner (`.ecf`, master `EdgeCombination`): two edge types meeting on a
/// shared border, with the corner height difference along it, give one edge type.
#[derive(Default)]
pub struct Ecf {
    /// Edge pair, result, and the height difference (`None`: a rule without a number).
    rules: Vec<(Option<String>, Option<String>, Option<String>, Option<i32>)>,
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
            let dh = parts[6].trim().parse().ok();
            rules.push((p(parts[1]), p(parts[3]), p(parts[5]), dh));
        }
        Self { rules }
    }

    /// `0x1420a28c0`: the rule for the pair (either order) and height difference, then one
    /// without a height difference; without one, a lone edge stays and two edges give none.
    fn combine(&self, a: &Option<String>, b: &Option<String>, dh: i32) -> Option<String> {
        let pair = |r: &&(Option<String>, Option<String>, Option<String>, Option<i32>)| (&r.0 == a && &r.1 == b) || (&r.0 == b && &r.1 == a);
        if let Some(r) = self.rules.iter().filter(pair).find(|r| r.3 == Some(dh)).or_else(|| self.rules.iter().filter(pair).find(|r| r.3.is_none())) {
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
    /// Edge types by path (lower case), for the exit wiggle's records.
    pub edge_types: &'a HashMap<String, crate::rooms::EdgeType>,
    /// `.tsi` `AllowInteriorOverlapFixes` (master +0x162): border exits may move to the middle
    /// on any side, not only towards another border entry.
    pub overlap_fixes: bool,
    /// `.tsi` `DisableDoors` (master +0x167): no door placement.
    pub doors_disabled: bool,
}

impl TilePass<'_> {
    /// Passes A and B; returns the ground-only cells in fill order (`0x1420d5060`: stable, by
    /// x then y).
    pub fn run(&self, grid: &Grid, rooms: &HashMap<String, Room>, map: &mut TileMap, rng: &mut TerrainRng) -> Result<(Vec<GroundCell>, Vec<(i32, i32)>), GenError> {
        let mut moved = Vec::new();
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
            self.room(p, room, map, &mut borders, &mut cells, &mut moved, rng)?;
        }
        self.borders(borders, map, &mut cells, &mut doors, rng)?;
        if doors.on && !self.doors_disabled {
            self.check_doors(&doors, map, rng)?;
        }
        // `0x141f9e780`: the rooms' ground overrides.
        let step = (self.unit - self.overlap) as i32;
        for p in &grid.rooms {
            if let Some(room) = rooms.get(&p.path) {
                map.room_overrides(room, (p.origin.0 * step, p.origin.1 * step), p.orientation);
            }
        }
        cells.sort_by_key(|c| c.pos);
        Ok((cells, moved))
    }

    /// `0x1420983d0`: a room's slots row by row.
    #[allow(clippy::too_many_arguments)]
    fn room(&self, p: &crate::dungeon::PlacedRoom, room: &Room, map: &mut TileMap, borders: &mut Vec<((i32, i32), Border)>, cells: &mut Vec<GroundCell>, moved: &mut Vec<(i32, i32)>, rng: &mut TerrainRng) -> Result<(), GenError> {
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
                let border = self.on_border(&feature, &key, (xflag, yflag), (lx, ly), (tw, th));
                // `0x142098b18`: square unnamed slots whose edges all have its size, with an edge
                // of wiggle 2 or more (1 on the border line), are recorded for the exit wiggle.
                if feature.is_empty() && key.size.0 == key.size.1 {
                    let need = if border { 1 } else { 2 };
                    let mut ok = false;
                    let mut fits = true;
                    for e in key.edges.iter().flatten() {
                        let Some(et) = self.edge_types.get(e) else { continue };
                        if et.size != key.size.0 {
                            fits = false;
                            break;
                        }
                        ok |= et.wiggle >= need;
                    }
                    if fits && ok {
                        moved.push(pos);
                    }
                }
                if border {
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
    fn borders(&self, mut borders: Vec<((i32, i32), Border)>, map: &mut TileMap, cells: &mut Vec<GroundCell>, doors: &mut Doors, rng: &mut TerrainRng) -> Result<(), GenError> {
        for idx in 0..borders.len() {
            let (pos, mut e) = borders[idx].clone();
            let r = self.border(&mut borders, pos, &mut e, map, cells, doors, rng);
            borders[idx].1 = e;
            r?;
        }
        Ok(())
    }

    /// One entry of `0x14209fbc0`.
    #[allow(clippy::too_many_arguments)]
    fn border(&self, borders: &mut [((i32, i32), Border)], pos: (i32, i32), e: &mut Border, map: &mut TileMap, cells: &mut Vec<GroundCell>, doors: &mut Doors, rng: &mut TerrainRng) -> Result<(), GenError> {
        {
            if e.names.is_empty() && uniform(map, &e.key) {
                mark_ground(map, cells, pos, &e.key, e.height, e.seen & 1 != 0);
                return Ok(());
            }
            let (w, h) = (e.key.size.0 as i32, e.key.size.1 as i32);
            let taken = (0..h).flat_map(|dy| (0..w).map(move |dx| (dx, dy))).filter(|&(dx, dy)| map.index(pos.0 + dx, pos.1 + dy).is_some_and(|i| map.piece_at[i].is_some())).count() as i32;
            if taken != 0 && taken != w * h {
                return Err(unsupported(format!("dungeon: overlap tile at {pos:?} partially over placed tiles")));
            }
            let placed = map.index(pos.0, pos.1).and_then(|i| map.cells[i].as_ref().map(|c| (map.piece_at[i], c.height)));
            match placed {
                None => {
                    if e.merged & 0x1e != 0x1e {
                        // Sides only one room gave: their `.gt` flag 0x10 grounds are dropped.
                        let only = !e.merged & e.seen & 0x1e;
                        let mut a = e.key.clone();
                        a.edges = Default::default();
                        let mut merged = e.merged;
                        e.key = merge(map, self.ecf, &a, &e.key, e.height, e.height, only, e.seen, &mut merged)?;
                        e.merged = merged;
                    }
                }
                Some((piece, height)) => {
                    // A tile is already there: its key counts for the sides no room gave.
                    let tile = piece.and_then(|p| Some((map.pieces[p].tile?, map.pieces[p].orientation)));
                    let Some((t, o)) = tile else {
                        return Err(unsupported(format!("dungeon: overlap tile at {pos:?} over a fill tile")));
                    };
                    let k = self.master.tiles.tiles[t].key.oriented(o);
                    let other = match e.seen & 0x1e {
                        0x1c => 0x1a,
                        0x1a => 0x1c,
                        0x16 => 0x0e,
                        0x0e => 0x16,
                        _ => 0x1e,
                    };
                    let mut merged = e.merged;
                    let mut out = merge(map, self.ecf, &e.key, &k, e.height, height as i32, e.seen, other, &mut merged)?;
                    let m = out.heights.iter().map(|&v| v as i8).min().unwrap_or(0);
                    for v in &mut out.heights {
                        *v = (*v as i8 - m) as u8;
                    }
                    e.key = out;
                    e.height += m as i32;
                    e.merged = merged;
                    e.seen |= other;
                }
            }
            // Named slots: a name drawn by its tiles' total weight (1 without tiles) until one
            // places.
            if !e.names.is_empty() {
                let mut names: Vec<(String, u32)> = e
                    .names
                    .iter()
                    .map(|n| {
                        let list = self.master.tiles.candidates(n, &e.key);
                        (n.clone(), if list.is_empty() { 1 } else { list.iter().map(|c| c.weight).sum() })
                    })
                    .filter(|n| n.1 != 0)
                    .collect();
                let mut total: u32 = names.iter().map(|n| n.1).sum();
                let mut done = false;
                while !names.is_empty() {
                    let mut r = rng.below(total);
                    let mut k = names.len() - 1;
                    for (j, n) in names.iter().enumerate() {
                        if r < n.1 {
                            k = j;
                            break;
                        }
                        r -= n.1;
                    }
                    if self.place_border(&e.key, &names[k].0, pos, e.height, map, rng, Some(&mut *borders))? {
                        if e.key.size == (1, 1) && names[k].0.eq_ignore_ascii_case("door") {
                            doors.add(pos, (1, 1));
                        }
                        done = true;
                        break;
                    }
                    let name = names[k].0.clone();
                    if let Some(j) = names.iter().position(|n| n.0 == name) {
                        total -= names[j].1;
                        names.remove(j);
                    }
                }
                if done {
                    return Ok(());
                }
            }
            if uniform(map, &e.key) {
                mark_ground(map, cells, pos, &e.key, e.height, e.seen & 1 != 0);
                return Ok(());
            }
            if !self.place_border(&e.key, "", pos, e.height, map, rng, Some(&mut *borders))? {
                return Err(unsupported(format!("dungeon: no border tile at {pos:?} for {:?}", e.key)));
            }
            e.seen |= 0x20;
            doors.after_border(pos, &e.key);
        }
        Ok(())
    }

    /// `0x14209f050`: the tile-set tiles of `feature` with `key`, `below(total)`, placed at
    /// `height`. Without tiles (and with `borders`, i.e. not for a neighbour's repair), sides
    /// whose exit is off the middle and that face another border entry move it to the middle
    /// one by one until tiles exist (`0x14209fa10`); the neighbours across moved sides then
    /// follow (`0x14209ec70`). The height fallback is not ported.
    #[allow(clippy::too_many_arguments)]
    fn place_border(&self, key: &TileKey, feature: &str, pos: (i32, i32), height: i32, map: &mut TileMap, rng: &mut TerrainRng, borders: Option<&mut [((i32, i32), Border)]>) -> Result<bool, GenError> {
        let lookup = |k: &TileKey| self.master.tiles.candidates(feature, k).to_vec();
        // Sides whose exit may move to the middle (only for the slot itself, not a repair).
        let mut sides = Vec::new();
        if let Some(b) = borders.as_deref() {
            for i in 0..4 {
                let Some(edge) = &key.edges[i] else { continue };
                let Some(et) = self.edge_types.get(edge) else { continue };
                if et.virtual_edge || et.centred || key.exits[i] == Self::mid(key, i) {
                    continue;
                }
                let n = self.across(key, pos, i);
                if n != pos && (self.overlap_fixes || b.binary_search_by_key(&(n.0 * 0x10000 + n.1), |e| e.0.0 * 0x10000 + e.0.1).is_ok()) {
                    sides.push(i);
                }
            }
        }
        // `0x14209fa10`: the sides take the middle exit one by one until tiles exist.
        let middle = |k: &mut TileKey, changed: &mut Vec<usize>| -> Vec<Candidate> {
            for &i in &sides {
                k.exits[i] = Self::mid(k, i);
                changed.push(i);
                let l = lookup(k);
                if !l.is_empty() {
                    return l;
                }
            }
            Vec::new()
        };
        let takes_height = |g: &Option<String>| g.as_ref().is_none_or(|g| map.gt_flags(g) & 1 != 0);
        // `0x14209fb00`: one corner whose ground takes no height drops it.
        let flatten_one = |k: &mut TileKey| -> Vec<Candidate> {
            for i in 0..4 {
                if takes_height(&k.grounds[i]) {
                    continue;
                }
                let old = k.heights[i];
                k.heights[i] = 0;
                let l = lookup(k);
                if !l.is_empty() {
                    return l;
                }
                k.heights[i] = old;
            }
            Vec::new()
        };
        let mut height = height;
        let mut k = key.clone();
        let mut changed: Vec<usize> = Vec::new();
        let mut list = lookup(&k);
        if list.is_empty() && !sides.is_empty() {
            list = middle(&mut k, &mut changed);
        }
        if list.is_empty() && key.heights.iter().any(|&h| h != 0) {
            k = key.clone();
            changed.clear();
            list = flatten_one(&mut k);
            if list.is_empty() && !sides.is_empty() {
                list = middle(&mut k, &mut changed);
                if list.is_empty() {
                    list = flatten_one(&mut k);
                }
                if list.is_empty() {
                    k = key.clone();
                    changed.clear();
                }
            }
            if list.is_empty() {
                // Corners whose ground takes no height drop it; the others' heights.
                let mut hs: Vec<i8> = Vec::new();
                for i in 0..4 {
                    if takes_height(&k.grounds[i]) {
                        hs.push(k.heights[i] as i8);
                    } else {
                        k.heights[i] = 0;
                    }
                }
                list = lookup(&k);
                if list.is_empty() && !sides.is_empty() {
                    list = middle(&mut k, &mut changed);
                }
                if list.is_empty() {
                    hs.sort();
                    hs.dedup();
                    list = lookup(&k);
                    // One height left: the tile is flat at that height.
                    if list.is_empty() && hs.len() == 1 {
                        k.heights = [0; 4];
                        height += hs[0] as i32;
                        list = lookup(&k);
                    }
                }
            }
        }
        if list.is_empty() {
            return Ok(false);
        }
        let total = list.iter().map(|c| c.weight).sum();
        let c = pick(&list, rng.below(total)).expect("non-empty list");
        let def = &self.master.tiles.tiles[c.tile];
        map.place(pos.0, pos.1, &def.path, &def.tdt, c.orientation, height as i16, Some(c.tile));
        map.set_heights(pos.0, pos.1, &def.tdt, height as i16);
        if let Some(b) = borders {
            self.fix_across(&k, pos, &changed, b, map, rng)?;
        }
        Ok(true)
    }

    /// The middle exit of side `i` (`size · 3 / 2`).
    fn mid(k: &TileKey, i: usize) -> u8 {
        (if i & 1 == 0 { k.size.0 } else { k.size.1 } as u32 * 3 / 2) as u8
    }

    /// `0x14209eb70`: the cell across side `i` of a tile at `pos`.
    fn across(&self, k: &TileKey, pos: (i32, i32), i: usize) -> (i32, i32) {
        let esize = |i: usize| k.edges[i].as_ref().and_then(|e| self.edge_types.get(e)).map_or(0, |et| et.size as i32);
        match i {
            0 => (pos.0, pos.1 - esize(0)),
            1 => (pos.0 + k.size.0 as i32, pos.1),
            2 => (pos.0, pos.1 + k.size.1 as i32),
            _ => (pos.0 - esize(3), pos.1),
        }
    }

    /// `0x14209ec70`: across each moved side, a placed tile with the same edge and side length
    /// moves its exit to the middle too and is drawn again; a border entry still to come takes
    /// the middle exit in its key.
    fn fix_across(&self, k: &TileKey, pos: (i32, i32), changed: &[usize], borders: &mut [((i32, i32), Border)], map: &mut TileMap, rng: &mut TerrainRng) -> Result<(), GenError> {
        for &u in changed {
            let Some(edge) = k.edges[u].clone() else { continue };
            let n = self.across(k, pos, u);
            if n.0 < 0 || n.1 < 0 || n.0 as u32 >= map.width || n.1 as u32 >= map.height {
                continue;
            }
            let o = (u + 2) & 3;
            let len = if u & 1 == 0 { k.size.0 } else { k.size.1 };
            let side_len = |k: &TileKey| if o & 1 == 0 { k.size.0 } else { k.size.1 };
            let entry = borders.binary_search_by_key(&(n.0 * 0x10000 + n.1), |e| e.0.0 * 0x10000 + e.0.1).ok();
            match entry {
                Some(j) if borders[j].1.seen & 0x20 == 0 => {
                    let e = &mut borders[j].1;
                    if e.key.edges[o].as_deref() == Some(edge.as_str()) && side_len(&e.key) == len {
                        e.key.exits[o] = Self::mid(&e.key, o);
                    }
                }
                _ => {
                    let Some(i) = map.index(n.0, n.1) else { continue };
                    let Some(cell) = map.cells[i].clone() else { continue };
                    let Some(t) = map.piece_at[i].and_then(|p| map.pieces[p].tile) else { continue };
                    let def = &self.master.tiles.tiles[t];
                    let mut nk = def.key.oriented(cell.orientation);
                    if side_len(&nk) != len || nk.edges[o].as_deref() != Some(edge.as_str()) {
                        continue;
                    }
                    let mid = Self::mid(&nk, o);
                    if nk.exits[o] == mid {
                        continue;
                    }
                    nk.exits[o] = mid;
                    let feature = def.tdt.feature.clone().unwrap_or_default();
                    if !self.place_border(&nk, &feature, n, cell.height as i32, map, rng, None)? {
                        return Err(unsupported(format!("dungeon: could not find matching adjacent tile at {n:?}")));
                    }
                }
            }
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

    /// The door sizes for a spot's edges (`0x141d09700`; the `+0x68` sets `0x141d09ab0`
    /// builds): every `door` tile orientation with those edges, its oriented size, sorted.
    fn door_sizes(&self, edges: &[Option<String>; 4]) -> Vec<(u8, u8)> {
        let mut out: Vec<(u8, u8)> = Vec::new();
        let doors = self.master.tiles.tiles.iter().filter(|t| t.tdt.feature.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("door")));
        for t in doors {
            for o in (0..8u8).filter(|o| t.orientations >> o & 1 != 0) {
                if &t.key.oriented(o).edges == edges {
                    let s = if o & 1 == 0 { t.key.size } else { (t.key.size.1, t.key.size.0) };
                    if let Err(i) = out.binary_search(&s) {
                        out.insert(i, s);
                    }
                }
            }
        }
        out
    }

    /// `0x14209d180`: a door tile on every door group without one, and `0x14209dc90` (not
    /// ported) for the 3×3 spots.
    fn check_doors(&self, doors: &Doors, map: &mut TileMap, rng: &mut TerrainRng) -> Result<(), GenError> {
        for g in &doors.groups {
            if g.iter().any(|&p| self.key_at(map, p).is_some_and(|k| k.1)) {
                continue;
            }
            // Vertical when the first two cells share x, or every cell is on a left and right
            // border; the anchor is the first cell with the lowest coordinate along the group.
            let vertical = (g.len() >= 2 && g[0].0 == g[1].0) || g.iter().all(|p| doors.markers.get(p).is_some_and(|&m| m & 0x18 == 0x18));
            let along = |p: (i32, i32)| if vertical { p.1 } else { p.0 };
            let mut anchor = 0;
            for i in 0..g.len() {
                if along(g[i]) < along(g[anchor]) {
                    anchor = i;
                }
            }
            let (w, h) = if vertical { (1, g.len() as u32) } else { (g.len() as u32, 1) };
            let Some((key, _)) = self.key_at(map, g[anchor]) else { continue };
            let sizes = self.door_sizes(&key.edges);
            if sizes.is_empty() {
                continue;
            }
            let mut fit: Vec<(u8, u8)> = sizes.into_iter().filter(|s| s.0 as u32 <= w && s.1 as u32 <= h).collect();
            if fit.is_empty() {
                return Err(unsupported(format!("dungeon: no door fits {w}x{h} at {:?}", g[anchor])));
            }
            rng.below(fit.len() as u32);
            let n = fit.len();
            for i in 0..n {
                let k = rng.uniform((n - i) as u32) as usize;
                fit.swap(i, i + k);
            }
            let mut found = None;
            for s in fit {
                let mut k = key.clone();
                k.size = s;
                let v = [s.0 * 3, s.1 * 3, s.0 * 3, s.1 * 3];
                for i in 0..4 {
                    if k.exits[i] == 3 {
                        k.exits[i] = v[i];
                    }
                }
                k.virtual_exits = v;
                let list = self.master.tiles.candidates("door", &k);
                if !list.is_empty() {
                    found = Some((s, list));
                    break;
                }
            }
            let Some((s, list)) = found else {
                return Err(unsupported(format!("dungeon: no matching door tile at {:?}", g[anchor])));
            };
            let span = if vertical { s.1 } else { s.0 } as u32;
            let (unit, overlap) = (self.unit, self.overlap);
            let centre = (span == 1 && unit & 1 != 0)
                .then(|| g.iter().position(|&p| along(p).rem_euclid((unit - overlap) as i32) == (unit >> 1) as i32))
                .flatten();
            let off = match centre {
                Some(i) => i as i32,
                None => rng.below(g.len() as u32 - span + 1) as i32,
            };
            let pos = if vertical { (g[anchor].0, g[anchor].1 + off) } else { (g[anchor].0 + off, g[anchor].1) };
            let total = list.iter().map(|c| c.weight).sum();
            let c = pick(list, rng.below(total)).expect("non-empty list");
            let def = &self.master.tiles.tiles[c.tile];
            let height = map.index(pos.0, pos.1).and_then(|i| map.cells[i].as_ref()).map_or(0, |c| c.height);
            map.place(pos.0, pos.1, &def.path, &def.tdt, c.orientation, height, Some(c.tile));
        }
        for &p in &doors.large {
            if self.key_at(map, p).is_some_and(|k| !k.1 && self.master.tiles.candidates("door", &k.0).iter().next().is_some()) {
                return Err(unsupported(format!("dungeon: door tile at {p:?} (0x14209dc90 not ported)")));
            }
        }
        Ok(())
    }
}

/// Edge types of the exit wiggle, loaded on demand.
#[derive(Default)]
pub struct Wiggles {
    edges: HashMap<String, crate::rooms::EdgeType>,
}

impl Wiggles {
    fn edge(&mut self, files: &dyn crate::generate::Files, path: &str) -> Result<&crate::rooms::EdgeType, GenError> {
        let key = path.to_ascii_lowercase();
        if !self.edges.contains_key(&key) {
            let et = crate::rooms::EdgeType::parse(&crate::text::decode_utf16(&files.read(&key)?));
            self.edges.insert(key.clone(), et);
        }
        Ok(&self.edges[&key])
    }

    /// `0x14209a2c0` / `0x1420969e0`: a square tile whose edges all have the tile's size and
    /// one of them takes hashed exits.
    pub fn movable(&mut self, files: &dyn crate::generate::Files, tdt: &crate::tdt::Tdt) -> Result<bool, GenError> {
        if tdt.size.0 != tdt.size.1 || tdt.feature.as_deref().is_some_and(|f| !f.is_empty()) {
            return Ok(false);
        }
        let mut hashed = false;
        for e in tdt.edges.iter().flatten().filter(|e| !e.is_empty()) {
            let et = self.edge(files, e)?;
            if et.size != tdt.size.0 {
                return Ok(false);
            }
            hashed |= et.hashed;
        }
        Ok(hashed)
    }
}

/// `0x1421969e0`: the hashed exit of side `side` (0 top, 1 right, 2 bottom, 3 left) at `(x, y)`
/// for an edge of size `size`.
pub fn hashed_exit(x: i32, y: i32, side: u32, size: u32) -> u8 {
    let (x, y) = (x as u32, y as u32);
    let k = match side {
        1 => {
            let v = y.wrapping_mul(0x7bb).wrapping_add(0x31e9).wrapping_add(x.wrapping_add(size).wrapping_mul(0x337)) % 0xd;
            ((v / 3 * 3).wrapping_sub(v).wrapping_add(2) & 0xff) * (size & 0xff)
        }
        2 => {
            let v = y.wrapping_add(size).wrapping_mul(0x7bb).wrapping_add(x.wrapping_mul(0x337)) % 0xd;
            ((v / 3 * 3).wrapping_sub(v).wrapping_add(2) & 0xff) * (size & 0xff)
        }
        _ => (y.wrapping_mul(0x7bb).wrapping_add(side.wrapping_mul(0x10a3)).wrapping_add(x.wrapping_mul(0x337)) % 0xd % 3) * (size & 0xff),
    };
    (k as u8).wrapping_add((size >> 1) as u8)
}

/// `0x1420a1f70` (after the fill): the recorded tiles, shuffled, each try their sides in a
/// shuffled order; a side with a hashed edge whose hashed exit differs from the tile's takes
/// it when tiles exist for both this key and the neighbour across (its opposite exit mirrored);
/// then both are drawn again and placed.
pub fn wiggle(master: &mut Master, files: &dyn crate::generate::Files, map: &mut TileMap, mut list: Vec<(i32, i32)>, rng: &mut TerrainRng) -> Result<(), GenError> {
    if list.is_empty() {
        return Ok(());
    }
    let mut w = Wiggles::default();
    // `0x141fff9b0`: a set ordered by x·65536 + y.
    list.sort_by_key(|p| p.0.wrapping_mul(0x10000).wrapping_add(p.1));
    list.dedup();
    rng.below(list.len() as u32);
    let n = list.len();
    for i in 0..n {
        let k = rng.uniform((n - i) as u32) as usize;
        list.swap(i, i + k);
    }
    const DIRS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    // The tile at a cell, when it is a movable tile's corner cell: its path, height and key.
    let at = |map: &TileMap, master: &mut Master, w: &mut Wiggles, p: (i32, i32)| -> Result<Option<(i16, TileKey)>, GenError> {
        let Some(c) = map.cell_or_spill(p.0, p.1).cloned() else { return Ok(None) };
        if c.offset != (0, 0) {
            return Ok(None);
        }
        let tdt = master.header(files, &c.path)?.clone();
        if !w.movable(files, &tdt)? {
            return Ok(None);
        }
        // The indexed key (`0x141d09900`): grounds with `.gt` flag 0x10 left out.
        let mut k = TileKey::from_tdt(&tdt);
        for g in &mut k.grounds {
            if g.as_ref().is_some_and(|g| map.gt_flags(g) & 0x10 != 0) {
                *g = None;
            }
        }
        Ok(Some((c.height, k.oriented(c.orientation))))
    };
    for p in list {
        let Some((height, mut key)) = at(map, master, &mut w, p)? else {
            if std::env::var("POE_DBG_WIGGLE").is_ok() {
                let c = map.index(p.0, p.1).and_then(|i| map.cells[i].clone());
                eprintln!("DBG wskip {p:?} {:?}", c.map(|c| (c.path, c.offset)));
            }
            continue;
        };
        let mut sides = [0u32, 1, 2, 3];
        rng.below(4);
        for i in 0..4 {
            let k = rng.uniform(4 - i as u32) as usize;
            sides.swap(i, i + k);
        }
        // Every side whose move both tiles allow keeps it; the tile takes the last list.
        let mut own: Option<Vec<Candidate>> = None;
        let mut across: [Option<((i32, i32), i16, Vec<Candidate>)>; 4] = Default::default();
        for s in sides {
            let Some(e) = key.edges[s as usize].clone() else { continue };
            let et = w.edge(files, &e)?;
            if !et.hashed {
                continue;
            }
            let size = et.size as u32;
            let new = hashed_exit(p.0, p.1, s, size);
            if new == key.exits[s as usize] {
                continue;
            }
            let d = DIRS[s as usize];
            let q = (p.0 + size as i32 * d.0, p.1 + size as i32 * d.1);
            if q.0 < 0 || q.1 < 0 || q.0 as u32 >= map.width || q.1 as u32 >= map.height {
                continue;
            }
            let Some((qh, mut qkey)) = at(map, master, &mut w, q)? else { continue };
            let old = key.exits[s as usize];
            key.exits[s as usize] = new;
            let list = master.tiles.candidates("", &key).to_vec();
            if !list.is_empty() {
                let o = ((s + 2) & 3) as usize;
                qkey.exits[o] = (size as u8 * 3).wrapping_sub(new).wrapping_sub(1);
                let qlist = master.tiles.candidates("", &qkey).to_vec();
                if std::env::var("POE_DBG_WIGGLE").is_ok() {
                    eprintln!("DBG wiggle {p:?} side {s} own {} {key:?}\nDBG   across {q:?} {} {qkey:?}", list.len(), qlist.len());
                }
                let ok = !qlist.is_empty();
                across[s as usize] = Some((q, qh, qlist));
                if ok {
                    own = Some(list);
                    continue;
                }
            }
            key.exits[s as usize] = old;
        }
        let Some(list) = own else { continue };
        let mut places = vec![(list, p, height)];
        places.extend(across.into_iter().flatten().filter(|a| !a.2.is_empty()).map(|(q, qh, l)| (l, q, qh)));
        for (list, pos, h) in places {
            let total = list.iter().map(|c| c.weight).sum();
            let c = pick(&list, rng.below(total)).expect("non-empty list");
            let def = &master.tiles.tiles[c.tile];
            if std::env::var("POE_DBG_WIGGLE").is_ok() {
                eprintln!("DBG wplace {} {} o{} h{}\t{}", pos.0, pos.1, c.orientation, h, def.path);
            }
            map.place(pos.0, pos.1, &def.path, &def.tdt, c.orientation, h, Some(c.tile));
        }
    }
    Ok(())
}

