//! Checks the generator against tile-hash vectors (`area level seed tile_hash doodad_hash build`,
//! from `tools/re/clientlog_terrain.py`): `cargo run --release -p poe-world --example
//! area_vectors -- <vectors.tsv> [build] [area filter]`. Prints one line per vector and a summary
//! per area.

use std::{collections::BTreeMap, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: area_vectors <vectors.tsv> [build] [area]")?;
    let build = args.next();
    let area_filter = args.next();
    let gd = poe_data::GameData::open(Path::new("GameFiles"), Path::new("Tools/dat-schema/schema-poe2.min.json"))?;
    let mut per_area: BTreeMap<String, (u32, u32, u32, String)> = BTreeMap::new();
    for line in std::fs::read_to_string(&path)?.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 6 || build.as_deref().is_some_and(|b| b != "all" && b != f[5]) {
            continue;
        }
        if area_filter.as_deref().is_some_and(|a| a != f[0]) {
            continue;
        }
        let (area, seed, want) = (f[0], f[2].parse::<u32>()?, f[3]);
        let e = per_area.entry(area.to_string()).or_default();
        let result = std::panic::catch_unwind(|| poe_world::area::generate(&gd, area, seed));
        match result {
            Ok(Ok(g)) => {
                let ours = g.map.tile_hash().to_string();
                if ours == want {
                    e.0 += 1;
                    println!("{area}\t{seed}\tmatch");
                } else {
                    e.1 += 1;
                    println!("{area}\t{seed}\tdiffer\tours {ours} client {want}\t{}", g.graph);
                }
            }
            Ok(Err(err)) => {
                e.2 += 1;
                e.3 = err.to_string();
                println!("{area}\t{seed}\terror\t{err}");
            }
            Err(_) => {
                e.2 += 1;
                e.3 = "panic".into();
                println!("{area}\t{seed}\tpanic");
            }
        }
    }
    println!("--- per area (match, differ, error, last error)");
    for (a, (m, d, e, last)) in &per_area {
        println!("{a}\t{m}\t{d}\t{e}\t{last}");
    }
    Ok(())
}
