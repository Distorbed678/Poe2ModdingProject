//! Generates an area and lines its RNG draws up with a client generator trace (an area export
//! of the DLL with `POE_HOOK_GEN_TRACE=1`): `cargo run --release -p poe-world --example
//! area_trace -- <area> <seed> <export dir> [forced topology, 1-based[:orientation] | all]`. Prints our result or error, then the first draw
//! that differs with the trace lines around it.

use std::path::Path;

use poe_world::{area::{Forced, Stage}, generate::TileMap, rng::TerrainRng, smoothing::LayoutGraph};

/// Compares the edge tiles with the trace's `place` lines of `0x1420b85bf`.
fn compare_edges(map: &TileMap, before: usize, lines: &[Vec<String>]) {
    let places: Vec<String> = lines.iter().filter(|f| f.len() > 4 && f[1] == "place" && f[2] == "1420b85bf").map(|f| format!("{} {}", f[3], f[4].to_ascii_lowercase())).collect();
    let ours: Vec<String> = map.pieces[before..].iter().map(|p| format!("{} {} o{} h0 {}", p.x, p.y, p.orientation, p.path.to_ascii_lowercase())).collect();
    let same = ours.iter().zip(&places).take_while(|(a, b)| a == b).count();
    println!("edge tiles: ours {}, client {}, first {same} identical", ours.len(), places.len());
    let extra: usize = std::env::var("POE_EDGE_SHOW").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    for k in same..(same + extra).min(ours.len().max(places.len())) {
        println!("  {k}: ours {:?}\n     client {:?}", ours.get(k), places.get(k));
    }
}

/// Compares the final map with the client's `layout.tsv` (tile, sub-tile, height,
/// orientation, offset per cell).
fn compare_layout(map: &TileMap, dir: &str) {
    let Ok(text) = std::fs::read_to_string(Path::new(dir).join("layout.tsv")) else { return };
    let (mut same, mut diff) = (0, 0);
    for l in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let (x, y): (u32, u32) = (f[0].parse().unwrap_or(0), f[1].parse().unwrap_or(0));
        let theirs = format!("{} s{} h{} o{} {}", f[2].to_ascii_lowercase(), f[3], f[4], f[5], f[6]);
        let ours = map.get(x, y).map(|c| format!("{} s{} h{} o{} {},{}", c.path.to_ascii_lowercase(), c.sub, c.height, c.orientation, c.offset.0, c.offset.1)).unwrap_or_else(|| " s0 h0 o0 0,0".into());
        if ours == theirs {
            same += 1;
        } else {
            diff += 1;
            if diff <= 8 {
                println!("  cell {x},{y}: ours {ours}\n           client {theirs}");
            }
        }
    }
    println!("layout: {same} cells same, {diff} different");
}

/// Compares the corner lattice with the client's `lattice <step>` snapshot (ground, height,
/// flags per corner, region coordinates).
fn compare_lattice(step: &str, map: &TileMap, lines: &[Vec<String>]) {
    let mut grounds: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<(usize, i8, u8)>> = Vec::new();
    for f in lines.iter().filter(|f| f.len() > 4 && f[1] == "lattice" && f[2] == step) {
        if f[3].starts_with("view") {
            grounds = f[4].split(',').map(|g| g.to_ascii_lowercase()).collect();
            rows.clear();
        } else {
            rows.push(f[4].split(' ').map(|c| {
                let v: Vec<i32> = c.split('.').map(|x| x.parse().unwrap_or(0)).collect();
                (v[0] as usize, v[1] as i8, v[2] as u8)
            }).collect());
        }
    }
    if rows.is_empty() {
        return;
    }
    let (mut same, mut diff) = (0, 0);
    for (y, row) in rows.iter().enumerate() {
        for (x, &(g, h, fl)) in row.iter().enumerate() {
            let Some(i) = map.point_index(x as i32, y as i32) else { continue };
            let p = &map.lattice[i];
            let og = p.ground.as_deref().map(|g| g.rsplit('/').next().unwrap().to_string()).unwrap_or("-".into());
            let tg = grounds.get(g).cloned().unwrap_or_default();
            if og == tg && p.height == h && p.flags == fl {
                same += 1;
            } else {
                diff += 1;
                if diff <= 6 {
                    println!("  lattice {step} ({x},{y}): ours {og} {} {:#x}, client {tg} {h} {fl:#x}", p.height, p.flags);
                }
            }
        }
    }
    println!("lattice before {step}: {same} same, {diff} different");
}

