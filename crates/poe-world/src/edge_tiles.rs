//! Edge tiles (`0x1420b6950`): each chain of layout-graph edges between rooms becomes a path
//! of edge tiles, found by a tile-level search over an occupancy grid, then each path tile gets
//! an edge tile from the tile set. Findings: `docs/re/terrain/tgr.md`, "Edge tiles".

use std::collections::{BTreeSet, HashMap};

use crate::{
    generate::TileMap,
    rng::TerrainRng,
    smoothing::{LayoutGraph, Pos},
    tileset::{pick, TileKey, TileSet},
    voronoi::isqrt,
};

/// What edge tiling needs of an edge's `.et`.
#[derive(Debug, Clone, Default)]
pub struct EdgeInfo {
    /// `.et` path (lower case), the edge slots of the tile key.
    pub path: String,
    /// `+0x4b`: edge tile size (1 or 3).
    pub size: u32,
    /// `+0x20`/`+0x28`: left and right ground (`None` = wildcard).
    pub grounds: [Option<String>; 2],
    /// `+0x4c` (the first number): the right ground's height in the key.
    pub height: u8,
    /// Flag 8 (third number): edges cross tile sides in the middle.
    pub centred: bool,
    /// `+0x4d`: crossing positions from a coordinate hash (not met yet; 0 for the Riverbank).
    pub hashed: bool,
}

/// A chain of edges turned into a tile path (the client's 0xb8-byte path record).
#[derive(Debug, Clone, Default)]
pub struct PathInfo {
    /// Path tiles (grid cells, rounded to the edge tile size); filled by the search.
    pub tiles: Vec<(i32, i32)>,
    /// The chain's points: its start node, the edge points, junction nodes, its end node.
    pub points: Vec<Pos>,
    /// Per segment (`points[i] → points[i + 1]`): whether its edge runs backwards.
    pub bits: Vec<bool>,
    pub final_node: usize,
    pub edge: usize,
    pub end: usize,
    /// The chain ends where it started.
    pub is_loop: bool,
    /// Off-map tiles before the first and after the last point (`+0x68`/`+0x70`).
    pub start_off: Option<(i32, i32)>,
    pub end_off: Option<(i32, i32)>,
    /// The chain's edges (an unordered set in the client; only used for commutative updates).
    pub edges: Vec<usize>,
}

/// One cell of the occupancy grid (`0x141ef5700`, 0x20-byte records).
#[derive(Debug, Clone, Default)]
struct Cell {
    /// `+0x0`: path cost; edges add 80 per tile they cross.
    cost: u32,
    /// `+0x4`: search parent (index + 1).
    parent: u32,
    /// `+0x8`, `+0xc`, `+0x10`: search total, cost so far, estimate.
    f: u32,
    g: u32,
    h: u32,
    /// `+0x14`: closed in the search.
    closed: bool,
    /// `+0x15`: taken by a room or a path tile.
    occupied: bool,
    /// `+0x18`: in the open set.
    open: bool,
}

/// The client's line walk (`0x141f05980` and its copies): steps along the major axis from `a`
/// to `b` inclusive.
fn line(a: Pos, b: Pos, mut f: impl FnMut(Pos)) {
    let steep = (b.0 - a.0).abs() < (b.1 - a.1).abs();
    let (a0, a1, b0, b1) = if steep { (a.1, b.1, a.0, b.0) } else { (a.0, b.0, a.1, b.1) };
    let da = (a1 - a0).abs();
    let db = (b1 - b0).abs();
    let bstep = (b1 - b0).signum();
    let astep = if a0 < a1 { 1 } else { -1 };
    let mut err = -(da / 2);
    let (mut x, mut y) = (a0, b0);
    while x != a1 + astep {
        f(if steep { (y, x) } else { (x, y) });
        x += astep;
        let e = err + db;
        if e < 1 {
            err = e;
        } else {
            err = e - da;
            y += bstep;
        }
    }
}

/// Grid cell of a world position, rounded down to a multiple of `size` (C division).
fn tile_of(p: Pos, size: i32) -> (i32, i32) {
    let (x, y) = (p.0 / 23, p.1 / 23);
    (x - x % size, y - y % size)
}

/// Tile key in the path builder (`0x1420b4510`): like [`tile_of`] but valid a little off the map.
fn path_key(p: Pos, size: i32) -> (i32, i32) {
    let k = |v: i32| {
        let q = ((v as i64 + 23 * size as i64) as u64 / 23) as i32;
        q - q % size - size
    };
    (k(p.0), k(p.1))
}

/// The edge tile stage's state (`0x1420b6950`'s object): occupancy grid, pending paths.
pub struct EdgeTiler<'a> {
    pub g: &'a mut LayoutGraph,
    map: &'a mut TileMap,
    tileset: &'a TileSet,
    edges: Vec<EdgeInfo>,
    w: u32,
    h: u32,
    cells: Vec<Cell>,
    open: BTreeSet<(u32, u32)>,
    /// `+0x40`: the current edge tile size.
    size: u32,
    /// `+0x30`: the search's end tile (exempt from the occupied test).
    end_tile: (u32, u32),
    /// Edge tile size per edge (`.et` size).
    sizes: Vec<u32>,
    /// `+0x6c` per edge: already part of a path.
    tilised: Vec<bool>,
    pending: Vec<PathInfo>,
    placed: Vec<PathInfo>,
    /// Paths in the order the client tiles them.
    pub order: Vec<PathInfo>,
}

