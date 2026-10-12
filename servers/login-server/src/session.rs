//! One client connection to the login server.

use std::sync::Arc;

use poe_data::views::StartState;
use poe_net::{Connection, DecodeError, Reader, hexdump::hexdump};
use poe_proto::{
    Direction, Message, Protocol,
    login::{
        Authenticate, AuthenticationResult, CharacterCreated, CharacterEntry, CharacterList,
        CreateCharacter, KeepAlive, LeagueEntry, LeagueList, RequestCharacterList,
    },
};
use poe_store::{Account, Character, NewCharacter};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::{LoginServer, handshake};

/// Messages the login server understands after the key exchange.
enum ClientMessage {
    KeepAlive,
    Authenticate(Authenticate),
    RequestCharacterList(RequestCharacterList),
    CreateCharacter(CreateCharacter),
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
        Some(CreateCharacter::NAME) => {
            CreateCharacter::decode(r).map(ClientMessage::CreateCharacter)
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
    // Account authenticated on this connection.
    let mut account: Option<Account> = None;
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
            ClientMessage::Authenticate(auth) => {
                account = Some(authenticate(&server, &mut conn, auth).await?);
            }
            ClientMessage::RequestCharacterList(req) => {
                if account.as_ref().map(|a| a.id) != Some(req.account_id) {
                    warn!(
                        account_id = req.account_id,
                        "character list for a foreign account"
                    );
                    continue;
                }
                info!(account_id = req.account_id, "character list requested");
                send_character_list(&server, &mut conn, req.account_id).await?;
                conn.send(&server.protocol.login.encode(&league_list(&server))?)
                    .await?;
            }
            ClientMessage::CreateCharacter(create) => {
                let Some(account) = &account else {
                    warn!("create character before authentication");
                    continue;
                };
                create_character(&server, &mut conn, account, create).await?;
            }
        }
    }
}

async fn authenticate(
    server: &LoginServer,
    conn: &mut Connection,
    auth: Authenticate,
) -> anyhow::Result<Account> {
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
    Ok(account)
}

/// League list from `[[leagues]]` in the server config.
fn league_list(server: &LoginServer) -> LeagueList {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    LeagueList {
        server_time: now,
        leagues: server
            .config
            .leagues
            .iter()
            .map(|l| LeagueEntry {
                id: l.id.clone(),
                name: l.id.clone(),
                text_60: l.id.clone(),
                text_80: l.id.clone(),
                text_a0: l.id.clone(),
                time_d0: 0,
                time_d8: 0,
                time_c8: 0,
                raw_e0: [0; 3],
                flag_c0: l.hardcore as u8,
                flag_c1: 0,
                flag_c2: 0,
            })
            .collect(),
        flags: 0,
    }
}

async fn send_character_list(
    server: &LoginServer,
    conn: &mut Connection,
    account_id: u32,
) -> anyhow::Result<()> {
    let characters = server.store.characters(account_id)?;
    let list = CharacterList {
        account_id,
        characters: characters.iter().map(character_entry).collect(),
        ..Default::default()
    };
    conn.send(&server.protocol.login.encode(&list)?).await?;
    Ok(())
}

/// Maps a stored character onto the list entry. Field placement is provisional
/// (Documentation/protocol/login.md): +0x4e is the class (used for the 3D preview),
/// +0x4c is assumed to be the level.
fn character_entry(c: &Character) -> CharacterEntry {
    CharacterEntry {
        name: c.name.clone(),
        league: c.league.clone(),
        unk_4c: c.level.min(255) as u8,
        unk_4e: c.class_id as u8,
        ..Default::default()
    }
}

/// Why a creation request was refused (logged; the client error code is still provisional).
fn validate_name(name: &str) -> Result<(), &'static str> {
    let len = name.chars().count();
    if !(3..=23).contains(&len) {
        return Err("name must be 3-23 characters");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("name may only contain letters, digits and underscores");
    }
    Ok(())
}

async fn create_character(
    server: &LoginServer,
    conn: &mut Connection,
    account: &Account,
    req: CreateCharacter,
) -> anyhow::Result<()> {
    info!(name = %req.name, league = %req.league, start_state = %req.start_state, unk_50 = req.unk_50, "create character");
    let refusal = if let Err(reason) = validate_name(&req.name) {
        Some(reason)
    } else if !server.config.leagues.iter().any(|l| l.id == req.league) {
        Some("unknown league")
    } else {
        None
    };
    let start = match refusal {
        Some(_) => None,
        None => StartState::find(&server.data, &req.start_state)?,
    };
    let created = match (refusal, start) {
        (Some(reason), _) => {
            warn!(reason, "character creation refused");
            None
        }
        (None, None) => {
            warn!(start_state = %req.start_state, "unknown start state");
            None
        }
        (None, Some(start)) => {
            let new = NewCharacter {
                account_id: account.id,
                name: req.name.clone(),
                league: req.league.clone(),
                class_id: start.class_id,
                start_state: start.id.clone(),
            };
            let created = server.store.create_character(&new)?;
            match &created {
                Some(c) => info!(id = c.id, class = %start.class_name, "character created"),
                None => warn!(name = %req.name, "character name taken"),
            }
            created
        }
    };
    let result = CharacterCreated {
        account_id: account.id,
        result: if created.is_some() { 0 } else { 1 },
        unk_16: 0,
    };
    conn.send(&server.protocol.login.encode(&result)?).await?;
    if created.is_some() {
        send_character_list(server, conn, account.id).await?;
    }
    Ok(())
}
