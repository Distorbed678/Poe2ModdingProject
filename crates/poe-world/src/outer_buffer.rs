//! The outer buffer (`0x141fd8360`): the map is the region plus the master's
//! `OuterBufferSize` margins; corner grounds and heights are carried out into the margins, then
//! each margin strip is tiled from the region's border outward: an edge tile where a border
//! tile's edge crosses the border (picked from the tile set), otherwise the ground's first fill
//! tile. Findings: `Documentation/re/terrain.md`, "Outer buffer".

use std::collections::HashMap;

use crate::{
    generate::{Files, GenError, Master, TileMap},
    rng::TerrainRng,
    rooms::EdgeType,
    tileset::{TileKey, pick},
};

/// Margins in cells.
#[derive(Debug, Clone, Copy, Default)]
pub struct Margins {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

impl Margins {
    /// `OuterBufferSize` from a `.tsi` (top, right, bottom, left; `0x1429285e0`: a missing
    /// right or bottom copies the top, a missing left copies the right).
    pub fn parse(tsi_text: &str) -> Margins {
        let line = tsi_text.lines().map(str::trim).find(|l| l.starts_with("OuterBufferSize"));
        let v: Vec<u32> = line
            .map(|l| l.split_whitespace().skip(1).filter_map(|t| t.parse().ok()).collect())
            .unwrap_or_default();
        let top = v.first().copied().unwrap_or(0);
        let right = v.get(1).copied().unwrap_or(top);
        let bottom = v.get(2).copied().unwrap_or(top);
        let left = v.get(3).copied().unwrap_or(right);
        Margins { top, right, bottom, left }
    }
}

struct Pass<'a> {
    map: &'a mut TileMap,
    master: &'a mut Master,
    files: &'a dyn Files,
    edges: &'a HashMap<String, EdgeType>,
    w: i32,
    h: i32,
}

