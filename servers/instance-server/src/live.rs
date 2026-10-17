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
    /// The client's TCP port (debug link key).
    pub port: u16,
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
}

/// One `who` row.
#[derive(Debug, Clone)]
pub struct Online {
    pub character: String,
    pub account: String,
    pub instance: Option<u32>,
    pub area: String,
}

impl Live {
    pub fn register(&self, account_id: u32, account: &str, character: &str, instance: Option<u32>, area: &str, port: u16) -> Registration {
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
                port,
                kick,
                chat,
            },
        );
        Registration { kicked, chat: rx }
    }

    /// Removes a session (only if it is still the one on `port`: a newer session of the same
    /// character may have replaced it).
    pub fn unregister(&self, character: &str, port: u16) {
        let mut map = self.sessions.lock().unwrap();
        if map.get(&character.to_lowercase()).is_some_and(|s| s.port == port) {
            map.remove(&character.to_lowercase());
        }
    }

    pub fn online(&self) -> Vec<Online> {
        let mut out: Vec<Online> = self
            .sessions
            .lock()
            .unwrap()
            .values()
            .map(|s| Online { character: s.character.clone(), account: s.account.clone(), instance: s.instance, area: s.area.clone() })
            .collect();
        out.sort_by(|a, b| a.character.cmp(&b.character));
        out
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
