//! A port of the client's grid pathfinder (0.5.5e, `FUN_141ed0390`; the algorithm and the
//! addresses are in `docs/re/pathfinding.md`), for static terrain.
//!
//! The client plans every walk, WASD step, roll frame and sprint frame with this search, so a
//! server that wants the same positions near walls has to plan the same way. It is a jump point
//! search with bounded jumps over the 4-bit cell grid, followed by a line-of-sight pass that
//! thins the path:
//!
//! - The search window is 272 x 272 cells around the start (the goal is pulled in to 126 cells by
//!   the caller). Cells outside an ellipse around start and goal (focal sum `L`, see
//!   [`plan`]) count as walls.
//! - A cell is blocked when its grid value (layer 0, value 5 meaning "far from walls") is at most
//!   the entity's clearance `level`: 0 normally, 4 while it sprints after a roll.
//! - Jumps along a direction are cut after `3 + 5 * alignment + 0.2 * excess` cells (at most
//!   `cap`, 5 to 22), so the open list holds nodes a few cells apart.
//! - The node with the lowest remaining distance `h` wins when the goal cannot be reached, and
//!   the path is rebuilt backwards through the nodes keeping only the cells that break
//!   line of sight, then thinned once more in windows of five.
//!
//! Not ported: dynamic obstacles (other entities), the A* fallback (taken when the start cell is
//! blocked; the search runs again at level 0 instead, which agreed with the client), and the
//! variant of the last line-of-sight test used while the entity is already moving along a line
//! (5 of 476 logged plans differ in one waypoint next to the start).
//!
//! The heap reproduces MSVC's `std::push_heap`/`pop_heap`, because equal costs are common on a
//! grid and the client's tie order decides which of several equally good paths it takes.

// The client's constants are rounded decimals (0.7071, 1.4142), not the exact values.
#![allow(clippy::approx_constant)]

use std::cell::RefCell;

use crate::movement::Walkable;

pub type Cell = (i32, i32);

/// Half the side of the search window (the window is `2 * HALF` cells square).
const HALF: i32 = 0x88;
const SIDE: usize = 0x110;
/// Directions as the client numbers them: the offset a node's `dir` points to its parent by.
const DXY: [Cell; 8] = [
    (0, 1),
    (1, 0),
    (0, -1),
    (-1, 0),
    (1, 1),
    (1, -1),
    (-1, -1),
    (-1, 1),
];
/// Opposite direction.
const REV: [usize; 8] = [2, 3, 0, 1, 6, 7, 4, 5];
/// Unit vectors of the directions (the client's float table; the diagonals are 0.7071).
const UNIT: [(f32, f32); 8] = [
    (0.0, 1.0),
    (1.0, 0.0),
    (0.0, -1.0),
    (-1.0, 0.0),
    (0.7071, 0.7071),
    (0.7071, -0.7071),
    (-0.7071, -0.7071),
    (-0.7071, 0.7071),
];
/// Neighbour codes checked for forced neighbours, per direction.
const ROW: [[usize; 4]; 8] = [
    [1, 3, 5, 6],
    [0, 2, 7, 6],
    [1, 3, 4, 7],
    [0, 2, 4, 5],
    [0, 1, 5, 7],
    [2, 1, 6, 4],
    [2, 3, 5, 7],
    [0, 3, 6, 4],
];

const SQRT2: f32 = 1.4142;
/// Node flag bits: low nibble = direction to the parent (8 = the start), `CLOSED` once
/// classified or expanded, `BLOCKED` with it for a wall, `SEEN` for a cell a jump passed.
const CLOSED: u8 = 0x10;
const BLOCKED: u8 = 0x20;
const SEEN: u8 = 0x80;

/// What the planner needs from the search call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// Lowest and highest wanted distance to the goal (0, 0: the goal cell itself).
    pub min_dist: u32,
    pub max_dist: u32,
    /// Stop this many cells from the goal (-1: none).
    pub radius: i8,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            min_dist: 0,
            max_dist: 0,
            radius: -1,
        }
    }
}

/// Result of [`plan`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Waypoints from the end of the path back to the start, the last entry being the start
    /// cell itself (the client's order); empty when there is nothing to walk to.
    pub path: Vec<Cell>,
    /// The goal cell itself was reached.
    pub reached: bool,
}

impl Plan {
    /// The cells to walk to, in walking order (without the start).
    pub fn waypoints(&self) -> impl Iterator<Item = Cell> + '_ {
        self.path.iter().rev().skip(1).copied()
    }
}

