//! The client's Voronoi diagram (`0x141ef73e0`, run by `0x141ef4720`): Steven Fortune's
//! sweepline in the `VoronoiDiagramGenerator` layout, adapted by GGG to 32-bit integers. The
//! generator uses it to jitter graph nodes inside their cells (`crate::layout`), so it is
//! ported operation by operation, integer rounding and overflow included
//! (`docs/re/terrain.md`, "Graph layout").
//!
//! - Sites are sorted by (y, x) (`0x141f05c10`); the bounding box is `(0, 0, w, h)` in grid
//!   cells.
//! - An edge `a·x… ` is stored unnormalised: `b = dx`, `a = -dy`,
//!   `c = (dx² + dy²)/2 + x₀·dx + y₀·dy` (so `b·x - a·y = c`).
//! - The edge and event hashes degenerate with integer arithmetic (every bucket index is the
//!   first or the last), so the beach line is searched from its left or right end and the
//!   event queue is one list sorted by (ystar, x).
//! - `clip_line` (`0x141ef7ee0`) pushes each finished edge to the front of the output list
//!   with both sites' numbers, and records which vertices it joins (`0x141ef9180`); vertices
//!   joined to one or three edges are the diagram's vertex list.

#[derive(Debug, Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
    /// Site number (input index) or vertex number.
    nbr: i32,
}

#[derive(Debug, Clone)]
struct Edge {
    a: i32,
    b: i32,
    c: i32,
    ep: [Option<usize>; 2],
    reg: [usize; 2],
}

#[derive(Debug, Clone)]
struct HalfEdge {
    left: usize,
    right: usize,
    edge: Option<usize>,
    pm: u8,
    vertex: Option<usize>,
    ystar: i32,
}

/// One finished Voronoi edge: its clipped end points and the two sites (input indices) it
/// separates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub p1: (i32, i32),
    pub p2: (i32, i32),
    pub sites: (usize, usize),
}

#[derive(Debug, Clone, Default)]
pub struct Diagram {
    /// In the client's list order (the last finished edge first).
    pub segments: Vec<Segment>,
    /// Vertices joined to one or three edges, by vertex number.
    pub vertices: Vec<(i32, i32)>,
}

/// The client's integer square root (`0x14196e010`): floor(sqrt(n)).
pub fn isqrt(n: u32) -> u32 {
    let (mut rem, mut root) = if n < 0x4000_0000 {
        (n, 0u32)
    } else {
        (n.wrapping_add(0xc000_0000), 0x10000u32)
    };
    for k in (0..15).rev() {
        let bit = 1u32 << k;
        let t = root.wrapping_add(bit).wrapping_shl(k);
        if rem >= t {
            rem -= t;
            root |= bit << 1;
        }
    }
    root >> 1
}

struct Fortune {
    points: Vec<Point>,
    edges: Vec<Edge>,
    he: Vec<HalfEdge>,
    /// The event queue, sorted by (ystar, vertex x).
    pq: Vec<usize>,
    xmin: i32,
    xmax: i32,
    deltax: i32,
    bounds: [i32; 4],
    bottom: usize,
    /// Vertex number -> point.
    vertices: Vec<usize>,
    /// Per vertex number: neighbours (up to 3) and their count.
    adjacency: Vec<([i32; 3], u32)>,
    segments: Vec<Segment>,
}

const LEFT_END: usize = 0;
const RIGHT_END: usize = 1;

impl Fortune {
    fn point(&mut self, x: i32, y: i32) -> usize {
        self.points.push(Point { x, y, nbr: 0 });
        self.points.len() - 1
    }

    fn he_create(&mut self, edge: Option<usize>, pm: u8) -> usize {
        self.he.push(HalfEdge {
            left: 0,
            right: 0,
            edge,
            pm,
            vertex: None,
            ystar: 0,
        });
        self.he.len() - 1
    }

    fn el_insert(&mut self, lb: usize, new: usize) {
        let r = self.he[lb].right;
        self.he[new].left = lb;
        self.he[new].right = r;
        self.he[r].left = new;
        self.he[lb].right = new;
    }

    fn el_delete(&mut self, he: usize) {
        let (l, r) = (self.he[he].left, self.he[he].right);
        self.he[l].right = r;
        self.he[r].left = l;
    }

    fn leftreg(&self, he: usize) -> usize {
        match self.he[he].edge {
            None => self.bottom,
            Some(e) => self.edges[e].reg[if self.he[he].pm == 0 { 0 } else { 1 }],
        }
    }