impl Pass<'_> {
    fn point(&self, x: i32, y: i32) -> Option<&crate::generate::LatticePoint> {
        self.map.point_index(x, y).map(|i| &self.map.lattice[i])
    }

    fn height(&self, x: i32, y: i32) -> i32 {
        self.point(x, y).map_or(0, |p| p.height as i32)
    }

    /// A corner's ground, none for grounds with `.gt` flag 8.
    fn ground(&self, x: i32, y: i32) -> Option<String> {
        self.point(x, y)?.ground.clone().filter(|g| self.map.gt_flags(g) & 8 == 0)
    }

    fn occupied(&self, x: i32, y: i32) -> bool {
        self.map.index(x, y).is_some_and(|i| self.map.cells[i].is_some())
    }

    fn mark(&mut self, x: i32, y: i32) {
        if let Some(i) = self.map.point_index(x, y) {
            self.map.lattice[i].flags |= 8;
        }
    }

    /// Copy height and ground to an unset corner (flag 2; `0x141d0e130`).
    fn carry(&mut self, x: i32, y: i32, h: i32, g: &Option<String>) {
        let Some(i) = self.map.point_index(x, y) else { return };
        let p = &mut self.map.lattice[i];
        if p.flags & 1 == 0 {
            p.height = h as i8;
            p.flags |= 2;
            if let Some(g) = g {
                p.ground = Some(g.clone());
            }
            p.flags |= 1;
        }
    }

    fn set_flag(&self, x: i32, y: i32) -> bool {
        self.point(x, y).is_some_and(|p| p.flags & 2 != 0)
    }

    /// Margin corners take the nearest set corner's ground and height along the column and
    /// row; margin corners are unwalkable.
    fn extend_lattice(&mut self, (x0, y0, x1, y1): (i32, i32, i32, i32)) {
        for x in x0..=x1 {
            let mut yb = y1;
            while yb < self.h && self.set_flag(x, yb + 1) {
                yb += 1;
            }
            let (h, g) = (self.height(x, yb), self.point(x, yb).and_then(|p| p.ground.clone()));
            self.mark(x, yb);
            for y in y1 + 1..=self.h {
                self.mark(x, y);
            }
            for y in yb + 1..=self.h {
                self.carry(x, y, h, &g);
            }
            let mut ya = y0;
            while ya != 0 && self.set_flag(x, ya - 1) {
                ya -= 1;
            }
            let (h, g) = (self.height(x, ya), self.point(x, ya).and_then(|p| p.ground.clone()));
            self.mark(x, ya);
            for y in 0..ya {
                self.carry(x, y, h, &g);
            }
            for y in 0..y0 {
                self.mark(x, y);
            }
        }
        for y in 0..=self.h {
            let mut xa = x0;
            while xa != 0 && self.set_flag(xa - 1, y) {
                xa -= 1;
            }
            let (h, g) = (self.height(xa, y), self.point(xa, y).and_then(|p| p.ground.clone()));
            self.mark(xa, y);
            for x in 0..xa {
                self.carry(x, y, h, &g);
            }
            for x in 0..x0 {
                self.mark(x, y);
            }
            let mut xb = x1;
            while xb < self.w && self.set_flag(xb + 1, y) {
                xb += 1;
            }
            let (h, g) = (self.height(xb, y), self.point(xb, y).and_then(|p| p.ground.clone()));
            self.mark(xb, y);
            for x in x1 + 1..=self.w {
                self.mark(x, y);
            }
            for x in xb + 1..=self.w {
                self.carry(x, y, h, &g);
            }
        }
    }

    /// `0x141fd5cd0`: the edge type crossing side `side` of a placed cell, where the cell's
    /// offset along that side is `off` (mirrored on the far sides); for virtual edges, the split
    /// type whose exit crosses. Returns it with the (adjusted) offset.
    fn border_edge(&self, x: i32, y: i32, side: usize, mut off: i32) -> Option<(String, i32)> {
        let i = self.map.index(x, y)?;
        let p = self.map.piece_at[i]?;
        let piece = &self.map.pieces[p];
        let tdt = &self.master.tiles.tiles[piece.tile?].tdt;
        let key = TileKey::from_tdt(tdt).oriented(piece.orientation);
        let path = key.edges[side].clone()?;
        let et = self.edges.get(&path)?;
        let size = et.size as i32;
        if side == 2 {
            off = key.size.0 as i32 - size - off;
        } else if side == 3 {
            off = key.size.1 as i32 - size - off;
        }
        let within = |v: u8| off <= v as i32 / 3 && (v as i32 / 3) < size + off;
        if !et.virtual_edge {
            return within(key.exits[side]).then_some((path, off));
        }
        if within(key.exits[side]) {
            return Some((et.splits[0].0.clone(), off));
        }
        within(key.virtual_exits[side]).then(|| (et.splits[1].0.clone(), off))
    }

    /// `0x141d083d0`: a placed cell's tile exit on map side `side`.
    fn exit(&self, x: i32, y: i32, side: usize) -> i32 {
        let i = self.map.index(x, y).unwrap();
        let piece = &self.map.pieces[self.map.piece_at[i].unwrap()];
        let tdt = &self.master.tiles.tiles[piece.tile.unwrap()].tdt;
        let o = piece.orientation;
        let own = crate::edge_tiles::own_side_pub(o, side);
        let dim = if own & 1 == 0 { tdt.size.0 } else { tdt.size.1 } as i32;
        let v = tdt.edge_distances[own] as i32;
        if (4..8).contains(&o) && v < 3 * dim { 3 * dim - v - 1 } else { v }
    }

    /// An edge tile key across the border: edge `et` on the two sides `sides`, exits per side,
    /// corner grounds and heights (relative to their minimum, returned).
    #[allow(clippy::too_many_arguments)]
    fn edge_key(&self, et: &str, sides: [usize; 2], exits: [u8; 4], grounds: [Option<String>; 4], hs: [i32; 4]) -> (TileKey, i32) {
        let s = self.edges[et].size;
        let m = *hs.iter().min().unwrap();
        let mut key = TileKey { size: (s, s), exits, virtual_exits: [s * 3; 4], grounds, ..Default::default() };
        key.edges[sides[0]] = Some(et.to_string());
        key.edges[sides[1]] = Some(et.to_string());
        for k in 0..4 {
            key.heights[k] = (hs[k] - m) as u8;
        }
        (key, m)
    }

    /// The lookup with the client's fallback for flat edges: corners whose ground does not take
    /// heights (`.gt` flag 1) get height 0 (other fallbacks not ported).
    fn edge_candidates(&self, et: &str, key: &mut TileKey) -> Result<Vec<crate::tileset::Candidate>, GenError> {
        let list = self.master.tiles.candidates("", key);
        if !list.is_empty() {
            return Ok(list.to_vec());
        }
        if self.edges[et].height_change != 0 {
            return Err(GenError::Unsupported(format!("outer buffer: sloped edge {et}")));
        }
        for k in 0..4 {
            if key.grounds[k].as_ref().is_some_and(|g| self.map.gt_flags(g) & 1 == 0) {
                key.heights[k] = 0;
            }
        }
        Ok(self.master.tiles.candidates("", key).to_vec())
    }

    fn place_candidate(&mut self, x: i32, y: i32, c: crate::tileset::Candidate, height: i32) {
        let def = &self.master.tiles.tiles[c.tile];
        let (path, tdt) = (def.path.clone(), def.tdt.clone());
        self.map.place(x, y, &path, &tdt, c.orientation, height as i16, Some(c.tile));
    }

    /// The fill tile for a border stretch, unless the two grounds disagree.
    fn fill_tile(&mut self, g1: &Option<String>, g2: &Option<String>) -> Result<Option<(String, crate::tdt::Tdt, u8)>, GenError> {
        if g1 != g2 {
            let plain = |g: &Option<String>| g.as_ref().is_some_and(|g| self.map.gt_flags(g) & 5 == 0);
            if !(plain(g1) && plain(g2)) {
                return Ok(None);
            }
        }
        let Some((path, o)) = self.master.first_fill_tile(self.files, g1.as_deref())? else { return Ok(None) };
        let tdt = self.master.header(self.files, &path)?.clone();
        Ok(Some((path, tdt, if o == 8 { 0 } else { o })))
    }

    /// `0x141f9e5f0` with the map's ground check off: inside the map (one cell of slack) and
    /// every cell free.
    fn fits(&self, x: i32, y: i32, size: (u8, u8), o: u8) -> bool {
        let (w, h) = if o & 1 == 1 && o < 8 { (size.1 as i32, size.0 as i32) } else { (size.0 as i32, size.1 as i32) };
        x >= 0 && y >= 0 && x + w <= self.w && y + h <= self.h && (0..h).all(|dy| (0..w).all(|dx| !self.occupied(x + dx, y + dy)))
    }

    fn below_strip(&mut self, (x0, y0, x1, _): (i32, i32, i32, i32), rng: &mut TerrainRng) -> Result<(), GenError> {
        let mut x = x0;
        while x != 0 && self.occupied(x - 1, y0) {
            x -= 1;
        }
        while x <= x1 {
            let h0 = self.height(x, 0);
            let mut top = y0;
            if top == 0 {
                x += 1;
                continue;
            }
            while self.occupied(x, top - 1) {
                top -= 1;
                if top == 0 {
                    break;
                }
            }
            if top == 0 {
                x += 1;
                continue;
            }
            let off = self.map.index(x, top).and_then(|i| self.map.cells[i].as_ref()).map_or(0, |c| c.offset.0 as i32);
            if let Some((et, off)) = self.border_edge(x, top, 0, off).filter(|(e, _)| !self.edges[e].wildcard) {
                let s = self.edges[&et].size as i32;
                let e = (self.exit(x, top, 0) - off * 3) as u8;
                let full = (s * 3) as u8;
                let (ga, gb) = (self.ground(x, top), self.ground(x + s, top));
                let (ha, hb) = (self.height(x, top), self.height(x + s, top));
                let (mut key, m) = self.edge_key(&et, [0, 2], [e, full, full.wrapping_sub(e).wrapping_sub(1), full], [ga.clone(), gb.clone(), gb, ga], [ha, hb, hb, ha]);
                let list = self.edge_candidates(&et, &mut key)?;
                if list.is_empty() {
                    return Err(GenError::Unsupported(format!("no edge tile to expand the outer buffer down at ({x},{top})")));
                }
                let total = list.iter().map(|c| c.weight).sum();
                let c = pick(&list, rng.below(total)).unwrap();
                let mut y = 0;
                while y < top {
                    self.place_candidate(x, y, c, m);
                    y += s;
                }
                x += s;
            } else {
                let (g1, g2) = (self.ground(x, top), self.ground(x + 1, top));
                let Some((path, tdt, o)) = self.fill_tile(&g1, &g2)? else {
                    x += 1;
                    continue;
                };
                if tdt.size.0 as i32 + x <= self.w {
                    let mut y = 0;
                    while y < top {
                        if self.fits(x, y, tdt.size, o) {
                            self.map.place(x, y, &path, &tdt, o, h0 as i16, None);
                        }
                        y += tdt.size.1 as i32;
                    }
                }
                x += tdt.size.0 as i32;
            }
        }
        Ok(())
    }

    fn above_strip(&mut self, (x0, _, x1, y1): (i32, i32, i32, i32), rng: &mut TerrainRng) -> Result<(), GenError> {
        let mut x = x0;
        while x != 0 && self.occupied(x - 1, y1 - 1) {
            x -= 1;
        }
        'cols: while x < x1 {
            let h0 = self.height(x, self.h);
            let mut bottom = y1;
            if bottom >= self.h {
                x += 1;
                continue;
            }
            while self.occupied(x, bottom) {
                bottom += 1;
                if bottom >= self.h {
                    x += 1;
                    continue 'cols;
                }
            }
            let off = self.map.index(x, bottom - 1).and_then(|i| self.map.cells[i].as_ref()).map_or(0, |c| c.offset.0 as i32);
            if let Some((et, off)) = self.border_edge(x, bottom - 1, 2, off).filter(|(e, _)| !self.edges[e].wildcard) {
                let s = self.edges[&et].size as i32;
                let e = (self.exit(x, bottom - 1, 2) - off * 3) as u8;
                let full = (s * 3) as u8;
                let (ga, gb) = (self.ground(x, bottom), self.ground(x + s, bottom));
                let (ha, hb) = (self.height(x, bottom), self.height(x + s, bottom));
                let (mut key, m) = self.edge_key(&et, [0, 2], [full.wrapping_sub(e).wrapping_sub(1), full, e, full], [ga.clone(), gb.clone(), gb, ga], [ha, hb, hb, ha]);
                let list = self.edge_candidates(&et, &mut key)?;
                if list.is_empty() {
                    return Err(GenError::Unsupported(format!("no edge tile to expand the outer buffer up at ({x},{bottom})")));
                }
                let total = list.iter().map(|c| c.weight).sum();
                let c = pick(&list, rng.below(total)).unwrap();
                let mut y = bottom;
                while y + s <= self.h {
                    self.place_candidate(x, y, c, m);
                    y += s;
                }
                x += s;
            } else {
                let (g1, g2) = (self.ground(x, bottom), self.ground(x + 1, bottom));
                let Some((path, tdt, o)) = self.fill_tile(&g1, &g2)? else {
                    x += 1;
                    continue;
                };
                if tdt.size.0 as i32 + x <= self.w {
                    let mut y = bottom;
                    while y + tdt.size.1 as i32 <= self.h {
                        if self.fits(x, y, tdt.size, o) {
                            self.map.place(x, y, &path, &tdt, o, h0 as i16, None);
                        }
                        y += tdt.size.1 as i32;
                    }
                }
                x += tdt.size.0 as i32;
            }
        }
        Ok(())
    }

    /// Left of the region (side 3), rows top to bottom; the mirror of `right_strip`.
    fn left_strip(&mut self, (x0, _, _, _): (i32, i32, i32, i32), rng: &mut TerrainRng) -> Result<(), GenError> {
        let mut y = 0;
        while y < self.h {
            let h0 = self.height(x0, y);
            let mut x = x0;
            while x != 0 && self.occupied(x - 1, y) {
                x -= 1;
            }
            if x == 0 {
                y += 1;
                continue;
            }
            let off = self.map.index(x, y).and_then(|i| self.map.cells[i].as_ref()).map_or(0, |c| c.offset.1 as i32);
            if let Some((et, off)) = self.border_edge(x, y, 3, off).filter(|(e, _)| !self.edges[e].wildcard) {
                let s = self.edges[&et].size as i32;
                let e = (self.exit(x, y, 3) - off * 3) as u8;
                let full = (s * 3) as u8;
                let (ga, gb) = (self.ground(x, y), self.ground(x, y + s));
                let (ha, hb) = (self.height(x, y), self.height(x, y + s));
                let (mut key, m) = self.edge_key(&et, [1, 3], [full, full.wrapping_sub(e).wrapping_sub(1), full, e], [ga.clone(), ga, gb.clone(), gb], [ha, ha, hb, hb]);
                let list = self.edge_candidates(&et, &mut key)?;
                if list.is_empty() {
                    return Err(GenError::Unsupported(format!("no edge tile to expand the outer buffer left at ({x},{y})")));
                }
                let total = list.iter().map(|c| c.weight).sum();
                let c = pick(&list, rng.below(total)).unwrap();
                let mut xx = 0;
                while xx < x {
                    self.place_candidate(xx, y, c, m);
                    xx += s;
                }
                y += s;
            } else {
                let (g1, g2) = (self.ground(x, y), self.ground(x, y + 1));
                let Some((path, tdt, o)) = self.fill_tile(&g1, &g2)? else {
                    y += 1;
                    continue;
                };
                if tdt.size.1 as i32 + y <= self.h {
                    let mut xx = 0;
                    while xx < x {
                        if self.fits(xx, y, tdt.size, o) {
                            self.map.place(xx, y, &path, &tdt, o, h0 as i16, None);
                        }
                        xx += tdt.size.0 as i32;
                    }
                }
                y += tdt.size.1 as i32;
            }
        }
        Ok(())
    }

    fn right_strip(&mut self, (_, _, x1, _): (i32, i32, i32, i32), rng: &mut TerrainRng) -> Result<(), GenError> {
        let mut y = 0;
        while y < self.h {
            let h0 = self.height(x1, y);
            let mut x = x1;
            while x < self.w && self.occupied(x, y) {
                x += 1;
            }
            if x == self.w {
                y += 1;
                continue;
            }
            let off = self.map.index(x - 1, y).and_then(|i| self.map.cells[i].as_ref()).map_or(0, |c| c.offset.1 as i32);
            if let Some((et, off)) = self.border_edge(x - 1, y, 1, off).filter(|(e, _)| !self.edges[e].wildcard) {
                let s = self.edges[&et].size as i32;
                let e = (self.exit(x - 1, y, 1) - off * 3) as u8;
                let full = (s * 3) as u8;
                let (ga, gb) = (self.ground(x, y), self.ground(x, y + s));
                let (ha, hb) = (self.height(x, y), self.height(x, y + s));
                let (mut key, m) = self.edge_key(&et, [1, 3], [full, e, full, full.wrapping_sub(e).wrapping_sub(1)], [ga.clone(), ga, gb.clone(), gb], [ha, ha, hb, hb]);
                let list = self.edge_candidates(&et, &mut key)?;
                if list.is_empty() {
                    return Err(GenError::Unsupported(format!("no edge tile to expand the outer buffer right at ({x},{y})")));
                }
                let total = list.iter().map(|c| c.weight).sum();
                let c = pick(&list, rng.below(total)).unwrap();
                let mut xx = x;
                while xx < self.w {
                    self.place_candidate(xx, y, c, m);
                    xx += s;
                }
                y += s;
            } else {
                let (g1, g2) = (self.ground(x, y), self.ground(x, y + 1));
                let Some((path, tdt, o)) = self.fill_tile(&g1, &g2)? else {
                    y += 1;
                    continue;
                };
                if tdt.size.1 as i32 + y <= self.h {
                    let mut xx = x;
                    while tdt.size.0 as i32 + xx <= self.w {
                        if self.fits(xx, y, tdt.size, o) {
                            self.map.place(xx, y, &path, &tdt, o, h0 as i16, None);
                        }
                        xx += tdt.size.0 as i32;
                    }
                }
                y += tdt.size.1 as i32;
            }
        }
        Ok(())
    }
}

