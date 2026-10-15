//! Coordination between the login server and instance servers.
//!
//! When a player picks a character, the login server issues a [`Ticket`] and tells the
//! client where to connect. The instance server redeems the ticket to learn who connected,
//! which character and area to load, and which keys encrypt the stream. [`TicketRegistry`]
//! is a trait so a networked implementation can replace [`MemoryTickets`] when the servers
//! run as separate processes. [`LoginSessions`] lets an instance server end an account's
//! login connection (the same split applies).

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use rand::RngCore;
use tokio::sync::watch;

/// The character being handed off (what the instance server needs to spawn it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketCharacter {
    pub id: u32,
    pub name: String,
    /// The character's league (sent in AreaInfo).
    pub league: String,
    /// `Characters.IntegerId` of the class.
    pub class_id: u32,
    pub level: u32,
    /// The start area's intro has been played (otherwise new characters wash up).
    pub intro_done: bool,
    /// Where the character left the game in the ticket's area: (terrain seed, grid x, grid y).
    pub location: Option<(u32, i32, i32)>,
}

/// A pending hand-off to an instance server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// Value sent to the client (and expected back from it).
    pub id: u32,
    pub account_id: u32,
    pub character: TicketCharacter,
    /// `WorldAreas.Id` to load.
    pub area: String,
    /// 64 bytes of stream key material for the instance connection.
    pub keys: [u8; 64],
}

/// Lets a client that leaves an instance for character selection log back in without
/// credentials: the instance server sends it to the client (`0x174`), and the client presents
/// it to the login server (`0x08`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeTicket {
    pub account_id: u32,
    pub token: u64,
}

pub trait TicketRegistry: Send + Sync {
    /// Issues a single-use resume ticket for an account.
    fn issue_resume(&self, account_id: u32) -> ResumeTicket;
    /// Takes a resume ticket out of the registry; `true` if it was valid.
    fn redeem_resume(&self, ticket: ResumeTicket) -> bool;
    /// Issues a ticket with a fresh id and keys.
    fn issue(&self, account_id: u32, character: TicketCharacter, area: &str) -> Ticket;
    /// Takes a ticket out of the registry (single use).
    fn redeem(&self, id: u32) -> Option<Ticket>;
    /// All unredeemed tickets (used to match a connection when the client does not send
    /// the id in clear).
    fn pending(&self) -> Vec<Ticket>;
}

/// In-process registry with an expiry time.
pub struct MemoryTickets {
    ttl: Duration,
    tickets: Mutex<HashMap<u32, (Instant, Ticket)>>,
    resume: Mutex<HashMap<u64, (Instant, ResumeTicket)>>,
}

impl MemoryTickets {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            tickets: Mutex::new(HashMap::new()),
            resume: Mutex::new(HashMap::new()),
        }
    }

    fn purge(&self, map: &mut HashMap<u32, (Instant, Ticket)>) {
        let ttl = self.ttl;
        map.retain(|_, (at, _)| at.elapsed() < ttl);
    }
}

impl TicketRegistry for MemoryTickets {
    fn issue_resume(&self, account_id: u32) -> ResumeTicket {
        let mut rng = rand::rng();
        let mut map = self.resume.lock().unwrap();
        let ttl = self.ttl;
        map.retain(|_, (at, _)| at.elapsed() < ttl);
        let token = loop {
            let token = rng.next_u64();
            if token != 0 && !map.contains_key(&token) {
                break token;
            }
        };
        let ticket = ResumeTicket { account_id, token };
        map.insert(token, (Instant::now(), ticket));
        ticket
    }

    fn redeem_resume(&self, ticket: ResumeTicket) -> bool {
        let mut map = self.resume.lock().unwrap();
        let ttl = self.ttl;
        map.retain(|_, (at, _)| at.elapsed() < ttl);
        match map.get(&ticket.token) {
            Some((_, t)) if *t == ticket => {
                map.remove(&ticket.token);
                true
            }
            _ => false,
        }
    }