impl<'a> EdgeTiler<'a> {
    /// `0x141ef5700`: cells under room tiles start occupied.
    pub fn new(g: &'a mut LayoutGraph, map: &'a mut TileMap, tileset: &'a TileSet, edges: Vec<EdgeInfo>) -> Self {
        let (w, h) = (map.width, map.height);
        let mut cells = vec![Cell::default(); (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let (Some(c), Some(p)) = (&map.cells[i], map.piece_at[i]) else { continue };
                if c.offset != (0, 0) {
                    continue;
                }
                let (pw, ph) = oriented(map.pieces[p].size, map.pieces[p].orientation);
                for j in 0..ph {
                    for k in 0..pw {
                        // No bounds check in the client: rows wrap.
                        if let Some(c) = cells.get_mut(((y + j) * w + x + k) as usize) {
                            c.occupied = true;
                        }
                    }
                }
            }
        }
        let n = g.edges.len();
        let sizes = edges.iter().map(|e| e.size).collect();
        Self {
            g,
            map,
            tileset,
            edges,
            w,
            h,
            cells,
            open: BTreeSet::new(),
            size: 1,
            end_tile: (0, 0),
            sizes,
            tilised: vec![false; n],
            pending: Vec::new(),
            placed: Vec::new(),
            order: Vec::new(),
        }
    }

    fn idx(&self, x: u32, y: u32) -> usize {
        (y * self.w + x) as usize
    }

    fn block(&mut self, x: u32, y: u32, s: u32, mut f: impl FnMut(&mut Cell)) {
        for j in 0..s {
            for i in 0..s {
                let k = ((y + j) * self.w + x + i) as usize;
                if let Some(c) = self.cells.get_mut(k) {
                    f(c);
                }
            }
        }
    }

    /// Point `i` of an edge counting both end nodes (0 = from, `len + 1` = to).
    fn pos(&self, e: usize, i: usize) -> Pos {
        let ed = &self.g.edges[e];
        if i == 0 {
            self.g.nodes[ed.from].pos
        } else if i > ed.points.len() {
            self.g.nodes[ed.to].pos
        } else {
            ed.points[i - 1]
        }
    }

    fn degree(&self, n: usize) -> usize {
        self.g.nodes[n].slots.iter().flatten().count()
    }

    /// Walks an edge's line tile by tile (size-rounded, inside the map), calling `f` on each
    /// new tile (`0x141ef6ab0`, `0x141ef6bf0`: one "last tile" per edge).
    fn edge_tiles(&self, e: usize, s: i32) -> Vec<(i32, i32)> {
        let n = self.g.edges[e].points.len();
        let mut out = Vec::new();
        let mut last = (-1, -1);
        for i in 0..=n {
            let (a, b) = (self.pos(e, i), self.pos(e, i + 1));
            line(a, b, |p| {
                if p.0 >= 0 && p.1 >= 0 && p.0 < self.w as i32 * 23 && p.1 < self.h as i32 * 23 {
                    let t = tile_of(p, s);
                    if t != last {
                        last = t;
                        out.push(t);
                    }
                }
            });
        }
        out
    }

    /// `0x141ef6ab0`: every tile an edge crosses costs 80 more.
    fn add_edge_cost(&mut self, e: usize) {
        let s = self.sizes[e];
        for (x, y) in self.edge_tiles(e, s as i32) {
            self.block(x as u32, y as u32, s, |c| c.cost += 80);
        }
    }

    /// `0x141ef6bf0`: a path's own edges are cheaper for its search (−60 on tiles at 80+).
    fn ease_edge_cost(&mut self, e: usize) {
        let s = self.size;
        for (x, y) in self.edge_tiles(e, s as i32) {
            self.block(x as u32, y as u32, s, |c| {
                if c.cost > 0x4f {
                    c.cost -= 60;
                }
            });
        }
    }

    /// `0x141ef7000`: the segment just passed gets 20 cheaper (drawn in the edge's direction).
    fn ease_segment(&mut self, a: Pos, b: Pos, s: u32, backwards: bool) {
        let (a, b) = if backwards { (b, a) } else { (a, b) };
        let mut tiles = Vec::new();
        let mut last = (-1, -1);
        line(a, b, |p| {
            if p.0 >= 0 && p.1 >= 0 && p.0 < self.w as i32 * 23 && p.1 < self.h as i32 * 23 {
                let t = tile_of(p, s as i32);
                if t != last {
                    last = t;
                    tiles.push(t);
                }
            }
        });
        for (x, y) in tiles {
            self.block(x as u32, y as u32, s, |c| {
                if c.cost > 0x13 {
                    c.cost -= 20;
                }
            });
        }
    }

    /// `0x141ef67a0`: the end tile next to a room is taken.
    fn take_end(&mut self, e: usize, end: usize) {
        let s = self.sizes[e];
        let ed = &self.g.edges[e];
        let p = if self.g.endpoint(e, end) == ed.from {
            ed.points.first().copied().unwrap_or(self.g.nodes[ed.to].pos)
        } else {
            ed.points.last().copied().unwrap_or(self.g.nodes[ed.from].pos)
        };
        let (x, y) = tile_of(p, s as i32);
        if (x as u32) < self.w && (y as u32) < self.h {
            self.block(x as u32, y as u32, s, |c| {
                c.occupied = true;
                c.cost += 80;
            });
        }
    }

    /// Room tile corner under a cell (`0x141ef5a30`), or the cell itself.
    fn room_corner(&self, t: (u32, u32)) -> (u32, u32) {
        if t.0 < self.w && t.1 < self.h {
            if let Some(c) = &self.map.cells[self.idx(t.0, t.1)] {
                return (t.0 - c.offset.0 as u32, t.1 - c.offset.1 as u32);
            }
        }
        t
    }

    /// `0x141d0d880`: no room tile in the block.
    fn free_of_rooms(&self, x: u32, y: u32, s: u32) -> bool {
        (0..s).all(|j| (0..s).all(|i| self.map.cells.get(self.idx(x + i, y + j)).is_some_and(|c| c.is_none())))
    }

    fn block_occupied(&self, x: u32, y: u32, s: u32) -> bool {
        (0..s).any(|j| (0..s).any(|i| self.cells.get(self.idx(x + i, y + j)).is_some_and(|c| c.occupied)))
    }

    /// `0x141ef5ab0`: average cost over the block (plus one).
    fn avg_cost(&self, idx: usize) -> u32 {
        let s = self.size;
        let mut sum = 1u32;
        for j in 0..s {
            for i in 0..s {
                sum = sum.wrapping_add(self.cells[idx + (j * self.w + i) as usize].cost);
            }
        }
        sum / (s * s)
    }

