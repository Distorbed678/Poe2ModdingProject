//! The pathfinder port against the paths the client planned (`path_log`, `PATHPLAN` lines of
//! the DLL's log, fixture `tests/fixtures/clientpath_plans.json`: start, goal, parameters, the
//! entity's clearance level and the waypoints the client returned).
//!
//! The terrain is the client's own cell grid of the test area (`terrain_grid_<tile>.bin`,
//! exported by the DLL into the git-ignored `dumps/`), so the test skips when the file is not
//! there.

use poe_game::{
    clientpath::{Params, plan},
    grid::Grid,
};
use serde_json::Value;
use std::path::PathBuf;

fn grid(tile: u64) -> Option<Grid> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let name = format!("terrain_grid_{tile}.bin");
    ["terrain/grids-keep", "terrain/grids"]
        .iter()
        .map(|d| root.join("dumps/0.5.5e").join(d).join(&name))
        .find_map(|p| std::fs::read(p).ok())
        .and_then(|d| Grid::parse(&d))
}

fn cell(v: &Value) -> (i32, i32) {
    (v[0].as_i64().unwrap() as i32, v[1].as_i64().unwrap() as i32)
}

#[test]
fn the_port_plans_the_same_paths_as_the_client() {
    let fx: Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/tests/fixtures/clientpath_plans.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    let Some(g) = grid(fx["tile"].as_u64().unwrap()) else {
        eprintln!("skipped: the test area's cell grid is not in dumps/");
        return;
    };
    let plans = fx["plans"].as_array().unwrap();
    let (mut exact, mut deviating) = (0, 0);
    let mut bad = Vec::new();
    for p in plans {
        let params = Params {
            min_dist: p["min"].as_u64().unwrap() as u32,
            max_dist: p["max"].as_u64().unwrap() as u32,
            radius: p["radius"].as_i64().unwrap() as i8,
        };
        let got = plan(
            &g,
            cell(&p["start"]),
            cell(&p["goal"]),
            p["level"].as_i64().unwrap() as i32,
            &params,
        );
        let want: Vec<_> = p["path"].as_array().unwrap().iter().map(cell).collect();
        if got.path == want {
            exact += 1;
        } else if p["note"] == "midline" {
            // The client's last line-of-sight test next to the start differs (docs/re/pathfinding.md).
            deviating += 1;
            assert_eq!(got.path.first(), want.first(), "{p}");
        } else {
            bad.push(format!("{p} -> {:?}", got.path));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} plans differ:\n{}",
        bad.len(),
        plans.len(),
        bad.join("\n")
    );
    eprintln!("{exact} plans exact, {deviating} with the known deviation");
    assert!(exact >= plans.len() - 8);
}