/// `0x141fd8360`: the region map placed inside its outer buffer and the margins tiled.
/// `outer_ground` is the master's `OuterGroundType`; `repair` levels its fill tiles (dungeons).
pub fn outer_buffer(
    region: &TileMap,
    margins: Margins,
    outer_ground: Option<&str>,
    repair: bool,
    master: &mut Master,
    files: &dyn Files,
    edges: &HashMap<String, EdgeType>,
    rng: &mut TerrainRng,
) -> Result<TileMap, GenError> {
    let (w, h) = (region.width + margins.left + margins.right, region.height + margins.top + margins.bottom);
    let mut map = region.embed((margins.left as i32, margins.top as i32), w, h);
    for p in &mut map.lattice {
        if p.flags & 1 != 0 {
            p.flags |= 2;
        }
    }
    let rect = (
        margins.left as i32,
        margins.top as i32,
        (margins.left + region.width) as i32,
        (margins.top + region.height) as i32,
    );
    if let Some(g) = outer_ground {
        fill_black(&mut map, rect, g, repair, master, files)?;
    }
    let mut pass = Pass { map: &mut map, master, files, edges, w: w as i32, h: h as i32 };
    pass.extend_lattice(rect);
    pass.below_strip(rect, rng)?;
    pass.above_strip(rect, rng)?;
    pass.left_strip(rect, rng)?;
    pass.right_strip(rect, rng)?;
    Ok(map)
}

