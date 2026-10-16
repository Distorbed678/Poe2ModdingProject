//! The fill pass (`0x141f93060`): every cell still empty after rooms and edge tiles gets a fill
//! tile or a fill room from the `.gft` group of the ground at its top-left corner. Findings:
//! `Documentation/re/terrain.md`, "Fill".

use std::collections::HashMap;

use crate::{
    generate::{GenError, Files, Master, Room, TileMap, cell_key, load_room, uniform_ground},
    region::compose,
    rng::TerrainRng,
    tdt::Tdt,
    tileset::{orient_cell, pick},
};

/// One `.gft` entry: a fill tile (`.tdt`) or fill room (`.arm`), its weight and orientation
/// (8 = any).
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    weight: u32,
    path: String,
    o: u8,
    room: bool,
}

fn entries(group: &[(u32, String, u8)]) -> Result<Vec<Entry>, GenError> {
    group
        .iter()
        .map(|(w, p, mask)| {
            let o = match mask {
                0xff => 8,
                m if m.count_ones() == 1 => m.trailing_zeros() as u8,
                m => return Err(GenError::Unsupported(format!("fill {p}: orientation mask {m:#x}"))),
            };
            Ok(Entry { weight: *w, path: p.clone(), o, room: p.to_ascii_lowercase().ends_with(".arm") })
        })
        .collect()
}

/// A fill room placed by the pass (for the de-repetition pass's ground overrides).
pub struct PlacedFillRoom {
    pub room: Room,
    pub origin: (i32, i32),
    pub orientation: u8,
}

struct Fill<'a> {
    map: &'a mut TileMap,
    master: &'a mut Master,
    files: &'a dyn Files,
    rooms: HashMap<String, Room>,
    placed: Vec<PlacedFillRoom>,
}

fn oriented(size: (u8, u8), o: u8) -> (i32, i32) {
    if o & 1 == 1 && o < 8 { (size.1 as i32, size.0 as i32) } else { (size.0 as i32, size.1 as i32) }
}

/// `0x1421c3d00`: the `r`-th (mod count) set bit of an orientation mask.
fn mask_pick(mask: u8, r: u32) -> u8 {
    let bits: Vec<u8> = (0..8).filter(|b| mask >> b & 1 != 0).collect();
    bits[(r as usize) % bits.len()]
}

