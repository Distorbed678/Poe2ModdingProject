//! Prints a dungeon master's room-size map per door type (`0x141f98bb0`): `cargo run --release
//! -p poe-world --example dungeon_sizes -- <room set .rs path>`.

use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rs = std::env::args().nth(1).ok_or("usage: dungeon_sizes <.rs>")?;
    let gd = poe_data::GameData::open(Path::new("GameFiles"), Path::new("Tools/dat-schema/schema-poe2.min.json"))?;
    let set = poe_world::rooms::RoomSet::load(gd.files(), &rs)?;
    for r in &set.rooms {
        let a = &r.room.arm;
        let doors: Vec<String> = a.thingies.iter().map(|t| t.et_file.clone().unwrap_or_default().rsplit('/').next().unwrap_or("").to_string()).collect();
        println!("{:50} tag {:14} w{:4} mask {:#04x} units {:?} doors {:?}", r.path.rsplit('/').next().unwrap(), r.tag, r.weight, r.orientations, a.numbers1, doors);
    }
    println!("{}", poe_world::dungeon::size_report(&set));
    Ok(())
}
