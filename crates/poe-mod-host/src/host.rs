//! The mod host: reads the mods folder, decides which mods load, loads the native server
//! libraries, registers their commands and delivers events.
//!
//! Decisions (docs/plans/server-modding-analysis.md, "Decisions (user, 2026-10-22)"):
//! every enabled mod is loaded, the owner is responsible; the API version and the game version
//! are still checked and a refused mod is a clear log line while the server keeps starting;
//! mods are never unloaded or reloaded (a change needs a restart).
//!
//! Failure handling at the boundary: the library's exports are `extern "C"`, so a Rust panic in
//! a mod aborts the process unless the SDK catches it (it does). A callback that returns an
//! unknown code, or runs longer than `[mods] callback_budget_ms`, counts as a failure; a mod
//! with `[mods] max_failures` failures in a row is disabled for the rest of the run.

use std::{
    collections::HashSet,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Instant,
};

use poe_cluster::{ModAdvert, ModDirectory, ModPackage, ModStatus};
use poe_config::ModsConfig;
use poe_hook_mods::{CommandRole, LoadSet, Manifest, Platform, Side, check_server_api, sha256_hex};
use poe_mod_api::{
    EVENT_COMMAND, EVENT_CONSUMED, EVENT_CONTINUE, Event, MOD_ABI_VERSION, MOD_INIT, ModAbiVersion,
    ModInfo, ModInit, OnEvent, OnShutdown, PlayerInfo, SERVER_API_VERSION, ServerApi, Snapshot,
    Str, event_bit,
};

use crate::{
    check::RequiredMod,
    ffi::{self, ModCommand, ModCtx, NativeCommand, Scope, ScopeGuard},
    store::{self, Installed, PackageEntry},
};

/// What the host needs from the server to decide and to talk to mods.
#[derive(Debug, Clone)]
pub struct HostConfig {
    pub mods: ModsConfig,
    /// The game client version the server speaks (`[game] version`).
    pub game_version: String,
    pub server_version: String,
    pub server_dir: PathBuf,
    /// Command names of the server itself: a mod may not take them.
    pub reserved_commands: Vec<String>,
}

/// What a chat command does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    /// The mod's library handles it (`EVENT_COMMAND`).
    Native,
    /// A fixed reply (`{player}`, `{args}` substituted).
    Reply(String),
    /// Another command line, run as the caller.
    Alias(String),
}

/// A chat command that mods add to the server's registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    pub name: String,
    pub help: String,
    pub admin: bool,
    pub mod_id: String,
    pub action: CommandAction,
}

/// A player in a snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSnap {
    /// The player's id for this session (the actor's session id).
    pub id: u64,
    pub entity: u64,
    pub account: String,
    pub character: String,
    pub admin: bool,
    pub x: f32,
    pub y: f32,
}

/// A read-only copy of an instance.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InstanceSnap {
    pub frame: u64,
    pub instance_id: u64,
    pub area: String,
    pub players: Vec<PlayerSnap>,
}

/// One event for the mods.
#[derive(Debug, Clone, Copy)]
pub struct EventData<'a> {
    pub kind: u32,
    pub frame: u64,
    pub instance_id: u64,
    pub snapshot: Option<&'a InstanceSnap>,
    pub player: Option<&'a PlayerSnap>,
    pub text: &'a str,
    pub args: &'a str,
}

impl<'a> EventData<'a> {
    /// A server-scope event (no instance, no snapshot).
    pub fn server(kind: u32) -> Self {
        Self {
            kind,
            frame: 0,
            instance_id: 0,
            snapshot: None,
            player: None,
            text: "",
            args: "",
        }
    }
}

/// A command a mod submitted, with the mod that did.
#[derive(Debug, Clone, PartialEq)]
pub struct Submitted {
    pub mod_id: String,
    pub command: ModCommand,
}

struct SendPtr(*mut c_void);
// The mod promises its state is `Send + Sync` (the SDK enforces it).
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

struct Native {
    id: String,
    version: String,
    ctx: &'static ModCtx,
    user: SendPtr,
    mask: u64,
    on_event: Option<OnEvent>,
    shutdown: Option<OnShutdown>,
    failures: AtomicU32,
    disabled: AtomicBool,
    stopped: AtomicBool,
}

/// What a successful `poe_server_mod_init` left.
struct Started {
    ctx: &'static ModCtx,
    info: ModInfo,
}

/// Calls `init` with a fresh [`ServerApi`]. The library (if any) is the caller's business.
fn run_init(
    init: ModInit,
    ctx: &'static ModCtx,
    mod_dir: &Path,
    cfg: &HostConfig,
) -> Result<Started, String> {
    let leak = |s: &str| -> Str { Str::new(Box::leak(s.to_owned().into_boxed_str())) };
    let api: &'static ServerApi = Box::leak(Box::new(ServerApi {
        abi_version: SERVER_API_VERSION,
        size: size_of::<ServerApi>() as u32,
        ctx: (ctx as *const ModCtx).cast::<c_void>(),
        mod_id: leak(&ctx.id),
        mod_dir: leak(&mod_dir.display().to_string()),
        server_dir: leak(&cfg.server_dir.display().to_string()),
        server_version: leak(&cfg.server_version),
        game_version: leak(&cfg.game_version),
        log: ffi::api_log,
        config_get: ffi::api_config_get,
        register_command: ffi::api_register_command,
        submit: ffi::api_submit,
    }));
    let mut info = ModInfo {
        size: size_of::<ModInfo>() as u32,
        reserved: 0,
        event_mask: 0,
        user: std::ptr::null_mut(),
        on_event: None,
        shutdown: None,
    };
    ctx.init_phase.store(true, Ordering::SeqCst);
    let rc = catch_unwind(AssertUnwindSafe(|| unsafe { init(api, &mut info) }));
    ctx.init_phase.store(false, Ordering::SeqCst);
    match rc {
        Ok(0) => Ok(Started { ctx, info }),
        Ok(rc) => Err(format!("poe_server_mod_init returned {rc}")),
        Err(_) => Err("poe_server_mod_init panicked".into()),
    }
}