impl Fill<'_> {
    fn free(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        (0..h).all(|dy| {
            (0..w).all(|dx| self.map.index(x + dx, y + dy).is_none_or(|i| self.map.cells[i].is_none()))
        })
    }

    /// `0x141f9e5f0`: a tile fits at `(x, y)` (one cell of slack past the region's far sides).
    fn tile_fits(&self, x: i32, y: i32, tdt: &Tdt, o: u8) -> bool {
        let (w, h) = oriented(tdt.size, o);
        x >= -1
            && y >= -1
            && x + w <= self.map.width as i32 + 1
            && y + h <= self.map.height as i32 + 1
            && x >= 0
            && y >= 0
            && self.free(x, y, w, h)
    }

    /// `0x141f90e70`: a fill room fits at `(x, y)` with orientation `o` (inside the region).
    fn room_fits(&self, x: i32, y: i32, dims: (u8, u8), o: u8) -> bool {
        let (w, h) = oriented(dims, o);
        let (mw, mh) = (self.map.width as i32, self.map.height as i32);
        x < mw && x + w <= mw && y < mh && y + h <= mh && self.free(x, y, w, h)
    }

    /// `0x141f90d60`: orientation for a fill tile, 8 when it does not fit.
    fn tile_orientation(&self, x: i32, y: i32, tdt: &Tdt, o: u8, rng: &mut TerrainRng) -> u8 {
        choose_orientation(tdt, o, &|o| self.tile_fits(x, y, tdt, o), rng)
    }

    /// `0x141f8c500`: a tile turned by `o` (room frame) covers only free uniform cells of the
    /// room grid from `(cx, cy)`: no feature or edges, one ground compatible with the tile's
    /// (equal, or both missing or `.gt` flag 8) and the start cell's height.
    fn grid_fits(&self, room: &Room, cx: usize, cy: usize, tdt: &Tdt, o: u8) -> bool {
        use poe_data_tools::file_parsers::arm::types::Slot;
        let (w, h) = oriented(tdt.size, o);
        let grid = &room.arm.grid;
        let (rw, rh) = (room.width(), room.height());
        if cx + w as usize > rw || cy + h as usize > rh {
            return false;
        }
        let uniform = |slot: &Slot| -> Option<(Option<String>, u8)> {
            match slot {
                Slot::S => Some((None, 0)),
                slot => cell_key(slot)
                    .filter(|(k, f)| uniform_ground(k, f))
                    .map(|(k, _)| (k.grounds[0].clone(), k.heights[0])),
            }
        };
        let tile_ground = tdt.point_grounds.first().cloned().flatten();
        let loose = |g: &Option<String>| g.as_ref().is_none_or(|g| self.map.gt_flags(g) & 8 != 0);
        let Some((_, height)) = uniform(&grid[cy][cx]) else { return false };
        (0..h as usize).all(|dy| {
            (0..w as usize).all(|dx| {
                uniform(&grid[cy + dy][cx + dx]).is_some_and(|(g, hh)| {
                    hh == height && (g == tile_ground || (loose(&g) && loose(&tile_ground)))
                })
            })
        })
    }

    /// `0x141f90f60`: orientation for a fill room, 8 when it does not fit.
    fn room_orientation(&self, x: i32, y: i32, dims: (u8, u8), o: u8, rng: &mut TerrainRng) -> u8 {
        if o != 8 {
            return if self.room_fits(x, y, dims, o) { o } else { 8 };
        }
        if dims.0 == dims.1 {
            return if self.room_fits(x, y, dims, 0) { rng.below(8) as u8 } else { 8 };
        }
        let mask = if self.room_fits(x, y, dims, 0) { 0x55 } else { 0 } | if self.room_fits(x, y, dims, 1) { 0xaa } else { 0 };
        if mask == 0 { 8 } else { mask_pick(mask, rng.below(840)) }
    }

    fn room(&mut self, path: &str) -> Result<&Room, GenError> {
        let key = path.to_ascii_lowercase();
        if !self.rooms.contains_key(&key) {
            let r = load_room(self.files, &key)?;
            self.rooms.insert(key.clone(), r);
        }
        Ok(&self.rooms[&key])
    }

    /// `0x141f93960`: the fill tiles and fill rooms for a ground (the `default` group without
    /// one, or when its name has neither).
    fn groups(&mut self, ground: Option<&str>) -> Result<(Vec<Entry>, Vec<Entry>), GenError> {
        let split = |g: &[(u32, String, u8)]| -> Result<(Vec<Entry>, Vec<Entry>), GenError> {
            let e = entries(g)?;
            Ok(e.into_iter().partition(|e| !e.room))
        };
        let default = self.master.fills.get("default").cloned().unwrap_or_default();
        let Some(g) = ground else { return split(&default) };
        let name = self.master.ground_name(self.files, g)?;
        match self.master.fills.get(&name).cloned() {
            Some(group) => split(&group),
            None => split(&default),
        }
    }

    /// Height of a fill tile (`0x141f90950`): the middle of its set corner heights.
    fn corner_height(&self, x: i32, y: i32, w: i32, h: i32) -> i16 {
        let hs: Vec<i32> = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)]
            .iter()
            .filter_map(|&(px, py)| self.map.point_index(px, py))
            .filter(|&i| self.map.lattice[i].flags & 2 != 0)
            .map(|i| self.map.lattice[i].height as i32)
            .collect();
        match (hs.iter().min(), hs.iter().max()) {
            (Some(&lo), Some(&hi)) => (hi - (hi - lo) / 2) as i16,
            _ => 0,
        }
    }

    fn place_tile(&mut self, x: i32, y: i32, path: &str, o: u8) -> Result<i32, GenError> {
        let tdt = self.master.header(self.files, path)?.clone();
        let (w, h) = oriented(tdt.size, o);
        let height = self.corner_height(x, y, w, h);
        self.map.place(x, y, path, &tdt, o, height, None);
        Ok(w - 1)
    }

    /// `0x141f8f930`: fill cell `(x, y)`; returns how many further cells of the row are taken.
    fn cell(&mut self, x: i32, y: i32, ground: Option<String>, walkable: bool, rng: &mut TerrainRng) -> Result<i32, GenError> {
        let ground = ground.filter(|g| self.map.gt_flags(g) & 0x10 == 0);
        let (tiles, rooms) = self.groups(ground.as_deref())?;
        let mut list: Vec<Entry> = tiles;
        let with_rooms = !rooms.is_empty();
        if with_rooms {
            for r in rooms {
                // `0x141f91110`: on an unwalkable corner only rooms whose room data +0x188 list
                // (0x30-byte entries: the points of interest, by size) is empty.
                let has_pois = self.room(&r.path)?.arm.points_of_interest.iter().any(|g| !g.is_empty());
                if walkable || !has_pois {
                    list.push(r);
                }
            }
        }
        let mut total: u32 = list.iter().map(|e| e.weight).sum();
        while !list.is_empty() {
            let e = pick_entry(&list, rng.below(total)).clone();
            let o = if e.room {
                let room = self.room(&e.path)?;
                let dims = (room.width() as u8, room.height() as u8);
                self.room_orientation(x, y, dims, e.o, rng)
            } else {
                let tdt = self.master.header(self.files, &e.path)?.clone();
                self.tile_orientation(x, y, &tdt, e.o, rng)
            };
            if o != 8 {
                if !e.room {
                    return self.place_tile(x, y, &e.path, o);
                }
                self.place_room(x, y, &e.path, o, ground.clone(), walkable, rng)?;
                let empty = self.map.index(x, y).is_some_and(|i| self.map.cells[i].is_none());
                return if empty { self.cell(x, y, ground, walkable, rng) } else { Ok(0) };
            }
            let k = list.iter().position(|f| f.path == e.path && f.o == e.o && f.room == e.room).unwrap();
            total -= list[k].weight;
            list.remove(k);
        }
        Err(GenError::Unsupported(format!("failed to find a fill tile or room at ({x},{y})")))
    }

    /// `0x141f8c8c0`: a fill room's cells (x outer, y inner), each re-picked up to five times
    /// until it fits, skipped when its cell is already taken; then the room's ground overrides,
    /// and the fill's ground on its corners that have none.
    #[allow(clippy::too_many_arguments)]
    fn place_room(&mut self, x: i32, y: i32, path: &str, o: u8, ground: Option<String>, _walkable: bool, rng: &mut TerrainRng) -> Result<(), GenError> {
        let room = load_room(self.files, &path.to_ascii_lowercase())?;
        let (w, h) = (room.width(), room.height());
        for cx in 0..w {
            for cy in 0..h {
                // An `s` slot, or a key without feature and edges on one ground: a fill tile of
                // that ground (no ground: `default`), as the fill pass picks it.
                let uniform = match &room.arm.grid[cy][cx] {
                    poe_data_tools::file_parsers::arm::types::Slot::S => Some(None),
                    slot => cell_key(slot).filter(|(k, f)| f.is_empty() && k.edges.iter().all(Option::is_none) && k.grounds.iter().all(|g| *g == k.grounds[0])).map(|(k, _)| k.grounds[0].clone()),
                };
                if let Some(g) = uniform {
                    let (ox, oy) = orient_cell(o, w as i32, h as i32, cx as i32, cy as i32);
                    if self.map.index(x + ox, y + oy).is_none_or(|i| self.map.cells[i].is_some()) {
                        continue;
                    }
                    // `0x141f8e400`: the tile turned by `to` in the room's frame, placed from the
                    // smallest corner its cells reach, turned by `to` then the room.
                    let at = |tdt: &Tdt, to: u8| {
                        let (tw, th) = oriented(tdt.size, to);
                        let (bx, by) = orient_cell(o, w as i32, h as i32, cx as i32 + tw - 1, cy as i32 + th - 1);
                        (x + ox.min(bx), y + oy.min(by))
                    };
                    let (mut list, _) = self.groups(g.as_deref())?;
                    let mut total: u32 = list.iter().map(|e| e.weight).sum();
                    while !list.is_empty() {
                        let e = pick_entry(&list, rng.below(total)).clone();
                        let tdt = self.master.header(self.files, &e.path)?.clone();
                        let fits = |to: u8| {
                            let (mx, my) = at(&tdt, to);
                            self.tile_fits(mx, my, &tdt, compose(to, o)) && self.grid_fits(&room, cx, cy, &tdt, to)
                        };
                        let to = choose_orientation(&tdt, e.o, &fits, rng);
                        if to != 8 {
                            let (mx, my) = at(&tdt, to);
                            self.place_tile(mx, my, &e.path, compose(to, o))?;
                            break;
                        }
                        let k = list.iter().position(|f| f.path == e.path && f.o == e.o).unwrap();
                        total -= list[k].weight;
                        list.remove(k);
                    }
                    continue;
                }
                let Some((key, feature)) = cell_key(&room.arm.grid[cy][cx]) else { continue };
                let (sw, sh) = (key.size.0.max(1) as i32, key.size.1.max(1) as i32);
                let key = key.oriented(o);
                let (ax, ay) = orient_cell(o, w as i32, h as i32, cx as i32, cy as i32);
                let (bx, by) = orient_cell(o, w as i32, h as i32, cx as i32 + sw - 1, cy as i32 + sh - 1);
                let (mx, my) = (x + ax.min(bx), y + ay.min(by));
                if uniform_ground(&key, &feature) {
                    return Err(GenError::Unsupported(format!("fill room {path}: uniform-ground cell")));
                }
                let list = self.master.tiles.candidates(&feature, &key).to_vec();
                if list.is_empty() {
                    return Err(GenError::Unsupported(format!("fill room {path}: no tile for cell ({cx},{cy})")));
                }
                if self.map.index(mx, my).is_some_and(|i| self.map.cells[i].is_some()) {
                    continue;
                }
                let total = list.iter().map(|c| c.weight).sum();
                let mut chosen = None;
                for _ in 0..5 {
                    let c = pick(&list, rng.below(total)).unwrap();
                    let tdt = &self.master.tiles.tiles[c.tile].tdt;
                    if self.tile_fits(mx, my, tdt, c.orientation) {
                        chosen = Some(c);
                        break;
                    }
                }
                let c = chosen.ok_or_else(|| GenError::Unsupported(format!("no tile fits at ({mx},{my}) for fill room {path}")))?;
                let def = self.master.tiles.tiles[c.tile].clone();
                self.map.place(mx, my, &def.path, &def.tdt, c.orientation, 0, Some(c.tile));
            }
        }
        self.map.room_overrides(&room, (x, y), o);
        let (ow, oh) = oriented((w as u8, h as u8), o);
        for v in 0..=oh {
            for u in 0..=ow {
                let Some(i) = self.map.point_index(x + u, y + v) else { continue };
                if self.map.lattice[i].ground.is_none() {
                    if let Some(g) = &ground {
                        self.map.lattice[i].ground = Some(g.clone());
                    }
                    self.map.lattice[i].flags |= 1;
                }
            }
        }
        self.placed.push(PlacedFillRoom { room, origin: (x, y), orientation: o });
        Ok(())
    }
}

