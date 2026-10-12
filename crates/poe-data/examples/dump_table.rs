//! Dump a table: `cargo run -p poe-data --example dump_table -- <Table> [col ...] [--rows N]`.
//! Without columns, prints every column (unnamed ones as `_<index>`).

use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let table_name = args
        .next()
        .ok_or("usage: dump_table <Table> [columns...] [--rows N]")?;
    let mut columns: Vec<String> = Vec::new();
    let mut max_rows = usize::MAX;
    while let Some(a) = args.next() {
        if a == "--rows" {
            max_rows = args.next().ok_or("--rows needs a value")?.parse()?;
        } else {
            columns.push(a);
        }
    }
    let data = poe_data::GameData::open(
        Path::new("GameFiles"),
        Path::new("Tools/dat-schema/schema-poe2.min.json"),
    )?;
    let table = data.table(&table_name)?;
    if columns.is_empty() {
        columns = table
            .schema()
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| c.name.clone().unwrap_or(format!("_{i}")))
            .collect();
    }
    println!("{table_name}: {} rows", table.len());
    for row in table.rows().take(max_rows) {
        let values: Vec<String> = columns
            .iter()
            .map(|c| match row.get(c) {
                Ok(v) => format!("{c}={v:?}"),
                Err(e) => format!("{c}=<{e}>"),
            })
            .collect();
        println!("[{}] {}", row.index(), values.join(" "));
    }
    Ok(())
}
