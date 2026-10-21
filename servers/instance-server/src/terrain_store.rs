//! Terrain cache fed by client uploads (WP3.4).
//!
//! The server does not generate terrain for every area, so the client DLL exports the layout it
//! generated (`terrain_layout_<tile hash>.tsv`) and the cell grid (`terrain_grid_<tile
//! hash>.bin`) and the server spawns the player from them ([`crate::spawn`]). On one machine
//! the DLL writes them to `[spawn] layout_dir`; a remote client cannot, so it uploads them to
//! the instance TCP port instead. [`TerrainStore`] receives the upload (offer, then data),
//! validates it, and writes it to the same directory, so the first client that generates an
//! area supplies it for everybody and local files keep working as a fallback.
//!
//! Upload connection (the first four bytes `PSUP` tell it from a game connection, whose first
//! bytes are an opcode): `b"PSUP"`, `u32 length`, a sealed frame (`Kind::UploadOffer`, payload
//! `u32 tile hash`), answered by `b"PSUR"`, status (`REPLY_SEND`, `REPLY_HAVE`, ...); then, on
//! `REPLY_SEND`, the same with a `Kind::Upload` frame ([`poe_hook_shared::sidechannel::Upload`])
//! answered by `REPLY_OK`. Frames are authenticated with the session's keys
//! ([`crate::sync::ClientReports::open_upload`]); sizes are capped before anything is read
//! and again after decompression.
//!
//! Consistency: the tile hash an upload names must be the one its session's client reported
//! in `TerrainHashes` (authenticated by the encrypted game stream). An upload that arrives
//! before the hashes waits in memory and is written once they match (dropped otherwise), so a
//! session can only fill the cache slot of its own area. The layout must parse strictly and
//! the grid must be `PWG1` of exactly the layout's size in cells (23 per tile). An existing
//! cache entry is never replaced.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use poe_config::SpawnConfig;
use poe_hook_shared::sidechannel::{self as sc, Kind, Upload, UploadLimits};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::Notify,
};
use tracing::{info, warn};

use crate::{
    spawn::{GRID_PER_TILE, Grid, Layout},
    sync::ClientReports,
};

/// What the server answers to an offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferReply {
    /// Send the data.
    Send,
    /// Already cached, or the same tile hash is being sent.
    Have(&'static str),
    Reject(String),
}

#[derive(Default)]
struct SessionState {
    /// The tile hash the client reported in `TerrainHashes`.
    reported: Option<u32>,
    /// The hash of an announced upload (waiting for its data).
    offered: Option<u32>,
    /// A validated upload waiting for the session's hash report.
    pending: Option<Upload>,
}

pub struct TerrainStore {
    config: SpawnConfig,
    sessions: Mutex<HashMap<u64, SessionState>>,
    changed: Notify,
}

impl TerrainStore {
    pub fn new(config: &SpawnConfig) -> Self {
        Self {
            config: config.clone(),
            sessions: Mutex::new(HashMap::new()),
            changed: Notify::new(),
        }
    }

    fn dir(&self) -> Option<PathBuf> {
        (!self.config.layout_dir.is_empty()).then(|| PathBuf::from(&self.config.layout_dir))
    }

    pub fn layout_path(&self, tile_hash: u32) -> Option<PathBuf> {
        Some(
            self.dir()?
                .join("layouts")
                .join(format!("terrain_layout_{tile_hash}.tsv")),
        )
    }

    pub fn grid_path(&self, tile_hash: u32) -> Option<PathBuf> {
        Some(
            self.dir()?
                .join("grids")
                .join(format!("terrain_grid_{tile_hash}.bin")),
        )
    }

    /// The layout and grid of `tile_hash` are both cached (the layout is written last).
    pub fn cached(&self, tile_hash: u32) -> bool {
        matches!(
            (self.layout_path(tile_hash), self.grid_path(tile_hash)),
            (Some(l), Some(g)) if l.is_file() && g.is_file()
        )
    }