#[derive(Clone, Copy, Default)]
struct Node {
    stamp: u16,
    flags: u8,
    g: f32,
    h: f32,
    f: f32,
}

#[derive(Default)]
struct Scratch {
    nodes: Vec<Node>,
    stamp: u16,
    heap: Vec<(u32, f32)>,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

/// MSVC `std::push_heap` with a "greater f" comparator (a min-heap on f).
fn heap_push(heap: &mut Vec<(u32, f32)>, key: u32, f: f32) {
    heap.push((key, f));
    let mut hole = heap.len() - 1;
    let val = heap[hole];
    while hole > 0 {
        let parent = (hole - 1) >> 1;
        if heap[parent].1 <= val.1 {
            break;
        }
        heap[hole] = heap[parent];
        hole = parent;
    }
    heap[hole] = val;
}

/// MSVC `std::pop_heap` followed by `pop_back`: returns the old top.
fn heap_pop(heap: &mut Vec<(u32, f32)>) -> (u32, f32) {
    let n = heap.len();
    let top = heap[0];
    if n >= 2 {
        let val = heap[n - 1];
        heap[n - 1] = top;
        let bottom = n - 1;
        let mut hole = 0usize;
        let mut idx = 0usize;
        let max_non_leaf = (bottom as isize - 1) >> 1;
        while (idx as isize) < max_non_leaf {
            idx = 2 * idx + 2;
            if heap[idx].1 > heap[idx - 1].1 {
                idx -= 1;
            }
            heap[hole] = heap[idx];
            hole = idx;
        }
        if idx as isize == max_non_leaf && bottom.is_multiple_of(2) {
            heap[hole] = heap[bottom - 1];
            hole = bottom - 1;
        }
        while hole > 0 {
            let parent = (hole - 1) >> 1;
            if heap[parent].1 <= val.1 {
                break;
            }
            heap[hole] = heap[parent];
            hole = parent;
        }
        heap[hole] = val;
    }
    heap.pop();
    top
}

/// The client's `sqrt(float(x*x + y*y))` of an integer vector.
fn length(x: i32, y: i32) -> f32 {
    (x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y)) as u32 as f32).sqrt()
}

struct Search<'a> {
    terrain: &'a dyn Walkable,
    bounds: Option<(i32, i32)>,
    scratch: &'a mut Scratch,
    start: Cell,
    goal: Cell,
    origin: Cell,
    level: i32,
    params: Params,
    /// Goal minus start.
    a: Cell,
    /// `(max_dist + radius)^2`: cells this close to the goal are tested for being the goal.
    goal2: u32,
    /// `max_dist + radius` as float.
    max_ext: f32,
    h_start: f32,
    cap: u32,
    ellipse: f32,
    mid: Cell,
    mid_r2: u32,
}

