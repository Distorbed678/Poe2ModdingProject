//! The corner lattice passes between edge tiles and the fill pass (`0x1420bec40`): grounds
//! flood out from tile borders into empty space, stopped by tile edges. Findings:
//! `docs/re/terrain/shared-passes.md`, "Corner lattice".

use std::collections::VecDeque;

use crate::{
    generate::TileMap,
    tileset::{TileKey, TileSet},
};

/// Lattice steps per direction (`0x142f0c608`): north, east, south, west.
const DIR: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// The tile on a cell: its origin, oriented size, offset of the cell and piece index.
fn tile_at(map: &TileMap, x: i32, y: i32) -> Option<((i32, i32), (i32, i32), (i32, i32), usize)> {
    if x < 0 || y < 0 || x as u32 >= map.width || y as u32 >= map.height {
        return None;
    }
    let i = map.index(x, y)?;
    let c = map.cells[i].as_ref()?;
    let p = map.piece_at[i]?;
    let piece = &map.pieces[p];
    let size = TileMap::oriented_size(piece.size, piece.orientation);
    let off = (c.offset.0 as i32, c.offset.1 as i32);
    Some(((x - off.0, y - off.1), size, off, p))
}

/// `0x141fe0210`: the corner lies strictly inside a placed tile.
fn inside(map: &TileMap, p: (i32, i32)) -> bool {
    tile_at(map, p.0, p.1).is_some_and(|(o, (w, h), _, _)| o.0 < p.0 && p.0 < o.0 + w && o.1 < p.1 && p.1 < o.1 + h)
}

/// `0x141fdffe0`: the tile's edge on `side` crosses that side of this cell.
fn edge_crosses(map: &TileMap, tiles: &TileSet, cell: (i32, i32), side: usize) -> bool {
    let Some((_, (w, h), off, p)) = tile_at(map, cell.0, cell.1) else { return false };
    let piece = &map.pieces[p];
    let Some(tile) = piece.tile else { return false };
    let tdt = &tiles.tiles[tile].tdt;
    let o = piece.orientation;
    let key = TileKey::from_tdt(tdt).oriented(o);
    let pos = match side {
        0 => off.0,
        1 => off.1,
        2 => w - 1 - off.0,
        _ => h - 1 - off.1,
    };
    let Some(edge) = &key.edges[side] else { return false };
    let flags = map.edge_flags.get(edge).copied().unwrap_or(0);
    if flags & 0x80 != 0 {
        return false;
    }
    if key.exits[side] as i32 / 3 == pos {
        return true;
    }
    flags & 4 != 0 && crate::edge_tiles::virtual_exit(tdt, o, side) / 3 == pos
}

/// `0x141fe0810` ("EdgeInTheWay"): a tile edge crosses the lattice segment `from → to`.
fn edge_in_the_way(map: &TileMap, tiles: &TileSet, from: (i32, i32), to: (i32, i32), dir: usize) -> bool {
    if inside(map, from) {
        return !inside(map, to);
    }
    if inside(map, to) {
        return false;
    }
    let c = |p: (i32, i32), s: usize| edge_crosses(map, tiles, p, s);
    match dir {
        0 => c(to, 3) || c((to.0 - 1, to.1), 1),
        1 => c(from, 0) || c((from.0, from.1 - 1), 2),
        2 => c(from, 3) || c((from.0 - 1, from.1), 1),
        _ => c(to, 0) || c((to.0, to.1 - 1), 2),
    }
}

fn in_lattice(map: &TileMap, p: (i32, i32)) -> bool {
    p.0 >= 0 && p.1 >= 0 && p.0 as u32 <= map.width && p.1 as u32 <= map.height
}

