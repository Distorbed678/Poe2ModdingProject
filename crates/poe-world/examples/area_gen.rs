//! Generates an area's layout and prints its tile hash:
//! `cargo run -p poe-world --example area_gen -- <area id> <seed> [<seed>...]`.
//! `POE_FORCE=<topology>:<orientation>` forces the layout (as `map_force_topology` /
//! `map_force_orientation` do; either side may be empty).

use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let area = args.next().ok_or("usage: area_gen <area id> <seed>...")?;
    let gd = poe_data::GameData::open(Path::new("GameFiles"), Path::new("Tools/dat-schema/schema-poe2.min.json"))?;
    for seed in args {
        let seed: u32 = seed.parse()?;
        let forced = std::env::var("POE_FORCE").ok().map(|v| {
            let (t, o) = v.split_once(':').unwrap_or((&v, ""));
            poe_world::area::Forced { topology: t.parse().ok(), orientation: o.parse().ok() }
        });
        let mut rng = poe_world::rng::TerrainRng::new(seed);
        match poe_world::area::generate_with(&gd, &area, &mut rng, forced) {
            Ok(g) => println!("{area}\t{seed}\t{}\t{}x{}\t{}", g.map.tile_hash(), g.map.width, g.map.height, g.graph),
            Err(e) => println!("{area}\t{seed}\terror\t{e}"),
        }
    }
    Ok(())
}
