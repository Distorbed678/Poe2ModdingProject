//! Splitting virtual edges into two parallel edges (`0x1420bc400` → `0x1420bb4f0`): a wide
//! river becomes its two banks, each with its own edge type, at a random width per internal
//! point. See `Documentation/re/terrain.md`, "Virtual edges".

use crate::{
    rng::TerrainRng,
    smoothing::{LEdge, LNode, LayoutGraph, Pos},
};

/// What the split needs from an edge type.
#[derive(Debug, Clone, Default)]
pub struct SplitType {
    pub virtual_edge: bool,
    pub grounds: [Option<String>; 2],
    pub height_change: u8,
    pub splits: [(String, bool); 2],
}

fn mid(a: Pos, b: Pos) -> Pos {
    ((a.0 + b.0) / 2, (a.1 + b.1) / 2)
}

/// `0x1420b9c30`: the two points `w` either side of `b` on the bisector of `a → b → c` (a miter),
/// clamped to the map (`size` in grid cells).
pub fn miter(a: Pos, b: Pos, c: Pos, w: i32, size: Pos) -> [Pos; 2] {
    let dy_ab = a.1 - b.1;
    let dx_ab = b.0 - a.0;
    let dy_bc = b.1 - c.1;
    let dx_bc = c.0 - b.0;
    let lab = crate::voronoi::isqrt(dx_ab.wrapping_mul(dx_ab).wrapping_add(dy_ab.wrapping_mul(dy_ab)) as u32) as i32;
    let mut px = dy_ab.wrapping_mul(w);
    let mut py = dx_ab.wrapping_mul(w);
    let div = |a: i32, b: i32| if b == 0 { 0 } else { a.wrapping_div(b) };
    if dx_bc.wrapping_mul(dy_ab).wrapping_sub(dy_bc.wrapping_mul(dx_ab)) < 2 {
        px = div(px, -lab);
        py = div(py, -lab);
    } else {
        let lbc = crate::voronoi::isqrt(dx_bc.wrapping_mul(dx_bc).wrapping_add(dy_bc.wrapping_mul(dy_bc)) as u32) as i32;
        let qx = div(dy_bc.wrapping_mul(w), lbc);
        let qy = div(dx_bc.wrapping_mul(w), lbc);
        let a2 = (a.0 + div(px, lab), a.1 + div(py, lab));
        let c2 = (c.0 + qx, c.1 + qy);
        let k12 = (div(py, lab) - a2.1) + b.1;
        let k14 = (qx - c2.0) + b.0;
        let k8 = (div(px, lab) - a2.0) + b.0;
        let k11 = (qy - c2.1) + b.1;
        let den = k11.wrapping_mul(k8).wrapping_sub(k14.wrapping_mul(k12));
        if den < 2 {
            px = div(px, -lab);
            py = div(py, -lab);
        } else {
            let t = (a2.1 - c2.1).wrapping_mul(k14).wrapping_sub((a2.0 - c2.0).wrapping_mul(k11));
            px = (b.0 - div(k8.wrapping_mul(t), den)) - a2.0;
            py = (b.1 - div(t.wrapping_mul(k12), den)) - a2.1;
        }
    }
    let clamp = |p: Pos| (p.0.min(size.0 - 2).max(0), p.1.min(size.1 - 2).max(0));
    [clamp((b.0 + px, b.1 + py)), clamp((b.0 - px, b.1 - py))]
}

impl LayoutGraph {
    fn pt(&self, e: usize, i: i64) -> Pos {
        let ed = &self.edges[e];
        if i <= 0 {
            self.nodes[ed.from].pos
        } else if i as usize > ed.points.len() {
            self.nodes[ed.to].pos
        } else {
            ed.points[i as usize - 1]
        }
    }

    pub fn endpoint(&self, e: usize, w: usize) -> usize {
        if w == 0 { self.edges[e].from } else { self.edges[e].to }
    }

