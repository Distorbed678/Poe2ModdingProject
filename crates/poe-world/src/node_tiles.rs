//! Node tiles (`0x1420cade0`): after the virtual split and the first rounding, every graph node
//! without a room that `0x1420c7400` sorted into the tile list and that is not a dead end at the
//! map border gets a tile from the tile set: a dead end inside the map, a junction of three or
//! more edges, or two edges of different `.et` types (a transition tile such as
//! `NarrowRiver_Mud/StMS_01.tdt`). Like a room node (`0x1420c87a0`): the node's key, candidates
//! by edges and size, a weighted draw, centred position; then the tiles push the graph away
//! (`0x1420cbe60` without the room flag), are placed (`0x1420c9970`), and each edge of the node
//! ends at its exit, the node split per exit (`0x1420c5b50`, `0x141ef2800`).

use std::collections::BTreeSet;

use crate::{
    generate::{Files, GenError, Master, TileMap},
    graph::Graph,
    rng::TerrainRng,
    rooms::{EdgeView, RepelRect, TerrainTypes, angle_between, fits_edges, node_key, node_orientation, repel_push},
    routing::{EXIT_CELL, SIDE_DIR},
    smoothing::{LNode, LayoutGraph},
    tileset::{TileKey, TileSet},
};

/// The layout graph as an [`EdgeView`] (heights from the `.tgr` nodes; split-off nodes 0).
pub struct LayoutView<'a> {
    pub lg: &'a LayoutGraph,
    pub graph: &'a Graph,
}

impl LayoutView<'_> {
    /// The `.tgr` node a layout node is.
    pub fn gnode(&self, n: usize) -> Option<&crate::graph::Node> {
        self.lg.nodes.get(n).and_then(|x| x.orig).and_then(|o| self.graph.nodes.get(o))
    }
}

impl EdgeView for LayoutView<'_> {
    fn slots(&self, node: usize) -> [Option<usize>; 4] {
        self.lg.nodes[node].slots
    }
    fn pos(&self, node: usize) -> (i32, i32) {
        self.lg.nodes[node].pos
    }
    fn towards(&self, node: usize, e: usize) -> (i32, i32) {
        let ed = &self.lg.edges[e];
        if ed.from == node {
            ed.points.first().copied().unwrap_or(self.lg.nodes[ed.to].pos)
        } else {
            ed.points.last().copied().unwrap_or(self.lg.nodes[ed.from].pos)
        }
    }
    fn edge_type(&self, e: usize) -> &str {
        &self.lg.edges[e].edge_type
    }
    fn starts(&self, node: usize, e: usize) -> bool {
        self.lg.edges[e].from == node
    }
    fn chirality(&self, e: usize) -> bool {
        self.lg.edges[e].chirality
    }
    fn other(&self, node: usize, e: usize) -> usize {
        let ed = &self.lg.edges[e];
        if ed.from == node { ed.to } else { ed.from }
    }
    fn height(&self, node: usize) -> i32 {
        self.gnode(node).and_then(|n| n.values.get(1).copied()).unwrap_or(0)
    }
}

/// `0x1420c7400`: whether a room-less node goes to the tile list (dead ends, junctions of three
/// or more edges, two edges of different types; nodes at a virtual edge only as dead ends at
/// the border, which `0x1420c75c0` handles).
pub fn needs_tile(lg: &LayoutGraph, n: usize, near_border: bool, types: &mut TerrainTypes, files: &dyn Files) -> Result<bool, GenError> {
    let edges: Vec<usize> = lg.nodes[n].slots.iter().flatten().copied().collect();
    let mut virtual_end = false;
    for &e in &edges {
        virtual_end |= types.edge(files, &lg.edges[e].edge_type)?.virtual_edge;
    }
    Ok(match edges.len() {
        0 => false,
        1 => !virtual_end || near_border,
        2 if lg.edges[edges[0]].edge_type == lg.edges[edges[1]].edge_type => false,
        _ => !virtual_end,
    })
}

#[derive(Debug, Clone)]
pub struct NodeTile {
    pub node: usize,
    /// Index in the tile set.
    pub tile: usize,
    pub orientation: u8,
    /// Corner (tiles) and size with the orientation.
    pub pos: (i32, i32),
    pub size: (u8, u8),
    /// Side per node slot (4 = none) and the tile's turned key.
    pub sides: [usize; 4],
    pub key: TileKey,
    pub align: i32,
}

/// A tile-set candidate for a node: tile index, orientation, weight, turned size.
#[derive(Debug, Clone)]
pub struct Entry {
    pub tile: usize,
    pub orientation: u8,
    pub weight: u32,
    pub size: (u8, u8),
}

/// `0x141d09700` non-empty: the tile set has tiles of `feature` with these edges (in some
/// orientation).
pub fn feature_has_edges(set: &TileSet, feature: &str, edges: &[Option<String>; 4]) -> bool {
    set.tiles.iter().any(|t| {
        t.weight != 0
            && t.tdt.feature.as_deref().unwrap_or("").eq_ignore_ascii_case(feature)
            && (0..8u8).any(|o| t.orientations & (1 << o) != 0 && t.key.oriented(o).edges == *edges)
    })
}

