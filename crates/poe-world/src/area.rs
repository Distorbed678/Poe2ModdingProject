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
        return topologies_for(gd, area_id, 1);
    }
    Ok(any)
}

/// Generates area `area_id` with `seed` (`TerrainGen_Run` up to the tile hash).
pub fn generate(gd: &GameData, area_id: &str, seed: u32) -> Result<Generated, GenError> {
    generate_with(gd, area_id, &mut TerrainRng::new(seed), None)
}

/// [`generate`] with a caller's RNG (for draw logs) and optionally a forced topology
/// (`map_force_topology`, stat key 0xb92: 1-based index into [`topologies`]; `0x141ff90f0`
/// still draws the topology value).
///
/// Without one the client chooses by the world map (`0x141ff9280` → `0x141fa6440`: the
/// directions of the connected areas' map pins against each topology's direction list, three
/// passes from exact to adjacent) and only falls back to the index draw here when nothing
/// matches; that choice is not ported yet.
pub fn generate_with(gd: &GameData, area_id: &str, rng: &mut TerrainRng, forced: Option<u32>) -> Result<Generated, GenError> {
    let graphs = topologies(gd, area_id)?;
    if graphs.is_empty() {
        return Err(data(format!("{area_id}: no topologies")));
    }
    let files = gd.files();
    let sub = rng.below(u32::MAX);
    let index = match forced {
        Some(f) if f >= 1 && (f as usize) <= graphs.len() => f as usize - 1,
        _ => lcg_index(sub, graphs.len() as u32) as usize,
    };
    let dgr = graphs[index].clone();
    let graph = Graph::parse_bytes(&crate::generate::Files::read(files, &dgr)?).map_err(|e| data(format!("{dgr}: {e:?}")))?;
    let map = generate_graph(files, &graph, rng)?;
    Ok(Generated { graph: dgr, map, draws: rng.draws })
}

/// Everything after the topology choice, for one graph.
pub fn generate_graph(files: &dyn crate::generate::Files, graph: &Graph, rng: &mut TerrainRng) -> Result<TileMap, GenError> {
    // Edge existence (`0x141feeaf0`).
    let removed: Vec<bool> = graph
        .edges
        .iter()
        .map(|e| {
            let exist = e.values.first().copied().unwrap_or(100);
            !(exist != 0 && (rng.below(100) as i32) < exist)
        })
        .collect();
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

    // Rooms.
    let mut placed: Vec<PlacedRoom> = Vec::new();
    for n in 0..graph.nodes.len() {
        if graph.nodes[n].room_type.is_empty() {
            continue;
        }
        let p = place_node(&set, &master.tiles, &placed_graph, n, tiles, &placed, &mut types, files, rng)?;
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
                map.place(p.pos.0, p.pos.1, &def.path, &def.tdt, p.orientation, 0, None);
            }
            None => room_pass_oriented(&mut map, &master, &set.rooms[p.room].room, p.pos, p.orientation, rng)?,
        }
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
        lg.nodes.push(LNode { pos: placed_graph.nodes[i], slots, fixed, virtual_end });
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
                lg.nodes.push(LNode { pos, slots: [Some(e), None, None, None], fixed: true, virtual_end: false });
                if which == 0 { lg.edges[e].from = nn } else { lg.edges[e].to = nn }
                lg.nodes[node].slots[k] = None;
            }
        }
    }
    smoothing::smooth(&mut lg);

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

    // Rounding, border dead ends and node tiles (`0x1420cade0`), slot compaction, rounding,
    // end points.
    smoothing::round_corners(&mut lg);
    let mut tile_nodes = Vec::new();
    for n in 0..graph.nodes.len() {
        if !graph.nodes[n].room_type.is_empty() {
            continue;
        }
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
    node_tiles::place_node_tiles(&mut lg, graph, &tile_nodes, &master, &mut map, tiles, &mut types, files, rng)?;
    smoothing::compact_slots(&mut lg);
    smoothing::round_corners(&mut lg);
    smoothing::add_end_points(&mut lg);

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
            hashed: false,
        });
    }
    EdgeTiler::new(&mut lg, &mut map, &master.tiles, infos).run(rng).map_err(data)?;

    // Corner lattice passes, fill, de-repetition.
    for p in &mut map.lattice {
        if p.flags & 1 != 0 { p.flags |= 4 } else { p.flags &= !4 }
    }
    lattice::border_floods(&mut map, &master.tiles);
    lattice::mark_unwalkable(&mut map, &master.tiles);
    let default_ground = graph.ground_types.iter().find(|g| !g.is_empty()).cloned();
    lattice::settle(&mut map, default_ground.as_deref());
    for p in &mut map.lattice {
        p.flags &= !4;
    }
    let fill_rooms = fill_pass(&mut map, &mut master, files, rng)?;
    let mut rooms: Vec<RoomPlacement> = placed
        .iter()
        .filter(|p| p.tile.is_none())
        .map(|p| RoomPlacement { room: &set.rooms[p.room].room, origin: p.pos, orientation: p.orientation })
        .collect();
    rooms.extend(fill_rooms.iter().map(|r| RoomPlacement { room: &r.room, origin: r.origin, orientation: r.orientation }));
    derepeat_pass(&mut map, &master, &rooms, rng);

    // Outer buffer.
    let edge_types = master.edge_type_table(files)?;
    outer_buffer(&map, margins, &mut master, files, &edge_types, rng)
}
