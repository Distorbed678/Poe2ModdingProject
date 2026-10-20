//! Edge smoothing after routing (`0x1420bd650`): virtual edges leaving a room get an arc so
//! they start (and end) along the exit's direction, then corners are rounded. All integer,
//! no RNG. See `docs/re/terrain.md`, "Edge smoothing".

pub type Pos = (i32, i32);

/// `0x142f10660`: 256 samples of `127·sin`, signed.
const SINE: [i8; 256] = {
    const RAW: [u8; 256] = [
        0, 3, 6, 9, 12, 16, 19, 22, 25, 28, 31, 34, 37, 40, 43, 46, 49, 51, 54, 57, 60, 63, 65,
        68, 71, 73, 76, 78, 81, 83, 85, 88, 90, 92, 94, 96, 98, 100, 102, 104, 106, 107, 109, 111,
        112, 113, 115, 116, 117, 118, 120, 121, 122, 122, 123, 124, 125, 125, 126, 126, 126, 127,
        127, 127, 127, 127, 127, 127, 126, 126, 126, 125, 125, 124, 123, 122, 122, 121, 120, 118,
        117, 116, 115, 113, 112, 111, 109, 107, 106, 104, 102, 100, 98, 96, 94, 92, 90, 88, 85, 83,
        81, 78, 76, 73, 71, 68, 65, 63, 60, 57, 54, 51, 49, 46, 43, 40, 37, 34, 31, 28, 25, 22,
        19, 16, 12, 9, 6, 3, 0, 253, 250, 247, 244, 240, 237, 234, 231, 228, 225, 222, 219, 216,
        213, 210, 207, 205, 202, 199, 196, 193, 191, 188, 185, 183, 180, 178, 175, 173, 171, 168,
        166, 164, 162, 160, 158, 156, 154, 152, 150, 149, 147, 145, 144, 143, 141, 140, 139, 138,
        136, 135, 134, 134, 133, 132, 131, 131, 130, 130, 130, 129, 129, 129, 129, 129, 129, 129,
        130, 130, 130, 131, 131, 132, 133, 134, 134, 135, 136, 138, 139, 140, 141, 143, 144, 145,
        147, 149, 150, 152, 154, 156, 158, 160, 162, 164, 166, 168, 171, 173, 175, 178, 180, 183,
        185, 188, 191, 193, 196, 199, 202, 205, 207, 210, 213, 216, 219, 222, 225, 228, 231, 234,
        237, 240, 244, 247, 250, 253,
    ];
    let mut out = [0i8; 256];
    let mut i = 0;
    while i < 256 {
        out[i] = RAW[i] as i8;
        i += 1;
    }
    out
};

/// `0x1421c4f40`: a vector of length about `r / 2` at angle `a` (0x10000 = full turn). The
/// client weights the two table samples the other way round (`T[i+1]·(255−f) + T[i]·f`).
pub fn polar(r: i32, a: u16) -> Pos {
    let sample = |a: u16| -> i32 {
        let i = (a >> 8) as usize;
        let f = (a & 0xff) as i32;
        let v = (SINE[(i + 1) & 0xff] as i32 * (255 - f) + SINE[i] as i32 * f) / 255;
        v.clamp(-127, 127)
    };
    let s = sample(a);
    let c = sample(0x4000u16.wrapping_sub(a));
    ((c * r) / 255, (s * r) / 255)
}

/// An edge as the smoothing passes see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmoothEdge {
    pub from: Pos,
    pub to: Pos,
    pub points: Vec<Pos>,
    /// End offsets (from, to).
    pub offsets: [Pos; 2],
    /// Max width (`+0x84`).
    pub width: i32,
}

fn angle(a: Pos, b: Pos) -> u16 {
    crate::rooms::angle_between(a, b)
}

