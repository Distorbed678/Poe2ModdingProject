//! `cargo xtask <command>`: development automation.
//!
//! Everything that touches `GameFiles/` goes through here, so every change can be reverted.
//! Backups use the `.orig` suffix and are created only once, so repeated deploys never
//! overwrite the pristine file.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use poe_config::Config;
use poe_hook_deploy::{DeployOptions, GatewayEntry, Mode, launch::ProtonLaunch};

/// The client DLL workspace (Poe2ClientHook submodule), relative to the repo root.
const CLIENT_HOOK_DIR: &str = "modules/client-hook";
/// The client's compile target.
const DLL_TARGET: &str = "x86_64-pc-windows-msvc";

#[derive(Parser)]
#[command(about = "PoE2 server development tasks")]
struct Cli {
    /// Server config to read ports/hosts from.
    #[arg(long, default_value = poe_config::DEFAULT_PATH)]
    config: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build the client DLLs (modules/client-hook: version.dll loader and the debug core) with cargo-xwin.
    BuildDll {
        #[arg(long)]
        debug: bool,
    },
    /// Point GameFiles at the local server: gateway entry, hook DLLs + config, steam_appid.txt.
    DeployClient {
        /// Skip building the DLLs first.
        #[arg(long)]
        no_build: bool,
        /// Official servers, read-only: keep the original gateway list and set the DLL's
        /// `mode = "official"` (no crypto bypass, no injection; only observing and logging).
        #[arg(long)]
        official: bool,
    },
    /// Build and install only the debug core; a running client reloads it by itself.
    DeployCore {
        #[arg(long)]
        no_build: bool,
    },
    /// Undo deploy-client (restores the original gateway list, removes installed files).
    RevertClient,
    /// Only rewrite the DLL's name tables in `GameFiles/poe-hook-data/` (stats, action types,
    /// areas); leaves the gateway and installed DLLs as they are.
    DllData,
    /// Launch the client through Proton with a dedicated prefix (Steam must be running).
    RunClient {
        /// Proton installation to use (folder name under steamapps/common).
        #[arg(long, default_value = "Proton - Experimental")]
        proton: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = repo_root();
    std::env::set_current_dir(&root)?;
    let config = Config::load(&cli.config)?;
    match cli.cmd {
        Cmd::BuildDll { debug } => build_dll(&root, !debug).map(|_| ()),
        Cmd::DeployClient { no_build, official } => {
            deploy_client(&root, &config, !no_build, official)
        }
        Cmd::DeployCore { no_build } => deploy_core(&root, &config, !no_build),
        Cmd::RevertClient => revert_client(&config),
        Cmd::DllData => export_dll_data(&config),
        Cmd::RunClient { proton } => run_client(&root, &config, &proton),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .expect("xtask lives in xtask")
        .to_owned()
}

fn client_hook_dir(root: &Path) -> PathBuf {
    root.join(CLIENT_HOOK_DIR)
}

fn build_dll(root: &Path, release: bool) -> Result<PathBuf> {
    let xwin = root.join("third_party/cargo-xwin/bin/cargo-xwin");
    if !xwin.exists() {
        bail!("{} missing; run third_party/fetch-tools.sh", xwin.display());
    }
    let client = client_hook_dir(root);
    if !client.join("Cargo.toml").exists() {
        bail!(
            "{} is empty; run `git submodule update --init --recursive`",
            client.display()
        );
    }
    let mut cmd = Command::new(&xwin);
    // When invoked directly, cargo subcommands expect their own name as the first argument.
    // Only the two DLL crates: the workspace also holds the host-side poe-hook-deploy.
    cmd.args(["xwin", "build", "-p", "poe-hook", "-p", "poe-hook-core"])
        .args(["--target", DLL_TARGET])
        .current_dir(&client)
        .env("XWIN_CACHE_DIR", root.join("third_party/xwin-cache"));
    if release {
        cmd.arg("--release");
    }
    run(&mut cmd)?;
    let out = dll_out_dir(root, release);
    println!("built {}", out.display());
    Ok(out)
}

fn dll_out_dir(root: &Path, release: bool) -> PathBuf {
    client_hook_dir(root)
        .join("target")
        .join(DLL_TARGET)
        .join(if release { "release" } else { "debug" })
}

fn dll_dir(root: &Path, build: bool) -> Result<PathBuf> {
    if build {
        build_dll(root, true)
    } else {
        Ok(dll_out_dir(root, true))
    }
}

/// Repo-specific part of the deploy; the logic itself is `poe-hook-deploy` (Poe2ClientHook).
fn deploy_client(root: &Path, config: &Config, build: bool, official: bool) -> Result<()> {
    let game = &config.game.game_dir;
    let dlls = dll_dir(root, build)?;
    let report = poe_hook_deploy::deploy(&DeployOptions {
        game_dir: game.clone(),
        dll_dir: dlls,
        mode: if official { Mode::Official } else { Mode::Private },
        gateway: Some(GatewayEntry {
            region: config.gateway.region.clone(),
            display_name: config.gateway.display_name.clone(),
            host: config.login.public_host.clone(),
            port: config.login.bind.port(),
        }),
        // Position reports go to the instance server's port only.
        position_sync_port: Some(config.instance.public_port),
        // Generated outputs (terrain grids/layouts, area exports, client tables) go to
        // dumps/<version>/ in the repo.
        dump_dir: Some(root.join("dumps").join(&config.game.version)),
    })?;
    match (&report.gateway, official) {
        (Some(entry), false) => println!("gateway -> {entry}"),
        (_, true) => println!("gateway -> official (original list)"),
        _ => {}
    }
    if let Some(dir) = &report.dump_dir {
        println!("dump_dir -> {dir}");
    }
    if !official && poe_hook_deploy::gateway::select_gateway_in_prefix(
        &proton_prefix(root),
        &config.gateway.display_name,
    )? {
        println!("client gateway selection -> {}", config.gateway.display_name);
    }
    export_dll_data(config)?;
    println!(
        "deployed to {} ({})",
        game.display(),
        if official {
            "official, read-only"
        } else {
            "private server"
        }
    );
    Ok(())
}

/// The prefix `run-client` uses: never the Steam install's compatdata/2694490.
fn proton_prefix(root: &Path) -> PathBuf {
    root.join("third_party/proton-prefix")
}

/// Name tables for the DLL's Stats and Actions tabs (`GameFiles/poe-hook-data/`), from the
/// game data: `stats.tsv` (network key = `Stats.dat` row + 1, id) and `action_types.tsv`
/// (`ActionTypes` key column, id).
fn export_dll_data(config: &Config) -> Result<()> {
    let data = poe_data::GameData::open(&config.game.game_dir, &config.game.schema)?;
    let dir = config.game.game_dir.join("poe-hook-data");
    std::fs::create_dir_all(&dir)?;
    let stats = data.table("Stats")?;
    let mut out = String::from("# key\tid (Stats.dat row + 1)\n");
    for (i, row) in stats.rows().enumerate() {
        out.push_str(&format!("{}\t{}\n", i + 1, row.str("Id")?));
    }
    std::fs::write(dir.join("stats.tsv"), out)?;
    let actions = data.table("ActionTypes")?;
    let mut out = String::from("# key\tid (ActionTypes column 1)\n");
    for row in actions.rows() {
        if let Some(key) = row.get_index(1)?.as_int() {
            out.push_str(&format!("{key}\t{}\n", row.str("Id")?));
        }
    }
    std::fs::write(dir.join("action_types.tsv"), out)?;
    // Area picker of the Instances tab: every WorldAreas row with its topologies (graph file
    // and allowed orientations: bit per orientation, 0xff = all).
    let areas = data.table("WorldAreas")?;
    let mut out = String::from("# id\tname\tact\tlevel\tkind\ttopologies (graph:orientation mask, ...)\n");
    let mut n_areas = 0;
    for row in areas.rows() {
        let id = row.str("Id")?;
        if id.is_empty() {
            continue;
        }
        let kind = if row.bool("IsTown")? {
            "town"
        } else if row.bool("IsHideout")? {
            "hideout"
        } else if id.starts_with("Map") {
            "map"
        } else {
            "area"
        };
        let topologies = poe_world::area::topology_entries(&data, &id, 1)
            .unwrap_or_default()
            .into_iter()
            .map(|(graph, mask)| format!("{}:{mask:#04x}", graph.rsplit('/').next().unwrap_or(&graph)))
            .collect::<Vec<_>>()
            .join(",");
        let name = row.str("Name")?.replace(['\t', '\n'], " ");
        out.push_str(&format!("{id}\t{name}\t{}\t{}\t{kind}\t{topologies}\n", row.int("Act")?, row.int("AreaLevel")?));
        n_areas += 1;
    }
    std::fs::write(dir.join("areas.tsv"), out)?;
    println!(
        "dll data -> {} ({} stats, {} action types, {n_areas} areas)",
        dir.display(),
        stats.len(),
        actions.len()
    );
    Ok(())
}

fn deploy_core(root: &Path, config: &Config, build: bool) -> Result<()> {
    let dlls = dll_dir(root, build)?;
    let target = poe_hook_deploy::deploy_core(&config.game.game_dir, &dlls)?;
    println!(
        "installed {} (a running client reloads it)",
        target.display()
    );
    Ok(())
}

fn revert_client(config: &Config) -> Result<()> {
    for p in poe_hook_deploy::revert(&config.game.game_dir)? {
        println!("restored/removed {}", p.display());
    }
    Ok(())
}

fn run_client(root: &Path, config: &Config, proton: &str) -> Result<()> {
    ProtonLaunch {
        steam_root: poe_hook_deploy::launch::default_steam_root()?,
        proton: proton.to_owned(),
        prefix_dir: proton_prefix(root),
        game_dir: config.game.game_dir.clone(),
        // Moved so nothing is uploaded; kept for our own debugging (git-ignored).
        crash_dump_dir: Some(root.join("runtime/crash-dumps")),
    }
    .launch()
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("running {cmd:?}"))?;
    if !status.success() {
        bail!("{cmd:?} failed with {status}");
    }
    Ok(())
}
