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

#[test]
fn resolves_start_state() {
    let Some(data) = open() else { return };
    let s = poe_data::views::StartState::find(&data, "DexFourDefault")
        .unwrap()
        .expect("DexFourDefault exists");
    assert_eq!(
        (s.class_id, s.class_name.as_str(), s.level),
        (2, "Ranger", 1)
    );
}

#[test]
fn ranger_template_resolves_through_ao() {
    let Some(data) = open() else { return };
    let t =
        poe_data::template::ObjectTemplate::load(&data, "Metadata/Characters/Dex/DexFour").unwrap();
    assert!(
        t.files.iter().any(|f| f.ends_with("Character.ot")),
        "{:?}",
        t.files
    );
    let stats = t.component("Stats");
    assert!(stats.iter().any(|(k, v)| k == "is_player" && v == "1"));
    assert!(
        stats
            .iter()
            .any(|(k, v)| k == "life_per_level" && v == "12")
    );
    assert!(
        t.component("Positioned")
            .iter()
            .any(|(k, v)| k == "team" && v == "1")
    );
}

#[test]
fn riverbank_entrance_room_points() {
    let Some(data) = open() else { return };
    let room = poe_data::rooms::Room::load(
        &data,
        "Metadata/Terrain/Gallows/Act1/1_1/Rooms/entrance_intro.arm",
    )
    .unwrap();
    assert_eq!(room.name, "entrance");
    assert_eq!((room.width, room.height), (45, 54));
    let tree = room.slot("intro_feature").unwrap();
    assert_eq!((tree.x, tree.y, tree.width, tree.height), (30, 36, 6, 6));
    let p = room.point("default1").unwrap();
    assert_eq!((p.x, p.y), (146, 169));
}
