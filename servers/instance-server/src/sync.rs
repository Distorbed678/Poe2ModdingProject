//! Client position reports, the debug link and the drift checker (research tools).
//!
//! The client DLL (`poe-hook` `position_sync`) sends one UDP datagram per simulation frame
//! to the instance port with the player's position and current action. Since WP3.4 every
//! datagram is a sealed side-channel frame (`poe_hook_shared::sidechannel`,
//! docs/protocol/side-channel.md): it names the session, carries a counter and an HMAC-SHA256
//! tag made with a key derived from the session's ticket keys. Reports are keyed by session,
//! not by the client's TCP port; datagrams without a valid tag, from an unknown session or with
//! a replayed counter are dropped. Since the movement rewrite (WP2.3) the server simulation is
//! authoritative and **reports never feed it**: they are only compared with the server's
//! position by [`DriftCheck`] (`[dev] drift_check`), which logs the distance in cells.
//! The same UDP socket carries the debug link (overlay commands, state for the overlay).

use std::{
    collections::HashMap,
    fmt::Arguments,
    fs::File,
    io::{BufWriter, Write},
    net::SocketAddr,
    path::Path,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use poe_hook_shared::modreport::ModReport;
use poe_hook_shared::sidechannel::{
    self as sc, Dir, Kind, Reject, ReplayWindow, ReportPayload, SessionKeys,
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
    /// The cursor in the game window's client area and the area's size (from `Report` frames).
    pub cursor: Option<Cursor>,
    pub at: Instant,
}

/// The cursor in the game window: client-area position and size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

impl ClientReport {
    /// The report of a `Kind::Report` payload.
    pub fn from_payload(p: &ReportPayload) -> Self {
        Self {
            flags: p.flags,
            frame: p.frame,
            x: p.x,
            y: p.y,
            action: p.action,
            action2: p.action2,
            cursor: Some(Cursor {
                x: p.cursor.0,
                y: p.cursor.1,
                width: p.window.0,
                height: p.window.1,
            })
            .filter(|c| c.width > 0 && c.height > 0),
            at: Instant::now(),
        }
    }

    pub fn cell(&self) -> (i32, i32) {
        (self.x.floor() as i32, self.y.floor() as i32)
    }
}

/// Distance between two positions in cells.
pub fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Drift checker (`[dev] drift_check`): compares the server's player position with the client's
/// latest position report and logs, every `interval`, the current and the largest distance (in
/// cells) since the last line. The client trails the server by the round trip (about two
/// frames, so up to ~2.5 cells while walking and ~4 while sprinting); a distance that keeps
/// growing, or stays high while standing, is real drift.
pub struct DriftCheck {
    interval: Duration,
    next: Instant,
    max: f32,
    samples: u32,
}

impl DriftCheck {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            next: Instant::now() + interval,
            max: 0.0,
            samples: 0,
        }
    }

    /// Notes one frame; returns a line to log when the interval has passed. `state` describes
    /// the server's movement state, `report` is the client's fresh report (none: the client
    /// sent nothing recently).
    pub fn sample(
        &mut self,
        server: (f32, f32),
        state: impl FnOnce() -> String,
        report: Option<&ClientReport>,
    ) -> Option<DriftLine> {
        let d = report.map(|r| distance(server, (r.x, r.y)));
        if let Some(d) = d {
            self.max = self.max.max(d);
            self.samples += 1;
        }
        if Instant::now() < self.next {
            return None;
        }
        self.next = Instant::now() + self.interval;
        let line = DriftLine {
            server,
            client: report.map(|r| (r.x, r.y)),
            cells: d,
            max_cells: (self.samples > 0).then_some(self.max),
            samples: self.samples,
            state: state(),
        };
        self.max = 0.0;
        self.samples = 0;
        Some(line)
    }
}

/// One drift log line.
#[derive(Debug, Clone, PartialEq)]
pub struct DriftLine {
    pub server: (f32, f32),
    pub client: Option<(f32, f32)>,
    pub cells: Option<f32>,
    pub max_cells: Option<f32>,
    pub samples: u32,
    pub state: String,
}

