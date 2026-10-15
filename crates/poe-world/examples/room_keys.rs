//! Prints the header of `.arm` rooms (tag, numbers, flags, root key) — what the room set
//! matches graph nodes against: `cargo run -p poe-world --example room_keys -- <path.arm>...`.

use poe_data_tools::file_parsers::{FileParser, arm::ARMParser};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = poe_data::GameFiles::open(std::path::Path::new("GameFiles"))?;
    for path in std::env::args().skip(1) {
        let bytes = files.read(&path.to_lowercase())?;
        let arm = ARMParser.parse(&bytes).map_err(|e| format!("{e:?}"))?;
        println!("{path}\n  v{} tag {:?} dims {:?} numbers1 {:?} bools {:?} thingies {:?} string1 {:?}",
            arm.version, arm.tag, arm.dimensions, arm.numbers1, arm.bools, arm.thingies, arm.string1);
        println!("  strings {:?}", arm.strings);
        println!("  root {:?}", arm.root_slot);
    }
    Ok(())
}
