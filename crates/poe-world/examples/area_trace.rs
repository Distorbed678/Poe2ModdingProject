//! Generates an area and lines its RNG draws up with a client generator trace (an area export
//! of the DLL with `POE_HOOK_GEN_TRACE=1`): `cargo run --release -p poe-world --example
//! area_trace -- <area> <seed> <export dir> [forced topology, 1-based | all]`. Prints our result or error, then the first draw
//! that differs with the trace lines around it.

use std::path::Path;

use poe_world::rng::TerrainRng;

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
        let n = poe_world::area::topologies(&gd, &area)?.len() as u32;
        for f in 1..=n {
            let mut rng = TerrainRng::new(seed);
            rng.log = Some(Vec::new());
            let r = poe_world::area::generate_with(&gd, &area, &mut rng, Some(f));
            let log = rng.log.take().unwrap_or_default();
            let k = log.iter().zip(&trace).take_while(|(o, t)| o.0 == t.2 && o.1 == t.3).count();
            println!("topology {f}: {k} draws in step, {}", match r { Ok(g) => format!("tile hash {}", g.map.tile_hash()), Err(e) => format!("error: {e}") });
        }
        return Ok(());
    }
    let forced = force.map(|f| f.parse::<u32>()).transpose()?;
    let mut rng = TerrainRng::new(seed);
    rng.log = Some(Vec::new());
    match poe_world::area::generate_with(&gd, &area, &mut rng, forced) {
        Ok(g) => println!("{} {}x{}: tile hash {}", g.graph, g.map.width, g.map.height, g.map.tile_hash()),
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
