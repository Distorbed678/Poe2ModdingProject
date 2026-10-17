//! Chat routing: a line typed by a player goes to its channel (the client's own prefixes:
//! `#` global, `$` trade, `@name` whisper, `%` party, `&` guild, none = local), server
//! commands (`[commands] prefix`) run instead of being sent.

use crate::live::{ChatOut, Live};

/// Where a typed line goes.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// A server command line (without the prefix).
    Command(String),
    Global(String),
    Trade(String),
    Whisper { to: String, text: String },
    Party(String),
    Guild(String),
    /// Players in the same instance.
    Local(String),
}

pub fn route(text: &str, command_prefix: &str) -> Route {
    if !command_prefix.is_empty() {
        if let Some(rest) = text.strip_prefix(command_prefix) {
            return Route::Command(rest.trim().to_owned());
        }
    }
    let rest = |p: char| text[p.len_utf8()..].trim_start().to_owned();
    match text.chars().next() {
        Some('#') => Route::Global(rest('#')),
        Some('$') => Route::Trade(rest('$')),
        Some('%') => Route::Party(rest('%')),
        Some('&') => Route::Guild(rest('&')),
        Some('@') => {
            let body = &text[1..];
            let (to, text) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
            Route::Whisper { to: to.to_owned(), text: text.trim_start().to_owned() }
        }
        _ => Route::Local(text.to_owned()),
    }
}

/// Delivers a player's line; returns an error line for the sender, if any.
pub fn deliver(live: &Live, from: &str, instance: Option<u32>, route: Route) -> Option<String> {
    if live.is_muted(from) {
        return Some("You are muted.".into());
    }
    let (global, trade) = live.channels(from);
    match route {
        Route::Command(_) => None,
        Route::Global(text) => {
            let line = ChatOut { sender: format!("#{from}"), text };
            live.broadcast(line, |s| s.global.is_some() && s.global == global);
            None
        }
        Route::Trade(text) => {
            let line = ChatOut { sender: format!("${from}"), text };
            live.broadcast(line, |s| s.trade.is_some() && s.trade == trade);
            None
        }
        Route::Local(text) => {
            let line = ChatOut { sender: from.to_owned(), text };
            live.broadcast(line, |s| s.instance == instance);
            None
        }
        Route::Whisper { to, text } => {
            let Some(target) = live.find(&to) else {
                return Some(format!("{to} is not online."));
            };
            live.tell(&target, ChatOut { sender: format!("@From {from}"), text: text.clone() });
            live.tell(from, ChatOut { sender: format!("@To {target}"), text });
            None
        }
        Route::Party(_) => Some("Parties are not supported yet.".into()),
        Route::Guild(_) => Some("Guilds are not supported yet.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes() {
        assert_eq!(route("!who", "!"), Route::Command("who".into()));
        assert_eq!(route("#hi all", "!"), Route::Global("hi all".into()));
        assert_eq!(route("$WTS x", "!"), Route::Trade("WTS x".into()));
        assert_eq!(route("@Bob hey there", "!"), Route::Whisper { to: "Bob".into(), text: "hey there".into() });
        assert_eq!(route("hello", "!"), Route::Local("hello".into()));
    }

    #[test]
    fn delivers_by_channel() {
        let live = Live::default();
        let mut a = live.register(1, "A", "Alice", Some(1), "G1_1", 1);
        let mut b = live.register(2, "B", "Bob", Some(2), "G1_2", 2);
        live.set_channel("Alice", false, 820);
        live.set_channel("Bob", false, 820);
        assert_eq!(deliver(&live, "Alice", Some(1), route("#yo", "!")), None);
        assert_eq!(b.chat.try_recv().unwrap().sender, "#Alice");
        assert_eq!(a.chat.try_recv().unwrap().text, "yo");
        // Local stays in the instance.
        deliver(&live, "Alice", Some(1), route("near", "!"));
        assert!(b.chat.try_recv().is_err());
        assert!(deliver(&live, "Alice", Some(1), route("@Nobody x", "!")).is_some());
        live.mute("Alice", None);
        assert!(deliver(&live, "Alice", Some(1), route("#x", "!")).is_some());
    }
}