/// `0x141fd5ff0` + `0x141fd8360` for a subgraph: `map` is its reserved window (the region at
/// `rect` inside its margins); corners set in the region count as height-set, then the outer
/// fill and the margin strips.
#[allow(clippy::too_many_arguments)]
pub fn outer_buffer_window(
    map: &mut TileMap,
    rect: (i32, i32, i32, i32),
    outer_ground: Option<&str>,
    repair: bool,
    master: &mut Master,
    files: &dyn Files,
    edges: &HashMap<String, EdgeType>,
    rng: &mut TerrainRng,
) -> Result<(), GenError> {
    for y in rect.1..=rect.3 {
        for x in rect.0..=rect.2 {
            if let Some(k) = map.point_index(x, y) {
                if map.lattice[k].flags & 1 != 0 {
                    map.lattice[k].flags |= 2;
                }
            }
        }
    }
    if let Some(g) = outer_ground {
        fill_black(map, rect, g, repair, master, files)?;
    }
    let (w, h) = (map.width as i32, map.height as i32);
    let mut pass = Pass { map, master, files, edges, w, h };
    pass.extend_lattice(rect);
    pass.below_strip(rect, rng)?;
    pass.above_strip(rect, rng)?;
    pass.left_strip(rect, rng)?;
    pass.right_strip(rect, rng)?;
    Ok(())
}