    fn rightreg(&self, he: usize) -> usize {
        match self.he[he].edge {
            None => self.bottom,
            Some(e) => self.edges[e].reg[if self.he[he].pm == 0 { 1 } else { 0 }],
        }
    }

    fn bisect(&mut self, s1: usize, s2: usize) -> usize {
        let (p, q) = (self.points[s1], self.points[s2]);
        let dx = q.x.wrapping_sub(p.x);
        let dy = q.y.wrapping_sub(p.y);
        let c = (dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) / 2)
            .wrapping_add(p.x.wrapping_mul(dx))
            .wrapping_add(p.y.wrapping_mul(dy));
        self.edges.push(Edge {
            a: dy.wrapping_neg(),
            b: dx,
            c,
            ep: [None, None],
            reg: [s1, s2],
        });
        self.edges.len() - 1
    }

    /// `0x141ef7a40`.
    fn right_of(&self, he: usize, p: (i32, i32)) -> bool {
        let h = &self.he[he];
        let e = &self.edges[h.edge.expect("right_of on an end")];
        let top = self.points[e.reg[1]];
        let (px, py) = p;
        if top.x < px {
            if h.pm == 0 {
                return true;
            }
        } else if h.pm == 1 {
            return false;
        }
        let (a, b, c) = (e.a, e.b, e.c);
        let dxp = px.wrapping_sub(top.x);
        let above = if a.wrapping_abs() < b.wrapping_abs() {
            let dyp = py.wrapping_sub(top.y);
            let neg = (b ^ a.wrapping_neg()) < 0;
            let quick = if px <= top.x { neg } else { !neg };
            let fast = if !quick {
                let t = px.wrapping_mul(b).wrapping_sub(py.wrapping_mul(a));
                let r = if b < 0 { t < c } else { c < t };
                let r = if neg { !r } else { r };
                (!r).then_some(false)
            } else {
                let l = b.wrapping_mul(dyp);
                let rr = a.wrapping_mul(dxp).wrapping_neg();
                let r = if b < 0 { l <= rr } else { rr <= l };
                r.then_some(true)
            };
            match fast {
                Some(v) => v,
                None => {
                    let dxs = top.x.wrapping_sub(self.points[e.reg[0]].x);
                    let lhs = dyp
                        .wrapping_mul(dyp)
                        .wrapping_sub(dxp.wrapping_mul(dxp))
                        .wrapping_mul(a);
                    let rhs = (b.wrapping_mul(dxp).wrapping_mul(2).wrapping_div(dxs)
                        .wrapping_add(b)
                        .wrapping_add(a.wrapping_mul(a).wrapping_div(b)))
                    .wrapping_mul(dxs)
                    .wrapping_mul(dyp);
                    let r = if b < 0 { rhs < lhs } else { lhs < rhs };
                    if neg { !r } else { r }
                }
            }
        } else {
            let yl = px.wrapping_mul(b).wrapping_sub(c).wrapping_div(a);
            let t1 = py.wrapping_sub(yl);
            let t3 = yl.wrapping_sub(top.y);
            t3.wrapping_mul(t3).wrapping_add(dxp.wrapping_mul(dxp)) < t1.wrapping_mul(t1)
        };
        if h.pm != 0 { !above } else { above }
    }

    /// `0x141ef76f0`: the beach-line half edge left of `p`.
    fn el_leftbnd(&self, p: (i32, i32)) -> usize {
        let start = if self.deltax != 0 && (p.0.wrapping_sub(self.xmin)) / self.deltax >= 1 {
            RIGHT_END
        } else {
            LEFT_END
        };
        let mut he = start;
        if he == LEFT_END || (he != RIGHT_END && self.right_of(he, p)) {
            loop {
                he = self.he[he].right;
                if he == RIGHT_END || !self.right_of(he, p) {
                    break;
                }
            }
            self.he[he].left
        } else {
            loop {
                he = self.he[he].left;
                if he == LEFT_END || self.right_of(he, p) {
                    break;
                }
            }
            he
        }
    }

    /// `0x141ef78f0`.
    fn intersect(&mut self, el1: usize, el2: usize) -> Option<usize> {
        let e1 = self.edges[self.he[el1].edge?].clone();
        let e2 = self.edges[self.he[el2].edge?].clone();
        if e1.reg[1] == e2.reg[1] {
            return None;
        }
        let d = e1.a.wrapping_mul(e2.b).wrapping_sub(e1.b.wrapping_mul(e2.a));
        if d == 0 {
            return None;
        }
        let (r1, r2) = (self.points[e1.reg[1]], self.points[e2.reg[1]]);
        let (q1, q2) = (self.points[e1.reg[0]], self.points[e2.reg[0]]);
        let r1_ge = r1.y > r2.y || (r1.y == r2.y && r1.x >= r2.x);
        let sy = r1.y.wrapping_add(q1.y);
        let sx = q1.x.wrapping_add(r1.x);
        let l = (sy.wrapping_sub(q2.y).wrapping_sub(r2.y) as i64) * (e2.a as i64)
            - (sx.wrapping_sub(q2.x).wrapping_sub(r2.x) as i64) * (e2.b as i64);
        let xint = (((e1.a as i64).wrapping_mul(l) / d as i64) as i32).wrapping_add(sx) / 2;
        let yint = (((e1.b as i64).wrapping_mul(l) / d as i64) as i32).wrapping_add(sy) / 2;
        let (el, e) = if r1_ge { (el2, &e2) } else { (el1, &e1) };
        let bad = if xint < self.points[e.reg[1]].x {
            self.he[el].pm == 1
        } else {
            self.he[el].pm == 0
        };
        (!bad).then(|| self.point(xint, yint))
    }

    fn dist(&self, p: usize, s: usize) -> i32 {
        let (p, s) = (self.points[p], self.points[s]);
        let dx = p.x.wrapping_sub(s.x);
        let dy = p.y.wrapping_sub(s.y);
        isqrt(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32) as i32
    }

    fn pq_insert(&mut self, he: usize, v: usize, offset: i32) {
        self.he[he].vertex = Some(v);
        let ystar = self.points[v].y.wrapping_add(offset);
        self.he[he].ystar = ystar;
        let vx = self.points[v].x;
        let at = self
            .pq
            .iter()
            .position(|&o| {
                let oy = self.he[o].ystar;
                ystar <= oy && (ystar != oy || vx <= self.points[self.he[o].vertex.unwrap()].x)
            })
            .unwrap_or(self.pq.len());
        self.pq.insert(at, he);
    }

    fn pq_delete(&mut self, he: usize) {
        if self.he[he].vertex.is_some() {
            if let Some(i) = self.pq.iter().position(|&o| o == he) {
                self.pq.remove(i);
            }
            self.he[he].vertex = None;
        }
    }

    fn new_vertex_number(&mut self, v: usize) {
        self.points[v].nbr = self.vertices.len() as i32;
        self.vertices.push(v);
    }

    /// `0x141ef9180`.
    fn join(&mut self, a: i32, b: i32) {
        let need = a.max(b) as usize + 1;
        if self.adjacency.len() < need {
            self.adjacency.resize(need, ([-1; 3], 0));
        }
        for (x, y) in [(a, b), (b, a)] {
            let slot = &mut self.adjacency[x as usize];
            if let Some(i) = slot.0.iter().position(|&s| s == -1) {
                slot.0[i] = y;
                slot.1 = i as u32 + 1;
            }
        }
    }

    fn endpoint(&mut self, e: usize, lr: usize, v: usize) {
        self.edges[e].ep[lr] = Some(v);
        if self.edges[e].ep[1 - lr].is_some() {
            self.clip_line(e);
        }
    }

    /// `0x141ef7ee0`.
    fn clip_line(&mut self, e: usize) {
        let edge = self.edges[e].clone();
        let (r0, r1) = (self.points[edge.reg[0]], self.points[edge.reg[1]]);
        let ddy = r1.y.wrapping_sub(r0.y);
        let ddx = r1.x.wrapping_sub(r0.x);
        if ddx.wrapping_mul(ddx).wrapping_add(ddy.wrapping_mul(ddy)) < 2 {
            return;
        }
        let [xmin, ymin, xmax, ymax] = self.bounds;
        let (a, b, c) = (edge.a, edge.b, edge.c);
        let (mut f1, mut f2) = (false, false);
        let ybranch = a.wrapping_abs() < b.wrapping_abs();
        let swap = ybranch && ((b >> 31) == (a.wrapping_neg() >> 31));
        let (s1, s2) = if swap {
            (edge.ep[1], edge.ep[0])
        } else {
            (edge.ep[0], edge.ep[1])
        };
        let div = |n: i32, d: i32| if d == 0 { 0 } else { n.wrapping_div(d) };
        let (mut x1, mut y1, mut x2, mut y2);
        if ybranch {
            y1 = match s1 {
                Some(s) if self.points[s].y > ymin => self.points[s].y,
                _ => {
                    f1 = true;
                    ymin
                }
            };
            if ymax < y1 {
                f1 = true;
                y1 = ymax;
            }
            x1 = div(a.wrapping_mul(y1).wrapping_add(c), b);
            y2 = match s2 {
                Some(s) if self.points[s].y < ymax => self.points[s].y,
                _ => {
                    f2 = true;
                    ymax
                }
            };
            if y2 < ymin {
                f2 = true;
                y2 = ymin;
            }
            x2 = div(a.wrapping_mul(y2).wrapping_add(c), b);
            if (x2 < xmin && x1 < xmin) || (xmax < x2 && xmax < x1) {
                return;
            }
            if a != 0 {
                if xmax < x1 {
                    f1 = true;
                    y1 = div(b.wrapping_mul(xmax).wrapping_sub(c), a);
                    x1 = xmax;
                }
                if x1 < xmin {
                    f1 = true;
                    y1 = div(b.wrapping_mul(xmin).wrapping_sub(c), a);
                    x1 = xmin;
                }
                if xmax < x2 {
                    f2 = true;
                    y2 = div(b.wrapping_mul(xmax).wrapping_sub(c), a);
                    x2 = xmax;
                }
                if x2 < xmin {
                    f2 = true;
                    y2 = div(xmin.wrapping_mul(b).wrapping_sub(c), a);
                    x2 = xmin;
                }
            }
        } else {
            x1 = match s1 {
                Some(s) if self.points[s].x > xmin => self.points[s].x,
                _ => {
                    f1 = true;
                    xmin
                }
            };
            if xmax < x1 {
                f1 = true;
                x1 = xmax;
            }
            y1 = div(b.wrapping_mul(x1).wrapping_sub(c), a);
            x2 = match s2 {
                Some(s) if self.points[s].x < xmax => self.points[s].x,
                _ => {
                    f2 = true;
                    xmax
                }
            };
            if x2 < xmin {
                f2 = true;
                x2 = xmin;
            }
            y2 = div(b.wrapping_mul(x2).wrapping_sub(c), a);
            if (ymax < y2 && ymax < y1) || (y2 < ymin && y1 < ymin) {
                return;
            }
            if b != 0 {
                if ymax < y1 {
                    f1 = true;
                    x1 = div(a.wrapping_mul(ymax).wrapping_add(c), b);
                    y1 = ymax;
                }
                if y1 < ymin {
                    f1 = true;
                    x1 = div(a.wrapping_mul(ymin).wrapping_add(c), b);
                    y1 = ymin;
                }
                if ymax < y2 {
                    f2 = true;
                    x2 = div(a.wrapping_mul(ymax).wrapping_add(c), b);
                    y2 = ymax;
                }
                if y2 < ymin {
                    f2 = true;
                    x2 = div(ymin.wrapping_mul(a).wrapping_add(c), b);
                    y2 = ymin;
                }
            }
        }
        let on_x_border = x1 == x2 && (x2 == xmin || x2 == xmax);
        let on_y_border = y1 == y2 && (y2 == ymin || y2 == ymax);
        if on_x_border || on_y_border {
            return;
        }
        self.segments.push(Segment {
            p1: (x1, y1),
            p2: (x2, y2),
            sites: (r0.nbr as usize, r1.nbr as usize),
        });
        let v1 = if f1 {
            let v = self.point(x1, y1);
            self.new_vertex_number(v);
            v
        } else {
            s1.unwrap()
        };
        let v2 = if f2 {
            let v = self.point(x2, y2);
            self.new_vertex_number(v);
            v
        } else {
            s2.unwrap()
        };
        self.join(self.points[v1].nbr, self.points[v2].nbr);
    }
}