    fn open_insert(&mut self, idx: usize) {
        self.cells[idx].open = true;
        self.open.insert((self.cells[idx].f, idx as u32));
    }

    fn open_remove(&mut self, idx: usize) {
        self.cells[idx].open = false;
        self.open.remove(&(self.cells[idx].f, idx as u32));
    }

    /// `0x141ef5c80`: a search from the seed points (`seeds`, the rest of the path) back to
    /// the tile of `from`; on success the tiles from `from` to the seed reached are appended.
    fn search(&mut self, p: &mut PathInfo, from: Pos, seeds: &[Pos], rng: &mut TerrainRng) -> bool {
        let s = self.size;
        let gt = tile_of(from, s as i32);
        let goal = self.room_corner((gt.0 as u32, gt.1 as u32));
        let st = tile_of(seeds[0], s as i32);
        let first = self.room_corner((st.0 as u32, st.1 as u32));
        for c in &mut self.cells {
            c.parent = 0;
            c.g = 0;
            c.h = 0;
            c.closed = false;
            c.open = false;
        }
        self.open.clear();
        if goal == first {
            self.reconstruct(p, goal);
            return true;
        }
        let goal_idx = goal.1 * self.w + goal.0;
        for (k, &pt) in seeds.iter().enumerate() {
            let t = tile_of(pt, s as i32);
            let (x, y) = self.room_corner((t.0 as u32, t.1 as u32));
            if !(x < self.w && y < self.h && (x as i32) < self.w as i32 && x + s <= self.w && (y as i32) < self.h as i32 && y + s <= self.h) {
                continue;
            }
            if !self.free_of_rooms(x, y, s) {
                continue;
            }
            if (x, y) != self.end_tile && self.block_occupied(x, y, s) {
                continue;
            }
            let idx = self.idx(x, y);
            let g0 = if k != 0 { self.avg_cost(idx) * 100 + (seeds.len() - k) as u32 } else { 0 };
            let dx = (goal.0 as i32 - x as i32).abs();
            let dy = (goal.1 as i32 - y as i32).abs();
            let h = isqrt((dx * dx + dy * dy) as u32) * 100;
            if self.cells[idx].open {
                if self.cells[idx].f <= h + g0 {
                    continue;
                }
                self.open_remove(idx);
            }
            self.cells[idx].g = g0;
            self.cells[idx].h = h;
            self.cells[idx].f = g0 + h;
            self.open_insert(idx);
        }
        let r = |rng: &mut TerrainRng| {
            let v = rng.below(0x4000) as u64 + 20000;
            (v + 1) * (v + 1) + v * v
        };
        let r1 = r(rng);
        let r2 = r(rng);
        while let Some(&(f, cur)) = self.open.iter().next() {
            if cur == goal_idx {
                self.reconstruct(p, goal);
                return true;
            }
            self.open.remove(&(f, cur));
            let cur = cur as usize;
            self.cells[cur].open = false;
            self.cells[cur].closed = true;
            let (cx, cy) = (cur as u32 % self.w, cur as u32 / self.w);
            for nb in self.neighbours(cx, cy) {
                self.relax(cur, nb, goal, goal_idx, r1, r2);
            }
        }
        false
    }

    /// `0x141ef64f0`: the tiles around a cell (or around the whole room tile on it).
    fn neighbours(&self, x: u32, y: u32) -> Vec<(u32, u32)> {
        let s = self.size as i32;
        let (w, h) = match (&self.map.cells[self.idx(x, y)], self.map.piece_at[self.idx(x, y)]) {
            (Some(_), Some(p)) => oriented(self.map.pieces[p].size, self.map.pieces[p].orientation),
            _ => (self.size, self.size),
        };
        let (x, y, w, h) = (x as i32, y as i32, w as i32, h as i32);
        let mut out = Vec::new();
        let mut i = 0;
        while i < w {
            out.push((x + i, y - s));
            out.push((x + i, y + h));
            i += s;
        }
        let mut j = 0;
        while j < h {
            out.push((x - s, y + j));
            out.push((x + w, y + j));
            j += s;
        }
        out.into_iter().map(|(a, b)| self.room_corner((a as u32, b as u32))).collect()
    }

    /// `0x141ef6250`: try reaching `nb` from `cur`.
    fn relax(&mut self, cur: usize, nb: (u32, u32), goal: (u32, u32), goal_idx: u32, r1: u64, r2: u64) {
        let s = self.size;
        let (x, y) = nb;
        if !(x < self.w && y < self.h) {
            return;
        }
        let idx = self.idx(x, y);
        if self.cells[idx].closed || self.cells[cur].parent == idx as u32 + 1 {
            return;
        }
        if !((x as i32) < self.w as i32 && x + s <= self.w && (y as i32) < self.h as i32 && y + s <= self.h) {
            return;
        }
        if !self.free_of_rooms(x, y, s) {
            return;
        }
        if idx as u32 != goal_idx && self.block_occupied(x, y, s) {
            return;
        }
        let avg = self.avg_cost(idx);
        let v = (x as u64 + 1).wrapping_mul(r1).wrapping_add((y as u64 + 1).wrapping_mul(r2));
        let jitter = (v % 707 / 101) as u32;
        let g = (avg + jitter + s) * 100 + self.cells[cur].g;
        if !self.cells[idx].open {
            let dx = (goal.0 as i32 - x as i32).abs();
            let dy = (goal.1 as i32 - y as i32).abs();
            self.cells[idx].h = isqrt((dx * dx + dy * dy) as u32);
        } else {
            if self.cells[idx].g <= g {
                return;
            }
            self.open_remove(idx);
        }
        self.cells[idx].g = g;
        self.cells[idx].f = self.cells[idx].h + g;
        self.open_insert(idx);
        self.cells[idx].parent = cur as u32 + 1;
    }

