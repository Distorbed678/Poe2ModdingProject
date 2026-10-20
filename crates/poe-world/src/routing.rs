//! Edge routing (`0x1420cdc30`): graph edges (rivers, paths) are routed tile by tile between
//! the rooms' exits over a random cost grid. See `docs/re/terrain.md`, "Edge
//! routing".
//!
//! 1. After each room pass (`0x1420c9970`) the room's edges end at its exits
//!    (`0x1420c5b50`): exit `e` on a side is tile `e / 3`, cell `[3, 11, 19][e % 3]` along the
//!    side (sides 2 and 3 counted backwards); a virtual edge ends between its virtual exit and
//!    its exit. The edge's end offset is the side's direction (one tile out of the room), and
//!    internal points inside the room are dropped (`0x1420c5750`).
//! 2. Cost grid: `below(1024)` per tile, +1024 on the border; tiles already placed are
//!    occupied.
//! 3. Every edge reserves the straight lines between its waypoints (`0x1420ce390`; start +
//!    offset, internal points, end + offset): the line's tiles get +1 use (blocked for other
//!    edges) and the tiles beside it, up to the edge's margin, extra cost.
//! 4. Edges are routed in graph order (`0x1420cd460`): per leg, its reservation is lifted and
//!    A* (`0x1420cd110`, 4 neighbours, Manhattan × 100, open set ordered by (f, tile index);
//!    g is the tile's own 100 + cost, not accumulated — a client quirk) runs from the leg's end
//!    back to its start; the path is reserved, and
//!    every 6th tile adds an internal point at a random cell of that tile.

use std::collections::BTreeSet;

use crate::{generate::TileMap, rng::TerrainRng};

pub type Pos = (i32, i32);