    fn set_end(&mut self, e: usize, w: usize, n: usize) {
        if w == 0 { self.edges[e].from = n } else { self.edges[e].to = n }
    }

    /// The point of `e` next to node `n`.
    fn next_to(&self, e: usize, n: usize) -> Pos {
        if self.edges[e].from == n {
            self.pt(e, 1)
        } else {
            self.pt(e, self.edges[e].points.len() as i64)
        }
    }

    fn add_node(&mut self, pos: Pos) -> usize {
        self.nodes.push(LNode { pos, slots: [None; 4], fixed: false, virtual_end: false });
        self.nodes.len() - 1
    }

    fn attach(&mut self, n: usize, e: usize) {
        if let Some(s) = self.nodes[n].slots.iter_mut().find(|s| s.is_none()) {
            *s = Some(e);
        }
    }
}

/// `0x1420ba050`: at a junction split into `node` and `new`, re-attach whichever of the
/// previous edge and its clone does not cross the other to `new`.
fn join(g: &mut LayoutGraph, pe: usize, pc: usize, node: usize, new: usize, first: bool) {
    let a = g.next_to(pe, node);
    let n = g.nodes[node].pos;
    let b = g.next_to(pc, node);
    let m = g.nodes[new].pos;
    let crosses = |a: Pos, n: Pos, b: Pos, m: Pos| {
        let r10 = n.0 - a.0;
        let r13 = n.1 - a.1;
        let r8x = m.0 - b.0;
        let r8y = m.1 - b.1;
        let den = r10 * r8y - r13 * r8x;
        if den == 0 {
            return false;
        }
        let t1 = (a.1 - b.1) * r8x - (a.0 - b.0) * r8y;
        let t2 = (a.1 - b.1) * r10 - (a.0 - b.0) * r13;
        if den < 1 {
            t1 < 1 && den <= t1 && t2 < 1 && den <= t2
        } else {
            t1 >= 0 && t1 <= den && t2 >= 0 && t2 <= den
        }
    };
    let target = if crosses(a, n, b, m) {
        pe
    } else if !first {
        pc
    } else {
        // Second test: a → m against b → n.
        let x15 = m.0 - a.0;
        let x8 = n.0 - b.0;
        let x5 = m.1 - a.1;
        let x2 = n.1 - b.1;
        let den = x15 * x2 - x5 * x8;
        let hit = den != 0 && {
            let u1 = (a.1 - b.1) * x8 - (a.0 - b.0) * x2;
            let u2 = (a.1 - b.1) * x15 - (a.0 - b.0) * x5;
            if den < 1 {
                u1 < 1 && den <= u1 && u2 < 1 && den <= u2
            } else {
                u1 >= 0 && u1 <= den && u2 >= 0 && u2 <= den
            }
        };
        if hit {
            pc
        } else {
            // `0x142196290` (distance comparison) not ported yet: keep the clone.
            pc
        }
    };
    if let Some(s) = g.nodes[node].slots.iter_mut().find(|s| **s == Some(target)) {
        *s = None;
    }
    let w = (g.edges[target].from != node) as usize;
    g.set_end(target, w, new);
    g.attach(new, target);
}

