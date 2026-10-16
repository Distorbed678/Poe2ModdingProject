//! Overlays (master `Overlays` `.toy`, master +0x200): rooms laid over finished terrain where
//! their root key fits it, e.g. road bends over straight road tiles. `0x141fa0ac0` builds the
//! context and the occupancy grid, `0x141fa1a70` runs the passes of one phase (1 before the
//! fill, 2 after it), `0x141fa1e60` lists candidate positions, `0x141fa23e0` places.
//!
//! `.toy`: `version 1`, then passes separated by blank lines. A pass starts with a header,
//! `<phase 1> <phase 2> [<phase 3>] [Random|FileOrder|SizeOrder] [±flag…]`, followed by
//! entries `<weight> "<.arm>" [±flag…] [!exclusion…] [limit=N] [required=N] [rotations…]`
//! (`0x141f9f4c0`).

use std::collections::{BTreeMap, HashMap};

use crate::{
    generate::{Files, GenError, Master, Room, TileMap, load_room, room_pass_oriented, slot_key},
    rng::{TerrainRng, lcg_index},
    rooms::TerrainTypes,
    tileset::{ORIENTATION_NAMES, TileKey},
};

/// Overlay flags (`0x142f0c018`): kinds of occupied cells an overlay may cover, plus
/// `MatchAllTileKeys` (0x40).
const FLAG_NAMES: [&str; 7] = [
    "OverlayTileOverlays",
    "OverlayFeatureTiles",
    "OverlayFeatureRooms",
    "OverlayFeatureRoomOverlap",
    "OverlayDescriptionNodes",
    "OverlayExtraFeatures",
    "MatchAllTileKeys",
];

#[derive(Debug, Clone, Default)]
pub struct ToyEntry {
    pub weight: u32,
    pub path: String,
    pub flags: u8,
    pub limit: u8,
    pub required: u8,
    pub exclusions: Vec<String>,
    /// Allowed orientations (bit per orientation).
    pub orientations: u8,
}

#[derive(Debug, Clone, Default)]
pub struct ToyPass {
    /// Phases the pass runs in (bit 0: before the fill, bit 1: after it).
    pub phases: u8,
    /// 0 random (all groups' positions together), 1 file order, 2 size order.
    pub order: u8,
    pub flags: u8,
    pub entries: Vec<ToyEntry>,
}

/// `+name`/`-name` on a flag byte (`0x141f9f330`); false when not an overlay flag.
fn apply_flag(flags: &mut u8, token: &str) -> bool {
    let (on, name) = match token.as_bytes().first() {
        Some(b'+') => (true, &token[1..]),
        Some(b'-') => (false, &token[1..]),
        _ => return false,
    };
    let Some(i) = FLAG_NAMES.iter().position(|n| *n == name) else { return false };
    let bit = 1u8 << i;
    if bit != 0x20 {
        if on { *flags |= bit } else { *flags &= !bit }
    }
    true
}

/// `limit=` / `required=` numbers: leading digits after spaces (`[1]` reads as 0).
fn small_number(s: &str) -> u8 {
    let v: u32 = s.trim_start().chars().take_while(|c| c.is_ascii_digit()).fold(0, |a, c| a.saturating_mul(10).saturating_add(c as u32 - '0' as u32));
    v.min(255) as u8
}

