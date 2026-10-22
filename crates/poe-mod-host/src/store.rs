//! The mods folder on disk: `state.toml`, one folder per mod, `packages/<id>-<version>.zip`.
//!
//! ```text
//! mods/
//!   state.toml                  enabled = ["hello-both", "motd"]   (load order)
//!   hello-both/mod.toml ...     the installed mod: what the server loads and hashes
//!   packages/
//!     hello-both-0.1.0.zip      what clients download (written by `mod install`, or packed
//!     hello-both-0.1.0.zip.sha256   from the folder at startup when missing)
//! ```
//!
//! Everything here is file logic with no server state, so the `poe-server mod` CLI and the
//! host share it.

use std::{
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

use poe_hook_mods::{
    Limits, MANIFEST_FILE, Manifest, State, parse_manifest, parse_state, sha256_file_text,
    sha256_hex, valid_id, verify_package_bytes,
};

/// The enabled mods in load order.
pub const STATE_FILE: &str = "state.toml";
/// The folder (inside the mods folder) that holds the packages.
pub const PACKAGES_DIR: &str = "packages";

/// A mod folder with a valid manifest.
#[derive(Debug, Clone)]
pub struct Installed {
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub unknown_keys: Vec<String>,
    /// SHA-256 of the folder's `mod.toml`, byte for byte (lower-case hex).
    pub manifest_sha256: String,
}

/// A verified package in `packages/`.
#[derive(Debug, Clone)]
pub struct PackageEntry {
    pub path: PathBuf,
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub size: u64,
    /// SHA-256 of the `mod.toml` inside the zip (lower-case hex).
    pub manifest_sha256: String,
}

/// What a scan of the mods folder found.
#[derive(Debug, Default)]
pub struct Scan {
    pub dir: PathBuf,
    /// `state.toml` exists (the mod system is on for this server).
    pub state_present: bool,
    pub enabled: Vec<String>,
    pub installed: Vec<Installed>,
    /// Folders that look like mods but cannot be used: (folder, reason).
    pub broken: Vec<(String, String)>,
    /// Problems with the folder itself (an unreadable `state.toml`).
    pub notes: Vec<String>,
}

impl Scan {
    pub fn find(&self, id: &str) -> Option<&Installed> {
        self.installed.iter().find(|m| m.id == id)
    }
}

/// Reads the mods folder. A missing folder is an empty scan (no `state.toml`).
pub fn scan(dir: &Path) -> Scan {
    let mut scan = Scan {
        dir: dir.to_owned(),
        ..Default::default()
    };
    match std::fs::read_to_string(dir.join(STATE_FILE)) {
        Ok(text) => {
            scan.state_present = true;
            match parse_state(&text) {
                Ok(s) => scan.enabled = s.enabled,
                Err(e) => scan.notes.push(format!("{e}: no mod is enabled")),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            scan.state_present = true;
            scan.notes.push(format!("cannot read {STATE_FILE}: {e}"));
        }
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return scan;
    };
    let mut folders: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .collect();
    folders.sort_by_key(|e| e.file_name());
    for e in folders {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == PACKAGES_DIR || name.starts_with('.') {
            continue;
        }
        let manifest_path = e.path().join(MANIFEST_FILE);
        let Ok(bytes) = std::fs::read(&manifest_path) else {
            continue; // not a mod folder
        };
        let parsed = std::str::from_utf8(&bytes)
            .map_err(|_| vec!["mod.toml is not UTF-8".to_owned()])
            .and_then(parse_manifest);
        match parsed {
            Ok(p) if p.manifest.id == name => scan.installed.push(Installed {
                id: name,
                dir: e.path(),
                manifest_sha256: sha256_hex(&bytes),
                unknown_keys: p.unknown_keys,
                manifest: p.manifest,
            }),
            Ok(p) => scan.broken.push((
                name.clone(),
                format!(
                    "the folder is named {name:?} but the manifest id is {:?}",
                    p.manifest.id
                ),
            )),
            Err(errors) => scan.broken.push((
                name,
                format!("invalid {MANIFEST_FILE}: {}", errors.join("; ")),
            )),
        }
    }
    scan
}

/// Writes `state.toml` (atomically).
pub fn write_state(dir: &Path, enabled: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let body = toml::to_string(&State {
        enabled: enabled.to_vec(),
    })
    .map_err(|e| e.to_string())?;
    let text = format!(
        "# The enabled server mods, in load order. Edit with `poe-server mod enable|disable`;\n# changes need a restart of the server.\n{body}"
    );
    let tmp = dir.join(".state.toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, dir.join(STATE_FILE))
        .map_err(|e| format!("cannot replace state.toml: {e}"))
}

/// Enables or disables an installed mod in `state.toml` (enabling appends it to the load
/// order). Returns whether the file changed.
pub fn set_enabled(dir: &Path, id: &str, enable: bool) -> Result<bool, String> {
    if !valid_id(id) {
        return Err(format!("{id:?} is not a valid mod id"));
    }
    let scan = scan(dir);
    if enable && scan.find(id).is_none() {
        return Err(match scan.broken.iter().find(|(f, _)| f == id) {
            Some((_, why)) => format!("{id} is installed but unusable: {why}"),
            None => format!(
                "{id} is not installed in {} (poe-server mod install <zip>)",
                dir.display()
            ),
        });
    }
    let mut enabled = scan.enabled;
    let had = enabled.iter().any(|e| e == id);
    if enable == had {
        // Writing the file also creates it, which turns the mod system on.
        if !scan.state_present {
            write_state(dir, &enabled)?;
        }
        return Ok(false);
    }
    if enable {
        enabled.push(id.to_owned());
    } else {
        enabled.retain(|e| e != id);
    }
    write_state(dir, &enabled)?;
    Ok(true)
}

/// Lists the verified packages in `packages/` (and the zips lying directly in the mods
/// folder). Problems are returned next to the good ones.
pub fn packages(dir: &Path) -> (Vec<PackageEntry>, Vec<(PathBuf, Vec<String>)>) {
    let mut good = Vec::new();
    let mut bad = Vec::new();
    for folder in [dir.join(PACKAGES_DIR), dir.to_owned()] {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let mut zips: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "zip"))
            .collect();
        zips.sort();
        for path in zips {
            match read_package(&path) {
                Ok(p) => good.push(p),
                Err(e) => bad.push((path, e)),
            }
        }
    }
    (good, bad)
}

/// Verifies one package file (the `.sha256` sidecar is checked when it exists).
pub fn read_package(path: &Path) -> Result<PackageEntry, Vec<String>> {
    let bytes =
        std::fs::read(path).map_err(|e| vec![format!("cannot read {}: {e}", path.display())])?;
    let expected = sidecar_hash(path)?;
    let name = path.file_name().and_then(|n| n.to_str());
    let report = verify_package_bytes(&bytes, name, expected.as_deref(), &Limits::default())?;
    let manifest_sha256 = zip_file(&bytes, MANIFEST_FILE)
        .map(|b| sha256_hex(&b))
        .ok_or_else(|| vec!["mod.toml not found in the package".to_owned()])?;
    Ok(PackageEntry {
        path: path.to_owned(),
        id: report.manifest.id,
        version: report.manifest.version,
        sha256: report.sha256,
        size: report.size,
        manifest_sha256,
    })
}

fn sidecar_hash(zip: &Path) -> Result<Option<String>, Vec<String>> {
    let mut side = zip.as_os_str().to_owned();
    side.push(".sha256");
    match std::fs::read_to_string(PathBuf::from(side)) {
        Ok(text) => poe_hook_mods::parse_sha256_file(&text)
            .map(|(hash, _)| Some(hash))
            .map_err(|e| vec![e]),
        Err(_) => Ok(None),
    }
}

/// The bytes of one file inside a zip.
pub fn zip_file(zip: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).ok()?;
    let mut f = archive.by_name(name).ok()?;
    let mut out = Vec::new();
    f.read_to_end(&mut out).ok()?;
    Some(out)
}

