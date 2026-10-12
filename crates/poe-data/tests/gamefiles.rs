//! Smoke tests against the real client data. Skipped when `GameFiles/` or the schema is
//! missing (e.g. on CI).

use std::path::PathBuf;

use poe_data::GameData;

fn open() -> Option<GameData> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let game = root.join("GameFiles");
    let schema = root.join("Tools/dat-schema/schema-poe2.min.json");
    if !game.join("Bundles2/_.index.bin").exists() || !schema.exists() {
        eprintln!("skipping: GameFiles or schema not present");
        return None;
    }
    Some(GameData::open(&game, &schema).expect("open game data"))
}

#[test]
fn reads_characters_table() {
    let Some(data) = open() else { return };
    let chars = data.table("Characters").unwrap();
    assert!(chars.len() >= 6, "expected the playable classes");
    let ids: Vec<String> = chars.rows().map(|r| r.str("Id").unwrap()).collect();
    println!("characters: {ids:?}");
    assert!(ids.iter().any(|id| id.starts_with("Metadata/Characters/")));
}

#[test]
fn finds_first_area() {
    let Some(data) = open() else { return };
    let areas = data.table("WorldAreas").unwrap();
    let row = areas
        .find_by_str("Id", "G1_1")
        .unwrap()
        .expect("G1_1 exists");
    println!(
        "G1_1: name={:?} act={} level={} hash16={} file_width={} schema_width={}",
        row.str("Name").unwrap(),
        row.int("Act").unwrap(),
        row.int("AreaLevel").unwrap(),
        row.int("HASH16").unwrap(),
        areas.row_width(),
        areas.schema().row_width(),
    );
    assert_eq!(row.int("Act").unwrap(), 1);
}