/// Compares the room-pass pieces with the trace's first `place` lines.
fn compare_rooms(map: &TileMap, lines: &[Vec<String>]) {
    let places: Vec<(i32, i32, u8, String)> = lines
        .iter()
        .filter(|f| f.len() > 4 && f[1] == "place")
        .map(|f| {
            let a: Vec<&str> = f[3].split(' ').collect();
            (a[0].parse().unwrap_or(0), a[1].parse().unwrap_or(0), a[2].trim_start_matches('o').parse().unwrap_or(0), f[4].to_ascii_lowercase())
        })
        .collect();
    let mut bad = 0;
    for (i, p) in map.pieces.iter().enumerate() {
        let Some(t) = places.get(i) else { break };
        if (p.x, p.y, p.orientation) != (t.0, t.1, t.2) || !t.3.ends_with(&p.path.to_ascii_lowercase()) {
            bad += 1;
            if bad <= 5 {
                println!("  room piece {i}: ours {} {} o{} {} client {:?}", p.x, p.y, p.orientation, p.path, t);
            }
        }
    }
    println!("room pass: {} pieces, {bad} differ", map.pieces.len());
}

/// Compares a layout graph with the client's dump of the same stage (positions, slots and
/// fixed/virtual flags of nodes; ends, type, offsets and points of edges).
fn compare_graph(stage: &str, lg: &LayoutGraph, lines: &[Vec<String>]) {
    let kind = match stage {
        "smooth" => "graph_after_smooth1",
        "split" => "graph_after_split",
        _ => "graph_before_tiles",
    };
    let (mut same, mut diff) = (0, 0);
    for l in lines.iter().filter(|l| l.len() > 3 && l[1] == kind) {
        let args = &l[3];
        let pts = l.get(4).cloned().unwrap_or_default();
        let mut it = args.split_whitespace();
        let Some(idx) = it.next().and_then(|v| v.parse::<usize>().ok()) else { continue };
        let (ours, theirs) = if l[2] == "node" {
            let pos = it.next().unwrap_or("");
            let flags = args.split("flags [").nth(1).and_then(|r| r.split(']').next()).unwrap_or("");
            let theirs = format!("{pos} {}", flags.split(", ").take(2).collect::<Vec<_>>().join(","));
            let ours = lg.nodes.get(idx).map(|n| format!("{},{} {},{}", n.pos.0, n.pos.1, n.fixed as u8, n.virtual_end as u8)).unwrap_or_default();
            (ours, theirs)
        } else {
            let ends = it.next().unwrap_or("");
            let et = it.next().unwrap_or("");
            let off = args.split(" off [").nth(1).and_then(|r| r.split(']').next()).unwrap_or("");
            let ours = lg.edges.get(idx).map(|e| {
                format!("{}->{} {} off {}, {}, {}, {} [{}]", e.from, e.to, e.edge_type.rsplit('/').next().unwrap_or(""),
                    e.offsets[0].0, e.offsets[0].1, e.offsets[1].0, e.offsets[1].1,
                    e.points.iter().map(|p| format!("{},{}", p.0, p.1)).collect::<Vec<_>>().join(" "))
            }).unwrap_or_default();
            (ours, format!("{ends} {} off {off} [{pts}]", et.to_ascii_lowercase()))
        };
        if ours == theirs {
            same += 1;
        } else {
            diff += 1;
            if diff <= 8 {
                println!("  {stage} {} {idx}: ours {ours}\n    client {theirs}", l[2]);
            }
        }
    }
    println!("graph {stage}: {same} same, {diff} different (ours {} nodes, {} edges)", lg.nodes.len(), lg.edges.len());
}

