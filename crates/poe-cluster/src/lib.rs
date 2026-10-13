//! Coordination between the login server and instance servers.
//!
//! When a player picks a character, the login server issues a [`Ticket`] and tells the
//! client where to connect. The instance server redeems the ticket to learn who connected,
//! which character and area to load, and which keys encrypt the stream. [`TicketRegistry`]
//! is a trait so a networked implementation can replace [`MemoryTickets`] when the servers
//! run as separate processes.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use rand::RngCore;

/// The character being handed off (what the instance server needs to spawn it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketCharacter {
    pub id: u32,
    pub name: String,
    /// `Characters.IntegerId` of the class.
    pub class_id: u32,
    pub level: u32,
    /// The start area's intro has been played (otherwise new characters wash up).
    pub intro_done: bool,
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

pub trait TicketRegistry: Send + Sync {
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
}

impl MemoryTickets {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            tickets: Mutex::new(HashMap::new()),
        }
    }

    fn purge(&self, map: &mut HashMap<u32, (Instant, Ticket)>) {
        let ttl = self.ttl;
        map.retain(|_, (at, _)| at.elapsed() < ttl);
    }
}

impl TicketRegistry for MemoryTickets {
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
            id: 2,
            name: "test".into(),
            class_id: 2,
            level: 1,
            intro_done: false,
        };
        let t = reg.issue(1, character, "G1_1");
        assert_eq!(reg.pending().len(), 1);
        assert_eq!(reg.redeem(t.id), Some(t.clone()));
        assert_eq!(reg.redeem(t.id), None);
    }
}