/// Side directions (`0x142f0c608`): up, right, down, left.
pub const SIDE_DIR: [Pos; 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
/// Exit cell within its tile (`0x142f0df10`).
pub const EXIT_CELL: [i32; 3] = [3, 11, 19];

/// An edge being routed.
#[derive(Debug, Clone)]
pub struct RouteEdge {
    /// End positions in grid cells (from, to).
    pub ends: [Pos; 2],
    /// End offsets in tiles (a room exit's side direction).
    pub offsets: [Pos; 2],
    /// Internal points in grid cells.
    pub points: Vec<Pos>,
    /// Reservation margin: edge size + 2, or max width / 23 + 3 for virtual edges.
    pub margin: i32,
}

impl RouteEdge {
    /// Waypoint `i` in tiles: 0 = start + offset, `n + 1` = end + offset, else internal point.
    fn waypoint(&self, i: usize) -> Pos {
        let n = self.points.len();
        if i == 0 {
            (self.ends[0].0 / 23 + self.offsets[0].0, self.ends[0].1 / 23 + self.offsets[0].1)
        } else if i == n + 1 {
            (self.ends[1].0 / 23 + self.offsets[1].0, self.ends[1].1 / 23 + self.offsets[1].1)
        } else if i > n {
            (self.ends[1].0 / 23, self.ends[1].1 / 23)
        } else {
            (self.points[i - 1].0 / 23, self.points[i - 1].1 / 23)
        }
    }

    /// Internal point `i` (1-based) in cells, or the end node beyond them.
    fn point(&self, i: usize) -> Pos {
        if i > self.points.len() {
            self.ends[1]
        } else {
            self.points[i - 1]
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Tile {
    cost: i32,
    uses: i32,
    occupied: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct Node {
    g: u32,
    h: u32,
    parent: u32,
    closed: bool,
    open: bool,
}

pub struct Router {
    pub w: i32,
    pub h: i32,
    tiles: Vec<Tile>,
    nodes: Vec<Node>,
}

#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error("could not route edges {0:?}")]
    Failed(Vec<usize>),
}

impl Router {
    /// The cost grid (`0x1420cdc30`, first part).
    pub fn new(w: i32, h: i32, map: &TileMap, rng: &mut TerrainRng) -> Self {
        let mut tiles: Vec<Tile> = (0..w * h)
            .map(|_| Tile {
                cost: rng.below(1024) as i32,
                ..Default::default()
            })
            .collect();
        for x in 0..w {
            tiles[x as usize].cost += 1024;
            tiles[((h - 1) * w + x) as usize].cost += 1024;
        }
        for y in 1..h - 1 {
            tiles[(y * w) as usize].cost += 1024;
            tiles[(y * w + w - 1) as usize].cost += 1024;
        }
        for y in 0..h {
            for x in 0..w {
                tiles[(y * w + x) as usize].occupied = map.get(x as u32, y as u32).is_some();
            }
        }
        Self {
            w,
            h,
            nodes: vec![Node::default(); (w * h) as usize],
            tiles,
        }
    }

    /// `0x1420ccc80`: the tile gets `delta` uses, the tiles beside it (perpendicular `perp`)
    /// up to `margin` away graded extra cost.
    fn mark_tile(&mut self, p: Pos, delta: i32, margin: i32, perp: Pos) {
        let inside = |x: i32, y: i32| x >= 0 && y >= 0 && x < self.w && y < self.h;
        if inside(p.0, p.1) {
            self.tiles[(p.1 * self.w + p.0) as usize].uses += delta;
        }
        let m = margin as u32;
        for k in 1..=m {
            let q = ((m - k + 1) * 200_000) / m;
            let add = (q.wrapping_mul(delta as u32) as i32) / 100;
            let k = k as i32;
            for (x, y) in [
                (p.0 - k * perp.0, p.1 - k * perp.1),
                (p.0 + k * perp.0, p.1 + k * perp.1),
            ] {
                if inside(x, y) {
                    self.tiles[(y * self.w + x) as usize].cost += add;
                }
            }
        }
    }

    /// `0x1420ccdc0`: marks the tiles of the line `a → b` (Bresenham along the major axis).
    fn mark_line(&mut self, a: Pos, b: Pos, delta: i32, margin: i32) {
        let adx = (b.0 - a.0).abs();
        let ady = (b.1 - a.1).abs();
        let perp = ((adx < ady) as i32, (ady <= adx) as i32);
        let y_major = adx < ady;
        let (mut u, u_end, mut v, v_end) = if y_major {
            (a.1, b.1, a.0, b.0)
        } else {
            (a.0, b.0, a.1, b.1)
        };
        let du = (u_end - u).abs();
        let dv = (v_end - v).abs();
        let vstep = (v_end - v).signum();
        let ustep = if u < u_end { 1 } else { -1 };
        let mut err = -(du / 2);
        loop {
            if u == u_end + ustep {
                return;
            }
            let p = if y_major { (v, u) } else { (u, v) };
            self.mark_tile(p, delta, margin, perp);
            let e = err + dv;
            u += ustep;
            if e < 1 {
                err = e;
            } else {
                err = e - du;
                v += vstep;
            }
        }
    }

    /// The grid's tiles: (cost, uses, occupied), row-major.
    pub fn snapshot(&self) -> Vec<(i32, i32, bool)> {
        self.tiles.iter().map(|t| (t.cost, t.uses, t.occupied)).collect()
    }

    /// `0x1420ce390`: reserves an edge's straight legs.
    pub fn reserve(&mut self, e: &RouteEdge) {
        let n = e.points.len();
        for i in 0..=n {
            self.mark_line(e.waypoint(i), e.waypoint(i + 1), 1, e.margin);
        }
    }

    /// `0x1420cd110`: A* from `start` to `goal`.
    fn astar(&mut self, start: Pos, goal: Pos) -> bool {
        for n in &mut self.nodes {
            *n = Node::default();
        }
        let w = self.w;
        let key = |nodes: &Vec<Node>, i: u32| (nodes[i as usize].g + nodes[i as usize].h, i);
        let mut open: BTreeSet<(u32, u32)> = BTreeSet::new();
        let si = (start.1 * w + start.0) as u32;
        open.insert(key(&self.nodes, si));
        self.nodes[si as usize].open = true;
        while let Some(&(f, cur)) = open.iter().next() {
            let (cx, cy) = ((cur % w as u32) as i32, (cur / w as u32) as i32);
            if (cx, cy) == goal {
                return true;
            }
            open.remove(&(f, cur));
            self.nodes[cur as usize].open = false;
            self.nodes[cur as usize].closed = true;
            let mut nbrs = Vec::with_capacity(4);
            if cx > 0 {
                nbrs.push((cx - 1, cy));
            }
            if cy > 0 {
                nbrs.push((cx, cy - 1));
            }
            if ((cx + 1) as u32) < self.w as u32 {
                nbrs.push((cx + 1, cy));
            }
            if ((cy + 1) as u32) < self.h as u32 {
                nbrs.push((cx, cy + 1));
            }
            for (nx, ny) in nbrs {
                let ni = (ny * w + nx) as u32;
                let node = self.nodes[ni as usize];
                let tile = self.tiles[ni as usize];
                let free = node.parent != ni + 1 && tile.uses + tile.occupied as i32 == 0;
                if node.closed || !((nx, ny) == goal || free) {
                    continue;
                }
                // The client adds to the neighbour's own g (0 until it is first reached), not
                // the current tile's, so g is just 100 + the tile's cost and an open tile is
                // never improved: in effect a best-first search on cost + distance.
                let g = node.g.wrapping_add(100).wrapping_add(tile.cost as u32);
                if !node.open {
                    self.nodes[ni as usize].h =
                        (((goal.0 - nx).abs() + (goal.1 - ny).abs()) * 100) as u32;
                } else {
                    if node.g <= g {
                        continue;
                    }
                    open.remove(&key(&self.nodes, ni));
                }
                self.nodes[ni as usize].g = g;
                open.insert(key(&self.nodes, ni));
                self.nodes[ni as usize].open = true;
                self.nodes[ni as usize].parent = cur + 1;
            }
        }
        false
    }

    /// `0x1420cd460`: routes one edge; on success its internal points are replaced.
    pub fn route(&mut self, e: &mut RouteEdge, rng: &mut TerrainRng) -> bool {
        let w = self.w;
        let n = e.points.len();
        let mut new_points: Vec<Pos> = Vec::new();
        let mut path: Vec<Pos> = Vec::new();
        let mut last = 0usize;
        let push = |pts: &mut Vec<Pos>, p: Pos| {
            if pts.last() != Some(&p) {
                pts.push(p);
            }
        };
        for i in 1..=n + 1 {
            let p = e.waypoint(last);
            let a = e.waypoint(i);
            if p == a {
                if last != 0 {
                    let q = e.point(last);
                    new_points.push(q);
                }
                continue;
            }
            let q = e.waypoint(i - 1);
            self.mark_line(q, a, -1, e.margin);
            if !self.astar(a, p) {
                if i == n + 1 {
                    for k in 1..path.len() {
                        self.mark_line(path[k - 1], path[k], -1, e.margin);
                    }
                    return false;
                }
                continue;
            }
            if last != 0 {
                push(&mut new_points, e.point(last));
            }
            let a_index = (a.1 * w + a.0) as u32;
            let mut prev = p;
            let mut cur = self.nodes[(p.1 * w + p.0) as usize].parent.wrapping_sub(1);
            let mut count = 0u32;
            while cur != a_index {
                let cell = ((cur % w as u32) as i32, (cur / w as u32) as i32);
                path.push(cell);
                self.mark_line(prev, cell, 1, e.margin);
                count += 1;
                if count % 6 == 0 {
                    let rx = rng.below(23) as i32;
                    let ry = rng.below(23) as i32;
                    push(&mut new_points, (cell.0 * 23 + rx, cell.1 * 23 + ry));
                }
                prev = cell;
                cur = self.nodes[cur as usize].parent.wrapping_sub(1);
            }
            path.push(a);
            self.mark_line(a, prev, 1, e.margin);
            last = i;
        }
        e.points = new_points;
        true
    }
}

/// Reserves and routes every edge in order, retrying failures (`0x1420cdc30`).
pub fn route_all(
    router: &mut Router,
    edges: &mut [RouteEdge],
    rng: &mut TerrainRng,
) -> Result<(), RouteError> {
    for e in edges.iter() {
        router.reserve(e);
    }
    route_reserved(router, edges, rng)
}

/// Routes edges already reserved.
pub fn route_reserved(
    router: &mut Router,
    edges: &mut [RouteEdge],
    rng: &mut TerrainRng,
) -> Result<(), RouteError> {
    let mut todo: Vec<usize> = (0..edges.len()).collect();
    while !todo.is_empty() {
        let mut failed = Vec::new();
        for &i in &todo {
            if !router.route(&mut edges[i], rng) {
                failed.push(i);
                router.reserve(&edges[i]);
            }
        }
        if failed.len() == todo.len() {
            return Err(RouteError::Failed(failed));
        }
        todo = failed;
    }
    Ok(())
}