/// The arc points from the node towards the first point (`0x1420d1b70`, start = true) or the
/// last point towards the end node (`0x1420d2470`). Returns the points to insert, in order.
fn arc(node: Pos, p: Pos, off: Pos) -> Option<Vec<Pos>> {
    let (ox, oy) = off;
    let aligned = if ox == 0 { node.0 == p.0 } else { node.1 == p.1 };
    if aligned {
        return None;
    }
    let dx = p.0 - node.0;
    let s = if ox != 0 { ox } else { -oy };
    let num = s
        .wrapping_mul(
            (node.1 - (node.1 + p.1) / 2)
                .wrapping_mul(node.1 - p.1)
                .wrapping_sub((node.0 - (node.0 + p.0) / 2).wrapping_mul(dx)),
        )
        .wrapping_mul(3);
    let den = dx.wrapping_mul(-oy).wrapping_sub(ox.wrapping_mul(node.1 - p.1));
    if den == 0 {
        return None;
    }
    let r = (num / den) / 2;
    let c = if ox == 0 { (node.0 + r, node.1) } else { (node.0, node.1 + r) };
    if c == node || c == p {
        return None;
    }
    let dy = p.1 - node.1;
    let oo = (oy * oy + ox * ox) as u32 as u64;
    let dd = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32;
    let dy2 = dy - oy;
    let l = (dd as u64)
        .wrapping_sub((dx - ox).wrapping_mul(dx - ox).wrapping_add(dy2.wrapping_mul(dy2)) as u32 as u64)
        .wrapping_add(oo) as i64;
    if l.wrapping_mul(l) >= ((dd as u64).wrapping_mul(oo).wrapping_mul(3)) as i64 {
        return None;
    }
    let mut ang = angle((node.0 - c.0, node.1 - c.1), (p.0 - c.0, p.1 - c.1));
    let ra = r.abs();
    let mut out = Vec::new();
    let mut push = |u: u32| {
        let v = polar(ra, u as u16);
        out.push((c.0 + v.0, c.1 + v.1));
    };
    if ox == 1 {
        if p.1 < node.1 {
            if c.0 <= p.0 {
                ang = ang.wrapping_neg();
            }
            let a = ang.wrapping_add(0x4000) as u32;
            let mut u: u32 = if a < 0x4001 { 0x2000 } else { 0x12000 };
            while a < u {
                push(u);
                u -= 0x2000;
            }
        } else {
            if p.0 < c.0 {
                ang = ang.wrapping_neg();
            }
            ang = ang.wrapping_add(0xc000);
            let lim: u32 = if ang > 0xbfff { ang as u32 } else { ang as u32 + 0x10000 };
            let mut u = 0xe000u32;
            while u < lim {
                push(u);
                u += 0x2000;
            }
        }
    } else if ox == -1 {
        if p.1 < node.1 {
            if c.0 < p.0 {
                ang = ang.wrapping_neg();
            }
            ang = ang.wrapping_add(0x4000);
            let lim: u32 = if ang > 0x3fff { ang as u32 } else { ang as u32 + 0x10000 };
            let mut u = 0x6000u32;
            while u < lim {
                push(u);
                u += 0x2000;
            }
        } else {
            if p.0 <= c.0 {
                ang = ang.wrapping_neg();
            }
            let a = ang.wrapping_add(0xc000) as u32;
            let mut u: u32 = if a < 0xc001 { 0xa000 } else { 0x1a000 };
            while a < u {
                push(u);
                u -= 0x2000;
            }
        }
    } else if oy == 1 {
        if p.0 < node.0 {
            if p.1 < c.1 {
                ang = ang.wrapping_neg();
            }
            let mut u = 0x2000u32;
            while u < ang as u32 {
                push(u);
                u += 0x2000;
            }
        } else {
            if c.1 <= p.1 {
                ang = ang.wrapping_neg();
            }
            let a = ang.wrapping_add(0x8000) as u32;
            let mut u: u32 = if a < 0x8001 { 0x6000 } else { 0x16000 };
            while a < u {
                push(u);
                u -= 0x2000;
            }
        }
    } else if p.0 < node.0 {
        if p.1 <= c.1 {
            ang = ang.wrapping_neg();
        }
        let mut u: u32 = if ang == 0 { 0xffff_e000 } else { 0xe000 };
        while (ang as u32) < u {
            push(u);
            u = u.wrapping_sub(0x2000);
        }
    } else {
        if c.1 < p.1 {
            ang = ang.wrapping_neg();
        }
        ang = ang.wrapping_add(0x8000);
        let lim: u32 = if ang > 0x7fff { ang as u32 } else { ang as u32 + 0x10000 };
        let mut u = 0xa000u32;
        while u < lim {
            push(u);
            u += 0x2000;
        }
    }
    Some(out)
}