    /// `0x141ef6900`: append the goal and its parents; their blocks become taken.
    fn reconstruct(&mut self, p: &mut PathInfo, goal: (u32, u32)) {
        let s = self.size;
        let gt = (goal.0 as i32, goal.1 as i32);
        if p.tiles.last() != Some(&gt) {
            p.tiles.push(gt);
            self.block(goal.0, goal.1, s, |c| {
                c.occupied = true;
                c.cost += 80;
            });
        }
        let mut idx = self.idx(goal.0, goal.1);
        while self.cells[idx].parent != 0 {
            idx = self.cells[idx].parent as usize - 1;
            let (x, y) = (idx as u32 % self.w, idx as u32 / self.w);
            self.block(x, y, s, |c| {
                c.occupied = true;
                c.cost += 80;
            });
            p.tiles.push((x as i32, y as i32));
        }
    }

    /// Clamped node tile, moved to the room tile corner (as `0x1420b5250` does for path ends).
    fn node_tile(&self, n: usize, s: i32) -> (i32, i32) {
        let (x, y) = self.g.nodes[n].pos;
        let x = x.max(0).min(self.w as i32 * 23 - 1);
        let y = y.max(0).min(self.h as i32 * 23 - 1);
        let t = tile_of((x, y), s);
        let c = self.room_corner((t.0 as u32, t.1 as u32));
        (c.0 as i32, c.1 as i32)
    }

    fn adjusted(&self, p: Pos, s: i32) -> (u32, u32) {
        let t = tile_of(p, s);
        self.room_corner((t.0 as u32, t.1 as u32))
    }

    /// `0x1420b5250`: find the path's tiles, search by search, backtracking when stuck.
    fn fit(&mut self, p: &mut PathInfo, rng: &mut TerrainRng) -> bool {
        if p.points.len() < 2 || p.bits.is_empty() {
            return false;
        }
        let s = self.sizes[p.edge];
        let si = s as i32;
        p.tiles.clear();
        // Backtracking stacks: the point index each step started from and the tile count.
        let mut at: Vec<usize> = Vec::new();
        let mut counts: Vec<usize> = Vec::new();
        let mut i = 0;
        if !p.is_loop {
            let start = self.g.endpoint(p.edge, p.end);
            if let Some(t) = p.start_off {
                p.tiles.push(t);
            }
            p.tiles.push(self.node_tile(start, si));
            i = 1;
            at.push(1);
            counts.push(p.tiles.len());
        }
        let n = p.points.len();
        self.end_tile = self.adjusted(p.points[n - i - 1], si);
        self.size = s;
        for e in p.edges.clone() {
            self.ease_edge_cost(e);
        }
        let mut next = i + 1;
        self.ease_segment(p.points[next - 1], p.points[next], s, p.bits[i]);
        let mut floor = 0;
        let mut floor_next = 0;
        let mut ok;
        loop {
            let end = if p.is_loop && next == n - 1 { n } else { n - 1 };
            let seeds: Vec<Pos> = p.points[next.min(end)..end].to_vec();
            ok = !seeds.is_empty() && self.search(p, p.points[i], &seeds, rng);
            if !ok {
                if at.is_empty() {
                    return false;
                }
                at.pop();
                let left = counts.len() - 1;
                if left <= floor {
                    counts.truncate(left);
                    let tiles = p.tiles.clone();
                    for &(x, y) in tiles.iter().skip(2) {
                        self.block(x as u32, y as u32, s, |c| {
                            c.cost = 0;
                            c.occupied = false;
                        });
                    }
                    p.tiles.clear();
                    return false;
                }
                let k = counts[counts.len() - 2];
                let tiles = p.tiles.clone();
                for &(x, y) in tiles.iter().skip(k) {
                    self.block(x as u32, y as u32, s, |c| {
                        c.cost = 0;
                        c.occupied = false;
                    });
                }
                i = *at.last().unwrap();
                p.tiles.truncate(k);
                counts.truncate(left);
            } else {
                let mut reached;
                loop {
                    let last = *p.tiles.last().unwrap();
                    if last == tile_of(p.points[i + 1], si) {
                        reached = i;
                        break;
                    }
                    reached = i + 1;
                    if p.bits.len() <= i + 2 {
                        break;
                    }
                    self.ease_segment(p.points[reached], p.points[reached + 1], s, p.bits[reached]);
                    i = reached;
                }
                i = next.max(reached + 1);
                if counts.last() != Some(&p.tiles.len()) {
                    at.push(i);
                    counts.push(p.tiles.len());
                    if at.len() > 10 {
                        floor_next = at.len() - 10;
                    }
                }
                next = i + 1;
                if next < n {
                    self.ease_segment(p.points[next - 1], p.points[next], s, p.bits[i]);
                }
            }
            floor = floor_next;
            if (!p.is_loop) as usize + i >= p.bits.len() {
                break;
            }
        }
        if !ok {
            p.tiles.clear();
            return false;
        }
        if !p.is_loop {
            let t = self.node_tile(p.final_node, si);
            if p.tiles.last() != Some(&t) {
                p.tiles.push(t);
            }
            if let Some(t) = p.end_off {
                p.tiles.push(t);
            }
        }
        // Drop back-and-forth spikes (a, b, a → a).
        let mut k = 2;
        while k < p.tiles.len() {
            if p.tiles[k - 2] == p.tiles[k] {
                p.tiles.drain(k - 1..k + 1);
            } else {
                k += 1;
            }
        }
        !p.tiles.is_empty()
    }
}

/// Size of a tile in orientation `o` (odd orientations below 8 swap).
fn oriented(size: (u8, u8), o: u8) -> (u32, u32) {
    if o & 1 == 1 && o < 8 { (size.1 as u32, size.0 as u32) } else { (size.0 as u32, size.1 as u32) }
}

/// The chain walk (`0x1420c4960`): along an edge's points, then on through degree-2 nodes.
struct ChainIter {
    cur: usize,
    start: usize,
    edge: usize,
    backwards: bool,
    /// Mark edges left behind as tilised.
    mark: bool,
    idx: usize,
}

