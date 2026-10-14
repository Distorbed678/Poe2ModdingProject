//! Client position reports (dev bridge until the server simulates movement exactly, M4).
//!
//! The client DLL (`poe-hook` `position_sync`) sends one UDP datagram per simulation frame
//! to the instance port with the player's position and current action. The server keys the
//! reports by the client's TCP port and uses the latest one to send positions that match the
//! client (no snapping) and to know when the client's dodge roll ended. Without reports the
//! server falls back to its own simulation.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use tokio::net::UdpSocket;
use tracing::{info, warn};

/// What the client reported for its player in one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientReport {
    /// The DLL's frame counter (player updates).
    pub frame: u32,
    /// Position in grid cells.
    pub x: f32,
    pub y: f32,
    /// Action types (`ActionTypes` keys) of the actor's current actions (0 = none).
    pub action: u16,
    pub action2: u16,
    /// Actor +0x2a2 (bit 5: sprint key held as set by `0x2e7`).
    pub flags: u16,
    pub at: Instant,
}

impl ClientReport {
    /// Parses a datagram: `b"PSY1", u16 tcp port, u16 flags, u32 frame, f32 x, f32 y,
    /// u16 action, u16 action2` (little-endian). Returns the TCP port and the report.
    pub fn parse(data: &[u8]) -> Option<(u16, Self)> {
        if data.len() < 24 || &data[..4] != b"PSY1" {
            return None;
        }
        let u16_at = |i: usize| u16::from_le_bytes([data[i], data[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
        Some((
            u16_at(4),
            Self {
                flags: u16_at(6),
                frame: u32_at(8),
                x: f32::from_bits(u32_at(12)),
                y: f32::from_bits(u32_at(16)),
                action: u16_at(20),
                action2: u16_at(22),
                at: Instant::now(),
            },
        ))
    }

    pub fn cell(&self) -> (i32, i32) {
        (self.x.floor() as i32, self.y.floor() as i32)
    }
}

/// Latest report per client TCP port.
#[derive(Default)]
pub struct ClientReports {
    latest: Mutex<HashMap<u16, ClientReport>>,
}

/// Reports older than this are ignored (the client stopped sending, e.g. paused or gone).
const FRESH: Duration = Duration::from_millis(250);

impl ClientReports {
    pub fn insert(&self, port: u16, report: ClientReport) {
        self.latest.lock().unwrap().insert(port, report);
    }

    /// The latest report from the client on `port`, if recent.
    pub fn fresh(&self, port: u16) -> Option<ClientReport> {
        self.latest
            .lock()
            .unwrap()
            .get(&port)
            .copied()
            .filter(|r| r.at.elapsed() < FRESH)
    }

    pub fn forget(&self, port: u16) {
        self.latest.lock().unwrap().remove(&port);
    }

    /// Receives reports on the instance address (UDP, same port as the TCP listener). Only
    /// loopback senders are accepted.
    pub async fn listen(&self, addr: SocketAddr) -> anyhow::Result<()> {
        let socket = UdpSocket::bind(addr).await?;
        info!(%addr, "client position reports (UDP) listening");
        let mut buf = [0u8; 64];
        loop {
            let (n, from) = match socket.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(e) => {
                    warn!("position report: {e}");
                    continue;
                }
            };
            if !from.ip().is_loopback() {
                continue;
            }
            if let Some((port, report)) = ClientReport::parse(&buf[..n]) {
                self.insert(port, report);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reports() {
        let mut d = b"PSY1".to_vec();
        d.extend(6112u16.to_le_bytes());
        d.extend(0x20u16.to_le_bytes());
        d.extend(7u32.to_le_bytes());
        d.extend(12.5f32.to_le_bytes());
        d.extend(300.25f32.to_le_bytes());
        d.extend(0xed4cu16.to_le_bytes());
        d.extend(0u16.to_le_bytes());
        let (port, r) = ClientReport::parse(&d).unwrap();
        assert_eq!(
            (port, r.frame, r.cell(), r.action, r.flags),
            (6112, 7, (12, 300), 0xed4c, 0x20)
        );
        assert!(ClientReport::parse(&d[..20]).is_none());
    }
}
