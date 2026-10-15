//! Summarises an `.arm` room: `cargo run -p poe-world --example arm_info -- <game path.arm>`.

use poe_data_tools::file_parsers::{
    FileParser,
    arm::{ARMParser, types::Slot},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = poe_data::GameFiles::open(std::path::Path::new("GameFiles"))?;
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: arm_info <path.arm>")?;
    let bytes = files.read(&path.to_lowercase())?;
    let arm = ARMParser.parse(&bytes).map_err(|e| format!("{e:?}"))?;
    println!(
        "v{} tag {:?} dims {:?} grid {}x{} root {:?}",
        arm.version,
        arm.tag,
        arm.dimensions,
        arm.grid.first().map_or(0, |r| r.len()),
        arm.grid.len(),
        match &arm.root_slot {
            Slot::K(k) => format!("K {}x{} tag {:?}", k.width, k.height, k.slot_tag),
            s => format!("{s:?}"),
        }
    );
    for (y, row) in arm.grid.iter().enumerate() {
        let line: String = row
            .iter()
            .map(|s| match s {
                Slot::K(_) => 'k',
                Slot::N => '.',
                Slot::F { .. } => 'f',
                Slot::S => 's',
                Slot::O => 'o',
            })
            .collect();
        println!("{y:3} {line}");
    }
    let mut keys = 0;
    for (y, row) in arm.grid.iter().enumerate() {
        for (x, s) in row.iter().enumerate() {
            if let Slot::K(k) = s {
                keys += 1;
                if keys <= 12 {
                    println!(
                        "K at ({x},{y}) {}x{} tag {:?} origin {:?} edges {:?} corners {:?}",
                        k.width,
                        k.height,
                        k.slot_tag,
                        k.origin,
                        k.edges
                            .iter()
                            .map(|e| (
                                e.edge
                                    .as_deref()
                                    .map(|p| p.rsplit('/').next().unwrap_or("")),
                                e.exit,
                                e.virtual_exit
                            ))
                            .collect::<Vec<_>>(),
                        k.corners
                            .iter()
                            .map(|c| (
                                c.ground
                                    .as_deref()
                                    .map(|p| p.rsplit('/').next().unwrap_or("")),
                                c.height
                            ))
                            .collect::<Vec<_>>()
                    );
                }
            }
            if let Slot::F { fill } = s {
                let _ = fill;
            }
        }
    }
    println!(
        "{keys} K slots; doodads {}, decals {}, poi groups {}",
        arm.doodads.len(),
        arm.decals.len(),
        arm.points_of_interest.len()
    );
    Ok(())
}