/// Maps a library and starts it. A mod that starts is never unloaded.
fn load_library(
    path: &Path,
    ctx: &'static ModCtx,
    cfg: &HostConfig,
    mod_dir: &Path,
) -> Result<Started, String> {
    // SAFETY: loading a library runs its initialisers; the owner enabled this mod and is
    // responsible for it (decision Q7).
    let lib = unsafe { libloading::Library::new(path) }
        .map_err(|e| format!("cannot load {}: {e}", path.display()))?;
    let abi: ModAbiVersion = *unsafe { lib.get::<ModAbiVersion>(MOD_ABI_VERSION) }
        .map_err(|_| "the library exports no poe_server_mod_abi_version".to_owned())?;
    let v = unsafe { abi() };
    if v != SERVER_API_VERSION {
        return Err(format!(
            "the library reports server API {v}, this server offers {SERVER_API_VERSION}"
        ));
    }
    let init: ModInit = *unsafe { lib.get::<ModInit>(MOD_INIT) }
        .map_err(|_| "the library exports no poe_server_mod_init".to_owned())?;
    let started = run_init(init, ctx, mod_dir, cfg)?;
    // Never unloaded: its callbacks and statics live until the process ends.
    std::mem::forget(lib);
    Ok(started)
}

/// A mod's chat commands as the registry sees them.
fn command_infos(m: &Manifest, native: &[NativeCommand]) -> Vec<CommandInfo> {
    let mut out: Vec<CommandInfo> = native
        .iter()
        .map(|c| CommandInfo {
            name: c.name.clone(),
            help: c.help.clone(),
            admin: c.admin,
            mod_id: m.id.clone(),
            action: CommandAction::Native,
        })
        .collect();
    for c in &m.commands {
        out.push(CommandInfo {
            name: c.name.clone(),
            help: c.help.clone(),
            admin: c.role == CommandRole::Admin,
            mod_id: m.id.clone(),
            action: match (&c.reply, &c.alias) {
                (Some(r), _) => CommandAction::Reply(r.clone()),
                (None, Some(a)) => CommandAction::Alias(a.clone()),
                (None, None) => CommandAction::Reply(String::new()),
            },
        });
    }
    out
}

struct Offered {
    advert: ModAdvert,
    package: PathBuf,
}

/// The mod host. Built once at startup by [`ModHost::load`]; shared (`Arc`) by the instance
/// server, the actors and the web API.
pub struct ModHost {
    cfg: ModsConfig,
    /// `state.toml` exists: the mod system is on (the login check applies).
    active: bool,
    natives: Vec<Native>,
    commands: Vec<CommandInfo>,
    status: Vec<ModStatus>,
    offered: Vec<Offered>,
    required: Vec<RequiredMod>,
    overlays: Vec<PathBuf>,
    server_only: usize,
    /// Union of the loaded mods' event masks.
    mask: u64,
    /// What the mods folder looked like when it was loaded (for `restart_required`).
    fingerprint: Fingerprint,
}

/// The enabled list and the installed mods (id, version, manifest hash).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Fingerprint {
    state_present: bool,
    enabled: Vec<String>,
    installed: Vec<(String, String, String)>,
}

fn fingerprint(scan: &store::Scan) -> Fingerprint {
    Fingerprint {
        state_present: scan.state_present,
        enabled: scan.enabled.clone(),
        installed: scan
            .installed
            .iter()
            .map(|m| {
                (
                    m.id.clone(),
                    m.manifest.version.clone(),
                    m.manifest_sha256.clone(),
                )
            })
            .collect(),
    }
}

impl ModHost {
    /// A host with no mods and the login check off.
    pub fn empty() -> Self {
        Self {
            cfg: ModsConfig::default(),
            active: false,
            natives: Vec::new(),
            commands: Vec::new(),
            status: Vec::new(),
            offered: Vec::new(),
            required: Vec::new(),
            overlays: Vec::new(),
            server_only: 0,
            mask: 0,
            fingerprint: Fingerprint::default(),
        }
    }

    /// Reads the mods folder and loads the enabled mods. Never fails: a mod that cannot load
    /// is logged and skipped.
    pub fn load(cfg: &HostConfig) -> Self {
        let mut host = Self::empty();
        host.cfg = cfg.mods.clone();
        let scan = store::scan(&cfg.mods.dir);
        for n in &scan.notes {
            tracing::error!("mods: {n}");
        }
        host.active = scan.state_present;
        host.fingerprint = fingerprint(&scan);
        if !scan.state_present {
            tracing::debug!(dir = %cfg.mods.dir.display(), "mods: no state.toml, the mod system is off");
            return host;
        }
        let (packages, bad_packages) = store::packages(&cfg.mods.dir);
        for (path, why) in &bad_packages {
            tracing::error!(
                "mods: package {} refused: {}",
                path.display(),
                why.join("; ")
            );
        }
        for (folder, why) in &scan.broken {
            tracing::error!("mods: {folder}: {why}");
        }
        host.plan(cfg, &scan, &packages);
        let loaded = host.natives.len();
        let data_only = host
            .status
            .iter()
            .filter(|s| s.state == "loaded")
            .count()
            .saturating_sub(loaded);
        let skipped = host.status.iter().filter(|s| s.state == "skipped").count();
        tracing::info!(
            "mods: {} enabled: {loaded} native, {data_only} without a library, {} offered to clients, {skipped} skipped",
            scan.enabled.len(),
            host.offered.len()
        );
        host
    }

