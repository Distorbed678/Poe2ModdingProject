mod lifecycle;

use std::sync::Arc;

use login_server::LoginServer;
use poe_cluster::{MemoryInstances, MemorySessions, MemoryTickets};
use poe_config::Config;
use poe_data::GameData;
use poe_proto::Protocol;
use poe_store::SqliteStore;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    lifecycle::wait_for_parent().await;
    let config = Arc::new(Config::load_default()?);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.logging.filter));
    // The web API keeps recent events for its live log viewer (`[web] log_buffer`).
    let logs = poe_web_api::LogBuffer::new(config.web.log_buffer);
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(logs.layer())
        .init();
    tracing::info!(version = %config.game.version, "starting poe-server");
    if let Some(v) = &config.version_info {
        tracing::info!(
            build_revision = %v.build_revision,
            status = ?v.status,
            opcodes = %config.game.protocol_dir.display(),
            client_hashes = v.client.hashes.len(),
            hashes_confirmed = v.client.hashes_confirmed,
            allow_unknown_client = config.security.allow_unknown_client,
            "client build (versions/{}.toml)",
            v.version
        );
        if v.status != poe_config::versions::Status::Supported {
            tracing::warn!("client {} is `{:?}`, not a supported build", v.version, v.status);
        }
    }
    for w in config.network.warnings().into_iter().chain(config.security_warnings()) {
        tracing::warn!("{w}");
    }

    let protocol = Arc::new(Protocol::load(&config.game.protocol_dir)?);
    let store = Arc::new(SqliteStore::open(&config.database.path)?);
    let accounts: Arc<dyn poe_accounts::AccountStore> =
        Arc::new(poe_accounts::SqliteAccounts::new(store.clone(), config.cluster.id.clone()));
    // `poe-server account <create|role|disable|enable|passwd|list> [name] [value]` manages
    // accounts offline (the first admin), then exits. `create`/`passwd` take the password with
    // `--password-stdin` (one line on standard input; it is not visible in the process list).
    // The password as a command-line argument still works but is discouraged.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("account") {
        let cli = poe_accounts::parse_account_cli(&args[1..], &mut std::io::stdin().lock()).map_err(anyhow::Error::msg)?;
        return match poe_accounts::run_account_command(&*accounts, None, &cli.sub, cli.name.as_deref(), cli.value.as_deref()) {
            Ok(done) => {
                println!("{}", done.text);
                Ok(())
            }
            Err(e) => anyhow::bail!(e),
        };
    }
    // `poe-server character <create|list> ...` manages characters offline (test setup), then exits.
    if args.first().map(String::as_str) == Some("character") {
        return character_command(&config, &store, &*accounts, &args[1..]);
    }
    let tickets = Arc::new(MemoryTickets::new(std::time::Duration::from_secs(60)));
    let sessions = Arc::new(MemorySessions::default());
    let instances = Arc::new(MemoryInstances::default());
    let data = Arc::new(GameData::open(&config.game.game_dir, &config.game.schema)?);
    tracing::info!(game_dir = %config.game.game_dir.display(), "game data opened");
    // One limiter for the login server and the web API.
    let limiter = Arc::new(poe_accounts::LoginLimiter::from_config(&config.security));
    let login = Arc::new(LoginServer {
        config: config.clone(),
        protocol: protocol.clone(),
        store: store.clone(),
        accounts: accounts.clone(),
        limiter: limiter.clone(),
        data: data.clone(),
        tickets: tickets.clone(),
        sessions: sessions.clone(),
        instances: instances.clone(),
    });

    let instance = Arc::new(instance_server::InstanceServer {
        config: config.clone(),
        protocol: protocol.clone(),
        tickets: tickets.clone(),
        data: data.clone(),
        stats: Arc::new(poe_game::stats::StatRegistry::load(&data)?),
        store: store.clone(),
        accounts: accounts.clone(),
        seeds: Arc::new(instance_server::seeds::KnownSeeds::load(
            &config.dev.known_seeds,
        )),
        reports: Default::default(),
        terrain: Arc::new(instance_server::terrain_store::TerrainStore::new(&config.spawn)),
        sessions,
        instances: instances.clone(),
        live: Default::default(),
        actors: Default::default(),
    });

    let (web_lifecycle, mut shutdown_requests) = poe_web_api::Lifecycle::new();
    let web_state = poe_web_api::WebState {
        config: config.clone(),
        accounts: accounts.clone(),
        limiter,
        store: store.clone(),
        instances: instances.clone(),
        admin: instance.clone(),
        logs: logs.clone(),
        web_tokens: Default::default(),
        ws_tickets: Default::default(),
        lifecycle: web_lifecycle,
        started: std::time::Instant::now(),
    };
    let web_enabled = config.web.enabled;
    let web = async move {
        if web_enabled {
            poe_web_api::serve(web_state).await
        } else {
            tracing::info!("web api disabled ([web] enabled = false)");
            std::future::pending().await
        }
    };
    // Runs until a failure or until a stop or restart is requested: by an admin (web API,
    // after its countdown) or by SIGINT/SIGTERM (at once).
    let kind = {
        let run = async { tokio::try_join!(login.clone().run(), instance.clone().run(), web) };
        tokio::select! {
            r = run => {
                r?;
                return Ok(());
            }
            k = shutdown_requests.recv() => k.unwrap_or(poe_web_api::ShutdownKind::Stop),
            () = lifecycle::signal() => poe_web_api::ShutdownKind::Stop,
        }
    };
    tracing::info!(action = kind.as_str(), "shutting down: disconnecting players");
    drain(&instance).await;
    let action = lifecycle::decide(kind, config.server.restart_mode, lifecycle::HAS_EXEC);
    tracing::info!(?action, "shutdown complete");
    lifecycle::perform(action)
}

