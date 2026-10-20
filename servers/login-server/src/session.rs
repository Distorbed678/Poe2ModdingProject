//! One client connection to the login server.

use std::sync::Arc;

use poe_cluster::TicketCharacter;
use poe_data::views::{StartState, WorldArea, league_flag_index};
use poe_net::{Connection, DecodeError, Reader, hexdump::hexdump};
use poe_proto::{
    Direction, Message, Protocol,
    login::{
        Authenticate, AuthenticationResult, CharacterCreated, CharacterDetails, CharacterEntry,
        CharacterList, CreateCharacter, DeleteCharacter, DeleteCharacterResult, InstanceDetails,
        KeepAlive, KeepAliveReply, LeagueEntry, LeagueList, PlayCharacter, RequestCharacterDetails,
        RequestCharacterList, ResumeSession,
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
    RequestCharacterDetails(RequestCharacterDetails),
    PlayCharacter(PlayCharacter),
    DeleteCharacter(DeleteCharacter),
    ResumeSession(ResumeSession),
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
        Some(RequestCharacterList::NAME | "RequestCharacterListAlt") => {
            RequestCharacterList::decode(r).map(ClientMessage::RequestCharacterList)
        }
        Some(CreateCharacter::NAME) => {
            CreateCharacter::decode(r).map(ClientMessage::CreateCharacter)
        }
        Some(RequestCharacterDetails::NAME) => {
            RequestCharacterDetails::decode(r).map(ClientMessage::RequestCharacterDetails)
        }
        Some(PlayCharacter::NAME) => PlayCharacter::decode(r).map(ClientMessage::PlayCharacter),
        Some(ResumeSession::NAME) => ResumeSession::decode(r).map(ClientMessage::ResumeSession),
        Some(DeleteCharacter::NAME) => {
            DeleteCharacter::decode(r).map(ClientMessage::DeleteCharacter)
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
    // Changes when the account leaves the game (instance `0x1b`, see `LoginSessions`).
    let mut logout: Option<tokio::sync::watch::Receiver<u64>> = None;
    // In-game instance changes for the account (see `LoginSessions::transfer`).
    let mut transfers: Option<tokio::sync::mpsc::UnboundedReceiver<poe_cluster::Ticket>> = None;
    // A transfer waiting for the client to leave its instance.
    let mut pending: Option<poe_cluster::Ticket> = None;
    loop {
        let decoded = tokio::select! {
            d = conn.decode(|r| decode_client(&server.protocol, r)) => d,
            () = logged_out(&mut logout) => {
                info!("account left the game: closing the login connection");
                return Ok(());
            }
            Some(ticket) = next_transfer(&mut transfers) => {
                // The client ignores InstanceDetails while it is in an instance: the instance
                // server ends that session, the client comes back to character selection on
                // this connection, and its character list request gets the details.
                info!(area = %ticket.area, instance = ?ticket.instance, ticket = format_args!("{:#010x}", ticket.id), "instance transfer pending");
                pending = Some(ticket);
                continue;
            }
        };
        let message = match decoded {
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
            ClientMessage::KeepAlive => {
                debug!("keep-alive");
                conn.send(&server.protocol.login.encode(&KeepAliveReply)?)
                    .await?;
            }
            ClientMessage::Authenticate(auth) => {
                let a = authenticate(&server, &mut conn, auth).await?;
                logout = Some(server.sessions.subscribe(a.id));
                transfers = Some(server.sessions.subscribe_transfers(a.id));
                account = Some(a);
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
                if let Some(ticket) = pending.take() {
                    info!(area = %ticket.area, instance = ?ticket.instance, "instance transfer: sending the client on");
                    send_instance_details(&server, &mut conn, &ticket).await?;
                }
            }
            ClientMessage::ResumeSession(req) => {
                let ticket = poe_cluster::ResumeTicket {
                    account_id: req.account_id,
                    token: req.token,
                };
                let resumed = server
                    .tickets
                    .redeem_resume(ticket)
                    .then(|| server.store.account_by_id(req.account_id))
                    .transpose()?
                    .flatten();
                let Some(resumed) = resumed else {
                    warn!(account_id = req.account_id, "invalid resume ticket");
                    return Ok(());
                };
                info!(account = %resumed.name, id = resumed.id, "session resumed");
                send_authentication_result(&server, &mut conn, &resumed).await?;
                logout = Some(server.sessions.subscribe(resumed.id));
                transfers = Some(server.sessions.subscribe_transfers(resumed.id));
                account = Some(resumed);
            }
            ClientMessage::DeleteCharacter(req) => {
                let Some(account) = &account else {
                    warn!("delete character before authentication");
                    continue;
                };
                let deleted = server.store.delete_character(account.id, &req.name)?;
                info!(name = %req.name, deleted, unk = ?req.unk, "delete character");
                // Result 0 closes the delete screen; the code is otherwise shown as an error.
                // TODO: the error code for "no such character" is not known yet.
                let result = DeleteCharacterResult {
                    result: if deleted { 0 } else { 1 },
                    unk: 0,
                };
                conn.send(&server.protocol.login.encode(&result)?).await?;
                // The client keeps its list until it gets a new one (it does not ask).
                send_character_list(&server, &mut conn, account.id).await?;
            }
            ClientMessage::CreateCharacter(create) => {
                let Some(account) = &account else {
                    warn!("create character before authentication");
                    continue;
                };
                create_character(&server, &mut conn, account, create).await?;
            }
            ClientMessage::RequestCharacterDetails(req) => {
                if account.as_ref().map(|a| a.id) != Some(req.account_id) {
                    warn!(account_id = req.account_id, "details for a foreign account");
                    continue;
                }
                info!(index = req.index, "character details requested");
                let details = CharacterDetails {
                    account_id: req.account_id,
                    index: req.index,
                    ..Default::default()
                };
                conn.send(&server.protocol.login.encode(&details)?).await?;
            }
            ClientMessage::PlayCharacter(play) => {
                let Some(account) = &account else {
                    warn!("play before authentication");
                    continue;
                };
                play_character(&server, &mut conn, account, play).await?;
            }
        }
    }
}

/// The next transfer for the account; never without an account.
async fn next_transfer(rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<poe_cluster::Ticket>>) -> Option<poe_cluster::Ticket> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Resolves when the account logs out; never without an account.
async fn logged_out(logout: &mut Option<tokio::sync::watch::Receiver<u64>>) {
    if let Some(rx) = logout
        && rx.changed().await.is_ok()
    {
        return;
    }
    std::future::pending().await
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
    send_authentication_result(server, conn, &account).await?;
    Ok(account)
}

async fn send_authentication_result(
    server: &LoginServer,
    conn: &mut Connection,
    account: &Account,
) -> anyhow::Result<()> {
    let reply = AuthenticationResult {
        account_id: account.id,
        result: 0,
        notice: String::new(),
        session_token: [0; 32],
        account_flags: server.config.accounts.flags,
        account_name: account.name.clone(),
    };
    conn.send(&server.protocol.login.encode(&reply)?).await?;
    Ok(())
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
                // A variant names its base league here (`0x140e7ca80` groups variants under
                // the base league's banner at character creation).
                text_80: l.variant_of.clone().unwrap_or_else(|| l.id.clone()),
                text_a0: l.id.clone(),
                time_d0: 0,
                time_d8: 0,
                time_c8: 0,
                // +0xe0 bit 1: hardcore; bit 7: not offered in the "Join" list (permanent
                // leagues). +0xe1 bit 1: a variant of another league. Character creation makes
                // a banner for every league with +0xe0 bit 7 that is not a variant.
                raw_e0: [
                    if l.hardcore { 0x02 } else { 0 } | if l.permanent { 0x80 } else { 0 },
                    if l.variant_of.is_some() { 0x02 } else { 0 },
                    0,
                ],
                flag_c0: 0,
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
    let entries = characters
        .iter()
        .map(|c| character_entry(server, c))
        .collect::<anyhow::Result<_>>()?;
    let list = CharacterList {
        account_id,
        characters: entries,
        // +0x29: character slot limit (select screen compares it with the count).
        unk_29: server.config.accounts.max_characters,
        ..Default::default()
    };
    conn.send(&server.protocol.login.encode(&list)?).await?;
    Ok(())
}

/// Maps a stored character onto the list entry. Field placement is provisional
/// (docs/protocol/login.md): +0x4e is the class (used for the 3D preview),
/// +0x4c is assumed to be the level, +0x4a is the current area and +0x48 a fallback area
/// (both WorldAreas.HASH16; select screen code at `0x140f02cef`).
fn character_entry(server: &LoginServer, c: &Character) -> anyhow::Result<CharacterEntry> {
    let area_hash = WorldArea::find(&server.data, &c.area)?
        .map(|a| a.hash16)
        .unwrap_or_default();
    // +0x4d: LeagueFlag row + 1 (banner; select screen `0x1406b9dc7` -> `0x140c7e460`).
    let flag = server
        .config
        .leagues
        .iter()
        .find(|l| l.id == c.league)
        .and_then(|l| l.flag.as_deref());
    let league_flag = match flag {
        Some(id) => league_flag_index(&server.data, id)?.map_or(0, |i| (i + 1) as u8),
        None => 0,
    };
    Ok(CharacterEntry {
        name: c.name.clone(),
        league: c.league.clone(),
        unk_4d: league_flag,
        unk_4c: c.level.min(255) as u8,
        experience: c.experience.min(u32::MAX as u64) as u32,
        unk_4e: c.class_id as u8,
        unk_4a: area_hash,
        unk_48: area_hash,
        ..Default::default()
    })
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
    } else if server.store.characters(account.id)?.len()
        >= server.config.accounts.max_characters as usize
    {
        Some("character slots full")
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
                level: start.level,
                area: start
                    .hub_area
                    .clone()
                    .or_else(|| server.config.player.start_area.clone())
                    .unwrap_or_default(),
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
        result: if created.is_some() { 0 } else { 1 },
        notify: false,
        account_id: account.id,
    };
    conn.send(&server.protocol.login.encode(&result)?).await?;
    if created.is_some() {
        send_character_list(server, conn, account.id).await?;
    }
    Ok(())
}

/// Hands the client off to the instance server for the selected character.
async fn play_character(
    server: &LoginServer,
    conn: &mut Connection,
    account: &Account,
    play: PlayCharacter,
) -> anyhow::Result<()> {
    // Local split-screen players are not supported: use the first selection.
    let Some(&(index, account_id)) = play.selections.first() else {
        warn!("play without a selection");
        return Ok(());
    };
    if account_id != account.id {
        warn!(account_id, "play for a foreign account");
        return Ok(());
    }
    let characters = server.store.characters(account.id)?;
    let Some(character) = characters.get(index as usize) else {
        warn!(index, "play: no such character");
        return Ok(());
    };
    // RE experiment: `POE_AREA=<WorldAreas.Id>` loads another area than the character's.
    let area_id = std::env::var("POE_AREA").unwrap_or_else(|_| character.area.clone());
    let Some(area) = WorldArea::find(&server.data, &area_id)? else {
        warn!(area = %character.area, "play: character area not in game data");
        return Ok(());
    };
    // Towns and hideouts are shared (one kept instance each); other areas get the newest
    // open instance, else a new one.
    let instance = match server.instances.find(&area.id) {
        Some(id) => id,
        None => server.instances.create(
            poe_cluster::InstanceSpec { area: area.id.clone(), seed: area.fixed_seed(), ..Default::default() },
            area.is_town || area.is_hideout,
        ),
    };
    let ticket = server.tickets.issue_for(
        account.id,
        TicketCharacter {
            id: character.id,
            name: character.name.clone(),
            league: character.league.clone(),
            class_id: character.class_id,
            level: character.level,
            intro_done: character.intro_done,
            // The saved position only applies to the area it was saved in.
            location: character
                .location
                .filter(|_| area.id == character.area)
                .map(|l| (l.seed, l.x, l.y)),
        },
        &area.id,
        Some(instance),
    );
    info!(
        character = %character.name,
        area = %area.id,
        instance,
        ticket = format_args!("{:#010x}", ticket.id),
        "handing off to instance server"
    );
    send_instance_details(server, conn, &ticket).await
}

/// `InstanceDetails` for a ticket: where to connect, the area and the stream keys.
async fn send_instance_details(server: &LoginServer, conn: &mut Connection, ticket: &poe_cluster::Ticket) -> anyhow::Result<()> {
    let Some(area) = WorldArea::find(&server.data, &ticket.area)? else {
        anyhow::bail!("ticket area {} not in game data", ticket.area);
    };
    let details = InstanceDetails {
        host: server.config.network.public_host.clone(),
        port: server.config.network.instance.public_port,
        unk_30: ticket.id,
        area_hash: area.hash16,
        unk_34: 0,
        keys: ticket.keys,
        unk_7c: [0; 8],
        unk_84: 0,
    };
    let mut out = server.protocol.login.encode(&details)?;
    // Official servers follow InstanceDetails with 0x0017 0x0014, which makes the client shut
    // its login connection down (handler `0x1420e7140` -> `0x1420e71d0`: `shutdown()`). We keep
    // the connection: in-game transfers arrive on it (`[login] close_after_handoff`).
    if server.config.network.login.close_after_handoff {
        out.extend(
            server
                .protocol
                .login
                .encode(&poe_proto::login::Unknown0017 { value: 0x0014 })?,
        );
    }
    conn.send(&out).await?;
    Ok(())
}