/// `0x1420b9180`: a breadth-first flood of the ground at border corner `p` outward in `dir`
/// (and to either side), over corners no tile set; grounds with `.gt` flag 8 also cover tile
/// corners without a ground.
fn flood(map: &mut TileMap, tiles: &TileSet, p: (i32, i32), dir: usize) {
    let Some(i) = map.point_index(p.0, p.1) else { return };
    let Some(g) = map.lattice[i].ground.clone() else { return };
    let gf = map.gt_flags(&g);
    if gf & 2 == 0 {
        return;
    }
    let blank = gf & 8 != 0;
    let mut queue = VecDeque::new();
    let q = (p.0 + DIR[dir].0, p.1 + DIR[dir].1);
    if in_lattice(map, q) && !edge_in_the_way(map, tiles, p, q, dir) {
        queue.push_back((q, dir));
    }
    while let Some((a, d)) = queue.pop_front() {
        let k = map.point_index(a.0, a.1).unwrap();
        let pt = &map.lattice[k];
        let free = pt.flags & 1 == 0 || (blank && pt.ground.is_none());
        if !free {
            continue;
        }
        let pt = &mut map.lattice[k];
        pt.ground = Some(g.clone());
        pt.flags |= 1;
        for nd in [d, (d + 1) & 3, (d + 3) & 3] {
            let n = (a.0 + DIR[nd].0, a.1 + DIR[nd].1);
            if in_lattice(map, n) && !edge_in_the_way(map, tiles, a, n, nd) {
                queue.push_back((n, nd));
            }
        }
    }
}

/// `0x1420b94d0`: floods from every border corner of every placed tile (columns, then rows;
/// per tile its top and bottom corners left to right, then its left and right corners).
pub fn border_floods(map: &mut TileMap, tiles: &TileSet) {
    for x in 0..map.width as i32 {
        for y in 0..map.height as i32 {
            let Some((o, (w, h), off, _)) = tile_at(map, x, y) else { continue };
            if off != (0, 0) || o != (x, y) {
                continue;
            }
            for i in 0..=w {
                flood(map, tiles, (x + i, y), 0);
                flood(map, tiles, (x + i, y + h), 2);
            }
            for j in 0..=h {
                flood(map, tiles, (x, y + j), 3);
                flood(map, tiles, (x + w, y + j), 1);
            }
        }
    }
}

const IMPASSABLE: &str = "metadata/terrain/impassable_ground.gt";

/// State of the unwalkable marking (`0x141fe0fd0`): visited corners, the flood queue and the
/// corners whose neighbours get marked when the queue runs dry.
struct Unwalkable<'a> {
    map: &'a mut TileMap,
    tiles: &'a TileSet,
    visited: Vec<bool>,
    queue: VecDeque<((i32, i32), usize)>,
    edge_points: Vec<(i32, i32)>,
}

impl Unwalkable<'_> {
    fn ground(&self, p: (i32, i32)) -> Option<&String> {
        self.map.point_index(p.0, p.1).and_then(|i| self.map.lattice[i].ground.as_ref())
    }

    /// A ground that stops the impassable flood: set, without `.gt` flag 4, not impassable.
    fn stops(&self, p: (i32, i32)) -> bool {
        self.ground(p).is_some_and(|g| self.map.gt_flags(g) & 4 == 0 && g != IMPASSABLE)
    }

    fn mark(&mut self, p: (i32, i32)) {
        if let Some(i) = self.map.point_index(p.0, p.1) {
            self.map.lattice[i].flags |= 8;
        }
    }

    /// `+0x16f` bit 0 of the tile a corner lies inside (not read from `.tdt` files yet).
    fn sealed_inside(&self, p: (i32, i32)) -> bool {
        inside(self.map, p) && false
    }

    /// `0x141fe2380`: start the flood at a border corner whose ground does not stop it.
    fn start(&mut self, p: (i32, i32), dir: usize) {
        if self.stops(p) {
            return;
        }
        let q = (p.0 + DIR[dir].0, p.1 + DIR[dir].1);
        if self.sealed_inside(q) {
            return;
        }
        self.queue.push_back((p, dir));
        self.run();
    }

    /// `0x141fe1df0`: look one step on; a stopping ground is marked (and its neighbours later),
    /// anything else joins the flood unless an edge is in the way.
    fn step(&mut self, p: (i32, i32), dir: usize) {
        let n = (p.0 + DIR[dir].0, p.1 + DIR[dir].1);
        if !in_lattice(self.map, n) || self.sealed_inside(n) {
            return;
        }
        if self.stops(n) {
            self.mark(n);
            self.edge_points.push(n);
            return;
        }
        if !edge_in_the_way(self.map, self.tiles, p, n, dir) {
            self.queue.push_back((n, dir));
        }
    }

    /// `0x141fe2100`: the impassable flood (corners take the impassable ground where they have
    /// none), then the stopping corners' walkable neighbours are marked (`0x141fe1fe0`).
    fn run(&mut self) {
        while let Some((p, d)) = self.queue.pop_front() {
            let k = self.map.point_index(p.0, p.1).unwrap();
            if self.visited[k] {
                continue;
            }
            self.visited[k] = true;
            let pt = &mut self.map.lattice[k];
            if pt.ground.is_none() || pt.flags & 1 == 0 {
                pt.ground = Some(IMPASSABLE.to_string());
                pt.flags |= 1;
            }
            pt.flags |= 8;
            self.step(p, d);
            self.step(p, (d + 3) & 3);
            self.step(p, (d + 1) & 3);
        }
        for (x, y) in std::mem::take(&mut self.edge_points) {
            for n in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                if in_lattice(self.map, n) && self.ground(n).is_some_and(|g| self.map.gt_flags(g) & 4 == 0) {
                    self.mark(n);
                }
            }
        }
    }
}