/// `0x1420d1b70`: the arc at the edge's start; returns whether points were added.
pub fn arc_start(e: &mut SmoothEdge) -> bool {
    let p = e.points.first().copied().unwrap_or(e.to);
    match arc(e.from, p, e.offsets[0]) {
        Some(pts) => {
            e.points.splice(0..0, pts);
            true
        }
        None => false,
    }
}

/// `0x1420d2470`: the arc at the edge's end.
pub fn arc_end(e: &mut SmoothEdge) -> bool {
    let p = e.points.last().copied().unwrap_or(e.from);
    match arc(e.to, p, e.offsets[1]) {
        Some(pts) => {
            // `0x141fee340` appends the generated points from the last one back.
            e.points.extend(pts.into_iter().rev());
            true
        }
        None => false,
    }
}

/// A node of the layout graph (client numbering).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LNode {
    pub pos: Pos,
    pub slots: [Option<usize>; 4],
    /// `+0x88`: a room node (or one of its exits), or a dead end pushed off the map.
    pub fixed: bool,
    /// `+0x89`: next to a virtual edge's end (`0x1420c7400`, then the virtual split).
    pub virtual_end: bool,
    /// The `.tgr` node this is (None for nodes made later).
    pub orig: Option<usize>,
}

/// An edge of the layout graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LEdge {
    pub from: usize,
    pub to: usize,
    pub points: Vec<Pos>,
    pub offsets: [Pos; 2],
    /// `.et` path (lower case).
    pub edge_type: String,
    pub virtual_edge: bool,
    /// (min, max) width (`+0x80`/`+0x84`).
    pub width: (i32, i32),
    /// Chirality (`+0x41`).
    pub chirality: bool,
    /// Virtual end points per end (`+0x4c`/`+0x5c`, flags `+0x48`/`+0x49`).
    pub vends: [Option<[Pos; 2]>; 2],
    /// Split already (`+0x6d`) / ends fixed (`+0x4a`).
    pub split: bool,
    pub fixed: bool,
}

/// The graph between routing and tile placement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutGraph {
    pub nodes: Vec<LNode>,
    pub edges: Vec<LEdge>,
    /// Logical links of tile-less virtual nodes (`0x1420c5ee0`): up to four nodes each (the
    /// two edge ends, then the nodes the virtual split makes of them, `0x1420baf40`).
    pub links: Vec<[Option<usize>; 4]>,
    /// Links per node (`+0x90`/`+0x94`).
    pub node_links: std::collections::BTreeMap<usize, Vec<usize>>,
}

impl LayoutGraph {
    /// `0x1420baf40`: `node` split off `new`; the links holding `node` take `new` as its partner.
    pub fn link_split(&mut self, node: usize, new: usize) {
        let Some(ls) = self.node_links.get(&node).cloned() else { return };
        for l in ls {
            let link = &mut self.links[l];
            if link[0] == Some(node) && link[2].is_none() {
                link[2] = Some(new);
            } else if link[1] == Some(node) && link[3].is_none() {
                link[3] = Some(new);
            }
        }
    }
}

impl LayoutGraph {
    fn len(&self, e: usize) -> i32 {
        self.edges[e].points.len() as i32
    }

    /// Point `i` of an edge: 0 the start node, `n + 1` (or beyond) the end node.
    fn p(&self, e: usize, i: i32) -> Pos {
        let ed = &self.edges[e];
        if i <= 0 {
            self.nodes[ed.from].pos
        } else if i as usize > ed.points.len() {
            self.nodes[ed.to].pos
        } else {
            ed.points[i as usize - 1]
        }
    }