impl Search<'_> {
    fn index(&self, c: Cell) -> Option<usize> {
        let (ux, uy) = (
            c.0.wrapping_sub(self.origin.0) as u32,
            c.1.wrapping_sub(self.origin.1) as u32,
        );
        (ux < SIDE as u32 && uy < SIDE as u32).then(|| ux as usize * SIDE + uy as usize)
    }

    fn cell_of(&self, idx: usize) -> Cell {
        (
            self.origin.0 + (idx / SIDE) as i32,
            self.origin.1 + (idx % SIDE) as i32,
        )
    }

    /// The node record of an index, if the current search has touched it.
    fn node(&self, idx: usize) -> Option<&Node> {
        let n = &self.scratch.nodes[idx];
        (n.stamp == self.scratch.stamp).then_some(n)
    }

    /// The node record, created (zeroed) when it is new in this search.
    fn node_mut(&mut self, idx: usize) -> &mut Node {
        let stamp = self.scratch.stamp;
        let n = &mut self.scratch.nodes[idx];
        if n.stamp != stamp {
            *n = Node {
                stamp,
                ..Node::default()
            };
        }
        n
    }

    fn nibble(&self, c: Cell) -> i32 {
        match self.terrain.nibble(c.0, c.1) {
            5 => 255,
            v => v as i32,
        }
    }

    fn blocked_at(&self, c: Cell) -> bool {
        self.level != -1 && self.level >= self.nibble(c)
    }

    fn in_terrain(&self, c: Cell) -> bool {
        match self.bounds {
            Some((w, h)) => c.0 >= 0 && c.1 >= 0 && c.0 < w && c.1 < h,
            None => true,
        }
    }

    /// 0 free, 1 wall, 2 outside the terrain, the window's ellipse, or already expanded.
    fn classify(&mut self, c: Cell) -> u8 {
        let idx = self.index(c);
        if let Some(i) = idx
            && let Some(n) = self.node(i)
            && n.flags & CLOSED != 0
        {
            return if n.flags & BLOCKED != 0 { 1 } else { 2 };
        }
        let mark = |s: &mut Self, bits: u8| {
            if let Some(i) = idx {
                s.node_mut(i).flags |= bits;
            }
        };
        if !self.in_terrain(c) {
            mark(self, CLOSED);
            return 2;
        }
        if self.blocked_at(c) {
            mark(self, CLOSED | BLOCKED);
            return 1;
        }
        let d2 = (c.0 - self.mid.0).wrapping_mul(c.0 - self.mid.0)
            + (c.1 - self.mid.1).wrapping_mul(c.1 - self.mid.1);
        if d2 as u32 >= self.mid_r2 {
            let sum = length(c.0 - self.start.0, c.1 - self.start.1)
                + length(c.0 - self.goal.0, c.1 - self.goal.1);
            if self.ellipse <= sum {
                mark(self, CLOSED);
                return 2;
            }
        }
        0
    }

    /// Whether the neighbour of `pos` in direction code `code` is a wall.
    fn is_blocked(&mut self, pos: Cell, code: usize) -> bool {
        let c = (pos.0 + DXY[code].0, pos.1 + DXY[code].1);
        let Some(i) = self.index(c) else {
            return false;
        };
        let unclassified = self.node(i).is_none_or(|n| n.flags & CLOSED != 0);
        if unclassified {
            let r = self.classify(c);
            if r != 0 {
                return r == 1;
            }
            let n = self.node_mut(i);
            n.flags = (n.flags & 0xf0) | SEEN;
            n.g = f32::INFINITY;
        }
        false
    }

    /// Remaining-distance estimate of a cell.
    fn hcell(&self, c: Cell) -> f32 {
        let b = (c.0 - self.goal.0, c.1 - self.goal.1);
        let ln = length(b.0, b.1);
        let mut d = ln;
        if self.params.radius >= 0 {
            d = (ln - self.params.radius as f32).max(0.0);
        }
        let cross = self
            .a
            .1
            .wrapping_mul(b.0)
            .wrapping_sub(b.1.wrapping_mul(self.a.0))
            .wrapping_abs();
        let c = cross as f32 * 0.001f32;
        let (maxd, mind) = (self.params.max_dist as f32, self.params.min_dist as f32);
        if maxd < d {
            return (d - maxd) + c;
        }
        if mind > d {
            return (mind - d) + c;
        }
        0.0
    }

    fn jump_limit(&self, pos: Cell, d: usize, g: f32, h: f32, diagonal: bool) -> u32 {
        let v = ((pos.0 - self.goal.0) as f32, (pos.1 - self.goal.1) as f32);
        let dot = UNIT[d].1 * v.1 + UNIT[d].0 * v.0;
        let q = (dot / (self.max_ext + h)).max(0.0);
        let mut x = q * 5.0f32 + 3.0f32;
        x += ((g + h) - self.h_start) * 0.2f32;
        if diagonal {
            x *= 0.7071f32;
        }
        let n = x as i64;
        if n < 0 || n > u32::MAX as i64 {
            return self.cap;
        }
        (n as u32).min(self.cap)
    }

    fn push(&mut self, idx: usize) {
        let f = self.scratch.nodes[idx].f;
        heap_push(&mut self.scratch.heap, idx as u32, f);
    }

    /// Writes a finished node (g, h, f, direction) and queues it.
    fn finish(&mut self, idx: usize, g: f32, h: f32, d: usize) {
        let n = self.node_mut(idx);
        n.h = h;
        n.g = g;
        n.f = h + g;
        n.flags = (n.flags & 0x70) | d as u8;
        self.push(idx);
    }

    fn jump_straight(&mut self, pos: Cell, d: usize, g: f32, h: f32) {
        let limit = self.jump_limit(pos, d, g, h, false);
        let step = (-DXY[d].0, -DXY[d].1);
        let row = ROW[d];
        let (mut flag_a, mut flag_b) = (false, false);
        let (mut x, mut y) = pos;
        let mut k = 1u32;
        let newg = loop {
            x += step.0;
            y += step.1;
            let c = (x, y);
            let Some(i) = self.index(c) else {
                return;
            };
            if self.node(i).is_none_or(|n| n.flags & CLOSED != 0) && self.classify(c) != 0 {
                return;
            }
            let newg = k as f32 + g;
            if let Some(n) = self.node(i)
                && n.flags & CLOSED == 0
                && n.g <= newg
            {
                return;
            }
            if k == limit {
                break newg;
            }
            let mut forced = false;
            if flag_a || self.is_blocked(c, row[0]) {
                if !self.is_blocked(c, row[2]) {
                    forced = true;
                } else {
                    flag_a = true;
                }
            }
            if !forced && (flag_b || self.is_blocked(c, row[1])) {
                if !self.is_blocked(c, row[3]) {
                    forced = true;
                } else {
                    flag_b = true;
                }
            }
            if forced {
                break newg;
            }
            let (dx, dy) = (x - self.goal.0, y - self.goal.1);
            if (dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))) as u32 <= self.goal2
                && self.hcell(c) == 0.0
            {
                self.finish(i, newg, 0.0, d);
                return;
            }
            let n = self.node_mut(i);
            n.flags = (n.flags & 0xf0) | d as u8 | SEEN;
            n.g = newg;
            k += 1;
        };
        let c = (x, y);
        let h = self.hcell(c);
        if let Some(i) = self.index(c) {
            self.finish(i, newg, h, d);
        }
    }

    fn jump_diagonal(&mut self, pos: Cell, d: usize, g: f32, h: f32) {
        let limit = self.jump_limit(pos, d, g, h, true);
        let step = (-DXY[d].0, -DXY[d].1);
        let row = ROW[d];
        let (mut x, mut y) = pos;
        let mut k = 1u32;
        loop {
            x += step.0;
            y += step.1;
            let c = (x, y);
            let Some(i) = self.index(c) else {
                return;
            };
            if self.node(i).is_none_or(|n| n.flags & CLOSED != 0) && self.classify(c) != 0 {
                return;
            }
            let newg = k as f32 * SQRT2 + g;
            if let Some(n) = self.node(i)
                && n.flags & CLOSED == 0
                && n.g <= newg
            {
                return;
            }
            let hh = self.hcell(c);
            let create = k == limit
                || hh == 0.0
                || (self.is_blocked(c, row[0]) && !self.is_blocked(c, row[2]))
                || (self.is_blocked(c, row[1]) && !self.is_blocked(c, row[3]));
            if create {
                self.finish(i, newg, hh, d);
                return;
            }
            self.jump_straight(c, row[0], newg, hh);
            self.jump_straight(c, row[1], newg, hh);
            let n = self.node_mut(i);
            n.flags = (n.flags & 0xf0) | d as u8 | SEEN;
            n.g = newg;
            k += 1;
        }
    }

    /// The client's cell-by-cell line check between two cells (blocked cells and cells outside
    /// the window end it).
    fn los(&self, a: Cell, b: Cell) -> bool {
        let (dx, dy) = ((b.0 - a.0).abs(), (b.1 - a.1).abs());
        let steep = dx < dy;
        let (m1, n1, m2, n2) = if steep {
            (a.1, a.0, b.1, b.0)
        } else {
            (a.0, a.1, b.0, b.1)
        };
        let (dm_major, dm_minor) = ((m2 - m1).abs(), (n2 - n1).abs());
        let step_minor = (n2 - n1).signum();
        let step_major = if m1 < m2 { 1 } else { -1 };
        let mut err = -(dm_major / 2);
        let (mut m, mut n) = (m1, n1);
        while m != m2 + step_major {
            let c = if steep { (n, m) } else { (m, n) };
            if self.blocked_at(c) {
                return false;
            }
            let Some(i) = self.index(c) else {
                return false;
            };
            if let Some(nd) = self.node(i)
                && nd.flags & (CLOSED | BLOCKED) == CLOSED | BLOCKED
            {
                return false;
            }
            err += dm_minor;
            m += step_major;
            if err >= 1 {
                n += step_minor;
                err -= dm_major;
            }
        }
        true
    }

    fn run(&mut self) -> Plan {
        let start = self.start;
        let si = self.index(start).expect("start in window");
        {
            let hs = self.h_start;
            let n = self.node_mut(si);
            n.g = 0.0;
            n.h = hs;
            n.f = hs;
            n.flags = (n.flags & 0x70) | 8;
        }
        self.push(si);
        let (mut best, mut best_h) = (start, self.h_start);
        let mut reached = None;
        while !self.scratch.heap.is_empty() {
            let (key, f) = heap_pop(&mut self.scratch.heap);
            let i = key as usize;
            let Some(&n) = self.node(i) else { continue };
            if f != n.f {
                continue;
            }
            let pos = self.cell_of(i);
            if n.h < best_h {
                best = pos;
                best_h = n.h;
            }
            if n.h == 0.0 {
                reached = Some(pos);
                break;
            }
            self.node_mut(i).flags |= CLOSED;
            let p = (n.flags & 0xf) as usize;
            let (g, h) = (n.g, n.h);
            if p == 8 {
                for u in 0..8 {
                    let c2 = (pos.0 + DXY[u].0, pos.1 + DXY[u].1);
                    let Some(j) = self.index(c2) else { continue };
                    if self.classify(c2) == 0 {
                        let step = if u < 4 { 1.0 } else { SQRT2 };
                        let hh = self.hcell(c2);
                        let nn = self.node_mut(j);
                        nn.g = step;
                        nn.h = hh;
                        nn.f = step + hh;
                        nn.flags = (nn.flags & 0x70) | REV[u] as u8;
                        self.push(j);
                    }
                }
                continue;
            }
            let row = ROW[p];
            let straight = if p > 3 {
                self.jump_diagonal(pos, p, g, h);
                self.jump_straight(pos, row[0], g, h);
                row[1]
            } else {
                p
            };
            self.jump_straight(pos, straight, g, h);
            for (side, behind) in [(row[0], row[2]), (row[1], row[3])] {
                let c2 = (pos.0 + DXY[side].0, pos.1 + DXY[side].1);
                let Some(j) = self.index(c2) else { continue };
                if self.node(j).is_none_or(|nn| nn.flags & CLOSED != 0) {
                    match self.classify(c2) {
                        0 => {
                            let nn = self.node_mut(j);
                            nn.g = f32::INFINITY;
                            nn.flags = (nn.flags & 0xf0) | REV[side] as u8 | SEEN;
                        }
                        1 => self.jump_diagonal(pos, REV[behind], g, h),
                        _ => {}
                    }
                }
            }
        }
        let end = reached.unwrap_or(best);
        if end == start {
            return Plan {
                path: Vec::new(),
                reached: reached.is_some(),
            };
        }
        // Rebuild backwards, keeping the cells where line of sight to the last kept one ends.
        let parent = |s: &Self, c: Cell| -> Cell {
            let d = s
                .index(c)
                .and_then(|i| s.node(i))
                .map_or(8, |n| (n.flags & 0xf) as usize)
                .min(7);
            (c.0 + DXY[d].0, c.1 + DXY[d].1)
        };
        let mut path = vec![end];
        let mut kept = end;
        let mut c = parent(self, end);
        while c != start {
            let next = parent(self, c);
            if !self.los(next, kept) {
                path.push(c);
                kept = c;
            }
            c = next;
            if path.len() >= 0x88 {
                return Plan::default();
            }
        }
        path.push(start);
        if path.len() > 2 {
            let mut i = 0;
            while i < path.len() {
                let last = path.len() - 1;
                let mut j = (i + 5).min(last);
                while j > i {
                    if self.los(path[j], path[i]) {
                        path.drain(i + 1..j);
                        break;
                    }
                    j -= 1;
                }
                i += 1;
            }
        }
        Plan {
            path,
            reached: reached.is_some(),
        }
    }
}

