//! Content diff of two client installs' Bundles2 folders (two game builds): prints one line per
//! virtual file that was added, removed or changed (`changed<TAB>path<TAB>old size<TAB>new size`),
//! a summary on stderr. Only files whose index record (bundle, offset, size) or bundle file size
//! differ are read and compared by SHA-256, so a hotfix takes minutes, not a 150 GB read.
//!
//! Usage (docs/guides/new-game-version.md):
//!   cargo run --release --manifest-path scripts/re/bundlediff/Cargo.toml -- <old game dir> <new game dir> > diff.tsv
use std::{collections::HashMap, fs, path::{Path, PathBuf}};
use poe_data_tools::file_parsers::{FileParser, bundle::BundleParser, bundle_index::{BundleIndexParser, types::BundleIndexFile}};
use poe_data_tools::fs::{FS, FileSystem};
use poe_data_tools::hasher::murmur64a::{BuildHasherEx, BuildMurmurHash64A};
use sha2::{Digest, Sha256};

const HASHER: BuildMurmurHash64A = BuildMurmurHash64A { seed: 0x1337b33f };

fn index(dir: &Path) -> BundleIndexFile {
    let b = fs::read(dir.join("Bundles2/_.index.bin")).unwrap();
    let c = BundleParser.parse(&b).unwrap().read_all().unwrap();
    BundleIndexParser.parse(&c).unwrap()
}
struct Side { dir: PathBuf, idx: BundleIndexFile, lut: HashMap<u64, usize>, paths: HashMap<u64, String> }
fn side(dir: &str) -> Side {
    let dir = PathBuf::from(dir);
    let idx = index(&dir);
    let lut = idx.files.iter().enumerate().map(|(i, f)| (f.hash, i)).collect();
    let mut paths = HashMap::new();
    let fsys = FS::from_steam(dir.clone()).unwrap(); for s in fsys.list() { paths.insert(HASHER.hash_one_str(&s.to_lowercase()), s); }
    Side { dir, idx, lut, paths }
}
fn bsize(s: &Side, bi: u32) -> Option<u64> { fs::metadata(s.dir.join(format!("Bundles2/{}.bundle.bin", s.idx.bundles[bi as usize].name))).ok().map(|m| m.len()) }

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (o, n) = (side(&a[1]), side(&a[2]));
    let mut cand: HashMap<u32, Vec<(u64, usize, usize)>> = HashMap::new(); // new bundle -> (hash, old file idx, new file idx)
    let mut nb_old: HashMap<u32, ()> = HashMap::new();
    let (mut added, mut removed) = (vec![], vec![]);
    for (h, &ni) in &n.lut {
        match o.lut.get(h) {
            None => added.push(n.paths.get(h).cloned().unwrap_or(format!("{h:016x}"))),
            Some(&oi) => {
                let (fo, fnw) = (&o.idx.files[oi], &n.idx.files[ni]);
                let on = &o.idx.bundles[fo.bundle_index as usize].name; let nn = &n.idx.bundles[fnw.bundle_index as usize].name;
                let same = on == nn && fo.offset == fnw.offset && fo.size == fnw.size && bsize(&o, fo.bundle_index) == bsize(&n, fnw.bundle_index);
                if !same { cand.entry(fnw.bundle_index).or_default().push((*h, oi, ni)); nb_old.insert(fo.bundle_index, ()); }
            }
        }
    }
    for h in o.lut.keys() { if !n.lut.contains_key(h) { removed.push(o.paths.get(h).cloned().unwrap_or(format!("{h:016x}"))); } }
    eprintln!("candidates: {} files in {} new bundles", cand.values().map(|v| v.len()).sum::<usize>(), cand.len());
    // old file contents keyed by hash: load old bundles lazily with a cache of one
    let mut old_by_bundle: HashMap<u32, Vec<(u64, usize)>> = HashMap::new();
    for v in cand.values() { for &(h, oi, _) in v { old_by_bundle.entry(o.idx.files[oi].bundle_index).or_default().push((h, oi)); } }
    let mut old_hash: HashMap<u64, [u8; 32]> = HashMap::new();
    for (bi, v) in &old_by_bundle {
        let b = BundleParser.parse(&fs::read(o.dir.join(format!("Bundles2/{}.bundle.bin", o.idx.bundles[*bi as usize].name))).unwrap()).unwrap();
        for &(h, oi) in v { let f = &o.idx.files[oi]; old_hash.insert(h, Sha256::digest(b.read_range(f.offset as usize, f.size as usize).unwrap()).into()); }
    }
    let mut changed = vec![];
    for (bi, v) in &cand {
        let b = BundleParser.parse(&fs::read(n.dir.join(format!("Bundles2/{}.bundle.bin", n.idx.bundles[*bi as usize].name))).unwrap()).unwrap();
        for &(h, oi, ni) in v {
            let f = &n.idx.files[ni];
            let nh: [u8; 32] = Sha256::digest(b.read_range(f.offset as usize, f.size as usize).unwrap()).into();
            if old_hash[&h] != nh { changed.push(format!("{}\t{}\t{}", n.paths.get(&h).cloned().unwrap_or(format!("{h:016x}")), o.idx.files[oi].size, f.size)); }
        }
    }
    added.sort(); removed.sort(); changed.sort();
    for p in &added { println!("added\t{p}"); }
    for p in &removed { println!("removed\t{p}"); }
    for p in &changed { println!("changed\t{p}"); }
    eprintln!("added {} removed {} changed {}", added.len(), removed.len(), changed.len());
}
