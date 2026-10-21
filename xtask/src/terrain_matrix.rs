//! `cargo xtask terrain-matrix`: the terrain verification matrix, area x topology x orientation
//! -> `untested` / `fail` / `match`, written to `docs/status/terrain-matrix.tsv`.
//!
//! The scope (which areas, topologies and orientations exist) is read from the game data:
//! every `WorldAreas` row, its `Topologies` rows (the default league mode, as
//! `poe_world::area::topology_entries` selects them) and each topology's orientation mask. The
//! results are read from the verification outputs in `dumps/0.5.5e/terrain/vectors/` (all
//! optional; a missing file means nothing is verified from that source):
//!
//! | File | Written by | Gives |
//! |---|---|---|
//! | `area-vectors-results.txt` | `area_vectors` example (stdout) | `match` for the topology:orientation that reproduced a client tile hash |
//! | `forced-results.tsv` | forced-layout client traces (`area_trace`, docs/plans/terrain-all-areas.md) | `match` or `fail` for one forced topology:orientation |
//! | `probe-results.tsv` | `area_probe` example | whether the generator runs at all (no hash check) |
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
/// Verification outputs, relative to the repo root.
pub const VECTORS_DIR: &str = "dumps/0.5.5e/terrain/vectors";
const VECTOR_RESULTS: &str = "area-vectors-results.txt";
const FORCED_RESULTS: &str = "forced-results.tsv";
const PROBE_RESULTS: &str = "probe-results.tsv";
const HEADER: &str = "area\tact\tkind\ttopology\torientation\tgraph\tstatus\tprobe\tarea_vectors\tevidence";

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
    Match,
    /// Fixed areas have no combinations.
    NotApplicable,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Untested => "untested",
            Status::Fail => "fail",
            Status::Match => "match",
            Status::NotApplicable => "n/a",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "untested" => Status::Untested,
            "fail" => Status::Fail,
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
    /// (`map_force_topology` is the 1-based index into this list).
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
}

/// `T:O` of `(natural 5:0)` / `(forced 2:7)`.
fn combo_of(s: &str) -> Option<(u32, u8)> {
    let (t, o) = s.split_once(':')?;
    Some((t.trim().parse().ok()?, o.trim().parse().ok()?))
}

/// Parses `area_vectors` stdout: `area  seed  match [(natural|forced T:O)]` or
/// `area  seed  differ|error|panic ...`. Other lines (the per-area summary) are skipped.
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
        out.push(VectorResult { area: f[0].to_string(), seed, matched, combo, forced });
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
}

/// Parses `area_probe` output: `area  topology  orientation  graph  seconds  status  detail`.
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
    /// `ok`/`error`/`panic`/`timeout` plus the detail (the error's first words), or `-`.
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

