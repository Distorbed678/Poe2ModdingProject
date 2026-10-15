//! Generates the Riverbank (`G1_1`) step by step and compares the RNG draws with a client
//! generator trace: `cargo run -p poe-world --example river_gen -- <export dir> <seed>`
//! (the export dir holds `generator_trace.tsv` and the layout).

use std::path::Path;

use poe_data::{GameData, Value};
use poe_world::{
    generate::{Files, Master, TileMap, room_pass_oriented},
    graph::Graph,
    layout::{self, JitterNode},
    rng::{TerrainRng, lcg_index},
    rooms::{PlacedGraph, PlacedRoom, RoomSet, TerrainTypes, place_node, tsi_entries},
    routing::{EXIT_CELL, RouteEdge, Router, SIDE_DIR},
};

/// The trace's bounded draws after the real seed: (call site, n, result).
fn trace_draws(path: &Path) -> Result<Vec<(String, u32, u32)>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let lines: Vec<Vec<&str>> = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split('\t').collect())
        .collect();
    let start = lines
        .iter()
        .position(|l| l.len() > 3 && l[1] == "seed" && !l[3].starts_with("0 "))
        .ok_or("no real seed in the trace")?;
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate().skip(start) {
        if l.len() < 4 || (l[1] != "below" && l[1] != "range") {
            continue;
        }
        let n: u32 = if l[1] == "below" {
            l[3].parse()?
        } else {
            let mut ab = l[3].split(' ').map(|v| v.parse::<u32>().unwrap_or(0));
            let (a, b) = (ab.next().unwrap_or(0), ab.next().unwrap_or(0));
            b.wrapping_sub(a).wrapping_add(1)
        };
        // The result is on the next `uniform` line (none when n <= 1).
        let result = lines[i + 1..]
            .iter()
            .take(3)
            .find(|x| x.len() > 4 && x[1] == "uniform")
            .and_then(|x| x[4].parse().ok())
            .unwrap_or(0);
        let result = if n <= 1 { 0 } else { result };
        out.push((l[2].split(',').next().unwrap_or("").to_string(), n, result));
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let export_dir = args.next().ok_or("usage: river_gen <export dir> <seed>")?;
    let seed: u32 = args.next().ok_or("seed")?.parse()?;
    let trace = trace_draws(&Path::new(&export_dir).join("generator_trace.tsv"))?;
    let data = GameData::open(
        Path::new("GameFiles"),
        Path::new("Tools/dat-schema/schema-poe2.min.json"),
    )?;
    let area = data.table("WorldAreas")?;
    let area = area.find_by_str("Id", "G1_1")?.ok_or("no G1_1")?;
    let topologies = data.table("Topologies")?;
    let rows: Vec<usize> = match area.get("Topologies")? {
        Value::List(l) => l.iter().filter_map(Value::as_row).collect(),
        _ => Vec::new(),
    };

    let mut rng = TerrainRng::new(seed);
    rng.log = Some(Vec::new());
    let sub = rng.below(u32::MAX);
    let pick = lcg_index(sub, rows.len() as u32) as usize;
    let dgr = topologies.row(rows[pick]).ok_or("bad row")?.str("DGRFile")?;
    println!("topology {pick}: {dgr}");
    let graph = Graph::parse_bytes(&Files::read(data.files(), &dgr)?)?;

    // Edge existence (`0x141feeaf0`): exist % 0 removes the edge without a draw.
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
    layout::jitter(&mut nodes, &mut points, tiles, &mut rng)?;
    let mut it = points.iter().copied();
    let edge_points: Vec<Vec<(i32, i32)>> = graph
        .edges
        .iter()
        .zip(&removed)
        .map(|(e, r)| if *r { Vec::new() } else { e.points.iter().map(|_| it.next().unwrap()).collect() })
        .collect();
    let jitter_draws = rng.log.as_ref().map_or(0, Vec::len);
    println!("jitter done after {jitter_draws} draws");

    // Master file: tile set, fill tiles, room set.
    let tsi_path = graph.master_file.clone();
    let tsi = tsi_entries(&Files::read(data.files(), &tsi_path)?);
    let dir = tsi_path.rsplit_once('/').map_or("", |d| d.0);
    let rel = |k: &str| tsi.get(k).map(|v| format!("{dir}/{v}"));
    let files = data.files();
    let mut master = Master::load(
        files,
        &rel("TileSet").ok_or("no TileSet")?,
        &rel("FillTiles").ok_or("no FillTiles")?,
    )?;
    let set = RoomSet::load(files, &rel("RoomSet").ok_or("no RoomSet")?)?;
    let mut placed_graph = PlacedGraph {
        graph: &graph,
        nodes: nodes.iter().map(|n| n.pos).collect(),
        points: edge_points,
        removed,
    };
    let mut types = TerrainTypes::default();
    let mut placed: Vec<PlacedRoom> = Vec::new();
    for n in 0..graph.nodes.len() {
        if graph.nodes[n].room_type.is_empty() {
            continue;
        }
        if std::env::var("DEBUG_NODE").ok().and_then(|v| v.parse::<usize>().ok()) == Some(n) {
            let (sides, key) = poe_world::rooms::node_key(&placed_graph, n, &mut types, files)?;
            println!("node {n} sides {sides:?} key {:?}", key.edges);
            for r in &set.rooms {
                if r.tag.eq_ignore_ascii_case(&graph.nodes[n].room_type) {
                    for o in 0..8 {
                        println!("  {} o{o} {:?}", r.path, r.key.oriented(o).edges);
                    }
                }
            }
        }
        let p = place_node(&set, &master.tiles, &placed_graph, n, tiles, &placed, &mut types, files, &mut rng)?;
        println!(
            "node {n} {:?}: {} o{} at {:?}",
            graph.nodes[n].room_type, set.rooms[p.room].path, p.orientation, p.pos
        );
        placed.push(p);
    }
    let fixed: Vec<(bool, bool)> = graph
        .nodes
        .iter()
        .map(|n| {
            (
                n.values.get(9).copied().unwrap_or(0) != 0,
                n.values.get(10).copied().unwrap_or(0) != 0,
            )
        })
        .collect();
    poe_world::rooms::repel(&mut placed, &mut placed_graph, &fixed, tiles, &mut types, files)?;
    for p in &placed {
        println!("after repel: node {} at {:?}", p.node, p.pos);
    }
    let mut map = TileMap::new(graph.width, graph.height);
    map.ground_flags = master.ground_flag_table(files)?;
    map.edge_flags = master.edge_flag_table(files)?;
    for p in &placed {
        room_pass_oriented(&mut map, &master, &set.rooms[p.room].room, p.pos, p.orientation, &mut rng)?;
    }
    println!("room passes: {} pieces", map.pieces.len());

    // Room-pass placements against the trace's `place` lines (region coordinates).
    let trace_places: Vec<(i32, i32, u8, String)> = std::fs::read_to_string(Path::new(&export_dir).join("generator_trace.tsv"))?
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() > 4 && f[1] == "place").then(|| {
                let a: Vec<&str> = f[3].split(' ').collect();
                (
                    a[0].parse().unwrap_or(0),
                    a[1].parse().unwrap_or(0),
                    a[2].trim_start_matches('o').parse().unwrap_or(0),
                    f[4].to_ascii_lowercase(),
                )
            })
        })
        .collect();
    let mut bad = 0;
    for (i, p) in map.pieces.iter().enumerate() {
        let t = &trace_places[i];
        if (p.x, p.y, p.orientation) != (t.0, t.1, t.2) || !t.3.ends_with(&p.path.to_ascii_lowercase().replace('\\', "/")) {
            bad += 1;
            if bad <= 5 {
                println!("piece {i}: ours {} {} o{} {} client {:?}", p.x, p.y, p.orientation, p.path, t);
            }
        }
    }
    println!("room-pass pieces differing from the trace: {bad} of {}", map.pieces.len());

    // Edge ends at the rooms' exits (`0x1420c5750`, `0x1420c5b50`).
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
            margin: if et.virtual_edge {
                e.tail.get(1).copied().unwrap_or(0) / 23 + 3
            } else {
                et.size as i32 + 2
            },
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
            let end = if et.virtual_edge {
                let mut v1 = at(p.key.virtual_exits[side]);
                let mut v2 = at(p.key.exits[side]);
                let which = (!from) as usize;
                // `0x1420c6860`: order the pair by the side of the edge they fall on.
                let n = placed_graph.nodes[node];
                let q = if from {
                    r.points.first().copied().unwrap_or(r.ends[1])
                } else {
                    r.points.last().copied().unwrap_or(r.ends[0])
                };
                let mut c = (v1.1 - q.1) * (n.0 - q.0) - (v1.0 - q.0) * (n.1 - q.1);
                let c2 = (n.0 - q.0) * (v2.1 - q.1) - (n.1 - q.1) * (v2.0 - q.0);
                if (c2 < 0) == (c < 0) {
                    c = ((v1.0 + v2.0) / 2 - n.0) * (v1.1 - n.1) - (v1.0 - n.0) * ((v1.1 + v2.1) / 2 - n.1);
                }
                if c < 0 {
                    std::mem::swap(&mut v1, &mut v2);
                }
                let m = ((v1.0 + v2.0) / 2, (v2.1 + v1.1) / 2);
                // `0x1420c5b50`: and by the side of the exit direction.
                let d = SIDE_DIR[side];
                if (v1.1 - (d.1 + m.1)) * (m.0 - (d.0 + m.0)) - (v1.0 - (d.0 + m.0)) * (m.1 - (d.1 + m.1)) < 0 {
                    std::mem::swap(&mut v1, &mut v2);
                }
                vends[route_index[ei]][which] = Some([v1, v2]);
                m
            } else {
                at(p.key.exits[side])
            };
            let which = (!from) as usize;
            r.ends[which] = end;
            r.offsets[which] = SIDE_DIR[side];
        }
    }
    let mut router = Router::new(tiles.0, tiles.1, &map, &mut rng);
    for e in &route {
        router.reserve(e);
    }
    let grids: Vec<Vec<(i32, i32, bool)>> = std::fs::read_to_string(Path::new(&export_dir).join("generator_trace.tsv"))?
        .lines()
        .filter(|l| l.split('\t').nth(1) == Some("route_grid"))
        .map(|l| {
            l.split('\t')
                .nth(4)
                .unwrap_or("")
                .split(' ')
                .map(|c| {
                    let v: Vec<i32> = c.split(':').map(|x| x.parse().unwrap_or(0)).collect();
                    (v[0], v[1], v[2] != 0)
                })
                .collect()
        })
        .collect();
    for k in 0..route.len() {
        if let Some(client) = grids.get(k) {
            let ours = router.snapshot();
            let diff: Vec<usize> = (0..ours.len()).filter(|&i| ours[i] != client[i]).collect();
            if !diff.is_empty() {
                println!("grid before edge {k}: {} tiles differ", diff.len());
            }
        }
        let ok = router.route(&mut route[k], &mut rng);
        if !ok {
            println!("edge {k} failed to route (retries not handled here)");
        }
    }
    if false {
    match poe_world::routing::route_reserved(&mut router, &mut route, &mut rng) {
        Ok(()) => {
            println!("routed {} edges", route.len());
            for (i, r) in route.iter().enumerate() {
                println!("route {i}: {:?} off {:?} -> {:?}", r.ends, r.offsets, r.points);
            }
        }
        Err(e) => println!("routing: {e}"),
    }
    }

    // The layout graph in the client's numbering (`0x141ef2800` splits room nodes per exit).
    let mut lg = poe_world::smoothing::LayoutGraph::default();
    for (i, _) in graph.nodes.iter().enumerate() {
        let mut slots = [None; 4];
        for (k, &ei) in graph.nodes[i].edges.iter().take(4).enumerate() {
            let ei = ei as usize;
            if route_index.get(ei).copied().unwrap_or(usize::MAX) != usize::MAX {
                slots[k] = Some(route_index[ei]);
            }
        }
        // `0x1420c7400`: next to a virtual edge in the topology graph.
        let mut virtual_end = false;
        for &ei in &graph.nodes[i].edges {
            virtual_end |= types.edge(files, &graph.edges[ei as usize].edge_type)?.virtual_edge;
        }
        let fixed = placed.iter().any(|p| p.node == i);
        lg.nodes.push(poe_world::smoothing::LNode { pos: placed_graph.nodes[i], slots, fixed, virtual_end });
    }
    for (gi, e) in graph.edges.iter().enumerate() {
        let ri = route_index[gi];
        if ri == usize::MAX {
            continue;
        }
        let et = types.edge(files, &e.edge_type)?.clone();
        lg.edges.push(poe_world::smoothing::LEdge {
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
                lg.nodes.push(poe_world::smoothing::LNode { pos, slots: [Some(e), None, None, None], fixed: true, virtual_end: false });
                if which == 0 { lg.edges[e].from = nn } else { lg.edges[e].to = nn }
                lg.nodes[node].slots[k] = None;
            }
        }
    }
    poe_world::smoothing::smooth(&mut lg);
    // Compare with the client's graph after smoothing.
    let dump: Vec<Vec<String>> = std::fs::read_to_string(Path::new(&export_dir).join("generator_trace.tsv"))?
        .lines()
        .filter(|l| l.split('\t').nth(1) == Some("graph_after_smooth1"))
        .map(|l| l.split('\t').map(String::from).collect())
        .collect();
    let (mut same, mut diff) = (0, 0);
    for l in &dump {
        let args = &l[3];
        let pts = l.get(4).cloned().unwrap_or_default();
        if l[2] == "node" {
            let mut it = args.split_whitespace();
            let idx: usize = it.next().unwrap().parse()?;
            let pos = it.next().unwrap();
            let ours = lg.nodes.get(idx).map(|n| format!("{},{}", n.pos.0, n.pos.1)).unwrap_or_default();
            if ours == pos { same += 1 } else { diff += 1; println!("node {idx}: ours {ours} client {pos}"); }
        } else {
            let idx: usize = args.split_whitespace().next().unwrap().parse()?;
            let ours = lg.edges.get(idx).map(|e| e.points.iter().map(|p| format!("{},{}", p.0, p.1)).collect::<Vec<_>>().join(" ")).unwrap_or_default();
            if ours == pts { same += 1 } else { diff += 1; println!("edge {idx}: ours [{ours}]\n  client [{pts}]"); }
        }
    }
    println!("graph after smoothing: {same} same, {diff} different");

    // Virtual edges split into their two banks.
    let mut split_types = std::collections::HashMap::new();
    let mut want: Vec<String> = lg.edges.iter().map(|e| e.edge_type.clone()).collect();
    while let Some(t) = want.pop() {
        if split_types.contains_key(&t) {
            continue;
        }
        let et = types.edge(files, &t)?.clone();
        for s in &et.splits {
            if !s.0.is_empty() {
                want.push(s.0.clone());
            }
        }
        split_types.insert(
            t,
            poe_world::split::SplitType {
                virtual_edge: et.virtual_edge,
                grounds: et.grounds.clone(),
                height_change: et.height_change,
                splits: et.splits.clone(),
            },
        );
    }
    let lookup = |t: &str| split_types.get(&t.to_ascii_lowercase()).cloned().unwrap_or_default();
    poe_world::split::split_virtual(&mut lg, &lookup, (tiles.0 * 23, tiles.1 * 23), &mut rng);
    let dump: Vec<Vec<String>> = std::fs::read_to_string(Path::new(&export_dir).join("generator_trace.tsv"))?
        .lines()
        .filter(|l| l.split('\t').nth(1) == Some("graph_after_split"))
        .map(|l| l.split('\t').map(String::from).collect())
        .collect();
    let (mut same, mut diff) = (0, 0);
    for l in &dump {
        let args = &l[3];
        let pts = l.get(4).cloned().unwrap_or_default();
        let mut it = args.split_whitespace();
        let idx: usize = it.next().unwrap().parse()?;
        if l[2] == "node" {
            let pos = it.next().unwrap();
            let ours = lg.nodes.get(idx).map(|n| format!("{},{}", n.pos.0, n.pos.1)).unwrap_or_default();
            if ours == pos { same += 1 } else { diff += 1; println!("split node {idx}: ours {ours} client {pos}"); }
        } else {
            let ends = it.next().unwrap();
            let et = it.next().unwrap();
            let ours = lg.edges.get(idx).map(|e| {
                format!("{}->{} {} [{}]", e.from, e.to, e.edge_type.rsplit('/').next().unwrap_or(""),
                    e.points.iter().map(|p| format!("{},{}", p.0, p.1)).collect::<Vec<_>>().join(" "))
            }).unwrap_or_default();
            let theirs = format!("{ends} {} [{pts}]", et.to_ascii_lowercase());
            if ours == theirs { same += 1 } else { diff += 1; println!("split edge {idx}: ours {ours}\n  client {theirs}"); }
        }
    }
    println!("graph after split: {same} same, {diff} different");

    // Up to tile placement: rounding, border nodes (`0x1420cade0`), slot compaction, rounding,
    // end points (`0x1420b6660`).
    poe_world::smoothing::round_corners(&mut lg);
    for n in 0..graph.nodes.len() {
        if !graph.nodes[n].room_type.is_empty() {
            continue;
        }
        let edges: Vec<usize> = lg.nodes[n].slots.iter().flatten().copied().collect();
        if edges.len() != 1 {
            continue;
        }
        let e = edges[0];
        let et = types.edge(files, &lg.edges[e].edge_type)?.clone();
        let b = et.size as i32;
        let (x, y) = lg.nodes[n].pos;
        let (tx, ty) = ((x / 23).min(tiles.0 - 1), (y / 23).min(tiles.1 - 1));
        let near = !(b <= tx && b <= ty && tx < tiles.0 - b && ty < tiles.1 - b);
        if et.virtual_edge && !near {
            continue;
        }
        if !near {
            println!("node {n}: a dead end inside the map (tile node) is not ported yet");
            continue;
        }
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
    poe_world::smoothing::compact_slots(&mut lg);
    poe_world::smoothing::round_corners(&mut lg);
    poe_world::smoothing::add_end_points(&mut lg);
    let dump: Vec<Vec<String>> = std::fs::read_to_string(Path::new(&export_dir).join("generator_trace.tsv"))?
        .lines()
        .filter(|l| l.split('\t').nth(1) == Some("graph_before_tiles"))
        .map(|l| l.split('\t').map(String::from).collect())
        .collect();
    let (mut same, mut diff) = (0, 0);
    for l in &dump {
        let args = &l[3];
        let pts = l.get(4).cloned().unwrap_or_default();
        let mut it = args.split_whitespace();
        let idx: usize = it.next().unwrap().parse()?;
        if l[2] == "node" {
            let pos = it.next().unwrap();
            let flags = args.split("flags [").nth(1).and_then(|r| r.split(']').next()).unwrap_or("");
            let theirs = format!("{pos} {}", flags.split(", ").take(2).collect::<Vec<_>>().join(","));
            let ours = lg.nodes.get(idx).map(|n| format!("{},{} {},{}", n.pos.0, n.pos.1, n.fixed as u8, n.virtual_end as u8)).unwrap_or_default();
            if ours == theirs { same += 1 } else { diff += 1; println!("tiles node {idx}: ours {ours} client {theirs}"); }
        } else {
            let ends = it.next().unwrap();
            let et = it.next().unwrap();
            let off = args.split(" off [").nth(1).and_then(|r| r.split(']').next()).unwrap_or("");
            let ours = lg.edges.get(idx).map(|e| {
                format!("{}->{} {} off {}, {}, {}, {} [{}]", e.from, e.to, e.edge_type.rsplit('/').next().unwrap_or(""),
                    e.offsets[0].0, e.offsets[0].1, e.offsets[1].0, e.offsets[1].1,
                    e.points.iter().map(|p| format!("{},{}", p.0, p.1)).collect::<Vec<_>>().join(" "))
            }).unwrap_or_default();
            let theirs = format!("{ends} {} off {off} [{pts}]", et.to_ascii_lowercase());
            if ours == theirs { same += 1 } else { diff += 1; println!("tiles edge {idx}: ours {ours}\n  client {theirs}"); }
        }
    }
    println!("graph before tiles: {same} same, {diff} different");

    // Edge tiles (`0x1420b6950`): chains into tile paths; compare the path tiles with the
    // client's edge tile placements (`place` lines at 0x1420b85bf, in path order).
    let mut infos = Vec::new();
    for e in &lg.edges {
        let et = types.edge(files, &e.edge_type)?;
        infos.push(poe_world::edge_tiles::EdgeInfo {
            path: e.edge_type.clone(),
            size: et.size as u32,
            grounds: et.grounds.clone(),
            height: et.height_change,
            centred: et.centred,
            hashed: false,
        });
    }
    let pieces_before = map.pieces.len();
    let mut tiler = poe_world::edge_tiles::EdgeTiler::new(&mut lg, &mut map, &master.tiles, infos);
    let result = tiler.run(&mut rng);
    let chains = tiler.order.len();
    drop(tiler);
    if let Err(e) = result {
        println!("edge tiles: {e}");
    }
    let theirs: Vec<(String, String)> = std::fs::read_to_string(Path::new(&export_dir).join("generator_trace.tsv"))?
        .lines()
        .filter(|l| l.split('\t').nth(1) == Some("place") && l.split('\t').nth(2) == Some("1420b85bf"))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[3].to_string(), f.get(4).unwrap_or(&"").to_ascii_lowercase())
        })
        .collect();
    let keys: Vec<String> = std::fs::read_to_string(Path::new(&export_dir).join("generator_trace.tsv"))?
        .lines()
        .filter(|l| l.split('\t').nth(1) == Some("tile_key") && l.split('\t').nth(2) == Some("1420b8564"))
        .map(|l| l.split('\t').skip(3).collect::<Vec<_>>().join(" | "))
        .collect();
    let ours: Vec<(String, String)> = map.pieces[pieces_before..]
        .iter()
        .map(|p| (format!("{} {} o{} h0", p.x, p.y, p.orientation), p.path.to_ascii_lowercase()))
        .collect();
    let same = ours.iter().zip(&theirs).take_while(|(a, b)| a == b).count();
    println!("edge tiles: {} placed ({chains} paths), client {}; first {same} identical", ours.len(), theirs.len());
    if same < theirs.len() {
        println!("  ours   {:?}", ours.get(same));
        println!("  client {:?}", theirs.get(same));
        println!("  client key {}", keys.get(same).cloned().unwrap_or_default());
    }

    // Corner lattice after the edge tiles against the client's `lattice borders` snapshot.
    // The region driver copies flag 1 to flag 4 before the post-tile passes.
    for p in &mut map.lattice {
        if p.flags & 1 != 0 { p.flags |= 4 } else { p.flags &= !4 }
    }
    let lattice_point = |p: &poe_world::generate::LatticePoint| (p.ground.clone(), p.height, p.flags);
    compare_lattice(&map, &export_dir, "borders", lattice_point)?;
    // Border floods (`0x1420b94d0`).
    poe_world::lattice::border_floods(&mut map, &master.tiles);
    compare_lattice(&map, &export_dir, "flood", lattice_point)?;
    // Unwalkable corners (`0x141fe0fd0`; the height fix-ups of `0x1420c0a60` change nothing
    // with all heights 0).
    poe_world::lattice::mark_unwalkable(&mut map, &master.tiles);
    compare_lattice(&map, &export_dir, "heights", lattice_point)?;
    poe_world::lattice::settle(&mut map, None);
    compare_lattice(&map, &export_dir, "92ce0", lattice_point)?;
    // The driver clears flag 4 again; `0x141f92ce0`, `0x1420bdae0` and `0x1420be360` change
    // nothing in the Riverbank.
    for p in &mut map.lattice {
        p.flags &= !4;
    }
    compare_lattice(&map, &export_dir, "fill", lattice_point)?;

    // Fill pass (`0x141f8f930`).
    let pieces_before_fill = map.pieces.len();
    let fill_rooms = match poe_world::fill::fill_pass(&mut map, &mut master, files, &mut rng) {
        Ok(rooms) => {
            println!("fill pass: {} pieces, {} fill rooms", map.pieces.len() - pieces_before_fill, rooms.len());
            rooms
        }
        Err(e) => {
            println!("fill pass: {e} (after {} pieces)", map.pieces.len() - pieces_before_fill);
            Vec::new()
        }
    };
    // De-repetition (`0x141fdd7c0`): node rooms, then fill rooms.
    use poe_world::generate::RoomPlacement;
    let mut room_list: Vec<RoomPlacement> = placed
        .iter()
        .map(|p| RoomPlacement { room: &set.rooms[p.room].room, origin: p.pos, orientation: p.orientation })
        .collect();
    room_list.extend(fill_rooms.iter().map(|r| RoomPlacement { room: &r.room, origin: r.origin, orientation: r.orientation }));
    let pieces_before_derepeat = map.pieces.len();
    poe_world::generate::derepeat_pass(&mut map, &master, &room_list, &mut rng);
    println!("de-repetition: {} re-picks", map.pieces.len() - pieces_before_derepeat);

    // Outer buffer (`0x141fd8360`) and the final layout against the client export.
    let tsi_text = poe_world::text_utf16(&Files::read(files, &tsi_path)?);
    let margins = poe_world::outer_buffer::Margins::parse(&tsi_text);
    let edge_types = master.edge_type_table(files)?;
    match poe_world::outer_buffer::outer_buffer(&map, margins, &mut master, files, &edge_types, &mut rng) {
        Ok(full) => {
            let export = poe_world::export::AreaExport::load(Path::new(&export_dir))?;
            println!("map {}x{}: tile hash ours {} client {}", full.width, full.height, full.tile_hash(), export.meta("tile_hash").unwrap_or("?"));
            let (mut same, mut diff) = (0, 0);
            for y in 0..full.height {
                for x in 0..full.width {
                    let (a, b) = (full.get(x, y), export.tile(x, y));
                    let ok = match (a, b) {
                        (Some(a), Some(b)) => a.path.eq_ignore_ascii_case(&b.path) && a.orientation == b.orientation && a.sub == b.sub,
                        (None, None) => true,
                        _ => false,
                    };
                    if ok {
                        same += 1;
                    } else {
                        diff += 1;
                        if diff <= 8 {
                            println!("  ({x},{y}) ours {:?} client {:?}", a.map(|c| (&c.path, c.orientation, c.sub)), b.map(|c| (&c.path, c.orientation, c.sub)));
                        }
                    }
                }
            }
            println!("cells: {same} identical, {diff} different");
        }
        Err(e) => println!("outer buffer: {e}"),
    }

    let log = rng.log.take().unwrap_or_default();
    for (i, (ours, theirs)) in log.iter().zip(&trace).enumerate() {
        if ours.0 != theirs.1 || ours.1 != theirs.2 {
            println!(
                "first divergence at draw {i}: ours n={} r={}, client {} n={} r={}",
                ours.0, ours.1, theirs.0, theirs.1, theirs.2
            );
            return Ok(());
        }
    }
    println!(
        "{} draws in step with the client (next client draw: {:?})",
        log.len(),
        trace.get(log.len())
    );
    Ok(())
}

