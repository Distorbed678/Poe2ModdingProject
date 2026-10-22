//! The sessions in game right now (for `who`, `kick`, chat delivery and mutes).

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use tokio::sync::{mpsc, watch};

/// A chat line for a session to send to its client (`ChatMessage`).
#[derive(Debug, Clone)]
pub struct ChatOut {
    /// Channel prefix and name (`#name` global, `$name` trade, plain = local).
    pub sender: String,
    pub text: String,
}

pub struct LiveSession {
    pub account_id: u32,
    pub account: String,
    pub character: String,
    pub instance: Option<u32>,
    pub area: String,
    /// The session (one connection; [`crate::actor::SessionId`]). Not the client's address:
    /// clients on different machines can share a port.
    pub session: u64,
    /// When the session registered (unix seconds).
    pub connected_at: i64,
    /// The client's address, when known (the web admin shows it to admins).
    pub ip: Option<std::net::IpAddr>,
    /// Joined global and trade channel numbers (ChatJoinRequest).
    pub global: Option<u16>,
    pub trade: Option<u16>,
    kick: watch::Sender<bool>,
    chat: mpsc::UnboundedSender<ChatOut>,
}

/// What a session gets when it registers: its kick signal and its chat queue.
pub struct Registration {
    pub kicked: watch::Receiver<bool>,
    pub chat: mpsc::UnboundedReceiver<ChatOut>,
}

#[derive(Default)]
pub struct Live {
    sessions: Mutex<HashMap<String, LiveSession>>,
    /// Muted characters (lower case) and until when (`None`: until unmuted).
    muted: Mutex<HashMap<String, Option<Instant>>>,
    /// Party invitations and trade requests between the sessions.
    pub social: crate::social::Social,
}

/// One `who` row.
#[derive(Debug, Clone)]
pub struct Online {
    pub character: String,
    pub account: String,
    pub account_id: u32,
    /// Unix seconds.
    pub connected_at: i64,
    pub ip: Option<std::net::IpAddr>,
    pub instance: Option<u32>,
    pub area: String,
}

impl Live {
    pub fn register(&self, account_id: u32, account: &str, character: &str, instance: Option<u32>, area: &str, session: u64) -> Registration {
        self.register_from(account_id, account, character, instance, area, session, None)
    }

    /// [`register`](Self::register) with the client's address.
    #[allow(clippy::too_many_arguments)]
    pub fn register_from(&self, account_id: u32, account: &str, character: &str, instance: Option<u32>, area: &str, session: u64, ip: Option<std::net::IpAddr>) -> Registration {
        let connected_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let (kick, kicked) = watch::channel(false);
        let (chat, rx) = mpsc::unbounded_channel();
        self.sessions.lock().unwrap().insert(
            character.to_lowercase(),
            LiveSession {
                account_id,
                account: account.to_owned(),
                character: character.to_owned(),
                instance,
                area: area.to_owned(),
                session,
                connected_at,
                ip,
                global: None,
                trade: None,
                kick,
                chat,
            },
        );
        Registration { kicked, chat: rx }
    }

    /// Removes a session (only if it is still `session`: a newer session of the same
    /// character may have replaced it).
    pub fn unregister(&self, character: &str, session: u64) {
        let mut map = self.sessions.lock().unwrap();
        if map.get(&character.to_lowercase()).is_some_and(|s| s.session == session) {
            map.remove(&character.to_lowercase());
        }
    }

    pub fn online(&self) -> Vec<Online> {
        let mut out: Vec<Online> = self
            .sessions
            .lock()
            .unwrap()
            .values()
            .map(|s| Online {
                character: s.character.clone(),
                account: s.account.clone(),
                account_id: s.account_id,
                connected_at: s.connected_at,
                ip: s.ip,
                instance: s.instance,
                area: s.area.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.character.cmp(&b.character));
        out
    }

    /// The account id and exact name of an online character.
    pub fn account_of(&self, character: &str) -> Option<(u32, String)> {
        self.sessions.lock().unwrap().get(&character.to_lowercase()).map(|s| (s.account_id, s.character.clone()))
    }

    /// The character name of an online session (`PlayerSnap::id` is a session id).
    pub fn character_of(&self, session: u64) -> Option<String> {
        self.sessions.lock().unwrap().values().find(|s| s.session == session).map(|s| s.character.clone())
    }

    /// The character's exact name if it is online.
    pub fn find(&self, character: &str) -> Option<String> {
        self.sessions.lock().unwrap().get(&character.to_lowercase()).map(|s| s.character.clone())
    }

    pub fn kick(&self, character: &str) -> bool {
        match self.sessions.lock().unwrap().get(&character.to_lowercase()) {
            Some(s) => {
                s.kick.send_replace(true);
                true
            }
            None => false,
        }
    }

    /// Sends a chat line to one character; `false` if it is not online.
    pub fn tell(&self, character: &str, line: ChatOut) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .get(&character.to_lowercase())
            .is_some_and(|s| s.chat.send(line).is_ok())
    }

    /// Sends a chat line to every session matching `to` (all of them for `|_| true`).
    pub fn broadcast(&self, line: ChatOut, to: impl Fn(&LiveSession) -> bool) -> usize {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| to(s))
            .filter(|s| s.chat.send(line.clone()).is_ok())
            .count()
    }

    /// Records the channel a session joined (`trade`: trade chat, else global).
    pub fn set_channel(&self, character: &str, trade: bool, channel: u16) {
        if let Some(s) = self.sessions.lock().unwrap().get_mut(&character.to_lowercase()) {
            if trade {
                s.trade = Some(channel);
            } else {
                s.global = Some(channel);
            }
        }
    }

    /// The session's (global, trade) channels.
    pub fn channels(&self, character: &str) -> (Option<u16>, Option<u16>) {
        self.sessions.lock().unwrap().get(&character.to_lowercase()).map_or((None, None), |s| (s.global, s.trade))
    }

    pub fn mute(&self, character: &str, minutes: Option<u64>) {
        let until = minutes.map(|m| Instant::now() + Duration::from_secs(m * 60));
        self.muted.lock().unwrap().insert(character.to_lowercase(), until);
    }

    pub fn unmute(&self, character: &str) -> bool {
        self.muted.lock().unwrap().remove(&character.to_lowercase()).is_some()
    }

    pub fn is_muted(&self, character: &str) -> bool {
        let mut map = self.muted.lock().unwrap();
        let key = character.to_lowercase();
        match map.get(&key) {
            Some(Some(until)) if *until <= Instant::now() => {
                map.remove(&key);
                false
            }
            Some(_) => true,
            None => false,
        }
    }
}
