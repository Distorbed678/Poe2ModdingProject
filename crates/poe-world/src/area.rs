//! A whole area's terrain layout from its `WorldAreas` id and seed: the client's generator
//! passes in order (topology, graph layout, rooms, edges, edge tiles, corner lattice, fill,
//! de-repetition, outer buffer). The tile hash of the result is what the client reports.

use std::collections::HashMap;

use poe_data::{GameData, Value};

use crate::{
    node_tiles,
    edge_tiles::{EdgeInfo, EdgeTiler},
    fill::fill_pass,
    generate::{GenError, Master, RoomPlacement, TileMap, derepeat_pass, room_pass_oriented},
    graph::Graph,
    layout::{self, JitterNode},
    lattice,
    outer_buffer::{Margins, outer_buffer},
    rng::{TerrainRng, lcg_index},
    rooms::{PlacedGraph, PlacedRoom, RoomSet, TerrainTypes, place_node, tsi_entries},
    routing::{EXIT_CELL, RouteEdge, Router, SIDE_DIR},
    smoothing::{self, LEdge, LNode, LayoutGraph},
    split::{SplitType, split_virtual},
};

fn data(e: impl std::fmt::Display) -> GenError {
    GenError::Unsupported(e.to_string())
}

/// A point in the pipeline [`generate_probed`] reports.
pub enum Stage<'a> {
    /// The layout graph after smoothing (`"smooth"`), the virtual split (`"split"`), before the
    /// edge tiles (`"tiles"`).
    Graph(&'a str, &'a LayoutGraph),
    /// The tile map after the room passes.
    Rooms(&'a TileMap),
    /// The tile map after the edge tiles, with the number of pieces before them.
    Edges(&'a TileMap, usize),
    /// The corner lattice before the client step it is named after (`borders`, `flood`,
    /// `heights`, `92ce0`, `fill`), as the DLL's `lattice` snapshots are.
    Lattice(&'a str, &'a TileMap),
}

/// A generated area.
pub struct Generated {
    /// The topology's graph file.
    pub graph: String,
    pub map: TileMap,
    /// RNG draws used (the client continues with doodads and entities).
    pub draws: u64,
}

/// The topology graphs (`.tgr`/`.dgr`) an area chooses from (`0x141fdfe30`): those whose last
/// Topologies column (league/mode, topology +0x40) equals `mode`, else those marked 0x39 (any);
/// none at all retries with mode 1 (the default).
pub fn topologies(gd: &GameData, area_id: &str) -> Result<Vec<String>, GenError> {
    topologies_for(gd, area_id, 1)
}

pub fn topologies_for(gd: &GameData, area_id: &str, mode: i64) -> Result<Vec<String>, GenError> {
    Ok(topology_entries(gd, area_id, mode)?.into_iter().map(|t| t.0).collect())
}

/// [`topologies_for`] with each topology's allowed region orientations (bit per orientation,
/// Topologies list column 6 at topology +0x34; 8 = all).
pub fn topology_entries(gd: &GameData, area_id: &str, mode: i64) -> Result<Vec<(String, u8)>, GenError> {
    let areas = gd.table("WorldAreas").map_err(data)?;
    let area = areas.find_by_str("Id", area_id).map_err(data)?.ok_or_else(|| data(format!("no area {area_id}")))?;
    let topologies = gd.table("Topologies").map_err(data)?;
    let rows: Vec<usize> = match area.get("Topologies").map_err(data)? {
        Value::List(l) => l.iter().filter_map(Value::as_row).collect(),
        _ => Vec::new(),
    };
    let last = topologies.schema().columns.len() - 1;
    let (mut exact, mut any) = (Vec::new(), Vec::new());
    for &r in &rows {
        let row = topologies.row(r).ok_or_else(|| data("bad topology row"))?;
        let path = row.str("DGRFile").map_err(data)?;
        let mut mask = 0u8;
        if let Value::List(l) = row.get_index(6).map_err(data)? {
            for v in l {
                if let Value::Int(o) = v {
                    mask |= if o == 8 { 0xff } else { 1 << (o & 7) };
                }
            }
        }
        let path = (path, mask);
        match row.get_index(last).map_err(data)? {
            Value::Int(v) if v == mode => exact.push(path),
            Value::Int(0x39) => any.push(path),
            _ => {}
        }
    }
    if !exact.is_empty() {
        return Ok(exact);
    }
    if any.is_empty() && mode != 1 {
        return topology_entries(gd, area_id, 1);
    }
    Ok(any)
}

/// Generates area `area_id` with `seed` (`TerrainGen_Run` up to the tile hash).
pub fn generate(gd: &GameData, area_id: &str, seed: u32) -> Result<Generated, GenError> {
    generate_with(gd, area_id, &mut TerrainRng::new(seed), None)
}

/// A layout choice the server makes for the client: `map_force_topology` (1-based index into
/// [`topologies`]) and `map_force_orientation` (stat value − 1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Forced {
    pub topology: Option<u32>,
    pub orientation: Option<u8>,
}

/// [`generate`] with a caller's RNG (for draw logs) and optionally a forced topology
/// (`map_force_topology`, stat key 0xb92: 1-based index into [`topologies`]; `0x141ff90f0`
/// still draws the topology value).
///
/// Without one the client chooses by the world map (`0x141ff9280` → `0x141fa6440`: the
/// directions of the connected areas' map pins against each topology's direction list, three
/// passes from exact to adjacent) and only falls back to the index draw here when nothing
/// matches; that choice is not ported yet.
pub fn generate_with(gd: &GameData, area_id: &str, rng: &mut TerrainRng, forced: Option<Forced>) -> Result<Generated, GenError> {
    generate_probed(gd, area_id, rng, forced, &mut |_| {})
}

