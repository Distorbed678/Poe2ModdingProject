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

/// Files installed next to the client executable by `deploy-client`.
const DLL_NAME: &str = "version.dll";
/// The hot-reloadable debug core loaded by `version.dll`.
const CORE_DLL: &str = "poe_hook_core.dll";
const HOOK_CONFIG: &str = "poe-hook.toml";
const STEAM_APPID_FILE: &str = "steam_appid.txt";
const STEAM_APPID: &str = "2694490";
const GATEWAY_FILE: &str = "gateway_list.txt";
const CLIENT_EXE: &str = "PathOfExileSteam.exe";
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
    /// Build the client DLLs (client/: version.dll loader and the debug core) with cargo-xwin.
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
        Cmd::DeployClient {
            no_build,
            official,
        } => deploy_client(&root, &config, !no_build, official),
        Cmd::DeployCore { no_build } => deploy_core(&root, &config, !no_build),
        Cmd::RevertClient => revert_client(&config),
        Cmd::RunClient { proton } => run_client(&root, &config, &proton),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask lives in tools/xtask")
        .to_owned()
}

fn build_dll(root: &Path, release: bool) -> Result<PathBuf> {
    let xwin = root.join("Tools/cargo-xwin/bin/cargo-xwin");
    if !xwin.exists() {
        bail!("{} missing; run Tools/fetch-tools.sh", xwin.display());
    }
    let client = root.join("client");
    let mut cmd = Command::new(&xwin);
    // When invoked directly, cargo subcommands expect their own name as the first argument.
    cmd.args(["xwin", "build", "--workspace", "--target", DLL_TARGET])
        .current_dir(&client)
        .env("XWIN_CACHE_DIR", root.join("Tools/xwin-cache"));
    if release {
        cmd.arg("--release");
    }
    run(&mut cmd)?;
    let out = client
        .join("target")
        .join(DLL_TARGET)
        .join(if release { "release" } else { "debug" });
    println!("built {}", out.display());
    Ok(out)
}

fn dll_dir(root: &Path, build: bool) -> Result<PathBuf> {
    if build {
        build_dll(root, true)
    } else {
        Ok(root
            .join("client/target")
            .join(DLL_TARGET)
            .join("release"))
    }
}

fn deploy_client(root: &Path, config: &Config, build: bool, official: bool) -> Result<()> {
    let game = &config.game.game_dir;
    let dlls = dll_dir(root, build)?;

    // Gateway list: keep the original once, then write a single local entry (or put the
    // original back for official servers).
    let gateway = game.join(GATEWAY_FILE);
    backup_once(&gateway)?;
    if official {
        let orig = backup_path(&gateway);
        if orig.exists() {
            copy(&orig, &gateway)?;
        }
        println!("gateway -> official (original list)");
    } else {
        let entry = format!(
            "{};{};{}:{};{};false",
            config.gateway.region,
            config.gateway.display_name,
            config.login.public_host,
            config.login.bind.port(),
            config.login.public_host,
        );
        std::fs::write(&gateway, utf16le_with_bom(&entry))?;
        println!("gateway -> {entry}");
    }

    install(&dlls.join("poe_hook.dll"), &game.join(DLL_NAME))?;
    install(&dlls.join(CORE_DLL), &game.join(CORE_DLL))?;
    let hook_cfg = game.join(HOOK_CONFIG);
    if !hook_cfg.exists() {
        copy(&root.join("client/poe-hook").join(HOOK_CONFIG), &hook_cfg)?;
    }
    set_hook_key(
        &hook_cfg,
        None,
        "mode",
        if official { "\"official\"" } else { "\"private\"" },
    )?;
    // Position reports go to the instance server's port only.
    set_hook_key(
        &hook_cfg,
        Some("features"),
        "position_sync_port",
        &config.instance.public_port.to_string(),
    )?;
    // Test input comes from POE_HOOK_AUTO_KEYS (tools/test-run.sh), never from the user's
    // config (an Enter at 20 s opened the chat box in normal play).
    set_hook_key(&hook_cfg, Some("features"), "auto_keys", "[]")?;
    std::fs::write(game.join(STEAM_APPID_FILE), STEAM_APPID)?;
    if !official {
        select_local_gateway(root, &config.gateway.display_name)?;
    }
    println!(
        "deployed to {} ({})",
        game.display(),
        if official { "official, read-only" } else { "private server" }
    );
    Ok(())
}

/// The client remembers the last gateway (`gateway_id` in its config, in our own test prefix);
/// after an official session it would stay on an official realm that our gateway list lacks.
fn select_local_gateway(root: &Path, name: &str) -> Result<()> {
    let ini = root.join(
        "Tools/proton-prefix/pfx/drive_c/users/steamuser/Documents/My Games/Path of Exile 2/poe2_production_Config.ini",
    );
    let Ok(text) = std::fs::read_to_string(&ini) else {
        return Ok(());
    };
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            if l.starts_with("gateway_id=") {
                format!("gateway_id={name}")
            } else {
                l.to_owned()
            }
        })
        .collect();
    std::fs::write(&ini, lines.join("\n") + "\n")?;
    println!("client gateway selection -> {name}");
    Ok(())
}

fn deploy_core(root: &Path, config: &Config, build: bool) -> Result<()> {
    let dlls = dll_dir(root, build)?;
    let target = config.game.game_dir.join(CORE_DLL);
    install(&dlls.join(CORE_DLL), &target)?;
    println!("installed {} (a running client reloads it)", target.display());
    Ok(())
}

