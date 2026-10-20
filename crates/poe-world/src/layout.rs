//! Graph layout before tile placement (`0x1420bec40` and what it calls): node jitter inside
//! Voronoi cells (`0x141ef4720` → `0x141ef3bb0`). See `docs/re/terrain/tgr.md`,
//! "Graph layout".
//!
//! The sites are the graph's nodes and then its edges' internal points. Their Voronoi diagram
//! becomes a cell graph: a node per diagram vertex, an edge per diagram edge (both cells keep a
//! list of the edges around them), the map's four corners, and edges along the map border,
//! each given to the cell on its inner side. A node is then moved to a random point of its
//! cell: one of the triangles (node, cell edge) by area, a point in it with two `below(128)`
//! draws, scaled by 128/160.

use std::collections::{BTreeMap, HashMap};

use crate::{rng::TerrainRng, voronoi};

type Pos = (i32, i32);

#[derive(Debug, Clone, Default)]
pub struct CellGraph {
    pub nodes: Vec<Pos>,
    slots: Vec<[Option<usize>; 4]>,
    /// (from, to) nodes.
    pub edges: Vec<(usize, usize)>,
    /// Site position -> the edges around its cell, in the client's order.
    pub cells: HashMap<Pos, Vec<usize>>,
    /// Grid size in cells (tiles × 23).
    pub size: Pos,
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("Voronoi creation failed: no node at {0:?}")]
    Voronoi(Pos),
    #[error("too many edges for node at {0:?}")]
    TooManyEdges(Pos),
}

/// Site sets per node, ordered like the client's set (`x·65536 + y`).
type SiteSet = BTreeMap<i64, Pos>;

fn key(p: Pos) -> i64 {
    p.0 as i64 * 0x10000 + p.1 as i64
}

impl CellGraph {
    fn add_node(&mut self, p: Pos) -> usize {
        self.nodes.push(p);
        self.slots.push([None; 4]);
        self.nodes.len() - 1
    }

    fn find(&self, p: Pos) -> Option<usize> {
        // The client keeps the last node matching.
        self.nodes.iter().rposition(|&n| n == p)
    }