/// [`generate_with`] reporting intermediate [`Stage`]s to `probe`, for comparing with the
/// client's trace.
pub fn generate_probed(gd: &GameData, area_id: &str, rng: &mut TerrainRng, forced: Option<Forced>, probe: &mut dyn FnMut(Stage)) -> Result<Generated, GenError> {
    let forced = forced.unwrap_or_default();
    let graphs = topology_entries(gd, area_id, 1)?;
    if graphs.is_empty() {
        return Err(data(format!("{area_id}: no topologies")));
    }
    let files = gd.files();
    let sub = rng.below(u32::MAX);
    let index = match forced.topology {
        Some(f) if f >= 1 && (f as usize) <= graphs.len() => f as usize - 1,
        _ => lcg_index(sub, graphs.len() as u32) as usize,
    };
    let (dgr, mask) = graphs[index].clone();
    // Region orientation: forced, else what the world-map match gives (not ported: the lowest
    // allowed one, right for the act 1 areas checked). Without a match the client draws it from
    // the topology's list (`0x141ff90f0`: `region::pick(mask, lcg_index(sub, 0x348))`).
    let orientation = forced.orientation.filter(|&o| o < 8).unwrap_or(if mask == 0 || mask == 0xff { 0 } else { mask.trailing_zeros() as u8 });
    let mut graph = Graph::parse_bytes(&crate::generate::Files::read(files, &dgr)?).map_err(|e| data(format!("{dgr}: {e:?}")))?;
    turn(files, &mut graph, orientation)?;
    let map = generate_graph_probed(files, &graph, rng, probe)?;
    Ok(Generated { graph: dgr, map, draws: rng.draws })
}

/// Turns a graph into region orientation `o` (edge types decide their mirroring).
fn turn(files: &dyn crate::generate::Files, graph: &mut Graph, o: u8) -> Result<(), GenError> {
    let mut types = TerrainTypes::default();
    let mut symmetric = std::collections::HashMap::new();
    for e in &graph.edges {
        let key = e.edge_type.to_ascii_lowercase();
        if !symmetric.contains_key(&key) {
            let v = edge_symmetric(&mut types, files, &key)?;
            symmetric.insert(key, v);
        }
    }
    crate::region::turn_graph(graph, o, &|t| symmetric.get(&t.to_ascii_lowercase()).copied().unwrap_or(false));
    Ok(())
}

/// `0x141fe9970`: existence rolls (nodes, node +0x44 = value 0: 0 removes, ≥100 keeps, else
/// `below(100)`; then every edge with a non-zero chance), the `shuffle` groups and the `unique`
/// groups. Removed nodes leave the graph (later node indices move up, as the client erases
/// them) and take their edges with them; removed edges stay as `true` in the returned flags.
/// `0x141fea2e0` / `0x141fee830` (`.dgr`, before the existence rolls): each node with a random
/// offset (values 5..9: x min, y min, x max, y max) moves by min + `range(0, max - min)` on x,
/// then on y.
fn random_offsets(graph: &mut Graph, rng: &mut TerrainRng) {
    for n in &mut graph.nodes {
        let v = |i: usize| n.values.get(i).copied().unwrap_or(0);
        let (x0, y0, x1, y1) = (v(5), v(6), v(7), v(8));
        if x0 == x1 && y0 == y1 && x0 == 0 && y0 == 0 {
            continue;
        }
        let dx = if x0 < x1 { x0 + rng.range(0, (x1 - x0) as u32) as i32 } else { x0 };
        let dy = if y0 < y1 { y0 + rng.range(0, (y1 - y0) as u32) as i32 } else { y0 };
        n.x += dx;
        n.y += dy;
    }
}

fn prune(graph: &Graph, rng: &mut TerrainRng) -> Result<(Graph, Vec<bool>), GenError> {
    let mut drop_node = vec![false; graph.nodes.len()];
    for (i, n) in graph.nodes.iter().enumerate() {
        let chance = n.values.first().copied().unwrap_or(100);
        drop_node[i] = chance == 0 || (chance < 100 && rng.below(100) as i32 >= chance);
    }
    let mut removed: Vec<bool> = graph
        .edges
        .iter()
        .map(|e| {
            let exist = e.values.first().copied().unwrap_or(100);
            !(exist != 0 && (rng.below(100) as i32) < exist)
        })
        .collect();
    let mut g = graph.clone();
    let mut dropped = vec![false; g.nodes.len()];
    for n in 0..g.nodes.len() {
        if drop_node[n] {
            remove_node(&mut g, &mut removed, &mut dropped, n);
        }
    }
    let drop_node_before = dropped.clone();
    shuffle_nodes(&mut g, &drop_node_before, rng);
    // `unique<N>[#count]` (`0x141feb3b0`): keep `count` random nodes of group N.
    let mut groups: Vec<(u32, Vec<usize>)> = Vec::new();
    for (i, n) in g.nodes.iter().enumerate() {
        if dropped[i] {
            continue;
        }
        let Some(t) = n.tags.iter().find(|t| t.len() >= 6 && t[..6].eq_ignore_ascii_case("unique")) else { continue };
        let mut rest = &t[6..];
        if rest.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            return Err(data(format!("node {i}: lettered unique tag {t:?} (session history) is not ported")));
        }
        let mut count = 1u32;
        if let Some((a, b)) = rest.split_once('#') {
            rest = a;
            count = b.trim().parse().unwrap_or(1);
        }
        let k = if rest.is_empty() { 0 } else { match rest.trim().parse::<usize>() { Ok(v) => v, Err(_) => continue } };
        if groups.len() <= k {
            groups.resize(k + 1, (0, Vec::new()));
        }
        groups[k].0 = groups[k].0.max(count);
        groups[k].1.push(i);
    }
    for (count, mut list) in groups {
        if (count as usize) < list.len() {
            for _ in 0..count {
                let k = rng.below(list.len() as u32) as usize;
                list.remove(k);
            }
            for n in list {
                remove_node(&mut g, &mut removed, &mut dropped, n);
            }
        }
    }
    let drop_node = dropped;
    // Erase the dropped nodes.
    let mut new_index = vec![usize::MAX; g.nodes.len()];
    let mut k = 0;
    for i in 0..g.nodes.len() {
        if !drop_node[i] {
            new_index[i] = k;
            k += 1;
        }
    }
    for (e, edge) in g.edges.iter_mut().enumerate() {
        let (f, t) = (new_index[edge.from as usize], new_index[edge.to as usize]);
        if f == usize::MAX || t == usize::MAX {
            removed[e] = true;
        }
        edge.from = f.min(u32::MAX as usize) as u32;
        edge.to = t.min(u32::MAX as usize) as u32;
    }
    let mut i = 0;
    g.nodes.retain(|_| {
        i += 1;
        !drop_node[i - 1]
    });
    Ok((g, removed))
}