/// `0x141fe0fd0`: lattice flag 8 marks corners nobody can walk on: the impassable flood from
/// the border, the border itself, impassable grounds inside, and one ring of corners next to
/// marked ones (unless their ground has `.gt` flag 4). Area transition tiles (`0x141fe0b40`)
/// and the tile-set option at `+0x165` are not ported yet (nothing in the Riverbank).
pub fn mark_unwalkable(map: &mut TileMap, tiles: &TileSet) {
    let (w, h) = (map.width as i32, map.height as i32);
    let n = map.lattice.len();
    let mut s = Unwalkable { map, tiles, visited: vec![false; n], queue: VecDeque::new(), edge_points: Vec::new() };
    for x in 0..=w {
        s.start((x, 0), 2);
        s.start((x, h), 0);
    }
    for y in 0..=h {
        s.start((0, y), 1);
        s.start((w, y), 3);
    }
    for x in 0..=w {
        s.mark((x, 0));
        s.mark((x, h));
    }
    for y in 0..=h {
        s.mark((0, y));
        s.mark((w, y));
    }
    for y in 1..h {
        for x in 1..w {
            let k = s.map.point_index(x, y).unwrap();
            if s.ground((x, y)).is_some_and(|g| g == IMPASSABLE) && s.map.lattice[k].flags & 8 == 0 {
                s.mark((x, y));
                s.visited[k] = true;
                for d in 0..4 {
                    s.step((x, y), d);
                }
                s.run();
            }
        }
    }
    let marked = |m: &TileMap, p: (i32, i32)| m.point_index(p.0, p.1).is_some_and(|i| m.lattice[i].flags & 8 != 0);
    let mut ring = Vec::new();
    for y in 1..h {
        for x in 1..w {
            let k = s.map.point_index(x, y).unwrap();
            let Some(g) = s.ground((x, y)) else { continue };
            if s.map.gt_flags(g) & 4 != 0 || s.map.lattice[k].flags & 8 != 0 {
                continue;
            }
            if [(x, y - 1), (x - 1, y), (x + 1, y), (x, y + 1)].iter().any(|&p| marked(s.map, p)) {
                ring.push((x, y));
            }
        }
    }
    for p in ring {
        s.mark(p);
    }
}

/// `0x1420b9820` (height offsets not ported: the Riverbank has none): every corner is set; one
/// without a ground (or never set) takes the graph's default ground (graph +0xb0) if any.
pub fn settle(map: &mut TileMap, default_ground: Option<&str>) {
    for p in &mut map.lattice {
        if p.flags & 1 == 0 || p.ground.is_none() {
            if let Some(g) = default_ground {
                p.ground = Some(g.to_ascii_lowercase());
            }
            p.flags |= 1;
        }
    }
}
