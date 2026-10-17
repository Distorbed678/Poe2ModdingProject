use std::sync::Arc;

use login_server::LoginServer;
use poe_cluster::{MemoryInstances, MemorySessions, MemoryTickets};
use poe_config::Config;
use poe_data::GameData;
use poe_proto::Protocol;
use poe_store::SqliteStore;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Arc::new(Config::load_default()?);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.logging.filter));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    tracing::info!(version = %config.game.version, "starting poe-server");

    let protocol = Arc::new(Protocol::load(&config.game.protocol_dir)?);
    let store = Arc::new(SqliteStore::open(&config.database.path)?);
    let tickets = Arc::new(MemoryTickets::new(std::time::Duration::from_secs(60)));
    let sessions = Arc::new(MemorySessions::default());
    let instances = Arc::new(MemoryInstances::default());
    let data = Arc::new(GameData::open(&config.game.game_dir, &config.game.schema)?);
    tracing::info!(game_dir = %config.game.game_dir.display(), "game data opened");
    let login = Arc::new(LoginServer {
        config: config.clone(),
        protocol: protocol.clone(),
        store: store.clone(),
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
        seeds: Arc::new(instance_server::seeds::KnownSeeds::load(
            &config.instance.known_seeds,
        )),
        reports: Default::default(),
        sessions,
        instances,
        live: Default::default(),
    });

    tokio::try_join!(login.run(), instance.run())?;
    Ok(())
}
