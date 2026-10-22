//! `poe-server mod list|enable|disable|install|verify`: manages the mods folder offline (the
//! server does not need to run; changes apply at the next start).

use std::path::Path;

use poe_mod_host::store::{self, Scan};

const USAGE: &str = "usage: poe-server mod <command>
  list                       the mods in the mods folder ([mods] dir) and whether they are enabled
  install <zip> [--enable]   verify a package and install it (a newer version replaces the old one)
  enable <id>                load the mod at the next start (appended to the load order)
  disable <id>               do not load it any more
  verify                     check packages, manifests and libraries without starting the server
Changes apply when the server is restarted.";

pub fn run(dir: &Path, args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("list") => {
            print!("{}", list(&store::scan(dir)));
            Ok(())
        }
        Some("enable") | Some("disable") if args.len() == 2 => {
            let enable = args[0] == "enable";
            if store::set_enabled(dir, &args[1], enable).map_err(anyhow::Error::msg)? {
                println!("{} {}d in {}; restart the server to apply it", args[1], args[0], dir.display());
            } else {
                println!("{} was already {}d", args[1], args[0]);
            }
            Ok(())
        }
        Some("install") if args.len() >= 2 => {
            let enable = args[2..].iter().any(|a| a == "--enable");
            if args[2..].iter().any(|a| a != "--enable") {
                anyhow::bail!(USAGE);
            }
            let done = store::install_package(dir, Path::new(&args[1])).map_err(|e| anyhow::anyhow!(e.join("\n")))?;
            for w in &done.warnings {
                println!("warning: {w}");
            }
            println!(
                "installed {} {}{} in {} (package sha256 {})",
                done.id,
                done.version,
                if done.replaced { ", replacing the installed version" } else { "" },
                done.dir.display(),
                done.sha256
            );
            if enable {
                store::set_enabled(dir, &done.id, true).map_err(anyhow::Error::msg)?;
                println!("enabled {}; restart the server to load it", done.id);
            } else {
                println!("not enabled yet: poe-server mod enable {}", done.id);
            }
            Ok(())
        }
        Some("verify") if args.len() == 1 => {
            let findings = store::verify_all(dir);
            for f in &findings {
                println!("{}: {}", if f.error { "error" } else { "warning" }, f.text);
            }
            let errors = findings.iter().filter(|f| f.error).count();
            if errors > 0 {
                anyhow::bail!("{errors} problem(s) in {}", dir.display());
            }
            println!("{} ok", dir.display());
            Ok(())
        }
        _ => anyhow::bail!(USAGE),
    }
}

/// The `list` table.
pub fn list(scan: &Scan) -> String {
    let mut out = String::new();
    if !scan.state_present {
        out.push_str(&format!(
            "{} has no state.toml: the mod system is off (poe-server mod enable <id> creates it)\n",
            scan.dir.display()
        ));
    }
    for n in &scan.notes {
        out.push_str(&format!("error: {n}\n"));
    }
    out.push_str("id\tversion\tside\tstate\n");
    // Enabled mods first, in load order; then the others.
    let mut ids: Vec<&str> = scan.enabled.iter().map(String::as_str).collect();
    ids.extend(scan.installed.iter().map(|m| m.id.as_str()).filter(|id| !scan.enabled.iter().any(|e| e == id)));
    for id in ids {
        let enabled = scan.enabled.iter().any(|e| e == id);
        match scan.find(id) {
            Some(m) => out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                m.id,
                m.manifest.version,
                m.manifest.side.as_str(),
                if enabled { "enabled" } else { "disabled" }
            )),
            None => out.push_str(&format!("{id}\t-\t-\tenabled but not installed\n")),
        }
    }
    for (folder, why) in &scan.broken {
        out.push_str(&format!("{folder}\t-\t-\tunusable: {why}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn list_shows_enabled_in_order_then_the_rest_and_the_broken() {
        let dir = tmp();
        for id in ["aa-mod", "bb-mod"] {
            let d = dir.path().join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("mod.toml"),
                format!("manifest_version = 2\nid = \"{id}\"\nname = \"x\"\nversion = \"1.0.0\"\nside = \"server\"\n[[commands]]\nname = \"c\"\nhelp = \"h\"\nreply = \"r\"\n"),
            )
            .unwrap();
        }
        std::fs::create_dir_all(dir.path().join("broken")).unwrap();
        std::fs::write(dir.path().join("broken/mod.toml"), "x =").unwrap();
        store::write_state(dir.path(), &["bb-mod".into(), "ghost-mod".into()]).unwrap();
        let text = list(&store::scan(dir.path()));
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows[0], "id\tversion\tside\tstate");
        assert_eq!(rows[1], "bb-mod\t1.0.0\tserver\tenabled");
        assert_eq!(rows[2], "ghost-mod\t-\t-\tenabled but not installed");
        assert_eq!(rows[3], "aa-mod\t1.0.0\tserver\tdisabled");
        assert!(rows[4].starts_with("broken\t-\t-\tunusable: invalid mod.toml"), "{text}");
    }

    #[test]
    fn the_commands_edit_the_folder() {
        let dir = tmp();
        let d = dir.path().join("only-mod");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("mod.toml"),
            "manifest_version = 2\nid = \"only-mod\"\nname = \"x\"\nversion = \"1.0.0\"\nside = \"server\"\n[[commands]]\nname = \"c\"\nhelp = \"h\"\nreply = \"r\"\n",
        )
        .unwrap();
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        run(dir.path(), &args(&["enable", "only-mod"])).unwrap();
        assert_eq!(store::scan(dir.path()).enabled, ["only-mod"]);
        run(dir.path(), &args(&["verify"])).unwrap();
        run(dir.path(), &args(&["disable", "only-mod"])).unwrap();
        assert!(store::scan(dir.path()).enabled.is_empty());
        assert!(run(dir.path(), &args(&["enable", "missing-mod"])).is_err());
        assert!(run(dir.path(), &args(&["bogus"])).is_err());
        assert!(run(dir.path(), &args(&["install", "nothing.zip"])).is_err());
    }
}