    fn plan(&mut self, cfg: &HostConfig, scan: &store::Scan, packages: &[PackageEntry]) {
        let mut set = LoadSet::default();
        let mut seen: HashSet<&str> = HashSet::new();
        // Mods that are for the other program only still satisfy dependencies of `both` mods.
        for id in &scan.enabled {
            if let Some(m) = scan.find(id)
                && !m.manifest.side.includes_server()
            {
                set.add_foreign(&m.manifest);
            }
        }
        let taken: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(
            cfg.reserved_commands
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .collect(),
        ));
        let mut statuses: Vec<ModStatus> = Vec::new();
        for id in &scan.enabled {
            let mut status =
                |state: &str, detail: String, m: Option<&Installed>, commands: Vec<String>| {
                    statuses.push(ModStatus {
                        id: id.clone(),
                        name: m.map_or_else(|| id.clone(), |m| m.manifest.name.clone()),
                        version: m.map_or_else(String::new, |m| m.manifest.version.clone()),
                        side: m.map_or_else(String::new, |m| m.manifest.side.as_str().to_owned()),
                        enabled: true,
                        state: state.to_owned(),
                        detail,
                        commands,
                    })
                };
            if !seen.insert(id) {
                tracing::error!("mods: skipped {id}: listed twice in state.toml");
                continue;
            }
            let Some(m) = scan.find(id) else {
                let why = match scan.broken.iter().find(|(f, _)| f == id) {
                    Some((_, why)) => why.clone(),
                    None => "enabled in state.toml but not installed".to_owned(),
                };
                tracing::error!("mods: skipped {id}: {why}");
                status("skipped", why, None, Vec::new());
                continue;
            };
            let man = &m.manifest;
            match self.plan_one(cfg, m, &set, &taken, packages) {
                Ok(planned) => {
                    set.add(man);
                    let names: Vec<String> =
                        planned.commands.iter().map(|c| c.name.clone()).collect();
                    let note = planned.note.clone();
                    tracing::info!(
                        "mods: loaded {} {} (side {}, {} commands, {} ms){}",
                        man.id,
                        man.version,
                        man.side.as_str(),
                        names.len(),
                        planned.took_ms,
                        if note.is_empty() {
                            String::new()
                        } else {
                            format!("; {note}")
                        }
                    );
                    status("loaded", note, Some(m), names);
                    self.commands.extend(planned.commands);
                    self.overlays.extend(planned.overlays);
                    if let Some(n) = planned.native {
                        self.mask |= n.mask;
                        self.natives.push(n);
                    }
                    if let Some(o) = planned.offered {
                        self.required.push(o.0);
                        self.offered.push(o.1);
                    }
                    if !man.side.includes_client() {
                        self.server_only += 1;
                    }
                }
                Err(why) => {
                    tracing::error!("mods: skipped {}: {why}", man.id);
                    status("skipped", why, Some(m), Vec::new());
                }
            }
        }
        self.status.extend(statuses);
        // Installed but not enabled.
        for m in &scan.installed {
            if !scan.enabled.contains(&m.id) {
                self.status.push(ModStatus {
                    id: m.id.clone(),
                    name: m.manifest.name.clone(),
                    version: m.manifest.version.clone(),
                    side: m.manifest.side.as_str().to_owned(),
                    enabled: false,
                    state: "installed".into(),
                    detail: String::new(),
                    commands: Vec::new(),
                });
            }
        }
    }

    /// Every check and load step for one enabled mod. `Err` is the reason it is skipped.
    fn plan_one(
        &self,
        cfg: &HostConfig,
        m: &Installed,
        set: &LoadSet,
        taken: &Arc<Mutex<HashSet<String>>>,
        packages: &[PackageEntry],
    ) -> Result<Planned, String> {
        let started = Instant::now();
        let man = &m.manifest;
        let mut planned = Planned::default();
        for k in &m.unknown_keys {
            tracing::warn!("mods: {}: unknown key {k} in mod.toml ignored", man.id);
        }
        if man.side.includes_server() {
            if !man.game_versions.is_empty() && !man.game_versions.contains(&cfg.game_version) {
                return Err(format!(
                    "made for client {}, this server speaks {}",
                    man.game_versions.join(", "),
                    cfg.game_version
                ));
            }
            check_server_api(man, SERVER_API_VERSION)?;
        }
        // Dependencies and conflicts: the mod must be able to load given what loaded before.
        if man.side != Side::Client {
            set.check(man)?;
        }
        // The client half: what the login check requires and what is offered for download.
        if man.side.includes_client() {
            let dll_hash = match &man.entry_dll {
                Some(rel) => Some(sha256_hex(&std::fs::read(m.dir.join(rel)).map_err(
                    |e| format!("the client half is incomplete: cannot read {rel}: {e}"),
                )?)),
                None => None,
            };
            let required = RequiredMod {
                id: man.id.clone(),
                name: man.name.clone(),
                version: man.version.clone(),
                manifest_sha256: m.manifest_sha256.clone(),
                dll_sha256: dll_hash,
            };
            let package = self.ensure_package(cfg, m, packages)?;
            planned.offered = Some((
                required,
                Offered {
                    advert: ModAdvert {
                        id: man.id.clone(),
                        name: man.name.clone(),
                        version: man.version.clone(),
                        side: man.side.as_str().to_owned(),
                        sha256: package.sha256.clone(),
                        size: package.size,
                        required: true,
                        hook_api: man.hook_api,
                        game_versions: man.game_versions.clone(),
                        contains_code: man.entry_dll.is_some(),
                        description: man.description.clone(),
                        authors: man.authors.clone(),
                    },
                    package: package.path.clone(),
                },
            ));
        }
        if man.side.includes_server() {
            // Data tier.
            for rel in &man.server_tables {
                planned.overlays.push(m.dir.join(rel));
            }
            if !man.server_opcodes.is_empty() {
                tracing::warn!("mods: {}: server opcodes are not applied yet", man.id);
                planned.note = "server opcodes not yet supported".into();
            }
            if man.server_config_defaults.is_some() {
                tracing::warn!("mods: {}: config_defaults are not applied yet", man.id);
                planned.note = "config_defaults not yet supported".into();
            }
            // Native library.
            let mut native_cmds = Vec::new();
            if man.has_server_library() {
                let platform =
                    Platform::current().ok_or("this platform has no server mod libraries")?;
                let rel = man.server_library(platform).ok_or_else(|| {
                    format!(
                        "no server library for {} (the mod ships {})",
                        platform.as_str(),
                        {
                            let mut v = Vec::new();
                            v.extend(man.server_linux.as_ref().map(|_| "linux"));
                            v.extend(man.server_windows.as_ref().map(|_| "windows"));
                            v.join(", ")
                        }
                    )
                })?;
                let path = m.dir.join(rel);
                if !path.is_file() {
                    return Err(format!("the server library {rel:?} is missing"));
                }
                let ctx: &'static ModCtx = Box::leak(Box::new(ModCtx {
                    id: man.id.clone(),
                    config: cfg.mods.config.get(&man.id).cloned().unwrap_or_default(),
                    init_phase: AtomicBool::new(false),
                    commands: Mutex::default(),
                    taken: taken.clone(),
                }));
                let s = load_library(&path, ctx, cfg, &m.dir)?;
                native_cmds = ctx.commands.lock().unwrap().clone();
                planned.native = Some(native_from(man, s));
            }
            // Data-tier commands: the same name rules as native ones; a taken name is ignored.
            let mut rejected = HashSet::new();
            for c in &man.commands {
                if !taken.lock().unwrap().insert(c.name.clone()) {
                    tracing::error!(
                        "mods: {}: command {} is taken by the server or another mod: ignored",
                        man.id,
                        c.name
                    );
                    rejected.insert(c.name.clone());
                }
            }
            planned.commands = command_infos(man, &native_cmds)
                .into_iter()
                .filter(|c| c.action == CommandAction::Native || !rejected.contains(&c.name))
                .collect();
        }
        planned.took_ms = started.elapsed().as_millis() as u64;
        Ok(planned)
    }

    /// The package of an offered mod: the verified file in `packages/`, or one packed from the
    /// folder when missing or out of date.
    fn ensure_package(
        &self,
        cfg: &HostConfig,
        m: &Installed,
        packages: &[PackageEntry],
    ) -> Result<PackageEntry, String> {
        let man = &m.manifest;
        let have = packages.iter().find(|p| {
            p.id == man.id && p.version == man.version && p.manifest_sha256 == m.manifest_sha256
        });
        if let Some(p) = have {
            return Ok(p.clone());
        }
        let stale = packages
            .iter()
            .any(|p| p.id == man.id && p.version == man.version);
        let bytes = store::pack_folder(&m.dir)
            .map_err(|e| format!("cannot pack the mod for download: {e}"))?;
        let path = store::write_package(&cfg.mods.dir, &man.id, &man.version, &bytes)?;
        tracing::info!(
            "mods: {} {}: {} the package {}",
            man.id,
            man.version,
            if stale {
                "repacked (it differed from the installed folder)"
            } else {
                "packed"
            },
            path.display()
        );
        store::read_package(&path)
            .map_err(|e| format!("the packed mod does not verify: {}", e.join("; ")))
    }

    /// A host whose mods are Rust functions instead of libraries: for tests and for embedding.
    /// Each `(id, init)` gets what a loaded library would (a `ServerApi`, its config from
    /// `cfg`, registered commands), as a mod of side `server`.
    pub fn from_inits(inits: &[(&str, ModInit)], cfg: ModsConfig) -> Self {
        let mut host = Self::empty();
        host.cfg = cfg.clone();
        host.active = true;
        host.fingerprint = fingerprint(&store::scan(&cfg.dir));
        let hc = HostConfig {
            mods: cfg,
            game_version: String::new(),
            server_version: String::new(),
            server_dir: PathBuf::from("."),
            reserved_commands: vec!["help".into()],
        };
        let taken: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(["help".to_owned()].into()));
        for (id, init) in inits {
            let ctx: &'static ModCtx = Box::leak(Box::new(ModCtx {
                id: (*id).to_owned(),
                config: hc.mods.config.get(*id).cloned().unwrap_or_default(),
                init_phase: AtomicBool::new(false),
                commands: Mutex::default(),
                taken: taken.clone(),
            }));
            let s = match run_init(*init, ctx, Path::new("."), &hc) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("mods: {id}: {e}");
                    continue;
                }
            };
            let man = Manifest {
                id: (*id).to_owned(),
                version: "1.0.0".into(),
                side: Side::Server,
                ..Default::default()
            };
            let cmds = ctx.commands.lock().unwrap().clone();
            host.commands.extend(command_infos(&man, &cmds));
            let n = native_from(&man, s);
            host.mask |= n.mask;
            host.natives.push(n);
        }
        host
    }

    // ---- accessors ----

    /// The login check applies (`[mods] require_client_mods` and a `state.toml` exists).
    pub fn check_enabled(&self) -> bool {
        self.active && self.cfg.require_client_mods
    }

    pub fn allow_extra_client_mods(&self) -> bool {
        self.cfg.allow_extra_client_mods
    }

    pub fn report_wait_secs(&self) -> u64 {
        self.cfg.report_wait_secs
    }

    pub fn extra_urls(&self) -> &[String] {
        &self.cfg.extra_urls
    }

    /// The client mods the server requires (side client or both, loaded).
    pub fn required(&self) -> &[RequiredMod] {
        &self.required
    }

    /// Chat commands that mods add (native and data-tier).
    pub fn commands(&self) -> &[CommandInfo] {
        &self.commands
    }

    /// Directories of `[server] tables` overlays, in load order, for `GameFiles::push_override`.
    pub fn overlay_dirs(&self) -> &[PathBuf] {
        &self.overlays
    }

    /// Whether any loaded mod subscribed to events of `kind` (a cheap check for the tick).
    pub fn wants(&self, kind: u32) -> bool {
        kind == EVENT_COMMAND && !self.natives.is_empty() || self.mask & event_bit(kind) != 0
    }

    /// Loaded native mods: (id, version).
    pub fn native_mods(&self) -> Vec<(String, String)> {
        self.natives
            .iter()
            .map(|n| (n.id.clone(), n.version.clone()))
            .collect()
    }

    // ---- events ----

    /// Delivers `ev` to every mod that subscribed to its kind and returns what they submitted.
    /// Runs the callbacks on the calling thread, one after the other.
    pub fn dispatch(&self, ev: &EventData<'_>) -> Vec<Submitted> {
        let mut out = Vec::new();
        for n in &self.natives {
            if n.mask & event_bit(ev.kind) == 0 {
                continue;
            }
            self.call(n, ev, &mut out);
        }
        out
    }

    /// Delivers a chat command (`EVENT_COMMAND`) to the mod that registered it.
    pub fn dispatch_command(&self, mod_id: &str, ev: &EventData<'_>) -> Vec<Submitted> {
        let mut out = Vec::new();
        if let Some(n) = self.natives.iter().find(|n| n.id == mod_id) {
            self.call(n, ev, &mut out);
        }
        out
    }

    fn call(&self, n: &Native, ev: &EventData<'_>, out: &mut Vec<Submitted>) {
        let Some(on_event) = n.on_event else { return };
        if n.disabled.load(Ordering::Relaxed) || n.stopped.load(Ordering::Relaxed) {
            return;
        }
        // The FFI view of the event; the vectors keep the strings alive for the call.
        let players: Vec<PlayerInfo> = ev
            .snapshot
            .map_or_else(Vec::new, |s| s.players.iter().map(player_info).collect());
        let snapshot = ev.snapshot.map(|s| Snapshot {
            size: size_of::<Snapshot>() as u32,
            reserved: 0,
            frame: s.frame,
            instance_id: s.instance_id,
            area: Str::new(&s.area),
            players: players.as_ptr(),
            player_count: players.len(),
        });
        let player = ev.player.map(player_info);
        let scope = Scope {
            ctx: n.ctx,
            queue: Default::default(),
        };
        let event = Event {
            size: size_of::<Event>() as u32,
            kind: ev.kind,
            frame: ev.frame,
            instance_id: ev.instance_id,
            scope: (&scope as *const Scope).cast::<c_void>(),
            snapshot: snapshot
                .as_ref()
                .map_or(std::ptr::null(), std::ptr::from_ref),
            player: player.as_ref().map_or(std::ptr::null(), std::ptr::from_ref),
            text: Str::new(ev.text),
            args: Str::new(ev.args),
        };
        let started = Instant::now();
        let result = {
            let _guard = ScopeGuard::enter(&scope);
            catch_unwind(AssertUnwindSafe(|| on_event(n.user.0, &event)))
        };
        let took = started.elapsed();
        let over = self.cfg.callback_budget_ms > 0
            && took.as_millis() as u64 > self.cfg.callback_budget_ms;
        let failed = match result {
            Ok(EVENT_CONTINUE | EVENT_CONSUMED) => false,
            Ok(rc) => {
                tracing::error!(
                    "mods: {}: callback for event {} returned {rc}",
                    n.id,
                    ev.kind
                );
                true
            }
            Err(_) => {
                tracing::error!("mods: {}: callback for event {} panicked", n.id, ev.kind);
                true
            }
        };
        if over {
            tracing::warn!(
                "mods: {}: callback for event {} took {} ms (budget {} ms)",
                n.id,
                ev.kind,
                took.as_millis(),
                self.cfg.callback_budget_ms
            );
        }
        self.record(n, failed || over);
        out.extend(
            scope
                .queue
                .into_inner()
                .into_iter()
                .map(|command| Submitted {
                    mod_id: n.id.clone(),
                    command,
                }),
        );
    }

    fn record(&self, n: &Native, failed: bool) {
        if !failed {
            n.failures.store(0, Ordering::Relaxed);
            return;
        }
        let count = n.failures.fetch_add(1, Ordering::Relaxed) + 1;
        let max = self.cfg.max_failures;
        if max > 0 && count >= max && !n.disabled.swap(true, Ordering::Relaxed) {
            tracing::error!(
                "mods: {} failed {count} callbacks in a row: disabled until the next start",
                n.id
            );
        }
    }

    /// Whether a mod was disabled at run time after repeated failures.
    pub fn is_disabled(&self, mod_id: &str) -> bool {
        self.natives
            .iter()
            .any(|n| n.id == mod_id && n.disabled.load(Ordering::Relaxed))
    }

    /// Calls every mod's `shutdown` once. Mods are not unloaded.
    pub fn shutdown(&self) {
        for n in &self.natives {
            if n.stopped.swap(true, Ordering::SeqCst) {
                continue;
            }
            if let Some(f) = n.shutdown {
                tracing::info!("mods: shutting down {}", n.id);
                if catch_unwind(AssertUnwindSafe(|| f(n.user.0))).is_err() {
                    tracing::error!("mods: {}: shutdown panicked", n.id);
                }
            }
        }
    }
}

