//! One client connection to the login server.

use std::sync::Arc;

use poe_net::{Connection, DecodeError, Reader, hexdump::hexdump};
use poe_proto::{
    Direction, Message, Protocol,
    login::{Authenticate, AuthenticationResult, CharacterList, KeepAlive, RequestCharacterList},
};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::{LoginServer, handshake};

/// Messages the login server understands after the key exchange.
enum ClientMessage {
    KeepAlive,
    Authenticate(Authenticate),
    RequestCharacterList(RequestCharacterList),
}

/// Reads one message of a known type (opcode checked against the table).
pub async fn read_message<M: Message>(
    conn: &mut Connection,
    protocol: &Protocol,
) -> anyhow::Result<Option<M>> {
    let expected = protocol.login.id(M::DIRECTION, M::NAME)?;
    Ok(conn
        .decode(|r| {
            let at = r.position();
            if r.u16()? != expected {
                return Err(DecodeError::Invalid {
                    offset: at,
                    what: "unexpected opcode",
                });
            }
            M::decode(r)
        })
        .await?)
}

/// Decodes the next client message. Unknown opcodes are fatal: without a length prefix
/// the rest of the stream cannot be framed.
fn decode_client(protocol: &Protocol, r: &mut Reader<'_>) -> Result<ClientMessage, DecodeError> {
    let at = r.position();
    let opcode = r.u16()?;
    match protocol.login.name(Direction::Client, opcode) {
        Some(KeepAlive::NAME) => Ok(ClientMessage::KeepAlive),
        Some(Authenticate::NAME) => Authenticate::decode(r).map(ClientMessage::Authenticate),
        Some(RequestCharacterList::NAME) => {
            RequestCharacterList::decode(r).map(ClientMessage::RequestCharacterList)
        }
        _ => Err(DecodeError::Invalid {
            offset: at,
            what: "unknown login opcode",
        }),
    }
}

pub async fn run(server: Arc<LoginServer>, stream: TcpStream) -> anyhow::Result<()> {
    let mut conn = Connection::new(stream)?;
    info!("client connected");
    if !handshake::perform(&mut conn, &server.protocol).await? {
        info!("client disconnected during key exchange");
        return Ok(());
    }
    loop {
        let message = match conn.decode(|r| decode_client(&server.protocol, r)).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                info!("client disconnected");
                return Ok(());
            }
            Err(e) => {
                warn!("{e}; undecoded data:\n{}", hexdump(conn.buffer()));
                return Ok(());
            }
        };
        match message {
            ClientMessage::KeepAlive => debug!("keep-alive"),
            ClientMessage::Authenticate(auth) => authenticate(&server, &mut conn, auth).await?,
            ClientMessage::RequestCharacterList(req) => {
                info!(account_id = req.account_id, "character list requested");
                // No characters yet (character persistence comes with character creation).
                let list = CharacterList::default();
                conn.send(&server.protocol.login.encode(&list)?).await?;
            }
        }
    }
}

async fn authenticate(
    server: &LoginServer,
    conn: &mut Connection,
    auth: Authenticate,
) -> anyhow::Result<()> {
    let login_key = hex::encode(auth.machine_hash);
    let account = match server.store.account_by_login_key(&login_key)? {
        Some(a) => a,
        None => {
            let a = server
                .store
                .create_account(&login_key, &server.config.accounts.name_prefix)?;
            info!(account = %a.name, "created account");
            a
        }
    };
    info!(
        account = %account.name,
        id = account.id,
        ticket_len = auth.ticket.len(),
        "authenticated"
    );
    let reply = AuthenticationResult {
        account_id: account.id,
        result: 0,
        account_name: account.name.clone(),
        session_token: [0; 32],
        account_flags: [0; 2],
        message: String::new(),
    };
    conn.send(&server.protocol.login.encode(&reply)?).await?;
    Ok(())
}