    pub fn limits(&self) -> UploadLimits {
        UploadLimits {
            layout: self.config.upload.max_layout_bytes,
            grid: self.config.upload.max_grid_bytes,
        }
    }

    /// The session's client reported its tile hash. Commits an upload that was waiting for it.
    pub fn reported(&self, session: u64, tile_hash: u32) {
        let pending = {
            let mut sessions = self.sessions.lock().unwrap();
            let s = sessions.entry(session).or_default();
            s.reported = Some(tile_hash);
            s.pending.take()
        };
        if let Some(up) = pending {
            if up.tile_hash == tile_hash {
                self.commit(session, &up);
            } else {
                warn!(
                    session = format_args!("{session:016x}"),
                    uploaded = up.tile_hash,
                    reported = tile_hash,
                    "dropped an upload for another tile hash than the session's"
                );
            }
        }
    }

    pub fn forget(&self, session: u64) {
        self.sessions.lock().unwrap().remove(&session);
    }

    /// Whether an upload of `tile_hash` is on its way (announced by this or another session, or
    /// held for the session's hash report): the spawn waits longer for the cache then.
    pub fn upload_expected(&self, session: u64, tile_hash: u32) -> bool {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(&session).is_some_and(|s| s.pending.is_some())
            || sessions.values().any(|s| s.offered == Some(tile_hash))
    }

    /// Forgets an announced upload whose connection ended without storing it.
    pub fn clear_offer(&self, session: u64) {
        if let Some(s) = self.sessions.lock().unwrap().get_mut(&session) {
            s.offered = None;
        }
    }

    /// Answers an offer of `tile_hash` from `session`.
    pub fn offer(&self, session: u64, tile_hash: u32) -> OfferReply {
        if !self.config.upload.enabled || self.dir().is_none() {
            return OfferReply::Reject("uploads are off on this server".into());
        }
        let mut sessions = self.sessions.lock().unwrap();
        let reported = sessions.entry(session).or_default().reported;
        if reported.is_some_and(|h| h != tile_hash) {
            return OfferReply::Reject("not the tile hash this session reported".into());
        }
        if self.cached(tile_hash) {
            return OfferReply::Have("cached");
        }
        // Another session is already sending this terrain: let it finish, spawn waits on the cache.
        if sessions
            .iter()
            .any(|(id, o)| *id != session && o.offered == Some(tile_hash))
        {
            return OfferReply::Have("being uploaded");
        }
        sessions.entry(session).or_default().offered = Some(tile_hash);
        OfferReply::Send
    }

