//! Timing series for the graphs and snapshots: frame time (from the game's presents), packet
//! arrivals and sends (from the packet capture) and the client's frame backlog (simulation
//! frames waiting in the world, from the frame-drain hook).

use std::{collections::VecDeque, sync::Mutex, time::Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Series {
    /// Milliseconds between two presents.
    FrameMs,
    /// One sample per received message (value: size).
    PacketIn,
    /// One sample per sent message (value: size).
    PacketOut,
    /// Simulation frames queued in the world (lockstep playback backlog).
    Backlog,
}

pub const ALL: [Series; 4] = [
    Series::FrameMs,
    Series::PacketIn,
    Series::PacketOut,
    Series::Backlog,
];

/// Samples kept per series (several minutes of frames).
const KEEP: usize = 40_000;

#[derive(Clone, Copy)]
pub struct Sample {
    pub unix_ms: u64,
    pub value: f32,
}

static SERIES: Mutex<[VecDeque<Sample>; 4]> = Mutex::new([
    VecDeque::new(),
    VecDeque::new(),
    VecDeque::new(),
    VecDeque::new(),
]);

pub fn record(series: Series, value: f32) {
    let mut all = SERIES.lock().unwrap();
    let q = &mut all[series as usize];
    if q.len() >= KEEP {
        q.pop_front();
    }
    q.push_back(Sample {
        unix_ms: poe_hook_shared::unix_ms(),
        value,
    });
}

/// Called for every present of the game (not only overlay frames).
pub fn present() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let now = Instant::now();
    let mut last = LAST.lock().unwrap();
    if let Some(prev) = last.replace(now) {
        record(
            Series::FrameMs,
            now.duration_since(prev).as_secs_f32() * 1000.0,
        );
    }
}

/// Samples since `since` (Unix ms).
pub fn samples(series: Series, since: u64) -> Vec<Sample> {
    let all = SERIES.lock().unwrap();
    let q = &all[series as usize];
    let start = q.partition_point(|s| s.unix_ms < since);
    q.range(start..).copied().collect()
}

pub fn snapshot_csv(since: u64) -> String {
    let mut out = String::from("series,unix_ms,value\n");
    for s in ALL {
        for x in samples(s, since) {
            out.push_str(&format!("{s:?},{},{}\n", x.unix_ms, x.value));
        }
    }
    out
}
