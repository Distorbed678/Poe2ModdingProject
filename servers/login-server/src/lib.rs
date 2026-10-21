//! Login (gateway) server: the first host the client connects to, taken from
//! `gateway_list.txt`. Handles the key exchange, authentication, leagues and the character
//! list, and hands the client off to an instance server.

mod auth;
mod codes;
mod handshake;
mod session;

use std::sync::Arc;

use poe_accounts::{AccountStore, LoginLimiter};
use poe_cluster::TicketRegistry;
use poe_config::Config;
use poe_data::GameData;
use poe_proto::Protocol;
use poe_store::Store;
use tokio::net::TcpListener;
use tracing::{Instrument, info, warn};

/// State shared by all login connections.
pub struct LoginServer {
    pub config: Arc<Config>,
    pub protocol: Arc<Protocol>,
    pub store: Arc<dyn Store>,
    /// Usernames, passwords and login tokens.
    pub accounts: Arc<dyn AccountStore>,
    /// Password attempt limits (shared with the web API).
    pub limiter: Arc<LoginLimiter>,
    pub data: Arc<GameData>,
    pub tickets: Arc<dyn TicketRegistry>,
    /// Lets an instance server close an account's login connection when it leaves the game.
    pub sessions: Arc<dyn poe_cluster::LoginSessions>,
    /// Instances of areas (characters enter an existing one or a new one).
    pub instances: Arc<dyn poe_cluster::InstanceDirectory>,
}

impl LoginServer {
    pub async fn run(self: Arc<Self>) -> anyhow::Result<()> {
        for problem in codes::verify(&self.data) {
            warn!("result code table: {problem} (client update? see poe_proto::backend_error)");
        }
        let listener = TcpListener::bind(self.config.network.login.bind).await?;
        info!(addr = %self.config.network.login.bind, "login server listening");
        loop {
            let (stream, peer) = listener.accept().await?;
            let span = tracing::info_span!("login", %peer);
            let server = self.clone();
            tokio::spawn(
                async move {
                    if let Err(e) = session::run(server, stream, peer.ip()).await {
                        warn!("connection error: {e:#}");
                    }
                }
                .instrument(span),
            );
        }
    }
}
