//! Bans (web admin, Players page; table `bans`, migration 0007).
//!
//! An account ban refuses the account's logins everywhere [`AccountStore::authenticate`] and
//! [`AccountStore::authenticate_token`] are used (game login, web login, tokens) with
//! [`AccountError::Banned`], whose text reaches the player. An IP ban is checked by the
//! servers where a connection or login arrives ([`AccountStore::active_ip_ban`]).
//!
//! [`AccountStore`]: crate::AccountStore
//! [`AccountError`]: crate::AccountError

use std::net::IpAddr;

/// What a ban applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BanKind {
    Account,
    Ip,
}

impl BanKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Ip => "ip",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "account" => Some(Self::Account),
            "ip" => Some(Self::Ip),
            _ => None,
        }
    }
}

/// One ban row, active or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BanRecord {
    pub id: u32,
    pub kind: BanKind,
    pub account_id: Option<u32>,
    /// The banned account's name (for account bans).
    pub account: Option<String>,
    /// Canonical text of the banned address (for IP bans).
    pub ip: Option<String>,
    pub reason: String,
    pub created_by: String,
    /// Unix seconds.
    pub created_at: i64,
    /// `None`: permanent.
    pub expires_at: Option<i64>,
    pub lifted_at: Option<i64>,
    pub lifted_by: Option<String>,
}

impl BanRecord {
    /// In force at `now`: not lifted and not expired.
    pub fn is_active(&self, now: i64) -> bool {
        self.lifted_at.is_none() && self.expires_at.is_none_or(|e| e > now)
    }

    /// `active`, `lifted` or `expired`.
    pub fn status(&self, now: i64) -> &'static str {
        if self.lifted_at.is_some() {
            "lifted"
        } else if self.is_active(now) {
            "active"
        } else {
            "expired"
        }
    }

    /// The text a banned player sees.
    pub fn notice(&self) -> String {
        let what = match self.kind {
            BanKind::Account => "This account is banned",
            BanKind::Ip => "Connections from your address are banned",
        };
        banned_text(what, &self.reason, self.expires_at)
    }
}

/// `<what> until <date>. Reason: <reason>` (or "permanently").
pub fn banned_text(what: &str, reason: &str, until: Option<i64>) -> String {
    let mut out = match until {
        Some(t) => format!("{what} until {}", format_utc(t)),
        None => format!("{what} permanently"),
    };
    out.push('.');
    if !reason.is_empty() {
        out.push_str(" Reason: ");
        out.push_str(reason);
        if !reason.ends_with(['.', '!', '?']) {
            out.push('.');
        }
    }
    out
}

/// Unix seconds to (year, month, day, hour, minute, second) in UTC.
fn civil(unix: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, secs / 3600, secs % 3600 / 60, secs % 60)
}

/// `2026-10-29 04:00 UTC`.
pub fn format_utc(unix: i64) -> String {
    let (y, m, d, hh, mm, _) = civil(unix);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02} UTC")
}

/// `20261029-040000`, for file names (UTC, sorts by time).
pub fn format_stamp(unix: i64) -> String {
    let (y, m, d, hh, mm, ss) = civil(unix);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// The canonical text of an address as stored in `bans.ip` (IPv4-mapped IPv6 as IPv4).
pub fn canonical_ip(ip: IpAddr) -> String {
    match ip {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map_or_else(|| v6.to_string(), |v4| v4.to_string()),
        v4 => v4.to_string(),
    }
}

/// Parses user input into the canonical text; `None` if it is not an address.
pub fn parse_ip(text: &str) -> Option<String> {
    text.trim().parse::<IpAddr>().ok().map(canonical_ip)
}

/// Longest stored reason (characters).
pub const REASON_MAX: usize = 200;

/// A reason as stored: whitespace collapsed, at most [`REASON_MAX`] characters; `None` if it
/// is longer.
pub fn clean_reason(text: &str) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (text.chars().count() <= REASON_MAX).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_formatted_in_utc() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(
            format_utc(951_782_400 + 3600 * 23 + 60 * 59),
            "2000-02-29 23:59 UTC"
        );
        assert_eq!(format_utc(1_792_000_000), "2026-10-14 17:46 UTC");
        assert_eq!(format_utc(-86_400), "1969-12-31 00:00 UTC");
        assert_eq!(format_stamp(1_792_000_000 + 5), "20261014-174645");
    }

    #[test]
    fn notices_name_expiry_and_reason() {
        let mut b = BanRecord {
            id: 1,
            kind: BanKind::Account,
            account_id: Some(1),
            account: Some("a".into()),
            ip: None,
            reason: "cheating".into(),
            created_by: "root".into(),
            created_at: 0,
            expires_at: Some(86_400),
            lifted_at: None,
            lifted_by: None,
        };
        assert_eq!(
            b.notice(),
            "This account is banned until 1970-01-02 00:00 UTC. Reason: cheating."
        );
        b.expires_at = None;
        b.reason.clear();
        assert_eq!(b.notice(), "This account is banned permanently.");
        b.kind = BanKind::Ip;
        b.reason = "spam!".into();
        assert_eq!(
            b.notice(),
            "Connections from your address are banned permanently. Reason: spam!"
        );
        assert!(b.is_active(10) && b.status(10) == "active");
        b.expires_at = Some(10);
        assert!(!b.is_active(10) && b.status(10) == "expired" && b.is_active(9));
        b.lifted_at = Some(5);
        assert_eq!(b.status(1), "lifted");
    }

    #[test]
    fn addresses_are_canonical() {
        assert_eq!(parse_ip(" 10.0.0.1 ").as_deref(), Some("10.0.0.1"));
        assert_eq!(parse_ip("::ffff:10.0.0.1").as_deref(), Some("10.0.0.1"));
        assert_eq!(parse_ip("2001:DB8::1").as_deref(), Some("2001:db8::1"));
        assert_eq!(parse_ip("10.0.0.0/8"), None);
        assert_eq!(parse_ip("example.com"), None);
        assert_eq!(clean_reason("  a \n b  ").as_deref(), Some("a b"));
        assert_eq!(clean_reason(&"x".repeat(REASON_MAX + 1)), None);
    }
}