    fn set_p(&mut self, e: usize, i: i32, v: Pos) {
        let n = self.len(e);
        if i == 0 {
            let f = self.edges[e].from;
            self.nodes[f].pos = v;
        } else if i == n + 1 {
            let t = self.edges[e].to;
            self.nodes[t].pos = v;
        } else {
            self.edges[e].points[i as usize - 1] = v;
        }
    }

    fn end(&self, e: usize, which: usize) -> usize {
        if which == 0 { self.edges[e].from } else { self.edges[e].to }
    }

    fn degree(&self, n: usize) -> usize {
        self.nodes[n].slots.iter().filter(|s| s.is_some()).count()
    }

    /// `0x1420b2cd0`: removes corner `i` of `e` (or, at a node joining `e` and `other`, moves
    /// the node onto the adjacent point and drops that point).
    fn remove_point(&mut self, e: usize, dir: usize, other: Option<(usize, usize)>, i: i32) -> bool {
        match other {
            None => {
                self.edges[e].points.remove(i as usize - 1);
                true
            }
            Some((o, odir)) => {
                let node = self.end(e, 1 - dir);
                if self.edges[e].points.is_empty() {
                    let op = &mut self.edges[o].points;
                    if op.is_empty() {
                        return false;
                    }
                    let k = if odir != 0 { op.len() - 1 } else { 0 };
                    let v = op.remove(k);
                    self.nodes[node].pos = v;
                } else {
                    let ep = &mut self.edges[e].points;
                    let k = if dir != 0 { 0 } else { ep.len() - 1 };
                    let v = ep.remove(k);
                    self.nodes[node].pos = v;
                }
                true
            }
        }
    }

    /// `0x1420ced20`: inserts `v` at index `idx` (0 / `n + 2` move the end node and keep its
    /// old position as a point).
    fn insert_point(&mut self, e: usize, idx: i32, v: Pos) {
        let n = self.len(e);
        if idx == 0 {
            let f = self.edges[e].from;
            let old = self.nodes[f].pos;
            self.edges[e].points.insert(0, old);
            self.nodes[f].pos = v;
        } else if idx == n + 2 {
            let t = self.edges[e].to;
            let old = self.nodes[t].pos;
            self.edges[e].points.push(old);
            self.nodes[t].pos = v;
        } else {
            self.edges[e].points.insert(idx as usize - 1, v);
        }
    }
}

#[derive(Default)]
struct RoundCtx {
    edges: std::collections::HashSet<usize>,
    nodes: std::collections::HashSet<usize>,
    changed: bool,
}

