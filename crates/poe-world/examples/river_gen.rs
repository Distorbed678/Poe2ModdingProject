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
    let dir = args.next().ok_or("usage: river_gen <export dir> <seed>")?;
    let seed: u32 = args.next().ok_or("seed")?.parse()?;
    let trace = trace_draws(&Path::new(&dir).join("generator_trace.tsv"))?;
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
    let placed_graph = PlacedGraph {
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
    let mut map = TileMap::new(graph.width, graph.height);
    for p in &placed {
        room_pass_oriented(&mut map, &master, &set.rooms[p.room].room, p.pos, p.orientation, &mut rng)?;
    }
    println!("room passes: {} pieces", map.pieces.len());

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
