//! Server metrics for the web admin's dashboard charts: players online, instances, the
//! simulation frame time of the instance actors, and the memory and CPU of the server process.
//!
//! - [`TickStats`] is the meter the instance actors write to (one `record` per simulation
//!   frame). The sampler reads and resets it, so each sample covers one interval.
//! - [`Metrics`] is a ring buffer of [`Sample`]s in memory (`[metrics] retention_hours`, one
//!   sample per `[metrics] interval_secs`). It is lost on a restart.
//! - [`spawn_sampler`] takes a sample at every interval. [`Metrics::query`] answers
//!   `GET /api/v1/admin/metrics`, reducing long ranges to a point budget ([`downsample`]).
//! - [`ProcessProbe`] reads the process's resident memory and CPU time from `/proc` on Linux;
//!   elsewhere they are unknown (reported as 0 and `None`).

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{AdminControl, InstanceDirectory};

/// Frame times of the instance actors since the last [`take`](TickStats::take).
#[derive(Debug, Default)]
pub struct TickStats {
    count: AtomicU64,
    sum_us: AtomicU64,
    max_us: AtomicU64,
}

/// What [`TickStats::take`] returns: one interval's frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickInterval {
    pub count: u32,
    pub avg_us: u32,
    pub max_us: u32,
}

impl TickStats {
    /// Records one frame (the time the actor spent advancing the world and encoding it).
    pub fn record(&self, took: Duration) {
        let us = u64::try_from(took.as_micros()).unwrap_or(u64::MAX);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_us.fetch_add(us, Ordering::Relaxed);
        self.max_us.fetch_max(us, Ordering::Relaxed);
    }

    /// The frames since the last call, and a fresh start.
    pub fn take(&self) -> TickInterval {
        let count = self.count.swap(0, Ordering::Relaxed);
        let sum = self.sum_us.swap(0, Ordering::Relaxed);
        let max = self.max_us.swap(0, Ordering::Relaxed);
        let clamp = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
        TickInterval {
            count: clamp(count),
            avg_us: clamp(sum.checked_div(count).unwrap_or(0)),
            max_us: clamp(max),
        }
    }
}

/// One point of the charts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sample {
    /// Unix time in milliseconds.
    pub t_ms: i64,
    /// Players in game.
    pub players: u32,
    /// Instances that exist (towns and hideouts included).
    pub instances: u32,
    /// Instances with at least one player.
    pub occupied: u32,
    /// Simulation frames in the interval, their mean and longest time (microseconds).
    pub tick_count: u32,
    pub tick_avg_us: u32,
    pub tick_max_us: u32,
    /// Resident memory of the server process in bytes; 0 where unknown.
    pub rss_bytes: u64,
    /// CPU use of the server process since the previous sample, in percent of one core (can
    /// exceed 100 with several busy threads); `None` for the first sample and where unknown.
    pub cpu_pct: Option<f32>,
}

/// Ring buffer of samples.
pub struct Metrics {
    interval_secs: u64,
    capacity: usize,
    ring: Mutex<VecDeque<Sample>>,
}

impl Metrics {
    /// Keeps `retention_secs` of samples taken every `interval_secs` (at least 2 samples).
    pub fn new(interval_secs: u64, retention_secs: u64) -> Self {
        let interval_secs = interval_secs.max(1);
        let capacity = usize::try_from(retention_secs / interval_secs)
            .unwrap_or(usize::MAX)
            .clamp(2, 1_000_000);
        Self {
            interval_secs,
            capacity,
            ring: Mutex::new(VecDeque::new()),
        }
    }

    pub fn interval_secs(&self) -> u64 {
        self.interval_secs
    }

    /// Seconds of history the buffer holds when full.
    pub fn retention_secs(&self) -> u64 {
        self.capacity as u64 * self.interval_secs
    }

