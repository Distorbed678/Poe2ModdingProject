//! Recomputes the tile and doodad hashes of the client DLL's area exports with the server's
//! code: `cargo run -p poe-world --example check_exports [<session or areas dir>...]`
//! (default: everything under `dumps/<version>/terrain/area-exports` of the newest version folder).

use std::path::{Path, PathBuf};

use poe_world::export::AreaExport;

fn area_dirs(root: &Path, out: &mut Vec<PathBuf>) {
    if root.join("meta.txt").is_file() {
        out.push(root.to_path_buf());
        return;
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                area_dirs(&e.path(), out);
            }
        }
    }
}

/// `dumps/<version>/terrain/area-exports` of the last (by name) version folder that has one.
fn default_root() -> PathBuf {
    let mut versions: Vec<PathBuf> = std::fs::read_dir("dumps")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("terrain/area-exports"))
        .filter(|p| p.is_dir())
        .collect();
    versions.sort();
    versions.pop().unwrap_or_else(|| "dumps/terrain/area-exports".into())
}

fn main() {
    let roots: Vec<PathBuf> = match std::env::args().skip(1).collect::<Vec<_>>() {
        a if a.is_empty() => vec![default_root()],
        a => a.into_iter().map(PathBuf::from).collect(),
    };
    let mut dirs = Vec::new();
    for r in &roots {
        area_dirs(r, &mut dirs);
    }
    dirs.sort();
    let (mut ok, mut bad) = (0, 0);
    for d in dirs {
        let Ok(a) = AreaExport::load(&d) else {
            continue;
        };
        let want = |k| a.meta(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let (t, dh) = (a.tile_hash(), a.doodad_hash());
        let good = t == want("tile_hash") && (a.doodads.is_empty() || dh == want("doodad_hash"));
        if good {
            ok += 1;
        } else {
            bad += 1;
            println!(
                "MISMATCH {}: tile {t} vs {}, doodad {dh} vs {}",
                d.display(),
                want("tile_hash"),
                want("doodad_hash")
            );
        }
    }
    println!("{ok} exports match, {bad} differ");
}