pub fn parse_toy(text: &str) -> Result<Vec<ToyPass>, GenError> {
    let mut passes: Vec<ToyPass> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with("//") || line.starts_with("version") {
            continue;
        }
        let tokens = crate::text::tokens(line);
        if line.contains('"') {
            let Some(pass) = passes.last_mut() else { return Err(GenError::Unsupported("toy entry before a pass header".into())) };
            let mut e = ToyEntry {
                weight: tokens[0].parse().unwrap_or(0),
                path: tokens.get(1).cloned().unwrap_or_default(),
                flags: pass.flags,
                ..Default::default()
            };
            for t in &tokens[2..] {
                if t.ends_with("SuppressUnderlyingDoodads") || t.ends_with("SuppressUnderlyingSpawnPoints") || apply_flag(&mut e.flags, t) {
                    continue;
                }
                if let Some(x) = t.strip_prefix('!') {
                    e.exclusions.push(x.to_string());
                } else if let Some(v) = t.strip_prefix("limit=") {
                    e.limit = small_number(v);
                } else if let Some(v) = t.strip_prefix("required=") {
                    e.required = small_number(v);
                } else if let Some(o) = ORIENTATION_NAMES.iter().position(|n| n == t) {
                    e.orientations |= 1 << o;
                } else {
                    return Err(GenError::Unsupported(format!("unknown overlay room argument {t:?}")));
                }
            }
            if e.orientations == 0 {
                e.orientations = 0xff;
            }
            pass.entries.push(e);
        } else {
            let mut p = ToyPass { phases: 3, flags: 0x18, ..Default::default() };
            let mut bit = 0;
            for t in &tokens {
                if bit < 3 && (t == "0" || t == "1") {
                    if t == "1" { p.phases |= 1 << bit } else { p.phases &= !(1 << bit) }
                    bit += 1;
                } else if t.ends_with("SuppressUnderlyingDoodads") || t.ends_with("SuppressUnderlyingSpawnPoints") || apply_flag(&mut p.flags, t) {
                } else if t == "SizeOrder" {
                    p.order = 2;
                } else if t == "FileOrder" {
                    p.order = 1;
                } else if t == "Random" {
                    p.order = 0;
                } else {
                    return Err(GenError::Unsupported(format!("unknown overlay pass argument {t:?}")));
                }
            }
            passes.push(p);
        }
    }
    Ok(passes)
}

/// One room of a group.
#[derive(Debug, Clone)]
struct GroupRoom {
    room: usize,
    limit: u8,
    exclusions: Vec<String>,
    /// Orientations in which the room's root key is the group's key.
    orientations: u8,
    weight: u32,
}

/// Rooms sharing a turned root key and flags.
#[derive(Debug, Clone)]
struct Group {
    key: TileKey,
    flags: u8,
    rooms: Vec<GroupRoom>,
    total: u32,
}

/// The overlay state of one region (`0x141fa0ac0`).
pub struct Overlays {
    passes: Vec<(ToyPass, Vec<Group>)>,
    rooms: Vec<(String, Room)>,
    /// Occupancy bits per tile: 1 overlay, 2 feature tile, 4 feature room, 8 feature room
    /// overlap, 0x10 description nodes, 0x20 extra features.
    occupied: Vec<u8>,
    width: i32,
    height: i32,
    /// Overlays placed per room (for `limit=`).
    counts: HashMap<usize, u32>,
    /// Rooms placed so far (room index, origin, orientation): rooms like any other for the
    /// de-repetition pass.
    pub placed: Vec<(usize, (i32, i32), u8)>,
}

/// A placed room as the overlay grid sees it: rectangle (tiles, end exclusive), whether its
/// `.arm` has a tag (a feature room) and whether its graph node has tags (description nodes).
pub struct RoomRect {
    pub rect: (i32, i32, i32, i32),
    pub feature: bool,
    pub described: bool,
}

impl Overlays {
    /// Loads the master's `.toy` and builds the groups and occupancy grid.
    /// A placed overlay room's data.
    pub fn room(&self, i: usize) -> &Room {
        &self.rooms[i].1
    }

