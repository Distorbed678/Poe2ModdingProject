//! `cargo xtask versions`: keeps the launcher's copies of the version files.
//!
//! `modules/protocol/versions/<version>.toml` (Poe2Protocol) is the single definition of a
//! supported build. The launcher is a separate repository (the submodule `modules/launcher`) that must
//! build on its own, so it embeds copies, `modules/launcher/versions/<version>.toml`, which
//! are byte-identical to the Poe2Protocol files. This command writes them (or with `--check` only
//! compares, exit 1 when one is stale); `check-build` and a test of this crate compare them
//! too. Nothing in the launcher repository is edited by hand.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// The launcher's folder of version files, relative to the repository root.
pub const LAUNCHER_DIR: &str = "modules/launcher/versions";

fn copy_path(root: &Path, version: &str) -> PathBuf {
    root.join(LAUNCHER_DIR).join(format!("{version}.toml"))
}

/// `Ok(None)` when the launcher's copy equals the root file; `Ok(Some(why))` when it differs or
/// is missing; `Err` when the launcher checkout is not there (nothing to compare).
pub fn check_launcher_copy(root: &Path, version: &str) -> Result<Option<String>> {
    if !root.join(LAUNCHER_DIR).is_dir() {
        bail!("{LAUNCHER_DIR} is not checked out (git submodule update --init modules/launcher)");
    }
    let source = std::fs::read(poe_config::VersionFile::path(root, version))?;
    match std::fs::read(copy_path(root, version)) {
        Ok(copy) if copy == source => Ok(None),
        Ok(_) => Ok(Some(format!(
            "{LAUNCHER_DIR}/{version}.toml differs from modules/protocol/versions/{version}.toml"
        ))),
        Err(_) => Ok(Some(format!("{LAUNCHER_DIR}/{version}.toml is missing"))),
    }
}

/// Copies every `modules/protocol/versions/*.toml` to the launcher (or only checks them).
pub fn run(root: &Path, check: bool) -> Result<()> {
    let dir = root.join(LAUNCHER_DIR);
    if !dir.is_dir() {
        bail!(
            "{} is not checked out: git submodule update --init modules/launcher",
            dir.display()
        );
    }
    let mut stale = 0;
    for version in poe_config::versions::known_versions(root) {
        // Parse first: a broken file must not reach the launcher.
        poe_config::VersionFile::load(root, &version)
            .with_context(|| format!("modules/protocol/versions/{version}.toml"))?;
        match check_launcher_copy(root, &version)? {
            None => println!("{LAUNCHER_DIR}/{version}.toml: in sync"),
            Some(why) if check => {
                println!("{why}");
                stale += 1;
            }
            Some(why) => {
                std::fs::copy(
                    poe_config::VersionFile::path(root, &version),
                    copy_path(root, &version),
                )?;
                println!("{why}: copied");
            }
        }
    }
    if stale > 0 {
        bail!("{stale} launcher version file(s) out of date: run `cargo xtask versions`");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launchers_copies_equal_the_version_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        if !root.join(LAUNCHER_DIR).is_dir() {
            eprintln!("skipped: modules/launcher is not checked out");
            return;
        }
        for version in poe_config::versions::known_versions(&root) {
            assert_eq!(
                check_launcher_copy(&root, &version).unwrap(),
                None,
                "run `cargo xtask versions`"
            );
        }
    }
}