/// One client's side-channel session (registered when its ticket is redeemed).
struct SideSession {
    keys: SessionKeys,
    /// The client's TCP port (trace column, log lines; not an identity).
    port: u16,
    /// Replay windows of the UDP stream (reports, commands) and the upload stream.
    udp: ReplayWindow,
    upload: ReplayWindow,
    /// Counter of the server's debug frames.
    out: u64,
    /// Where the client's authenticated datagrams come from (debug frames go back there).
    peer: Option<SocketAddr>,
    latest: Option<ClientReport>,
    commands: Vec<String>,
    /// The client mods the hook reported (`Kind::ModReport`), once it did.
    mods: Option<ModReport>,
}

/// The side-channel sessions of the instance server: the latest position report, the debug
/// link's peer address and commands of each, keyed by session id (`SessionKeys::id`).
#[derive(Default)]
pub struct ClientReports {
    sessions: Mutex<HashMap<u64, SideSession>>,
    trace: OnceLock<Arc<MovementTrace>>,
    socket: OnceLock<Arc<UdpSocket>>,
    /// Datagrams dropped for failing authentication (for the log's rate limit).
    rejected: AtomicU64,
}

/// Movement trace (dev, `[dev] movement_trace`): one TSV line per client position
/// report, movement input and movement reply, so a play session can be analysed afterwards.
/// Columns: milliseconds since the trace was opened, client TCP port, kind, then fields.
pub struct MovementTrace {
    start: Instant,
    out: Mutex<BufWriter<File>>,
}

impl MovementTrace {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let mut out = BufWriter::new(file);
        writeln!(out, "# trace opened (ms, client port, kind, fields)")?;
        Ok(Self {
            start: Instant::now(),
            out: Mutex::new(out),
        })
    }

    pub fn line(&self, port: u16, kind: &str, fields: Arguments<'_>) {
        let ms = self.start.elapsed().as_secs_f64() * 1000.0;
        let mut out = self.out.lock().unwrap();
        let _ = writeln!(out, "{ms:.1}\t{port}\t{kind}\t{fields}");
    }

    pub fn flush(&self) {
        let _ = self.out.lock().unwrap().flush();
    }
}

/// Reports older than this are ignored (the client stopped sending, e.g. paused or gone).
const FRESH: Duration = Duration::from_millis(250);

impl ClientReports {
    /// Turns on the movement trace.
    pub fn set_trace(&self, trace: Arc<MovementTrace>) {
        let _ = self.trace.set(trace);
    }

    pub fn trace(&self) -> Option<&MovementTrace> {
        self.trace.get().map(|t| &**t)
    }

    /// Registers a session from its ticket keys (the key material the client got in
    /// `InstanceDetails`); returns the session id the client's datagrams carry.
    pub fn register(&self, material: &[u8; 64], port: u16) -> u64 {
        let keys = SessionKeys::derive(material);
        let id = keys.id;
        self.sessions.lock().unwrap().insert(
            id,
            SideSession {
                keys,
                port,
                udp: ReplayWindow::default(),
                upload: ReplayWindow::default(),
                out: 0,
                peer: None,
                latest: None,
                commands: Vec::new(),
                mods: None,
            },
        );
        id
    }

    pub fn forget(&self, id: u64) {
        self.sessions.lock().unwrap().remove(&id);
    }

    /// Whether `id` is a registered session.
    pub fn knows(&self, id: u64) -> bool {
        self.sessions.lock().unwrap().contains_key(&id)
    }

    /// The latest report of session `id`, if recent.
    pub fn fresh(&self, id: u64) -> Option<ClientReport> {
        self.sessions
            .lock()
            .unwrap()
            .get(&id)?
            .latest
            .filter(|r| r.at.elapsed() < FRESH)
    }

    /// Commands the client DLL sent for session `id` since the last call.
    pub fn take_commands(&self, id: u64) -> Vec<String> {
        self.sessions
            .lock()
            .unwrap()
            .get_mut(&id)
            .map(|s| std::mem::take(&mut s.commands))
            .unwrap_or_default()
    }

    /// Sends the debug state (`key\tvalue` lines) to the client DLL of session `id`, if it
    /// reports: a sealed `Kind::Debug` frame to the address its last authentic datagram came from.
    pub fn send_debug(&self, id: u64, text: &str) {
        let Some(socket) = self.socket.get() else {
            return;
        };
        let (frame, peer) = {
            let mut sessions = self.sessions.lock().unwrap();
            let Some(s) = sessions.get_mut(&id) else {
                return;
            };
            let Some(peer) = s.peer else {
                return;
            };
            s.out += 1;
            (sc::seal(&s.keys, Kind::Debug, s.out, text.as_bytes()), peer)
        };
        let _ = socket.try_send_to(&frame, peer);
    }