    /// Takes the data of an upload: validates it and stores it, or holds it until the session's
    /// tile hash is known.
    pub fn accept(&self, session: u64, payload: &[u8]) -> Result<&'static str, String> {
        if !self.config.upload.enabled || self.dir().is_none() {
            return Err("uploads are off on this server".into());
        }
        let up = Upload::decode(payload, self.limits()).map_err(|e| e.to_string())?;
        validate(&up)?;
        let reported = {
            let mut sessions = self.sessions.lock().unwrap();
            let s = sessions.entry(session).or_default();
            if s.offered != Some(up.tile_hash) {
                return Err("no matching offer".into());
            }
            s.offered = None;
            match s.reported {
                Some(h) if h != up.tile_hash => {
                    return Err("not the tile hash this session reported".into());
                }
                Some(_) => true,
                None => {
                    s.pending = Some(up.clone());
                    false
                }
            }
        };
        if reported {
            self.commit(session, &up);
            Ok("stored")
        } else {
            Ok("held until the session reports its tile hash")
        }
    }

    fn commit(&self, session: u64, up: &Upload) {
        let (Some(layout), Some(grid)) =
            (self.layout_path(up.tile_hash), self.grid_path(up.tile_hash))
        else {
            return;
        };
        if layout.is_file() && grid.is_file() {
            return;
        }
        match write_atomic(&grid, &up.grid).and_then(|()| write_atomic(&layout, &up.layout)) {
            Ok(()) => info!(
                session = format_args!("{session:016x}"),
                tile_hash = up.tile_hash,
                layout_kib = up.layout.len() / 1024,
                grid_kib = up.grid.len() / 1024,
                "terrain upload cached"
            ),
            Err(e) => warn!(tile_hash = up.tile_hash, "caching terrain upload: {e}"),
        }
        self.changed.notify_waiters();
    }

    /// Waits for the layout of `tile_hash` (written locally by the DLL or uploaded): the time
    /// in `[spawn.upload] wait_ms`, or `wait_upload_secs` once the session announced an upload.
    /// Returns the layout text.
    pub async fn wait_layout(&self, session: u64, tile_hash: u32) -> Option<String> {
        let path = self.layout_path(tile_hash)?;
        let start = Instant::now();
        loop {
            // The layout is written last (grid first), so its presence means both are there.
            if let Ok(text) = std::fs::read_to_string(&path) {
                return Some(text);
            }
            let limit = if self.upload_expected(session, tile_hash) {
                Duration::from_secs(self.config.upload.wait_upload_secs)
            } else {
                Duration::from_millis(self.config.upload.wait_ms)
            };
            if start.elapsed() >= limit {
                return None;
            }
            let _ = tokio::time::timeout(Duration::from_millis(100), self.changed.notified()).await;
        }
    }
}

fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

/// Strict checks on an upload: the layout parses completely and the grid is the right size.
pub fn validate(up: &Upload) -> Result<(), String> {
    let text = std::str::from_utf8(&up.layout).map_err(|_| "layout is not UTF-8")?;
    let layout = Layout::parse(text).ok_or("layout has no size line")?;
    let (w, h) = (layout.width, layout.height);
    if !(1..=2048).contains(&w) || !(1..=2048).contains(&h) {
        return Err(format!("layout size {w}x{h} out of range"));
    }
    let lines = text.lines().count() - 1;
    if lines != layout.tiles.len() || lines as i64 > w as i64 * h as i64 {
        return Err("layout has malformed or too many tile lines".into());
    }
    if layout.tiles.is_empty() {
        return Err("layout has no tiles".into());
    }
    if layout
        .tiles
        .iter()
        .any(|t| t.x < 0 || t.y < 0 || t.x >= w || t.y >= h)
    {
        return Err("a tile lies outside the layout".into());
    }
    let grid = Grid::parse(&up.grid).ok_or("grid is not a valid PWG1 file")?;
    if (grid.cells_x, grid.cells_y) != (w * GRID_PER_TILE, h * GRID_PER_TILE) {
        return Err(format!(
            "grid is {}x{} cells, the layout needs {}x{}",
            grid.cells_x,
            grid.cells_y,
            w * GRID_PER_TILE,
            h * GRID_PER_TILE
        ));
    }
    Ok(())
}

/// Handles one upload connection (the caller saw the `PSUP` magic with `peek`).
pub async fn serve_upload(
    mut stream: TcpStream,
    reports: &ClientReports,
    store: &TerrainStore,
) -> std::io::Result<()> {
    // At most an offer and the data; the whole exchange has a deadline (slow links get the
    // configured `wait_upload_secs`).
    let deadline = Duration::from_secs(store.config.upload.wait_upload_secs.max(10));
    let mut session = None;
    let result = tokio::time::timeout(
        deadline,
        serve_frames(&mut stream, reports, store, &mut session),
    )
    .await
    .unwrap_or(Ok(()));
    // An announced upload that never arrived must not block the others.
    if let Some(id) = session {
        store.clear_offer(id);
    }
    result
}