/// `0x1420c76d0`, tile branch: the sizes the feature's tiles with the key's edges come in
/// (`0x141d09700`, sorted), and per size every tile entry with those edges and that size
/// (`0x141d0a7d0`; the size swapped when there is none) of that feature, filtered by the node's
/// orientation, or for `(any)` by `0x1420b87b0` (with the climb check, then without).
#[allow(clippy::too_many_arguments)]
pub fn candidates(view: &dyn EdgeView, node: usize, node_o: u8, feature: &str, key: &TileKey, sides: &[usize; 4], set: &TileSet, types: &mut TerrainTypes, files: &dyn Files) -> Result<Vec<Entry>, GenError> {
    let mut all = Vec::new();
    for (i, t) in set.tiles.iter().enumerate() {
        if t.weight == 0 {
            continue;
        }
        let base = t.key.clone();
        for o in 0..8u8 {
            if t.orientations & (1 << o) == 0 {
                continue;
            }
            let k = base.oriented(o);
            // The filter lambdas (`0x143505830`/`0x143505a10`) keep the node's feature only.
            if k.edges == key.edges && t.tdt.feature.as_deref().unwrap_or("").eq_ignore_ascii_case(feature) {
                all.push((i, o, t.weight, k, true));
            }
        }
    }
    let sizes: BTreeSet<(u8, u8)> = all.iter().filter(|a| a.4).map(|a| a.3.size).collect();
    let mut out = Vec::new();
    for strict in [true, false] {
        for &(a, b) in &sizes {
            let mut group: Vec<_> = all.iter().filter(|x| x.3.size == (a, b)).collect();
            if group.is_empty() && a != b {
                group = all.iter().filter(|x| x.3.size == (b, a)).collect();
            }
            for (tile, o, w, k, _) in group {
                let ok = if node_o < 8 { *o == node_o } else { fits_edges(k, sides, view, node, strict, types, files)? };
                if ok {
                    out.push(Entry { tile: *tile, orientation: *o, weight: *w, size: k.size });
                }
            }
        }
        if !out.is_empty() || node_o < 8 {
            break;
        }
    }
    Ok(out)
}

/// `0x141d00f00` for a quarter turn: orientation `o` turned once more (`r` 1) or back (`r` 3).
pub(crate) fn turn(o: u8, r: u8) -> u8 {
    const ONE: [u8; 8] = [1, 2, 3, 0, 5, 6, 7, 4];
    const THREE: [u8; 8] = [3, 0, 1, 2, 7, 4, 5, 6];
    if o >= 8 {
        return o;
    }
    if r == 1 { ONE[o as usize] } else { THREE[o as usize] }
}

/// `0x1420b8e20`: how far the node's edges point away from their sides (degrees).
pub(crate) fn side_cost(view: &dyn EdgeView, node: usize, sides: &[usize; 4]) -> i32 {
    let slots = view.slots(node);
    let (nx, ny) = view.pos(node);
    let mut cost = 0;
    for i in 0..4 {
        let (Some(e), s) = (slots[i], sides[i]) else { continue };
        if s >= 4 {
            continue;
        }
        let (px, py) = view.towards(node, e);
        cost += angle_between(SIDE_DIR[s], (px - nx, py - ny)) as i32 / 0xb6;
    }
    cost
}

/// The node's alignment (`0x141ef3290`): an `align<N>` tag, else its largest edge size.
fn alignment(view: &LayoutView, node: usize, types: &mut TerrainTypes, files: &dyn Files) -> Result<i32, GenError> {
    if let Some(n) = view.gnode(node) {
        for t in &n.tags {
            if let Some(v) = t.strip_prefix("align").and_then(|v| v.parse::<i32>().ok()) {
                if v != 0 {
                    return Ok(v);
                }
            }
        }
    }
    let mut a = 0;
    for e in view.slots(node).into_iter().flatten() {
        a = a.max(types.edge(files, view.edge_type(e))?.size as i32);
    }
    Ok(a)
}

/// `0x1420c59e0`: keep a tile `margin` tiles (twice for virtual edges) off the map border on
/// each side its key has an edge.
fn keep_off_border(pos: &mut (i32, i32), size: (i32, i32), edges: &[Option<String>; 4], margin: i32, tiles: (i32, i32), types: &mut TerrainTypes, files: &dyn Files) -> Result<(), GenError> {
    let (w, h) = size;
    let mut m = |side: usize| -> Result<Option<i32>, GenError> {
        Ok(match &edges[side] {
            None => None,
            Some(p) => Some((types.edge(files, p)?.virtual_edge as i32 + 1) * margin),
        })
    };
    if let Some(m) = m(0)? {
        pos.1 = pos.1.max(m).min(tiles.1 - h);
    }
    if let Some(m) = m(3)? {
        pos.0 = pos.0.max(m).min(tiles.0 - w);
    }
    if let Some(m) = m(2)? {
        pos.1 = pos.1.min(tiles.1 - m - h).max(0);
    }
    if let Some(m) = m(1)? {
        pos.0 = pos.0.min(tiles.0 - m - w).max(0);
    }
    Ok(())
}