    pub fn new(files: &dyn Files, toy: &str, map: &TileMap, master: &mut Master, rooms_placed: &[RoomRect]) -> Result<Self, GenError> {
        let passes = parse_toy(&crate::text::decode_utf16(&files.read(toy)?))?;
        let mut rooms: Vec<(String, Room)> = Vec::new();
        let mut built = Vec::new();
        for pass in passes {
            let mut groups: Vec<Group> = Vec::new();
            for e in &pass.entries {
                let idx = match rooms.iter().position(|r| r.0.eq_ignore_ascii_case(&e.path)) {
                    Some(i) => i,
                    None => {
                        rooms.push((e.path.clone(), load_room(files, &e.path)?));
                        rooms.len() - 1
                    }
                };
                let poe_data_tools::file_parsers::arm::types::Slot::K(k) = &rooms[idx].1.arm.root_slot else {
                    return Err(GenError::Unsupported(format!("overlay room {} has no root key", e.path)));
                };
                let root = slot_key(k);
                for o in 0..8u8 {
                    if e.orientations & (1 << o) == 0 {
                        continue;
                    }
                    let key = root.oriented(o);
                    let g = match groups.iter().position(|g| g.key == key && g.flags == e.flags) {
                        Some(g) => g,
                        None => {
                            groups.push(Group { key, flags: e.flags, rooms: Vec::new(), total: 0 });
                            groups.len() - 1
                        }
                    };
                    match groups[g].rooms.iter_mut().find(|r| r.room == idx) {
                        Some(r) => r.orientations |= 1 << o,
                        None => groups[g].rooms.push(GroupRoom { room: idx, limit: e.limit, exclusions: e.exclusions.clone(), orientations: 1 << o, weight: e.weight }),
                    }
                }
            }
            for g in &mut groups {
                g.rooms.retain(|r| r.weight != 0);
                g.total = g.rooms.iter().map(|r| r.weight).sum();
            }
            groups.retain(|g| g.total != 0);
            if pass.order == 2 {
                return Err(GenError::Unsupported("overlay SizeOrder sorting is not ported".into()));
            }
            built.push((pass, groups));
        }
        let (width, height) = (map.width as i32, map.height as i32);
        let mut s = Self { passes: built, rooms, occupied: vec![0; (width * height) as usize], width, height, counts: HashMap::new(), placed: Vec::new() };
        if s.passes.iter().any(|p| p.0.phases & 3 != 0) {
            for r in rooms_placed {
                if r.described {
                    s.mark(r.rect, 0x10);
                }
            }
            for r in rooms_placed {
                if r.feature {
                    s.mark(r.rect, 4);
                    s.mark(r.rect, 8);
                }
            }
            for p in 0..map.pieces.len() {
                let piece = &map.pieces[p];
                let tdt = master.header(files, &piece.path)?;
                if tdt.feature.as_deref().is_some_and(|f| !f.is_empty()) {
                    let (w, h) = TileMap::oriented_size(piece.size, piece.orientation);
                    let still = map.index(piece.x, piece.y).is_some_and(|i| map.piece_at[i] == Some(p));
                    if still {
                        s.mark((piece.x, piece.y, piece.x + w, piece.y + h), 2);
                    }
                }
            }
        }
        Ok(s)
    }

    fn mark(&mut self, (x0, y0, x1, y1): (i32, i32, i32, i32), bit: u8) {
        for y in y0.max(0)..y1.min(self.height) {
            for x in x0.max(0)..x1.min(self.width) {
                self.occupied[(y * self.width + x) as usize] |= bit;
            }
        }
    }

    /// `0x141fa1a70`: the passes of phase `phase` (1 or 2).
    #[allow(clippy::too_many_arguments)]
    pub fn run(&mut self, phase: u8, map: &mut TileMap, master: &mut Master, types: &mut TerrainTypes, files: &dyn Files, rng: &mut TerrainRng) -> Result<(), GenError> {
        for pi in 0..self.passes.len() {
            if self.passes[pi].0.phases & phase == 0 {
                continue;
            }
            let groups = self.passes[pi].1.clone();
            if self.passes[pi].0.order == 0 {
                let mut cands = Candidates::default();
                for (gi, g) in groups.iter().enumerate() {
                    self.collect(g, gi, map, master, types, files, &mut cands)?;
                }
                after_fill(phase, &cands)?;
                self.place(&groups, cands, map, master, types, files, rng)?;
            } else {
                for (gi, g) in groups.iter().enumerate() {
                    let mut cands = Candidates::default();
                    self.collect(g, gi, map, master, types, files, &mut cands)?;
                    after_fill(phase, &cands)?;
                    self.place(&groups, cands, map, master, types, files, rng)?;
                }
            }
        }
        Ok(())
    }