impl ChainIter {
    fn new(t: &EdgeTiler, e: usize, end: usize, mark: bool) -> Self {
        let start = t.g.endpoint(e, end);
        let backwards = t.g.edges[e].from != start;
        let idx = if backwards { t.g.edges[e].points.len() } else { 1 };
        Self { cur: start, start, edge: e, backwards, mark, idx }
    }

    fn next(&mut self, t: &mut EdgeTiler) -> bool {
        let n = t.g.edges[self.edge].points.len();
        if (self.idx != n + 1 || self.backwards) && (self.idx != 0 || !self.backwards) {
            if self.backwards { self.idx -= 1 } else { self.idx += 1 }
            return true;
        }
        let ed = &t.g.edges[self.edge];
        self.cur = if ed.from == self.cur { ed.to } else { ed.from };
        if t.degree(self.cur) == 2 && self.cur != self.start {
            if self.mark {
                t.tilised[self.edge] = true;
            }
            let Some(e2) = t.g.nodes[self.cur].slots.iter().flatten().copied().find(|&e| e != self.edge) else {
                return false;
            };
            self.edge = e2;
            self.backwards = t.g.edges[e2].from != self.cur;
            self.idx = if self.backwards { t.g.edges[e2].points.len() } else { 1 };
            return !t.tilised[e2];
        }
        false
    }

    fn pos(&self, t: &EdgeTiler) -> Pos {
        t.pos(self.edge, self.idx)
    }
}