/// `0x1420c87a0` for a room-less node: draws its tile and position.
#[allow(clippy::too_many_arguments)]
fn choose(view: &LayoutView, node: usize, master: &Master, map: &TileMap, chosen: &[NodeTile], tiles: (i32, i32), types: &mut TerrainTypes, files: &dyn Files, rng: &mut TerrainRng) -> Result<NodeTile, GenError> {
    let (mut sides, key) = node_key(view, node, types, files)?;
    let node_o = view.gnode(node).map_or(8, |n| node_orientation(&n.orientation));
    let mut list = candidates(view, node, node_o, "", &key, &sides, &master.tiles, types, files)?;
    if list.is_empty() {
        return Err(GenError::Unsupported(format!("node {node}: no tile matches its edges {:?}", key.edges)));
    }
    let align = alignment(view, node, types, files)?;
    let mut max_size = 0;
    for e in view.slots(node).into_iter().flatten() {
        max_size = max_size.max(types.edge(files, view.edge_type(e))?.size as i32);
    }
    let (x, y) = view.pos(node);
    let cx = (x / 23).min(tiles.0 - 1);
    let cy = (y / 23).min(tiles.1 - 1);
    loop {
        let total: u32 = list.iter().map(|c| c.weight).sum();
        let mut r = rng.below(total);
        if list.is_empty() {
            return Err(GenError::Unsupported(format!("node {node}: no tile fits the map")));
        }
        let mut i = 0;
        while i + 1 < list.len() && r >= list[i].weight {
            r -= list[i].weight;
            i += 1;
        }
        let c = list[i].clone();
        let (w, h) = (c.size.0 as i32, c.size.1 as i32);
        let ro = view.graph.region;
        let c0 = (crate::region::centred(ro, cx, w, true).max(0), crate::region::centred(ro, cy, h, false).max(0));
        let (sx, sy) = crate::region::snap(ro, c0, align);
        let px = sx.max(0).min(tiles.0 - w);
        let py = sy.max(0).min(tiles.1 - h);
        let inside = px >= 0 && py >= 0 && px + w <= tiles.0 && py + h <= tiles.1;
        let overlaps = chosen.iter().any(|p| px < p.pos.0 + p.size.0 as i32 && p.pos.0 < px + w && py < p.pos.1 + p.size.1 as i32 && p.pos.1 < py + h);
        if inside && map.free(px, py, w, h) && !overlaps {
            let mut key = TileKey::from_tdt(&master.tiles.tiles[c.tile].tdt).oriented(c.orientation);
            let mut pos = (px, py);
            keep_off_border(&mut pos, (w, h), &key.edges.clone(), max_size, tiles, types, files)?;
            let mut orientation = c.orientation;
            if node_o >= 8 {
                if let Some(d) = quarter_turn(view, node, &sides, pos, c.size, tiles) {
                    sides = sides.map(|s| if s == 4 { 4 } else { (s + d as usize) & 3 });
                    orientation = turn(orientation, d);
                    key = key.oriented(d);
                }
            }
            return Ok(NodeTile { node, tile: c.tile, orientation, pos, size: key.size, sides, key, align });
        }
        // `0x1420c87a0`: the first entry of that tile and orientation goes.
        if let Some(k) = list.iter().position(|o| o.tile == c.tile && o.orientation == c.orientation) {
            list.remove(k);
        }
    }
}

/// `0x1420c9d70`: a quarter turn (1 or 3) when that points the node's edges better at their
/// sides and puts none against the map border the placement touches.
pub(crate) fn quarter_turn(view: &dyn EdgeView, node: usize, sides: &[usize; 4], pos: (i32, i32), size: (u8, u8), tiles: (i32, i32)) -> Option<u8> {
    let turned = |d: usize| sides.map(|s| if s == 4 { 4 } else { (s + d) & 3 });
    let allowed = |sd: &[usize; 4]| {
        !((pos.0 == 0 && sd.contains(&3))
            || (pos.1 == 0 && sd.contains(&0))
            || (size.0 as i32 + pos.0 == tiles.0 && sd.contains(&1))
            || (size.1 as i32 + pos.1 == tiles.1 && sd.contains(&2)))
    };
    let (minus, plus) = (turned(3), turned(1));
    let cost = side_cost(view, node, sides);
    let cost_minus = side_cost(view, node, &minus);
    if allowed(&minus) && cost_minus < cost {
        if allowed(&plus) && side_cost(view, node, &plus) < cost_minus { Some(1) } else { Some(3) }
    } else if allowed(&plus) && side_cost(view, node, &plus) < cost {
        Some(1)
    } else {
        None
    }
}