/// `0x1420bb0c0`: the split edge types and chiralities of an edge and its clone.
fn assign_types(g: &mut LayoutGraph, e: usize, c: usize, cur: usize, types: &dyn Fn(&str) -> SplitType) {
    let d = types(&g.edges[e].edge_type);
    let sym = |t: &SplitType| t.height_change == 0 && t.grounds[0] == t.grounds[1];
    let f0 = d.splits[0].1;
    let f1 = d.splits[1].1;
    // The client also asks `0x141d047b0` about the split type when both splits are the same;
    // assumed true here.
    let symmetric = sym(&d) && (!d.virtual_edge || (d.splits[0].0 == d.splits[1].0 && f0 != f1));
    let cur1 = cur != 0;
    if symmetric {
        g.edges[c].edge_type = d.splits[1].0.clone();
        let ct = types(&d.splits[1].0);
        g.edges[c].chirality = !(sym(&ct) && !ct.virtual_edge) && (f1 != cur1);
        g.edges[e].edge_type = d.splits[0].0.clone();
        let et = types(&d.splits[0].0);
        g.edges[e].chirality = !(sym(&et) && !et.virtual_edge) && (f0 != cur1);
        return;
    }
    let echir = g.edges[e].chirality;
    let bit;
    if (!echir) == cur1 {
        g.edges[c].edge_type = d.splits[0].0.clone();
        g.edges[c].chirality ^= f0;
        g.edges[e].edge_type = d.splits[1].0.clone();
        bit = f1;
    } else {
        g.edges[c].edge_type = d.splits[1].0.clone();
        g.edges[c].chirality ^= f1;
        g.edges[e].edge_type = d.splits[0].0.clone();
        bit = f0;
    }
    g.edges[e].chirality ^= bit;
}

/// `0x1420baa20`: moves the internal points of `e` and its clone apart, one random width per
/// point, walking away from end `cur`.
#[allow(clippy::too_many_arguments)]
fn split_points(
    g: &mut LayoutGraph,
    e: usize,
    c: usize,
    cur: usize,
    last: bool,
    end_point: Pos,
    size: Pos,
    rng: &mut TerrainRng,
) {
    let n = g.edges[e].points.len() as i64;
    let step: i64 = 1 - 2 * cur as i64;
    let mut i: i64 = (n - 1) * cur as i64 + 1;
    let stop: i64 = (n + 1) * (1 - cur as i64);
    let far_point = match g.edges[e].vends[1 - cur] {
        Some(v) => ((v[1].0 + v[0].0) / 2, (v[1].1 + v[0].1) / 2),
        None => end_point,
    };
    let (wmin, wmax) = g.edges[e].width;
    while i != stop {
        let prev = i - step;
        let (pe, pc) = if prev == 0 {
            (g.nodes[g.edges[e].from].pos, g.nodes[g.edges[c].from].pos)
        } else {
            (g.pt(e, prev), g.pt(c, prev))
        };
        let next = if !last || i != stop - step {
            mid(g.pt(e, i + step), g.pt(c, i + step))
        } else {
            far_point
        };
        let w = rng.below((wmax - wmin + 1) as u32) as i32 + wmin;
        let cur_pt = g.pt(e, i);
        let pm = mid(pe, pc);
        let [mut o1, mut o2] = miter(pm, cur_pt, next, w, size);
        if 0 < ((o2.0 + o1.0) / 2 - pm.0) * (o1.1 - pm.1) - ((o2.1 + o1.1) / 2 - pm.1) * (o1.0 - pm.0) {
            std::mem::swap(&mut o1, &mut o2);
        }
        g.edges[e].points[i as usize - 1] = o1;
        g.edges[c].points[i as usize - 1] = o2;
        i += step;
    }
}

