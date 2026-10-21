//! Rate limiting of password attempts (`[security]` in `config/server.toml`).
//!
//! One [`LoginLimiter`] is shared by the login server and the web API so an attacker cannot
//! split attempts between them. Failures are counted per account name (case-insensitive,
//! whether or not the account exists, so names cannot be probed through the limit) and per
//! client IP (IPv6 per /64) in a sliding window. When a count reaches its limit the key is
//! locked for the lockout time, counted from that failure; attempts while locked are refused
//! without checking the password and do not extend the lock.
//!
//! Callers: [`check`](LoginLimiter::check) before verifying a password,
//! [`failure`](LoginLimiter::failure) after a wrong one, [`success`](LoginLimiter::success)
//! after a right one (clears the account's count, not the IP's: an attacker with an account of
//! their own must not be able to reset their IP count).

use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use poe_config::SecurityConfig;

/// Refusal: the key is locked for this many more seconds (at least 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locked {
    pub retry_after_secs: u64,
}

#[derive(Default)]
struct Entry {
    /// Times (seconds) of the failures inside the window.
    failures: VecDeque<i64>,
    /// Locked until this time (seconds), 0 = not locked.
    locked_until: i64,
}

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub struct LoginLimiter {
    max_account: u32,
    max_ip: u32,
    window: i64,
    lockout: i64,
    clock: Clock,
    accounts: Mutex<HashMap<String, Entry>>,
    ips: Mutex<HashMap<IpAddr, Entry>>,
}

/// IPv6 addresses are limited per /64 (a host owns the whole prefix); IPv4-mapped IPv6
/// addresses count as the IPv4 address.
fn ip_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let mut o = v6.octets();
                o[8..].fill(0);
                IpAddr::V6(o.into())
            }
        },
        v4 => v4,
    }
}

fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

impl LoginLimiter {
    pub fn from_config(cfg: &SecurityConfig) -> Self {
        Self::new(
            cfg.login_max_failures_per_account,
            cfg.login_max_failures_per_ip,
            cfg.login_failure_window_secs,
            cfg.login_lockout_secs,
        )
    }