    fn issue(&self, account_id: u32, character: TicketCharacter, area: &str) -> Ticket {
        let mut rng = rand::rng();
        let mut keys = [0u8; 64];
        rng.fill_bytes(&mut keys);
        let mut map = self.tickets.lock().unwrap();
        self.purge(&mut map);
        let id = loop {
            let id = rng.next_u32();
            if id != 0 && !map.contains_key(&id) {
                break id;
            }
        };
        let ticket = Ticket {
            id,
            account_id,
            character,
            area: area.to_owned(),
            keys,
        };
        map.insert(id, (Instant::now(), ticket.clone()));
        ticket
    }

    fn redeem(&self, id: u32) -> Option<Ticket> {
        let mut map = self.tickets.lock().unwrap();
        self.purge(&mut map);
        map.remove(&id).map(|(_, t)| t)
    }

    fn pending(&self) -> Vec<Ticket> {
        let mut map = self.tickets.lock().unwrap();
        self.purge(&mut map);
        map.values().map(|(_, t)| t.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tickets_are_single_use() {
        let reg = MemoryTickets::new(Duration::from_secs(60));
        let character = TicketCharacter {
            location: None,
            id: 2,
            name: "test".into(),
            league: "Standard".into(),
            class_id: 2,
            level: 1,
            intro_done: false,
        };
        let t = reg.issue(1, character, "G1_1");
        assert_eq!(reg.pending().len(), 1);
        assert_eq!(reg.redeem(t.id), Some(t.clone()));
        assert_eq!(reg.redeem(t.id), None);
    }

    #[test]
    fn resume_tickets_match_account_and_are_single_use() {
        let reg = MemoryTickets::new(Duration::from_secs(60));
        let t = reg.issue_resume(7);
        assert!(!reg.redeem_resume(ResumeTicket { account_id: 8, ..t }));
        assert!(reg.redeem_resume(t));
        assert!(!reg.redeem_resume(t));
    }
}

/// Login connections by account. The client keeps its login connection open while in game;
/// leaving the game ("Exit to log in screen" or quitting) sends instance `0x1b`, and the
/// client shows the log-in screen only if that connection is gone (otherwise it goes back to
/// character selection on it). So the instance server ends the account's login session here.
pub trait LoginSessions: Send + Sync {
    /// Called by a login connection once its account is known; the receiver changes when the
    /// account logs out.
    fn subscribe(&self, account_id: u32) -> watch::Receiver<u64>;
    /// Ends the account's login connections.
    fn logout(&self, account_id: u32);
}

/// In-process [`LoginSessions`].
#[derive(Default)]
pub struct MemorySessions {
    accounts: Mutex<HashMap<u32, watch::Sender<u64>>>,
}

impl LoginSessions for MemorySessions {
    fn subscribe(&self, account_id: u32) -> watch::Receiver<u64> {
        self.accounts
            .lock()
            .unwrap()
            .entry(account_id)
            .or_insert_with(|| watch::channel(0).0)
            .subscribe()
    }

    fn logout(&self, account_id: u32) {
        if let Some(tx) = self.accounts.lock().unwrap().get(&account_id) {
            tx.send_modify(|n| *n += 1);
        }
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[tokio::test]
    async fn logout_reaches_the_accounts_connections_only() {
        let sessions = MemorySessions::default();
        let mut a = sessions.subscribe(1);
        let b = sessions.subscribe(2);
        sessions.logout(1);
        assert!(a.has_changed().unwrap());
        assert!(!b.has_changed().unwrap());
        a.changed().await.unwrap();
        // A connection subscribed after a logout is not ended by it.
        let late = sessions.subscribe(1);
        assert!(!late.has_changed().unwrap());
        sessions.logout(3);
        assert!(!b.has_changed().unwrap());
    }
}
