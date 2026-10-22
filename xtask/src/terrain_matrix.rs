//! `cargo xtask terrain-matrix`: the terrain verification matrix, area x topology x orientation
//! -> `untested` / `fail` / `match`, written to `docs/status/terrain-matrix.tsv`.
//!
//! The scope (which areas, topologies and orientations exist) is read from the game data:
//! every `WorldAreas` row, its `Topologies` rows (the default league mode, as
//! `poe_world::area::topology_entries` selects them) and each topology's orientation mask. The
//! results are read from the verification outputs in `dumps/<version>/terrain/vectors/` (all
//! optional; a missing file means nothing is verified from that source):
//!
//! | File | Written by | Gives |
//! |---|---|---|
//! | `area-vectors-results.txt` | `area_vectors` example (stdout) | a matching seed for the topology:orientation that reproduced a client tile hash (a vector that does not match gives no combination, so it is no fail) |
//! | `forced-results.tsv` | `forced_verify` example (forced-layout sweeps and the client's batch, docs/plans/terrain-all-areas.md) | `match` or `fail` for one seed at one forced topology:orientation |
//! | `probe-results.tsv` | `area_probe` example | whether the generator runs at all (no hash check), per topology, orientation and seed |
//!
//! A combination keeps the seeds it was tested with: `seeds` is `matched/tested` (a seed counts
//! once, a match from any source beats a fail), the status is `match` when every tested seed
//! matched, `partial` when some did, `fail` when none did and `untested` without any seed.
//! `old` counts the matched seeds that only an older client build's vector vouches for.
//!
//! The tracker (`cargo xtask tracker`) renders the committed TSV into the README
//! ([`summary_markdown`]), so it needs no game files.

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

/// The matrix, relative to the repo root.
pub const MATRIX_PATH: &str = "docs/status/terrain-matrix.tsv";
/// Verification outputs of the newest supported build (`modules/protocol/versions/<version>.toml`):
/// `dumps/<version>/terrain/vectors` below the repo root.
pub fn vectors_dir(root: &Path) -> Result<PathBuf> {
    let v = poe_config::VersionFile::latest(root).context("reading modules/protocol/versions/")?;
    Ok(v.dumps_dir(root).join("terrain/vectors"))
}
const VECTOR_RESULTS: &str = "area-vectors-results.txt";
const FORCED_RESULTS: &str = "forced-results.tsv";
const PROBE_RESULTS: &str = "probe-results.tsv";
const HEADER: &str = "area\tact\tkind\ttopology\torientation\tgraph\tstatus\tseeds\told\tprobe\tarea_vectors\tevidence";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A town or hideout (a one-node `.tgr` covering the map).
    Town,
    Tgr,
    Dgr,
    /// No `Topologies` rows: nothing is generated from a graph.
    Fixed,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Town => "town",
            Kind::Tgr => "tgr",
            Kind::Dgr => "dgr",
            Kind::Fixed => "fixed",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "town" => Kind::Town,
            "tgr" => Kind::Tgr,
            "dgr" => Kind::Dgr,
            "fixed" => Kind::Fixed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Untested,
    Fail,
    /// Some tested seeds matched, some did not.
    Partial,
    Match,
    /// Fixed areas have no combinations.
    NotApplicable,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Untested => "untested",
            Status::Fail => "fail",
            Status::Partial => "partial",
            Status::Match => "match",
            Status::NotApplicable => "n/a",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "untested" => Status::Untested,
            "fail" => Status::Fail,
            "partial" => Status::Partial,
            "match" => Status::Match,
            "n/a" => Status::NotApplicable,
            _ => return None,
        })
    }
}

/// One area as the game data describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaScope {
    pub id: String,
    pub act: i64,
    pub kind: Kind,
    /// `(graph path, orientation mask)` per topology, in the order the client indexes them
    /// (`map_force_topology` is the 1-based index into this list; the client indexes the raw
    /// `WorldAreas` list of all league modes, but the mode 1 entries are a prefix of it in every
    /// area, so the indices agree, docs/re/terrain/batch.md "Open issues").
    pub topologies: Vec<(String, u8)>,
}

/// The orientations a topology allows: one per set bit of the mask (`0xff` is all eight). A
/// mask of 0 (no list in the data) is laid out unturned.
pub fn orientations(mask: u8) -> Vec<u8> {
    if mask == 0 {
        return vec![0];
    }
    (0..8).filter(|o| mask & (1 << o) != 0).collect()
}

/// The generator type of an area from its flags and graph files.
pub fn kind_of(is_town: bool, is_hideout: bool, topologies: &[(String, u8)]) -> Kind {
    let Some((first, _)) = topologies.first() else { return Kind::Fixed };
    if is_town || is_hideout {
        return Kind::Town;
    }
    if first.to_ascii_lowercase().ends_with(".tgr") { Kind::Tgr } else { Kind::Dgr }
}

