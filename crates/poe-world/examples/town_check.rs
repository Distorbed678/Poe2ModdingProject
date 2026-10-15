//! Replays the Clearfell Encampment's key-slot pass with the server's code and compares each
//! slot's tile and orientation with a client export:
//! `cargo run -p poe-world --example town_check -- <export dir> [seed] [generator_trace.tsv]`
//! (with a trace, the room pass is compared with the client's room-pass placements instead of
//! the final layout, which a later pass changes).

use std::path::Path;

use poe_data_tools::file_parsers::{
    FileParser,
    arm::{ARMParser, types::Slot},
    tst::TSTParser,
};
use poe_world::{
    export::AreaExport,
    rng::TerrainRng,
    tdt::Tdt,
    tileset::{TileDef, TileKey, TileSet, orientation_mask, pick},
};

const ROOM: &str = "Metadata/Terrain/Gallows/Act1/1_town_ExileEncampment/Rooms/town_1.arm";
const TILES: &str = "Metadata/Terrain/Gallows/Act1/1_town_ExileEncampment/tiles.tst";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let export = AreaExport::load(Path::new(
        &args.next().ok_or("usage: town_check <export dir> [seed]")?,
    ))?;
    let seed: u32 = args.next().map_or(Ok(1), |s| s.parse())?;
    // Room-pass placements from a client trace: (x, y) -> (path, orientation).
    let mut traced: std::collections::HashMap<(u32, u32), (String, u8)> = Default::default();
    if let Some(trace) = args.next() {
        for line in std::fs::read_to_string(trace)?.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() >= 5 && f[1] == "place" && f[2] == "141f8ede4" {
                let a: Vec<&str> = f[3].split(' ').collect();
                traced.insert(
                    (a[0].parse()?, a[1].parse()?),
                    (f[4].to_string(), a[2].trim_start_matches('o').parse()?),
                );
            }
        }
        println!("{} room-pass placements in the trace", traced.len());
    }
    let files = poe_data::GameFiles::open(Path::new("GameFiles"))?;
    let read = |p: &str| files.read(&p.to_lowercase());

    let tst = TSTParser
        .parse(&read(TILES)?)
        .map_err(|e| format!("{e:?}"))?;
    let mut tiles = Vec::new();
    for e in &tst.tdt_files {
        let tdt = Tdt::parse(&read(&e.tdt_file)?)?;
        tiles.push(TileDef {
            path: e.tdt_file.clone(),
            tdt,
            weight: e.weight.unwrap_or(1),
            orientations: orientation_mask(&e.rotations),
        });
    }
    println!(
        "{} tiles in the tile set ({} includes)",
        tiles.len(),
        tst.includes.len()
    );
    let set = TileSet::new(tiles);
    let arm = ARMParser
        .parse(&read(ROOM)?)
        .map_err(|e| format!("{e:?}"))?;
    let (w, h) = (arm.grid[0].len(), arm.grid.len());

    let mut rng = TerrainRng::new(seed);
    rng.below(u32::MAX); // topology sub-seed
    rng.below(8); // the graph node's room
    let (mut ok, mut bad, mut none) = (0, 0, 0);
    for x in 0..w {
        for y in 0..h {
            let (key, feature) = match &arm.grid[y][x] {
                Slot::K(k) => {
                    let key = TileKey {
                        edges: k
                            .edges
                            .clone()
                            .map(|e| e.edge.map(|s| s.to_ascii_lowercase())),
                        size: (k.width as u8, k.height as u8),
                        grounds: k
                            .corners
                            .clone()
                            .map(|c| c.ground.map(|s| s.to_ascii_lowercase())),
                        heights: k.corners.clone().map(|c| c.height as u8),
                        exits: k.edges.clone().map(|e| e.exit as u8),
                        virtual_exits: k.edges.clone().map(|e| e.virtual_exit as u8),
                    };
                    // Uniform ground and no edges: only the ground is marked (fill pass later).
                    if k.slot_tag.is_none()
                        && key.edges.iter().all(Option::is_none)
                        && key.grounds.iter().all(|g| *g == key.grounds[0])
                    {
                        continue;
                    }
                    (key, k.slot_tag.clone().unwrap_or_default())
                }
                // A feature cell: a 1x1 tile with that feature name.
                Slot::F { fill: Some(name) } => (
                    TileKey {
                        size: (1, 1),
                        exits: [3; 4],
                        virtual_exits: [3; 4],
                        ..Default::default()
                    },
                    name.clone(),
                ),
                _ => continue,
            };
            let list = set.candidates(&feature, &key);
            let got = match traced.get(&(x as u32, y as u32)) {
                Some((p, o)) => Some(poe_world::export::ExportedTile {
                    x: x as u32,
                    y: y as u32,
                    path: p.clone(),
                    sub: 0,
                    height: 0,
                    orientation: *o,
                    skip: (0, 0),
                }),
                None if !traced.is_empty() => None,
                None => export.tile(x as u32, y as u32).cloned(),
            };
            let got = got.as_ref();
            if list.is_empty() {
                if none == 0 {
                    println!("slot ({x},{y}) key {key:?} feature {feature:?}");
                    for (i, t) in set.tiles.iter().enumerate() {
                        if t.path.contains("Slash/Fill_") {
                            println!(
                                "  tile {i} {} key {:?} orientations {:#x} feature {:?}",
                                t.path,
                                TileKey::from_tdt(&t.tdt),
                                t.orientations,
                                t.tdt.feature
                            );
                        }
                    }
                }
                none += 1;
                println!(
                    "({x},{y}) no candidates; client: {:?}",
                    got.map(|t| (&t.path, t.orientation))
                );
                continue;
            }
            let total: u32 = list.iter().map(|c| c.weight).sum();
            let c = pick(list, rng.below(total)).unwrap();
            let path = &set.tiles[c.tile].path;
            match got {
                Some(t) if t.path.eq_ignore_ascii_case(path) && t.orientation == c.orientation => {
                    ok += 1
                }
                _ => {
                    bad += 1;
                    println!(
                        "({x},{y}) n={total} ours {path} o{} ; client {:?}",
                        c.orientation,
                        got.map(|t| (&t.path, t.orientation))
                    );
                }
            }
        }
    }
    println!(
        "key slots: {ok} match, {bad} differ, {none} without candidates; draws {}",
        rng.draws
    );
    Ok(())
}