/// The orientation for a fill tile (entry orientation `o`, 8 = any) among those `fits`
/// accepts: a square tile any of 8 when unturned fits, else the even or odd ones (`0x1421c3d00`
/// on a draw of 840); 8 when none fits.
fn choose_orientation(tdt: &Tdt, o: u8, fits: &dyn Fn(u8) -> bool, rng: &mut TerrainRng) -> u8 {
    if o != 8 {
        return if fits(o) { o } else { 8 };
    }
    if tdt.size.0 == tdt.size.1 {
        return if fits(0) { rng.below(8) as u8 } else { 8 };
    }
    let mask = if fits(0) { 0x55 } else { 0 } | if fits(1) { 0xaa } else { 0 };
    if mask == 0 { 8 } else { mask_pick(mask, rng.below(840)) }
}

/// `0x1415d5a30`/`0x141faccc0`: weighted pick (`r` in `0..total`).
fn pick_entry(list: &[Entry], mut r: u32) -> &Entry {
    for e in list {
        if r < e.weight {
            return e;
        }
        r -= e.weight;
    }
    list.last().unwrap()
}

/// `0x141f93060`: rows top to bottom, cells left to right; a cell's ground is its top-left
/// corner's, and fill rooms with points of interest only go where that corner is walkable (no
/// flag 8, and no ground or one with `.gt` flag 4).
pub fn fill_pass(
    map: &mut TileMap,
    master: &mut Master,
    files: &dyn Files,
    rng: &mut TerrainRng,
) -> Result<Vec<PlacedFillRoom>, GenError> {
    let mut f = Fill { map, master, files, rooms: HashMap::new(), placed: Vec::new() };
    for y in 0..f.map.height as i32 {
        let mut x = 0;
        while x < f.map.width as i32 {
            if f.map.index(x, y).is_some_and(|i| f.map.cells[i].is_none()) {
                let p = &f.map.lattice[f.map.point_index(x, y).unwrap()];
                let ground = p.ground.clone();
                let walkable = ground.as_ref().is_none_or(|g| f.map.gt_flags(g) & 4 != 0) && p.flags & 8 == 0;
                x += f.cell(x, y, ground, walkable, rng)?;
            }
            x += 1;
        }
    }
    Ok(f.placed)
}