/// Packs a mod folder deterministically (sorted entries, one timestamp, fixed permissions):
/// the same folder gives the same bytes. Used when a mod has no package yet.
pub fn pack_folder(dir: &Path) -> Result<Vec<u8>, String> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut out = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut out);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for rel in files {
        zip.start_file(&rel, options).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(dir.join(&rel)).map_err(|e| format!("cannot read {rel}: {e}"))?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for e in std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let meta = e.file_type().map_err(|e| e.to_string())?;
        if meta.is_symlink() {
            return Err(format!(
                "{} is a symbolic link; packages may not contain any",
                path.display()
            ));
        }
        if meta.is_dir() {
            collect(root, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(rel);
        }
    }
    Ok(())
}

/// Writes a package and its `.sha256` sidecar into `packages/`.
pub fn write_package(dir: &Path, id: &str, version: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let folder = dir.join(PACKAGES_DIR);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    let name = poe_hook_mods::package_file_name(id, version);
    let path = folder.join(&name);
    std::fs::write(&path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let side = folder.join(format!("{name}.sha256"));
    std::fs::write(&side, sha256_file_text(&sha256_hex(bytes), &name))
        .map_err(|e| format!("cannot write {}: {e}", side.display()))?;
    Ok(path)
}

/// One finding of [`verify_all`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub error: bool,
    pub text: String,
}