/// A node's live edge slots (`.tgr` order, first four, removed edges empty).
fn live_slots(g: &Graph, removed: &[bool], n: usize) -> [Option<usize>; 4] {
    let mut s = [None; 4];
    for (k, &e) in g.nodes[n].edges.iter().take(4).enumerate() {
        if !removed[e as usize] {
            s[k] = Some(e as usize);
        }
    }
    s
}

/// `0x141feb9d0`: removes a node. Two edges of one type with the far nodes at the node's
/// height merge into one through it (`0x141fec1c0`: the first edge takes the second's far
/// node, the node's position and the second edge's points); a room node with two or more
/// edges only loses its room; anything else goes with its edges (`0x141fec520`).
fn remove_node(g: &mut Graph, removed: &mut [bool], dropped: &mut Vec<bool>, n: usize) {
    let slots: Vec<usize> = live_slots(g, removed, n).into_iter().flatten().collect();
    let height = |g: &Graph, i: usize| g.nodes[i].values.get(1).copied().unwrap_or(0);
    let other = |g: &Graph, e: usize| if g.edges[e].from as usize == n { g.edges[e].to as usize } else { g.edges[e].from as usize };
    if slots.len() == 2 {
        let (e1, e2) = (slots[0], slots[1]);
        let (o1, o2) = (other(g, e1), other(g, e2));
        if g.edges[e1].edge_type.eq_ignore_ascii_case(&g.edges[e2].edge_type) && height(g, o1) == height(g, n) && height(g, o2) == height(g, n) {
            let e1_from = g.edges[e1].from as usize == n;
            let e2_from = g.edges[e2].from as usize == n;
            // e2's slot at its far node: cleared, then e1 takes that node's first free slot.
            removed[e2] = true;
            let free = live_slots(g, removed, o2).iter().position(|s| s.is_none());
            if let Some(k) = free {
                if k < g.nodes[o2].edges.len() {
                    g.nodes[o2].edges[k] = e1 as u32;
                } else {
                    g.nodes[o2].edges.push(e1 as u32);
                }
            }
            if e1_from { g.edges[e1].from = o2 as u32 } else { g.edges[e1].to = o2 as u32 }
            // Points: the node, then e2's (running away from the node).
            let pos = (g.nodes[n].x, g.nodes[n].y);
            let mut far: Vec<(i32, i32)> = g.edges[e2].points.clone();
            if !e2_from {
                far.reverse();
            }
            // `far` now runs node → o2.
            let pts = &mut g.edges[e1].points;
            if e1_from {
                // e1 now starts at o2: o2 … node, then e1's own points.
                let mut v: Vec<(i32, i32)> = far.into_iter().rev().collect();
                v.push(pos);
                v.extend(pts.iter().copied());
                *pts = v;
            } else {
                pts.push(pos);
                pts.extend(far);
            }
            dropped[n] = true;
            return;
        }
    }
    if slots.len() >= 2 && !g.nodes[n].room_type.is_empty() {
        g.nodes[n].room_type.clear();
        g.nodes[n].tags.clear();
        g.nodes[n].orientation = "(any)".into();
        return;
    }
    // `0x141fec520`: an edge with internal points ends at a new node on its nearest point
    // instead of going.
    for e in slots {
        if g.edges[e].points.is_empty() {
            removed[e] = true;
            continue;
        }
        let from = g.edges[e].from as usize == n;
        let p = if from { g.edges[e].points.remove(0) } else { g.edges[e].points.pop().unwrap() };
        let nn = g.nodes.len();
        g.nodes.push(crate::graph::Node {
            x: p.0,
            y: p.1,
            edges: vec![e as u32],
            room_type: String::new(),
            orientation: "(any)".into(),
            tags: Vec::new(),
            values: vec![100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            extra: Vec::new(),
        });
        if from { g.edges[e].from = nn as u32 } else { g.edges[e].to = nn as u32 }
    }
    dropped.resize(g.nodes.len(), false);
    dropped[n] = true;
}

/// The `shuffle` / `shuffle<N>` groups (by N, in node order): while a group holds two or more
/// nodes, two draws pick nodes that leave it, and two different ones swap room type and tags
/// (positions and orientations stay).
fn shuffle_nodes(g: &mut Graph, dropped: &[bool], rng: &mut TerrainRng) {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, n) in g.nodes.iter().enumerate() {
        if dropped[i] {
            continue;
        }
        let Some(t) = n.tags.iter().find(|t| t.starts_with("shuffle")) else { continue };
        let rest = &t["shuffle".len()..];
        let k = if rest.is_empty() {
            0
        } else {
            match rest.trim().parse::<i32>() {
                Ok(v) if v >= 0 => v as usize,
                _ => continue,
            }
        };
        if groups.len() <= k {
            groups.resize(k + 1, Vec::new());
        }
        groups[k].push(i);
    }
    for mut list in groups {
        while list.len() > 1 {
            let n = list.len() as u32;
            let a = list[rng.below(n) as usize];
            let b = list[rng.below(n) as usize];
            list.retain(|&x| x != a);
            list.retain(|&x| x != b);
            if a != b {
                let (ra, ta) = (g.nodes[a].room_type.clone(), g.nodes[a].tags.clone());
                g.nodes[a].room_type = g.nodes[b].room_type.clone();
                g.nodes[a].tags = g.nodes[b].tags.clone();
                g.nodes[b].room_type = ra;
                g.nodes[b].tags = ta;
            }
        }
    }
}

