//! Generates Clearfell Encampment with the server's port and compares every tile with a client
//! export: `cargo run -p poe-world --example town_gen -- <export dir> <seed>`.

use std::path::Path;

use poe_world::{
    export::AreaExport,
    generate::{Master, TileMap, derepeat_pass, edge_cost_grid, fill_pass, load_room, room_pass},
    rng::TerrainRng,
};

const DIR: &str = "Metadata/Terrain/Gallows/Act1/1_town_ExileEncampment/";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let export = AreaExport::load(Path::new(
        &args.next().ok_or("usage: town_gen <export dir> <seed>")?,
    ))?;
    let seed: u32 = args.next().ok_or("seed")?.parse()?;
    let files = poe_data::GameFiles::open(Path::new("GameFiles"))?;
    let mut master = Master::load(
        &files,
        &format!("{DIR}tiles.tst"),
        &format!("{DIR}random_fill_tiles.gft"),
    )?;
    let room = load_room(&files, &format!("{DIR}Rooms/town_1.arm"))?;

    let mut rng = TerrainRng::new(seed);
    rng.below(u32::MAX); // topology sub-seed
    rng.below(8); // the graph node's room
    let mut map = TileMap::new(33, 30);
    room_pass(&mut map, &master, &room, (0, 0), &mut rng)?;
    edge_cost_grid(&map, &mut rng);
    rng.below(0);
    fill_pass(&mut map, &mut master, &files, &mut rng)?;
    derepeat_pass(&mut map, &master, &[poe_world::generate::RoomPlacement { room: &room, origin: (0, 0), orientation: 0 }], &mut rng);
    println!(
        "draws: {}; tile hash ours {} client {}",
        rng.draws,
        map.tile_hash(),
        export.meta("tile_hash").unwrap_or("?")
    );

    let (mut same, mut orient, mut other, mut sub) = (0, 0, 0, 0);
    for y in 0..map.height {
        for x in 0..map.width {
            let ours = map.get(x, y);
            let theirs = export.tile(x, y);
            match (ours, theirs) {
                (Some(a), Some(b)) if a.path.eq_ignore_ascii_case(&b.path) => {
                    if a.orientation != b.orientation {
                        orient += 1;
                        if a.sub == 0 {
                            println!(
                                "({x},{y}) {} ours o{} client o{}",
                                a.path, a.orientation, b.orientation
                            );
                        }
                    } else if a.sub != b.sub {
                        sub += 1;
                        if sub <= 3 {
                            println!("({x},{y}) {} sub ours {} client {}", a.path, a.sub, b.sub);
                        }
                    } else {
                        same += 1;
                    }
                }
                _ => {
                    other += 1;
                    if other <= 10 {
                        println!(
                            "({x},{y}) ours {:?} client {:?}",
                            ours.map(|c| (&c.path, c.orientation)),
                            theirs.map(|c| (&c.path, c.orientation))
                        );
                    }
                }
            }
        }
    }
    println!(
        "cells: {same} identical, {sub} other sub-tile, {orient} other orientation, {other} other tile (of {})",
        map.width * map.height
    );
    Ok(())
}
