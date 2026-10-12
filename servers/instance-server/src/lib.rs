//! Instance server: hosts areas. The login server hands clients over with a ticket;
//! the instance server validates it, sends the area and runs the simulation.

use std::sync::Arc;

use poe_config::Config;
use poe_net::{Connection, hexdump::hexdump};
use tokio::net::TcpListener;
use tracing::{Instrument, info, warn};

pub async fn run(config: Arc<Config>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(config.instance.bind).await?;
    info!(addr = %config.instance.bind, "instance server listening");
    loop {
        let (stream, peer) = listener.accept().await?;
        let span = tracing::info_span!("instance", %peer);
        tokio::spawn(
            async move {
                if let Err(e) = handle(stream).await {
                    warn!("connection error: {e:#}");
                }
            }
            .instrument(span),
        );
    }
}

async fn handle(stream: tokio::net::TcpStream) -> anyhow::Result<()> {
    let mut conn = Connection::new(stream)?;
    info!("client connected");
    // Protocol not implemented yet: log everything the client sends.
    loop {
        if conn.read_more().await? == 0 {
            info!("client disconnected");
            return Ok(());
        }
        info!(
            len = conn.buffer().len(),
            "received\n{}",
            hexdump(conn.buffer())
        );
        let n = conn.buffer().len();
        conn.consume(n);
    }
}
