//! Prints an area's terrain inputs: its topologies (with the Topologies row's numbers), each
//! graph's nodes and edges, and the graph's master file:
//! `cargo run -p poe-world --example area_info -- <WorldAreas.Id>`.

use std::path::Path;

use poe_data::{GameData, Value};
use poe_world::{generate::Files, graph::Graph};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let id = std::env::args().nth(1).ok_or("usage: area_info <WorldAreas.Id>")?;
    let data = GameData::open(
        Path::new("GameFiles"),
        Path::new("third_party/dat-schema/schema-poe2.min.json"),
    )?;
    let areas = data.table("WorldAreas")?;
    let area = areas.find_by_str("Id", &id)?.ok_or("no such area")?;
    let topologies = data.table("Topologies")?;
    let rows: Vec<usize> = match area.get("Topologies")? {
        Value::List(l) => l.iter().filter_map(Value::as_row).collect(),
        _ => Vec::new(),
    };
    println!("{id}: {} topologies", rows.len());
    for r in rows {
        let t = topologies.row(r).ok_or("bad topology row")?;
        let values: Vec<String> = (2..topologies.schema().columns.len())
            .map(|c| format!("{:?}", t.get_index(c).ok()))
            .collect();
        let dgr = t.str("DGRFile")?;
        println!("- {} {dgr} {}", t.str("Id")?, values.join(" "));
        let g = Graph::parse_bytes(&data.files().read_vec(&dgr)?)?;
        println!(
            "  size {}x{} master {} grounds {:?}",
            g.width, g.height, g.master_file, g.ground_types
        );
        for (i, n) in g.nodes.iter().enumerate() {
            println!(
                "  node {i}: ({}, {}) edges {:?} room {:?} {} tags {:?} values {:?}",
                n.x, n.y, n.edges, n.room_type, n.orientation, n.tags, n.values
            );
        }
        for (i, e) in g.edges.iter().enumerate() {
            println!(
                "  edge {i}: {} -> {} points {:?} values {:?} {} tail {:?}",
                e.from, e.to, e.points, e.values, e.edge_type, e.tail
            );
        }
        let tsi = String::from_utf8_lossy(&data.files().read_vec(&g.master_file)?).to_string();
        let tsi = poe_world_text(&tsi, &data.files().read_vec(&g.master_file)?);
        println!("  master:\n{}", tsi.lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}

/// `.tsi` files are UTF-16.
fn poe_world_text(_: &str, bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units).trim_start_matches('\u{feff}').to_string()
}

trait ReadVec {
    fn read_vec(&self, path: &str) -> Result<Vec<u8>, poe_world::generate::GenError>;
}

impl ReadVec for poe_data::GameFiles {
    fn read_vec(&self, path: &str) -> Result<Vec<u8>, poe_world::generate::GenError> {
        Files::read(self, path)
    }
}