    pub fn push(&self, sample: Sample) {
        let mut ring = self.ring.lock().unwrap();
        if ring.len() == self.capacity {
            ring.pop_front();
        }
        ring.push_back(sample);
    }

    pub fn len(&self) -> usize {
        self.ring.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The newest sample.
    pub fn latest(&self) -> Option<Sample> {
        self.ring.lock().unwrap().back().copied()
    }

    /// Samples with `t_ms > since_ms` (all with `None`), oldest first, reduced to at most
    /// `max_points` by [`downsample`].
    pub fn query(&self, since_ms: Option<i64>, max_points: usize) -> Vec<Sample> {
        let ring = self.ring.lock().unwrap();
        // Samples are pushed in time order: skip the old ones with a binary search.
        let start = since_ms.map_or(0, |s| ring.partition_point(|x| x.t_ms <= s));
        let picked: Vec<Sample> = ring.iter().skip(start).copied().collect();
        drop(ring);
        downsample(&picked, max_points)
    }
}

/// Reduces `samples` to at most `max_points` by merging neighbours: a bucket keeps the peak of
/// players, instances, memory and the longest frame, the frame-weighted mean frame time, the
/// mean CPU and the time of its last sample. `max_points` of 0 is treated as 1.
pub fn downsample(samples: &[Sample], max_points: usize) -> Vec<Sample> {
    let max_points = max_points.max(1);
    if samples.len() <= max_points {
        return samples.to_vec();
    }
    let bucket = samples.len().div_ceil(max_points);
    samples
        .chunks(bucket)
        .map(|chunk| {
            let frames: u64 = chunk.iter().map(|s| u64::from(s.tick_count)).sum();
            let weighted: u64 = chunk
                .iter()
                .map(|s| u64::from(s.tick_avg_us) * u64::from(s.tick_count))
                .sum();
            let cpus: Vec<f32> = chunk.iter().filter_map(|s| s.cpu_pct).collect();
            Sample {
                t_ms: chunk.last().map_or(0, |s| s.t_ms),
                players: chunk.iter().map(|s| s.players).max().unwrap_or(0),
                instances: chunk.iter().map(|s| s.instances).max().unwrap_or(0),
                occupied: chunk.iter().map(|s| s.occupied).max().unwrap_or(0),
                tick_count: u32::try_from(frames).unwrap_or(u32::MAX),
                tick_avg_us: u32::try_from(weighted.checked_div(frames).unwrap_or(0))
                    .unwrap_or(u32::MAX),
                tick_max_us: chunk.iter().map(|s| s.tick_max_us).max().unwrap_or(0),
                rss_bytes: chunk.iter().map(|s| s.rss_bytes).max().unwrap_or(0),
                cpu_pct: (!cpus.is_empty()).then(|| cpus.iter().sum::<f32>() / cpus.len() as f32),
            }
        })
        .collect()
}

/// Clock ticks per second of the `utime`/`stime` fields of `/proc/<pid>/stat` (`USER_HZ`; 100 on
/// every Linux platform in practice).
const CLOCK_TICKS_PER_SEC: f64 = 100.0;

/// `utime + stime` in clock ticks from the text of `/proc/<pid>/stat`.
pub fn parse_proc_stat(text: &str) -> Option<u64> {
    // The command name is in parentheses and may contain spaces and parentheses: the fields
    // after it start behind the last `)`. They begin at field 3 (state); utime and stime are
    // fields 14 and 15.
    let rest = text.get(text.rfind(')')? + 1..)?;
    let mut fields = rest.split_whitespace();
    let utime: u64 = fields.nth(11)?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some(utime + stime)
}

/// Resident set size in bytes from the text of `/proc/<pid>/status` (`VmRSS: 1234 kB`).
pub fn parse_proc_rss(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

/// Reads the process's own memory and CPU time.
#[derive(Debug, Default)]
pub struct ProcessProbe {
    last: Option<(i64, u64)>,
}

impl ProcessProbe {
    /// Resident memory (bytes, 0 if unknown) and CPU percent since the previous call (`None`
    /// for the first call or where `/proc` is not available). `now_ms` is the sample's time.
    pub fn read(&mut self, now_ms: i64) -> (u64, Option<f32>) {
        let rss = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|t| parse_proc_rss(&t))
            .unwrap_or(0);
        let ticks = std::fs::read_to_string("/proc/self/stat")
            .ok()
            .and_then(|t| parse_proc_stat(&t));
        (rss, self.cpu(now_ms, ticks))
    }

    /// CPU percent between the previous reading and this one.
    pub fn cpu(&mut self, now_ms: i64, ticks: Option<u64>) -> Option<f32> {
        let ticks = ticks?;
        let previous = self.last.replace((now_ms, ticks));
        let (t0, c0) = previous?;
        let wall = (now_ms - t0) as f64 / 1000.0;
        if wall <= 0.0 || ticks < c0 {
            return None;
        }
        Some(((ticks - c0) as f64 / CLOCK_TICKS_PER_SEC / wall * 100.0) as f32)
    }
}

/// Takes the samples. `now_ms` is the clock (unix milliseconds): the system clock in the
/// server, a controlled one in tests.
pub struct Sampler {
    pub metrics: Arc<Metrics>,
    pub ticks: Arc<TickStats>,
    pub admin: Arc<dyn AdminControl>,
    pub instances: Arc<dyn InstanceDirectory>,
    pub now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    probe: Mutex<ProcessProbe>,
}

impl Sampler {
    pub fn new(
        metrics: Arc<Metrics>,
        ticks: Arc<TickStats>,
        admin: Arc<dyn AdminControl>,
        instances: Arc<dyn InstanceDirectory>,
        now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self {
            metrics,
            ticks,
            admin,
            instances,
            now_ms,
            probe: Mutex::new(ProcessProbe::default()),
        }
    }

