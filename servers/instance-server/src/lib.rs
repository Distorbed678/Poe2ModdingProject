//! Instance server: hosts areas. The login server hands clients over with a ticket;
//! the instance server validates it, sends the area and runs the simulation.

use std::sync::Arc;

use poe_cluster::TicketRegistry;
use poe_config::Config;
use poe_net::{Connection, KeyMaterial, StreamCipher, hexdump::hexdump};
use tokio::net::TcpListener;
use tracing::{Instrument, info, warn};

pub async fn run(config: Arc<Config>, tickets: Arc<dyn TicketRegistry>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(config.instance.bind).await?;
    info!(addr = %config.instance.bind, "instance server listening");
    loop {
        let (stream, peer) = listener.accept().await?;
        let span = tracing::info_span!("instance", %peer);
        let tickets = tickets.clone();
        tokio::spawn(
            async move {
                if let Err(e) = handle(stream, tickets).await {
                    warn!("connection error: {e:#}");
                }
            }
            .instrument(span),
        );
    }
}

async fn handle(
    stream: tokio::net::TcpStream,
    tickets: Arc<dyn TicketRegistry>,
) -> anyhow::Result<()> {
    let mut conn = Connection::new(stream)?;
    info!("client connected");
    // Protocol not implemented yet: log what the client sends, raw and decrypted with the
    // keys of each pending hand-off ticket (to learn how the instance stream starts).
    let mut first = true;
    loop {
        if conn.read_more().await? == 0 {
            info!("client disconnected");
            return Ok(());
        }
        let data = conn.buffer().to_vec();
        info!(len = data.len(), "received (raw)\n{}", hexdump(&data));
        if first {
            for ticket in tickets.pending() {
                let mut copy = data.clone();
                KeyMaterial(ticket.keys).client_to_server().apply(&mut copy);
                info!(
                    ticket = format_args!("{:#010x}", ticket.id),
                    "decrypted with ticket keys\n{}",
                    hexdump(&copy)
                );
            }
            first = false;
        }
        conn.consume(data.len());
    }
}
