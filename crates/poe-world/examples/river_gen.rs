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
    let master = Master::load(
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
        let p = place_node(&set, &placed_graph, n, tiles, &placed, &mut types, files, &mut rng)?;
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
                let a = at(p.key.virtual_exits[side]);
                let b = at(p.key.exits[side]);
                ((a.0 + b.0) / 2, (a.1 + b.1) / 2)
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
        lg.nodes.push(poe_world::smoothing::LNode { pos: placed_graph.nodes[i], slots });
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
                lg.nodes.push(poe_world::smoothing::LNode { pos, slots: [Some(e), None, None, None] });
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