impl EdgeTiler<'_> {
    /// `0x1420b6660` (after the end points): edge costs, taken ends and the two start lists.
    fn categorise(&mut self) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
        let (mut first, mut second) = (Vec::new(), Vec::new());
        for e in 0..self.g.edges.len() {
            self.add_edge_cost(e);
            let (from, to) = (self.g.edges[e].from, self.g.edges[e].to);
            if self.g.nodes[from].fixed {
                first.push((e, 0));
                self.take_end(e, 0);
                if self.g.nodes[to].fixed {
                    self.take_end(e, 1);
                }
            } else if self.g.nodes[to].fixed {
                first.push((e, 1));
                self.take_end(e, 1);
            } else {
                second.push((e, if self.degree(from) == 1 { 0 } else { 1 }));
            }
        }
        (first, second)
    }

    /// `0x1420b3fd0`: the room tiles under two cells touch.
    fn rooms_touch(&self, a: (i32, i32), b: (i32, i32)) -> bool {
        let room = |t: (i32, i32)| -> Option<(i32, i32, i32, i32)> {
            if (t.0 as u32) >= self.w || (t.1 as u32) >= self.h {
                return None;
            }
            let i = self.idx(t.0 as u32, t.1 as u32);
            let c = self.map.cells[i].as_ref()?;
            let p = self.map.piece_at[i]?;
            let (w, h) = oriented(self.map.pieces[p].size, self.map.pieces[p].orientation);
            Some((t.0 - c.offset.0 as i32, t.1 - c.offset.1 as i32, w as i32, h as i32))
        };
        let (Some((sx, sy, sw, sh)), Some((fx, fy, fw, fh))) = (room(a), room(b)) else { return false };
        if sx + sw == fx || fx + fw == sx {
            return sy <= fy + fh && fy <= sy + sh;
        }
        (sy + sh == fy || fy + fh == sy) && sx <= fx + fw && fx <= sx + sw
    }

    /// `0x1420b41b0`: a chain that stays inside the end tiles of two touching rooms.
    fn trivial_chain(&mut self, e: usize, end: usize, last: usize, s: i32) -> bool {
        let a = tile_of(self.g.nodes[self.g.endpoint(e, end)].pos, s);
        let b = tile_of(self.g.nodes[last].pos, s);
        if !self.rooms_touch(a, b) {
            return false;
        }
        let mut it = ChainIter::new(self, e, end, false);
        while it.next(self) {
            let t = tile_of(it.pos(self), s);
            if t != a && t != b {
                return false;
            }
        }
        true
    }

    /// `0x1420b43b0`: a free dead end inside the map repeats its position as an edge point.
    fn extend_dead_end(&mut self, n: usize) {
        let node = &self.g.nodes[n];
        if node.fixed || node.virtual_end || self.degree(n) != 1 {
            return;
        }
        let (x, y) = node.pos;
        let (tx, ty) = (x / 23, y / 23);
        if !(0 < tx && tx < self.w as i32 - 1 && 0 < ty && ty < self.h as i32 - 1) {
            return;
        }
        let e = node.slots.iter().flatten().copied().next().unwrap();
        let pos = node.pos;
        if self.g.edges[e].from == n {
            self.g.edges[e].points.insert(0, pos);
        } else {
            self.g.edges[e].points.push(pos);
        }
    }

    /// `0x1420b4510`: the chain's points from `end` of `e` to `last`, one per tile, loops cut.
    fn build_path(&mut self, e: usize, end: usize, last: usize) -> PathInfo {
        let s = self.sizes[e] as i32;
        let mut p = PathInfo { final_node: last, edge: e, end, ..Default::default() };
        let mut it = ChainIter::new(self, e, end, true);
        let mut index: HashMap<(i32, i32), usize> = HashMap::new();
        let mut pos = self.g.nodes[it.start].pos;
        let mut count = 0;
        loop {
            if count < 2 {
                p.points.push(pos);
            } else {
                let key = path_key(pos, s);
                match index.get(&key).copied() {
                    None => {
                        index.insert(key, p.points.len());
                        p.points.push(pos);
                    }
                    Some(k) => {
                        for q in &p.points[k + 1..] {
                            index.remove(&path_key(*q, s));
                        }
                        p.points.truncate(k + 1);
                        p.bits.truncate(k);
                    }
                }
            }
            count += 1;
            p.bits.push(it.backwards);
            pos = it.pos(self);
            if !p.edges.contains(&it.edge) {
                p.edges.push(it.edge);
            }
            if !it.next(self) {
                p.points.push(it.pos(self));
                self.tilised[it.edge] = true;
                break;
            }
        }
        let (a, b) = (p.points[0], *p.points.last().unwrap());
        let d = (a.0 - b.0, a.1 - b.1);
        if d.0 * d.0 + d.1 * d.1 == 1 {
            p.points.clear();
            return p;
        }
        let (mw, mh) = (self.w as i32 * 23, self.h as i32 * 23);
        let off = |q: Pos| q.0 < 0 || q.1 < 0 || q.0 >= mw || q.1 >= mh;
        let clamp = |q: Pos| (q.0.max(0).min(mw - 1), q.1.max(0).min(mh - 1));
        if off(a) {
            p.start_off = Some(path_key(a, s));
            p.points[0] = clamp(a);
        }
        let l = p.points.len() - 1;
        if off(p.points[l]) {
            p.end_off = Some(path_key(p.points[l], s));
            p.points[l] = clamp(p.points[l]);
        }
        p.is_loop = p.points[0] == p.points[l];
        p
    }

    /// `0x1420b5f30`: turn the chain starting at `end` of `e` into a path.
    fn start_chain(&mut self, e: usize, end: usize, rng: &mut TerrainRng) {
        if self.tilised[e] {
            return;
        }
        let mut it = ChainIter::new(self, e, end, false);
        while it.next(self) {}
        let last = it.cur;
        let start = self.g.endpoint(e, end);
        if self.trivial_chain(e, end, last, self.sizes[e] as i32) {
            return;
        }
        self.extend_dead_end(start);
        self.extend_dead_end(last);
        let mut p = self.build_path(e, end, last);
        if p.points.is_empty() {
            return;
        }
        if p.is_loop {
            // Loops are split in two halves by the client (not met yet).
            eprintln!("edge tiles: loop chain at edge {e} not ported");
            return;
        }
        if self.fit(&mut p, rng) {
            for k in 1..p.tiles.len().saturating_sub(1) {
                let (x, y) = p.tiles[k];
                let i = self.idx(x as u32, y as u32);
                self.cells[i].cost += 80;
            }
            self.placed.push(p);
        } else {
            for &e in &p.edges.clone() {
                self.add_edge_cost(e);
            }
            self.pending.push(p);
        }
    }

    /// `0x1420b6950`: all chains into paths, in the client's order (see [`Self::order`]).
    pub fn run(&mut self, rng: &mut TerrainRng) -> Result<(), String> {
        let (mut first, second) = self.categorise();
        rng.below(first.len() as u32);
        let n = first.len();
        for k in 0..n {
            let j = k + rng.uniform((n - k) as u32) as usize;
            first.swap(k, j);
        }
        // Stable sorts: by edge priority (all 0 so far), then split edges first.
        first.sort_by_key(|&(e, _)| !self.g.edges[e].split);
        for (e, end) in first.into_iter().chain(second) {
            self.start_chain(e, end, rng);
        }
        while !self.pending.is_empty() {
            for k in 0..self.placed.len() {
                let s = self.sizes[self.placed[k].edge];
                let tiles = self.placed[k].tiles.clone();
                for &(x, y) in tiles.iter().take(tiles.len().saturating_sub(2)).skip(2) {
                    self.block(x as u32, y as u32, s, |c| c.occupied = false);
                }
            }
            for mut p in std::mem::take(&mut self.pending) {
                if !self.fit(&mut p, rng) {
                    return Err(format!("could not tilise edge {}", p.edge));
                }
                self.place_path(&p, rng)?;
                self.order.push(p);
            }
            let mut k = 0;
            while k < self.placed.len() {
                let tiles = self.placed[k].tiles.clone();
                let inner = &tiles[1.min(tiles.len())..tiles.len().saturating_sub(1)];
                let hit = inner.iter().any(|&(x, y)| self.cells[self.idx(x as u32, y as u32)].occupied);
                if !hit {
                    k += 1;
                    continue;
                }
                for &(x, y) in inner {
                    let i = self.idx(x as u32, y as u32);
                    if self.cells[i].cost > 0x4f {
                        self.cells[i].cost -= 80;
                    }
                }
                let mut p = self.placed[k].clone();
                if self.fit(&mut p, rng) {
                    for &(x, y) in &p.tiles[1..p.tiles.len().saturating_sub(1)] {
                        let i = self.idx(x as u32, y as u32);
                        self.cells[i].cost += 80;
                    }
                    self.placed[k] = p;
                    k += 1;
                } else {
                    for &e in &p.edges.clone() {
                        self.add_edge_cost(e);
                    }
                    self.pending.push(p);
                    self.placed.remove(k);
                }
            }
        }
        for p in std::mem::take(&mut self.placed) {
            self.place_path(&p, rng)?;
            self.order.push(p);
        }
        Ok(())
    }

    /// Size of the tile at a cell: the room tile's, or the edge tile size.
    fn tile_size_at(&self, t: (i32, i32), s: i32) -> (i32, i32) {
        if (t.0 as u32) < self.w && (t.1 as u32) < self.h {
            let i = self.idx(t.0 as u32, t.1 as u32);
            if let (Some(_), Some(p)) = (&self.map.cells[i], self.map.piece_at[i]) {
                let (w, h) = oriented(self.map.pieces[p].size, self.map.pieces[p].orientation);
                return (w as i32, h as i32);
            }
        }
        (s, s)
    }

    /// `0x1420b78d0`: the side of `t` that `nb` lies on (0 north, 1 east, 2 south, 3 west).
    fn side(&self, t: (i32, i32), nb: (i32, i32), s: i32) -> Result<usize, String> {
        let (tw, th) = self.tile_size_at(t, s);
        if nb.0 == t.0 + tw {
            return Ok(1);
        }
        if nb.1 == t.1 + th {
            return Ok(2);
        }
        let (nw, nh) = self.tile_size_at(nb, s);
        if t.1 == nb.1 + nh {
            return Ok(0);
        }
        if t.0 == nb.0 + nw {
            return Ok(3);
        }
        if nb.0 == t.0 + s {
            return Ok(1);
        }
        if nb.1 == t.1 + s {
            return Ok(2);
        }
        if t.1 == nb.1 + s {
            return Ok(0);
        }
        if t.0 != nb.0 + s {
            return Err(format!("tiles {t:?} and {nb:?} are not adjacent"));
        }
        Ok(3)
    }

    /// `0x141f9e990`: where the room exit on `nb`'s tile meets side `side` of `t`.
    fn room_exit(&self, t: (i32, i32), nb: (i32, i32), side: usize, s: i32) -> Result<u8, String> {
        let i = self.idx(nb.0 as u32, nb.1 as u32);
        let (Some(c), Some(p)) = (&self.map.cells[i], self.map.piece_at[i]) else {
            return Err("neighbour does not exist".into());
        };
        let piece = &self.map.pieces[p];
        let o = piece.orientation;
        let tdt = &self.tileset.tiles[piece.tile.ok_or("room tile outside the tile set")?].tdt;
        let (rw, rh) = oriented(piece.size, o);
        let (rw, rh) = (rw as i32, rh as i32);
        let flipped = (4..8).contains(&o);
        let dim = |own: usize| if own & 1 == 0 { tdt.size.0 } else { tdt.size.1 } as i32;
        let them = (side + 2) & 3;
        let own = own_side(o, them);
        let mut e = tdt.edge_distances[own] as i32;
        if flipped {
            e = 3 * dim(own) - e - 1;
        }
        // `0x141d08470`: the virtual exit seen with the orientation.
        let virtual_exit = |side: usize| -> i32 {
            let own = own_side(o, side);
            let v = tdt.edge_distances[4 + own] as i32;
            if flipped && v < 3 * dim(own) {
                3 * dim(own) - v - 1
            } else {
                v
            }
        };
        let (rx, ry) = (nb.0 - c.offset.0 as i32, nb.1 - c.offset.1 as i32);
        let fail = || format!("could not connect edge tiles from {t:?} to {nb:?}");
        let v = match them {
            0 => {
                let col = e / 3 + rx;
                if t.0 <= col && col < t.0 + s {
                    ((t.0 - rx) + s) * 3 - e - 1
                } else {
                    let v = virtual_exit(0);
                    if v == rw * 3 {
                        return Err(fail());
                    }
                    ((t.0 - rx) + s) * 3 - v - 1
                }
            }
            1 => {
                let row = e / 3 + ry;
                if t.1 <= row && row < t.1 + s {
                    ((t.1 - ry) + s) * 3 - e - 1
                } else {
                    let v = virtual_exit(1);
                    if v == rh * 3 {
                        return Err(fail());
                    }
                    ((t.1 - ry) + s) * 3 - v - 1
                }
            }
            2 => {
                let col = rx - e / 3 - 1 + rw;
                if t.0 <= col && col < t.0 + s {
                    ((rx - t.0) + rw) * 3 - e - 1
                } else {
                    let v = virtual_exit(2);
                    if v == rw * 3 {
                        return Err(fail());
                    }
                    ((rx - t.0) + rw) * 3 - v - 1
                }
            }
            _ => {
                let row = ry - e / 3 - 1 + rh;
                if t.1 <= row && row < t.1 + s {
                    ((ry - t.1) + rh) * 3 - e - 1
                } else {
                    let v = virtual_exit(3);
                    if v == rh * 3 {
                        return Err(fail());
                    }
                    ((ry - t.1) + rh) * 3 - v - 1
                }
            }
        };
        Ok(v as u8)
    }

    /// `0x1420b7f30`: an edge tile for every inner tile of a path.
    fn place_path(&mut self, p: &PathInfo, rng: &mut TerrainRng) -> Result<(), String> {
        let info = self.edges[p.edge].clone();
        let s = info.size as i32;
        let full = (3 * info.size) as u8;
        let curve = if info.centred || info.hashed {
            Vec::new()
        } else if p.is_loop {
            return Err("edge tiles: loop paths (0x1420b70e0) not ported".into());
        } else {
            crossings(&p.tiles, info.size)
        };
        let chirality = self.g.edges[p.edge].chirality;
        let skip = (!p.is_loop) as usize;
        let n = p.tiles.len();
        for i in skip..n - skip {
            let t = p.tiles[i];
            let prev = p.tiles[(i + n - 1) % n];
            let next = p.tiles[(i + 1) % n];
            let side_in = self.side(t, prev, s)?;
            let side_out = self.side(t, next, s)?;
            let mut key = TileKey {
                size: (s as u8, s as u8),
                exits: [full; 4],
                virtual_exits: [full; 4],
                ..Default::default()
            };
            key.edges[side_in] = Some(info.path.clone());
            key.edges[side_out] = Some(info.path.clone());
            if info.centred {
                key.exits[side_in] = full >> 1;
                key.exits[side_out] = full >> 1;
            } else {
                for (side, nb, k) in [(side_in, prev, i - skip), (side_out, next, i - skip + 1)] {
                    key.exits[side] = if self.is_room(nb) {
                        self.room_exit(t, nb, side, s)?
                    } else if !info.hashed {
                        crossing_exit(side, curve[k], info.size)
                    } else {
                        hashed_exit(t.0, t.1, side as u32, info.size)
                    };
                }
            }
            // `0x1420b7d70`: grounds either side of the edge, the missing corner of a turn.
            let rev = p.end != 0;
            for (side, flag) in [(side_in, rev), (side_out, !rev)] {
                let (mut a, mut b) = (side, (side + 1) & 3);
                if chirality != flag {
                    std::mem::swap(&mut a, &mut b);
                }
                key.grounds[a] = info.grounds[0].clone();
                key.heights[a] = 0;
                key.grounds[b] = info.grounds[1].clone();
                key.heights[b] = info.height;
            }
            if (side_in + 2) & 3 != side_out {
                const MISSING: [usize; 16] = [4, 3, 4, 2, 3, 4, 0, 4, 4, 0, 4, 1, 2, 4, 1, 4];
                let c = MISSING[side_in * 4 + side_out];
                if c == 4 {
                    return Err("bad corner index for missing corner".into());
                }
                key.grounds[c] = key.grounds[(c + 1) & 3].clone();
                key.heights[c] = key.heights[(c + 1) & 3];
            }
            let list = self.tileset.candidates("", &key);
            if std::env::var_os("POE_DEBUG_TILES").is_some() { eprintln!("edge lookup {} at {:?}", list.len(), key); for c in list { eprintln!("edge cand {} o{} w{} key {:?}", self.tileset.tiles[c.tile].path, c.orientation, c.weight, self.tileset.tiles[c.tile].key.oriented(c.orientation)); } }
            if list.is_empty() {
                return Err(format!("no edge tile at {t:?} for key {key:?}"));
            }
            let total = list.iter().map(|c| c.weight).sum();
            let c = pick(list, rng.below(total)).expect("non-empty list");
            let def = &self.tileset.tiles[c.tile];
            self.map.place(t.0, t.1, &def.path, &def.tdt, c.orientation, 0, Some(c.tile));
        }
        Ok(())
    }

    fn is_room(&self, t: (i32, i32)) -> bool {
        (t.0 as u32) < self.w && (t.1 as u32) < self.h && self.map.cells[self.idx(t.0 as u32, t.1 as u32)].is_some()
    }
}