#[derive(Default)]
struct Planned {
    native: Option<Native>,
    commands: Vec<CommandInfo>,
    overlays: Vec<PathBuf>,
    offered: Option<(RequiredMod, Offered)>,
    note: String,
    took_ms: u64,
}

fn native_from(man: &Manifest, s: Started) -> Native {
    Native {
        id: man.id.clone(),
        version: man.version.clone(),
        ctx: s.ctx,
        user: SendPtr(s.info.user),
        mask: s.info.event_mask & poe_mod_api::EVENTS_ALL,
        on_event: s.info.on_event,
        shutdown: s.info.shutdown,
        failures: AtomicU32::new(0),
        disabled: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
    }
}

fn player_info(p: &PlayerSnap) -> PlayerInfo {
    PlayerInfo {
        size: size_of::<PlayerInfo>() as u32,
        is_admin: u32::from(p.admin),
        id: p.id,
        entity: p.entity,
        account: Str::new(&p.account),
        character: Str::new(&p.character),
        x: p.x,
        y: p.y,
    }
}

impl ModDirectory for ModHost {
    fn adverts(&self) -> Vec<ModAdvert> {
        self.offered.iter().map(|o| o.advert.clone()).collect()
    }

    fn server_only(&self) -> usize {
        self.server_only
    }