/// The trace's bounded draws after the real seed: (trace line, call site, n, result).
fn trace_draws(lines: &[Vec<String>]) -> Vec<(usize, String, u32, u32)> {
    let Some(start) = lines.iter().position(|l| l.len() > 3 && l[1] == "seed" && !l[3].starts_with("0 ")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate().skip(start) {
        if l.len() < 4 || (l[1] != "below" && l[1] != "range") {
            continue;
        }
        let n: u32 = if l[1] == "below" {
            l[3].parse().unwrap_or(0)
        } else {
            let mut ab = l[3].split(' ').map(|v| v.parse::<u32>().unwrap_or(0));
            let (a, b) = (ab.next().unwrap_or(0), ab.next().unwrap_or(0));
            b.wrapping_sub(a).wrapping_add(1)
        };
        let result = lines[i + 1..]
            .iter()
            .take(3)
            .find(|x| x.len() > 4 && x[1] == "uniform")
            .and_then(|x| x[4].parse().ok())
            .unwrap_or(0);
        let result = if n <= 1 { 0 } else { result };
        out.push((i, l[2].split(',').next().unwrap_or("").to_string(), n, result));
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: area_trace <area> <seed> <export dir>";
    let area = args.next().ok_or(usage)?;
    let seed: u32 = args.next().ok_or(usage)?.parse()?;
    let dir = args.next().ok_or(usage)?;
    let force = args.next();
    let gd = poe_data::GameData::open(Path::new("GameFiles"), Path::new("Tools/dat-schema/schema-poe2.min.json"))?;
    let text = std::fs::read_to_string(Path::new(&dir).join("generator_trace.tsv"))?;
    let lines: Vec<Vec<String>> = text.lines().filter(|l| !l.starts_with('#')).map(|l| l.split('\t').map(String::from).collect()).collect();
    let trace = trace_draws(&lines);
    if force.as_deref() == Some("all") {
        // Which topology the client used: the one staying in step longest.
        let entries = poe_world::area::topology_entries(&gd, &area, 1)?;
        for (f, (_, mask)) in (1..).zip(&entries) {
            for o in (0..8u8).filter(|o| mask & (1 << o) != 0) {
                let mut rng = TerrainRng::new(seed);
                rng.log = Some(Vec::new());
                let forced = Forced { topology: Some(f), orientation: Some(o) };
                let r = poe_world::area::generate_with(&gd, &area, &mut rng, Some(forced));
                let log = rng.log.take().unwrap_or_default();
                let k = log.iter().zip(&trace).take_while(|(o, t)| o.0 == t.2 && o.1 == t.3).count();
                println!("topology {f} o{o}: {k} draws in step, {}", match r { Ok(g) => format!("tile hash {}", g.map.tile_hash()), Err(e) => format!("error: {e}") });
            }
        }
        return Ok(());
    }
    // `<topology>[:<orientation>]`
    let forced = match force {
        Some(f) => {
            let mut it = f.split(':');
            let topology = it.next().map(|v| v.parse::<u32>()).transpose()?;
            let orientation = it.next().map(|v| v.parse::<u8>()).transpose()?;
            Some(Forced { topology, orientation })
        }
        None => None,
    };
    let mut rng = TerrainRng::new(seed);
    rng.log = Some(Vec::new());
    let mut probe = |stage: Stage| match stage {
        Stage::Graph(name, lg) => compare_graph(name, lg, &lines),
        Stage::Rooms(map) => compare_rooms(map, &lines),
        Stage::Edges(map, before) => compare_edges(map, before, &lines),
        Stage::Lattice(step, map) => compare_lattice(step, map, &lines),
    };
    match poe_world::area::generate_probed(&gd, &area, &mut rng, forced, &mut probe) {
        Ok(g) => {
            compare_layout(&g.map, &dir);
            println!("{} {}x{}: tile hash {}", g.graph, g.map.width, g.map.height, g.map.tile_hash());
        }
        Err(e) => println!("error: {e}"),
    }
    let log = rng.log.take().unwrap_or_default();
    match log.iter().zip(&trace).position(|(o, t)| o.0 != t.2 || o.1 != t.3) {
        Some(i) => {
            let (line, site, n, r) = &trace[i];
            println!("first divergence at draw {i}: ours n={} r={}, client {site} n={n} r={r} (trace line {line})", log[i].0, log[i].1);
            for l in &lines[line.saturating_sub(12)..(*line + 3).min(lines.len())] {
                let s = l.join("\t");
                println!("  {}", &s[..s.len().min(200)]);
            }
        }
        None => println!("{} draws in step (ours {}, client {})", log.len().min(trace.len()), log.len(), trace.len()),
    }
    Ok(())
}