/// Every `WorldAreas` row in table order (rows without an id are skipped).
pub fn inventory(game_dir: &Path, schema: &Path) -> Result<Vec<AreaScope>> {
    let gd = poe_data::GameData::open(game_dir, schema).context("opening the game data")?;
    let areas = gd.table("WorldAreas").context("reading WorldAreas")?;
    let mut out = Vec::new();
    for row in areas.rows() {
        let id = row.str("Id")?;
        if id.is_empty() {
            continue;
        }
        let topologies = poe_world::area::topology_entries(&gd, &id, 1).unwrap_or_default();
        out.push(AreaScope {
            act: row.int("Act")?,
            kind: kind_of(row.bool("IsTown")?, row.bool("IsHideout")?, &topologies),
            id,
            topologies,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Verification inputs

/// A line of the `area_vectors` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorResult {
    pub area: String,
    pub seed: u32,
    pub matched: bool,
    /// The topology (1-based) and orientation that reproduced the hash, when the line says so.
    pub combo: Option<(u32, u8)>,
    pub forced: bool,
    /// The client build of the vector (the trailing `build=N` field), when the line has one.
    pub build: Option<String>,
}

/// `T:O` of `(natural 5:0)` / `(forced 2:7)`.
fn combo_of(s: &str) -> Option<(u32, u8)> {
    let (t, o) = s.split_once(':')?;
    Some((t.trim().parse().ok()?, o.trim().parse().ok()?))
}

/// Parses `area_vectors` stdout: `area  seed  match [(natural|forced T:O)]` or
/// `area  seed  differ|error|panic|skipped ...`, each optionally ending in `build=N`. Other lines
/// (the per-area summary) are skipped; `skipped` ones carry no result.
pub fn parse_vector_results(text: &str) -> Vec<VectorResult> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with("---") {
            break;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 3 {
            continue;
        }
        let Ok(seed) = f[1].parse() else { continue };
        let (matched, combo, forced) = if let Some(rest) = f[2].strip_prefix("match") {
            let rest = rest.trim().trim_start_matches('(').trim_end_matches(')');
            let (kind, c) = rest.split_once(' ').unwrap_or((rest, ""));
            (true, combo_of(c), kind == "forced")
        } else if matches!(f[2], "differ" | "error" | "panic") {
            (false, None, false)
        } else {
            continue;
        };
        let build = f.iter().skip(3).find_map(|x| x.strip_prefix("build=")).map(str::to_string);
        out.push(VectorResult { area: f[0].to_string(), seed, matched, combo, forced, build });
    }
    out
}

/// A forced-layout verification: the client was told topology and orientation, the generator
/// ran the same choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForcedResult {
    pub area: String,
    pub seed: u32,
    pub topology: u32,
    pub orientation: u8,
    pub matched: bool,
    pub detail: String,
}

/// Parses `forced-results.tsv`: `area  seed  topology  orientation  match|fail  [detail]`;
/// `#` lines and blanks are skipped.
pub fn parse_forced_results(text: &str) -> Vec<ForcedResult> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 5 {
            continue;
        }
        let (Ok(seed), Ok(topology), Ok(orientation)) = (f[1].parse(), f[2].parse(), f[3].parse()) else { continue };
        let matched = match f[4] {
            "match" => true,
            "fail" => false,
            _ => continue,
        };
        out.push(ForcedResult {
            area: f[0].to_string(),
            seed,
            topology,
            orientation,
            matched,
            detail: f.get(5).unwrap_or(&"").to_string(),
        });
    }
    out
}

/// One `area_probe` attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub area: String,
    pub topology: u32,
    pub orientation: u8,
    /// `ok`, `error`, `panic` or `timeout`.
    pub status: String,
    pub detail: String,
    /// The probed seed; `None` in files from before the seed column.
    pub seed: Option<u32>,
}

/// Parses `area_probe` output: `area  topology  orientation  graph  seconds  status  detail
/// [seed]`.
pub fn parse_probe(text: &str) -> Vec<ProbeResult> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 6 {
            continue;
        }
        let (Ok(topology), Ok(orientation)) = (f[1].parse(), f[2].parse()) else { continue };
        out.push(ProbeResult {
            area: f[0].to_string(),
            topology,
            orientation,
            status: f[5].to_string(),
            detail: f.get(6).unwrap_or(&"").to_string(),
            seed: f.get(7).and_then(|s| s.parse().ok()),
        });
    }
    out
}

/// What the verification outputs say, gathered.
#[derive(Debug, Default)]
pub struct Results {
    pub vectors: Vec<VectorResult>,
    pub forced: Vec<ForcedResult>,
    pub probe: Vec<ProbeResult>,
    /// The newest supported client build (`build_revision`): vectors of other builds are "older".
    pub current_build: Option<String>,
}