/// Compares the map's corner lattice with a client `lattice` snapshot (region coordinates).
fn compare_lattice(
    map: &poe_world::generate::TileMap,
    export_dir: &str,
    step: &str,
    ours: impl Fn(&poe_world::generate::LatticePoint) -> (Option<String>, i8, u8),
) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(Path::new(export_dir).join("generator_trace.tsv"))?;
    let mut grounds: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<(usize, i8, u8)>> = Vec::new();
    for l in text.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 4 || f[1] != "lattice" || f[2] != step {
            continue;
        }
        if f[3].starts_with("view") {
            grounds = f.get(4).unwrap_or(&"").split(',').map(|g| g.to_ascii_lowercase()).collect();
        } else {
            rows.push(f[4].split(' ').map(|c| {
                let v: Vec<i32> = c.split('.').map(|x| x.parse().unwrap()).collect();
                (v[0] as usize, v[1] as i8, v[2] as u8)
            }).collect());
        }
    }
    let (mut same, mut diff) = (0, 0);
    let mut shown = 0;
    for (y, row) in rows.iter().enumerate() {
        for (x, &(g, h, fl)) in row.iter().enumerate() {
            let Some(i) = map.point_index(x as i32, y as i32) else { continue };
            let (og, oh, of) = ours(&map.lattice[i]);
            let og = og.map(|g| g.rsplit('/').next().unwrap().to_string()).unwrap_or("-".into());
            let tg = grounds.get(g).cloned().unwrap_or_default();
            if og == tg && oh == h && of == fl {
                same += 1;
            } else {
                diff += 1;
                if shown < 8 {
                    shown += 1;
                    println!("  lattice {step} ({x},{y}): ours {og} {oh} {of}, client {tg} {h} {fl}");
                }
            }
        }
    }
    println!("lattice before {step}: {same} same, {diff} different");
    Ok(())
}