/// `0x141d14d10`: an edge type that a mirrored region leaves alone (no height change, the same
/// ground either side, and for a virtual edge a symmetric split).
fn edge_symmetric(types: &mut TerrainTypes, files: &dyn crate::generate::Files, et: &str) -> Result<bool, GenError> {
    let t = types.edge(files, et)?.clone();
    if t.height_change != 0 || t.grounds[0] != t.grounds[1] {
        return Ok(false);
    }
    if !t.virtual_edge {
        return Ok(true);
    }
    if t.splits[0].0 != t.splits[1].0 || t.splits[0].1 != t.splits[1].1 {
        return Ok(false);
    }
    edge_symmetric(types, files, &t.splits[0].0)
}

/// A `.dgr` graph: the dungeon layout (`crate::dungeon`), then its tile pass
/// (`crate::dungeon_tiles`).
fn generate_dungeon(files: &dyn crate::generate::Files, graph: &Graph, removed: &[bool], rng: &mut TerrainRng) -> Result<TileMap, GenError> {
    let mut built = build_dungeon(files, graph, removed, rng, None)?;
    // `0x141ff7e00` / `0x141ff05f0`: each `graph` node's subgraph is generated after the main
    // graph's fill and de-repetition, straight into its reserved rectangle of the map.
    let mut done: Vec<(Built, (i32, i32), Margins)> = Vec::new();
    for (file, origin) in std::mem::take(&mut built.subgraphs) {
        let mut sub = Graph::parse_bytes(&files.read(&file)?).map_err(|e| data(format!("{file}: {e:?}")))?;
        if sub.dungeon_defaults.is_none() {
            return Err(GenError::Unsupported(format!("subgraph {file}: not a dungeon graph (not ported)")));
        }
        // Turned like the main graph (`0x141ff05f0` sets the parent's region).
        turn(files, &mut sub, graph.region)?;
        random_offsets(&mut sub, rng);
        let (sub, removed) = prune(&sub, rng)?;
        // The region sits inside the subgraph master's `OuterBufferSize` margins.
        let m = Margins::parse(&crate::text::decode_utf16(&files.read(&sub.master_file)?));
        let at = (origin.0 + m.left as i32, origin.1 + m.top as i32);
        let inner = build_dungeon(files, &sub, &removed, rng, Some((&built.map, at)))?;
        if !inner.subgraphs.is_empty() {
            return Err(GenError::Unsupported(format!("subgraph {file}: nested subgraphs (not ported)")));
        }
        built.map.paste(&inner.map, at);
        done.push((inner, origin, m));
    }
    // `0x141ff7a10`: the outer fill and outer buffer per graph, the last subgraph first, then the
    // main graph's below.
    for (mut inner, origin, m) in done.into_iter().rev() {
        let g = inner.tsi.get("OuterGroundType").cloned();
        let (w, h) = (inner.map.width, inner.map.height);
        if m.top + m.right + m.bottom + m.left == 0 {
            if let Some(g) = g {
                let rect = (origin.0, origin.1, origin.0 + w as i32, origin.1 + h as i32);
                crate::outer_buffer::outer_fill(&mut built.map, rect, &g, true, &mut inner.master, files)?;
            }
            continue;
        }
        let mut window = built.map.window(origin, w + m.left + m.right, h + m.top + m.bottom);
        let rect = (m.left as i32, m.top as i32, (m.left + w) as i32, (m.top + h) as i32);
        let edges = inner.master.edge_type_table(files)?;
        crate::outer_buffer::outer_buffer_window(&mut window, rect, g.as_deref(), true, &mut inner.master, files, &edges, rng)?;
        built.map.paste(&window, origin);
    }
    let Built { map, mut master, tsi_bytes, tsi, .. } = built;
    let margins = Margins::parse(&crate::text::decode_utf16(&tsi_bytes));
    let edge_types = master.edge_type_table(files)?;
    let mut map = outer_buffer(&map, margins, tsi.get("OuterGroundType").map(String::as_str), true, &mut master, files, &edge_types, rng)?;
    // Tile records keep heights in world units (`StoreyHeight` per storey).
    let storey: i16 = tsi.get("StoreyHeight").and_then(|v| v.trim().parse().ok()).unwrap_or(16);
    for c in map.cells.iter_mut().flatten() {
        c.height *= storey;
    }
    Ok(map)
}

struct Built {
    map: TileMap,
    master: Master,
    tsi_bytes: Vec<u8>,
    tsi: std::collections::HashMap<String, String>,
    /// Reserved subgraph rectangles: graph file and tile origin.
    subgraphs: Vec<(String, (i32, i32))>,
}

