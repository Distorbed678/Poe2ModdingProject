//! Tries to generate every WorldAreas row that has topologies, one attempt per topology (first
//! allowed orientation, one seed), and prints one TSV row per attempt:
//! `area  topology(1-based)  orientation  graph  seconds  ok|error|panic|timeout  detail`
//! (`detail` is the tile hash and draw count, or the error). It checks no hash: it shows what the
//! generator can run at all, which `cargo xtask terrain-matrix` reads as the probe column.
//!
//! `cargo run --release -p poe-world --example area_probe -- [seed 1001] [area filter | all]
//! [timeout seconds 30] > dumps/0.5.5e/terrain/vectors/probe-results.tsv`
//! A hung attempt is abandoned (its thread keeps spinning); after eight the run stops.

use std::{path::Path, sync::mpsc, time::Duration};

use poe_data::GameData;
use poe_world::{area::Forced, rng::TerrainRng};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let seed: u32 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(1001);
    let filter = args.next().filter(|a| a != "all");
    let timeout = Duration::from_secs(args.next().map(|s| s.parse()).transpose()?.unwrap_or(30));
    let gd: &'static GameData = Box::leak(Box::new(GameData::open(
        Path::new("GameFiles"),
        Path::new("third_party/dat-schema/schema-poe2.min.json"),
    )?));
    std::panic::set_hook(Box::new(|_| {}));
    let ids: Vec<String> = gd.table("WorldAreas")?.rows().filter_map(|r| r.str("Id").ok()).collect();
    let mut abandoned = 0;
    for id in ids {
        if filter.as_deref().is_some_and(|f| f != id) {
            continue;
        }
        let Ok(entries) = poe_world::area::topology_entries(gd, &id, 1) else { continue };
        for (t, (path, mask)) in (1u32..).zip(&entries) {
            let o = if *mask == 0 || *mask == 0xff { 0 } else { mask.trailing_zeros() as u8 };
            let (tx, rx) = mpsc::channel();
            let area = id.clone();
            let start = std::time::Instant::now();
            std::thread::spawn(move || {
                let r = std::panic::catch_unwind(|| {
                    let mut rng = TerrainRng::new(seed);
                    let forced = Forced { topology: Some(t), orientation: Some(o) };
                    poe_world::area::generate_with(gd, &area, &mut rng, Some(forced)).map(|g| (g.map.tile_hash(), g.draws))
                });
                let _ = tx.send(match r {
                    Ok(Ok((h, d))) => ("ok", format!("hash {h} draws {d}")),
                    Ok(Err(e)) => ("error", e.to_string().replace(['\n', '\t'], " ")),
                    Err(_) => ("panic", String::new()),
                });
            });
            let (status, detail) = rx.recv_timeout(timeout).unwrap_or_else(|_| {
                abandoned += 1;
                ("timeout", String::new())
            });
            println!("{id}\t{t}\t{o}\t{path}\t{:.2}\t{status}\t{detail}", start.elapsed().as_secs_f32());
            if abandoned >= 8 {
                eprintln!("eight attempts hung; stopping");
                std::process::exit(2);
            }
        }
    }
    Ok(())
}