/// Combines the scope with the results. A combination is `match` when a client vector or a
/// forced trace reproduced it, `fail` when a forced trace did not, else `untested`. A match
/// beats a fail (the generator is deterministic per seed, so a fail on another seed is
/// evidence of a bug, which the evidence column keeps).
pub fn build(scope: &[AreaScope], r: &Results) -> Vec<Row> {
    let mut matches: HashMap<(&str, u32, u8), Vec<String>> = HashMap::new();
    let mut fails: HashMap<(&str, u32, u8), Vec<String>> = HashMap::new();
    for v in &r.vectors {
        if let (true, Some((t, o))) = (v.matched, v.combo) {
            let how = if v.forced { "forced" } else { "natural" };
            matches.entry((v.area.as_str(), t, o)).or_default().push(format!("seed {} ({how})", v.seed));
        }
    }
    for f in &r.forced {
        let key = (f.area.as_str(), f.topology, f.orientation);
        let what = format!("seed {} (forced trace{}{})", f.seed, if f.detail.is_empty() { "" } else { ": " }, f.detail);
        if f.matched { matches.entry(key).or_default().push(what) } else { fails.entry(key).or_default().push(what) }
    }
    let probe: HashMap<(&str, u32), &ProbeResult> = r.probe.iter().map(|p| ((p.area.as_str(), p.topology), p)).collect();
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
                let (status, ev) = match (matches.get(&key), fails.get(&key)) {
                    (Some(m), f) => {
                        let mut e = m.join("; ");
                        if let Some(f) = f {
                            e.push_str(&format!(" | also failed: {}", f.join("; ")));
                        }
                        (Status::Match, e)
                    }
                    (None, Some(f)) => (Status::Fail, f.join("; ")),
                    (None, None) => (Status::Untested, String::new()),
                };
                // The probe ran one orientation per topology: the first allowed one.
                let first = orientations(*mask)[0];
                let probe_cell = probe
                    .get(&(a.id.as_str(), t))
                    .filter(|p| p.orientation == o && o == first)
                    .map(|p| if p.detail.is_empty() || p.status == "ok" { p.status.clone() } else { format!("{}: {}", p.status, short_detail(&p.detail)) })
                    .unwrap_or_else(|| "-".into());
                rows.push(Row {
                    area: a.id.clone(),
                    act: a.act,
                    kind: a.kind,
                    topology: t,
                    orientation: o,
                    graph: short_graph(path),
                    status,
                    probe: probe_cell,
                    area_vectors: area_vectors.clone(),
                    evidence: ev,
                });
            }
        }
    }
    rows
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n'], " ")
}