impl Results {
    /// Reads the three files under `dir`; a missing one is empty.
    pub fn load(dir: &Path) -> Result<Self> {
        let read = |name: &str| -> Result<String> {
            let p = dir.join(name);
            match std::fs::read_to_string(&p) {
                Ok(t) => Ok(t),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
                Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
            }
        };
        Ok(Self {
            vectors: parse_vector_results(&read(VECTOR_RESULTS)?),
            forced: parse_forced_results(&read(FORCED_RESULTS)?),
            probe: parse_probe(&read(PROBE_RESULTS)?),
            current_build: None,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// The matrix

/// One row: an area's topology at one allowed orientation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub area: String,
    pub act: i64,
    pub kind: Kind,
    /// 1-based; 0 for a fixed area.
    pub topology: u32,
    pub orientation: u8,
    pub graph: String,
    pub status: Status,
    /// Seeds that matched and seeds tested for this combination.
    pub seeds_matched: u32,
    pub seeds_tested: u32,
    /// Matched seeds that only an older client build's vector vouches for.
    pub older: u32,
    /// `ok`/`error`/`panic`/`timeout` plus the detail (the error's first words), or `-`. With
    /// several probed seeds: `ok 3/3` or `error (2/3 ok): detail`.
    pub probe: String,
    /// Client vectors that matched / were checked for the area (`2/3`; `-` when none).
    pub area_vectors: String,
    pub evidence: String,
}

fn short_graph(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// A probe detail cut to a TSV-friendly length with its numbers kept.
fn short_detail(d: &str) -> String {
    let d: String = d.chars().map(|c| if c == '\t' || c == '\n' { ' ' } else { c }).collect();
    let mut d: String = d.chars().take(60).collect();
    d.truncate(d.trim_end().len());
    d
}

/// What is known of one seed at one combination.
#[derive(Debug, Clone)]
struct SeedResult {
    matched: bool,
    /// Every match came from an older build's vector (only meaningful when `matched`).
    older: bool,
    text: String,
}

/// Records `new` for its seed: a match beats a fail; matches from several sources are "older"
/// only when all of them are.
fn merge_seed(map: &mut BTreeMap<u32, SeedResult>, seed: u32, new: SeedResult) {
    match map.get_mut(&seed) {
        None => {
            map.insert(seed, new);
        }
        Some(old) => match (old.matched, new.matched) {
            (true, true) => old.older &= new.older,
            (false, true) => *old = new,
            _ => {}
        },
    }
}

/// The probe cell of one combination: its attempts (one per probed seed) as one string.
fn probe_cell(attempts: &[&ProbeResult]) -> String {
    let Some(first) = attempts.first() else { return "-".into() };
    let n = attempts.len();
    let ok = attempts.iter().filter(|p| p.status == "ok").count();
    let rank = |s: &str| match s {
        "ok" => 0,
        "error" => 1,
        "timeout" => 2,
        _ => 3,
    };
    if ok == n {
        return if n == 1 { "ok".into() } else { format!("ok {n}/{n}") };
    }
    let worst = attempts.iter().max_by_key(|p| rank(&p.status)).unwrap_or(first);
    let bad = attempts.iter().find(|p| p.status == worst.status).unwrap_or(first);
    let head = if n == 1 { worst.status.clone() } else { format!("{} ({ok}/{n} ok)", worst.status) };
    if bad.detail.is_empty() { head } else { format!("{head}: {}", short_detail(&bad.detail)) }
}

/// The evidence text: up to three matched seeds and two failed ones, the rest counted.
fn evidence_text(seeds: &BTreeMap<u32, SeedResult>) -> String {
    let part = |matched: bool, limit: usize, label: &str| -> Option<String> {
        let items: Vec<(&u32, &SeedResult)> = seeds.iter().filter(|(_, r)| r.matched == matched).collect();
        if items.is_empty() {
            return None;
        }
        let mut s: Vec<String> = items.iter().take(limit).map(|(k, r)| format!("seed {k} ({})", r.text)).collect();
        if items.len() > limit {
            s.push(format!("+{} more", items.len() - limit));
        }
        Some(format!("{label}{}", s.join("; ")))
    };
    [part(true, 3, ""), part(false, 2, "failed: ")].into_iter().flatten().collect::<Vec<_>>().join(" | ")
}

/// Combines the scope with the results. Every seed a combination was tested with counts once: a
/// match from a client vector or a forced trace beats a fail on the same seed. The status is
/// `match` when all tested seeds matched, `partial` when some did, `fail` when none did, else
/// `untested`. A vector that did not match says nothing about a combination (its layout is
/// unknown), so only forced results give a fail.
pub fn build(scope: &[AreaScope], r: &Results) -> Vec<Row> {
    type Key<'a> = (&'a str, u32, u8);
    let mut seeds: HashMap<Key, BTreeMap<u32, SeedResult>> = HashMap::new();
    for v in &r.vectors {
        if let (true, Some((t, o))) = (v.matched, v.combo) {
            let how = if v.forced { "forced" } else { "natural" };
            let older = matches!((&v.build, &r.current_build), (Some(b), Some(c)) if b != c);
            let text = match (&v.build, older) {
                (Some(b), true) => format!("{how}, build {b}"),
                _ => how.to_string(),
            };
            merge_seed(seeds.entry((v.area.as_str(), t, o)).or_default(), v.seed, SeedResult { matched: true, older, text });
        }
    }
    for f in &r.forced {
        let text = if f.matched || f.detail.is_empty() { "forced".to_string() } else { format!("forced: {}", f.detail) };
        merge_seed(seeds.entry((f.area.as_str(), f.topology, f.orientation)).or_default(), f.seed, SeedResult { matched: f.matched, older: false, text });
    }
    let mut probes: HashMap<Key, Vec<&ProbeResult>> = HashMap::new();
    for p in &r.probe {
        probes.entry((p.area.as_str(), p.topology, p.orientation)).or_default().push(p);
    }
    // Area-level vector counts: matched / checked.
    let mut vec_counts: HashMap<&str, (u32, u32)> = HashMap::new();
    for v in &r.vectors {
        let e = vec_counts.entry(v.area.as_str()).or_default();
        e.0 += v.matched as u32;
        e.1 += 1;
    }
    let mut rows = Vec::new();
    for a in scope {
        let area_vectors = vec_counts.get(a.id.as_str()).map_or("-".to_string(), |(m, n)| format!("{m}/{n}"));
        if a.topologies.is_empty() {
            rows.push(Row {
                area: a.id.clone(),
                act: a.act,
                kind: a.kind,
                topology: 0,
                orientation: 0,
                graph: "-".into(),
                status: Status::NotApplicable,
                seeds_matched: 0,
                seeds_tested: 0,
                older: 0,
                probe: "-".into(),
                area_vectors,
                evidence: "no Topologies rows".into(),
            });
            continue;
        }
        for (i, (path, mask)) in a.topologies.iter().enumerate() {
            let t = i as u32 + 1;
            for o in orientations(*mask) {
                let key = (a.id.as_str(), t, o);
                let known = seeds.get(&key);
                let tested = known.map_or(0, |m| m.len() as u32);
                let matched = known.map_or(0, |m| m.values().filter(|s| s.matched).count() as u32);
                let older = known.map_or(0, |m| m.values().filter(|s| s.matched && s.older).count() as u32);
                let status = match (tested, matched) {
                    (0, _) => Status::Untested,
                    (n, m) if m == n => Status::Match,
                    (_, 0) => Status::Fail,
                    _ => Status::Partial,
                };
                rows.push(Row {
                    area: a.id.clone(),
                    act: a.act,
                    kind: a.kind,
                    topology: t,
                    orientation: o,
                    graph: short_graph(path),
                    status,
                    seeds_matched: matched,
                    seeds_tested: tested,
                    older,
                    probe: probes.get(&key).map_or_else(|| "-".into(), |p| probe_cell(p)),
                    area_vectors: area_vectors.clone(),
                    evidence: known.map(evidence_text).unwrap_or_default(),
                });
            }
        }
    }
    rows
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n'], " ")
}

/// The `seeds` column: `matched/tested`, `-` when untested.
fn seeds_cell(matched: u32, tested: u32) -> String {
    if tested == 0 { "-".into() } else { format!("{matched}/{tested}") }
}

/// The TSV text (header, one line per row, final newline).
pub fn to_tsv(rows: &[Row]) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for r in rows {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.area,
            r.act,
            r.kind.name(),
            r.topology,
            r.orientation,
            clean(&r.graph),
            r.status.name(),
            seeds_cell(r.seeds_matched, r.seeds_tested),
            r.older,
            clean(&r.probe),
            r.area_vectors,
            clean(&r.evidence)
        );
    }
    out
}

/// Parses [`to_tsv`]'s output.
pub fn parse_tsv(text: &str) -> Result<Vec<Row>> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        bail!("not a terrain matrix (header differs from `{HEADER}`)");
    }
    let mut rows = Vec::new();
    for (n, line) in lines.enumerate() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 12 {
            bail!("line {}: {} columns, expected 12", n + 2, f.len());
        }
        let bad = |what: &str| anyhow::anyhow!("line {}: bad {what}", n + 2);
        let (seeds_matched, seeds_tested) = match f[7] {
            "-" => (0, 0),
            s => {
                let (m, t) = s.split_once('/').ok_or_else(|| bad("seeds"))?;
                (m.parse().map_err(|_| bad("seeds"))?, t.parse().map_err(|_| bad("seeds"))?)
            }
        };
        rows.push(Row {
            area: f[0].to_string(),
            act: f[1].parse().map_err(|_| bad("act"))?,
            kind: Kind::parse(f[2]).ok_or_else(|| bad("kind"))?,
            topology: f[3].parse().map_err(|_| bad("topology"))?,
            orientation: f[4].parse().map_err(|_| bad("orientation"))?,
            graph: f[5].to_string(),
            status: Status::parse(f[6]).ok_or_else(|| bad("status"))?,
            seeds_matched,
            seeds_tested,
            older: f[8].parse().map_err(|_| bad("old"))?,
            probe: f[9].to_string(),
            area_vectors: f[10].to_string(),
            evidence: f[11].to_string(),
        });
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------------------------
// Summaries

