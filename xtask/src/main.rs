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
use poe_hook_deploy::{DeployOptions, GatewayEntry, LoginMode, Mode, launch::ProtonLaunch};

mod check_build;
mod fingerprint;
mod terrain_matrix;
mod tracker;
mod versions;

/// The client DLL workspace (Poe2ClientHook submodule), relative to the repo root.
const CLIENT_HOOK_DIR: &str = "modules/client-hook";
/// The client's compile target.
const DLL_TARGET: &str = "x86_64-pc-windows-msvc";
/// The sample mod (a cdylib in the client DLL workspace) and where it is installed from.
const SAMPLE_MOD_PACKAGE: &str = "poe-hook-sample-mod";
const SAMPLE_MOD_DIR: &str = "modules/client-hook/examples/sample-mod";
const SAMPLE_MOD_DLL: &str = "sample_mod.dll";

#[derive(Parser)]
#[command(about = "PoE2 server development tasks")]
struct Cli {
    /// Server config to read ports/hosts from.
    #[arg(long, default_value = poe_config::DEFAULT_PATH)]
    config: PathBuf,
    /// Proton prefix for `run-client`, `login-token`, `select-gateway` and the gateway selection
    /// of `deploy-client` (default `third_party/proton-prefix`; a second client of the local
    /// two-client test uses `third_party/proton-prefix-2`, see scripts/test-mp.sh). Never the
    /// Steam install's compatdata.
    #[arg(long, global = true)]
    prefix: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum LoginModeArg {
    Standalone,
    Steam,
}

impl From<LoginModeArg> for LoginMode {
    fn from(arg: LoginModeArg) -> Self {
        match arg {
            LoginModeArg::Standalone => LoginMode::Standalone,
            LoginModeArg::Steam => LoginMode::Steam,
        }
    }
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
        /// Observe the official servers, read-only: keep the original gateway list and set the
        /// DLL's `mode = "official"` (no crypto bypass, no injection; it only observes and
        /// logs). The DLL still patches game code in memory to observe, which the game's
        /// anti-cheat may detect: use it at your own risk and not on an account you care
        /// about. The launcher never does this. Guide: docs/guides/official-mode.md
        #[arg(long)]
        official: bool,
        /// `login_mode` written to poe-hook.toml for private deploys: `standalone` (account
        /// name + password screen, needs the login server's 0x0003 support) or `steam` (the
        /// Steam login, as before). Official deploys always write `steam`.
        #[arg(long, value_enum, default_value = "standalone")]
        login_mode: LoginModeArg,
        /// Also install the sample mod (modules/client-hook/examples/sample-mod) as enabled, like
        /// `deploy-mod`. Not with --official: the loader loads no mods in official mode.
        #[arg(long)]
        sample_mod: bool,
    },
    /// Install a mod into GameFiles/poe-hook-mods/<id>/ and enable it in state.toml (the layout
    /// the launcher and the DLL's mod loader use, modules/client-hook/docs/mod-manifest.md).
    /// Without a folder: the sample mod, built with the DLLs.
    DeployMod {
        /// Folder with the mod's mod.toml (default: the sample mod).
        dir: Option<PathBuf>,
        /// The built entry DLL (default for the sample mod: its build output; otherwise the file
        /// named by `[entry] dll` inside the folder).
        #[arg(long)]
        dll: Option<PathBuf>,
        /// Install the mod but leave it disabled (or disable it if it was enabled).
        #[arg(long)]
        disable: bool,
        /// Skip building the DLLs first.
        #[arg(long)]
        no_build: bool,
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
    /// Render docs/status/tracker.toml into the README.md tracker block.
    Tracker {
        /// Only check; exit 1 if README.md is out of date.
        #[arg(long)]
        check: bool,
    },
    /// What a game build breaks: scans an exe for every entry of the client DLL's signature
    /// table, checks the build identity against versions/<version>.toml, the opcode tables
    /// (HNC table, deserializer fingerprints), the strings the tools need and the game data the
    /// server reads, writes dumps/<version>/check-build.md and prints a summary. Exit code 0 all
    /// fine, 1 warnings, 2 something required is missing, ambiguous or different
    /// (docs/guides/new-game-version.md).
    CheckBuild {
        /// The game executable to check (read only).
        exe: PathBuf,
        /// Game directory (the folder with Bundles2/) for the data checks; default `[game]
        /// game_dir` of the config.
        #[arg(long)]
        bundles: Option<PathBuf>,
        /// Version file to check against (default: the one this exe is the build of, else the
        /// config's `[game] version`).
        #[arg(long)]
        version: Option<String>,
        /// Report file (default dumps/<version>/check-build.md).
        #[arg(long)]
        report: Option<PathBuf>,
        /// Instead of checking, write the opcode baseline `<opcodes dir>/deserializers.tsv`
        /// from this exe and the factory dumps in dumps/<version>/ghidra/.
        #[arg(long)]
        write_fingerprints: bool,
    },
    /// Copy versions/<version>.toml into the launcher (modules/launcher/versions/), which embeds
    /// byte-identical copies; with --check only compare.
    Versions {
        #[arg(long)]
        check: bool,
    },
    /// Write the terrain verification matrix (area x topology x orientation -> untested/fail/match)
    /// to docs/status/terrain-matrix.tsv from the game data and the verification outputs in
    /// dumps/<version>/terrain/vectors/ (docs/plans/terrain-all-areas.md).
    TerrainMatrix {
        /// Folder of the verification outputs (default dumps/<version>/terrain/vectors of the newest version file).
        #[arg(long)]
        dumps: Option<PathBuf>,
        /// Output file (default docs/status/terrain-matrix.tsv).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Write the web API's OpenAPI document to docs/architecture/openapi.json (the web admin
    /// and the launcher generate their types from it).
    Openapi {
        /// Only check; exit 1 if the file is out of date.
        #[arg(long)]
        check: bool,
    },
    /// Make the standalone client sign `<account>` in without a password: writes the 32-byte
    /// remember-me token (64 hex digits) and `[LOGIN] username` / `save_login` into the
    /// Proton prefix's `Documents/My Games/Path of Exile 2/` (see docs/guides/setup.md).
    LoginToken {
        /// Account name (the one the token was issued for).
        account: String,
        /// The token as 64 hex digits (`session_token` of the login server).
        token_hex: String,
    },
    /// Point the prefix's remembered gateway (`gateway_id` in `poe2_production_Config.ini`) at
    /// our server's gateway entry (`deploy-client` does it for the default prefix only).
    SelectGateway,
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
    // Needs no server config (and must work while that is being edited).
    if let Cmd::Tracker { check } = cli.cmd {
        return tracker::run(&root, check);
    }
    if let Cmd::Openapi { check } = cli.cmd {
        return openapi(&root, check);
    }
    if let Cmd::TerrainMatrix { dumps, out } = cli.cmd {
        return terrain_matrix::run(&root, dumps, out);
    }
    if let Cmd::Versions { check } = cli.cmd {
        return versions::run(&root, check);
    }
    if let Cmd::CheckBuild { exe, bundles, version, report, write_fingerprints } = cli.cmd {
        // The config is optional: it only supplies the game directory, the schema and the
        // server's version.
        let config = Config::load(&cli.config).ok();
        let code = check_build::run(
            &root,
            config.as_ref(),
            check_build::Options { exe, bundles, version, report, write_fingerprints },
        )?;
        std::process::exit(code);
    }
    let prefix = cli.prefix.map(|p| root.join(p)).unwrap_or_else(|| proton_prefix(&root));
    if let Cmd::LoginToken { account, token_hex } = &cli.cmd {
        return login_token(&prefix, account, token_hex);
    }
    let config = Config::load(&cli.config)?;
    match cli.cmd {
        Cmd::BuildDll { debug } => build_dll(&root, !debug).map(|_| ()),
        Cmd::DeployClient {
            no_build,
            official,
            login_mode,
            sample_mod,
        } => {
            if sample_mod && official {
                bail!("--sample-mod with --official: the loader loads no mods in official mode");
            }
            deploy_client(&root, &config, !no_build, official, login_mode.into(), &prefix)?;
            if sample_mod {
                deploy_mod(&root, &config, None, None, false, false)?;
            }
            Ok(())
        }
        Cmd::DeployMod {
            dir,
            dll,
            disable,
            no_build,
        } => deploy_mod(&root, &config, dir, dll, disable, !no_build),
        Cmd::DeployCore { no_build } => deploy_core(&root, &config, !no_build),
        Cmd::RevertClient => revert_client(&config),
        Cmd::DllData => export_dll_data(&config),
        Cmd::Tracker { .. }
        | Cmd::Openapi { .. }
        | Cmd::LoginToken { .. }
        | Cmd::TerrainMatrix { .. }
        | Cmd::Versions { .. }
        | Cmd::CheckBuild { .. } => {
            unreachable!("handled before the config is loaded")
        }
        Cmd::SelectGateway => {
            let name = &config.gateway.display_name;
            let changed = poe_hook_deploy::gateway::select_gateway_in_prefix(&prefix, name)?;
            println!("{}: gateway selection -> {name} ({})", prefix.display(), if changed { "changed" } else { "no config file yet" });
            Ok(())
        }
        Cmd::RunClient { proton } => run_client(&config, &proton, &prefix),
    }
}

/// Writes (or checks) `docs/architecture/openapi.json` from `poe_web_api::openapi_json`.
fn openapi(root: &Path, check: bool) -> Result<()> {
    let path = root.join("docs/architecture/openapi.json");
    let text = poe_web_api::openapi_json() + "\n";
    if check {
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            bail!("{} is out of date: run `cargo xtask openapi`", path.display());
        }
        return Ok(());
    }
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
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
    // Only the DLL crates (loader, core, the sample mod): the workspace also holds host-side crates.
    cmd.args(["xwin", "build", "-p", "poe-hook", "-p", "poe-hook-core"])
        .args(["-p", SAMPLE_MOD_PACKAGE])
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
fn deploy_client(
    root: &Path,
    config: &Config,
    build: bool,
    official: bool,
    login_mode: LoginMode,
    prefix: &Path,
) -> Result<()> {
    let game = &config.game.game_dir;
    let dlls = dll_dir(root, build)?;
    let report = poe_hook_deploy::deploy(&DeployOptions {
        game_dir: game.clone(),
        dll_dir: dlls,
        mode: if official {
            Mode::Official
        } else {
            Mode::Private
        },
        gateway: Some(GatewayEntry {
            region: config.gateway.region.clone(),
            display_name: config.gateway.display_name.clone(),
            host: config.network.public_host.clone(),
            port: config.network.login.public_port(),
        }),
        // The side channel (position reports, debug link, terrain upload) goes to the instance
        // server's port only.
        position_sync_port: Some(config.network.instance.public_port),
        // Generated outputs (terrain grids/layouts, area exports, client tables) go to
        // dumps/<version>/ in the repo.
        dump_dir: Some(root.join("dumps").join(&config.game.version)),
        // Private deploys default to the account-name + password login (WP3.1); official ones
        // are forced to the Steam login (and an empty trusted list) by the deploy.
        login_mode,
        trusted_servers: trusted_servers(config),
    })?;
    match (&report.gateway, official) {
        (Some(entry), false) => println!("gateway -> {entry}"),
        (_, true) => println!("gateway -> official (original list)"),
        _ => {}
    }
    if let Some(dir) = &report.dump_dir {
        println!("dump_dir -> {dir}");
    }
    if let Some(mode) = report.login_mode {
        println!(
            "login_mode -> {}, trusted servers -> {:?} (+ loopback)",
            mode.as_str(),
            report.trusted_servers
        );
    }
    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    if !official
        && poe_hook_deploy::gateway::select_gateway_in_prefix(
            prefix,
            &config.gateway.display_name,
        )?
    {
        println!(
            "client gateway selection -> {}",
            config.gateway.display_name
        );
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

/// Servers the DLL treats like loopback: our login and instance server at the advertised
/// host (`[network] public_host`), so a client on another machine can reach a remote cluster
/// without the read-only latch. Loopback itself is always trusted by the DLL. Host names are
/// resolved to IPv4 by the deploy.
fn trusted_servers(config: &Config) -> Vec<String> {
    let host = &config.network.public_host;
    vec![
        format!("{host}:{}", config.network.login.public_port()),
        format!("{host}:{}", config.network.instance.public_port),
    ]
}

/// `cargo xtask login-token <account> <hex32>`: remember-me token for the standalone client.
fn login_token(prefix: &Path, account: &str, token_hex: &str) -> Result<()> {
    let token = poe_hook_deploy::login_token::parse_token_hex(token_hex)?;
    let report = poe_hook_deploy::write_login_token(prefix, account, token)?;
    println!("token -> {}", report.credentials.display());
    println!(
        "[LOGIN] username={account}, save_login=true -> {}",
        report.ini.display()
    );
    println!("start the client and press Login with the password box empty");
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
    let mut out =
        String::from("# id\tname\tact\tlevel\tkind\ttopologies (graph:orientation mask, ...)\n");
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
            .map(|(graph, mask)| {
                format!("{}:{mask:#04x}", graph.rsplit('/').next().unwrap_or(&graph))
            })
            .collect::<Vec<_>>()
            .join(",");
        let name = row.str("Name")?.replace(['\t', '\n'], " ");
        out.push_str(&format!(
            "{id}\t{name}\t{}\t{}\t{kind}\t{topologies}\n",
            row.int("Act")?,
            row.int("AreaLevel")?
        ));
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

/// Installs a mod through `poe-hook-deploy` (never by hand): `poe-hook-mods/<id>/` with its
/// `mod.toml`, data and entry DLL, and the id in `state.toml`.
fn deploy_mod(
    root: &Path,
    config: &Config,
    dir: Option<PathBuf>,
    dll: Option<PathBuf>,
    disable: bool,
    build: bool,
) -> Result<()> {
    let game = &config.game.game_dir;
    // The loader loads no mod in official mode; installing one for it would be a trap.
    let hook_toml = std::fs::read_to_string(game.join(poe_hook_deploy::HOOK_CONFIG)).unwrap_or_default();
    let mode = poe_hook_deploy::hook_config::top_level_string(&hook_toml, "mode");
    if mode.as_deref() == Some("official") {
        bail!("GameFiles is deployed in official mode (read-only): the loader loads no mods there");
    }
    let sample = root.join(SAMPLE_MOD_DIR);
    let dir = dir.map_or_else(|| sample.clone(), |d| root.join(d));
    let dll = match dll {
        Some(d) => Some(root.join(d)),
        None if dir.canonicalize().ok() == sample.canonicalize().ok() => {
            Some(dll_dir(root, build)?.join(SAMPLE_MOD_DLL))
        }
        None => None,
    };
    let m = poe_hook_deploy::install_mod(game, &dir, dll.as_deref(), !disable)?;
    println!(
        "installed mod {} -> {} ({})",
        m.id,
        m.dir.display(),
        if m.enabled {
            "enabled; the client loads it at its next start"
        } else {
            "disabled"
        }
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

fn run_client(config: &Config, proton: &str, prefix: &Path) -> Result<()> {
    let root = repo_root();
    ProtonLaunch {
        steam_root: poe_hook_deploy::launch::default_steam_root()?,
        proton: proton.to_owned(),
        prefix_dir: prefix.to_path_buf(),
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
