//! Prints a `.tdt`'s header and corner points (ground, height):
//! `cargo run -p poe-world --example tdt_points -- <path.tdt>...`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = poe_data::GameFiles::open(std::path::Path::new("GameFiles"))?;
    for path in std::env::args().skip(1) {
        let t = poe_world::tdt::Tdt::parse(&files.read(&path.to_lowercase())?)?;
        println!("{path}: v{} size {:?} grounds {:?} edges {:?}", t.version, t.size, t.grounds, t.edges);
        let w = t.size.0 as usize + 1;
        for (i, row) in t.point_grounds.chunks(w).enumerate() {
            let names: Vec<String> = row
                .iter()
                .map(|g| g.as_deref().map_or("-", |g| g.rsplit('/').next().unwrap()).to_string())
                .collect();
            let hs = t.point_heights.get(i * w..(i + 1) * w).unwrap_or(&[]);
            println!("  {i}: {} | {:?}", names.join(" "), hs);
        }
    }
    Ok(())
}