/// Counts of a set of rows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub areas: u32,
    pub topologies: u32,
    pub combos: u32,
    /// Combinations where every tested seed matched.
    pub matched: u32,
    /// Where some did.
    pub partial: u32,
    pub failed: u32,
    pub untested: u32,
    pub fixed: u32,
    /// Seeds that matched / were tested, summed over the combinations.
    pub seeds_matched: u32,
    pub seeds_tested: u32,
    /// Combinations with a match that only older builds' vectors vouch for.
    pub older_only: u32,
    /// Topologies whose probe attempts all ran to the end.
    pub probe_ok: u32,
    /// Topologies with a probe attempt.
    pub probed: u32,
}

/// The status word of a probe cell (`ok`, `ok 3/3`, `error (1/3 ok): ...`, `error: ...`).
fn probe_status(p: &str) -> &str {
    p.split(':').next().unwrap_or("").split_whitespace().next().unwrap_or("")
}

/// Counts over `rows`; `areas` counts distinct area ids. A topology counts as probed when any
/// of its orientations has a probe cell, and as ok when all of those are ok.
pub fn count(rows: &[&Row]) -> Counts {
    let mut c = Counts::default();
    let mut last: Option<&str> = None;
    let mut topology: Option<(&str, u32)> = None;
    // (any probe cell seen, all of them ok) of the current topology.
    let mut probe = (false, true);
    let flush = |c: &mut Counts, probe: &mut (bool, bool)| {
        if probe.0 {
            c.probed += 1;
            c.probe_ok += probe.1 as u32;
        }
        *probe = (false, true);
    };
    for r in rows {
        if last != Some(r.area.as_str()) {
            c.areas += 1;
            last = Some(r.area.as_str());
        }
        match r.status {
            Status::NotApplicable => {
                c.fixed += 1;
                continue;
            }
            Status::Match => c.matched += 1,
            Status::Partial => c.partial += 1,
            Status::Fail => c.failed += 1,
            Status::Untested => c.untested += 1,
        }
        c.combos += 1;
        c.seeds_matched += r.seeds_matched;
        c.seeds_tested += r.seeds_tested;
        c.older_only += (r.seeds_matched > 0 && r.older == r.seeds_matched) as u32;
        if topology != Some((r.area.as_str(), r.topology)) {
            flush(&mut c, &mut probe);
            topology = Some((r.area.as_str(), r.topology));
            c.topologies += 1;
        }
        match probe_status(&r.probe) {
            "-" | "" => {}
            s => {
                probe.0 = true;
                probe.1 &= s == "ok";
            }
        }
    }
    flush(&mut c, &mut probe);
    c
}