/// One dungeon graph up to its de-repetition: layout, tile pass, fill. `window` is the map and
/// origin a subgraph is generated into (its corners start as the map's).
fn build_dungeon(files: &dyn crate::generate::Files, graph: &Graph, removed: &[bool], rng: &mut TerrainRng, window: Option<(&TileMap, (i32, i32))>) -> Result<Built, GenError> {
    let tsi_path = graph.master_file.clone();
    let tsi_bytes = files.read(&tsi_path)?;
    let tsi = tsi_entries(&tsi_bytes);
    let dir = tsi_path.rsplit_once('/').map_or("", |d| d.0);
    let rel = |k: &str| tsi.get(k).map(|v| format!("{dir}/{v}"));
    let set = RoomSet::load(files, &rel("RoomSet").ok_or_else(|| data("no RoomSet"))?)?;
    let grid = crate::dungeon::layout(files, graph, removed, &set, rng)?;
    if std::env::var("POE_DBG").is_ok() {
        for (i, r) in grid.rooms.iter().enumerate() {
            eprintln!("DBG room {i} {} o{} origin {:?} size {:?} special {}", r.path, r.orientation, r.origin, r.size, r.special);
        }
    }
    let mut master = Master::load(files, &rel("TileSet").ok_or_else(|| data("no TileSet"))?, &rel("FillTiles").ok_or_else(|| data("no FillTiles"))?)?;
    let ecf = match rel("EdgeCombination") {
        Some(p) => crate::dungeon_tiles::Ecf::parse(&crate::text::decode_utf16(&files.read(&p)?)),
        None => Default::default(),
    };
    let unit: u32 = tsi.get("RoomUnitSize").and_then(|v| v.trim().parse().ok()).unwrap_or(9);
    let step = unit - grid.overlap;
    let mut map = TileMap::new(grid.w * step + grid.overlap, grid.h * step + grid.overlap);
    map.ground_flags = master.ground_flag_table(files)?;
    map.edge_flags = master.edge_flag_table(files)?;
    if let Some((outer, origin)) = window {
        map.copy_lattice(outer, origin);
    }
    let mut rooms = std::collections::HashMap::new();
    for r in &grid.rooms {
        if !r.path.is_empty() && !rooms.contains_key(&r.path) {
            rooms.insert(r.path.clone(), crate::generate::load_room(files, &r.path)?);
        }
    }
    let edge_types = master.edge_type_table(files)?;
    let flag = |k: &str| tsi.get(k).is_some_and(|v| v.trim() != "0");
    let pass = crate::dungeon_tiles::TilePass { master: &master, ecf: &ecf, unit, overlap: grid.overlap, edge_types: &edge_types, overlap_fixes: flag("AllowInteriorOverlapFixes"), doors_disabled: flag("DisableDoors") };
    let (cells, moved) = pass.run(&grid, &rooms, &mut map, rng)?;
    if let Ok(path) = std::env::var("POE_DUMP_PIECES") {
        let mut out = String::new();
        for p in &map.pieces {
            let h = map.get(p.x as u32, p.y as u32).map_or(0, |c| c.height);
            out += &format!("{} {} o{} h{}\t{}\n", p.x, p.y, p.orientation, h, p.path);
        }
        std::fs::write(path, out).ok();
    }
    let (fill_rooms, wiggle) = crate::fill::fill_cells(&mut map, &mut master, files, &cells, rng)?;
    let wiggle = [moved, wiggle].concat();
    crate::dungeon_tiles::wiggle(&mut master, files, &mut map, wiggle, rng)?;
    // De-repetition over the rooms and fill rooms, then the outer buffer.
    let mut placements: Vec<RoomPlacement> = grid
        .rooms
        .iter()
        .filter_map(|r| Some(RoomPlacement { room: rooms.get(&r.path)?, origin: (r.origin.0 * step as i32, r.origin.1 * step as i32), orientation: r.orientation }))
        .collect();
    placements.extend(fill_rooms.iter().map(|r| RoomPlacement { room: &r.room, origin: r.origin, orientation: r.orientation }));
    derepeat_pass(&mut map, &master, &placements, rng);
    let subgraphs = grid.rooms.iter().filter_map(|r| Some((r.subgraph.clone()?, (r.origin.0 * step as i32, r.origin.1 * step as i32)))).collect();
    drop(placements);
    Ok(Built { map, master, tsi_bytes, tsi, subgraphs })
}

/// Everything after the topology choice, for one graph.
pub fn generate_graph(files: &dyn crate::generate::Files, graph: &Graph, rng: &mut TerrainRng) -> Result<TileMap, GenError> {
    generate_graph_probed(files, graph, rng, &mut |_| {})
}