/// Fixed axes of a node (`+0x8b`/`+0x8c`, the `.tgr` node values 9 and 10).
fn fixed_axes(graph: &Graph, lg: &LayoutGraph, n: usize) -> (bool, bool) {
    lg.nodes[n].orig.and_then(|o| graph.nodes.get(o)).map_or((false, false), |n| (n.values.get(9).copied().unwrap_or(0) != 0, n.values.get(10).copied().unwrap_or(0) != 0))
}

/// Places the tiles of `nodes` (in order) and rewires the layout graph to their exits.
#[allow(clippy::too_many_arguments)]
pub fn place_node_tiles(lg: &mut LayoutGraph, graph: &Graph, nodes: &[usize], master: &Master, map: &mut TileMap, tiles: (i32, i32), types: &mut TerrainTypes, files: &dyn Files, rng: &mut TerrainRng) -> Result<Vec<NodeTile>, GenError> {
    let mut chosen: Vec<NodeTile> = Vec::new();
    for &n in nodes {
        let view = LayoutView { lg, graph };
        let t = choose(&view, n, master, map, &chosen, tiles, types, files, rng)?;
        chosen.push(t);
    }
    if chosen.is_empty() {
        return Ok(chosen);
    }
    repel(lg, graph, &mut chosen, map, tiles, types, files)?;
    for t in &chosen {
        place(lg, graph.region, t, master, map, types, files)?;
    }
    Ok(chosen)
}

/// `0x1420cbe60` with the room flag clear: the tiles' rectangles push every node with other
/// than one edge (a tile's own node not at all) and every internal point; then each tile is
/// centred on its node again where it fits.
fn repel(lg: &mut LayoutGraph, graph: &Graph, chosen: &mut [NodeTile], map: &TileMap, tiles: (i32, i32), types: &mut TerrainTypes, files: &dyn Files) -> Result<(), GenError> {
    let (wc, hc) = (tiles.0 * 23, tiles.1 * 23);
    let mut rects = Vec::new();
    for t in chosen.iter() {
        let mut at = [4usize; 4];
        for (slot, &side) in t.sides.iter().enumerate() {
            if side < 4 {
                at[side] = slot;
            }
        }
        let slots = lg.nodes[t.node].slots;
        let mut margin = |side: usize| -> Result<i32, GenError> {
            let Some(e) = (at[side] < 4).then(|| slots[at[side]]).flatten() else { return Ok(0) };
            let et = types.edge(files, &lg.edges[e].edge_type)?;
            Ok(et.size as i32 * (et.virtual_edge as i32 + 1))
        };
        rects.push(RepelRect {
            x0: (t.pos.0 - margin(3)?) * 23,
            y0: (t.pos.1 - margin(0)?) * 23,
            x1: (t.size.0 as i32 + t.pos.0 + margin(1)?) * 23,
            y1: (t.size.1 as i32 + t.pos.1 + margin(2)?) * 23,
            node: t.node,
        });
    }
    let mut node_push = Vec::with_capacity(lg.nodes.len());
    for n in 0..lg.nodes.len() {
        if lg.nodes[n].slots.iter().flatten().count() == 1 {
            node_push.push(None);
            continue;
        }
        let mut acc = (0, 0);
        for r in &rects {
            if r.node != n {
                let v = repel_push(r, lg.nodes[r.node].pos, lg.nodes[n].pos);
                acc = (acc.0 + v.0, acc.1 + v.1);
            }
        }
        node_push.push(Some(acc));
    }
    let mut point_push = Vec::new();
    for e in &lg.edges {
        for &p in &e.points {
            let mut acc = (0, 0);
            for r in &rects {
                let v = repel_push(r, lg.nodes[r.node].pos, p);
                acc = (acc.0 + v.0, acc.1 + v.1);
            }
            point_push.push(acc);
        }
    }
    for (n, push) in node_push.into_iter().enumerate() {
        let Some(push) = push else { continue };
        let (x, y) = lg.nodes[n].pos;
        let inside = x >= 0 && y >= 0 && x < wc && y < hc;
        let (fx, fy) = fixed_axes(graph, lg, n);
        let mut q = (x, y);
        if !fx {
            q.0 += push.0;
        }
        if !fy {
            q.1 += push.1;
        }
        lg.nodes[n].pos = if inside { (q.0.clamp(0, wc - 1), q.1.clamp(0, hc - 1)) } else { (q.0.max(-1).min(wc), q.1.max(-1).min(hc)) };
    }
    let mut k = 0;
    for e in lg.edges.iter_mut() {
        for p in e.points.iter_mut() {
            let v = point_push[k];
            k += 1;
            *p = ((p.0 + v.0).clamp(0, wc - 1), (p.1 + v.1).clamp(0, hc - 1));
        }
    }
    for t in chosen.iter_mut() {
        let (nx, ny) = lg.nodes[t.node].pos;
        let (w, h) = (t.size.0 as i32, t.size.1 as i32);
        let cx = (nx / 23).min(tiles.0 - 1);
        let cy = (ny / 23).min(tiles.1 - 1);
        let ro = graph.region;
        let x = crate::region::centred(ro, cx, w, true).max(0).min(tiles.0 - w);
        let y = crate::region::centred(ro, cy, h, false).max(0).min(tiles.1 - h);
        let (sx, sy) = crate::region::snap(ro, (x, y), t.align);
        if sx < tiles.0 && sx + w <= tiles.0 && sy < tiles.1 && sy + h <= tiles.1 && map.free(sx, sy, w, h) {
            t.pos = (sx, sy);
        }
        // The node's own non-wildcard edges by side.
        let mut edges: [Option<String>; 4] = Default::default();
        for (slot, &side) in t.sides.iter().enumerate() {
            let Some(e) = lg.nodes[t.node].slots[slot] else { continue };
            if side < 4 && !types.edge(files, &lg.edges[e].edge_type)?.wildcard {
                edges[side] = Some(lg.edges[e].edge_type.clone());
            }
        }
        let mut pos = t.pos;
        keep_off_border(&mut pos, (w, h), &edges, t.align, tiles, types, files)?;
        t.pos = pos;
    }
    Ok(())
}