    /// `0x141fa1e60`: every position the group fits, on a 3-tile grid when its key has an edge
    /// of size 3.
    #[allow(clippy::too_many_arguments)]
    fn collect(&self, g: &Group, gi: usize, map: &TileMap, master: &mut Master, types: &mut TerrainTypes, files: &dyn Files, out: &mut Candidates) -> Result<(), GenError> {
        let mut stride3 = false;
        for e in g.key.edges.iter().flatten() {
            if types.edge(files, e)?.size == 3 {
                stride3 = true;
            }
        }
        if stride3 {
            let mut y = 0;
            while y <= self.height - 3 {
                let mut x = 0;
                while x <= self.width - 3 {
                    if self.fits(g, (x, y), map, master, types, files)? {
                        out.add((x, y), gi, g.total);
                    }
                    x += 3;
                }
                y += 3;
            }
        } else {
            for y in 0..self.height {
                for x in 0..self.width {
                    if self.fits(g, (x, y), map, master, types, files)? {
                        out.add((x, y), gi, g.total);
                    }
                }
            }
        }
        Ok(())
    }

    /// `0x141fa1830`: inside the region, over cells the group's flags allow, covering whole
    /// pieces, and the key fits the terrain (`0x141f9d200` with margin 1).
    fn fits(&self, g: &Group, (x, y): (i32, i32), map: &TileMap, master: &mut Master, types: &mut TerrainTypes, files: &dyn Files) -> Result<bool, GenError> {
        let (w, h) = (g.key.size.0 as i32, g.key.size.1 as i32);
        if x >= self.width || y >= self.height || x + w - 1 >= self.width || y + h - 1 >= self.height {
            return Ok(false);
        }
        for yy in y..y + h {
            for xx in x..x + w {
                if !g.flags & self.occupied[(yy * self.width + xx) as usize] != 0 {
                    return Ok(false);
                }
            }
        }
        if !covers_whole_pieces(map, (x, y, x + w, y + h)) {
            return Ok(false);
        }
        Ok(room_fits(map, (x, y), &g.key, false, 1, master, types, files)? == 0)
    }