    /// A limit of 0 turns that limit off.
    pub fn new(max_per_account: u32, max_per_ip: u32, window_secs: u64, lockout_secs: u64) -> Self {
        Self {
            max_account: max_per_account,
            max_ip: max_per_ip,
            window: window_secs.min(i64::MAX as u64) as i64,
            lockout: lockout_secs.min(i64::MAX as u64) as i64,
            clock: Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64)
            }),
            accounts: Mutex::default(),
            ips: Mutex::default(),
        }
    }

    /// Replaces the clock (seconds), for tests.
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Whether an attempt for this account name from this IP may be checked now.
    pub fn check(&self, account: &str, ip: IpAddr) -> Result<(), Locked> {
        let now = self.now();
        let mut worst = 0;
        if self.max_account > 0
            && let Some(e) = self.accounts.lock().unwrap().get(&name_key(account))
        {
            worst = worst.max(e.locked_until - now);
        }
        if self.max_ip > 0
            && let Some(e) = self.ips.lock().unwrap().get(&ip_key(ip))
        {
            worst = worst.max(e.locked_until - now);
        }
        if worst > 0 {
            Err(Locked {
                retry_after_secs: worst as u64,
            })
        } else {
            Ok(())
        }
    }

    fn record<K: std::hash::Hash + Eq>(
        &self,
        map: &Mutex<HashMap<K, Entry>>,
        key: K,
        max: u32,
        now: i64,
    ) {
        if max == 0 {
            return;
        }
        let mut map = map.lock().unwrap();
        // Keep the maps bounded: drop keys with nothing left to remember.
        if map.len() >= 4096 {
            map.retain(|_, e| {
                e.locked_until > now || e.failures.back().is_some_and(|t| now - t < self.window)
            });
        }
        let e = map.entry(key).or_default();
        if e.locked_until > now {
            return; // refused attempts do not extend the lock
        }
        while e.failures.front().is_some_and(|t| now - t >= self.window) {
            e.failures.pop_front();
        }
        e.failures.push_back(now);
        if e.failures.len() >= max as usize {
            e.locked_until = now + self.lockout.max(1);
            e.failures.clear();
        }
    }

    /// A wrong password (or an unknown account name) for this account name from this IP.
    pub fn failure(&self, account: &str, ip: IpAddr) {
        let now = self.now();
        self.record(&self.accounts, name_key(account), self.max_account, now);
        self.record(&self.ips, ip_key(ip), self.max_ip, now);
    }

    /// A right password: clears the account's failures and lock.
    pub fn success(&self, account: &str) {
        self.accounts.lock().unwrap().remove(&name_key(account));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;

    fn limiter(acct: u32, ip: u32) -> (LoginLimiter, Arc<AtomicI64>) {
        let t = Arc::new(AtomicI64::new(1000));
        let t2 = t.clone();
        (
            LoginLimiter::new(acct, ip, 60, 120).with_clock(move || t2.load(Ordering::SeqCst)),
            t,
        )
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn locks_an_account_after_the_limit_and_releases_it() {
        let (l, t) = limiter(3, 0);
        let a = ip("10.0.0.1");
        for _ in 0..2 {
            assert!(l.check("Alice", a).is_ok());
            l.failure("Alice", a);
        }
        assert!(l.check("Alice", a).is_ok(), "two failures of three");
        l.failure("alice", a); // case-insensitive
        assert_eq!(
            l.check("ALICE", ip("10.0.0.2")),
            Err(Locked {
                retry_after_secs: 120
            })
        );
        assert!(
            l.check("Bob", a).is_ok(),
            "other accounts and the IP (limit off) are free"
        );
        t.fetch_add(100, Ordering::SeqCst);
        assert_eq!(
            l.check("Alice", a),
            Err(Locked {
                retry_after_secs: 20
            })
        );
        // Refused attempts do not extend the lock.
        l.failure("Alice", a);
        assert_eq!(
            l.check("Alice", a),
            Err(Locked {
                retry_after_secs: 20
            })
        );
        t.fetch_add(20, Ordering::SeqCst);
        assert!(l.check("Alice", a).is_ok());
    }

    #[test]
    fn failures_outside_the_window_do_not_count() {
        let (l, t) = limiter(3, 0);
        let a = ip("10.0.0.1");
        l.failure("Alice", a);
        l.failure("Alice", a);
        t.fetch_add(60, Ordering::SeqCst);
        l.failure("Alice", a);
        assert!(l.check("Alice", a).is_ok(), "the first two aged out");
    }

    #[test]
    fn success_clears_the_account_but_not_the_ip() {
        let (l, _) = limiter(3, 3);
        let a = ip("10.0.0.1");
        l.failure("Alice", a);
        l.failure("Alice", a);
        l.success("Alice");
        l.failure("Alice", a); // account: 1 failure; IP: 3 -> locked
        assert!(
            l.check("Bob", a).is_err(),
            "the IP is locked for every account name"
        );
        assert!(
            l.check("Alice", ip("10.0.0.9")).is_ok(),
            "the account is not"
        );
    }

    #[test]
    fn unknown_names_count_and_ipv6_is_per_prefix() {
        let (l, _) = limiter(0, 2);
        l.failure("nobody", ip("2001:db8::1"));
        l.failure("nobody-else", ip("2001:db8::2"));
        assert!(l.check("x", ip("2001:db8::ffff")).is_err(), "same /64");
        assert!(l.check("x", ip("2001:db9::1")).is_ok());
        let (l, _) = limiter(0, 1);
        l.failure("x", ip("::ffff:10.1.1.1"));
        assert!(
            l.check("x", ip("10.1.1.1")).is_err(),
            "IPv4-mapped is the IPv4 address"
        );
    }

    #[test]
    fn zero_limits_are_off() {
        let (l, _) = limiter(0, 0);
        for _ in 0..100 {
            l.failure("Alice", ip("10.0.0.1"));
        }
        assert!(l.check("Alice", ip("10.0.0.1")).is_ok());
    }
}