    fn package(&self, id: &str, version: &str) -> Option<ModPackage> {
        let o = self
            .offered
            .iter()
            .find(|o| o.advert.id == id && o.advert.version == version)?;
        let bytes = std::fs::read(&o.package).ok()?;
        // The file may have been replaced since startup: serve only what was verified.
        if sha256_hex(&bytes) != o.advert.sha256 {
            tracing::error!(path = %o.package.display(), "mods: the package changed on disk since startup: not served");
            return None;
        }
        Some(ModPackage {
            sha256: o.advert.sha256.clone(),
            bytes,
        })
    }

    fn package_sha256(&self, id: &str, version: &str) -> Option<String> {
        self.offered
            .iter()
            .find(|o| o.advert.id == id && o.advert.version == version)
            .map(|o| o.advert.sha256.clone())
    }

    fn active(&self) -> bool {
        self.active
    }

    fn restart_required(&self) -> bool {
        fingerprint(&store::scan(&self.cfg.dir)) != self.fingerprint
    }

    fn status(&self) -> Vec<ModStatus> {
        self.status
            .iter()
            .map(|s| {
                let mut s = s.clone();
                if s.state == "loaded" && self.is_disabled(&s.id) {
                    s.state = "failed".into();
                    s.detail = "disabled after repeated callback failures".into();
                }
                s
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poe_mod_api::{
        COMMAND_SEND_CHAT, Command, CommandSpec, EVENT_PLAYER_JOIN, EVENT_SERVER_START, EVENT_TICK,
        EVENTS_ALL, ROLE_PLAYER,
    };
    use std::sync::atomic::AtomicUsize;

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn greeter_event(user: *mut c_void, event: *const Event) -> i32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        let api = unsafe { &*(user as *const ServerApi) };
        let e = unsafe { &*event };
        if e.kind == EVENT_COMMAND {
            let who = unsafe { &*e.player };
            let mut cmd = Command::new(COMMAND_SEND_CHAT);
            cmd.target = who.id;
            let text = format!("hello {} ({})", unsafe { who.character.as_str() }, unsafe {
                e.args.as_str()
            });
            cmd.text = Str::new(&text);
            assert_eq!((api.submit)(api.ctx, e.scope, &cmd), 0);
        }
        if e.kind == EVENT_PLAYER_JOIN {
            let snap = unsafe { &*e.snapshot };
            assert_eq!(unsafe { snap.area.as_str() }, "1_1_1");
            assert_eq!(unsafe { snap.players() }.len(), 2);
        }
        EVENT_CONTINUE
    }

    extern "C" fn greeter_init(api: *const ServerApi, info: *mut ModInfo) -> i32 {
        let a = unsafe { &*api };
        let spec = CommandSpec {
            size: size_of::<CommandSpec>() as u32,
            role: ROLE_PLAYER,
            name: Str::new("hello"),
            help: Str::new("Greets you."),
        };
        assert_eq!((a.register_command)(a.ctx, &spec), 0);
        let i = unsafe { &mut *info };
        i.user = api as *mut c_void;
        i.event_mask = event_bit(EVENT_PLAYER_JOIN);
        i.on_event = Some(greeter_event);
        0
    }

    extern "C" fn failing_event(_: *mut c_void, _: *const Event) -> i32 {
        42
    }

    extern "C" fn failing_init(_: *const ServerApi, info: *mut ModInfo) -> i32 {
        let i = unsafe { &mut *info };
        i.event_mask = EVENTS_ALL;
        i.on_event = Some(failing_event);
        0
    }

    extern "C" fn refusing_init(_: *const ServerApi, _: *mut ModInfo) -> i32 {
        7
    }

    fn player(id: u64, name: &str) -> PlayerSnap {
        PlayerSnap {
            id,
            entity: 12_000 + id,
            account: format!("acc{id}"),
            character: name.into(),
            admin: false,
            x: 1.0,
            y: 2.0,
        }
    }

    fn cfg() -> ModsConfig {
        ModsConfig {
            max_failures: 3,
            ..Default::default()
        }
    }

    #[test]
    fn a_mod_registers_a_command_gets_events_and_submits_commands() {
        let host = ModHost::from_inits(&[("greeter", greeter_init)], cfg());
        assert_eq!(host.commands().len(), 1);
        assert_eq!(host.commands()[0].name, "hello");
        assert_eq!(host.commands()[0].action, CommandAction::Native);
        assert!(host.wants(EVENT_PLAYER_JOIN));
        assert!(!host.wants(EVENT_TICK));
        assert!(
            host.wants(EVENT_COMMAND),
            "a mod with commands always gets them"
        );
        // A subscribed event with a snapshot.
        let snap = InstanceSnap {
            frame: 5,
            instance_id: 9,
            area: "1_1_1".into(),
            players: vec![player(1, "Ann"), player(2, "Bob")],
        };
        let before = CALLS.load(Ordering::SeqCst);
        let ev = EventData {
            kind: EVENT_PLAYER_JOIN,
            frame: 5,
            instance_id: 9,
            snapshot: Some(&snap),
            player: Some(&snap.players[1]),
            text: "",
            args: "",
        };
        assert!(host.dispatch(&ev).is_empty());
        assert_eq!(CALLS.load(Ordering::SeqCst), before + 1);
        // An event it did not subscribe to is not delivered.
        host.dispatch(&EventData::server(EVENT_SERVER_START));
        assert_eq!(CALLS.load(Ordering::SeqCst), before + 1);
        // The command: delivered to the owner, answered by a submitted chat line.
        let cmd = EventData {
            kind: EVENT_COMMAND,
            frame: 5,
            instance_id: 9,
            snapshot: Some(&snap),
            player: Some(&snap.players[0]),
            text: "hello",
            args: "world",
        };
        let out = host.dispatch_command("greeter", &cmd);
        assert_eq!(
            out,
            [Submitted {
                mod_id: "greeter".into(),
                command: ModCommand::SendChat {
                    target: 1,
                    text: "hello Ann (world)".into()
                }
            }]
        );
        assert!(host.dispatch_command("nobody", &cmd).is_empty());
    }

    #[test]
    fn a_mod_that_keeps_failing_is_disabled_and_a_success_resets_the_count() {
        let host = ModHost::from_inits(&[("flaky", failing_init)], cfg());
        let ev = EventData::server(EVENT_SERVER_START);
        for _ in 0..2 {
            host.dispatch(&ev);
        }
        assert!(!host.is_disabled("flaky"));
        host.dispatch(&ev);
        assert!(host.is_disabled("flaky"), "three failures in a row");
        // Disabled mods are not called any more.
        host.dispatch(&ev);
        assert_eq!(host.natives[0].failures.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn init_failures_and_duplicate_command_names_are_errors_not_panics() {
        let ctx: &'static ModCtx = Box::leak(Box::new(ModCtx {
            id: "refuser".into(),
            config: Default::default(),
            init_phase: AtomicBool::new(false),
            commands: Mutex::default(),
            taken: Arc::default(),
        }));
        let hc = HostConfig {
            mods: cfg(),
            game_version: "x".into(),
            server_version: "y".into(),
            server_dir: ".".into(),
            reserved_commands: vec![],
        };
        let err = run_init(refusing_init, ctx, Path::new("."), &hc)
            .err()
            .unwrap();
        assert!(err.contains("returned 7"), "{err}");
        // Two mods with the same command: the second one's register fails (the mod decides).
        let host = ModHost::from_inits(&[("one", greeter_init)], cfg());
        assert_eq!(host.commands().len(), 1);
    }

    #[test]
    fn load_plans_a_mods_folder_from_disk() {
        use crate::store::{self, tests::write_mod};
        use poe_cluster::ModDirectory;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // A data-only server mod with a command, a server-only mod made for another game
        // version, a `both` mod with a client DLL and an overlay folder, and one that is not enabled.
        write_mod(
            dir,
            "data-mod",
            "1.0.0",
            "[[commands]]\nname = \"motd\"\nhelp = \"Shows it.\"\nreply = \"Hi {player}\"\n[[commands]]\nname = \"help\"\nhelp = \"taken\"\nreply = \"x\"",
        );
        write_mod(dir, "old-mod", "1.0.0", "");
        let p = dir.join("old-mod/mod.toml");
        std::fs::write(
            &p,
            std::fs::read_to_string(&p)
                .unwrap()
                .replace("0.5.5e", "0.4.0"),
        )
        .unwrap();
        let both = dir.join("both-mod");
        std::fs::create_dir_all(both.join("client")).unwrap();
        std::fs::create_dir_all(both.join("data/tables")).unwrap();
        std::fs::write(both.join("client/c.dll"), b"MZ").unwrap();
        std::fs::write(both.join("data/tables/x.txt"), b"x").unwrap();
        std::fs::write(
            both.join("mod.toml"),
            "manifest_version = 2\nid = \"both-mod\"\nname = \"Both\"\nversion = \"0.2.0\"\nside = \"both\"\n[client]\ndll = \"client/c.dll\"\nhook_api = 1\n[server]\ntables = [\"data/tables\"]\n",
        )
        .unwrap();
        write_mod(
            dir,
            "idle-mod",
            "1.0.0",
            "[[commands]]\nname = \"idle\"\nhelp = \"x\"\nreply = \"x\"",
        );
        store::write_state(
            dir,
            &[
                "data-mod".into(),
                "old-mod".into(),
                "both-mod".into(),
                "ghost-mod".into(),
            ],
        )
        .unwrap();
        let mods = ModsConfig {
            dir: dir.to_owned(),
            ..Default::default()
        };
        let host = ModHost::load(&HostConfig {
            mods,
            game_version: "0.5.5e".into(),
            server_version: "0.0.0".into(),
            server_dir: dir.to_owned(),
            reserved_commands: vec!["help".into(), "who".into()],
        });
        assert!(host.check_enabled());
        let status: Vec<_> = host.status().into_iter().map(|s| (s.id, s.state)).collect();
        assert_eq!(
            status,
            [
                ("data-mod".to_owned(), "loaded".to_owned()),
                ("old-mod".to_owned(), "skipped".to_owned()),
                ("both-mod".to_owned(), "loaded".to_owned()),
                ("ghost-mod".to_owned(), "skipped".to_owned()),
                ("idle-mod".to_owned(), "installed".to_owned()),
            ]
        );
        // Only the data-mod's free command is registered; `help` belongs to the server.
        assert_eq!(
            host.commands()
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["motd"]
        );
        assert_eq!(
            host.commands()[0].action,
            CommandAction::Reply("Hi {player}".into())
        );
        // The both mod is offered, required, packed on the spot and served by hash.
        let adverts = host.adverts();
        assert_eq!(adverts.len(), 1);
        let a = &adverts[0];
        assert_eq!(
            (a.id.as_str(), a.version.as_str(), a.side.as_str()),
            ("both-mod", "0.2.0", "both")
        );
        assert!(a.contains_code && a.required);
        assert_eq!(a.hook_api, Some(1));
        let pkg = host.package("both-mod", "0.2.0").unwrap();
        assert_eq!(pkg.sha256, a.sha256);
        assert_eq!(sha256_hex(&pkg.bytes), a.sha256);
        assert_eq!(pkg.bytes.len() as u64, a.size);
        assert!(host.package("both-mod", "9.9.9").is_none());
        assert!(
            host.package("data-mod", "1.0.0").is_none(),
            "server-only mods are not served"
        );
        assert_eq!(host.server_only(), 1);
        let req = &host.required()[0];
        assert_eq!(req.dll_sha256.as_deref(), Some(sha256_hex(b"MZ").as_str()));
        assert_eq!(host.overlay_dirs(), [both.join("data/tables")]);
        // A change on disk after the load is reported as needing a restart.
        assert!(host.active());
        assert!(!host.restart_required());
        store::set_enabled(dir, "idle-mod", true).unwrap();
        assert!(host.restart_required());
        store::set_enabled(dir, "idle-mod", false).unwrap();
        assert!(!host.restart_required());
        // The package on disk verifies like any other.
        let (pkgs, bad) = store::packages(dir);
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(pkgs.len(), 1);
        // A tampered package is no longer served.
        let path = pkgs[0].path.clone();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.push(0);
        std::fs::write(&path, bytes).unwrap();
        assert!(host.package("both-mod", "0.2.0").is_none());
        // A second start repacks the (now invalid) package from the folder and serves again.
        let again = ModHost::load(&HostConfig {
            mods: ModsConfig {
                dir: dir.to_owned(),
                ..Default::default()
            },
            game_version: "0.5.5e".into(),
            server_version: "0.0.0".into(),
            server_dir: dir.to_owned(),
            reserved_commands: vec![],
        });
        assert!(again.package("both-mod", "0.2.0").is_some());
        assert_eq!(
            again.adverts()[0].sha256,
            a.sha256,
            "packing is reproducible"
        );
    }

    #[test]
    fn shutdown_runs_once() {
        static DOWN: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn down(_: *mut c_void) {
            DOWN.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn init(_: *const ServerApi, info: *mut ModInfo) -> i32 {
            unsafe { (*info).shutdown = Some(down) };
            0
        }
        let host = ModHost::from_inits(&[("closer", init)], cfg());
        host.shutdown();
        host.shutdown();
        assert_eq!(DOWN.load(Ordering::SeqCst), 1);
    }
}