/// `0x141d08470`: a tile's virtual exit on map side `side` with orientation `o`.
pub fn virtual_exit(tdt: &crate::tdt::Tdt, o: u8, side: usize) -> i32 {
    let own = own_side(o, side);
    let dim = if own & 1 == 0 { tdt.size.0 } else { tdt.size.1 } as i32;
    let v = tdt.edge_distances[4 + own] as i32;
    if (4..8).contains(&o) && v < 3 * dim { 3 * dim - v - 1 } else { v }
}

/// `0x141d00fd0(1, 1, side, inverse orientation)`: a tile's own side shown at map side `side`.
pub fn own_side_pub(o: u8, side: usize) -> usize {
    own_side(o, side)
}

fn own_side(o: u8, side: usize) -> usize {
    const INVERSE: [usize; 9] = [0, 3, 2, 1, 4, 5, 6, 7, 8];
    const A: [u32; 9] = [0, 0, 1, 1, 1, 1, 2, 2, 0];
    const B: [u32; 9] = [0, 1, 1, 2, 0, 1, 1, 2, 0];
    let o2 = INVERSE[(o as usize).min(8)];
    let mut s = side as u32;
    if (4..8).contains(&o2) {
        s = 3u32.wrapping_sub(s);
    }
    (s.wrapping_add(A[o2.min(8)]).wrapping_add(B[o2.min(8)]) % 4) as usize
}