/// `0x1420c9970`: the tile into the map, the node's edges cut at the tile and ended at its
/// exits (`0x1420c5750`, `0x1420c5b50`), the node split per exit (`0x141ef2800`).
fn place(lg: &mut LayoutGraph, region: u8, t: &NodeTile, master: &Master, map: &mut TileMap, types: &mut TerrainTypes, files: &dyn Files) -> Result<(), GenError> {
    let pos = crate::region::snap(region, t.pos, t.align);
    let def = &master.tiles.tiles[t.tile];
    map.place(pos.0, pos.1, &def.path, &def.tdt, t.orientation, 0, Some(t.tile));
    let (rx, ry) = (pos.0 * 23, pos.1 * 23);
    let (rw, rh) = (t.size.0 as i32 * 23, t.size.1 as i32 * 23);
    let inside = |q: &(i32, i32)| q.0 >= rx && q.0 < rx + rw && q.1 >= ry && q.1 < ry + rh;
    let node = t.node;
    let slots = lg.nodes[node].slots;
    let mut first = true;
    for (slot, s) in slots.iter().enumerate() {
        let Some(e) = *s else { continue };
        let from = lg.edges[e].from == node;
        let pts = &mut lg.edges[e].points;
        if from {
            let k = pts.iter().position(|q| !inside(q)).unwrap_or(pts.len());
            pts.drain(..k);
        } else if let Some(k) = pts.iter().position(|q| inside(q)) {
            pts.truncate(k);
        }
        let side = t.sides[slot];
        if side > 3 {
            continue;
        }
        if types.edge(files, &lg.edges[e].edge_type)?.virtual_edge {
            return Err(GenError::Unsupported(format!("node tile {node}: virtual edge {e}")));
        }
        let ex = t.key.exits[side];
        let o = EXIT_CELL[(ex % 3) as usize] + ex as i32 / 3 * 23;
        let (x, y) = match side {
            0 => (o, 0),
            1 => (rw - 1, o),
            2 => (rw - o - 1, rh - 1),
            _ => (0, rh - o - 1),
        };
        let end = (rx + x, ry + y);
        let which = (!from) as usize;
        lg.edges[e].offsets[which] = SIDE_DIR[side];
        if first {
            lg.nodes[node].pos = end;
            lg.nodes[node].fixed = true;
            first = false;
        } else {
            let nn = lg.nodes.len();
            lg.nodes.push(LNode { pos: end, slots: [Some(e), None, None, None], fixed: true, virtual_end: false, orig: None });
            if which == 0 { lg.edges[e].from = nn } else { lg.edges[e].to = nn }
            lg.nodes[node].slots[slot] = None;
        }
    }
    Ok(())
}

/// `0x1420cb200`: before routing, a junction (or a virtual node no tile fits) gets one point
/// 23 cells out of the node on each edge's side (`0x1420d1220`), after its edges drop the
/// points within 23 cells of it (`0x1420c6b80`).
pub fn explode(g: &mut crate::rooms::PlacedGraph, node: usize, tiles: (i32, i32), types: &mut TerrainTypes, files: &dyn Files) -> Result<(), GenError> {
    const R: i32 = 23;
    let (nx, ny) = g.nodes[node];
    let near = |p: &(i32, i32)| {
        let (dx, dy) = ((p.0 - nx).abs(), (p.1 - ny).abs());
        ((dx * dx + dy * dy) as u32) < (R * R) as u32
    };
    let slots = EdgeView::slots(&*g, node);
    for e in slots.into_iter().flatten() {
        let from = g.graph.edges[e].from as usize == node;
        let pts = &mut g.points[e];
        if from {
            if let Some(i) = pts.iter().rposition(|p| near(p)) {
                pts.drain(..=i);
            }
        } else if let Some(i) = pts.iter().position(|p| near(p)) {
            pts.truncate(i);
        }
    }
    let (sides, _) = node_key(&*g, node, types, files)?;
    for (slot, e) in slots.iter().enumerate() {
        let Some(e) = *e else { continue };
        let side = sides[slot];
        if side > 3 {
            return Err(GenError::Unsupported(format!("could not explode node {node}: edge {e} has no side")));
        }
        let d = SIDE_DIR[side];
        let p = ((nx + d.0 * R).clamp(0, tiles.0 * 23 - 1), (ny + d.1 * R).clamp(0, tiles.1 * 23 - 1));
        if g.graph.edges[e].from as usize == node {
            g.points[e].insert(0, p);
        } else {
            g.points[e].push(p);
        }
    }
    Ok(())
}