    /// Takes one sample and stores it.
    pub fn sample(&self) -> Sample {
        let t_ms = (self.now_ms)();
        let list = self.instances.list();
        let tick = self.ticks.take();
        let (rss_bytes, cpu_pct) = self.probe.lock().unwrap().read(t_ms);
        let sample = Sample {
            t_ms,
            players: u32::try_from(self.admin.online().len()).unwrap_or(u32::MAX),
            instances: u32::try_from(list.len()).unwrap_or(u32::MAX),
            occupied: u32::try_from(list.iter().filter(|i| !i.players.is_empty()).count())
                .unwrap_or(u32::MAX),
            tick_count: tick.count,
            tick_avg_us: tick.avg_us,
            tick_max_us: tick.max_us,
            rss_bytes,
            cpu_pct,
        };
        self.metrics.push(sample);
        sample
    }
}

/// Samples every `metrics.interval_secs()` until the runtime ends. Missed intervals (a stalled
/// runtime) are skipped, not caught up.
pub fn spawn_sampler(sampler: Arc<Sampler>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(sampler.metrics.interval_secs()));
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            every.tick().await;
            sampler.sample();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InstanceSpec, MemoryInstances, OnlinePlayer};

    fn sample(t_ms: i64, players: u32) -> Sample {
        Sample {
            t_ms,
            players,
            ..Sample::default()
        }
    }

    #[test]
    fn the_ring_drops_the_oldest_and_answers_since() {
        // 60 s of history at 10 s: 6 samples.
        let m = Metrics::new(10, 60);
        assert_eq!((m.interval_secs(), m.retention_secs()), (10, 60));
        assert!(m.is_empty() && m.latest().is_none());
        for i in 0..10 {
            m.push(sample(i * 10_000, i as u32));
        }
        assert_eq!(m.len(), 6);
        let all = m.query(None, 100);
        assert_eq!(
            all.iter().map(|s| s.players).collect::<Vec<_>>(),
            vec![4, 5, 6, 7, 8, 9]
        );
        assert_eq!(m.latest().unwrap().players, 9);
        // `since` is exclusive; older than the buffer returns all; newer returns nothing.
        assert_eq!(
            m.query(Some(70_000), 100)
                .iter()
                .map(|s| s.players)
                .collect::<Vec<_>>(),
            vec![8, 9]
        );
        assert_eq!(m.query(Some(0), 100).len(), 6);
        assert!(m.query(Some(90_000), 100).is_empty());
        // At least two samples are kept whatever the numbers say.
        assert_eq!(Metrics::new(60, 10).retention_secs(), 120);
    }

    #[test]
    fn a_day_at_five_seconds_is_a_bounded_ring() {
        let m = Metrics::new(5, 24 * 3600);
        for i in 0..20_000 {
            m.push(sample(i, 0));
        }
        assert_eq!(m.len(), 17_280);
        // The oldest were dropped: the first kept is 20000 - 17280.
        assert_eq!(m.query(None, usize::MAX)[0].t_ms, 2720);
        assert!((590..=600).contains(&m.query(None, 600).len()));
    }

    #[test]
    fn downsampling_keeps_peaks_and_weights_the_mean() {
        let mk = |t: i64, players, count, avg, max, rss, cpu| Sample {
            t_ms: t,
            players,
            instances: 1,
            occupied: 1,
            tick_count: count,
            tick_avg_us: avg,
            tick_max_us: max,
            rss_bytes: rss,
            cpu_pct: cpu,
        };
        let v = vec![
            mk(1, 1, 100, 1000, 5000, 10, Some(10.0)),
            mk(2, 3, 300, 2000, 9000, 30, Some(30.0)),
            mk(3, 2, 0, 0, 0, 20, None),
            mk(4, 0, 100, 4000, 1000, 5, Some(50.0)),
            mk(5, 7, 10, 500, 700, 8, None),
        ];
        assert_eq!(downsample(&v, 5), v, "within the budget: unchanged");
        assert_eq!(downsample(&[], 3), vec![]);
        let d = downsample(&v, 2);
        assert_eq!(d.len(), 2);
        // Bucket 1 = samples 1-3: peak players 3, weighted mean (100*1000 + 300*2000)/400 = 1750.
        assert_eq!(
            (
                d[0].t_ms,
                d[0].players,
                d[0].tick_count,
                d[0].tick_avg_us,
                d[0].tick_max_us,
                d[0].rss_bytes
            ),
            (3, 3, 400, 1750, 9000, 30)
        );
        assert_eq!(d[0].cpu_pct, Some(20.0));
        // Bucket 2 = samples 4-5.
        assert_eq!(
            (d[1].t_ms, d[1].players, d[1].tick_avg_us, d[1].cpu_pct),
            (5, 7, 3681, Some(50.0))
        );
        assert_eq!(downsample(&v, 0).len(), 1, "a budget of 0 is 1");
    }

    #[test]
    fn tick_stats_report_one_interval_at_a_time() {
        let t = TickStats::default();
        assert_eq!(t.take(), TickInterval::default());
        for ms in [2, 4, 12] {
            t.record(Duration::from_millis(ms));
        }
        assert_eq!(
            t.take(),
            TickInterval {
                count: 3,
                avg_us: 6000,
                max_us: 12_000
            }
        );
        assert_eq!(t.take(), TickInterval::default(), "the meter restarts");
    }

    #[test]
    fn proc_files_are_parsed() {
        // The command name holds spaces and parentheses; utime 150 and stime 25 are fields 14, 15.
        let stat = "1234 (my (odd) app) S 1 1234 1234 0 -1 4194560 100 0 0 0 150 25 0 0 20 0 8 0 100 1000 200 18446744073709551615";
        assert_eq!(parse_proc_stat(stat), Some(175));
        assert_eq!(parse_proc_stat("garbage"), None);
        assert_eq!(parse_proc_stat("1 (x) S 1"), None);
        assert_eq!(
            parse_proc_rss("Name:\tx\nVmPeak:\t9 kB\nVmRSS:\t   2048 kB\nThreads:\t3\n"),
            Some(2_097_152)
        );
        assert_eq!(parse_proc_rss("Name:\tx\n"), None);
        let mut p = ProcessProbe::default();
        assert_eq!(p.cpu(1000, Some(0)), None, "first reading");
        // 100 ticks = 1 s of CPU in 2 s of wall time = 50 %.
        assert_eq!(p.cpu(3000, Some(100)), Some(50.0));
        assert_eq!(p.cpu(3000, Some(100)), None, "no time passed");
        assert_eq!(p.cpu(4000, None), None, "unknown");
        // On Linux the real files are readable.
        #[cfg(target_os = "linux")]
        {
            let (rss, _) = ProcessProbe::default().read(0);
            assert!(rss > 0);
        }
    }

    struct Online(Vec<OnlinePlayer>);

    impl AdminControl for Online {
        fn online(&self) -> Vec<OnlinePlayer> {
            self.0.clone()
        }
        fn run_command(&self, _: &str, _: &str) -> crate::AdminOutcome {
            crate::AdminOutcome::ok("")
        }
        fn create_instance(
            &self,
            _: &str,
            _: &crate::NewInstance,
        ) -> Result<u32, crate::AdminOutcome> {
            Err(crate::AdminOutcome::err(""))
        }
        fn transfer_player(
            &self,
            _: &str,
            _: &str,
            _: crate::TransferTarget,
        ) -> crate::AdminOutcome {
            crate::AdminOutcome::err("")
        }
    }

    fn player(name: &str) -> OnlinePlayer {
        OnlinePlayer {
            character: name.into(),
            account: name.into(),
            account_id: 1,
            connected_at: 0,
            ip: None,
            muted: false,
            instance: Some(1),
            area: "G1_1".into(),
        }
    }

    /// The sampler under a paused clock: one sample per interval, with what the server holds
    /// at that moment and the frame meter reset after each.
    #[tokio::test(start_paused = true)]
    async fn the_sampler_takes_a_sample_per_interval() {
        let instances = Arc::new(MemoryInstances::default());
        let a = instances.create(
            InstanceSpec {
                area: "G1_1".into(),
                ..Default::default()
            },
            true,
        );
        instances.create(
            InstanceSpec {
                area: "G1_2".into(),
                ..Default::default()
            },
            false,
        );
        instances.join(a, "Bob");
        let ticks = Arc::new(TickStats::default());
        let metrics = Arc::new(Metrics::new(5, 3600));
        let start = tokio::time::Instant::now();
        let sampler = Arc::new(Sampler::new(
            metrics.clone(),
            ticks.clone(),
            Arc::new(Online(vec![player("Bob"), player("Amy")])),
            instances,
            Arc::new(move || 1_000_000 + start.elapsed().as_millis() as i64),
        ));
        let task = spawn_sampler(sampler);
        // The first tick fires at once.
        tokio::task::yield_now().await;
        assert_eq!(metrics.len(), 1);
        ticks.record(Duration::from_millis(3));
        tokio::time::sleep(Duration::from_secs(5)).await;
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(metrics.len(), 2);
        tokio::time::sleep(Duration::from_secs(10)).await;
        let all = metrics.query(None, 100);
        assert_eq!(all.len(), 4, "{all:?}");
        assert_eq!(
            (all[1].players, all[1].instances, all[1].occupied),
            (2, 2, 1)
        );
        assert_eq!(
            (all[0].tick_count, all[1].tick_count, all[2].tick_count),
            (0, 1, 0)
        );
        assert_eq!(all[1].tick_avg_us, 3000);
        // Timestamps follow the injected clock, 5 s apart.
        assert_eq!(all[1].t_ms - all[0].t_ms, 5000);
        assert!(all.windows(2).all(|w| w[0].t_ms < w[1].t_ms));
        task.abort();
    }
}