/// `0x1420b2f60`: rounds the corners along `e` from its end `dir` (0 = from the start),
/// continuing through nodes with two edges.
fn round(g: &mut LayoutGraph, ctx: &mut RoundCtx, mut e: usize, mut dir: usize) {
    let n = g.len(e);
    let (mut i, mut step, mut end_idx) = if dir == 0 { (1, 1, n + 2) } else { (n, -1, -1) };
    let mut stop = i - step;
    let sq = |a: Pos, b: Pos| {
        let dx = (b.0 - a.0).abs();
        let dy = (b.1 - a.1).abs();
        dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx))
    };
    loop {
        if ctx.edges.contains(&e) {
            return;
        }
        if i == end_idx {
            let node = g.end(e, 1 - dir);
            ctx.nodes.insert(node);
            ctx.edges.insert(e);
            let Some(next) = g.nodes[node].slots.iter().flatten().copied().find(|&x| x != e) else {
                return;
            };
            dir = (g.edges[next].from != node) as usize;
            let nn = g.len(next);
            if dir == 1 {
                step = -1;
                end_idx = -1;
                i = nn;
                stop = nn + 1;
            } else {
                step = 1;
                end_idx = nn + 2;
                i = 1;
                stop = 0;
            }
            e = next;
        }
        let prev = i - step;
        let a = g.p(e, prev);
        let b = g.p(e, i);
        let next_i = i + step;
        let mut other = None;
        let c = if next_i == end_idx {
            let node = g.end(e, 1 - dir);
            if ctx.nodes.contains(&node) {
                i += step;
                continue;
            }
            if g.degree(node) != 2 {
                ctx.edges.insert(e);
                return;
            }
            let o = g.nodes[node].slots.iter().flatten().copied().find(|&x| x != e).unwrap();
            let odir = (g.edges[o].from != node) as usize;
            other = Some((o, odir));
            if odir == 0 { g.p(o, 1) } else { g.p(o, g.len(o)) }
        } else {
            g.p(e, next_i)
        };
        if a == b || b == c || c == a {
            let ni;
            if !g.remove_point(e, dir, other, i) {
                g.set_p(e, i, ((a.0 + c.0) / 2, (c.1 + a.1) / 2));
                ni = next_i;
            } else {
                let len = g.len(e);
                end_idx = (len + 2) * (1 - dir as i32) - dir as i32;
                stop = (len + 1) * dir as i32;
                ni = dir as i32 - 1 + i;
            }
            i = if ni != stop { ni } else { ni + step };
            continue;
        }
        let d_ac = sq(a, c) as u32;
        let d_ab = sq(a, b);
        let d_bc = sq(b, c);
        let r = g.edges[e].width.1 + 10;
        let lab = crate::voronoi::isqrt(d_ab as u32) as i32;
        let lbc = crate::voronoi::isqrt(d_bc as u32) as i32;
        let s2 = ((lbc + lab).wrapping_mul(lbc + lab)) as u32;
        let t2 = ((lab - lbc).wrapping_mul(lab - lbc)) as u32;
        let fa = if s2 < d_ac { 1 } else { s2 - d_ac };
        let fb = if d_ac <= t2 { 1 } else { d_ac - t2 };
        let num = (fa as i32).wrapping_mul(r).wrapping_mul(r) as u32 as u64;
        let q = num / fb as u64;
        let d = crate::voronoi::isqrt(q as u32) as i32;
        let t_bc = (b.0 + ((c.0 - b.0) * d) / lbc, b.1 + ((c.1 - b.1) * d) / lbc);
        let t_ab = (((a.0 - b.0) * d) / lab + b.0, b.1 + ((a.1 - b.1) * d) / lab);
        let sharp = (lbc.wrapping_mul(lab).wrapping_mul(-0x4c))
            < ((d_bc.wrapping_sub(d_ac as i32)).wrapping_add(d_ab)).wrapping_mul(0x32);
        if !((q as u32) > 3 && sharp) {
            i += step;
            continue;
        }
        let distinct = |t: Pos| t != a && t != b && t != c;
        if lbc - 6 <= d {
            ctx.changed = true;
            if d < lab - 6 {
                g.set_p(e, i, t_ab);
                if distinct(t_ab) {
                    i = prev;
                    if i == stop {
                        i += step;
                    }
                } else {
                    i += step;
                }
                continue;
            }
            let ni;
            if !g.remove_point(e, dir, other, i) {
                g.set_p(e, i, ((a.0 + c.0) / 2, (c.1 + a.1) / 2));
                ni = next_i;
            } else {
                let len = g.len(e);
                end_idx = (len + 2) * (1 - dir as i32) - dir as i32;
                stop = (len + 1) * dir as i32;
                ni = (i - 1) + dir as i32;
            }
            i = ni;
            if i == stop {
                i += step;
            }
            continue;
        }
        if !distinct(t_bc) {
            i += step;
            continue;
        }
        if lab - 6 <= d {
            g.set_p(e, i, t_bc);
            ctx.changed = true;
            i = prev;
            if i == stop {
                i += step;
            }
            continue;
        }
        if !distinct(t_ab) {
            i += step;
            continue;
        }
        let dx = (t_bc.0 - t_ab.0).abs();
        let dy = (t_bc.1 - t_ab.1).abs();
        if ((dy * dy + dx * dx) as u32) < 0x101 {
            i += step;
            continue;
        }
        g.set_p(e, i, t_ab);
        g.insert_point(e, (step + 1) / 2 + i, t_bc);
        end_idx += 1 - dir as i32;
        stop = (g.len(e) + 1) * dir as i32;
        i -= (dir as i32 + 1) * step;
        ctx.changed = true;
        if i == stop {
            i += step;
        }
    }
}

