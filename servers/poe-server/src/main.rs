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
    for w in config.network.warnings() {
        tracing::warn!("{w}");
    }

    let protocol = Arc::new(Protocol::load(&config.game.protocol_dir)?);
    let store = Arc::new(SqliteStore::open(&config.database.path)?);
    let accounts: Arc<dyn poe_accounts::AccountStore> =
        Arc::new(poe_accounts::SqliteAccounts::new(store.clone(), config.cluster.id.clone()));
    // `poe-server account <create|role|disable|enable|passwd|list> [name] [value]` manages
    // accounts offline (the first admin), then exits.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("account") {
        let sub = args.get(1).map(String::as_str).unwrap_or("list");
        let value = (args.len() > 3).then(|| args[3..].join(" "));
        return match poe_accounts::run_account_command(&*accounts, None, sub, args.get(2).map(String::as_str), value.as_deref()) {
            Ok(done) => {
                println!("{}", done.text);
                Ok(())
            }
            Err(e) => anyhow::bail!(e),
        };
    }
    let tickets = Arc::new(MemoryTickets::new(std::time::Duration::from_secs(60)));
    let sessions = Arc::new(MemorySessions::default());
    let instances = Arc::new(MemoryInstances::default());
    let data = Arc::new(GameData::open(&config.game.game_dir, &config.game.schema)?);
    tracing::info!(game_dir = %config.game.game_dir.display(), "game data opened");
    let login = Arc::new(LoginServer {
        config: config.clone(),
        protocol: protocol.clone(),
        store: store.clone(),
        accounts: accounts.clone(),
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

    let web_state = poe_web_api::WebState {
        config: config.clone(),
        accounts: accounts.clone(),
        store: store.clone(),
        instances: instances.clone(),
        admin: instance.clone(),
        logs: logs.clone(),
        started: std::time::Instant::now(),
    };
    let web = async move {
        if config.web.enabled {
            poe_web_api::serve(web_state).await
        } else {
            tracing::info!("web api disabled ([web] enabled = false)");
            std::future::pending().await
        }
    };
    tokio::try_join!(login.run(), instance.run(), web)?;
    Ok(())
}