/// Checks the whole mods folder without loading anything: every package verifies (hash,
/// names, paths), every installed manifest is valid, enabled mods are installed, a folder and its
/// package agree, server libraries exist for this platform and match `server_api`. Errors make
/// `poe-server mod verify` fail.
pub fn verify_all(dir: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    macro_rules! err {
        ($($a:tt)*) => {
            out.push(Finding { error: true, text: format!($($a)*) })
        };
    }
    let scan = scan(dir);
    if !scan.state_present {
        err!(
            "{} has no {STATE_FILE}: the mod system is off",
            dir.display()
        );
    }
    for n in &scan.notes {
        err!("{n}");
    }
    for (folder, why) in &scan.broken {
        err!("{folder}: {why}");
    }
    let (pkgs, bad) = packages(dir);
    for (path, why) in &bad {
        err!("{}: {}", path.display(), why.join("; "));
    }
    for id in &scan.enabled {
        let Some(m) = scan.find(id) else {
            if !scan.broken.iter().any(|(f, _)| f == id) {
                err!("{id}: enabled but not installed");
            }
            continue;
        };
        let man = &m.manifest;
        if man.side.includes_server() {
            if let Err(e) = poe_hook_mods::check_server_api(man, poe_mod_api::SERVER_API_VERSION) {
                err!("{id}: {e}");
            }
            if man.has_server_library() {
                match poe_hook_mods::Platform::current().and_then(|p| man.server_library(p)) {
                    Some(rel) if m.dir.join(rel).is_file() => {}
                    Some(rel) => err!("{id}: the server library {rel:?} is missing"),
                    None => err!("{id}: no server library for this platform"),
                }
            }
        }
        if man.side.includes_client() {
            if let Some(rel) = &man.entry_dll
                && !m.dir.join(rel).is_file()
            {
                err!("{id}: the client DLL {rel:?} is missing");
            }
            match pkgs
                .iter()
                .find(|p| p.id == *id && p.version == man.version)
            {
                Some(p) if p.manifest_sha256 != m.manifest_sha256 => {
                    err!(
                        "{id}: the package differs from the installed folder (it is repacked at the next start)"
                    )
                }
                Some(_) => {}
                None => out.push(Finding {
                    error: false,
                    text: format!("{id}: no package yet (it is packed at the next start)"),
                }),
            }
        }
    }
    out
}

/// What `install` did.
#[derive(Debug)]
pub struct Installation {
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub dir: PathBuf,
    pub replaced: bool,
    pub warnings: Vec<String>,
}