/// `0x1420b3bd0`: one rounding pass over the graph; returns whether anything changed.
fn round_all(g: &mut LayoutGraph) -> bool {
    let mut ctx = RoundCtx::default();
    let mut ones = Vec::new();
    let mut many = Vec::new();
    for n in 0..g.nodes.len() {
        match g.degree(n) {
            0 => {}
            1 => ones.push(n),
            _ => many.push(n),
        }
    }
    for n in ones {
        let e = g.nodes[n].slots.iter().flatten().copied().next().unwrap();
        if !ctx.edges.contains(&e) {
            let dir = (g.edges[e].from != n) as usize;
            round(g, &mut ctx, e, dir);
        }
    }
    for n in many {
        for k in 0..4 {
            if let Some(e) = g.nodes[n].slots[k] {
                if !ctx.edges.contains(&e) {
                    let dir = (g.edges[e].from != n) as usize;
                    round(g, &mut ctx, e, dir);
                }
            }
        }
    }
    ctx.changed
}

/// `0x1420bd650` with its flag set: arcs at virtual edges' room ends, then corner rounding.
pub fn smooth(g: &mut LayoutGraph) {
    let starts: Vec<usize> = (0..g.edges.len())
        .filter(|&e| g.edges[e].virtual_edge && g.edges[e].offsets[0] != (0, 0))
        .collect();
    let ends: Vec<usize> = (0..g.edges.len())
        .filter(|&e| g.edges[e].virtual_edge && g.edges[e].offsets[1] != (0, 0))
        .collect();
    for _ in 0..5 {
        let mut changed = false;
        for &e in &starts {
            let mut s = SmoothEdge {
                from: g.nodes[g.edges[e].from].pos,
                to: g.nodes[g.edges[e].to].pos,
                points: std::mem::take(&mut g.edges[e].points),
                offsets: g.edges[e].offsets,
                width: g.edges[e].width.1,
            };
            changed |= arc_start(&mut s);
            g.edges[e].points = s.points;
        }
        for &e in &ends {
            let mut s = SmoothEdge {
                from: g.nodes[g.edges[e].from].pos,
                to: g.nodes[g.edges[e].to].pos,
                points: std::mem::take(&mut g.edges[e].points),
                offsets: g.edges[e].offsets,
                width: g.edges[e].width.1,
            };
            changed |= arc_end(&mut s);
            g.edges[e].points = s.points;
        }
        if !changed {
            break;
        }
    }
    for _ in 0..5 {
        if !round_all(g) {
            break;
        }
    }
}

/// `0x1420b3bd0` repeated up to 5 times while it changes something.
pub fn round_corners(g: &mut LayoutGraph) {
    for _ in 0..5 {
        if !round_all(g) {
            break;
        }
    }
}

/// `0x1420bd5f0`: a node's edges moved to the first slots.
pub fn compact_slots(g: &mut LayoutGraph) {
    for n in &mut g.nodes {
        let edges: Vec<usize> = n.slots.iter().flatten().copied().collect();
        n.slots = [None; 4];
        for (k, e) in edges.into_iter().enumerate() {
            n.slots[k] = Some(e);
        }
    }
}

/// `0x1420b6660`: each edge gets a point one step out of its end nodes along the end offsets.
pub fn add_end_points(g: &mut LayoutGraph) {
    for e in 0..g.edges.len() {
        let [o0, o1] = g.edges[e].offsets;
        if o0 != (0, 0) {
            let p = g.nodes[g.edges[e].from].pos;
            g.edges[e].points.insert(0, (p.0 + o0.0, p.1 + o0.1));
        }
        if o1 != (0, 0) {
            let p = g.nodes[g.edges[e].to].pos;
            g.edges[e].points.push((p.0 + o1.0, p.1 + o1.1));
        }
    }
}