/// Disconnects every player and waits (a few seconds at most) until their sessions have ended:
/// a session saves its character when it ends. The database commits every write at once, so
/// nothing else is pending afterwards.
async fn drain(instance: &instance_server::InstanceServer) {
    for p in instance.live.online() {
        instance.live.kick(&p.character);
    }
    let end = tokio::time::Instant::now() + lifecycle::DRAIN_TIMEOUT;
    while !instance.live.online().is_empty() && tokio::time::Instant::now() < end {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let left = instance.live.online().len();
    if left > 0 {
        tracing::warn!(left, "sessions still open after the drain timeout");
    }
}

const CHARACTER_USAGE: &str = "usage: poe-server character create <account> <name> <start_state> [area] [league]\n       poe-server character list [account]\n  start_state: a CharacterStartStates id (e.g. DexFourDefault); area: a WorldAreas id (default: the start state's hub area, else [player] start_area); league default: the first [[leagues]] entry. A character created with an explicit town/hideout area counts as having played the intro.";

/// Offline character management for test setups: `character create` makes a character the way
/// the character creation screen does (same slot limit, name rules and start-state lookup).
fn character_command(
    config: &Config,
    store: &Arc<SqliteStore>,
    accounts: &dyn poe_accounts::AccountStore,
    args: &[String],
) -> anyhow::Result<()> {
    use poe_store::{NewCharacter, Store};
    match args.first().map(String::as_str) {
        Some("list") => {
            let all = store.all_characters()?;
            for c in all {
                let owner = accounts.find_by_id(c.account_id)?.map(|a| a.username).unwrap_or_default();
                if args.get(1).is_some_and(|a| *a != owner) {
                    continue;
                }
                println!("{owner}\t{}\t{}\t{}\t{}\t{}\tlevel {}", c.id, c.name, c.start_state, c.league, c.area, c.level);
            }
            Ok(())
        }
        Some("create") if args.len() >= 4 => {
            let (account, name, start_state) = (&args[1], &args[2], &args[3]);
            let Some(owner) = accounts.find(account)? else {
                anyhow::bail!("no account {account:?}");
            };
            let data = GameData::open(&config.game.game_dir, &config.game.schema)?;
            let Some(start) = poe_data::views::StartState::find(&data, start_state)? else {
                anyhow::bail!("unknown start state {start_state:?}");
            };
            let explicit_area = args.get(4).cloned();
            let area = explicit_area
                .clone()
                .or_else(|| start.hub_area.clone())
                .or_else(|| config.player.start_area.clone())
                .unwrap_or_default();
            let Some(world_area) = poe_data::views::WorldArea::find(&data, &area)? else {
                anyhow::bail!("unknown area {area:?}");
            };
            let league = match args.get(5) {
                Some(l) => l.clone(),
                None => config.leagues.first().map(|l| l.id.clone()).unwrap_or_default(),
            };
            if !config.leagues.iter().any(|l| l.id == league) {
                anyhow::bail!("unknown league {league:?}");
            }
            if store.characters(owner.id)?.len() >= config.accounts.max_characters as usize {
                anyhow::bail!("character slots full");
            }
            let created = store.create_character(&NewCharacter {
                account_id: owner.id,
                name: name.clone(),
                league,
                class_id: start.class_id,
                start_state: start.id.clone(),
                level: start.level,
                area,
            })?;
            let Some(c) = created else {
                anyhow::bail!("character name {name:?} is taken");
            };
            if explicit_area.is_some() && (world_area.is_town || world_area.is_hideout) {
                store.set_intro_done(c.id)?;
            }
            println!("created {} (id {}, {}, {}, area {}) for {account}", c.name, c.id, start.class_name, c.league, c.area);
            Ok(())
        }
        _ => anyhow::bail!(CHARACTER_USAGE),
    }
}