/// `0x141fd5ff0` on one graph's rectangle of an already embedded map (a subgraph's window):
/// set corners there count as height-set, then the outer ground's fill.
pub fn outer_fill(map: &mut TileMap, rect: (i32, i32, i32, i32), ground: &str, repair: bool, master: &mut Master, files: &dyn Files) -> Result<(), GenError> {
    for y in rect.1..=rect.3 {
        for x in rect.0..=rect.2 {
            if let Some(k) = map.point_index(x, y) {
                if map.lattice[k].flags & 1 != 0 {
                    map.lattice[k].flags |= 2;
                }
            }
        }
    }
    fill_black(map, rect, ground, repair, master, files)
}

/// `0x141fd5ff0`: region cells still empty (dungeon grid cells without a room) take the outer
/// ground's first fill tile (`0x141fd6990`; orientation "any" counts as 0). A 3×3 tile on a
/// region whose corner and size are 1 mod 3 is aligned one cell outside the region (and may
/// overhang it by one). With `repair` and an outer ground whose heights do not follow the tiles
/// (no `.gt` flag 1), each new tile takes the middle of its cell's set corner heights (cells
/// without any wait for their neighbours), and then those tiles and the fill's tiles of that
/// ground are levelled to the middle of their corners, again while some tile moved by more
/// than one storey (up to 15 rounds).
fn fill_black(map: &mut TileMap, rect: (i32, i32, i32, i32), ground: &str, repair: bool, master: &mut Master, files: &dyn Files) -> Result<(), GenError> {
    let Some((path, o)) = master.first_fill_tile(files, Some(ground))? else {
        return Err(GenError::Unsupported("no fill tile found for outer fill ground type".into()));
    };
    let o = if o == 8 { 0 } else { o };
    let tdt = master.header(files, &path)?.clone();
    let (tw, th) = TileMap::oriented_size(tdt.size, o);
    let (x0, y0, w, h) = (rect.0, rect.1, rect.2 - rect.0, rect.3 - rect.1);
    let aligned = (tw, th) == (3, 3) && x0 % 3 == 1 && y0 % 3 == 1 && w % 3 == 1 && h % 3 == 1;
    let a = aligned as i32;
    // `0x141fd6990`: inside the region (one cell of slack when aligned) and free.
    let fits = |map: &TileMap, x: i32, y: i32| {
        let (lo, hi_x, hi_y) = if aligned { (-1, w + 1, h + 1) } else { (0, w, h) };
        x >= lo && y >= lo && x + tw <= hi_x && y + th <= hi_y && (0..th).all(|dy| (0..tw).all(|dx| match map.index(x0 + x + dx, y0 + y + dy) {
            Some(i) => map.cells[i].is_none(),
            None => !map.spilled(x0 + x + dx, y0 + y + dy),
        }))
    };
    let ground = ground.to_ascii_lowercase();
    if !repair || map.gt_flags(&ground) & 1 != 0 {
        for y in -a..h - h % th {
            for x in -a..w - w % tw {
                if fits(map, x, y) {
                    map.place(x0 + x, y0 + y, &path, &tdt, o, 0, None);
                }
            }
        }
        return Ok(());
    }
    // The tile's corners take its height first.
    let place = |map: &mut TileMap, x: i32, y: i32, ht: i32| {
        if !fits(map, x, y) {
            return;
        }
        for py in y..=y + th {
            for px in x..=x + tw {
                if let Some(k) = map.point_index(x0 + px, y0 + py) {
                    map.lattice[k].height = ht as i8;
                    map.lattice[k].flags |= 2;
                }
            }
        }
        map.place(x0 + x, y0 + y, &path, &tdt, o, ht as i16, None);
    };
    let mut fixed: Vec<(i32, i32)> = Vec::new();
    let mut waiting: Vec<(i32, i32)> = Vec::new();
    let mut ready: Vec<(i32, i32, i32)> = Vec::new();
    // `0x141fd7060`: cells on the fill tile grid.
    let scan = |map: &TileMap, c: (i32, i32), waiting: &mut Vec<(i32, i32)>, ready: &mut Vec<(i32, i32, i32)>| {
        if (c.0 + a) % tw != 0 || (c.1 + a) % th != 0 {
            return;
        }
        match crate::generate::middle(&map.tile_corner_heights(x0 + c.0, y0 + c.1)) {
            None => waiting.push(c),
            Some(ht) => ready.push((c.0, c.1, ht)),
        }
    };
    for y in -a..h {
        for x in -a..w {
            let cell = map.index(x0 + x, y0 + y).and_then(|i| map.cells[i].as_ref());
            match cell {
                None => scan(map, (x, y), &mut waiting, &mut ready),
                Some(c) if c.path.eq_ignore_ascii_case(&path) => fixed.push((x, y)),
                Some(_) => {}
            }
        }
    }
    while !waiting.is_empty() {
        for (x, y, ht) in ready.drain(..) {
            place(map, x, y, ht);
            fixed.push((x, y));
        }
        let before = waiting.len();
        for c in std::mem::take(&mut waiting) {
            scan(map, c, &mut waiting, &mut ready);
        }
        if waiting.len() == before && ready.is_empty() {
            if std::env::var("POE_DBG").is_ok() {
                eprintln!("DBG stuck rect {:?} waiting {:?}", rect, waiting.iter().map(|c| (c.0 + x0, c.1 + y0)).collect::<Vec<_>>());
            }
            return Err(GenError::Unsupported("could not fix heights in dungeon".into()));
        }
    }
    for (x, y, ht) in ready.drain(..) {
        place(map, x, y, ht);
        fixed.push((x, y));
    }
    // `0x141fd71c0` rounds.
    for _ in 0..15 {
        let mut small = true;
        let mut changes: Vec<((i32, i32), i32)> = Vec::new();
        for &(x, y) in &fixed {
            let ht = crate::generate::middle(&map.tile_corner_heights(x0 + x, y0 + y)).unwrap_or(0);
            let cur = map.index(x0 + x, y0 + y).and_then(|i| map.cells[i].as_ref()).map_or(0, |c| c.height as i32);
            if ht != cur {
                changes.push(((x, y), ht));
                if (ht - cur).abs() > 1 {
                    small = false;
                }
            }
        }
        if changes.is_empty() {
            break;
        }
        for ((x, y), ht) in changes {
            if map.index(x0 + x, y0 + y).and_then(|i| map.cells[i].as_ref()).is_some_and(|c| c.offset == (0, 0)) {
                map.set_heights(x0 + x, y0 + y, &tdt, ht as i16);
            }
        }
        if small {
            break;
        }
    }
    Ok(())
}