/// `Counts` per act (BTreeMap order) and per area (row order), plus the total.
pub fn summarize(rows: &[Row]) -> (Counts, BTreeMap<i64, Counts>, Vec<(String, Kind, Counts, String)>) {
    let all: Vec<&Row> = rows.iter().collect();
    let mut acts: BTreeMap<i64, Vec<&Row>> = BTreeMap::new();
    let mut areas: Vec<(String, Kind, Vec<&Row>)> = Vec::new();
    for r in rows {
        acts.entry(r.act).or_default().push(r);
        match areas.last_mut() {
            Some((id, _, v)) if *id == r.area => v.push(r),
            _ => areas.push((r.area.clone(), r.kind, vec![r])),
        }
    }
    (
        count(&all),
        acts.into_iter().map(|(a, v)| (a, count(&v))).collect(),
        areas
            .into_iter()
            .map(|(id, k, v)| {
                let vectors = v[0].area_vectors.clone();
                (id, k, count(&v), vectors)
            })
            .collect(),
    )
}

/// The sentence the tracker's Terrain items quote.
pub fn headline(rows: &[Row]) -> String {
    let (t, _, _) = summarize(rows);
    format!(
        "{} areas ({} without topologies), {} topologies, {} combinations: {} match, {} partial, {} fail, {} untested; seeds matched {}/{}",
        t.areas, t.fixed, t.topologies, t.combos, t.matched, t.partial, t.failed, t.untested, t.seeds_matched, t.seeds_tested
    )
}