/// Runs the client's Voronoi over `sites` inside `bounds` = `[xmin, ymin, xmax, ymax]`.
pub fn voronoi(sites: &[(i32, i32)], bounds: [i32; 4]) -> Diagram {
    if sites.is_empty() {
        return Diagram::default();
    }
    let mut points: Vec<Point> = sites
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| Point {
            x,
            y,
            nbr: i as i32,
        })
        .collect();
    points.sort_by_key(|p| (p.y, p.x));
    let (mut xmin, mut xmax) = (points[0].x, points[0].x);
    for p in &points {
        if p.x < xmin {
            xmin = p.x;
        } else if p.x > xmax {
            xmax = p.x;
        }
    }
    let n = points.len();
    let end = |_| HalfEdge {
        left: LEFT_END,
        right: RIGHT_END,
        edge: None,
        pm: 0,
        vertex: None,
        ystar: 0,
    };
    let mut f = Fortune {
        points,
        edges: Vec::new(),
        he: vec![end(()), end(())],
        pq: Vec::new(),
        xmin,
        xmax,
        deltax: xmax - xmin,
        bounds,
        bottom: 0,
        vertices: Vec::new(),
        adjacency: Vec::new(),
        segments: Vec::new(),
    };
    let _ = f.xmax;
    let mut next_site = 1usize;
    let mut newsite = (n > 1).then_some(1usize);
    loop {
        let circle = match newsite {
            None => {
                if f.pq.is_empty() {
                    break;
                }
                true
            }
            Some(ns) => {
                if f.pq.is_empty() {
                    false
                } else {
                    let top = f.pq[0];
                    let ystar = f.he[top].ystar;
                    let vx = f.points[f.he[top].vertex.unwrap()].x;
                    let p = f.points[ns];
                    !(p.y < ystar || (p.y == ystar && p.x < vx))
                }
            }
        };
        if !circle {
            let ns = newsite.unwrap();
            let p = (f.points[ns].x, f.points[ns].y);
            let lbnd = f.el_leftbnd(p);
            let rbnd = f.he[lbnd].right;
            let bot = f.rightreg(lbnd);
            let e = f.bisect(bot, ns);
            let bisector = f.he_create(Some(e), 0);
            f.el_insert(lbnd, bisector);
            if let Some(v) = f.intersect(lbnd, bisector) {
                f.pq_delete(lbnd);
                let d = f.dist(v, ns);
                f.pq_insert(lbnd, v, d);
            }
            let bisector2 = f.he_create(Some(e), 1);
            f.el_insert(bisector, bisector2);
            if let Some(v) = f.intersect(bisector2, rbnd) {
                let d = f.dist(v, ns);
                f.pq_insert(bisector2, v, d);
            }
            next_site += 1;
            newsite = (next_site <= n - 1).then_some(next_site);
            continue;
        }
        let lbnd = f.pq.remove(0);
        let llbnd = f.he[lbnd].left;
        let rbnd = f.he[lbnd].right;
        let rrbnd = f.he[rbnd].right;
        let bot = f.leftreg(lbnd);
        let top = f.rightreg(rbnd);
        let v = f.he[lbnd].vertex.unwrap();
        f.new_vertex_number(v);
        let (le, lpm) = (f.he[lbnd].edge.unwrap(), f.he[lbnd].pm as usize);
        f.endpoint(le, lpm, v);
        let (re, rpm) = (f.he[rbnd].edge.unwrap(), f.he[rbnd].pm as usize);
        f.endpoint(re, rpm, v);
        f.el_delete(lbnd);
        f.pq_delete(rbnd);
        f.el_delete(rbnd);
        let (b, t, pm) = if f.points[top].y < f.points[bot].y {
            (top, bot, 1u8)
        } else {
            (bot, top, 0u8)
        };
        let e = f.bisect(b, t);
        let bisector = f.he_create(Some(e), pm);
        f.el_insert(llbnd, bisector);
        f.endpoint(e, 1 - pm as usize, v);
        if let Some(p) = f.intersect(llbnd, bisector) {
            f.pq_delete(llbnd);
            let d = f.dist(p, b);
            f.pq_insert(llbnd, p, d);
        }
        if let Some(p) = f.intersect(bisector, rrbnd) {
            let d = f.dist(p, b);
            f.pq_insert(bisector, p, d);
        }
    }
    let mut he = f.he[LEFT_END].right;
    while he != RIGHT_END {
        if let Some(e) = f.he[he].edge {
            f.clip_line(e);
        }
        he = f.he[he].right;
    }
    let vertices = f
        .adjacency
        .iter()
        .enumerate()
        .filter(|(_, (_, count))| *count == 1 || *count == 3)
        .filter_map(|(i, _)| f.vertices.get(i))
        .map(|&v| (f.points[v].x, f.points[v].y))
        .collect();
    f.segments.reverse();
    Diagram {
        segments: f.segments,
        vertices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_is_floor_sqrt() {
        for n in [0u32, 1, 2, 3, 4, 15, 16, 17, 99, 100, 1 << 30, u32::MAX, 2_513_808] {
            assert_eq!(isqrt(n) as u64, (n as f64).sqrt().floor() as u64, "n={n}");
        }
    }

    #[test]
    fn square_of_four_sites() {
        // Four sites in a square: two vertical and two horizontal edges meeting at the centre.
        let d = voronoi(&[(10, 10), (30, 10), (10, 30), (30, 30)], [0, 0, 40, 40]);
        assert!(d.segments.len() >= 4, "{d:?}");
        assert!(d.vertices.contains(&(20, 20)), "{d:?}");
    }
}