    /// `0x141f05890`: the node at `p`, else the nearest one (the first at the smallest
    /// squared distance). Clipped edge ends are recomputed from the line equation and can be
    /// a cell away from their vertex.
    fn nearest(&self, p: Pos) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None;
        for (i, &n) in self.nodes.iter().enumerate() {
            if n == p {
                best = Some((i, 0));
                continue;
            }
            let dx = (p.0 - n.0).wrapping_abs();
            let dy = (p.1 - n.1).wrapping_abs();
            let d = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32;
            if d < best.map_or(u32::MAX, |b| b.1) {
                best = Some((i, d));
            }
        }
        best.map(|b| b.0)
    }

    fn degree(&self, n: usize) -> usize {
        self.slots[n].iter().filter(|s| s.is_some()).count()
    }

    fn connected(&self, a: usize, b: usize) -> bool {
        self.slots[a]
            .iter()
            .flatten()
            .any(|&e| self.edges[e].0 == b || self.edges[e].1 == b)
    }

    /// `0x141f04c50`.
    fn connect(&mut self, a: usize, b: usize) -> Result<usize, LayoutError> {
        let e = self.edges.len();
        self.edges.push((a, b));
        for n in [a, b] {
            let slot = self.slots[n]
                .iter_mut()
                .find(|s| s.is_none())
                .ok_or(LayoutError::TooManyEdges(self.nodes[n]))?;
            *slot = Some(e);
        }
        Ok(e)
    }

    /// The far end of a node's first edge (edges here have no internal points).
    fn first_neighbour(&self, n: usize) -> Option<Pos> {
        let e = self.slots[n].iter().flatten().next()?;
        let (a, b) = self.edges[*e];
        Some(self.nodes[if a == n { b } else { a }])
    }

    /// Builds the cell graph of `sites` on a `w × h` cell grid (`0x141ef4720`).
    pub fn build(sites: &[Pos], w: i32, h: i32) -> Result<Self, LayoutError> {
        let mut g = Self {
            size: (w, h),
            ..Default::default()
        };
        let diagram = voronoi::voronoi(sites, [0, 0, w, h]);
        for &v in &diagram.vertices {
            if g.find(v).is_none() {
                g.add_node(v);
            }
        }
        let mut node_sites: HashMap<usize, SiteSet> = HashMap::new();
        for s in &diagram.segments {
            let a = g.nearest(s.p1).ok_or(LayoutError::Voronoi(s.p1))?;
            let b = g.nearest(s.p2).ok_or(LayoutError::Voronoi(s.p2))?;
            if a == b || g.connected(a, b) {
                continue;
            }
            let e = g.connect(a, b)?;
            let (sa, sb) = (sites[s.sites.0], sites[s.sites.1]);
            g.cells.entry(sa).or_default().push(e);
            g.cells.entry(sb).or_default().push(e);
            for n in [a, b] {
                let set = node_sites.entry(n).or_default();
                set.insert(key(sa), sa);
                set.insert(key(sb), sb);
            }
        }
        for c in [(0, 0), (0, h - 1), (w - 1, 0), (w - 1, h - 1)] {
            g.add_node(c);
        }
        // Border nodes (`0x141f09c90`): nodes with at most two edges on each side.
        let (mut x0, mut xmax, mut y0, mut ymax) = (vec![], vec![], vec![], vec![]);
        for n in 0..g.nodes.len() {
            if g.degree(n) > 2 {
                continue;
            }
            let (x, y) = g.nodes[n];
            if x >= w - 1 {
                xmax.push(n);
            } else if x == 0 {
                x0.push(n);
            }
            if y >= h - 1 {
                ymax.push(n);
            } else if y == 0 {
                y0.push(n);
            }
        }
        x0.sort_by_key(|&n| g.nodes[n].1);
        xmax.sort_by_key(|&n| g.nodes[n].1);
        y0.sort_by_key(|&n| g.nodes[n].0);
        ymax.sort_by_key(|&n| g.nodes[n].0);
        for (list, first, last) in [(&x0, -1, 1), (&xmax, 1, -1), (&y0, 1, -1), (&ymax, -1, 1)] {
            if list.len() >= 2 {
                g.inherit_site(&mut node_sites, list[0], list[1], first);
                let k = list.len();
                g.inherit_site(&mut node_sites, list[k - 1], list[k - 2], last);
            }
        }
        for (list, dir) in [(&x0, 1), (&xmax, -1), (&y0, -1), (&ymax, 1)] {
            g.chain_border(list, &node_sites, dir)?;
        }
        Ok(g)
    }

    /// `0x141ef4490`: a corner takes the site of its neighbour's cell on the `dir` side.
    fn inherit_site(&self, sets: &mut HashMap<usize, SiteSet>, corner: usize, next: usize, dir: i32) {
        let Some(set) = sets.get(&next) else { return };
        if self.degree(next) != 1 {
            return;
        }
        let p = self.first_neighbour(next).unwrap();
        let b = self.nodes[next];
        let found = set.values().copied().find(|s| {
            let cross = (s.1 - b.1)
                .wrapping_mul(p.0 - b.0)
                .wrapping_sub((s.0 - b.0).wrapping_mul(p.1 - b.1));
            (0 < cross) == (0 < dir)
        });
        if let Some(s) = found {
            sets.entry(corner).or_default().insert(key(s), s);
        }
    }

    /// `0x141ef3e10`: edges between consecutive border nodes, each added to one cell.
    fn chain_border(
        &mut self,
        list: &[usize],
        sets: &HashMap<usize, SiteSet>,
        dir: i32,
    ) -> Result<(), LayoutError> {
        if list.len() < 2 {
            return Ok(());
        }
        let e = self.connect(list[0], list[1])?;
        if list.len() == 2 {
            let (Some(a), Some(b)) = (sets.get(&list[0]), sets.get(&list[1])) else {
                return Ok(());
            };
            if let Some(s) = a.values().find(|s| b.contains_key(&key(**s))) {
                self.cells.entry(*s).or_default().push(e);
            }
            return Ok(());
        }
        // The first edge lies before `list[1]`, the others after `list[i]`: the client tests
        // the side from the neighbour for the first and from the border node for the rest.
        self.assign(list[1], e, sets, dir, false);
        for i in 1..list.len() - 1 {
            let e = self.connect(list[i], list[i + 1])?;
            self.assign(list[i], e, sets, dir, true);
        }
        Ok(())
    }

    fn assign(
        &mut self,
        n: usize,
        e: usize,
        sets: &HashMap<usize, SiteSet>,
        dir: i32,
        from_node: bool,
    ) {
        let Some(mut p) = self.first_neighbour(n) else { return };
        let mut l = self.nodes[n];
        if from_node {
            std::mem::swap(&mut p, &mut l);
        }
        let Some(set) = sets.get(&n) else { return };
        let found = set.values().copied().find(|s| {
            let cross = (s.1 - p.1)
                .wrapping_mul(l.0 - p.0)
                .wrapping_sub((s.0 - p.0).wrapping_mul(l.1 - p.1));
            (0 < cross) == (0 < dir)
        });
        if let Some(s) = found {
            self.cells.entry(s).or_default().push(e);
        }
    }

    /// Triangle weights of a site's cell (`0x141ef3390`): twice-area / 2 of (site, edge);
    /// a site in a border tile whose cell has an edge lying on that border takes that edge
    /// alone. `tiles` is the graph size in tiles.
    pub fn weights(&self, site: Pos, tiles: Pos) -> Vec<(u32, Pos, Pos)> {
        let Some(list) = self.cells.get(&site) else {
            return Vec::new();
        };
        let (w, h) = self.size;
        let mut out: Vec<(u32, Pos, Pos)> = list
            .iter()
            .map(|&e| {
                let (a, b) = self.edges[e];
                (0, self.nodes[a], self.nodes[b])
            })
            .collect();
        let tx = (site.0 as i64 * tiles.0 as i64 / w as i64) as i32;
        for i in 0..out.len() {
            let (_, p, q) = out[i];
            let cross = (q.1 - site.1)
                .wrapping_mul(p.0 - site.0)
                .wrapping_sub((p.1 - site.1).wrapping_mul(q.0 - site.0));
            out[i].0 = (cross.wrapping_abs() / 2) as u32;
            let border = if tx == 0 {
                p.0 == 0 && q.0 == 0
            } else {
                let ty = (site.1 as i64 * tiles.1 as i64 / h as i64) as i32;
                if ty == 0 {
                    p.1 == 0 && q.1 == 0
                } else if tx == tiles.0 - 1 {
                    p.0 == w - 1 && q.0 == p.0
                } else {
                    ty == tiles.1 - 1 && p.1 == h - 1 && q.1 == p.1
                }
            };
            if border {
                for o in out.iter_mut().take(i) {
                    o.0 = 0;
                }
                out[i].0 = 1;
                for o in out.iter_mut().skip(i + 1) {
                    o.0 = 0;
                }
                return out;
            }
        }
        out
    }

    /// The random offset for a site (`0x141ef3560`).
    pub fn offset(&self, site: Pos, tiles: Pos, rng: &mut TerrainRng) -> Pos {
        let tri = self.weights(site, tiles);
        if tri.is_empty() {
            return (0, 0);
        }
        let total: u32 = tri.iter().map(|t| t.0).sum();
        let mut r = rng.below(total);
        let mut i = 0;
        if tri[0].0 <= r {
            loop {
                r -= tri[i].0;
                i += 1;
                if i >= tri.len() || tri[i].0 > r {
                    break;
                }
            }
        }
        let (_, p, q) = tri[i.min(tri.len() - 1)];
        let mut a = rng.below(128) as i32;
        let mut b = rng.below(128) as i32;
        if a + b > 128 {
            a = 128 - a;
            b = 128 - b;
        }
        (
            ((q.0 - site.0) * b + (p.0 - site.0) * a) / 160,
            ((q.1 - site.1) * b + (p.1 - site.1) * a) / 160,
        )
    }
}