/// `0x1420b7490`: where a path's smoothed line crosses each pair of neighbouring tiles, as an
/// offset from the middle of their shared side (f32, like the client).
fn crossings(tiles: &[(i32, i32)], size: u32) -> Vec<(f32, f32)> {
    let h = (size >> 1) & 0x7fff_ffff;
    let centre = |t: (i32, i32)| -> (f32, f32) {
        let x = (t.0 as u32).wrapping_add(h) as f32 + 0.5;
        let y = (t.1 as u32).wrapping_add(h) as f32 + 0.5;
        (x * 23.0, y * 23.0)
    };
    let mut pts = Vec::with_capacity(tiles.len() + 8);
    let first = centre(tiles[0]);
    let last = centre(*tiles.last().unwrap());
    pts.extend([first; 4]);
    pts.extend(tiles.iter().map(|&t| centre(t)));
    pts.extend([last; 4]);
    let mut out = Vec::new();
    for i in 0..tiles.len() - 1 {
        let mut w: Vec<(f32, f32)> = pts[i..i + 10].to_vec();
        let mid = ((w[4].0 + w[5].0) / 2.0, (w[4].1 + w[5].1) / 2.0);
        for k in 0..10 {
            for j in 0..(10usize - k).saturating_sub(1) {
                let a = (w[j].0 * 0.5, w[j].1 * 0.5);
                let b = (w[j + 1].0 * 0.5, w[j + 1].1 * 0.5);
                w[j] = (a.0 + b.0, a.1 + b.1);
            }
        }
        out.push((w[0].0 - mid.0, w[0].1 - mid.1));
    }
    out
}

/// `0x1420b7ba0`: which third of a side the crossing falls in.
fn crossing_exit(side: usize, v: (f32, f32), size: u32) -> u8 {
    let t = (size as u64 * 23) as f32 / 6.0;
    let half = size >> 1;
    let r = match side {
        0 | 1 => {
            let v = if side == 0 { v.0 } else { v.1 };
            let i = if -t <= v { (t < v) as u32 + 1 } else { 0 };
            i * size + half
        }
        2 | 3 => {
            let v = if side == 2 { v.0 } else { v.1 };
            if -t <= v { (v <= t) as u32 * size + half } else { size * 2 + half }
        }
        _ => size * 3,
    };
    r as u8
}

/// `0x1421969e0`: crossing position from a coordinate hash.
fn hashed_exit(x: i32, y: i32, side: u32, size: u32) -> u8 {
    let (x, y) = (x as u32, y as u32);
    let i = match side {
        1 => {
            let u = y.wrapping_mul(0x7bb).wrapping_add(0x31e9).wrapping_add(x.wrapping_add(size).wrapping_mul(0x337)) % 13;
            ((u / 3) * 3).wrapping_sub(u).wrapping_add(2) & 0xff
        }
        2 => {
            let u = y.wrapping_add(size).wrapping_mul(0x7bb).wrapping_add(x.wrapping_mul(0x337)) % 13;
            ((u / 3) * 3).wrapping_sub(u).wrapping_add(2) & 0xff
        }
        _ => y.wrapping_mul(0x7bb).wrapping_add(side.wrapping_mul(0x10a3)).wrapping_add(x.wrapping_mul(0x337)) % 13 % 3,
    };
    (i.wrapping_mul(size & 0xff) as u8).wrapping_add((size >> 1) as u8)
}
