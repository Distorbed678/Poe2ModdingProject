//! Prints `.tdt` headers: `cargo run -p poe-world --example tdt_info -- <path.tdt>...`
//! (game paths, read from GameFiles).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = poe_data::GameFiles::open(std::path::Path::new("GameFiles"))?;
    for path in std::env::args().skip(1) {
        let bytes = files.read(&path.to_lowercase())?;
        match poe_world::tdt::Tdt::parse(&bytes) {
            Ok(t) => println!(
                "{path}: v{} feature {:?} size {:?} inherits {:?} edges {:?} grounds {:?} unk {:?} dist {:?} rest {} bytes",
                t.version,
                t.feature,
                t.size,
                t.inherits,
                t.edges
                    .iter()
                    .map(|e| e.as_deref().unwrap_or("-").rsplit('/').next().unwrap_or(""))
                    .collect::<Vec<_>>(),
                t.grounds
                    .iter()
                    .map(|e| e.as_deref().unwrap_or("-").rsplit('/').next().unwrap_or(""))
                    .collect::<Vec<_>>(),
                t.unk,
                t.edge_distances,
                t.rest.len()
            ),
            Err(e) => println!("{path}: {e}"),
        }
    }
    Ok(())
}