/// `0x1420c6cd0`, after smoothing: a virtual node no tile fitted. With three or more edges,
/// its edges first drop the points within the widest edge's maximum width of it
/// (`0x1420c6b80`); then, for each pair of neighbouring slots, a virtual edge next to a
/// non-virtual one gets a virtual end point a random width (`below(max − min + 1) + min` of the
/// virtual edge) from the node towards the other edge (`0x1420c6420`), filled into its end two
/// points at a time and ordered (`0x1420c6860`). Last (any edge count ≥ 2), the node splits:
/// each edge after the first moves to a new node at the same place (`0x1420c5ee0`).
pub fn virtual_junction(lg: &mut LayoutGraph, n: usize, rng: &mut TerrainRng) -> Result<(), GenError> {
    let slots = lg.nodes[n].slots;
    let live: Vec<usize> = (0..4).filter(|&k| slots[k].is_some()).collect();
    if live.len() < 2 {
        return Err(GenError::Unsupported(format!("tile-less virtual node {n} with {} edges", live.len())));
    }
    let node = lg.nodes[n].pos;
    if live.len() > 2 {
        let r = live.iter().map(|&k| lg.edges[slots[k].unwrap()].width.1).max().unwrap_or(0);
        for &k in &live {
            let e = slots[k].unwrap();
            let from = lg.edges[e].from == n;
            let near = |p: &(i32, i32)| {
                let (dx, dy) = ((p.0 - node.0).abs(), (p.1 - node.1).abs());
                ((dx * dx + dy * dy) as u32) < (r * r) as u32
            };
            let pts = &mut lg.edges[e].points;
            if from {
                if let Some(i) = pts.iter().rposition(|p| near(p)) {
                    pts.drain(..=i);
                }
            } else if let Some(i) = pts.iter().position(|p| near(p)) {
                pts.truncate(i);
            }
        }
        // The point next to the node on an edge, and the edge's end at the node.
        let towards = |lg: &LayoutGraph, e: usize| -> (i32, i32) {
            let ed = &lg.edges[e];
            if ed.from == n {
                ed.points.first().copied().unwrap_or(lg.nodes[ed.to].pos)
            } else {
                ed.points.last().copied().unwrap_or(lg.nodes[ed.from].pos)
            }
        };
        let toward = |p: (i32, i32), w: i32| -> (i32, i32) {
            let (dx, dy) = (p.0 - node.0, p.1 - node.1);
            let d2 = dy * dy + dx * dx;
            if ((w * w * 100) as u32) < ((d2 * 0x51) as u32) {
                let d = crate::voronoi::isqrt(d2 as u32) as i32;
                (dx * w / d + node.0, dy * w / d + node.1)
            } else {
                (dx * 9 / 10 + node.0, dy * 9 / 10 + node.1)
            }
        };
        let mut pending: Vec<(usize, usize, (i32, i32))> = Vec::new();
        for i in 0..live.len() {
            let (a, b) = (slots[live[i]].unwrap(), slots[live[(i + 1) % live.len()]].unwrap());
            let (va, vb) = (lg.edges[a].virtual_edge, lg.edges[b].virtual_edge);
            match (va, vb) {
                (false, false) => {}
                (false, true) => {
                    let (lo, hi) = lg.edges[b].width;
                    let w = rng.below((hi - lo + 1) as u32) as i32 + lo;
                    let v = toward(towards(lg, a), w);
                    pending.push((b, (lg.edges[b].from != n) as usize, v));
                }
                (true, false) => {
                    let (lo, hi) = lg.edges[a].width;
                    let w = rng.below((hi - lo + 1) as u32) as i32 + lo;
                    let v = toward(towards(lg, b), w);
                    pending.push((a, (lg.edges[a].from != n) as usize, v));
                }
                (true, true) => {
                    return Err(GenError::Unsupported(format!("tile-less virtual node {n}: two virtual edges side by side (0x1420c64f0) not ported")));
                }
            }
        }
        // `0x1420c6860`: two points per end, ordered by the side of the edge they fall on.
        let mut first: std::collections::HashMap<(usize, usize), (i32, i32)> = Default::default();
        for (e, end, v) in pending {
            match first.remove(&(e, end)) {
                None => {
                    first.insert((e, end), v);
                }
                Some(mut p) => {
                    let mut q2 = v;
                    let ed = &lg.edges[e];
                    let m = lg.nodes[if end == 0 { ed.from } else { ed.to }].pos;
                    let q = if ed.from == if end == 0 { ed.from } else { ed.to } {
                        ed.points.first().copied().unwrap_or(lg.nodes[ed.to].pos)
                    } else {
                        ed.points.last().copied().unwrap_or(lg.nodes[ed.from].pos)
                    };
                    let mut c = (p.1 - q.1) * (m.0 - q.0) - (p.0 - q.0) * (m.1 - q.1);
                    let c2 = (m.0 - q.0) * (q2.1 - q.1) - (m.1 - q.1) * (q2.0 - q.0);
                    if (c2 < 0) == (c < 0) {
                        c = ((p.0 + q2.0) / 2 - m.0) * (p.1 - m.1) - (p.0 - m.0) * ((p.1 + q2.1) / 2 - m.1);
                    }
                    if c < 0 {
                        std::mem::swap(&mut p, &mut q2);
                    }
                    lg.edges[e].vends[end] = Some([p, q2]);
                }
            }
        }
        if !first.is_empty() {
            return Err(GenError::Unsupported(format!("tile-less virtual node {n}: an edge end got one virtual point")));
        }
    }
    // `0x1420c5ee0`: one node per edge; neighbouring edges with a virtual one between them
    // stay logically linked (their end nodes).
    let ends: Vec<usize> = live.iter().map(|&k| slots[k].unwrap()).collect();
    let mut end_nodes = vec![n];
    for &k in live.iter().skip(1) {
        let e = slots[k].unwrap();
        let nn = lg.nodes.len();
        lg.nodes.push(LNode { pos: node, slots: [Some(e), None, None, None], fixed: false, virtual_end: false, orig: None });
        if lg.edges[e].from == n { lg.edges[e].from = nn } else { lg.edges[e].to = nn }
        lg.nodes[n].slots[k] = None;
        end_nodes.push(nn);
    }
    for i in 0..ends.len() {
        let j = (i + 1) % ends.len();
        if lg.edges[ends[i]].virtual_edge || lg.edges[ends[j]].virtual_edge {
            let (a, b) = (end_nodes[i], end_nodes[j]);
            let l = lg.links.len();
            lg.links.push([Some(a), Some(b), None, None]);
            for x in [a, b] {
                let v = lg.node_links.entry(x).or_default();
                if v.len() >= 2 {
                    return Err(GenError::Unsupported(format!("too many logical links for node {x}")));
                }
                v.push(l);
            }
        }
    }
    Ok(())
}

