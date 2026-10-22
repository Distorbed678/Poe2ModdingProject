//! `cargo xtask server <build|run|test|web-admin>`: the server (Poe2Server, `modules/server`)
//! from the hub.
//!
//! The server is its own Cargo workspace with its own `target/`: it is built with cargo inside
//! `modules/server` (`modules/server/target/<profile>/poe-server`) but RUN with the hub root as
//! the working directory, because every path of its config (`GameFiles/`, `dumps/`, `runtime/`,
//! `modules/protocol/...`, `modules/web-admin/dist`) is relative to the working directory and
//! the hub is where those folders are. The config comes from `--config`, `POE_CONFIG` or
//! `modules/server/config/server.toml`.
//!
//! The web admin served by a hub-run server is the hub's own `modules/web-admin` (the single
//! source of truth in development; the copy under `modules/server/modules/web-admin` only
//! feeds the server's releases). `server run` rebuilds it with `npm run build` when its sources
//! are newer than `dist/build.json` (or `dist` is missing), and installs `node_modules` first
//! when they are missing. `--no-web-build` skips that.

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

use anyhow::{Context, Result, bail};
use clap::Subcommand;

/// The server checkout, relative to the hub root.
pub const SERVER_DIR: &str = "modules/server";
/// The web admin checkout the dev server serves, relative to the hub root.
pub const WEB_ADMIN_DIR: &str = "modules/web-admin";

