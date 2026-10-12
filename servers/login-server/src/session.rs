//! One client connection to the login server.

use std::sync::Arc;

use poe_net::{Connection, hexdump::hexdump};
use tokio::net::TcpStream;
use tracing::info;

use crate::{LoginServer, handshake};

pub async fn run(server: Arc<LoginServer>, stream: TcpStream) -> anyhow::Result<()> {
    let mut conn = Connection::new(stream)?;
    info!("client connected");
    if !handshake::perform(&mut conn, &server.protocol).await? {
        info!("client disconnected during key exchange");
        return Ok(());
    }
    // Messages after the key exchange are not decoded yet: log them decrypted.
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