    /// Authenticates and applies one UDP datagram (`Report` or `Command`). The session is found
    /// by the id in the frame; the tag, the direction and the counter must all check out.
    pub fn handle_datagram(&self, data: &[u8], from: SocketAddr) -> Result<Kind, Reject> {
        let id = sc::peek_session(data).ok_or(Reject::Malformed)?;
        let keys = self
            .sessions
            .lock()
            .unwrap()
            .get(&id)
            .map(|s| s.keys.clone())
            .ok_or(Reject::WrongSession)?;
        let frame = sc::open(&keys, Dir::ClientToServer, data)?;
        if !matches!(frame.kind, Kind::Report | Kind::Command) {
            // Uploads travel over TCP; nothing else is valid here.
            return Err(Reject::UnknownKind(frame.kind as u8));
        }
        let mut sessions = self.sessions.lock().unwrap();
        let s = sessions.get_mut(&id).ok_or(Reject::WrongSession)?;
        if !s.udp.accept(frame.counter) {
            return Err(Reject::Replay);
        }
        s.peer = Some(from);
        match frame.kind {
            Kind::Report => {
                let p = ReportPayload::decode(frame.payload).ok_or(Reject::Malformed)?;
                let report = ClientReport::from_payload(&p);
                if let Some(t) = self.trace() {
                    t.line(
                        s.port,
                        "report",
                        format_args!(
                            "{}\t{:.3}\t{:.3}\t{:#06x}\t{:#06x}\t{:#x}",
                            report.frame,
                            report.x,
                            report.y,
                            report.action,
                            report.action2,
                            report.flags
                        ),
                    );
                }
                if s.latest.replace(report).is_none() {
                    info!(
                        port = s.port,
                        session = format_args!("{id:016x}"),
                        "client position reports started"
                    );
                }
            }
            _ => {
                let text = String::from_utf8_lossy(frame.payload).trim().to_owned();
                info!(port = s.port, command = %text, "dev command from the client DLL");
                s.commands.push(text);
            }
        }
        Ok(frame.kind)
    }

    /// Opens a frame of the upload stream (TCP): the session by its id, tag and direction, and
    /// the upload counter (which must rise). Returns the session id, the kind and the payload.
    pub fn open_upload(&self, data: &[u8]) -> Result<(u64, Kind, Vec<u8>), Reject> {
        let id = sc::peek_session(data).ok_or(Reject::Malformed)?;
        let keys = self
            .sessions
            .lock()
            .unwrap()
            .get(&id)
            .map(|s| s.keys.clone())
            .ok_or(Reject::WrongSession)?;
        let frame = sc::open(&keys, Dir::ClientToServer, data)?;
        if !matches!(frame.kind, Kind::Upload | Kind::UploadOffer | Kind::ModReport) {
            return Err(Reject::UnknownKind(frame.kind as u8));
        }
        let mut sessions = self.sessions.lock().unwrap();
        let s = sessions.get_mut(&id).ok_or(Reject::WrongSession)?;
        if !s.upload.accept(frame.counter) {
            return Err(Reject::Replay);
        }
        Ok((id, frame.kind, frame.payload.to_vec()))
    }

    /// Keeps the client's mod report for session `id`.
    pub fn store_mod_report(&self, id: u64, report: ModReport) {
        if let Some(s) = self.sessions.lock().unwrap().get_mut(&id) {
            s.mods = Some(report);
        }
    }

    /// The mod report of session `id`, if the hook has sent one.
    pub fn mod_report(&self, id: u64) -> Option<ModReport> {
        self.sessions.lock().unwrap().get(&id)?.mods.clone()
    }