async fn serve_frames(
    stream: &mut TcpStream,
    reports: &ClientReports,
    store: &TerrainStore,
    session_out: &mut Option<u64>,
) -> std::io::Result<()> {
    let max = store.config.upload.max_bytes;
    for _ in 0..2 {
        let mut head = [0u8; 8];
        if stream.read_exact(&mut head).await.is_err() {
            return Ok(());
        }
        let len = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
        if head[..4] != sc::UPLOAD_MAGIC || len > max || len < sc::HEADER_LEN + sc::TAG_LEN {
            return reply(stream, sc::REPLY_REJECTED, "bad upload header").await;
        }
        // Only sessions we know are worth reading a large body for.
        let mut frame = vec![0u8; len];
        stream.read_exact(&mut frame[..sc::HEADER_LEN]).await?;
        if !sc::peek_session_header(&frame).is_some_and(|id| reports.knows(id)) {
            warn!("upload from an unknown session");
            return reply(stream, sc::REPLY_AUTH, "unknown session").await;
        }
        stream.read_exact(&mut frame[sc::HEADER_LEN..]).await?;
        let (session, kind, payload) = match reports.open_upload(&frame) {
            Ok(v) => v,
            Err(r) => {
                warn!(?r, "upload frame rejected");
                return reply(stream, sc::REPLY_AUTH, &format!("{r:?}")).await;
            }
        };
        *session_out = Some(session);
        match kind {
            Kind::UploadOffer => {
                let Ok(bytes) = <[u8; 4]>::try_from(payload.as_slice()) else {
                    return reply(stream, sc::REPLY_REJECTED, "bad offer").await;
                };
                let hash = u32::from_le_bytes(bytes);
                match store.offer(session, hash) {
                    OfferReply::Send => {
                        info!(
                            session = format_args!("{session:016x}"),
                            tile_hash = hash,
                            "terrain upload wanted"
                        );
                        reply(stream, sc::REPLY_SEND, "send").await?;
                    }
                    OfferReply::Have(why) => {
                        return reply(stream, sc::REPLY_HAVE, why).await;
                    }
                    OfferReply::Reject(why) => {
                        warn!(why, "terrain offer refused");
                        return reply(stream, sc::REPLY_REJECTED, &why).await;
                    }
                }
            }
            _ => {
                return match store.accept(session, &payload) {
                    Ok(msg) => reply(stream, sc::REPLY_OK, msg).await,
                    Err(why) => {
                        warn!(why, "terrain upload rejected");
                        reply(stream, sc::REPLY_REJECTED, &why).await
                    }
                };
            }
        }
    }
    Ok(())
}

