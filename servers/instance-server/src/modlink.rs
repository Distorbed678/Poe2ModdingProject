//! The instance server's side of the mod host: what mods may change, and who applies it.
//!
//! The host (`poe-mod-host`) returns the commands a mod submitted during a callback. The code
//! that dispatched the event applies them: the instance actor for everything about its world
//! (a teleport), and a [`ModSink`] for what lives outside the actor (chat, kick).

use std::sync::Arc;

use crate::live::{ChatOut, Live};

/// The name server messages are sent under.
use crate::exec::SERVER_NAME;

/// Chat and kick for the mods' submitted commands. Player ids are session ids
/// (`PlayerSnap::id`).
pub trait ModSink: Send + Sync {
    /// A server message to one player; `false` when they are gone.
    fn send_chat(&self, player: u64, text: &str) -> bool;
    /// A server message to everybody online.
    fn broadcast(&self, text: &str);
    /// Shows `reason` (when not empty) to the player and disconnects them.
    fn kick(&self, player: u64, reason: &str) -> bool;
}

/// Delivers through the live session registry.
pub struct LiveSink(pub Arc<Live>);

impl ModSink for LiveSink {
    fn send_chat(&self, player: u64, text: &str) -> bool {
        match self.0.character_of(player) {
            Some(name) => self.0.tell(&name, ChatOut { sender: SERVER_NAME.into(), text: text.to_owned() }),
            None => false,
        }
    }

    fn broadcast(&self, text: &str) {
        self.0.broadcast(ChatOut { sender: SERVER_NAME.into(), text: text.to_owned() }, |_| true);
    }

    fn kick(&self, player: u64, reason: &str) -> bool {
        let Some(name) = self.0.character_of(player) else {
            return false;
        };
        if !reason.is_empty() {
            self.0.tell(&name, ChatOut { sender: SERVER_NAME.into(), text: reason.to_owned() });
        }
        self.0.kick(&name)
    }
}

/// Drops everything (tests, a server without mods).
pub struct NullSink;

impl ModSink for NullSink {
    fn send_chat(&self, _: u64, _: &str) -> bool {
        false
    }
    fn broadcast(&self, _: &str) {}
    fn kick(&self, _: u64, _: &str) -> bool {
        false
    }
}