/// A node to jitter: its position and whether x / y are fixed (the `.tgr` node's x/y
/// constraint flags).
#[derive(Debug, Clone, Copy)]
pub struct JitterNode {
    pub pos: Pos,
    pub fix_x: bool,
    pub fix_y: bool,
}

/// Moves graph nodes and then edge internal points inside their cells (`0x141ef3bb0`).
/// `points` are the internal points of every edge, in edge order. Returns nothing; positions
/// are updated in place.
pub fn jitter(
    nodes: &mut [JitterNode],
    points: &mut [Pos],
    tiles: Pos,
    rng: &mut TerrainRng,
) -> Result<CellGraph, LayoutError> {
    let sites: Vec<Pos> = nodes
        .iter()
        .map(|n| n.pos)
        .chain(points.iter().copied())
        .collect();
    let g = CellGraph::build(&sites, tiles.0 * 23, tiles.1 * 23)?;
    for n in nodes.iter_mut() {
        let (dx, dy) = g.offset(n.pos, tiles, rng);
        if !n.fix_x {
            n.pos.0 += dx;
        }
        if !n.fix_y {
            n.pos.1 += dy;
        }
    }
    for p in points.iter_mut() {
        if g.cells.contains_key(p) {
            let (dx, dy) = g.offset(*p, tiles, rng);
            p.0 += dx;
            p.1 += dy;
        }
    }
    Ok(g)
}
