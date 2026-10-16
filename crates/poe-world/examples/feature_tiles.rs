//! Lists a tile set's tiles with a feature name:
//! `cargo run -p poe-world --example feature_tiles -- <tiles.tst> <fill.gft> [feature]`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = poe_data::GameFiles::open(std::path::Path::new("GameFiles"))?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let master = poe_world::generate::Master::load(&files, &args[0].to_lowercase(), &args[1].to_lowercase())?;
    for t in &master.tiles.tiles {
        let Some(f) = &t.tdt.feature else { continue };
        if args.get(2).is_some_and(|want| !f.eq_ignore_ascii_case(want)) {
            continue;
        }
        println!("{f}\t{}\tw{} o{:#04x}\t{:?}", t.path, t.weight, t.orientations, t.key);
    }
    Ok(())
}