#[derive(Subcommand)]
pub enum ServerCmd {
    /// `cargo build -p poe-server` in modules/server.
    Build {
        #[arg(long)]
        release: bool,
    },
    /// Build (and rebuild the web admin when stale), then run poe-server with the hub as the
    /// working directory. Arguments after `--` go to poe-server (`account ...`, `character ...`, `mod ...`); the web
    /// admin is then not checked.
    Run {
        #[arg(long)]
        release: bool,
        /// Do not check or rebuild the web admin.
        #[arg(long)]
        no_web_build: bool,
        /// Do not build the server first (run the existing binary).
        #[arg(long)]
        no_build: bool,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// `cargo test` in modules/server (extra arguments after `--`).
    Test {
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Check the hub's web admin build against its sources and rebuild it when stale.
    WebAdmin {
        /// Only report whether the build is current (exit 1 when stale or missing).
        #[arg(long)]
        check: bool,
        /// Rebuild even when the build is current.
        #[arg(long)]
        force: bool,
    },
}

pub fn run(root: &Path, config: &Path, cmd: ServerCmd) -> Result<()> {
    let server = root.join(SERVER_DIR);
    if !server.join("Cargo.toml").is_file() {
        bail!(
            "{} is not checked out: git submodule update --init --recursive",
            server.display()
        );
    }
    match cmd {
        ServerCmd::Build { release } => build(&server, release).map(|_| ()),
        ServerCmd::Test { args } => {
            let mut c = Command::new("cargo");
            c.arg("test").args(args).current_dir(&server);
            crate::run(&mut c)
        }
        ServerCmd::WebAdmin { check, force } => {
            let state = web_admin_state(root)?;
            println!("web admin: {}", state.describe());
            if check {
                if state.is_current() {
                    return Ok(());
                }
                bail!("the web admin build is not current: run `cargo xtask server web-admin`");
            }
            if force || !state.is_current() {
                build_web_admin(root)?;
            }
            Ok(())
        }
        ServerCmd::Run {
            release,
            no_web_build,
            no_build,
            args,
        } => {
            // Arguments mean a one-shot command (`account ...`, `character ...`, `mod ...`): the
            // web admin is not needed for those.
            if !no_web_build && args.is_empty() {
                let state = web_admin_state(root)?;
                if !state.is_current() {
                    println!("web admin: {}; rebuilding", state.describe());
                    build_web_admin(root)?;
                }
            }
            let bin = if no_build {
                binary(&server, release)
            } else {
                build(&server, release)?
            };
            if !bin.is_file() {
                bail!("{} not found: run `cargo xtask server build`", bin.display());
            }
            println!(
                "running {} (working directory {}, config {})",
                bin.display(),
                root.display(),
                config.display()
            );
            let status = Command::new(&bin)
                .args(args)
                .current_dir(root)
                .env(poe_config::ENV_PATH, config)
                .status()
                .with_context(|| bin.display().to_string())?;
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

fn binary(server: &Path, release: bool) -> PathBuf {
    let exe = if cfg!(windows) { "poe-server.exe" } else { "poe-server" };
    server
        .join("target")
        .join(if release { "release" } else { "debug" })
        .join(exe)
}

fn build(server: &Path, release: bool) -> Result<PathBuf> {
    let mut c = Command::new("cargo");
    c.args(["build", "-p", "poe-server"]).current_dir(server);
    if release {
        c.arg("--release");
    }
    crate::run(&mut c)?;
    Ok(binary(server, release))
}

/// How the web admin build compares with its sources.
#[derive(Debug, PartialEq, Eq)]
pub enum WebState {
    NoBuild,
    /// The newest source and its age compared with the build (seconds newer).
    Stale { newest: String, secs: u64 },
    Current { built_at: Option<String> },
}

impl WebState {
    pub fn is_current(&self) -> bool {
        matches!(self, WebState::Current { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            WebState::NoBuild => "no dist/build.json".into(),
            WebState::Stale { newest, secs } => {
                format!("{newest} is {secs} s newer than dist/build.json")
            }
            WebState::Current { built_at } => format!(
                "dist/build.json is current (built {})",
                built_at.as_deref().unwrap_or("at an unknown time")
            ),
        }
    }
}

const WEB_FILES: &[&str] = &[
    "index.html",
    "package.json",
    "package-lock.json",
    "vite.config.ts",
    "svelte.config.js",
    "tsconfig.json",
    "tsconfig.node.json",
];

pub fn web_admin_state(root: &Path) -> Result<WebState> {
    let dir = root.join(WEB_ADMIN_DIR);
    if !dir.join("package.json").is_file() {
        bail!("{WEB_ADMIN_DIR} is not checked out: git submodule update --init modules/web-admin");
    }
    let build = dir.join("dist/build.json");
    let Ok(meta) = std::fs::metadata(&build) else {
        return Ok(WebState::NoBuild);
    };
    let built = meta.modified()?;
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    let mut visit = |p: &Path| {
        if let Ok(t) = std::fs::metadata(p).and_then(|m| m.modified())
            && newest.as_ref().is_none_or(|(n, _)| t > *n)
        {
            newest = Some((t, p.to_owned()));
        }
    };
    for f in WEB_FILES {
        visit(&dir.join(f));
    }
    for d in ["src", "public"] {
        walk(&dir.join(d), &mut visit);
    }
    if let Some((t, p)) = newest
        && let Ok(d) = t.duration_since(built)
        && d.as_secs() >= 1
    {
        let rel = p.strip_prefix(&dir).unwrap_or(&p).display().to_string();
        return Ok(WebState::Stale { newest: rel, secs: d.as_secs() });
    }
    let built_at = std::fs::read_to_string(&build)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["built_at"].as_str().map(str::to_owned));
    Ok(WebState::Current { built_at })
}

fn walk(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, visit);
        } else {
            visit(&p);
        }
    }
}

/// `npm ci` (when `node_modules` is missing) and `npm run build` in the hub's web admin.
pub fn build_web_admin(root: &Path) -> Result<()> {
    let dir = root.join(WEB_ADMIN_DIR);
    if !dir.join("node_modules").is_dir() {
        let mut c = Command::new("npm");
        c.args(["ci"]).current_dir(&dir);
        crate::run(&mut c).context("npm ci (is Node installed?)")?;
    }
    let mut c = Command::new("npm");
    c.args(["run", "build"]).current_dir(&dir);
    crate::run(&mut c).context("npm run build")?;
    match web_admin_state(root)? {
        s if s.is_current() => {
            println!("web admin: {}", s.describe());
            Ok(())
        }
        s => bail!("the web admin build is still not current after npm run build: {}", s.describe()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_every_state() {
        assert!(WebState::NoBuild.describe().contains("build.json"));
        assert!(!WebState::NoBuild.is_current());
        assert!(
            WebState::Stale { newest: "src/a.ts".into(), secs: 5 }
                .describe()
                .contains("src/a.ts")
        );
        assert!(WebState::Current { built_at: None }.is_current());
    }
}