fn generate_graph_probed(files: &dyn crate::generate::Files, graph: &Graph, rng: &mut TerrainRng, probe: &mut dyn FnMut(Stage)) -> Result<TileMap, GenError> {
    let mut moved;
    let graph = if graph.dungeon_defaults.is_some() {
        moved = graph.clone();
        random_offsets(&mut moved, rng);
        &moved
    } else {
        graph
    };
    let (pruned, removed) = prune(graph, rng)?;
    let graph = &pruned;
    if graph.dungeon_defaults.is_some() {
        return generate_dungeon(files, graph, &removed, rng);
    }
    let mut nodes: Vec<JitterNode> = graph
        .nodes
        .iter()
        .map(|n| JitterNode {
            pos: (n.x, n.y),
            fix_x: n.values.get(9).copied().unwrap_or(0) != 0,
            fix_y: n.values.get(10).copied().unwrap_or(0) != 0,
        })
        .collect();
    let mut points: Vec<(i32, i32)> = graph
        .edges
        .iter()
        .zip(&removed)
        .filter(|(_, r)| !**r)
        .flat_map(|(e, _)| e.points.iter().copied())
        .collect();
    let tiles = (graph.width as i32, graph.height as i32);
    layout::jitter(&mut nodes, &mut points, tiles, rng).map_err(|e| data(format!("{e:?}")))?;
    let mut it = points.iter().copied();
    let edge_points: Vec<Vec<(i32, i32)>> = graph
        .edges
        .iter()
        .zip(&removed)
        .map(|(e, r)| if *r { Vec::new() } else { e.points.iter().map(|_| it.next().unwrap()).collect() })
        .collect();

    // Master: tile set, fill tiles, room set, outer buffer.
    let tsi_path = graph.master_file.clone();
    let tsi_bytes = files.read(&tsi_path)?;
    let tsi = tsi_entries(&tsi_bytes);
    let margins = Margins::parse(&crate::text::decode_utf16(&tsi_bytes));
    let dir = tsi_path.rsplit_once('/').map_or("", |d| d.0);
    let rel = |k: &str| tsi.get(k).map(|v| format!("{dir}/{v}"));
    let mut master = Master::load(files, &rel("TileSet").ok_or_else(|| data("no TileSet"))?, &rel("FillTiles").ok_or_else(|| data("no FillTiles"))?)?;
    let set = RoomSet::load(files, &rel("RoomSet").ok_or_else(|| data("no RoomSet"))?)?;
    let mut placed_graph = PlacedGraph { graph, nodes: nodes.iter().map(|n| n.pos).collect(), points: edge_points, removed };
    let mut types = TerrainTypes::default();

    // `0x1420c7400` sorts the room-less nodes before placement: virtual nodes (touching a
    // virtual edge; not dead ends at the border) and junctions of three or more edges get
    // exploded before routing (virtual ones only when no tile fits them at the room stage).
    let (mut virtual_nodes, mut junctions) = (Vec::new(), Vec::new());
    for n in 0..graph.nodes.len() {
        if !graph.nodes[n].room_type.is_empty() {
            continue;
        }
        let edges: Vec<usize> = crate::rooms::EdgeView::slots(&placed_graph, n).into_iter().flatten().collect();
        let mut virtual_end = false;
        for &e in &edges {
            virtual_end |= types.edge(files, &graph.edges[e].edge_type)?.virtual_edge;
        }
        match edges.len() {
            0 => {}
            1 => {
                let b = types.edge(files, &graph.edges[edges[0]].edge_type)?.size as i32;
                let (x, y) = placed_graph.nodes[n];
                let (tx, ty) = ((x / 23).min(tiles.0 - 1), (y / 23).min(tiles.1 - 1));
                let near = !(b <= tx && b <= ty && tx < tiles.0 - b && ty < tiles.1 - b);
                if virtual_end && !near {
                    virtual_nodes.push(n);
                }
            }
            2 if graph.edges[edges[0]].edge_type.eq_ignore_ascii_case(&graph.edges[edges[1]].edge_type) => {}
            k => {
                if virtual_end {
                    virtual_nodes.push(n);
                } else if k > 2 {
                    junctions.push(n);
                }
            }
        }
    }

    // Rooms.
    let mut placed: Vec<PlacedRoom> = Vec::new();
    for n in 0..graph.nodes.len() {
        if graph.nodes[n].room_type.is_empty() {
            continue;
        }
        let p = place_node(&set, &master.tiles, &placed_graph, n, tiles, &placed, &mut types, files, rng)?;
        if std::env::var_os("POE_DEBUG_ROOMS").is_some() {
            let name = p.tile.map_or_else(|| set.rooms[p.room].path.clone(), |t| master.tiles.tiles[t].path.clone());
            eprintln!("room node {n} {name} o{} at {},{} sides {:?}", p.orientation, p.pos.0, p.pos.1, p.sides);
        }
        placed.push(p);
    }
    let fixed: Vec<(bool, bool)> = graph
        .nodes
        .iter()
        .map(|n| (n.values.get(9).copied().unwrap_or(0) != 0, n.values.get(10).copied().unwrap_or(0) != 0))
        .collect();
    crate::rooms::repel(&mut placed, &mut placed_graph, &fixed, tiles, &mut types, files)?;
    let mut map = TileMap::new(graph.width, graph.height);
    map.ground_flags = master.ground_flag_table(files)?;
    map.edge_flags = master.edge_flag_table(files)?;
    for p in &placed {
        match p.tile {
            // `0x1420c9970`: a node that took a tile places just that tile.
            Some(t) => {
                let def = &master.tiles.tiles[t];
                map.place(p.pos.0, p.pos.1, &def.path, &def.tdt, p.orientation, 0, Some(t));
            }
            None => room_pass_oriented(&mut map, &master, &set.rooms[p.room].room, p.pos, p.orientation, false, rng)?,
        }
    }
    probe(Stage::Rooms(&map));

    // Exploded nodes (`0x1420cb200`). A virtual node that a tile fits is placed with the rooms
    // instead (`0x1420caa40`), which is not ported yet.
    for &n in virtual_nodes.iter().chain(&junctions) {
        if virtual_nodes.contains(&n) {
            let (sides, key) = crate::rooms::node_key(&placed_graph, n, &mut types, files)?;
            if !node_tiles::candidates(&placed_graph, n, 8, "", &key, &sides, &master.tiles, &mut types, files)?.is_empty() {
                return Err(data(format!("node {n}: virtual node tile at the room stage is not ported")));
            }
        }
        node_tiles::explode(&mut placed_graph, n, tiles, &mut types, files)?;
    }

    // Edge ends at the rooms' exits (`0x1420c5750`, `0x1420c5b50`), then routing.
    let mut route: Vec<RouteEdge> = Vec::new();
    let mut route_index = vec![usize::MAX; graph.edges.len()];
    for (i, e) in graph.edges.iter().enumerate() {
        if placed_graph.removed[i] {
            continue;
        }
        let et = types.edge(files, &e.edge_type)?.clone();
        if et.wildcard {
            continue;
        }
        route_index[i] = route.len();
        route.push(RouteEdge {
            ends: [placed_graph.nodes[e.from as usize], placed_graph.nodes[e.to as usize]],
            offsets: [(0, 0); 2],
            points: placed_graph.points[i].clone(),
            margin: if et.virtual_edge { e.tail.get(1).copied().unwrap_or(0) / 23 + 3 } else { et.size as i32 + 2 },
        });
    }
    let mut vends: Vec<[Option<[(i32, i32); 2]>; 2]> = vec![[None, None]; route.len()];
    for p in &placed {
        let node = p.node;
        let (rx, ry) = (p.pos.0 * 23, p.pos.1 * 23);
        let (rw, rh) = (p.size.0 as i32 * 23, p.size.1 as i32 * 23);
        let inside = |q: &(i32, i32)| q.0 >= rx && q.0 < rx + rw && q.1 >= ry && q.1 < ry + rh;
        for (slot, &ei) in graph.nodes[node].edges.iter().take(4).enumerate() {
            let ei = ei as usize;
            if route_index.get(ei).copied().unwrap_or(usize::MAX) == usize::MAX {
                continue;
            }
            let r = &mut route[route_index[ei]];
            let from = graph.edges[ei].from as usize == node;
            if from {
                let k = r.points.iter().position(|q| !inside(q)).unwrap_or(r.points.len());
                r.points.drain(..k);
            } else if let Some(k) = r.points.iter().position(|q| inside(q)) {
                r.points.truncate(k);
            }
            let side = p.sides[slot];
            if side > 3 {
                continue;
            }
            let at = |e: u8| {
                let (t, c) = (e as i32 / 3, EXIT_CELL[(e % 3) as usize]);
                let o = c + t * 23;
                let (x, y) = match side {
                    0 => (o, 0),
                    1 => (rw - 1, o),
                    2 => (rw - o - 1, rh - 1),
                    _ => (0, rh - o - 1),
                };
                (rx + x, ry + y)
            };
            let et = types.edge(files, &graph.edges[ei].edge_type)?;
            let which = (!from) as usize;
            let end = if et.virtual_edge {
                let mut v1 = at(p.key.virtual_exits[side]);
                let mut v2 = at(p.key.exits[side]);
                let n = placed_graph.nodes[node];
                let q = if from { r.points.first().copied().unwrap_or(r.ends[1]) } else { r.points.last().copied().unwrap_or(r.ends[0]) };
                let mut c = (v1.1 - q.1) * (n.0 - q.0) - (v1.0 - q.0) * (n.1 - q.1);
                let c2 = (n.0 - q.0) * (v2.1 - q.1) - (n.1 - q.1) * (v2.0 - q.0);
                if (c2 < 0) == (c < 0) {
                    c = ((v1.0 + v2.0) / 2 - n.0) * (v1.1 - n.1) - (v1.0 - n.0) * ((v1.1 + v2.1) / 2 - n.1);
                }
                if c < 0 {
                    std::mem::swap(&mut v1, &mut v2);
                }
                let m = ((v1.0 + v2.0) / 2, (v2.1 + v1.1) / 2);
                let d = SIDE_DIR[side];
                if (v1.1 - (d.1 + m.1)) * (m.0 - (d.0 + m.0)) - (v1.0 - (d.0 + m.0)) * (m.1 - (d.1 + m.1)) < 0 {
                    std::mem::swap(&mut v1, &mut v2);
                }
                vends[route_index[ei]][which] = Some([v1, v2]);
                m
            } else {
                at(p.key.exits[side])
            };
            r.ends[which] = end;
            r.offsets[which] = SIDE_DIR[side];
        }
    }
    let mut router = Router::new(tiles.0, tiles.1, &map, rng);
    for e in &route {
        router.reserve(e);
    }
    for (k, r) in route.iter_mut().enumerate() {
        if !router.route(r, rng) {
            return Err(data(format!("edge {k} failed to route (retries not ported)")));
        }
    }

    // The layout graph in the client's numbering (`0x141ef2800` splits room nodes per exit).
    let mut lg = LayoutGraph::default();
    for i in 0..graph.nodes.len() {
        let mut slots = [None; 4];
        for (k, &ei) in graph.nodes[i].edges.iter().take(4).enumerate() {
            let ei = ei as usize;
            if route_index.get(ei).copied().unwrap_or(usize::MAX) != usize::MAX {
                slots[k] = Some(route_index[ei]);
            }
        }
        let mut virtual_end = false;
        for &ei in &graph.nodes[i].edges {
            virtual_end |= types.edge(files, &graph.edges[ei as usize].edge_type)?.virtual_edge;
        }
        let fixed = placed.iter().any(|p| p.node == i);
        lg.nodes.push(LNode { pos: placed_graph.nodes[i], slots, fixed, virtual_end, orig: Some(i) });
    }
    for (gi, e) in graph.edges.iter().enumerate() {
        let ri = route_index[gi];
        if ri == usize::MAX {
            continue;
        }
        let et = types.edge(files, &e.edge_type)?.clone();
        lg.edges.push(LEdge {
            from: e.from as usize,
            to: e.to as usize,
            points: route[ri].points.clone(),
            offsets: route[ri].offsets,
            edge_type: e.edge_type.to_ascii_lowercase(),
            virtual_edge: et.virtual_edge,
            width: (e.tail.first().copied().unwrap_or(0), e.tail.get(1).copied().unwrap_or(0)),
            chirality: e.values.get(1).copied().unwrap_or(0) != 0,
            vends: vends[ri],
            split: false,
            fixed: false,
        });
    }
    for p in &placed {
        let node = p.node;
        let mut first = true;
        let slots = lg.nodes[node].slots;
        for (k, s) in slots.iter().enumerate() {
            let Some(e) = *s else { continue };
            let which = (lg.edges[e].from != node) as usize;
            let pos = route[e].ends[which];
            if first {
                lg.nodes[node].pos = pos;
                first = false;
            } else {
                let nn = lg.nodes.len();
                lg.nodes.push(LNode { pos, slots: [Some(e), None, None, None], fixed: true, virtual_end: false, orig: None });
                if which == 0 { lg.edges[e].from = nn } else { lg.edges[e].to = nn }
                lg.nodes[node].slots[k] = None;
            }
        }
    }
    smoothing::smooth(&mut lg);
    probe(Stage::Graph("smooth", &lg));
    // Virtual nodes no tile fitted (`0x1420c6cd0`).
    for &n in &virtual_nodes {
        node_tiles::virtual_junction(&mut lg, n, rng)?;
    }

    // Virtual edges split into their two banks.
    let mut split_types: HashMap<String, SplitType> = HashMap::new();
    let mut want: Vec<String> = lg.edges.iter().map(|e| e.edge_type.clone()).collect();
    while let Some(t) = want.pop() {
        if split_types.contains_key(&t) {
            continue;
        }
        let et = types.edge(files, &t)?.clone();
        want.extend(et.splits.iter().filter(|s| !s.0.is_empty()).map(|s| s.0.clone()));
        split_types.insert(t, SplitType { virtual_edge: et.virtual_edge, grounds: et.grounds.clone(), height_change: et.height_change, splits: et.splits.clone() });
    }
    let lookup = |t: &str| split_types.get(&t.to_ascii_lowercase()).cloned().unwrap_or_default();
    split_virtual(&mut lg, &lookup, (tiles.0 * 23, tiles.1 * 23), rng);
    probe(Stage::Graph("split", &lg));
    // Logical links of tile-less virtual nodes (`0x1420bcff0`).
    let (link_tiles, _) = node_tiles::resolve_links(&mut lg)?;

    // Rounding, border dead ends and node tiles (`0x1420cade0`), slot compaction, rounding,
    // end points.
    smoothing::round_corners(&mut lg);
    let mut tile_nodes = Vec::new();
    for gi in 0..graph.nodes.len() {
        // Sorted at the start of the tile stage (`0x1420c7400`): virtual nodes are not in the
        // tile list.
        if !graph.nodes[gi].room_type.is_empty() || virtual_nodes.contains(&gi) {
            continue;
        }
        let Some(n) = lg.nodes.iter().position(|x| x.orig == Some(gi)) else { continue };
        let edges: Vec<usize> = lg.nodes[n].slots.iter().flatten().copied().collect();
        let mut near = false;
        if let [e] = edges[..] {
            let b = types.edge(files, &lg.edges[e].edge_type)?.size as i32;
            let (x, y) = lg.nodes[n].pos;
            let (tx, ty) = ((x / 23).min(tiles.0 - 1), (y / 23).min(tiles.1 - 1));
            near = !(b <= tx && b <= ty && tx < tiles.0 - b && ty < tiles.1 - b);
        }
        if !node_tiles::needs_tile(&lg, n, near, &mut types, files)? {
            continue;
        }
        if !near {
            tile_nodes.push(n);
            continue;
        }
        let e = edges[0];
        let b = types.edge(files, &lg.edges[e].edge_type)?.size as i32;
        let (x, y) = lg.nodes[n].pos;
        let (tx, ty) = ((x / 23).min(tiles.0 - 1), (y / 23).min(tiles.1 - 1));
        let w = (lg.edges[e].from != n) as usize;
        let mut pos = lg.nodes[n].pos;
        let mut off = lg.edges[e].offsets[w];
        if tx < b {
            pos.0 = -1;
            off.0 = 1;
        } else if ty < b {
            pos.1 = -1;
            off.1 = 1;
        }
        if tx < tiles.0 - b {
            if tiles.1 - b <= ty {
                pos.1 = tiles.1 * 23;
                off.1 = -1;
            }
        } else {
            pos.0 = tiles.0 * 23;
            off.0 = -1;
        }
        lg.nodes[n].pos = pos;
        lg.nodes[n].fixed = true;
        lg.edges[e].offsets[w] = off;
    }
    tile_nodes.extend(link_tiles);
    node_tiles::place_node_tiles(&mut lg, graph, &tile_nodes, &master, &mut map, tiles, &mut types, files, rng)?;
    smoothing::compact_slots(&mut lg);
    smoothing::round_corners(&mut lg);
    smoothing::add_end_points(&mut lg);
    probe(Stage::Graph("tiles", &lg));

    // Edge tiles.
    let mut infos = Vec::new();
    for e in &lg.edges {
        let et = types.edge(files, &e.edge_type)?;
        infos.push(EdgeInfo {
            path: e.edge_type.clone(),
            size: et.size as u32,
            grounds: et.grounds.clone(),
            height: et.height_change,
            centred: et.centred,
            hashed: et.hashed,
        });
    }
    let before = map.pieces.len();
    let tiled = EdgeTiler::new(&mut lg, &mut map, &master.tiles, infos).run(rng).map_err(data);
    probe(Stage::Edges(&map, before));
    tiled?;

    // Corner lattice passes, fill, de-repetition.
    for p in &mut map.lattice {
        if p.flags & 1 != 0 { p.flags |= 4 } else { p.flags &= !4 }
    }
    probe(Stage::Lattice("borders", &map));
    lattice::border_floods(&mut map, &master.tiles);
    probe(Stage::Lattice("flood", &map));
    lattice::mark_unwalkable(&mut map, &master.tiles);
    probe(Stage::Lattice("heights", &map));
    let default_ground = graph.ground_types.iter().find(|g| !g.is_empty()).cloned();
    lattice::settle(&mut map, default_ground.as_deref());
    probe(Stage::Lattice("92ce0", &map));
    for p in &mut map.lattice {
        p.flags &= !4;
    }
    // Overlays before and after the fill (`0x141fa1a70`).
    let mut overlays = match rel("Overlays") {
        Some(toy) => {
            let rects: Vec<crate::overlay::RoomRect> = placed
                .iter()
                .map(|p| {
                    let feature = p.tile.is_none() && !set.rooms[p.room].room.arm.tag.is_empty();
                    crate::overlay::RoomRect {
                        rect: (p.pos.0, p.pos.1, p.pos.0 + p.size.0 as i32, p.pos.1 + p.size.1 as i32),
                        feature,
                        described: !graph.nodes[p.node].tags.is_empty(),
                    }
                })
                .collect();
            Some(crate::overlay::Overlays::new(files, &toy, &map, &mut master, &rects)?)
        }
        None => None,
    };
    if let Some(o) = &mut overlays {
        o.run(1, &mut map, &mut master, &mut types, files, rng)?;
    }
    probe(Stage::Lattice("fill", &map));
    let fill_rooms = fill_pass(&mut map, &mut master, files, rng)?;
    if let Some(o) = &mut overlays {
        o.run(2, &mut map, &mut master, &mut types, files, rng)?;
    }
    let mut rooms: Vec<RoomPlacement> = placed
        .iter()
        .filter(|p| p.tile.is_none())
        .map(|p| RoomPlacement { room: &set.rooms[p.room].room, origin: p.pos, orientation: p.orientation })
        .collect();
    rooms.extend(fill_rooms.iter().map(|r| RoomPlacement { room: &r.room, origin: r.origin, orientation: r.orientation }));
    if let Some(o) = &overlays {
        rooms.extend(o.placed.iter().map(|&(i, origin, orientation)| RoomPlacement { room: o.room(i), origin, orientation }));
    }
    derepeat_pass(&mut map, &master, &rooms, rng);

    // Outer buffer.
    let edge_types = master.edge_type_table(files)?;
    outer_buffer(&map, margins, tsi.get("OuterGroundType").map(String::as_str), false, &mut master, files, &edge_types, rng)
}
