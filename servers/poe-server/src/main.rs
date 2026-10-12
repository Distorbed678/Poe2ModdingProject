use std::sync::Arc;

use login_server::LoginServer;
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
    let data = Arc::new(GameData::open(&config.game.game_dir, &config.game.schema)?);
    tracing::info!(game_dir = %config.game.game_dir.display(), "game data opened");
    let login = Arc::new(LoginServer {
        config: config.clone(),
        protocol: protocol.clone(),
        store: store.clone(),
        data: data.clone(),
    });

    tokio::try_join!(login.run(), instance_server::run(config.clone()),)?;
    Ok(())
}