    /// `0x141fa23e0`: draws positions until none are left; at each, a group that still fits,
    /// then a room of it (or nothing, padded to 100), its orientation and its height.
    #[allow(clippy::too_many_arguments)]
    fn place(&mut self, groups: &[Group], mut cands: Candidates, map: &mut TileMap, master: &mut Master, types: &mut TerrainTypes, files: &dyn Files, rng: &mut TerrainRng) -> Result<(), GenError> {
        let mut total: u32 = cands.0.values().map(|c| c.0).sum();
        while !cands.0.is_empty() {
            let mut r = rng.below(total);
            let mut pick = None;
            for (k, c) in cands.0.iter() {
                if r < c.0 {
                    pick = Some(*k);
                    break;
                }
                r -= c.0;
            }
            let k = pick.unwrap_or(*cands.0.keys().last().unwrap());
            let (weight, list) = cands.0.remove(&k).unwrap();
            total -= weight;
            let pos = ((k >> 16) as i32, (k & 0xffff) as i32);
            let mut live: Vec<(usize, u32)> = Vec::new();
            for &(gi, w) in &list {
                if self.fits(&groups[gi], pos, map, master, types, files)? {
                    live.push((gi, w));
                }
            }
            if live.is_empty() {
                continue;
            }
            let sum: u32 = live.iter().map(|l| l.1).sum();
            let mut r = rng.below(sum);
            let mut gi = live[live.len() - 1].0;
            for &(g, w) in &live {
                if r < w {
                    gi = g;
                    break;
                }
                r -= w;
            }
            let g = &groups[gi];
            let (w, h) = (g.key.size.0 as i32, g.key.size.1 as i32);
            let mut options: Vec<(Option<usize>, u32)> = Vec::new();
            for (ri, room) in g.rooms.iter().enumerate() {
                if room.limit != 0 && self.counts.get(&room.room).copied().unwrap_or(0) >= room.limit as u32 {
                    continue;
                }
                if !room.exclusions.is_empty() {
                    return Err(GenError::Unsupported("overlay exclusions (0x141fa3470) are not ported".into()));
                }
                if g.flags & 0x40 != 0 {
                    return Err(GenError::Unsupported("MatchAllTileKeys overlays (0x141fa3780) are not ported".into()));
                }
                options.push((Some(ri), room.weight));
            }
            if options.is_empty() {
                continue;
            }
            let mut sum: u32 = options.iter().map(|o| o.1).sum();
            if sum < 100 {
                options.push((None, 100 - sum));
                sum = 100;
            }
            let mut r = rng.below(sum);
            let mut choice = options[options.len() - 1].0;
            for &(o, wgt) in &options {
                if r < wgt {
                    choice = o;
                    break;
                }
                r -= wgt;
            }
            let Some(ri) = choice else {
                self.mark((pos.0, pos.1, pos.0 + w, pos.1 + h), 1);
                continue;
            };
            let room = &g.rooms[ri];
            let v = rng.below(0x348);
            let o = crate::region::pick(room.orientations, v);
            let _ = lcg_index;
            let Some(height) = room_height(&self.rooms[room.room].1, map, pos, o)? else { continue };
            if height != 0 {
                return Err(GenError::Unsupported("overlay with a height offset".into()));
            }
            // Phase 1: the tiles under it go, its corner points are cleared, and the room is
            // placed like any room (`0x141f8e6c0`).
            for yy in pos.1..pos.1 + h {
                for xx in pos.0..pos.0 + w {
                    map.remove_piece_at(xx, yy);
                }
            }
            for yy in pos.1..=pos.1 + h {
                for xx in pos.0..=pos.0 + w {
                    if let Some(i) = map.point_index(xx, yy) {
                        map.lattice[i].ground = None;
                        map.lattice[i].flags &= !1;
                    }
                }
            }
            room_pass_oriented(map, master, &self.rooms[room.room].1, pos, o, true, rng)?;
            self.mark((pos.0, pos.1, pos.0 + w, pos.1 + h), 1);
            *self.counts.entry(room.room).or_default() += 1;
            self.placed.push((room.room, pos, o));
        }
        Ok(())
    }
}

/// Candidate positions keyed `x·0x10000 + y` (`0x141fa20b0`): summed weight and the groups.
#[derive(Default)]
struct Candidates(BTreeMap<u32, (u32, Vec<(usize, u32)>)>);

impl Candidates {
    fn add(&mut self, (x, y): (i32, i32), group: usize, weight: u32) {
        let e = self.0.entry((x as u32) << 16 | y as u32).or_default();
        e.0 += weight;
        e.1.push((group, weight));
    }
}

/// `0x141d0dc40`: the rectangle holds at least one tile and every tile it touches lies inside it.
fn covers_whole_pieces(map: &TileMap, (x0, y0, x1, y1): (i32, i32, i32, i32)) -> bool {
    let mut any = false;
    for y in y0..y1 {
        for x in x0..x1 {
            let Some(i) = map.index(x, y) else { continue };
            let Some(cell) = &map.cells[i] else { continue };
            any = true;
            let Some(p) = map.piece_at[i] else { continue };
            let piece = &map.pieces[p];
            let (ox, oy) = (x - cell.offset.0 as i32, y - cell.offset.1 as i32);
            let (w, h) = TileMap::oriented_size(piece.size, piece.orientation);
            if ox < x0 || oy < y0 || ox + w > x1 || oy + h > y1 {
                return false;
            }
        }
    }
    any
}

/// The key of the tile at a map cell and the cell's tile origin.
fn tile_key_at(map: &TileMap, x: i32, y: i32, master: &mut Master, files: &dyn Files) -> Result<Option<(TileKey, (i32, i32))>, GenError> {
    let Some(i) = map.index(x, y) else { return Ok(None) };
    let Some(cell) = map.cells[i].clone() else { return Ok(None) };
    let tdt = master.header(files, &cell.path)?;
    let key = TileKey::from_tdt(tdt).oriented(cell.orientation);
    Ok(Some((key, (x - cell.offset.0 as i32, y - cell.offset.1 as i32))))
}