/// The README section: totals, a per-act table, and a collapsible per-area table.
pub fn summary_markdown(rows: &[Row]) -> String {
    let (total, acts, areas) = summarize(rows);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "### Terrain matrix\n\nGenerated by `cargo xtask terrain-matrix` into [docs/status/terrain-matrix.tsv](docs/status/terrain-matrix.tsv) \
         (area x topology x orientation; plan: [docs/plans/terrain-all-areas.md](docs/plans/terrain-all-areas.md)). \
         **{}**. Match = every tested seed matched the client's tile hash (the goal is 50/50 seeds per combination), partial = some did, \
         seeds = matched/tested seeds summed over the combinations. {} of the combinations with a match are vouched for only by vectors of older client builds. \
         Probe = the generator runs to the end for the probed seeds (no hash check).\n",
        headline(rows),
        total.older_only
    );
    out.push_str("| Act | Areas | Topologies | Combinations | Match | Partial | Fail | Untested | Seeds | Probe ok |\n|---|---|---|---|---|---|---|---|---|---|\n");
    for (act, c) in &acts {
        let _ = writeln!(
            out,
            "| {act} | {} | {} | {} | {} | {} | {} | {} | {}/{} | {}/{} |",
            c.areas, c.topologies, c.combos, c.matched, c.partial, c.failed, c.untested, c.seeds_matched, c.seeds_tested, c.probe_ok, c.probed
        );
    }
    let _ = writeln!(
        out,
        "| **all** | {} | {} | {} | {} | {} | {} | {} | {}/{} | {}/{} |\n",
        total.areas, total.topologies, total.combos, total.matched, total.partial, total.failed, total.untested, total.seeds_matched, total.seeds_tested, total.probe_ok, total.probed
    );
    let _ = writeln!(out, "<details><summary>Per area ({} rows)</summary>\n", areas.len());
    out.push_str("| Area | Type | Topologies | Combinations | Match | Partial | Fail | Untested | Seeds | Vectors | Probe ok |\n|---|---|---|---|---|---|---|---|---|---|---|\n");
    for (id, kind, c, vectors) in &areas {
        let _ = writeln!(
            out,
            "| {id} | {} | {} | {} | {} | {} | {} | {} | {}/{} | {vectors} | {}/{} |",
            kind.name(),
            c.topologies,
            c.combos,
            c.matched,
            c.partial,
            c.failed,
            c.untested,
            c.seeds_matched,
            c.seeds_tested,
            c.probe_ok,
            c.probed
        );
    }
    out.push_str("\n</details>\n");
    out
}

/// [`summary_markdown`] of the committed matrix, or `None` when there is none yet.
pub fn summary_of_repo(root: &Path) -> Result<Option<String>> {
    let path = root.join(MATRIX_PATH);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let rows = parse_tsv(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(summary_markdown(&rows)))
}

