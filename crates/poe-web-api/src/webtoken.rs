//! Single-use web sign-in tokens: the hand-off from the launcher to the browser.
//!
//! The launcher holds a long-lived launcher token; the browser needs a web session without the
//! user typing a password again. `POST /web-token` (launcher token) mints a random 256-bit
//! token that is valid for `[web] web_token_ttl_secs` and can be redeemed exactly once
//! (`POST /web-token/redeem`) for a normal web session.
//!
//! The tokens live in memory only (a restart drops them, which is fine for a 60 second
//! hand-off) and only as their SHA-256, so the raw value exists nowhere on the server after it
//! was sent. Lookup is by the hash of a random key, so there is nothing to compare in constant
//! time and no timing to learn from.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use poe_accounts::token_hash;
use rand::RngCore;

/// Most pending tokens at once (a flood of mint requests cannot grow the map without bound).
const MAX_PENDING: usize = 1024;

struct Pending {
    account_id: u32,
    expires: Instant,
}

/// Why [`WebTokens::mint`] refused.
#[derive(Debug, PartialEq, Eq)]
pub enum MintError {
    /// [`MAX_PENDING`] unexpired tokens exist.
    Full,
}

/// The pending hand-off tokens.
#[derive(Default)]
pub struct WebTokens {
    pending: Mutex<HashMap<[u8; 32], Pending>>,
}

impl WebTokens {
    /// A new token for the account, valid for `ttl` from `now`. An older pending token of the
    /// same account is dropped: the newest "Open web admin" click wins.
    pub fn mint(
        &self,
        account_id: u32,
        ttl: Duration,
        now: Instant,
    ) -> Result<[u8; 32], MintError> {
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, p| p.expires > now && p.account_id != account_id);
        if pending.len() >= MAX_PENDING {
            return Err(MintError::Full);
        }
        let mut token = [0u8; 32];
        while token == [0u8; 32] {
            rand::rng().fill_bytes(&mut token);
        }
        pending.insert(
            token_hash(&token),
            Pending {
                account_id,
                expires: now + ttl,
            },
        );
        Ok(token)
    }

    /// Consumes a token: the account it was minted for, or `None` when it is unknown, already
    /// used or expired. A token that is found is gone afterwards, whatever the outcome.
    pub fn redeem(&self, token: &[u8; 32], now: Instant) -> Option<u32> {
        let p = self.pending.lock().unwrap().remove(&token_hash(token))?;
        (p.expires > now).then_some(p.account_id)
    }

    /// Number of tokens waiting (expired ones included until the next mint).
    pub fn len(&self) -> usize {
        self.pending.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(60);

    #[test]
    fn a_token_redeems_once_for_its_account() {
        let t = WebTokens::default();
        let now = Instant::now();
        let token = t.mint(7, TTL, now).unwrap();
        assert_eq!(t.redeem(&token, now), Some(7));
        assert_eq!(t.redeem(&token, now), None, "single use");
        assert!(t.is_empty());
    }

    #[test]
    fn an_expired_token_is_refused_and_removed() {
        let t = WebTokens::default();
        let now = Instant::now();
        let token = t.mint(7, TTL, now).unwrap();
        assert_eq!(t.redeem(&token, now + TTL), None);
        assert!(t.is_empty(), "looking it up consumed it");
        let token = t.mint(7, TTL, now).unwrap();
        assert_eq!(
            t.redeem(&token, now + TTL - Duration::from_millis(1)),
            Some(7)
        );
    }

    #[test]
    fn unknown_tokens_are_refused() {
        let t = WebTokens::default();
        let now = Instant::now();
        let token = t.mint(7, TTL, now).unwrap();
        let mut other = token;
        other[0] ^= 1;
        assert_eq!(t.redeem(&other, now), None);
        assert_eq!(
            t.redeem(&token, now),
            Some(7),
            "a wrong guess keeps the real token"
        );
    }

    #[test]
    fn a_new_token_replaces_the_accounts_older_one() {
        let t = WebTokens::default();
        let now = Instant::now();
        let a = t.mint(1, TTL, now).unwrap();
        let b = t.mint(1, TTL, now).unwrap();
        let other = t.mint(2, TTL, now).unwrap();
        assert_eq!(t.redeem(&a, now), None);
        assert_eq!(t.redeem(&b, now), Some(1));
        assert_eq!(t.redeem(&other, now), Some(2));
    }

    #[test]
    fn the_pending_set_is_bounded_and_expired_tokens_make_room() {
        let t = WebTokens::default();
        let now = Instant::now();
        for id in 0..MAX_PENDING as u32 {
            t.mint(id, TTL, now).unwrap();
        }
        assert_eq!(t.mint(u32::MAX, TTL, now), Err(MintError::Full));
        assert!(t.mint(u32::MAX, TTL, now + TTL).is_ok());
        assert_eq!(t.len(), 1);
    }
}