/// `0x1420bb4f0`: splits the chain of virtual edges starting with `e0` at `node0`.
fn split_chain(
    g: &mut LayoutGraph,
    e0: usize,
    node0: usize,
    types: &dyn Fn(&str) -> SplitType,
    size: Pos,
    rng: &mut TerrainRng,
) {
    let (mut e, mut node) = (e0, node0);
    let mut prev: Option<(usize, usize)> = None;
    let mut first_join = true;
    loop {
        let cur = (g.edges[e].from != node) as usize;
        let far = 1 - cur;
        let far_node = g.endpoint(e, far);
        let next = g.nodes[far_node].slots.iter().flatten().copied().find(|&x| x != e);
        let mut clone: LEdge = g.edges[e].clone();
        let vend = match g.edges[e].vends[cur] {
            Some(v) => {
                g.edges[e].fixed = true;
                v
            }
            None => {
                let (src, p) = match prev {
                    Some((pe, pc)) => (pe, mid(g.next_to(pe, node), g.next_to(pc, node))),
                    None => {
                        // A chain starting without virtual end points (a loop): not met yet.
                        let other = g.nodes[node].slots.iter().flatten().copied().find(|&x| x != e);
                        let o = other.unwrap_or(e);
                        (o, g.next_to(o, node))
                    }
                };
                let (smin, smax) = g.edges[src].width;
                let (emin, emax) = g.edges[e].width;
                let span = (((smax + emax - smin - emin) as u32) >> 1) + 1;
                let w = rng.below(span) as i32 + (((smin + emin) as u32) >> 1) as i32;
                let q = g.next_to(e, node);
                let v = miter(p, g.nodes[node].pos, q, w, size);
                g.edges[e].vends[cur] = Some(v);
                v
            }
        };
        let new = g.add_node(vend[1]);
        g.nodes[new].fixed = g.nodes[node].fixed;
        g.nodes[new].virtual_end = g.nodes[node].virtual_end;
        if cur == 0 { clone.from = new } else { clone.to = new }
        clone.split = true;
        let ci = g.edges.len();
        g.edges.push(clone);
        let (cf, ct) = (g.edges[ci].from, g.edges[ci].to);
        g.attach(cf, ci);
        g.attach(ct, ci);
        g.nodes[node].pos = vend[0];
        g.edges[e].split = true;
        assign_types(g, e, ci, cur, types);
        if let Some((pe, pc)) = prev {
            join(g, pe, pc, node, new, first_join);
            first_join = false;
        }
        let next_split = next.is_some_and(|n| g.edges[n].split);
        if g.edges[e].vends[far].is_some() || next_split {
            g.edges[e].fixed = true;
            // Both far ends are next to a virtual end unless the far node is on the map border.
            let (x, y) = g.nodes[far_node].pos;
            let (tx, ty) = (x / 23, y / 23);
            let tiles = (size.0 / 23, size.1 / 23);
            let inside = !(tx == 0 || ty == 0 || tx == tiles.0 - 1 || ty == tiles.1 - 1);
            if let Some(v) = g.edges[e].vends[far] {
                g.nodes[far_node].pos = v[1];
                let nn = g.add_node(v[0]);
                // The client only looks at slots 1..3 when detaching the clone.
                for k in 1..4 {
                    if g.nodes[far_node].slots[k] == Some(ci) {
                        g.nodes[far_node].slots[k] = None;
                        break;
                    }
                }
                g.attach(nn, ci);
                if far == 0 { g.edges[ci].from = nn } else { g.edges[ci].to = nn }
            }
            let clone_far = g.endpoint(ci, far);
            g.nodes[far_node].virtual_end = inside;
            g.nodes[clone_far].virtual_end = inside;
            g.nodes[clone_far].fixed = g.nodes[far_node].fixed;
            split_points(g, e, ci, cur, true, (-1, -1), size, rng);
            return;
        }
        split_points(g, e, ci, cur, false, (-1, -1), size, rng);
        node = far_node;
        prev = Some((e, ci));
        match next {
            None => return,
            Some(n) => e = n,
        }
    }
}

/// `0x1420bc400`: splits every virtual edge (those with virtual end points first).
pub fn split_virtual(
    g: &mut LayoutGraph,
    types: &dyn Fn(&str) -> SplitType,
    size: Pos,
    rng: &mut TerrainRng,
) {
    let mut first = Vec::new();
    let mut second = Vec::new();
    for e in 0..g.edges.len() {
        if !g.edges[e].virtual_edge {
            continue;
        }
        match g.edges[e].vends {
            [Some(_), _] => first.push((e, 0)),
            [None, Some(_)] => first.push((e, 1)),
            _ => second.push((e, 0)),
        }
    }
    for (e, end) in first.into_iter().chain(second) {
        if !g.edges[e].split && !g.edges[e].fixed {
            let node = g.endpoint(e, end);
            split_chain(g, e, node, types, size, rng);
        }
    }
}