/// Plans a path from `start` to `goal` the way the client does.
///
/// `level` is the entity's clearance: cells whose grid value is at most `level` (value 5 counts
/// as 255) are walls; -1 turns the check off. A start cell that is itself a wall at `level > 0`
/// is planned again at level 0, which is what the client's fallback search came to.
pub fn plan(terrain: &dyn Walkable, start: Cell, goal: Cell, level: i32, params: &Params) -> Plan {
    let bounds = terrain.bounds();
    if let Some((w, h)) = bounds
        && (goal.0 < 0 || goal.1 < 0 || goal.0 >= w || goal.1 >= h)
    {
        return Plan::default();
    }
    SCRATCH.with(|s| {
        let mut s = s.borrow_mut();
        let s = &mut *s;
        if s.nodes.len() != SIDE * SIDE {
            s.nodes = vec![Node::default(); SIDE * SIDE];
            s.stamp = 0;
        }
        s.stamp = s.stamp.wrapping_add(1);
        if s.stamp == 0 {
            s.nodes.iter_mut().for_each(|n| n.stamp = 0);
            s.stamp = 1;
        }
        s.heap.clear();
        let max_ext = if params.radius < 0 {
            params.max_dist
        } else {
            params.max_dist.wrapping_add(params.radius as u32)
        };
        let a = (goal.0 - start.0, goal.1 - start.1);
        let d0 = length(start.0 - goal.0, start.1 - goal.1);
        let ellipse = (d0 * 2.0f32).clamp(20.0f32, 136.0f32);
        let mid = (
            (goal.0.wrapping_add(start.0)) / 2,
            (goal.1.wrapping_add(start.1)) / 2,
        );
        let r_m = ellipse * 0.433f32;
        let mut search = Search {
            terrain,
            bounds,
            scratch: s,
            start,
            goal,
            origin: (start.0 - HALF, start.1 - HALF),
            level,
            params: *params,
            a,
            goal2: max_ext.wrapping_mul(max_ext),
            max_ext: max_ext as f32,
            h_start: 0.0,
            cap: 5,
            ellipse,
            mid,
            mid_r2: (r_m * r_m) as u32,
        };
        search.h_start = search.hcell(start);
        search.cap = ((search.h_start * 0.33f32) as u32).clamp(5, 22);
        if level > 0 && search.blocked_at(start) {
            search.level = 0;
            if search.blocked_at(start) {
                return Plan::default();
            }
        } else if level != -1 && search.blocked_at(start) {
            return Plan::default();
        }
        search.run()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::VecGrid;

    fn open() -> VecGrid {
        VecGrid::open(400, 400)
    }

    #[test]
    fn an_open_field_gives_one_straight_leg() {
        let plan = plan(&open(), (100, 100), (111, 111), 0, &Params::default());
        assert!(plan.reached);
        assert_eq!(plan.path, vec![(111, 111), (100, 100)]);
    }

    #[test]
    fn a_goal_behind_a_wall_ends_at_the_closest_reachable_cell() {
        // A wall along x = 120 for y < 200 and the goal behind it.
        let mut g = open();
        for y in 0..200 {
            g.set(120, y, false);
        }
        let plan = plan(&g, (100, 100), (130, 100), 0, &Params::default());
        assert!(!plan.reached);
        let end = plan.path[0];
        assert!(end.0 <= 119, "{end:?}");
        assert!((end.1 - 100).abs() <= 3, "{end:?}");
    }

    #[test]
    fn the_path_goes_around_an_obstacle() {
        let mut g = open();
        for y in 95..=105 {
            g.set(105, y, false);
        }
        let plan = plan(&g, (100, 100), (110, 100), 0, &Params::default());
        assert!(plan.reached);
        assert_eq!(plan.path[0], (110, 100));
        assert!(plan.path.len() >= 3, "{:?}", plan.path);
        // Every leg keeps clear of the wall.
        let pts: Vec<_> = plan.path.iter().rev().copied().collect();
        for w in pts.windows(2) {
            assert!(super::super::pathfinding::line_of_sight(&g, w[0], w[1]));
        }
    }

    #[test]
    fn a_goal_at_the_start_has_no_path() {
        let p = plan(&open(), (100, 100), (100, 100), 0, &Params::default());
        assert!(p.path.is_empty());
    }

    #[test]
    fn a_goal_outside_the_terrain_has_no_path() {
        let p = plan(&open(), (100, 100), (500, 100), 0, &Params::default());
        assert_eq!(p, Plan::default());
    }

    #[test]
    fn the_heap_pops_in_cost_order_and_keeps_msvcs_tie_order() {
        let mut h = Vec::new();
        for (k, f) in [
            (1u32, 3.0f32),
            (2, 1.0),
            (3, 2.0),
            (4, 1.0),
            (5, 3.0),
            (6, 2.0),
        ] {
            heap_push(&mut h, k, f);
        }
        let order: Vec<u32> =
            std::iter::from_fn(|| (!h.is_empty()).then(|| heap_pop(&mut h).0)).collect();
        let costs: Vec<f32> = order
            .iter()
            .map(|k| [0.0, 3.0, 1.0, 2.0, 1.0, 3.0, 2.0][*k as usize])
            .collect();
        assert!(costs.windows(2).all(|w| w[0] <= w[1]), "{order:?}");
        // MSVC's sift order for this input.
        assert_eq!(order, vec![2, 4, 3, 6, 1, 5]);
    }
}
