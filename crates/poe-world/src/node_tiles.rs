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
    tileset::TileKey,
};

/// The layout graph as an [`EdgeView`] (heights from the `.tgr` nodes; split-off nodes 0).
pub struct LayoutView<'a> {
    pub lg: &'a LayoutGraph,
    pub graph: &'a Graph,
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
        self.graph.nodes.get(node).and_then(|n| n.values.get(1).copied()).unwrap_or(0)
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

#[derive(Debug, Clone)]
struct Entry {
    tile: usize,
    orientation: u8,
    weight: u32,
    size: (u8, u8),
}

/// `0x1420c76d0`, tile branch: the sizes the feature-less tiles with the key's edges come in
/// (`0x141d09700`, sorted), and per size every tile entry with those edges and that size
/// (`0x141d0a7d0`; the size swapped when there is none), filtered by the node's orientation, or
/// for `(any)` by `0x1420b87b0` (with the climb check, then without).
fn candidates(view: &LayoutView, node: usize, key: &TileKey, sides: &[usize; 4], master: &Master, types: &mut TerrainTypes, files: &dyn Files) -> Result<Vec<Entry>, GenError> {
    let set = &master.tiles;
    let mut all = Vec::new();
    for (i, t) in set.tiles.iter().enumerate() {
        if t.weight == 0 {
            continue;
        }
        let base = TileKey::from_tdt(&t.tdt);
        for o in 0..8u8 {
            if t.orientations & (1 << o) == 0 {
                continue;
            }
            let k = base.oriented(o);
            if k.edges == key.edges {
                let plain = t.tdt.feature.as_deref().unwrap_or("").is_empty();
                all.push((i, o, t.weight, k, plain));
            }
        }
    }
    let sizes: BTreeSet<(u8, u8)> = all.iter().filter(|a| a.4).map(|a| a.3.size).collect();
    let node_o = view.graph.nodes.get(node).map_or(8, |n| node_orientation(&n.orientation));
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
fn turn(o: u8, r: u8) -> u8 {
    const ONE: [u8; 8] = [1, 2, 3, 0, 5, 6, 7, 4];
    const THREE: [u8; 8] = [3, 0, 1, 2, 7, 4, 5, 6];
    if o >= 8 {
        return o;
    }
    if r == 1 { ONE[o as usize] } else { THREE[o as usize] }
}

/// `0x1420b8e20`: how far the node's edges point away from their sides (degrees).
fn side_cost(view: &LayoutView, node: usize, sides: &[usize; 4]) -> i32 {
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
    if let Some(n) = view.graph.nodes.get(node) {
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

/// `0x141fe7400` (region orientation 0): a remainder above half the alignment rounds up.
fn snap(v: i32, align: i32) -> i32 {
    if align <= 1 {
        return v;
    }
    let r = v.rem_euclid(align);
    v - r + if r > align / 2 { align } else { 0 }
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
    let mut list = candidates(view, node, &key, &sides, master, types, files)?;
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
        let px = snap((cx - w / 2).max(0), align).max(0).min(tiles.0 - w);
        let py = snap((cy - h / 2).max(0), align).max(0).min(tiles.1 - h);
        let inside = px >= 0 && py >= 0 && px + w <= tiles.0 && py + h <= tiles.1;
        let overlaps = chosen.iter().any(|p| px < p.pos.0 + p.size.0 as i32 && p.pos.0 < px + w && py < p.pos.1 + p.size.1 as i32 && p.pos.1 < py + h);
        if inside && map.free(px, py, w, h) && !overlaps {
            let mut key = TileKey::from_tdt(&master.tiles.tiles[c.tile].tdt).oriented(c.orientation);
            let mut pos = (px, py);
            keep_off_border(&mut pos, (w, h), &key.edges.clone(), max_size, tiles, types, files)?;
            let mut orientation = c.orientation;
            if view.graph.nodes.get(node).is_none_or(|n| node_orientation(&n.orientation) >= 8) {
                // `0x1420c9d70`: a quarter turn either way when that points the edges better and
                // puts none against the map border.
                let turned = |d: u8| sides.map(|s| if s == 4 { 4 } else { (s + d as usize) & 3 });
                let allowed = |sd: &[usize; 4]| {
                    !((pos.0 == 0 && sd.contains(&3))
                        || (pos.1 == 0 && sd.contains(&0))
                        || (c.size.0 as i32 + pos.0 == tiles.0 && sd.contains(&1))
                        || (c.size.1 as i32 + pos.1 == tiles.1 && sd.contains(&2)))
                };
                let (minus, plus) = (turned(3), turned(1));
                let cost = side_cost(view, node, &sides);
                let cost_minus = side_cost(view, node, &minus);
                let turn_by = if allowed(&minus) && cost_minus < cost {
                    if allowed(&plus) && side_cost(view, node, &plus) < cost_minus { Some(1) } else { Some(3) }
                } else if allowed(&plus) && side_cost(view, node, &plus) < cost {
                    Some(1)
                } else {
                    None
                };
                if let Some(d) = turn_by {
                    sides = if d == 1 { plus } else { minus };
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

/// Fixed axes of a node (`+0x8b`/`+0x8c`, the `.tgr` node values 9 and 10).
fn fixed_axes(graph: &Graph, n: usize) -> (bool, bool) {
    graph.nodes.get(n).map_or((false, false), |n| (n.values.get(9).copied().unwrap_or(0) != 0, n.values.get(10).copied().unwrap_or(0) != 0))
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
        place(lg, t, master, map, types, files)?;
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
        let (fx, fy) = fixed_axes(graph, n);
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
        let x = (cx - w / 2).max(0).min(tiles.0 - w);
        let y = (cy - h / 2).max(0).min(tiles.1 - h);
        let (sx, sy) = (snap(x, t.align), snap(y, t.align));
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
fn place(lg: &mut LayoutGraph, t: &NodeTile, master: &Master, map: &mut TileMap, types: &mut TerrainTypes, files: &dyn Files) -> Result<(), GenError> {
    let pos = (snap(t.pos.0, t.align), snap(t.pos.1, t.align));
    let def = &master.tiles.tiles[t.tile];
    map.place(pos.0, pos.1, &def.path, &def.tdt, t.orientation, 0, None);
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
            lg.nodes.push(LNode { pos: end, slots: [Some(e), None, None, None], fixed: true, virtual_end: false });
            if which == 0 { lg.edges[e].from = nn } else { lg.edges[e].to = nn }
            lg.nodes[node].slots[slot] = None;
        }
    }
    Ok(())
}