/// Writes the matrix from the game data and the verification outputs.
pub fn run(root: &Path, dumps: Option<PathBuf>, out: Option<PathBuf>) -> Result<()> {
    let game_dir = root.join("GameFiles");
    let schema = root.join("third_party/dat-schema/schema-poe2.min.json");
    let scope = inventory(&game_dir, &schema)?;
    let dir = match dumps {
        Some(d) => root.join(d),
        None => vectors_dir(root)?,
    };
    let mut results = Results::load(&dir)?;
    results.current_build = poe_config::VersionFile::latest(root).ok().map(|v| v.build_revision);
    let rows = build(&scope, &results);
    let path = out.map(|o| root.join(o)).unwrap_or_else(|| root.join(MATRIX_PATH));
    std::fs::write(&path, to_tsv(&rows)).with_context(|| format!("writing {}", path.display()))?;
    println!(
        "{}: {}\nsources in {}: {} vector results, {} forced results, {} probe attempts",
        path.display(),
        headline(&rows),
        dir.display(),
        results.vectors.len(),
        results.forced.len(),
        results.probe.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> Vec<AreaScope> {
        vec![
            AreaScope {
                id: "G1_1".into(),
                act: 1,
                kind: Kind::Tgr,
                topologies: vec![("Metadata/Terrain/Gallows/Act1/1_1/Graphs/intro_1.tgr".into(), 0x81), ("Metadata/x/intro_2.tgr".into(), 0x01)],
            },
            AreaScope { id: "G1_12".into(), act: 1, kind: Kind::Dgr, topologies: vec![("Metadata/x/Canopy.dgr".into(), 0xff)] },
            AreaScope { id: "NULL".into(), act: 0, kind: Kind::Fixed, topologies: vec![] },
        ]
    }

    #[test]
    fn orientation_masks() {
        assert_eq!(orientations(0x01), vec![0]);
        assert_eq!(orientations(0x81), vec![0, 7]);
        assert_eq!(orientations(0xff), (0..8).collect::<Vec<u8>>());
        assert_eq!(orientations(0xf0), vec![4, 5, 6, 7]);
        assert_eq!(orientations(0), vec![0]);
    }

    #[test]
    fn kinds() {
        let tgr = vec![("a/B.TGR".to_string(), 1)];
        let dgr = vec![("a/b.dgr".to_string(), 1)];
        assert_eq!(kind_of(false, false, &tgr), Kind::Tgr);
        assert_eq!(kind_of(false, false, &dgr), Kind::Dgr);
        assert_eq!(kind_of(true, false, &tgr), Kind::Town);
        assert_eq!(kind_of(false, true, &dgr), Kind::Town);
        assert_eq!(kind_of(true, false, &[]), Kind::Fixed);
    }

    #[test]
    fn parses_area_vectors_output() {
        let text = "G1_1\t1000\tmatch (natural 1:0)\n\
                    G1_8\t1001\tmatch (forced 3:7)\tbuild=321592\n\
                    G1_2\t1001\tmatch\n\
                    G1_5\t1001\terror\tcould not tilise edge 14\tbuild=323943\n\
                    G1_4\t1003\tdiffer\tours 1 client 2\tx.tgr\n\
                    G1_9\t1002\tpanic\n\
                    G1_3\t1001\tskipped (budget)\tbuild=1\n\
                    --- per area (match, differ, error, skipped, last error)\n\
                    G1_1\t1\t0\t0\t0\t\n";
        let v = parse_vector_results(text);
        assert_eq!(v.len(), 6);
        assert_eq!(v[0], VectorResult { area: "G1_1".into(), seed: 1000, matched: true, combo: Some((1, 0)), forced: false, build: None });
        assert_eq!(v[1].combo, Some((3, 7)));
        assert!(v[1].forced);
        assert_eq!(v[1].build.as_deref(), Some("321592"));
        assert!(v[2].matched && v[2].combo.is_none());
        assert!(!v[3].matched && !v[4].matched && !v[5].matched);
        assert_eq!(v[3].build.as_deref(), Some("323943"));
    }

    #[test]
    fn parses_forced_results_and_probe() {
        let f = parse_forced_results("# comment\n\nG1_12\t1001\t2\t7\tfail\tmap 169x85 vs 174x87\nG1_8\t1001\t1\t3\tmatch\nG1_8\tbad\t1\t3\tmatch\nG1_8\t1\t1\t3\tmaybe\n");
        assert_eq!(f.len(), 2);
        assert!(!f[0].matched);
        assert_eq!(f[0].detail, "map 169x85 vs 174x87");
        assert!(f[1].matched);
        let p = parse_probe("G1_5\t4\t4\tx/redvale_N2.tgr\t0.69\terror\tcould not tilise edge 14\nG1_5\t5\t0\ty.tgr\t1.19\tok\thash 1 draws 2\t1002\nshort\tline\n");
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].topology, p[0].orientation, p[0].status.as_str(), p[0].seed), (4, 4, "error", None));
        assert_eq!(p[0].detail, "could not tilise edge 14");
        assert_eq!(p[1].seed, Some(1002));
    }

    #[test]
    fn builds_the_matrix() {
        let results = Results {
            vectors: parse_vector_results("G1_1\t1000\tmatch (natural 1:0)\tbuild=323943\nG1_1\t1001\tdiffer\tx\n"),
            forced: parse_forced_results("G1_12\t1001\t1\t7\tfail\tgrid overlap\n"),
            probe: parse_probe("G1_1\t1\t0\ta.tgr\t1.0\tok\thash 1\nG1_1\t2\t0\tb.tgr\t1.0\terror\tfoo\n"),
            current_build: Some("323943".into()),
        };
        let rows = build(&scope(), &results);
        // G1_1: topology 1 has orientations 0 and 7, topology 2 has 0; G1_12 has 8; NULL one row.
        assert_eq!(rows.len(), 2 + 1 + 8 + 1);
        assert_eq!((rows[0].topology, rows[0].orientation, rows[0].status), (1, 0, Status::Match));
        assert_eq!((rows[0].seeds_matched, rows[0].seeds_tested, rows[0].older), (1, 1, 0));
        assert_eq!(rows[0].evidence, "seed 1000 (natural)");
        assert_eq!(rows[0].probe, "ok");
        assert_eq!(rows[0].area_vectors, "1/2");
        assert_eq!((rows[1].orientation, rows[1].status, rows[1].probe.as_str(), rows[1].seeds_tested), (7, Status::Untested, "-", 0));
        assert_eq!(rows[2].probe, "error: foo");
        let fail = rows.iter().find(|r| r.area == "G1_12" && r.orientation == 7).unwrap();
        assert_eq!(fail.status, Status::Fail);
        assert_eq!((fail.seeds_matched, fail.seeds_tested), (0, 1));
        assert_eq!(fail.evidence, "failed: seed 1001 (forced: grid overlap)");
        assert_eq!(rows.last().unwrap().status, Status::NotApplicable);
        assert_eq!(rows.last().unwrap().topology, 0);
    }

    #[test]
    fn seeds_make_partial_and_older_builds_are_counted() {
        let results = Results {
            vectors: parse_vector_results("G1_12\t7\tmatch (forced 1:0)\tbuild=321592\nG1_12\t8\tmatch (forced 1:0)\tbuild=323943\nG1_1\t5\tmatch (forced 2:0)\tbuild=321592\n"),
            forced: parse_forced_results("G1_12\t1\t1\t0\tfail\tx\nG1_12\t2\t1\t0\tmatch\nG1_12\t7\t1\t0\tfail\tsame seed as a vector\n"),
            current_build: Some("323943".into()),
            ..Results::default()
        };
        let rows = build(&scope(), &results);
        let r = rows.iter().find(|r| r.area == "G1_12" && r.orientation == 0).unwrap();
        // Seeds 1 (fail), 2 (match), 7 (vector match beats the forced fail), 8 (match).
        assert_eq!((r.status, r.seeds_matched, r.seeds_tested, r.older), (Status::Partial, 3, 4, 1));
        assert!(r.evidence.contains("seed 7 (forced, build 321592)") && r.evidence.contains("failed: seed 1 (forced: x)"), "{}", r.evidence);
        let g = rows.iter().find(|r| r.area == "G1_1" && r.topology == 2).unwrap();
        assert_eq!((g.status, g.older), (Status::Match, 1));
        let (t, _, _) = summarize(&rows);
        assert_eq!((t.matched, t.partial, t.older_only), (1, 1, 1));
        assert_eq!((t.seeds_matched, t.seeds_tested), (4, 5));
    }

    #[test]
    fn probe_cells_cover_every_seed_and_orientation() {
        let results = Results {
            probe: parse_probe(
                "G1_12\t1\t0\ta.dgr\t1.0\tok\thash 1\t1001\nG1_12\t1\t0\ta.dgr\t1.0\tok\thash 2\t1002\n\
                 G1_12\t1\t1\ta.dgr\t1.0\tok\thash 1\t1001\nG1_12\t1\t1\ta.dgr\t1.0\terror\tno room fits\t1002\n\
                 G1_12\t1\t2\ta.dgr\t1.0\ttimeout\t\t1001\n",
            ),
            ..Results::default()
        };
        let rows = build(&scope(), &results);
        let cell = |o: u8| rows.iter().find(|r| r.area == "G1_12" && r.orientation == o).unwrap().probe.clone();
        assert_eq!(cell(0), "ok 2/2");
        assert_eq!(cell(1), "error (1/2 ok): no room fits");
        assert_eq!(cell(2), "timeout");
        assert_eq!(cell(3), "-");
        // One topology: probed, and not ok because one orientation failed.
        let (t, _, _) = summarize(&rows);
        assert_eq!((t.probe_ok, t.probed), (0, 1));
        assert_eq!(probe_status("error (1/2 ok): no room fits"), "error");
        assert_eq!(probe_status("ok 3/3"), "ok");
    }

    #[test]
    fn tsv_round_trip() {
        let results = Results {
            vectors: parse_vector_results("G1_1\t1000\tmatch (natural 1:0)\n"),
            forced: parse_forced_results("G1_12\t1\t1\t0\tfail\tx\nG1_12\t2\t1\t0\tmatch\n"),
            ..Results::default()
        };
        let rows = build(&scope(), &results);
        let text = to_tsv(&rows);
        assert!(text.starts_with(HEADER));
        assert!(text.contains("\tpartial\t1/2\t"));
        assert_eq!(parse_tsv(&text).unwrap(), rows);
        assert!(parse_tsv("wrong\n").is_err());
        assert!(parse_tsv(&format!("{HEADER}\na\tb\n")).is_err());
        assert!(parse_tsv(&format!("{HEADER}\nG1_1\t1\ttgr\t1\t0\tg\tbogus\t-\t0\t-\t-\t\n")).is_err());
        assert!(parse_tsv(&format!("{HEADER}\nG1_1\t1\ttgr\t1\t0\tg\tmatch\t3\t0\t-\t-\t\n")).is_err());
    }

    #[test]
    fn counts_and_markdown() {
        let results = Results {
            vectors: parse_vector_results("G1_1\t1000\tmatch (natural 1:0)\n"),
            forced: parse_forced_results("G1_12\t1001\t1\t7\tfail\tx\n"),
            probe: parse_probe("G1_1\t1\t0\ta.tgr\t1.0\tok\t\nG1_12\t1\t0\ta.dgr\t1.0\ttimeout\t\n"),
            ..Results::default()
        };
        let rows = build(&scope(), &results);
        let (t, acts, areas) = summarize(&rows);
        assert_eq!((t.areas, t.fixed, t.topologies, t.combos), (3, 1, 3, 11));
        assert_eq!((t.matched, t.partial, t.failed, t.untested), (1, 0, 1, 9));
        assert_eq!((t.seeds_matched, t.seeds_tested), (1, 2));
        assert_eq!((t.probe_ok, t.probed), (1, 2));
        assert_eq!(acts[&1].areas, 2);
        assert_eq!(acts[&0].fixed, 1);
        assert_eq!(areas.len(), 3);
        assert_eq!(areas[0].3, "1/1");
        assert_eq!(
            headline(&rows),
            "3 areas (1 without topologies), 3 topologies, 11 combinations: 1 match, 0 partial, 1 fail, 9 untested; seeds matched 1/2"
        );
        let md = summary_markdown(&rows);
        assert!(md.starts_with("### Terrain matrix"));
        assert!(md.contains("| 1 | 2 | 3 | 11 | 1 | 0 | 1 | 9 | 1/2 | 1/2 |"));
        assert!(md.contains("<details><summary>Per area (3 rows)</summary>"));
        assert!(md.contains("| G1_12 | dgr | 1 | 8 | 0 | 0 | 1 | 7 | 0/1 | - | 0/1 |"));
        assert!(md.ends_with("</details>\n"));
    }
}