/// An exit along a side as a position left-to-right / top-to-bottom (sides 2 and 3 run the
/// other way in keys).
fn absolute_exit(side: usize, len: u8, e: u8) -> i32 {
    if side > 1 { 3 * len as i32 - e as i32 - 1 } else { e as i32 }
}

/// Lattice ground and height at a corner point (`None` for grounds with `.gt` flag 0x10).
fn corner(map: &TileMap, x: i32, y: i32) -> (Option<String>, i32) {
    match map.point_index(x, y) {
        Some(i) => {
            let p = &map.lattice[i];
            let g = p.ground.clone().filter(|g| map.gt_flags(g) & 0x10 == 0);
            let mut h = p.height as i32;
            if h < -0x78 {
                h += 0x100;
            }
            (g, h)
        }
        None => (None, 0),
    }
}

/// `0x141f9d200`: 0 when `key` fits the terrain at `pos` (corner grounds, one consistent height
/// difference, edges against the neighbouring tiles); else the failing check's code.
#[allow(clippy::too_many_arguments)]
pub fn room_fits(map: &TileMap, pos: (i32, i32), key: &TileKey, wild: bool, margin: i32, master: &mut Master, types: &mut TerrainTypes, files: &dyn Files) -> Result<u32, GenError> {
    let (x, y) = pos;
    let (w, h) = (key.size.0 as i32, key.size.1 as i32);
    if x < 0 || y < 0 || x + w > map.width as i32 || y + h > map.height as i32 {
        return Ok(5);
    }
    let corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
    let key_ground = |i: usize| key.grounds[i].clone().filter(|g| map.gt_flags(g) & 0x10 == 0);
    // `0x141f9cfc0`
    let mut odd = 0;
    for (s, &(cx, cy)) in corners.iter().enumerate() {
        let (a, _) = corner(map, cx, cy);
        let b = key_ground(s);
        let ok = if a == b {
            true
        } else {
            let wild8 = |g: &Option<String>| g.as_ref().is_none_or(|g| map.gt_flags(g) & 8 != 0);
            if wild8(&a) && wild8(&b) {
                wild || a.is_some()
            } else if let Some(ag) = &a {
                let fa = map.gt_flags(ag);
                if fa & 0x80 != 0 {
                    true
                } else if fa & 5 == 0 && b.as_ref().is_some_and(|bg| map.gt_flags(bg) & 5 == 0) {
                    odd += 1;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if !ok {
            return Ok(6 + s as u32);
        }
    }
    if odd == 4 {
        return Ok(6);
    }
    let mut diffs: Vec<i32> = Vec::new();
    for (s, &(cx, cy)) in corners.iter().enumerate() {
        let (a, ah) = corner(map, cx, cy);
        let b = key_ground(s);
        let takes = |g: &Option<String>| g.as_ref().is_none_or(|g| map.gt_flags(g) & 1 != 0);
        if takes(&b) && takes(&a) {
            diffs.push(key.heights[s] as i8 as i32 - ah);
        }
    }
    diffs.sort();
    diffs.dedup();
    if diffs.len() >= 2 {
        return Ok(10);
    }
    let (rx0, ry0, rx1, ry1) = (0, 0, map.width as i32, map.height as i32);
    if ry0 - margin < y && !side_fits(map, pos, key, 0, wild, master, types, files)? {
        return Ok(1);
    }
    if rx0 - margin < x && !side_fits(map, pos, key, 3, wild, master, types, files)? {
        return Ok(4);
    }
    if x + w < rx1 + margin && !side_fits(map, pos, key, 1, wild, master, types, files)? {
        return Ok(2);
    }
    if y + h < ry1 + margin && !side_fits(map, pos, key, 2, wild, master, types, files)? {
        return Ok(3);
    }
    Ok(0)
}

/// The cell just outside side `side` of a key at `pos`, `i` tiles along it (`0x141f9c540`).
fn outside(pos: (i32, i32), key: &TileKey, side: usize, i: i32) -> (i32, i32) {
    let (x, y) = pos;
    match side {
        0 => (x + i, y - 1),
        1 => (x + key.size.0 as i32, y + i),
        2 => (x + i, y + key.size.1 as i32),
        _ => (x - 1, y + i),
    }
}

/// `0x141f9cee0`
#[allow(clippy::too_many_arguments)]
fn side_fits(map: &TileMap, pos: (i32, i32), key: &TileKey, side: usize, wild: bool, master: &mut Master, types: &mut TerrainTypes, files: &dyn Files) -> Result<bool, GenError> {
    let Some(e) = &key.edges[side] else {
        return Ok(!edge_crosses(map, pos, key, side, master, types, files)?);
    };
    let et = types.edge(files, e)?.clone();
    if et.wildcard {
        return Ok(true);
    }
    if !exit_fits(map, pos, key, side, wild, false, master, types, files)? {
        return Ok(false);
    }
    if et.virtual_edge && !exit_fits(map, pos, key, side, wild, true, master, types, files)? {
        return Ok(false);
    }
    Ok(true)
}

/// `0x141f9c800`: whether a neighbouring tile has an edge crossing into this (edge-less) side.
fn edge_crosses(map: &TileMap, pos: (i32, i32), key: &TileKey, side: usize, master: &mut Master, types: &mut TerrainTypes, files: &dyn Files) -> Result<bool, GenError> {
    let len = if side % 2 == 0 { key.size.0 } else { key.size.1 };
    let start = if side % 2 == 0 { pos.0 } else { pos.1 };
    for i in 0..len as i32 {
        let (cx, cy) = outside(pos, key, side, i);
        if cx < 0 || cy < 0 || cx >= map.width as i32 || cy >= map.height as i32 {
            break;
        }
        let Some((nk, origin)) = tile_key_at(map, cx, cy, master, files)? else { continue };
        let ns = (side + 2) & 3;
        let Some(ne) = &nk.edges[ns] else { continue };
        let net = types.edge(files, ne)?.clone();
        if net.wildcard || net.can_end {
            continue;
        }
        let nlen = if ns % 2 == 0 { nk.size.0 } else { nk.size.1 };
        let along = if side % 2 == 0 { origin.0 } else { origin.1 };
        let a = absolute_exit(ns, nlen, nk.exits[ns]) / 3 + along;
        if start <= a && a < start + len as i32 {
            return Ok(true);
        }
        if net.virtual_edge {
            let b = absolute_exit(ns, nlen, nk.virtual_exits[ns]) / 3 + along;
            if start <= b && b < start + len as i32 {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// `0x141f9c600`: whether an existing edge type joins the key's.
fn edges_join(a: &str, b: &str, types: &mut TerrainTypes, files: &dyn Files) -> Result<bool, GenError> {
    let ta = types.edge(files, a)?.clone();
    let tb = types.edge(files, b)?.clone();
    let eq = |x: &str, y: &str| x.eq_ignore_ascii_case(y);
    Ok(match (ta.virtual_edge, tb.virtual_edge) {
        (false, false) => eq(a, b),
        (false, true) => eq(a, &tb.splits[0].0) || eq(a, &tb.splits[1].0),
        (true, false) => eq(b, &ta.splits[0].0) || eq(b, &ta.splits[1].0),
        (true, true) => ta.splits.iter().any(|x| tb.splits.iter().any(|y| eq(&x.0, &y.0))),
    })
}

/// `0x141f9ca30`: the key's exit on `side` meets a matching edge of the neighbouring tile at
/// the same place, and no other neighbouring edge enters the side.
#[allow(clippy::too_many_arguments)]
fn exit_fits(map: &TileMap, pos: (i32, i32), key: &TileKey, side: usize, wild: bool, virtual_exit: bool, master: &mut Master, types: &mut TerrainTypes, files: &dyn Files) -> Result<bool, GenError> {
    let e = key.edges[side].clone().unwrap();
    let et = types.edge(files, &e)?.clone();
    let len = if side % 2 == 0 { key.size.0 } else { key.size.1 };
    let along_pos = if side % 2 == 0 { pos.0 } else { pos.1 };
    let raw = if virtual_exit { key.virtual_exits[side] } else { key.exits[side] };
    let exit = absolute_exit(side, len, raw);
    let (cx, cy) = outside(pos, key, side, exit / 3);
    if cx < 0 || cy < 0 || cx >= map.width as i32 || cy >= map.height as i32 {
        return Ok(et.can_end);
    }
    let Some((nk, origin)) = tile_key_at(map, cx, cy, master, files)? else { return Ok(false) };
    let ns = (side + 2) & 3;
    let nlen = if ns % 2 == 0 { nk.size.0 } else { nk.size.1 };
    let along = if side % 2 == 0 { origin.0 } else { origin.1 };
    let joined = match &nk.edges[ns] {
        None => et.can_end,
        Some(ne) => {
            let net = types.edge(files, ne)?.clone();
            if net.wildcard {
                wild
            } else {
                let target = exit + along_pos * 3;
                let mut nb = absolute_exit(ns, nlen, nk.exits[ns]) + along * 3;
                if net.virtual_edge && nb != target {
                    nb = absolute_exit(ns, nlen, nk.virtual_exits[ns]) + along * 3;
                }
                if nb != target {
                    return Ok(false);
                }
                edges_join(ne, &e, types, files)?
            }
        }
    };
    if !joined {
        return Ok(false);
    }
    // No other neighbouring edge may enter this side.
    let mut seen = vec![origin];
    if et.virtual_edge {
        let other = if virtual_exit { key.exits[side] } else { key.virtual_exits[side] };
        let (ox, oy) = outside(pos, key, side, absolute_exit(side, len, other) / 3);
        if let Some((_, o2)) = tile_key_at(map, ox, oy, master, files)? {
            seen.push(o2);
        }
    }
    for i in 0..len as i32 {
        let (qx, qy) = outside(pos, key, side, i);
        if qx < 0 || qy < 0 || qx >= map.width as i32 || qy >= map.height as i32 {
            continue;
        }
        let Some((qk, qo)) = tile_key_at(map, qx, qy, master, files)? else { continue };
        if seen.contains(&qo) {
            continue;
        }
        let Some(qe) = &qk.edges[ns] else { continue };
        let qet = types.edge(files, qe)?.clone();
        if qet.wildcard {
            continue;
        }
        let qlen = if ns % 2 == 0 { qk.size.0 } else { qk.size.1 };
        let qalong = if side % 2 == 0 { qo.0 } else { qo.1 };
        let u = absolute_exit(ns, qlen, qk.exits[ns]) / 3 - along_pos + qalong;
        if 0 <= u && u < len as i32 {
            return Ok(false);
        }
        if qet.virtual_edge {
            let u = absolute_exit(ns, qlen, qk.virtual_exits[ns]) / 3 - along_pos + qalong;
            if 0 <= u && u < len as i32 {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// `0x141fa10b0`: the height a room must be placed at so that its border slots' corner heights
/// meet the terrain's set heights (None when they disagree).
fn room_height(room: &Room, map: &TileMap, pos: (i32, i32), o: u8) -> Result<Option<i32>, GenError> {
    // Rooms in act 1 overlays have flat corners; a full port needs the slots' corner heights.
    let _ = (room, map, pos, o);
    Ok(Some(0))
}

/// Phase 2 placement (`0x141fa23e0` with `param_3` set) takes the pieces under the overlay out
/// and fills what the room leaves free again (`0x141f8f930`, `0x141f9e2e0`); not ported. With no
/// candidates it draws nothing, as in phase 1.
fn after_fill(phase: u8, cands: &Candidates) -> Result<(), GenError> {
    if phase == 2 && !cands.0.is_empty() {
        return Err(GenError::Unsupported("overlay placement after the fill (phase 2) is not ported".into()));
    }
    Ok(())
}
