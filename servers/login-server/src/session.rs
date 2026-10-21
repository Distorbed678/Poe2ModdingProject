//! One client connection to the login server.

use std::sync::Arc;

use poe_cluster::TicketCharacter;
use poe_data::views::{StartState, WorldArea, league_flag_index};
use poe_net::{Connection, DecodeError, Reader, hexdump::hexdump};
use poe_proto::{
    Direction, Message, Protocol,
    backend_error::{self as be, BackendError},
    login::{
        Authenticate, AuthenticationResult, Credentials, CharacterCreated, CharacterDetails, CharacterEntry,
        CharacterList, CreateCharacter, DeleteCharacter, DeleteCharacterResult, InstanceDetails,
        KeepAlive, KeepAliveReply, LeagueEntry, LeagueList, PlayCharacter, RequestCharacterDetails,
        RequestCharacterList, ResumeSession,
    },
};
use poe_store::{Account, Character, NewCharacter};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::{LoginServer, auth, handshake};

/// Messages the login server understands after the key exchange.
pub(crate) enum ClientMessage {
    KeepAlive,
    Authenticate(Authenticate),
    Credentials(Credentials),
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
pub(crate) fn decode_client(protocol: &Protocol, r: &mut Reader<'_>) -> Result<ClientMessage, DecodeError> {
    let at = r.position();
    let opcode = r.u16()?;
    match protocol.login.name(Direction::Client, opcode) {
        Some(KeepAlive::NAME) => Ok(ClientMessage::KeepAlive),
        Some(Authenticate::NAME) => Authenticate::decode(r).map(ClientMessage::Authenticate),
        Some(Credentials::NAME) => Credentials::decode(r).map(ClientMessage::Credentials),
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

pub async fn run(server: Arc<LoginServer>, stream: TcpStream, peer: std::net::IpAddr) -> anyhow::Result<()> {
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
                if let Some(a) = authenticate(&server, &mut conn, auth).await? {
                    logout = Some(server.sessions.subscribe(a.id));
                    transfers = Some(server.sessions.subscribe_transfers(a.id));
                    account = Some(a);
                }
            }
            ClientMessage::Credentials(creds) => {
                if let Some(a) = credentials(&server, &mut conn, peer, creds).await? {
                    logout = Some(server.sessions.subscribe(a.id));
                    transfers = Some(server.sessions.subscribe_transfers(a.id));
                    account = Some(a);
                }
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
                let outcome = delete_character(&*server.store, account.id, &req.name);
                info!(name = %req.name, result = outcome.code(), slot = req.unk[0], "delete character");
                // Result 0 closes the delete screen; any other code is shown as the table text.
                let result = delete_result(outcome, account.id);
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

/// Dev-only Steam login (`0x0005`): the account is keyed by the machine hash, no password.
/// Off unless `[accounts] dev_machine_hash_login`.
async fn authenticate(
    server: &LoginServer,
    conn: &mut Connection,
    auth: Authenticate,
) -> anyhow::Result<Option<Account>> {
    if !server.config.accounts.dev_machine_hash_login {
        warn!("Steam login (0x0005) refused: [accounts] dev_machine_hash_login is off");
        send_refusal(server, conn, auth::code::NOT_ENABLED, "This server needs a username and password login.").await?;
        return Ok(None);
    }
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
    if server.accounts.find_by_id(account.id)?.is_some_and(|r| r.disabled) {
        warn!(account = %account.name, "disabled account refused");
        send_refusal(server, conn, auth::code::DISABLED, "This account has been disabled.").await?;
        return Ok(None);
    }
    server.accounts.record_login(account.id)?;
    info!(
        account = %account.name,
        id = account.id,
        ticket_len = auth.ticket.len(),
        "authenticated (dev machine hash)"
    );
    // A second login of the account replaces the first one (login connection and game).
    server.sessions.supersede(account.id);
    send_authentication_result(server, conn, &account).await?;
    Ok(Some(account))
}

/// Standalone login (`0x0003`): username and password hash, or a saved login token.
async fn credentials(
    server: &LoginServer,
    conn: &mut Connection,
    peer: std::net::IpAddr,
    creds: Credentials,
) -> anyhow::Result<Option<Account>> {
    if creds.mode != 2 {
        warn!(mode = creds.mode, "0x0003 with an unexpected mode");
    }
    let accounts = server.accounts.clone();
    let config = server.config.clone();
    let limiter = server.limiter.clone();
    let name = creds.account_name.clone();
    let token_login = creds.token_login != 0;
    // argon2id is deliberately slow: keep it off the async workers.
    let verdict = tokio::task::spawn_blocking(move || auth::credentials_login(&*accounts, &config.accounts, &limiter, peer, &creds)).await?;
    let reply = auth::reply(&server.config.accounts, &verdict);
    match &verdict {
        auth::Verdict::Refused { result, .. } => {
            warn!(account = %name, token_login, result, "login refused");
            conn.send(&server.protocol.login.encode(&reply)?).await?;
            Ok(None)
        }
        auth::Verdict::Accepted { account, token } => {
            let Some(stored) = server.store.account_by_id(account.id)? else {
                anyhow::bail!("account {} has no row in the accounts table", account.id);
            };
            info!(account = %stored.name, id = stored.id, token_login, remembered = token.is_some(), "authenticated");
            // A second login of the account replaces the first one (login connection and game).
            server.sessions.supersede(stored.id);
            conn.send(&server.protocol.login.encode(&reply)?).await?;
            Ok(Some(stored))
        }
    }
}

async fn send_refusal(
    server: &LoginServer,
    conn: &mut Connection,
    result: u16,
    notice: &'static str,
) -> anyhow::Result<()> {
    let reply = auth::reply(&server.config.accounts, &auth::Verdict::Refused { result, notice: notice.to_owned() });
    conn.send(&server.protocol.login.encode(&reply)?).await?;
    Ok(())
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

/// Name rules of character creation: 3 to 23 characters, letters, digits and underscores.
/// Returns the `BackendErrors` code the client shows for the first rule that fails.
fn validate_name(name: &str) -> Result<(), BackendError> {
    let len = name.chars().count();
    if len < 3 {
        return Err(be::CHARACTER_NAME_TOO_SHORT);
    }
    if len > 23 {
        return Err(be::CHARACTER_NAME_TOO_LONG);
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(be::CHARACTER_NAME_INVALID);
    }
    Ok(())
}

/// The checks of character creation that need no game data, in the order the client's own
/// flow suggests: name, league, free slot.
fn create_refusal(
    name: &str,
    league: &str,
    leagues: &[poe_config::LeagueConfig],
    existing: usize,
    max_characters: usize,
) -> Option<BackendError> {
    if let Err(e) = validate_name(name) {
        Some(e)
    } else if !leagues.iter().any(|l| l.id == league) {
        Some(be::LEAGUE_DOES_NOT_EXIST)
    } else if existing >= max_characters {
        Some(be::TOO_MANY_CHARACTERS)
    } else {
        None
    }
}

/// The `0x0013` reply: `result` 0 or a `BackendErrors` code. `notify` stays clear (set, the
/// client calls the official website's conversion tracking).
fn create_result(refusal: Option<BackendError>, account_id: u32) -> CharacterCreated {
    CharacterCreated { result: refusal.map_or(0, |e| e.code), notify: false, account_id }
}

/// How a delete ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteOutcome {
    Deleted,
    Refused(BackendError),
}

impl DeleteOutcome {
    fn code(self) -> u16 {
        match self {
            DeleteOutcome::Deleted => 0,
            DeleteOutcome::Refused(e) => e.code,
        }
    }
}

/// The `0x000d` reply carries the account id like the official one.
fn delete_result(outcome: DeleteOutcome, account_id: u32) -> DeleteCharacterResult {
    DeleteCharacterResult { result: outcome.code(), account_id }
}

fn delete_character(store: &dyn poe_store::Store, account_id: u32, name: &str) -> DeleteOutcome {
    match store.delete_character(account_id, name) {
        Ok(true) => DeleteOutcome::Deleted,
        // Is the name somebody else's character?
        Ok(false) => match store.all_characters() {
            Ok(all) if all.iter().any(|c| c.name == name) => DeleteOutcome::Refused(be::NOT_CHARACTER_OWNER),
            Ok(_) => DeleteOutcome::Refused(be::CHARACTER_DOES_NOT_EXIST),
            Err(e) => {
                warn!("delete character: {e}");
                DeleteOutcome::Refused(be::DATABASE_ERROR)
            }
        },
        Err(e) => {
            warn!("delete character: {e}");
            DeleteOutcome::Refused(be::DATABASE_ERROR)
        }
    }
}

/// Decides a creation: the new character's row, or the code to refuse with.
fn plan_create(
    server: &LoginServer,
    account: &Account,
    req: &CreateCharacter,
) -> Result<NewCharacter, BackendError> {
    let existing = server.store.characters(account.id).map_err(|e| {
        warn!("create character: {e}");
        be::DATABASE_ERROR
    })?;
    if let Some(refusal) = create_refusal(
        &req.name,
        &req.league,
        &server.config.leagues,
        existing.len(),
        usize::from(server.config.accounts.max_characters),
    ) {
        return Err(refusal);
    }
    let start = match StartState::find(&server.data, &req.start_state) {
        Ok(Some(start)) => start,
        Ok(None) => {
            warn!(start_state = %req.start_state, "unknown start state");
            return Err(be::CHARACTER_INVALID_CLASS);
        }
        Err(e) => {
            warn!("create character: {e}");
            return Err(be::DATABASE_ERROR);
        }
    };
    Ok(NewCharacter {
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
    })
}

async fn create_character(
    server: &LoginServer,
    conn: &mut Connection,
    account: &Account,
    req: CreateCharacter,
) -> anyhow::Result<()> {
    info!(name = %req.name, league = %req.league, start_state = %req.start_state, unk_50 = req.unk_50, "create character");
    let outcome = plan_create(server, account, &req).and_then(|new| match server.store.create_character(&new) {
        Ok(Some(c)) => {
            info!(id = c.id, start_state = %new.start_state, "character created");
            Ok(())
        }
        Ok(None) => Err(be::CHARACTER_NAME_ALREADY_EXISTS),
        Err(e) => {
            warn!("create character: {e}");
            Err(be::DATABASE_ERROR)
        }
    });
    if let Err(e) = outcome {
        let text = crate::codes::text(&server.data, e.code).unwrap_or_default();
        warn!(code = e.code, id = e.id, text, "character creation refused");
    }
    let result = create_result(outcome.err(), account.id);
    conn.send(&server.protocol.login.encode(&result)?).await?;
    if outcome.is_ok() {
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

#[cfg(test)]
mod tests {
    use poe_config::LeagueConfig;

    use super::*;

    fn protocol() -> Protocol {
        Protocol::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/protocol/opcodes/0.5.5e")).unwrap()
    }

    fn league(id: &str) -> LeagueConfig {
        LeagueConfig { id: id.into(), hardcore: false, permanent: true, variant_of: None, flag: None }
    }

    #[test]
    fn name_rules_pick_the_client_codes() {
        assert_eq!(validate_name("Abc"), Ok(()));
        assert_eq!(validate_name("a_b_9"), Ok(()));
        assert_eq!(validate_name(""), Err(be::CHARACTER_NAME_TOO_SHORT));
        assert_eq!(validate_name("ab"), Err(be::CHARACTER_NAME_TOO_SHORT));
        assert_eq!(validate_name(&"x".repeat(23)), Ok(()));
        assert_eq!(validate_name(&"x".repeat(24)), Err(be::CHARACTER_NAME_TOO_LONG));
        assert_eq!(validate_name("bad name"), Err(be::CHARACTER_NAME_INVALID));
        assert_eq!(validate_name("Exile!"), Err(be::CHARACTER_NAME_INVALID));
        assert_eq!(be::CHARACTER_NAME_INVALID.code, 22);
    }

    #[test]
    fn creation_refusals_in_order() {
        let leagues = [league("Standard")];
        let r = |name: &str, lg: &str, existing| create_refusal(name, lg, &leagues, existing, 3);
        assert_eq!(r("Abc", "Standard", 2), None);
        assert_eq!(r("A", "Standard", 0), Some(be::CHARACTER_NAME_TOO_SHORT));
        assert_eq!(r("Abc", "Nope", 0), Some(be::LEAGUE_DOES_NOT_EXIST));
        assert_eq!(r("Abc", "Standard", 3), Some(be::TOO_MANY_CHARACTERS));
        assert_eq!(be::LEAGUE_DOES_NOT_EXIST.code, 26);
        assert_eq!(be::TOO_MANY_CHARACTERS.code, 49);
    }

    /// The failure replies as they go over the wire (opcodes from `login.toml`).
    #[test]
    fn create_and_delete_failures_on_the_wire() {
        let p = protocol();
        let wire = |r: CharacterCreated| p.login.encode(&r).unwrap();
        // 0x0013, result, notify 0, account id.
        assert_eq!(wire(create_result(Some(be::CHARACTER_NAME_ALREADY_EXISTS), 9)), [0, 0x13, 0, 21, 0, 0, 0, 0, 9]);
        assert_eq!(wire(create_result(Some(be::CHARACTER_INVALID_CLASS), 9)), [0, 0x13, 0, 25, 0, 0, 0, 0, 9]);
        assert_eq!(wire(create_result(Some(be::DATABASE_ERROR), 9)), [0, 0x13, 0, 19, 0, 0, 0, 0, 9]);
        assert_eq!(wire(create_result(None, 9)), [0, 0x13, 0, 0, 0, 0, 0, 0, 9]);

        let del = |o| p.login.encode(&delete_result(o, 0x0102)).unwrap();
        // 0x000d, result, account id (official replies carry it; before this it was 0).
        assert_eq!(del(DeleteOutcome::Deleted), [0, 0x0d, 0, 0, 0, 0, 1, 2]);
        assert_eq!(del(DeleteOutcome::Refused(be::CHARACTER_DOES_NOT_EXIST)), [0, 0x0d, 0, 16, 0, 0, 1, 2]);
        assert_eq!(del(DeleteOutcome::Refused(be::NOT_CHARACTER_OWNER)), [0, 0x0d, 0, 17, 0, 0, 1, 2]);
    }

    #[test]
    fn delete_decides_between_missing_and_foreign() {
        use poe_store::{SqliteStore, Store};
        let store = SqliteStore::in_memory().unwrap();
        let (a, b) = (store.create_account("a", "Exile").unwrap(), store.create_account("b", "Exile").unwrap());
        let new = |account_id, name: &str| NewCharacter {
            account_id,
            name: name.into(),
            league: "Standard".into(),
            class_id: 2,
            start_state: "DexFourDefault".into(),
            level: 1,
            area: "G1_1".into(),
        };
        store.create_character(&new(b.id, "Theirs")).unwrap().unwrap();
        store.create_character(&new(a.id, "Mine")).unwrap().unwrap();
        let refused = |e| DeleteOutcome::Refused(e);
        assert_eq!(delete_character(&store, a.id, "Theirs"), refused(be::NOT_CHARACTER_OWNER));
        assert_eq!(delete_character(&store, a.id, "Nobody"), refused(be::CHARACTER_DOES_NOT_EXIST));
        assert_eq!(delete_character(&store, a.id, "Mine"), DeleteOutcome::Deleted);
        assert_eq!(delete_character(&store, a.id, "Mine"), refused(be::CHARACTER_DOES_NOT_EXIST));
    }
}