/// The TSV text (header, one line per row, final newline).
pub fn to_tsv(rows: &[Row]) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for r in rows {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.area,
            r.act,
            r.kind.name(),
            r.topology,
            r.orientation,
            clean(&r.graph),
            r.status.name(),
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
        if f.len() != 10 {
            bail!("line {}: {} columns, expected 10", n + 2, f.len());
        }
        let bad = |what: &str| anyhow::anyhow!("line {}: bad {what}", n + 2);
        rows.push(Row {
            area: f[0].to_string(),
            act: f[1].parse().map_err(|_| bad("act"))?,
            kind: Kind::parse(f[2]).ok_or_else(|| bad("kind"))?,
            topology: f[3].parse().map_err(|_| bad("topology"))?,
            orientation: f[4].parse().map_err(|_| bad("orientation"))?,
            graph: f[5].to_string(),
            status: Status::parse(f[6]).ok_or_else(|| bad("status"))?,
            probe: f[7].to_string(),
            area_vectors: f[8].to_string(),
            evidence: f[9].to_string(),
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
    pub matched: u32,
    pub failed: u32,
    pub untested: u32,
    pub fixed: u32,
    /// Topologies whose probe attempt ran to the end.
    pub probe_ok: u32,
    /// Topologies with a probe attempt.
    pub probed: u32,
}

fn probe_status(p: &str) -> &str {
    p.split(':').next().unwrap_or("")
}

/// Counts over `rows`; `areas` counts distinct area ids.
pub fn count(rows: &[&Row]) -> Counts {
    let mut c = Counts::default();
    let mut last: Option<&str> = None;
    let mut seen_topology: Option<(&str, u32)> = None;
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
            Status::Fail => c.failed += 1,
            Status::Untested => c.untested += 1,
        }
        c.combos += 1;
        if seen_topology != Some((r.area.as_str(), r.topology)) {
            seen_topology = Some((r.area.as_str(), r.topology));
            c.topologies += 1;
        }
        match probe_status(&r.probe) {
            "-" | "" => {}
            "ok" => {
                c.probe_ok += 1;
                c.probed += 1;
            }
            _ => c.probed += 1,
        }
    }
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
        "{} areas ({} without topologies), {} topologies, {} combinations: {} match, {} fail, {} untested",
        t.areas, t.fixed, t.topologies, t.combos, t.matched, t.failed, t.untested
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
         **{}**. Probe = the generator runs to the end for one seed (no hash check).\n",
        headline(rows)
    );
    out.push_str("| Act | Areas | Topologies | Combinations | Match | Fail | Untested | Probe ok |\n|---|---|---|---|---|---|---|---|\n");
    for (act, c) in &acts {
        let _ = writeln!(out, "| {act} | {} | {} | {} | {} | {} | {} | {}/{} |", c.areas, c.topologies, c.combos, c.matched, c.failed, c.untested, c.probe_ok, c.probed);
    }
    let _ = writeln!(
        out,
        "| **all** | {} | {} | {} | {} | {} | {} | {}/{} |\n",
        total.areas, total.topologies, total.combos, total.matched, total.failed, total.untested, total.probe_ok, total.probed
    );
    let _ = writeln!(out, "<details><summary>Per area ({} rows)</summary>\n", areas.len());
    out.push_str("| Area | Type | Topologies | Combinations | Match | Fail | Untested | Vectors | Probe ok |\n|---|---|---|---|---|---|---|---|---|\n");
    for (id, kind, c, vectors) in &areas {
        let _ = writeln!(
            out,
            "| {id} | {} | {} | {} | {} | {} | {} | {vectors} | {}/{} |",
            kind.name(),
            c.topologies,
            c.combos,
            c.matched,
            c.failed,
            c.untested,
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
    let dir = dumps.map(|d| root.join(d)).unwrap_or_else(|| root.join(VECTORS_DIR));
    let results = Results::load(&dir)?;
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
                    G1_8\t1001\tmatch (forced 3:7)\n\
                    G1_2\t1001\tmatch\n\
                    G1_5\t1001\terror\tcould not tilise edge 14\n\
                    G1_4\t1003\tdiffer\tours 1 client 2\tx.tgr\n\
                    G1_9\t1002\tpanic\n\
                    --- per area (match, differ, error, last error)\n\
                    G1_1\t1\t0\t0\t\n";
        let v = parse_vector_results(text);
        assert_eq!(v.len(), 6);
        assert_eq!(v[0], VectorResult { area: "G1_1".into(), seed: 1000, matched: true, combo: Some((1, 0)), forced: false });
        assert_eq!(v[1].combo, Some((3, 7)));
        assert!(v[1].forced);
        assert!(v[2].matched && v[2].combo.is_none());
        assert!(!v[3].matched && !v[4].matched && !v[5].matched);
    }

    #[test]
    fn parses_forced_results_and_probe() {
        let f = parse_forced_results("# comment\n\nG1_12\t1001\t2\t7\tfail\tmap 169x85 vs 174x87\nG1_8\t1001\t1\t3\tmatch\nG1_8\tbad\t1\t3\tmatch\nG1_8\t1\t1\t3\tmaybe\n");
        assert_eq!(f.len(), 2);
        assert!(!f[0].matched);
        assert_eq!(f[0].detail, "map 169x85 vs 174x87");
        assert!(f[1].matched);
        let p = parse_probe("G1_5\t4\t4\tx/redvale_N2.tgr\t0.69\terror\tcould not tilise edge 14\nG1_5\t5\t0\ty.tgr\t1.19\tok\thash 1 draws 2\nshort\tline\n");
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].topology, p[0].orientation, p[0].status.as_str()), (4, 4, "error"));
        assert_eq!(p[0].detail, "could not tilise edge 14");
    }

    #[test]
    fn builds_the_matrix() {
        let results = Results {
            vectors: parse_vector_results("G1_1\t1000\tmatch (natural 1:0)\nG1_1\t1001\tdiffer\tx\n"),
            forced: parse_forced_results("G1_12\t1001\t1\t7\tfail\tgrid overlap\n"),
            probe: parse_probe("G1_1\t1\t0\ta.tgr\t1.0\tok\thash 1\nG1_1\t2\t0\tb.tgr\t1.0\terror\tfoo\n"),
        };
        let rows = build(&scope(), &results);
        // G1_1: topology 1 has orientations 0 and 7, topology 2 has 0; G1_12 has 8; NULL one row.
        assert_eq!(rows.len(), 2 + 1 + 8 + 1);
        assert_eq!((rows[0].topology, rows[0].orientation, rows[0].status), (1, 0, Status::Match));
        assert_eq!(rows[0].evidence, "seed 1000 (natural)");
        assert_eq!(rows[0].probe, "ok");
        assert_eq!(rows[0].area_vectors, "1/2");
        assert_eq!((rows[1].orientation, rows[1].status, rows[1].probe.as_str()), (7, Status::Untested, "-"));
        assert_eq!(rows[2].probe, "error: foo");
        let fail = rows.iter().find(|r| r.area == "G1_12" && r.orientation == 7).unwrap();
        assert_eq!(fail.status, Status::Fail);
        assert_eq!(fail.evidence, "seed 1001 (forced trace: grid overlap)");
        assert_eq!(rows.last().unwrap().status, Status::NotApplicable);
        assert_eq!(rows.last().unwrap().topology, 0);
    }

    #[test]
    fn a_match_beats_a_fail_on_another_seed() {
        let results = Results {
            forced: parse_forced_results("G1_12\t1\t1\t0\tfail\tx\nG1_12\t2\t1\t0\tmatch\n"),
            ..Results::default()
        };
        let rows = build(&scope(), &results);
        let r = rows.iter().find(|r| r.area == "G1_12" && r.orientation == 0).unwrap();
        assert_eq!(r.status, Status::Match);
        assert!(r.evidence.contains("also failed"));
    }

    #[test]
    fn tsv_round_trip() {
        let results = Results { vectors: parse_vector_results("G1_1\t1000\tmatch (natural 1:0)\n"), ..Results::default() };
        let rows = build(&scope(), &results);
        let text = to_tsv(&rows);
        assert!(text.starts_with(HEADER));
        assert_eq!(parse_tsv(&text).unwrap(), rows);
        assert!(parse_tsv("wrong\n").is_err());
        assert!(parse_tsv(&format!("{HEADER}\na\tb\n")).is_err());
        assert!(parse_tsv(&format!("{HEADER}\nG1_1\t1\ttgr\t1\t0\tg\tbogus\t-\t-\t\n")).is_err());
    }

    #[test]
    fn counts_and_markdown() {
        let results = Results {
            vectors: parse_vector_results("G1_1\t1000\tmatch (natural 1:0)\n"),
            forced: parse_forced_results("G1_12\t1001\t1\t7\tfail\tx\n"),
            probe: parse_probe("G1_1\t1\t0\ta.tgr\t1.0\tok\t\nG1_12\t1\t0\ta.dgr\t1.0\ttimeout\t\n"),
        };
        let rows = build(&scope(), &results);
        let (t, acts, areas) = summarize(&rows);
        assert_eq!((t.areas, t.fixed, t.topologies, t.combos), (3, 1, 3, 11));
        assert_eq!((t.matched, t.failed, t.untested), (1, 1, 9));
        assert_eq!((t.probe_ok, t.probed), (1, 2));
        assert_eq!(acts[&1].areas, 2);
        assert_eq!(acts[&0].fixed, 1);
        assert_eq!(areas.len(), 3);
        assert_eq!(areas[0].3, "1/1");
        assert_eq!(headline(&rows), "3 areas (1 without topologies), 3 topologies, 11 combinations: 1 match, 1 fail, 9 untested");
        let md = summary_markdown(&rows);
        assert!(md.starts_with("### Terrain matrix"));
        assert!(md.contains("| 1 | 2 | 3 | 11 | 1 | 1 | 9 | 1/2 |"));
        assert!(md.contains("<details><summary>Per area (3 rows)</summary>"));
        assert!(md.contains("| G1_12 | dgr | 1 | 8 | 0 | 1 | 7 | - | 0/1 |"));
        assert!(md.ends_with("</details>\n"));
    }
}