async fn reply(stream: &mut TcpStream, status: u8, text: &str) -> std::io::Result<()> {
    stream.write_all(&sc::encode_reply(status, text)).await?;
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::Path, sync::Arc};

    use poe_config::StartTile;
    use poe_hook_shared::sidechannel::{SessionKeys, seal};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    /// A scratch directory removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("poe-terrain-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn config(dir: &Path) -> SpawnConfig {
        let mut c = SpawnConfig {
            layout_dir: dir.to_string_lossy().into_owned(),
            filler: vec!["fill".into()],
            ..Default::default()
        };
        c.start_tiles.insert(
            "G1_1".into(),
            StartTile {
                tile: "Intro_Tree".into(),
                point: None,
            },
        );
        c.upload.wait_ms = 300;
        c.upload.wait_upload_secs = 10;
        c
    }

    /// A 2 x 1 tile layout and its 46 x 23 cell grid (everything walkable).
    fn upload(tile_hash: u32) -> Upload {
        let layout =
            "size\t2\t1\n0\t0\tMetadata/a/Intro_Tree_01.tdt\t0\n1\t0\tMetadata/a/ground.tdt\t0\n";
        let (cx, cy, bpr) = (46u32, 23u32, 23u32);
        let mut grid = b"PWG1".to_vec();
        for v in [cx, cy, bpr, 4] {
            grid.extend_from_slice(&v.to_le_bytes());
        }
        grid.extend(std::iter::repeat_n(0x55u8, (bpr * cy * 4) as usize));
        Upload {
            tile_hash,
            layout: layout.as_bytes().to_vec(),
            grid,
        }
    }

    #[test]
    fn validation_checks_layout_and_grid() {
        assert_eq!(validate(&upload(1)), Ok(()));
        let mut up = upload(1);
        up.layout = b"size\t3\t1\n0\t0\ta\t0\n".to_vec();
        assert!(validate(&up).unwrap_err().contains("grid is 46x23"));
        let mut up = upload(1);
        up.layout = b"nonsense".to_vec();
        assert!(validate(&up).is_err());
        let mut up = upload(1);
        up.layout.extend(b"1\t0\n");
        assert!(validate(&up).unwrap_err().contains("malformed"));
        let mut up = upload(1);
        up.layout = b"size\t2\t1\n5\t0\ta\t0\n".to_vec();
        assert!(validate(&up).unwrap_err().contains("outside"));
        let mut up = upload(1);
        up.layout = vec![0xff, 0xfe];
        assert!(validate(&up).unwrap_err().contains("UTF-8"));
        let mut up = upload(1);
        up.grid.truncate(30);
        assert!(validate(&up).unwrap_err().contains("PWG1"));
        let mut up = upload(1);
        up.layout = b"size\t0\t1\n".to_vec();
        assert!(validate(&up).is_err());
    }

    #[test]
    fn offer_accept_rules() {
        let dir = Scratch::new("rules");
        let store = TerrainStore::new(&config(dir.path()));
        let payload = upload(77).encode();
        // No offer, no upload.
        assert!(
            store
                .accept(1, &payload)
                .unwrap_err()
                .contains("no matching offer")
        );
        assert_eq!(store.offer(1, 77), OfferReply::Send);
        // The same terrain from a second session: the first one is on it.
        assert_eq!(store.offer(2, 77), OfferReply::Have("being uploaded"));
        assert!(store.upload_expected(2, 77));
        // Data for another hash than the offer is refused.
        assert!(store.accept(1, &upload(78).encode()).is_err());
        assert_eq!(store.offer(1, 77), OfferReply::Send);
        // The session's reported hash decides: a mismatching offer is refused up front.
        store.reported(1, 99);
        assert!(matches!(store.offer(1, 77), OfferReply::Reject(_)));
        store.reported(3, 77);
        assert_eq!(store.accept(3, &payload).unwrap_err(), "no matching offer");
        assert_eq!(store.offer(3, 77), OfferReply::Have("being uploaded"));
        store.clear_offer(1);
        assert_eq!(store.offer(3, 77), OfferReply::Send);
        assert_eq!(store.accept(3, &payload), Ok("stored"));
        assert!(store.cached(77));
        assert_eq!(store.offer(4, 77), OfferReply::Have("cached"));
        // A corrupt payload and an oversized one.
        assert_eq!(store.offer(5, 5), OfferReply::Send);
        assert!(store.accept(5, &payload[..20]).is_err());
        let mut small = config(dir.path());
        small.upload.max_grid_bytes = 100;
        let store = TerrainStore::new(&small);
        assert_eq!(store.offer(1, 78), OfferReply::Send);
        assert!(
            store
                .accept(1, &upload(78).encode())
                .unwrap_err()
                .contains("grid")
        );
    }

    #[test]
    fn upload_before_the_hash_report_waits_for_it() {
        let dir = Scratch::new("pending");
        let store = TerrainStore::new(&config(dir.path()));
        assert_eq!(store.offer(1, 5), OfferReply::Send);
        assert!(
            store
                .accept(1, &upload(5).encode())
                .unwrap()
                .contains("held")
        );
        assert!(!store.cached(5), "not cached before the hash is confirmed");
        store.reported(1, 5);
        assert!(store.cached(5));
        // A session that reports another hash gets nothing cached for the one it uploaded.
        assert_eq!(store.offer(2, 6), OfferReply::Send);
        store.accept(2, &upload(6).encode()).unwrap();
        store.reported(2, 7);
        assert!(!store.cached(6) && !store.cached(7));
    }

    #[test]
    fn disabled_uploads_and_an_empty_layout_dir_refuse() {
        let dir = Scratch::new("off");
        let mut c = config(dir.path());
        c.upload.enabled = false;
        let store = TerrainStore::new(&c);
        assert!(matches!(store.offer(1, 5), OfferReply::Reject(_)));
        assert!(store.accept(1, &upload(5).encode()).is_err());
        let store = TerrainStore::new(&SpawnConfig::default());
        assert!(matches!(store.offer(1, 5), OfferReply::Reject(_)));
        assert_eq!(store.layout_path(1), None);
    }

    /// The DLL's side of an upload connection (`poe-hook-core` `side.rs`), for the tests.
    async fn client_exchange(addr: SocketAddr, frames: &[Vec<u8>]) -> Vec<(u8, String)> {
        let mut tcp = TcpStream::connect(addr).await.unwrap();
        let mut out = Vec::new();
        for f in frames {
            let mut msg = sc::UPLOAD_MAGIC.to_vec();
            msg.extend((f.len() as u32).to_le_bytes());
            msg.extend(f);
            if tcp.write_all(&msg).await.is_err() {
                break;
            }
            let mut head = [0u8; 7];
            if tcp.read_exact(&mut head).await.is_err() {
                break;
            }
            assert_eq!(head[..4], sc::UPLOAD_REPLY_MAGIC);
            let mut text = vec![0u8; u16::from_le_bytes([head[5], head[6]]) as usize];
            tcp.read_exact(&mut text).await.unwrap();
            out.push((head[4], String::from_utf8(text).unwrap()));
            if head[4] != sc::REPLY_SEND {
                break;
            }
        }
        out
    }

    struct Rig {
        addr: SocketAddr,
        reports: Arc<ClientReports>,
        store: Arc<TerrainStore>,
        cfg: SpawnConfig,
    }

    /// An instance-port stand-in: accepts connections, hands upload ones to `serve_upload`.
    async fn rig(dir: &Path) -> Rig {
        let cfg = config(dir);
        let reports = Arc::new(ClientReports::default());
        let store = Arc::new(TerrainStore::new(&cfg));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (r, s) = (reports.clone(), store.clone());
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, s) = (r.clone(), s.clone());
                tokio::spawn(async move {
                    if crate::is_upload(&stream).await {
                        let _ = serve_upload(stream, &r, &s).await;
                    }
                });
            }
        });
        Rig {
            addr,
            reports,
            store,
            cfg,
        }
    }

    #[tokio::test]
    async fn an_upload_over_tcp_feeds_the_spawn() {
        let dir = Scratch::new("tcp");
        let rig = rig(dir.path()).await;
        let material = [42u8; 64];
        let sid = rig.reports.register(&material, 50000);
        let keys = SessionKeys::derive(&material);
        let hash = 3958003549u32;
        // The session waits for its spawn with an empty cache (no shared disk) ...
        let (store, cfg) = (rig.store.clone(), rig.cfg.clone());
        let spawn = tokio::spawn(async move {
            crate::spawn::from_client_layout(&store, &cfg, "G1_1", hash, sid).await
        });
        // ... the client reports its hash on the game stream and uploads on the side channel.
        rig.store.reported(sid, hash);
        let offer = seal(&keys, Kind::UploadOffer, 1, &hash.to_le_bytes());
        let data = seal(&keys, Kind::Upload, 2, &upload(hash).encode());
        let replies = client_exchange(rig.addr, &[offer, data]).await;
        assert_eq!(
            replies,
            vec![
                (sc::REPLY_SEND, "send".to_owned()),
                (sc::REPLY_OK, "stored".to_owned())
            ]
        );
        // Spawn picked it up: the centre of the start tile (0, 0), walkable on the uploaded grid.
        assert_eq!(spawn.await.unwrap(), Some((11, 11)));
        assert!(rig.store.cached(hash));
        assert!(
            dir.path()
                .join("layouts")
                .join(format!("terrain_layout_{hash}.tsv"))
                .is_file()
        );
        assert!(
            dir.path()
                .join("grids")
                .join(format!("terrain_grid_{hash}.bin"))
                .is_file()
        );
        // The second client of the same area is told to skip the upload.
        let material2 = [43u8; 64];
        rig.reports.register(&material2, 50001);
        let k2 = SessionKeys::derive(&material2);
        let offer = seal(&k2, Kind::UploadOffer, 1, &hash.to_le_bytes());
        assert_eq!(
            client_exchange(rig.addr, &[offer]).await,
            vec![(sc::REPLY_HAVE, "cached".to_owned())]
        );
    }

    #[tokio::test]
    async fn bad_uploads_get_nothing_cached() {
        let dir = Scratch::new("bad");
        let rig = rig(dir.path()).await;
        let material = [7u8; 64];
        rig.reports.register(&material, 1);
        let keys = SessionKeys::derive(&material);
        let hash = 12345u32;
        // An unknown session (never redeemed a ticket).
        let stranger = SessionKeys::derive(&[8u8; 64]);
        let f = seal(&stranger, Kind::UploadOffer, 1, &hash.to_le_bytes());
        assert_eq!(client_exchange(rig.addr, &[f]).await[0].0, sc::REPLY_AUTH);
        // A forged tag.
        let mut f = seal(&keys, Kind::UploadOffer, 1, &hash.to_le_bytes());
        *f.last_mut().unwrap() ^= 1;
        assert_eq!(client_exchange(rig.addr, &[f]).await[0].0, sc::REPLY_AUTH);
        // A replayed offer.
        let f = seal(&keys, Kind::UploadOffer, 5, &hash.to_le_bytes());
        assert_eq!(
            client_exchange(rig.addr, &[f.clone()]).await[0].0,
            sc::REPLY_SEND
        );
        assert_eq!(client_exchange(rig.addr, &[f]).await[0].0, sc::REPLY_AUTH);
        // An offer, then data that fails validation.
        let mut bad = upload(hash);
        bad.grid.truncate(40);
        let offer = seal(&keys, Kind::UploadOffer, 6, &hash.to_le_bytes());
        let data = seal(&keys, Kind::Upload, 7, &bad.encode());
        let replies = client_exchange(rig.addr, &[offer, data]).await;
        assert_eq!(replies[1].0, sc::REPLY_REJECTED, "{replies:?}");
        assert!(!rig.store.cached(hash));
        // A header announcing more than max_bytes is refused before the body is read.
        let mut tcp = TcpStream::connect(rig.addr).await.unwrap();
        let mut msg = sc::UPLOAD_MAGIC.to_vec();
        msg.extend(u32::MAX.to_le_bytes());
        tcp.write_all(&msg).await.unwrap();
        let mut head = [0u8; 7];
        tcp.read_exact(&mut head).await.unwrap();
        assert_eq!(head[4], sc::REPLY_REJECTED);
        assert!(!rig.store.cached(hash));
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "nothing written"
        );
    }

    #[tokio::test]
    async fn game_connections_are_not_uploads() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let mut s = TcpStream::connect(addr).await.unwrap();
            // A Hello: an opcode, then the ticket.
            s.write_all(&[0x01, 0x00, 0xde, 0xad, 0xbe, 0xef])
                .await
                .unwrap();
            s
        });
        let (stream, _) = listener.accept().await.unwrap();
        assert!(!crate::is_upload(&stream).await);
        drop(client.await.unwrap());
    }
}