/// `0x142196290`: squared distance of `p` from the segment `a`–`b` as a fraction
/// `(numerator, denominator)`.
fn seg_dist(p: (i32, i32), a: (i32, i32), b: (i32, i32)) -> (i64, i64) {
    let sq = |x: i64, y: i64| x * x + y * y;
    let (px, py, ax, ay, bx, by) = (p.0 as i64, p.1 as i64, a.0 as i64, a.1 as i64, b.0 as i64, b.1 as i64);
    if ax != bx {
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let t = (px - ax) * dx + (py - ay) * dy;
        if t < 1 {
            (sq(px - ax, py - ay), 1)
        } else if t < len2 {
            let c = dy * (px - ax) - dx * (py - ay);
            (c * c, len2)
        } else {
            (sq(px - bx, py - by), 1)
        }
    } else {
        let (lo, hi) = if by < ay { ((bx, by), (ax, ay)) } else { ((ax, ay), (bx, by)) };
        if py < lo.1 {
            (sq(px - lo.0, py - lo.1), 1)
        } else if py < hi.1 {
            ((px - lo.0) * (px - lo.0), 1)
        } else {
            (sq(px - hi.0, py - hi.1), 1)
        }
    }
}

/// Live edges of a layout node, in slot order.
fn degree(lg: &LayoutGraph, n: usize) -> usize {
    lg.nodes[n].slots.iter().flatten().count()
}

/// Moves edge `e`'s end at `from` to `to` (`from`'s slot cleared, `to`'s first free slot).
fn move_end(lg: &mut LayoutGraph, e: usize, from: usize, to: usize) {
    if lg.edges[e].from == from { lg.edges[e].from = to } else { lg.edges[e].to = to }
    if let Some(s) = lg.nodes[from].slots.iter_mut().find(|s| **s == Some(e)) {
        *s = None;
    }
    if let Some(s) = lg.nodes[to].slots.iter_mut().find(|s| s.is_none()) {
        *s = Some(e);
    }
}

