//! Counts the client's placed tiles (a generator trace) that the exit wiggle could move:
//! `cargo run -p poe-world --example wiggle_check -- <generator_trace.tsv> <stop line>`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = poe_data::GameFiles::open(std::path::Path::new("GameFiles"))?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stop: usize = args[1].parse()?;
    let text = std::fs::read_to_string(&args[0])?;
    let mut w = poe_world::dungeon_tiles::Wiggles::default();
    let mut cells: std::collections::BTreeMap<(i32, i32), (String, String)> = Default::default();
    for (n, l) in text.lines().enumerate() {
        if n + 1 >= stop {
            break;
        }
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 5 || f[1] != "place" {
            continue;
        }
        let mut it = f[3].split(' ');
        let (x, y): (i32, i32) = (it.next().unwrap().parse()?, it.next().unwrap().parse()?);
        cells.insert((x, y), (f[4].to_string(), f[2].to_string()));
    }
    let mut count = std::collections::BTreeMap::<String, u32>::new();
    for ((x, y), (path, site)) in &cells {
        let tdt = poe_world::generate::load_tdt(&files, path)?;
        if w.movable(&files, &tdt)? {
            *count.entry(site.clone()).or_default() += 1;
            if std::env::var("LIST").is_ok() {
                println!("{x} {y} {site} {path}");
            }
        }
    }
    println!("{count:?}");
    Ok(())
}