/// Verifies `zip` and installs it: the folder `<dir>/<id>/` (replacing an installed version)
/// and the package under `packages/`. Does not enable the mod.
pub fn install_package(dir: &Path, zip: &Path) -> Result<Installation, Vec<String>> {
    let bytes =
        std::fs::read(zip).map_err(|e| vec![format!("cannot read {}: {e}", zip.display())])?;
    let expected = sidecar_hash(zip)?;
    let name = zip.file_name().and_then(|n| n.to_str());
    let report = verify_package_bytes(&bytes, name, expected.as_deref(), &Limits::default())?;
    let id = report.manifest.id.clone();
    let version = report.manifest.version.clone();
    let target = dir.join(&id);
    let staging = dir.join(format!(".install-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    extract(&bytes, &staging).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staging);
        vec![e]
    })?;
    let replaced = target.exists();
    if replaced {
        let old = dir.join(format!(".replaced-{id}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&old);
        std::fs::rename(&target, &old)
            .map_err(|e| vec![format!("cannot replace {}: {e}", target.display())])?;
        let _ = std::fs::remove_dir_all(&old);
    }
    std::fs::rename(&staging, &target).map_err(|e| vec![format!("cannot move into place: {e}")])?;
    write_package(dir, &id, &version, &bytes).map_err(|e| vec![e])?;
    Ok(Installation {
        sha256: report.sha256,
        warnings: report.warnings,
        id,
        version,
        dir: target,
        replaced,
    })
}

/// Unpacks a verified package (paths were checked by `verify_package_bytes`; this re-checks
/// them against escaping `into` anyway).
fn extract(bytes: &[u8], into: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(format!("unsafe path {:?} in the package", entry.name()));
        };
        let path = into.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out =
            std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn write_mod(dir: &Path, id: &str, version: &str, extra: &str) {
        let d = dir.join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join(MANIFEST_FILE),
            format!(
                "manifest_version = 2\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nside = \"server\"\n[game]\nversions = [\"0.5.5e\"]\n{extra}\n"
            ),
        )
        .unwrap();
    }

    const CMD: &str = "[[commands]]\nname = \"hi\"\nhelp = \"x\"\nreply = \"hello\"\n";

    #[test]
    fn scan_finds_mods_state_and_broken_folders() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!scan(tmp.path()).state_present);
        write_mod(tmp.path(), "good-mod", "1.0.0", CMD);
        std::fs::create_dir_all(tmp.path().join("badmod")).unwrap();
        std::fs::write(tmp.path().join("badmod").join(MANIFEST_FILE), "nonsense =").unwrap();
        std::fs::create_dir_all(tmp.path().join("plain-dir")).unwrap();
        write_state(tmp.path(), &["good-mod".into()]).unwrap();
        let s = scan(tmp.path());
        assert!(s.state_present);
        assert_eq!(s.enabled, ["good-mod"]);
        assert_eq!(s.installed.len(), 1);
        assert_eq!(s.installed[0].manifest_sha256.len(), 64);
        assert_eq!(s.broken.len(), 1);
        assert_eq!(s.broken[0].0, "badmod");
    }

    #[test]
    fn enable_and_disable_edit_state_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "aa-mod", "1.0.0", CMD);
        write_mod(tmp.path(), "bb-mod", "1.0.0", CMD);
        assert!(set_enabled(tmp.path(), "bb-mod", true).unwrap());
        assert!(set_enabled(tmp.path(), "aa-mod", true).unwrap());
        assert!(!set_enabled(tmp.path(), "aa-mod", true).unwrap());
        assert_eq!(scan(tmp.path()).enabled, ["bb-mod", "aa-mod"]);
        assert!(set_enabled(tmp.path(), "bb-mod", false).unwrap());
        assert_eq!(scan(tmp.path()).enabled, ["aa-mod"]);
        assert!(set_enabled(tmp.path(), "nope-mod", true).is_err());
        assert!(set_enabled(tmp.path(), "Bad Id", true).is_err());
        // Disabling something that is not enabled is fine (and still creates state.toml).
        assert!(!set_enabled(tmp.path(), "bb-mod", false).unwrap());
    }

    #[test]
    fn packing_is_reproducible_and_installs_back() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "pack-mod", "2.0.0", CMD);
        std::fs::write(tmp.path().join("pack-mod").join("README.md"), "hi").unwrap();
        let a = pack_folder(&tmp.path().join("pack-mod")).unwrap();
        let b = pack_folder(&tmp.path().join("pack-mod")).unwrap();
        assert_eq!(a, b);
        // A fresh server folder: install the zip.
        let other = tempfile::tempdir().unwrap();
        let zip = other.path().join("pack-mod-2.0.0.zip");
        std::fs::write(&zip, &a).unwrap();
        let srv = tempfile::tempdir().unwrap();
        let done = install_package(srv.path(), &zip).unwrap();
        assert_eq!(
            (done.id.as_str(), done.version.as_str()),
            ("pack-mod", "2.0.0")
        );
        assert!(!done.replaced);
        assert_eq!(done.sha256, sha256_hex(&a));
        assert!(srv.path().join("pack-mod").join("README.md").is_file());
        let (pkgs, bad) = packages(srv.path());
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].sha256, done.sha256);
        assert_eq!(
            pkgs[0].manifest_sha256,
            scan(srv.path()).installed[0].manifest_sha256
        );
        // Installing again replaces it.
        assert!(install_package(srv.path(), &zip).unwrap().replaced);
    }

    #[test]
    fn a_wrong_file_name_or_hash_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "pack-mod", "2.0.0", CMD);
        let bytes = pack_folder(&tmp.path().join("pack-mod")).unwrap();
        let wrong_name = tmp.path().join("something.zip");
        std::fs::write(&wrong_name, &bytes).unwrap();
        assert!(install_package(&tmp.path().join("srv"), &wrong_name).is_err());
        let zip = tmp.path().join("pack-mod-2.0.0.zip");
        std::fs::write(&zip, &bytes).unwrap();
        std::fs::write(
            tmp.path().join("pack-mod-2.0.0.zip.sha256"),
            format!("{}  pack-mod-2.0.0.zip\n", "0".repeat(64)),
        )
        .unwrap();
        let err = install_package(&tmp.path().join("srv"), &zip).unwrap_err();
        assert!(err[0].contains("SHA-256 mismatch"), "{err:?}");
    }
}