    /// Waits up to `wait` for the session's mod report (the hook sends it right after its
    /// instance connection starts, from a background thread); `None` when it never comes.
    pub async fn wait_mod_report(&self, id: u64, wait: Duration) -> Option<ModReport> {
        let end = tokio::time::Instant::now() + wait;
        loop {
            if let Some(r) = self.mod_report(id) {
                return Some(r);
            }
            if tokio::time::Instant::now() >= end {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Receives side-channel datagrams on the instance address (UDP, same port as the TCP
    /// listener). Anything that is not an authentic frame of a registered session is dropped
    /// (the first rejection and every thousandth are logged).
    pub async fn listen(&self, addr: SocketAddr) -> anyhow::Result<()> {
        let socket = Arc::new(UdpSocket::bind(addr).await?);
        let _ = self.socket.set(socket.clone());
        info!(%addr, "side channel (UDP) listening");
        let mut buf = [0u8; 4096];
        loop {
            let (n, from) = match socket.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(e) => {
                    warn!("side channel: {e}");
                    continue;
                }
            };
            if let Err(reject) = self.handle_datagram(&buf[..n], from) {
                let total = self.rejected.fetch_add(1, Ordering::Relaxed) + 1;
                if total == 1 || total.is_multiple_of(1000) {
                    warn!(%from, ?reject, total, "side-channel datagram rejected");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> SocketAddr {
        "203.0.113.5:50000".parse().unwrap()
    }

    fn report_payload(frame: u32, x: f32) -> Vec<u8> {
        ReportPayload {
            flags: 0x20,
            frame,
            x,
            y: 300.25,
            action: 0xed4c,
            action2: 0,
            cursor: (1500, 300),
            window: (2560, 1440),
        }
        .encode()
    }

    #[test]
    fn authentic_reports_are_applied_per_session() {
        let reports = ClientReports::default();
        let (a, b) = ([1u8; 64], [2u8; 64]);
        let (ida, idb) = (reports.register(&a, 50000), reports.register(&b, 50000));
        // Same TCP port, different remote hosts: the sessions stay apart.
        let ka = SessionKeys::derive(&a);
        let f = sc::seal(&ka, Kind::Report, 1, &report_payload(7, 12.5));
        assert_eq!(reports.handle_datagram(&f, peer()), Ok(Kind::Report));
        let r = reports.fresh(ida).unwrap();
        assert_eq!(
            (r.frame, r.cell(), r.action, r.flags),
            (7, (12, 300), 0xed4c, 0x20)
        );
        assert_eq!(
            r.cursor,
            Some(Cursor {
                x: 1500,
                y: 300,
                width: 2560,
                height: 1440
            })
        );
        assert!(reports.fresh(idb).is_none());
        // A command from the same session queues for it only.
        let c = sc::seal(&ka, Kind::Command, 2, b"area G1_1 5\n");
        assert_eq!(reports.handle_datagram(&c, peer()), Ok(Kind::Command));
        assert_eq!(reports.take_commands(ida), vec!["area G1_1 5".to_owned()]);
        assert!(reports.take_commands(idb).is_empty());
        reports.forget(ida);
        assert!(reports.fresh(ida).is_none());
    }

    #[test]
    fn unauthenticated_datagrams_are_rejected() {
        let reports = ClientReports::default();
        let material = [9u8; 64];
        let id = reports.register(&material, 50000);
        let keys = SessionKeys::derive(&material);
        let good = sc::seal(&keys, Kind::Report, 1, &report_payload(1, 1.0));
        // Unknown session (a client whose ticket was never redeemed).
        let stranger = SessionKeys::derive(&[3u8; 64]);
        let f = sc::seal(&stranger, Kind::Report, 1, &report_payload(1, 1.0));
        assert_eq!(
            reports.handle_datagram(&f, peer()),
            Err(Reject::WrongSession)
        );
        // Right session, a flipped bit in the tag.
        let mut bad = good.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(reports.handle_datagram(&bad, peer()), Err(Reject::BadTag));
        // A modified payload (moves the player somewhere else) breaks the tag.
        let mut moved = good.clone();
        moved[HEADER_END] ^= 0xff;
        assert_eq!(reports.handle_datagram(&moved, peer()), Err(Reject::BadTag));
        // The old unauthenticated formats are gone.
        let mut old = b"PSY2".to_vec();
        old.extend([0u8; 28]);
        assert_eq!(
            reports.handle_datagram(&old, peer()),
            Err(Reject::Malformed)
        );
        let mut pdc = b"PDC1".to_vec();
        pdc.extend(50000u16.to_le_bytes());
        pdc.extend(b"teleport 1 1");
        assert_eq!(
            reports.handle_datagram(&pdc, peer()),
            Err(Reject::Malformed)
        );
        // Another version byte.
        let mut v2 = good.clone();
        v2[3] = 2;
        assert_eq!(
            reports.handle_datagram(&v2, peer()),
            Err(Reject::Version(2))
        );
        // A server frame reflected back at the server.
        let reflected = sc::seal(&keys, Kind::Debug, 1, b"x\ty");
        assert_eq!(
            reports.handle_datagram(&reflected, peer()),
            Err(Reject::WrongDirection)
        );
        // Nothing of the above got through; the real one still does.
        assert!(reports.fresh(id).is_none());
        assert_eq!(reports.handle_datagram(&good, peer()), Ok(Kind::Report));
    }

    const HEADER_END: usize = sc::HEADER_LEN;

    #[test]
    fn replayed_datagrams_are_rejected_and_reordering_is_tolerated() {
        let reports = ClientReports::default();
        let material = [5u8; 64];
        let id = reports.register(&material, 1);
        let keys = SessionKeys::derive(&material);
        let frame = |counter, x| {
            sc::seal(
                &keys,
                Kind::Report,
                counter,
                &report_payload(counter as u32, x),
            )
        };
        let (f1, f2, f3) = (frame(1, 1.0), frame(2, 2.0), frame(3, 3.0));
        assert!(reports.handle_datagram(&f1, peer()).is_ok());
        assert!(reports.handle_datagram(&f3, peer()).is_ok());
        // Reordered but new: accepted. Captured and replayed: not.
        assert!(reports.handle_datagram(&f2, peer()).is_ok());
        for f in [&f1, &f2, &f3] {
            assert_eq!(reports.handle_datagram(f, peer()), Err(Reject::Replay));
        }
        assert_eq!(
            reports.fresh(id).unwrap().frame,
            2,
            "the replay did not roll the state back"
        );
    }

    #[test]
    fn upload_frames_have_their_own_counter() {
        let reports = ClientReports::default();
        let material = [6u8; 64];
        let id = reports.register(&material, 1);
        let keys = SessionKeys::derive(&material);
        // Plenty of UDP traffic first: far beyond a 64-wide window.
        for c in 1..=500 {
            let f = sc::seal(&keys, Kind::Report, c, &report_payload(c as u32, 1.0));
            reports.handle_datagram(&f, peer()).unwrap();
        }
        let offer = sc::seal(&keys, Kind::UploadOffer, 1, &7u32.to_le_bytes());
        let (sid, kind, payload) = reports.open_upload(&offer).unwrap();
        assert_eq!(
            (sid, kind, payload.as_slice()),
            (id, Kind::UploadOffer, &7u32.to_le_bytes()[..])
        );
        assert_eq!(reports.open_upload(&offer), Err(Reject::Replay));
        let up = sc::seal(&keys, Kind::Upload, 2, b"data");
        assert!(reports.open_upload(&up).is_ok());
        // Uploads are not accepted over UDP, reports not over the upload stream.
        assert_eq!(
            reports.handle_datagram(&up, peer()),
            Err(Reject::UnknownKind(4))
        );
        let report = sc::seal(&keys, Kind::Report, 501, &report_payload(1, 1.0));
        assert_eq!(reports.open_upload(&report), Err(Reject::UnknownKind(1)));
    }

    #[test]
    fn drift_check_logs_once_per_interval_with_the_largest_distance() {
        let mut d = DriftCheck::new(Duration::ZERO);
        let report = |x: f32| ClientReport {
            frame: 1,
            x,
            y: 10.0,
            action: 0,
            action2: 0,
            flags: 0,
            cursor: None,
            at: Instant::now(),
        };
        let line = d
            .sample((13.0, 10.0), || "Idle".into(), Some(&report(10.0)))
            .unwrap();
        assert_eq!(line.cells, Some(3.0));
        assert_eq!(line.max_cells, Some(3.0));
        // The next interval starts over; without a report there is no distance.
        let line = d.sample((13.0, 10.0), || "Idle".into(), None).unwrap();
        assert_eq!((line.cells, line.max_cells, line.samples), (None, None, 0));
        // A long interval stays quiet.
        let mut d = DriftCheck::new(Duration::from_secs(3600));
        assert!(
            d.sample((0.0, 0.0), || "Idle".into(), Some(&report(0.0)))
                .is_none()
        );
    }
}