/// `0x1420bcff0`, after the virtual split: each logical link joins the nodes it holds at the
/// same place (`0x1420bc630`, `0x1420bd1f0`: the node with one edge hands it to the other);
/// a link without such pairs (`0x1420bc9a0`) hands the split virtual node nearest the plain
/// edge to the plain node, which moves there. Emptied nodes are erased (later indices move
/// up). Returns the linked nodes left with more than two edges (they join the tile list,
/// `0x1420bd530`) and the old → new index map.
pub fn resolve_links(lg: &mut LayoutGraph) -> Result<(Vec<usize>, Vec<Option<usize>>), GenError> {
    let mut removed = vec![false; lg.nodes.len()];
    let mut emptied: Vec<usize> = Vec::new();
    let mut empty = |removed: &mut Vec<bool>, n: usize| {
        if !removed[n] {
            removed[n] = true;
            emptied.push(n);
        }
    };
    for l in 0..lg.links.len() {
        let link = lg.links[l];
        let (Some(p0), Some(p1)) = (link[0], link[1]) else {
            return Err(GenError::Unsupported("bad logical link".into()));
        };
        let cand: Vec<usize> = link.iter().flatten().copied().filter(|&n| !removed[n]).collect();
        let mut pairs = Vec::new();
        for i in 0..cand.len() {
            for j in i + 1..cand.len() {
                if lg.nodes[cand[i]].pos == lg.nodes[cand[j]].pos {
                    pairs.push((cand[i], cand[j]));
                }
            }
        }
        if pairs.is_empty() {
            // `0x1420bc9a0`
            let (plain, v1, v2) = match (link[2], link[3]) {
                (None, Some(s3)) => (p0, p1, s3),
                (Some(s2), None) => (p1, p0, s2),
                _ => return Err(GenError::Unsupported("logical link without exactly one split node".into())),
            };
            let deg = degree(lg, plain);
            let pp = lg.nodes[plain].pos;
            let choose_v1 = match deg {
                1 => {
                    if removed[v1] && removed[v2] {
                        continue;
                    }
                    let e = lg.nodes[plain].slots.iter().flatten().copied().next().unwrap();
                    let ed = &lg.edges[e];
                    let q = if ed.from == plain {
                        ed.points.first().copied().unwrap_or(lg.nodes[ed.to].pos)
                    } else {
                        ed.points.last().copied().unwrap_or(lg.nodes[ed.from].pos)
                    };
                    if removed[v2] {
                        true
                    } else if !removed[v1] {
                        let a = seg_dist(lg.nodes[v1].pos, q, pp);
                        let b = seg_dist(lg.nodes[v2].pos, q, pp);
                        a.0 * b.1 < b.0 * a.1
                    } else {
                        false
                    }
                }
                d if d > 2 => continue,
                _ => {
                    let d2 = |n: usize| {
                        let (dx, dy) = ((lg.nodes[n].pos.0 - pp.0) as i64, (lg.nodes[n].pos.1 - pp.1) as i64);
                        dx * dx + dy * dy
                    };
                    d2(v1) < d2(v2)
                }
            };
            let chosen = if choose_v1 { v1 } else { v2 };
            let edges: Vec<usize> = lg.nodes[chosen].slots.iter().flatten().copied().collect();
            for e in edges {
                move_end(lg, e, chosen, plain);
            }
            lg.nodes[plain].pos = lg.nodes[chosen].pos;
            lg.nodes[plain].virtual_end = false;
            empty(&mut removed, chosen);
        } else {
            for (p, q) in pairs {
                let (from, to) = if degree(lg, p) == 1 { (p, q) } else { (q, p) };
                for x in [p, q] {
                    for &li in lg.node_links.get(&x).cloned().unwrap_or_default().iter() {
                        for slot in lg.links[li].iter_mut() {
                            if *slot == Some(from) {
                                *slot = Some(to);
                            }
                        }
                    }
                }
                if let Some(e) = lg.nodes[from].slots.iter().flatten().copied().next() {
                    move_end(lg, e, from, to);
                }
                if degree(lg, from) == 0 {
                    empty(&mut removed, from);
                }
            }
        }
    }
    let mut tiles = Vec::new();
    for link in lg.links.clone() {
        for n in link.into_iter().flatten() {
            if !removed[n] && degree(lg, n) > 2 && !tiles.contains(&n) {
                tiles.push(n);
            }
        }
    }
    lg.links.clear();
    lg.node_links.clear();
    // Erase the emptied nodes.
    let mut map = vec![None; lg.nodes.len()];
    let mut k = 0;
    for (i, m) in map.iter_mut().enumerate() {
        if !removed[i] {
            *m = Some(k);
            k += 1;
        }
    }
    let mut i = 0;
    lg.nodes.retain(|_| {
        i += 1;
        !removed[i - 1]
    });
    for e in lg.edges.iter_mut() {
        e.from = map[e.from].ok_or_else(|| GenError::Unsupported("edge at an erased node".into()))?;
        e.to = map[e.to].ok_or_else(|| GenError::Unsupported("edge at an erased node".into()))?;
    }
    let tiles = tiles.into_iter().filter_map(|n| map[n]).collect();
    Ok((tiles, map))
}