/// Sets `key = value` in `table` (`None`: the top level, before the first table) of the
/// installed `poe-hook.toml`, keeping the rest of the file (the user's settings).
fn set_hook_key(path: &Path, table: Option<&str>, key: &str, value: &str) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let line = format!("{key} = {value}");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let header = table.map(|t| format!("[{t}]"));
    // The section: from its header (or the file start) to the next header.
    let start = match &header {
        Some(h) => lines.iter().position(|l| l.trim() == h).map(|i| i + 1),
        None => Some(0),
    };
    let Some(start) = start else {
        lines.push(header.unwrap());
        lines.push(line);
        std::fs::write(path, lines.join("\n") + "\n")?;
        return Ok(());
    };
    let end = lines[start..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |i| start + i);
    let is_key = |l: &str| {
        !l.trim_start().starts_with('#')
            && l.split('=').next().is_some_and(|k| k.trim() == key)
            && l.contains('=')
    };
    match lines[start..end].iter().position(|l| is_key(l)) {
        Some(i) => lines[start + i] = line,
        None if table.is_none() => {
            // After the leading comment block.
            let at = lines[..end]
                .iter()
                .position(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
                .unwrap_or(end);
            lines.insert(at, line);
        }
        None => lines.insert(start, line),
    }
    std::fs::write(path, lines.join("\n") + "\n")?;
    Ok(())
}

fn revert_client(config: &Config) -> Result<()> {
    let game = &config.game.game_dir;
    let gateway = game.join(GATEWAY_FILE);
    let orig = backup_path(&gateway);
    if orig.exists() {
        std::fs::rename(&orig, &gateway)?;
        println!("restored {}", gateway.display());
    }
    for name in [DLL_NAME, CORE_DLL, HOOK_CONFIG, STEAM_APPID_FILE] {
        let p = game.join(name);
        if p.exists() {
            std::fs::remove_file(&p)?;
            println!("removed {}", p.display());
        }
    }
    Ok(())
}

fn run_client(root: &Path, config: &Config, proton: &str) -> Result<()> {
    let steam = PathBuf::from(std::env::var("HOME")?).join(".local/share/Steam");
    let proton_bin = steam.join("steamapps/common").join(proton).join("proton");
    if !proton_bin.exists() {
        bail!("{} not found", proton_bin.display());
    }
    // A dedicated prefix: never the Steam install's compatdata/2694490.
    let prefix = root.join("Tools/proton-prefix");
    std::fs::create_dir_all(&prefix)?;
    quarantine_crash_dumps(root, &prefix)?;
    let game = std::fs::canonicalize(&config.game.game_dir)?;
    let mut cmd = Command::new(proton_bin);
    cmd.arg("run")
        .arg(game.join(CLIENT_EXE))
        .current_dir(&game)
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", &steam)
        .env("STEAM_COMPAT_DATA_PATH", &prefix)
        .env("SteamAppId", STEAM_APPID)
        .env("SteamGameId", STEAM_APPID)
        .env("WINEDLLOVERRIDES", "version=n,b");
    run(&mut cmd)
}

/// The client uploads pending crash dumps (`PoE-*.dmp` plus `.poecap`/`.txt` side files) from
/// a helper process at startup. Move them into `data/crash-dumps/` (git-ignored) so nothing is
/// left to upload and the dumps stay available for our own debugging.
fn quarantine_crash_dumps(root: &Path, prefix: &Path) -> Result<()> {
    let users = prefix.join("pfx/drive_c/users");
    let Ok(entries) = std::fs::read_dir(&users) else {
        return Ok(());
    };
    let target = root.join("data/crash-dumps");
    for user in entries.flatten() {
        let dir = user
            .path()
            .join("Documents/My Games/Path of Exile 2/poe2_production");
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name();
            if !name.to_string_lossy().starts_with("PoE-") {
                continue;
            }
            std::fs::create_dir_all(&target)?;
            std::fs::rename(file.path(), target.join(&name))
                .with_context(|| format!("moving {}", file.path().display()))?;
            println!(
                "moved crash dump {} to data/crash-dumps",
                name.to_string_lossy()
            );
        }
    }
    Ok(())
}

fn utf16le_with_bom(text: &str) -> Vec<u8> {
    let mut out = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(".orig");
    path.with_file_name(name)
}

fn backup_once(path: &Path) -> Result<()> {
    let orig = backup_path(path);
    if path.exists() && !orig.exists() {
        std::fs::copy(path, &orig).with_context(|| format!("backing up {}", path.display()))?;
        println!("backed up {} -> {}", path.display(), orig.display());
    }
    Ok(())
}

/// Copies a DLL through a temporary file and a rename: a running client keeps its mapping of
/// the old file intact (overwriting a mapped file in place can corrupt it), and the core
/// loader only ever sees a complete file.
fn install(from: &Path, to: &Path) -> Result<()> {
    let tmp = to.with_extension("dll.tmp");
    copy(from, &tmp)?;
    std::fs::rename(&tmp, to).with_context(|| format!("renaming to {}", to.display()))?;
    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to)
        .with_context(|| format!("copying {} -> {}", from.display(), to.display()))?;
    Ok(())
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("running {cmd:?}"))?;
    if !status.success() {
        bail!("{cmd:?} failed with {status}");
    }
    Ok(())
}
