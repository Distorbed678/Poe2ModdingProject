use std::sync::Arc;

use poe_config::Config;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Arc::new(Config::load_default()?);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.logging.filter));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    tracing::info!(version = %config.game.version, "starting poe-server");

    tokio::try_join!(
        login_server::run(config.clone()),
        instance_server::run(config.clone()),
    )?;
    Ok(())
}
