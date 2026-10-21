//! `cargo xtask check-build <exe> [--bundles <dir>]`: what a game build breaks.
//!
//! Scans an executable for every entry of the client DLL's signature table
//! (`poe-hook-sig`, the same matcher the DLL uses), checks the build identity against
//! `versions/<version>.toml`, the opcode tables against the exe (HNC table, deserializer
//! fingerprints), the strings the tools depend on, and the game data the server reads
//! (`poe_data::used`, the anchors of the version file) against `Bundles2`. Writes
//! `dumps/<version>/check-build.md` and prints a summary. Exit code: 0 all fine, 1 warnings
//! (a research signature, a changed row count, ...), 2 something required is missing, ambiguous
//! or different. Procedure around it: docs/guides/new-game-version.md.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use poe_config::{Config, VersionFile};
use poe_hook_sig::{Failure, Image, Table, map_file, resolve};
use sha2::{Digest, Sha256};

use crate::fingerprint;

pub struct Options {
    pub exe: PathBuf,
    /// The game directory (the folder with `Bundles2/`) for the data checks.
    pub bundles: Option<PathBuf>,
    /// Version file to check against (default: the one whose exe fingerprint matches, else
    /// the config's `[game] version`, else the newest).
    pub version: Option<String>,
    /// Write the report here instead of `dumps/<version>/check-build.md`.
    pub report: Option<PathBuf>,
    /// Write `<opcodes>/deserializers.tsv` (the baseline of the drift check) from this exe and
    /// the factory dumps instead of checking.
    pub write_fingerprints: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fail,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Info => "info",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        }
    }
}

struct Finding {
    block: &'static str,
    item: String,
    status: String,
    level: Level,
    detail: String,
}

#[derive(Default)]
struct Report {
    findings: Vec<Finding>,
    /// Free text per block, printed above its table.
    notes: BTreeMap<&'static str, Vec<String>>,
}

impl Report {
    fn add(
        &mut self,
        block: &'static str,
        item: impl Into<String>,
        status: impl Into<String>,
        level: Level,
        detail: impl Into<String>,
    ) {
        self.findings.push(Finding {
            block,
            item: item.into(),
            status: status.into(),
            level,
            detail: detail.into(),
        });
    }

    fn note(&mut self, block: &'static str, text: impl Into<String>) {
        self.notes.entry(block).or_default().push(text.into());
    }

    fn worst(&self) -> Level {
        self.findings
            .iter()
            .map(|f| f.level)
            .max()
            .unwrap_or(Level::Ok)
    }
}

const IDENTITY: &str = "Identity";
const SIGNATURES: &str = "Signatures";
const FIELDS: &str = "Struct offsets";
const FEATURES: &str = "Features";
const OPCODES: &str = "Opcodes";
const STRINGS: &str = "Strings";
const DATA: &str = "Game data";
const FILES: &str = "Files";
const BLOCKS: [&str; 8] = [
    IDENTITY, SIGNATURES, FIELDS, FEATURES, OPCODES, STRINGS, DATA, FILES,
];

/// Runs the checks. Returns the process exit code.
pub fn run(root: &Path, config: Option<&Config>, opts: Options) -> Result<i32> {
    let file =
        std::fs::read(&opts.exe).with_context(|| format!("reading {}", opts.exe.display()))?;
    let mapped = map_file(&file).map_err(|e| anyhow::anyhow!("{}: {e}", opts.exe.display()))?;
    let image =
        Image::from_mapped(&mapped).map_err(|e| anyhow::anyhow!("{}: {e}", opts.exe.display()))?;
    let sha256 = hex::encode(Sha256::digest(&file));

    let version = pick_version(root, config, &opts, &image, &sha256)?;
    println!(
        "check-build: {} against versions/{}.toml (exe {} bytes, sha256 {}...)",
        opts.exe.display(),
        version.version,
        file.len(),
        &sha256[..16]
    );

    if opts.write_fingerprints {
        return write_fingerprints(root, &version, &image);
    }

    let mut r = Report::default();
    let table = load_table(root, &version, &mut r);
    identity(
        root,
        &version,
        &file,
        &sha256,
        &image,
        table.as_ref(),
        &mut r,
    );
    if let Some(t) = &table {
        signatures(&version, t, &image, &mut r);
    }
    opcodes(root, &version, &file, &image, &mut r);
    strings(&version, &file, &mut r);
    let game_dir = game_dir(root, config, &opts);
    data(&game_dir, root, config, &version, &mut r);
    files(&game_dir, &version, &mut r);

    let report_path = opts
        .report
        .clone()
        .unwrap_or_else(|| version.dumps_dir(root).join("check-build.md"));
    let md = markdown(
        &opts.exe,
        &version,
        &file,
        &sha256,
        &image,
        table.as_ref(),
        &r,
    );
    if let Some(dir) = report_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&report_path, md)
        .with_context(|| format!("writing {}", report_path.display()))?;
    Ok(summary(&r, &report_path))
}

/// The version file to check against.
fn pick_version(
    root: &Path,
    config: Option<&Config>,
    opts: &Options,
    image: &Image,
    sha256: &str,
) -> Result<VersionFile> {
    if let Some(v) = &opts.version {
        return Ok(VersionFile::load(root, v)?);
    }
    let known = poe_config::versions::known_versions(root);
    let loaded: Vec<VersionFile> = known
        .iter()
        .filter_map(|v| VersionFile::load(root, v).ok())
        .collect();
    // The file this exe is the build of: checksum, else the PE fingerprint.
    if let Some(v) = loaded
        .iter()
        .find(|v| v.exe_sha256().as_deref() == Some(sha256))
        .or_else(|| {
            loaded.iter().find(|v| {
                v.exe.pe_timestamp == image.info.timestamp
                    && v.exe.image_size == image.info.size_of_image
            })
        })
    {
        return Ok(v.clone());
    }
    // A new build: compare with what the server speaks, else the newest.
    if let Some(c) = config
        && let Some(v) = loaded.iter().find(|v| v.version == c.game.version)
    {
        println!(
            "this exe is none of the known builds ({}); checking against the server's version {}",
            known.join(", "),
            v.version
        );
        return Ok(v.clone());
    }
    let latest = VersionFile::latest(root).context("no versions/*.toml")?;
    println!(
        "this exe is none of the known builds ({}); checking against {}",
        known.join(", "),
        latest.version
    );
    Ok(latest)
}

fn load_table(root: &Path, v: &VersionFile, r: &mut Report) -> Option<Table> {
    let Some(path) = v.signatures_path(root) else {
        r.add(
            SIGNATURES,
            "signature table",
            "not configured",
            Level::Fail,
            "versions file has no [client_hook] signatures",
        );
        return None;
    };
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| Table::parse(&t))
    {
        Ok(t) => Some(t),
        Err(e) => {
            r.add(
                SIGNATURES,
                path.display().to_string(),
                "unreadable",
                Level::Fail,
                e,
            );
            None
        }
    }
}

fn identity(
    root: &Path,
    v: &VersionFile,
    file: &[u8],
    sha256: &str,
    image: &Image,
    table: Option<&Table>,
    r: &mut Report,
) {
    let eq = |r: &mut Report, item: &str, got: String, want: String, level: Level| {
        if got == want {
            r.add(IDENTITY, item, "matches", Level::Ok, got);
        } else {
            r.add(
                IDENTITY,
                item,
                "DIFFERENT",
                level,
                format!("exe {got}, versions/{}.toml {want}", v.version),
            );
        }
    };
    eq(
        r,
        "exe size",
        file.len().to_string(),
        v.exe.size.to_string(),
        Level::Fail,
    );
    match v.exe_sha256() {
        Some(want) => eq(r, "exe sha256", sha256.to_owned(), want, Level::Fail),
        None => r.add(IDENTITY, "exe sha256", "not recorded", Level::Info, sha256),
    }
    eq(
        r,
        "PE timestamp",
        format!("{:#x}", image.info.timestamp),
        format!("{:#x}", v.exe.pe_timestamp),
        Level::Fail,
    );
    eq(
        r,
        "image size",
        format!("{:#x}", image.info.size_of_image),
        format!("{:#x}", v.exe.image_size),
        Level::Fail,
    );
    if let Some(t) = table {
        // The revision string, found the way the DLL finds it.
        let b = poe_hook_sig::resolve::check_build(t, image);
        match (&b.revision_found, b.revision_checked) {
            (Some(found), _) => eq(
                r,
                "build revision (string in the exe)",
                found.clone(),
                v.build_revision.clone(),
                Level::Fail,
            ),
            (None, true) => r.add(
                IDENTITY,
                "build revision (string in the exe)",
                "not found",
                Level::Warn,
                "the table's revision_pattern matched nothing (or several revisions)",
            ),
            (None, false) => r.add(
                IDENTITY,
                "build revision (string in the exe)",
                "not checked",
                Level::Info,
                "the signature table has no revision_pattern",
            ),
        }
        // The two definitions of the build must agree.
        let tb = &t.build;
        let mut disagree = Vec::new();
        if tb.version != v.version {
            disagree.push(format!("version {} / {}", tb.version, v.version));
        }
        if tb.build_revision != v.build_revision {
            disagree.push(format!(
                "build_revision {} / {}",
                tb.build_revision, v.build_revision
            ));
        }
        if tb.pe_timestamp != v.exe.pe_timestamp {
            disagree.push(format!(
                "pe_timestamp {:#x} / {:#x}",
                tb.pe_timestamp, v.exe.pe_timestamp
            ));
        }
        if tb.image_size != v.exe.image_size {
            disagree.push(format!(
                "image_size {:#x} / {:#x}",
                tb.image_size, v.exe.image_size
            ));
        }
        if let (Some(a), Some(b)) = (&tb.exe_sha256, v.exe_sha256())
            && !a.eq_ignore_ascii_case(&b)
        {
            disagree.push("exe_sha256".into());
        }
        if disagree.is_empty() {
            r.add(
                IDENTITY,
                "signature table [build] = version file",
                "agree",
                Level::Ok,
                "",
            );
        } else {
            r.add(
                IDENTITY,
                "signature table [build] = version file",
                "DISAGREE",
                Level::Fail,
                disagree.join("; "),
            );
        }
    }
    // Where the server learns the version.
    let hashes = &v.client.hashes;
    match (hashes.is_empty(), v.client.hashes_confirmed) {
        (true, _) => r.add(IDENTITY, "client_hash", "none recorded", Level::Warn, "the server cannot tell clients apart; record the client_hash of the in-game check in [client] hashes"),
        (false, false) => r.add(
            IDENTITY,
            "client_hash",
            "derived, unconfirmed",
            Level::Info,
            format!(
                "{} recorded ({}the exe's sha256); the server refuses nothing until the in-game check sets hashes_confirmed = true",
                hashes.len(),
                if hashes.iter().any(|h| h.eq_ignore_ascii_case(sha256)) { "this is " } else { "NOT " }
            ),
        ),
        (false, true) => {
            let level = if hashes.iter().any(|h| h.eq_ignore_ascii_case(sha256)) { Level::Ok } else { Level::Info };
            r.add(IDENTITY, "client_hash", "confirmed", level, format!("{} recorded", hashes.len()));
        }
    }
    // The launcher's copy of the version file.
    match crate::versions::check_launcher_copy(root, &v.version) {
        Ok(None) => r.add(
            IDENTITY,
            "launcher copy of the version file",
            "in sync",
            Level::Ok,
            "",
        ),
        Ok(Some(why)) => r.add(
            IDENTITY,
            "launcher copy of the version file",
            "STALE",
            Level::Fail,
            format!("{why}; run `cargo xtask versions`"),
        ),
        Err(e) => r.add(
            IDENTITY,
            "launcher copy of the version file",
            "not checked",
            Level::Info,
            e.to_string(),
        ),
    }
    // Terrain vectors recorded for an older build.
    vectors(root, v, r);
}

/// `terrain-vectors.tsv` has a `build` column in newer files: vectors of another build are stale.
fn vectors(root: &Path, v: &VersionFile, r: &mut Report) {
    let path = v
        .dumps_dir(root)
        .join("terrain/vectors/terrain-vectors.tsv");
    let Ok(text) = std::fs::read_to_string(&path) else {
        r.add(
            IDENTITY,
            "terrain vectors",
            "no file",
            Level::Info,
            path.display().to_string(),
        );
        return;
    };
    let mut builds: BTreeMap<String, usize> = BTreeMap::new();
    let mut rows = 0;
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        rows += 1;
        *builds
            .entry(line.split('\t').nth(5).unwrap_or("?").to_owned())
            .or_default() += 1;
    }
    let current = builds.get(&v.build_revision).copied().unwrap_or(0);
    if builds.keys().all(|b| b == "?") {
        r.add(
            IDENTITY,
            "terrain vectors",
            "no build column",
            Level::Info,
            format!("{rows} vectors; cannot tell which build made them"),
        );
    } else if current == rows {
        r.add(
            IDENTITY,
            "terrain vectors",
            "current",
            Level::Ok,
            format!("{rows} vectors of build {}", v.build_revision),
        );
    } else {
        r.add(
            IDENTITY,
            "terrain vectors",
            "stale",
            Level::Warn,
            format!("{current} of {rows} vectors are of build {} ({builds:?}); re-export after a game update", v.build_revision),
        );
    }
}

fn signatures(
    v: &VersionFile,
    table: &Table,
    image: &Image,
    r: &mut Report,
) -> poe_hook_sig::Resolution {
    let required = |features: &[String]| {
        features
            .iter()
            .any(|f| v.client_hook.required_features.contains(f))
    };
    let t0 = std::time::Instant::now();
    let res = resolve(table, image, None);
    r.note(
        SIGNATURES,
        format!(
            "{} entries resolved in {:.1} s; image base {:#x}",
            table.sig.len() + table.field.len(),
            t0.elapsed().as_secs_f64(),
            res.image_base
        ),
    );
    for s in &table.sig {
        let level_failed = if s.features.is_empty() || !required(&s.features) {
            Level::Warn
        } else {
            Level::Fail
        };
        match &res.sigs[&s.name] {
            Ok(found) => {
                let va = res.image_base + found.rva;
                let (level, detail) = match s.ghidra {
                    Some(g) if g == va => (Level::Ok, format!("{va:#x} = table address")),
                    Some(g) => (
                        Level::Info,
                        format!("{va:#x} (table {g:#x}, moved {:+#x})", va as i64 - g as i64),
                    ),
                    None => (Level::Ok, format!("{va:#x}")),
                };
                r.add(SIGNATURES, &s.name, "found", level, detail);
            }
            Err(Failure::Missing) => r.add(
                SIGNATURES,
                &s.name,
                "missing",
                level_failed,
                feature_note(&s.features, "matched 0 times"),
            ),
            Err(Failure::Ambiguous(n)) => r.add(
                SIGNATURES,
                &s.name,
                "ambiguous",
                level_failed,
                feature_note(&s.features, &format!("matched {n} times")),
            ),
            Err(e) => r.add(
                SIGNATURES,
                &s.name,
                "failed",
                level_failed,
                feature_note(&s.features, &e.to_string()),
            ),
        }
    }
    for f in &table.field {
        let level_failed = if f.features.is_empty() || !required(&f.features) {
            Level::Warn
        } else {
            Level::Fail
        };
        match &res.fields[&f.name] {
            Ok(found) => match found.changed_from {
                None => r.add(
                    FIELDS,
                    &f.name,
                    "found",
                    Level::Ok,
                    format!("{:#x}", found.value),
                ),
                Some(old) => r.add(
                    FIELDS,
                    &f.name,
                    "changed",
                    Level::Info,
                    format!("{old:#x} -> {:#x} (the DLL reads the new one)", found.value),
                ),
            },
            Err(Failure::Missing) => r.add(
                FIELDS,
                &f.name,
                "missing",
                level_failed,
                feature_note(&f.features, "matched 0 times"),
            ),
            Err(Failure::Ambiguous(n)) => r.add(
                FIELDS,
                &f.name,
                "ambiguous",
                level_failed,
                feature_note(&f.features, &format!("matched {n} times")),
            ),
            Err(e) => r.add(
                FIELDS,
                &f.name,
                "failed",
                level_failed,
                feature_note(&f.features, &e.to_string()),
            ),
        }
    }
    for (name, st) in &res.features {
        let req = v.client_hook.required_features.contains(name);
        let kind = if req { "required" } else { "research" };
        if st.ok {
            r.add(
                FEATURES,
                name,
                "on",
                Level::Ok,
                format!("{kind}, {} entries", st.entries),
            );
        } else {
            r.add(
                FEATURES,
                name,
                "OFF",
                if req { Level::Fail } else { Level::Warn },
                format!("{kind}: {}", st.problems.join("; ")),
            );
        }
    }
    for name in &v.client_hook.required_features {
        if !res.features.contains_key(name) {
            r.add(
                FEATURES,
                name,
                "unknown",
                Level::Warn,
                "listed as required in the version file, but no signature entry names it",
            );
        }
    }
    res
}

fn feature_note(features: &[String], what: &str) -> String {
    if features.is_empty() {
        format!("{what} (anchor only)")
    } else {
        format!("{what}; turns off: {}", features.join(", "))
    }
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let first = needle[0];
    let mut from = 0;
    while let Some(p) = hay[from..hay.len() - needle.len() + 1]
        .iter()
        .position(|b| *b == first)
    {
        let at = from + p;
        if &hay[at..at + needle.len()] == needle {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

fn strings(v: &VersionFile, file: &[u8], r: &mut Report) {
    for s in &v.client.strings {
        let ascii = find_bytes(file, s.as_bytes());
        let wide: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let utf16 = find_bytes(file, &wide);
        match (ascii, utf16) {
            (Some(o), _) => r.add(
                STRINGS,
                s,
                "found",
                Level::Ok,
                format!("ASCII at file offset {o:#x}"),
            ),
            (None, Some(o)) => r.add(
                STRINGS,
                s,
                "found",
                Level::Ok,
                format!("UTF-16 at file offset {o:#x}"),
            ),
            (None, None) => r.add(
                STRINGS,
                s,
                "missing",
                Level::Warn,
                "a tool or the DLL depends on this text",
            ),
        }
    }
}

fn opcodes(root: &Path, v: &VersionFile, file: &[u8], image: &Image, r: &mut Report) {
    let dir = v.opcodes_dir(root);
    match poe_proto::Protocol::load(&dir) {
        Ok(p) => {
            r.add(
                OPCODES,
                "opcode tables",
                "load",
                Level::Ok,
                dir.display().to_string(),
            );
            match hex::decode(p.hnc.key_xor.trim()) {
                Ok(key) if key.len() == 64 => match find_bytes(file, &key) {
                    Some(o) => r.add(OPCODES, "HNC key_xor (64 bytes by value)", "found", Level::Ok, format!("file offset {o:#x}")),
                    None => r.add(OPCODES, "HNC key_xor (64 bytes by value)", "missing", Level::Fail, "the table moved or changed: re-extract it (docs/re/crypto.md); the HNC side channel cannot be decoded"),
                },
                _ => r.add(OPCODES, "HNC key_xor (64 bytes by value)", "invalid", Level::Fail, "hnc.toml key_xor is not 64 hex bytes"),
            }
            r.add(OPCODES, "HNC swaps", "not checked", Level::Info, "derived from the code at the swap routine (0x1421c50b0 in 0.5.5e); check by the live handshake");
            fingerprints(&dir, &p, image, r);
        }
        Err(e) => r.add(
            OPCODES,
            "opcode tables",
            "unreadable",
            Level::Fail,
            e.to_string(),
        ),
    }
}

/// One line of `deserializers.tsv`.
struct Baseline {
    channel: String,
    opcode: u32,
    fingerprint: String,
    count: usize,
    ghidra: String,
}

fn read_baseline(path: &Path) -> Result<Vec<Baseline>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() < 6 {
            bail!("{}: bad line {line:?}", path.display());
        }
        out.push(Baseline {
            channel: c[0].to_owned(),
            opcode: u32::from_str_radix(c[1].trim_start_matches("0x"), 16)?,
            fingerprint: c[3].to_owned(),
            count: c[4].parse()?,
            ghidra: c[5].to_owned(),
        });
    }
    Ok(out)
}

/// Deserializer fingerprint drift (docs/re/update-compatibility.md S4): the baseline holds,
/// per server-to-client opcode of the factories, a fingerprint of the class's deserializer.
/// In the new exe every `.pdata` function is fingerprinted; a baseline entry whose fingerprint
/// is gone has a changed layout (or the function was rewritten), one that is there is unchanged.
fn fingerprints(dir: &Path, p: &poe_proto::Protocol, image: &Image, r: &mut Report) {
    let path = dir.join("deserializers.tsv");
    let baseline = match read_baseline(&path) {
        Ok(b) => b,
        Err(e) => {
            r.add(OPCODES, "deserializer fingerprints", "no baseline", Level::Warn, format!("{}: {e:#}; create it with `cargo xtask check-build <exe> --write-fingerprints` (needs the factory dumps, docs/re/datamining.md)", path.display()));
            return;
        }
    };
    let t0 = std::time::Instant::now();
    let now = fingerprint::all(image);
    r.note(
        OPCODES,
        format!(
            "{} function fingerprints computed in {:.1} s",
            now.len(),
            t0.elapsed().as_secs_f64()
        ),
    );
    let (mut same, mut changed, mut changed_named) = (0, 0, 0);
    let mut seen: BTreeMap<(String, String), ()> = BTreeMap::new();
    for b in &baseline {
        let named = {
            let table = if b.channel == "login" {
                &p.login
            } else {
                &p.instance
            };
            table
                .name(poe_proto::Direction::Server, b.opcode as u16)
                .map(str::to_owned)
        };
        let found = now.get(&b.fingerprint).map_or(0, Vec::len);
        let item = format!(
            "{} {:#06x} {}",
            b.channel,
            b.opcode,
            named.as_deref().unwrap_or("")
        );
        if found == b.count {
            same += 1;
            // One line for the named ones (what we implement); the rest is counted.
            if named.is_some()
                && seen
                    .insert((b.channel.clone(), b.fingerprint.clone()), ())
                    .is_none()
            {
                r.add(
                    OPCODES,
                    item.trim().to_owned(),
                    "unchanged",
                    Level::Ok,
                    format!("deserializer {}", b.fingerprint),
                );
            }
        } else if found == 0 {
            changed += 1;
            if named.is_some() {
                changed_named += 1;
            }
            r.add(
                OPCODES,
                item.trim().to_owned(),
                "layout changed",
                if named.is_some() { Level::Fail } else { Level::Warn },
                format!("deserializer {} (was {}) has no match: the message layout changed or the function was rewritten; re-read it in Ghidra", b.fingerprint, b.ghidra),
            );
        } else {
            changed += 1;
            r.add(
                OPCODES,
                item.trim().to_owned(),
                "count changed",
                if named.is_some() {
                    Level::Warn
                } else {
                    Level::Info
                },
                format!(
                    "{found} functions now share the fingerprint (baseline {}); ambiguous",
                    b.count
                ),
            );
        }
    }
    r.add(
        OPCODES,
        "deserializer fingerprints",
        if changed == 0 { "unchanged" } else { "drift" },
        if changed_named > 0 { Level::Fail } else if changed > 0 { Level::Warn } else { Level::Ok },
        format!("{same} of {} baseline opcodes unchanged, {changed} changed ({changed_named} of them named in the opcode tables)", baseline.len()),
    );
    r.note(
        OPCODES,
        "Not derived from the exe: renumbered opcodes (needs the factory switch of the new build: `scripts/re/find_switches.py` + `dump_factory.py`, then compare the fingerprints by hand) and the client-to-server messages (only the factories' server-to-client deserializers are fingerprinted).",
    );
}

/// `--write-fingerprints`: the baseline of the drift check from this exe and the factory dumps.
fn write_fingerprints(root: &Path, v: &VersionFile, image: &Image) -> Result<i32> {
    let ghidra = v.dumps_dir(root).join("ghidra");
    let protocol = poe_proto::Protocol::load(&v.opcodes_dir(root))?;
    let sources = [
        ("login", "login_factory_s2c.json"),
        ("instance", "instance_factory_s2c.json"),
        ("instance", "instance_factory2_s2c.json"),
    ];
    let all = fingerprint::all(image);
    let mut lines = Vec::new();
    for (channel, name) in sources {
        let path = ghidra.join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            println!(
                "skipped {} (not found; scripts/re/dump_factory.py writes it)",
                path.display()
            );
            continue;
        };
        let entries: Vec<serde_json::Value> = serde_json::from_str(&text)?;
        for e in entries {
            let hex = |k: &str| {
                e.get(k)
                    .and_then(|x| x.as_str())
                    .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            };
            let (Some(op), Some(des)) = (hex("opcode"), hex("deserialize")) else {
                continue;
            };
            let rva = des.wrapping_sub(image.info.image_base);
            let Some((b, end)) = image.function_range(rva) else {
                println!("{channel} {op:#x}: deserializer {des:#x} is not in .pdata");
                continue;
            };
            if end - b < fingerprint::MIN_LEN {
                println!(
                    "{channel} {op:#x}: deserializer {des:#x} is a {}-byte stub: no fingerprint",
                    end - b
                );
                continue;
            }
            let Some((fp, _)) = fingerprint::of_function(image, b as u32, end as u32) else {
                continue;
            };
            let count = all.get(&fp).map_or(1, Vec::len);
            let table = if channel == "login" {
                &protocol.login
            } else {
                &protocol.instance
            };
            let named = table
                .name(poe_proto::Direction::Server, op as u16)
                .unwrap_or("");
            lines.push(format!(
                "{channel}\t{op:#06x}\t{named}\t{fp}\t{count}\t{des:#x}"
            ));
        }
    }
    if lines.is_empty() {
        bail!(
            "no factory dump found in {}: run scripts/re/dump_factory.py first (docs/re/datamining.md)",
            ghidra.display()
        );
    }
    let path = v.opcodes_dir(root).join("deserializers.tsv");
    let mut out = format!(
        "# Deserializer fingerprints of client {} (build {}): the baseline of `cargo xtask check-build`.\n\
         # channel, opcode, name (server-to-client, from the opcode table), fingerprint of the class's\n\
         # deserializer (poe: xtask/src/fingerprint.rs), functions with that fingerprint, Ghidra address.\n\
         # Written by `cargo xtask check-build <exe> --write-fingerprints` from the factory dumps\n\
         # (scripts/re/dump_factory.py -> dumps/<version>/ghidra/*_factory*_s2c.json).\n",
        v.version, v.build_revision
    );
    for l in &lines {
        out.push_str(l);
        out.push('\n');
    }
    std::fs::write(&path, out)?;
    println!("wrote {} ({} opcodes)", path.display(), lines.len());
    Ok(0)
}

/// The folder with `Bundles2/`: `--bundles`, else the config's `[game] game_dir`.
fn game_dir(root: &Path, config: Option<&Config>, opts: &Options) -> PathBuf {
    match (&opts.bundles, config) {
        (Some(b), _) => {
            // The folder with Bundles2/, or Bundles2 itself.
            if b.file_name().is_some_and(|n| n == "Bundles2") {
                b.parent().unwrap_or(b).to_path_buf()
            } else {
                b.clone()
            }
        }
        (None, Some(c)) => root.join(&c.game.game_dir),
        (None, None) => root.join("GameFiles"),
    }
}

fn data(game_dir: &Path, root: &Path, config: Option<&Config>, v: &VersionFile, r: &mut Report) {
    let schema_path = config.map_or_else(
        || root.join("third_party/dat-schema/schema-poe2.min.json"),
        |c| root.join(&c.game.schema),
    );
    if !game_dir.join("Bundles2").is_dir() {
        r.add(
            DATA,
            "Bundles2",
            "not found",
            Level::Warn,
            format!(
                "{} has no Bundles2/: data checks skipped (--bundles <dir>)",
                game_dir.display()
            ),
        );
        return;
    }
    match std::fs::read(&schema_path) {
        Ok(bytes) => {
            let got = hex::encode(Sha256::digest(&bytes));
            match &v.data.schema_sha256 {
                Some(want) if want.eq_ignore_ascii_case(&got) => r.add(DATA, "dat schema", "pinned", Level::Ok, format!("sha256 {}...", &got[..16])),
                Some(want) => r.add(DATA, "dat schema", "DIFFERENT", Level::Warn, format!("{} has sha256 {got}, the version file pins {want}: column widths may differ; fetch the pinned schema or re-run the checks and update the pin", schema_path.display())),
                None => r.add(DATA, "dat schema", "not pinned", Level::Info, got),
            }
        }
        Err(e) => {
            r.add(
                DATA,
                "dat schema",
                "unreadable",
                Level::Fail,
                format!("{}: {e}", schema_path.display()),
            );
            return;
        }
    }
    let gd = match poe_data::GameData::open(game_dir, &schema_path) {
        Ok(g) => g,
        Err(e) => {
            r.add(
                DATA,
                "game data",
                "cannot open",
                Level::Fail,
                format!("{}: {e}", game_dir.display()),
            );
            return;
        }
    };
    for used in poe_data::used::USED {
        let table = match gd.table(used.name) {
            Ok(t) => t,
            Err(e) => {
                r.add(DATA, used.name, "unreadable", Level::Fail, e.to_string());
                continue;
            }
        };
        let mut problems = Vec::new();
        let schema_width = table.schema().row_width();
        if table.len() > 0 {
            if schema_width > table.row_width() {
                problems.push(format!("schema rows are {schema_width} bytes, the file's {}: trailing columns read garbage", table.row_width()));
            } else if schema_width < table.row_width() {
                // Not an error: the schema is older than the file or knows fewer columns.
                r.add(
                    DATA,
                    format!("{} row width", used.name),
                    "wider file",
                    Level::Info,
                    format!(
                        "file rows are {} bytes, the schema's {schema_width}",
                        table.row_width()
                    ),
                );
            }
        }
        for col in used.columns {
            match table.schema().column_index(col) {
                None => problems.push(format!("column `{col}` is not in the schema")),
                Some(i) => {
                    // Decode a few rows: wrong widths show as errors here.
                    let n = table.len();
                    for row in [0, 1, n / 2, n.saturating_sub(1)]
                        .iter()
                        .filter(|&&i| i < n)
                    {
                        if let Err(e) = table.row(*row).map(|x| x.get_index(i)).transpose() {
                            problems.push(format!("column `{col}` row {row}: {e}"));
                            break;
                        }
                    }
                    if table.schema().offset(i) + table.schema().columns[i].width()
                        > table.row_width()
                        && table.len() > 0
                    {
                        problems.push(format!("column `{col}` lies beyond the file's row width"));
                    }
                }
            }
        }
        for &i in used.indexed {
            if table.len() > 0 && i >= table.schema().columns.len() {
                problems.push(format!(
                    "indexed column {i} is beyond the schema's {} columns",
                    table.schema().columns.len()
                ));
            } else if let Some(row) = table.row(0)
                && let Err(e) = row.get_index(i)
            {
                problems.push(format!("indexed column {i}: {e}"));
            }
        }
        let rows_note = match v.data.row_counts.get(used.name) {
            Some(&want) if want != table.len() => Some(format!(
                "{} rows, the version file has {want} (a data patch adds rows; check the row anchors)",
                table.len()
            )),
            _ => None,
        };
        if problems.is_empty() {
            r.add(
                DATA,
                used.name,
                "ok",
                Level::Ok,
                format!(
                    "{} rows, {} columns used",
                    table.len(),
                    used.columns.len() + used.indexed.len()
                ),
            );
        } else {
            r.add(
                DATA,
                used.name,
                "mismatch",
                Level::Fail,
                problems.join("; "),
            );
        }
        if let Some(n) = rows_note {
            r.add(
                DATA,
                format!("{} row count", used.name),
                "changed",
                Level::Warn,
                n,
            );
        }
    }
    for a in &v.data.anchor {
        let item = format!("{}.{} = {:?} at row {}", a.table, a.column, a.value, a.row);
        let table = match gd.table(&a.table) {
            Ok(t) => t,
            Err(e) => {
                r.add(DATA, item, "unreadable", Level::Fail, e.to_string());
                continue;
            }
        };
        let at = table.row(a.row).and_then(|row| row.str(&a.column).ok());
        if at.as_deref() == Some(a.value.as_str()) {
            let mut level = Level::Ok;
            let mut detail = String::new();
            if let (Some(i), Some(want)) = (a.column_index, a.int) {
                let got = table
                    .row(a.row)
                    .and_then(|row| row.get_index(i).ok())
                    .and_then(|x| x.as_int());
                if got != Some(want) {
                    level = Level::Fail;
                    detail = format!("column {i} is {got:?}, expected {want}");
                } else {
                    detail = format!("column {i} = {want}");
                }
            }
            r.add(
                DATA,
                item,
                if level == Level::Ok {
                    "anchored"
                } else {
                    "DIFFERENT"
                },
                level,
                detail,
            );
        } else {
            let elsewhere = table.find_index_by_str(&a.column, &a.value).ok().flatten();
            r.add(
                DATA,
                item,
                "MOVED",
                Level::Fail,
                match elsewhere {
                    Some(i) => format!("now at row {i} (row {} holds {at:?}): the row index is a key, update the config and the anchor", a.row),
                    None => format!("not found (row {} holds {at:?})", a.row),
                },
            );
        }
    }
    if let Some(c) = config {
        let buffs = c.movement.sprint_buff_rows;
        if let Ok(table) = gd.table("BuffDefinitions") {
            for row in buffs {
                let id = table.row(usize::from(row)).and_then(|x| x.str("Id").ok());
                let anchored = v.data.anchor.iter().any(|a| {
                    a.table == "BuffDefinitions"
                        && a.row == usize::from(row)
                        && Some(&a.value) == id.as_ref()
                });
                match (&id, anchored) {
                    (Some(id), true) => r.add(
                        DATA,
                        format!("[movement] sprint_buff_rows {row}"),
                        "ok",
                        Level::Ok,
                        id.clone(),
                    ),
                    (Some(id), false) => r.add(
                        DATA,
                        format!("[movement] sprint_buff_rows {row}"),
                        "not anchored",
                        Level::Warn,
                        format!("row holds {id}, no [[data.anchor]] pins it"),
                    ),
                    (None, _) => r.add(
                        DATA,
                        format!("[movement] sprint_buff_rows {row}"),
                        "no such row",
                        Level::Fail,
                        format!("BuffDefinitions has {} rows", table.len()),
                    ),
                }
            }
        }
    }
    r.note(DATA, "Format versions of `.tgr`/`.dgr`/`.tdt`/`.ot` files are not checked (they are checked by the parsers, which read the version number).");
}

fn files(dir: &Path, v: &VersionFile, r: &mut Report) {
    if !dir.is_dir() {
        return;
    }
    for f in &v.verify.files {
        match std::fs::metadata(dir.join(&f.path)) {
            Ok(m) if m.len() == f.size => {
                r.add(FILES, &f.path, "size ok", Level::Ok, f.size.to_string())
            }
            Ok(m) => r.add(
                FILES,
                &f.path,
                "size DIFFERENT",
                Level::Fail,
                format!(
                    "{} bytes, versions/{}.toml says {}",
                    m.len(),
                    v.version,
                    f.size
                ),
            ),
            Err(e) => r.add(FILES, &f.path, "missing", Level::Warn, e.to_string()),
        }
    }
    r.note(
        FILES,
        format!(
            "Key files of {} (a data-only patch changes `Bundles2/_.index.bin`).",
            dir.display()
        ),
    );
}

fn markdown(
    exe: &Path,
    v: &VersionFile,
    file: &[u8],
    sha256: &str,
    image: &Image,
    table: Option<&Table>,
    r: &Report,
) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "# check-build: {}\n", exe.display());
    let _ = writeln!(
        o,
        "Checked against `versions/{}.toml` (status `{:?}`). Verdict: **{}**.\n",
        v.version,
        v.status,
        match r.worst() {
            Level::Fail => "FAIL (exit 2)",
            Level::Warn => "warnings (exit 1)",
            _ => "all fine (exit 0)",
        }
    );
    let _ = writeln!(o, "## Build fingerprint\n");
    let _ = writeln!(o, "| | exe | version file |\n|---|---|---|");
    let _ = writeln!(o, "| size | {} | {} |", file.len(), v.exe.size);
    let _ = writeln!(
        o,
        "| sha256 | `{sha256}` | `{}` |",
        v.exe_sha256().unwrap_or_else(|| "-".into())
    );
    let _ = writeln!(
        o,
        "| PE timestamp | `{:#x}` | `{:#x}` |",
        image.info.timestamp, v.exe.pe_timestamp
    );
    let _ = writeln!(
        o,
        "| image size | `{:#x}` | `{:#x}` |",
        image.info.size_of_image, v.exe.image_size
    );
    let rev = table.map(|t| {
        poe_hook_sig::resolve::check_build(t, image)
            .revision_found
            .unwrap_or_else(|| "not found".into())
    });
    let _ = writeln!(
        o,
        "| build revision | {} | {} |",
        rev.unwrap_or_else(|| "-".into()),
        v.build_revision
    );
    let _ = writeln!(o, "\n## Summary\n");
    let _ = writeln!(
        o,
        "| block | ok | info | warn | fail |\n|---|---|---|---|---|"
    );
    for b in BLOCKS {
        let count = |l: Level| {
            r.findings
                .iter()
                .filter(|f| f.block == b && f.level == l)
                .count()
        };
        let _ = writeln!(
            o,
            "| {b} | {} | {} | {} | {} |",
            count(Level::Ok),
            count(Level::Info),
            count(Level::Warn),
            count(Level::Fail)
        );
    }
    for b in BLOCKS {
        let rows: Vec<&Finding> = r.findings.iter().filter(|f| f.block == b).collect();
        if rows.is_empty() && !r.notes.contains_key(b) {
            continue;
        }
        let _ = writeln!(o, "\n## {b}\n");
        for n in r.notes.get(b).into_iter().flatten() {
            let _ = writeln!(o, "{n}\n");
        }
        if rows.is_empty() {
            continue;
        }
        let _ = writeln!(o, "| level | item | status | detail |\n|---|---|---|---|");
        for f in rows {
            let esc = |s: &str| s.replace('|', "\\|").replace('\n', " ");
            let _ = writeln!(
                o,
                "| {} | `{}` | {} | {} |",
                f.level.label(),
                esc(&f.item),
                esc(&f.status),
                esc(&f.detail)
            );
        }
    }
    o
}

/// Prints the summary and returns the exit code.
fn summary(r: &Report, report: &Path) -> i32 {
    println!();
    for b in BLOCKS {
        let rows: Vec<&Finding> = r.findings.iter().filter(|f| f.block == b).collect();
        if rows.is_empty() {
            continue;
        }
        let count = |l: Level| rows.iter().filter(|f| f.level == l).count();
        println!(
            "{b:<15} {:>4} ok  {:>3} info  {:>3} warn  {:>3} FAIL",
            count(Level::Ok),
            count(Level::Info),
            count(Level::Warn),
            count(Level::Fail)
        );
    }
    let problems: Vec<&Finding> = r
        .findings
        .iter()
        .filter(|f| f.level >= Level::Warn)
        .collect();
    if !problems.is_empty() {
        println!();
        for f in problems.iter().take(60) {
            println!(
                "{:<4} [{}] {}: {}: {}",
                f.level.label(),
                f.block,
                f.item,
                f.status,
                f.detail
            );
        }
        if problems.len() > 60 {
            println!("... and {} more in the report", problems.len() - 60);
        }
    }
    let worst = r.worst();
    println!("\nreport: {}", report.display());
    match worst {
        Level::Fail => {
            println!("result: FAIL (something required is missing, ambiguous or different)");
            2
        }
        Level::Warn => {
            println!("result: warnings");
            1
        }
        _ => {
            println!("result: all found, build matches");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_bytes_finds_the_first_occurrence() {
        assert_eq!(find_bytes(b"abcabc", b"ca"), Some(2));
        assert_eq!(find_bytes(b"abc", b"abcd"), None);
        assert_eq!(find_bytes(b"abc", b""), None);
        assert_eq!(find_bytes(b"aab", b"ab"), Some(1));
    }

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    fn opts(exe: &Path, report: &Path) -> Options {
        Options {
            exe: exe.to_path_buf(),
            bundles: None,
            version: None,
            report: Some(report.to_path_buf()),
            write_fingerprints: false,
        }
    }

    /// The real exe checks out (exit 0), and a copy with the first byte of one required
    /// function changed fails (exit 2) and names that function. Skipped without `GameFiles/`.
    #[test]
    fn the_real_exe_passes_and_a_mutated_copy_fails() {
        let root = root();
        let exe = root.join("GameFiles/PathOfExileSteam.exe");
        if !exe.is_file() {
            eprintln!("skipped: no GameFiles/PathOfExileSteam.exe");
            return;
        }
        let tmp = std::env::temp_dir().join(format!("check-build-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let config = Config::load_in(&root.join(poe_config::DEFAULT_PATH), &root).ok();
        let report = tmp.join("report.md");
        assert_eq!(
            run(&root, config.as_ref(), opts(&exe, &report)).unwrap(),
            0,
            "{}",
            std::fs::read_to_string(&report).unwrap_or_default()
        );
        assert!(
            std::fs::read_to_string(&report)
                .unwrap()
                .contains("| ok | `client_platform_detect` | found")
        );

        // Mutate the first byte of read_message (a required, `packets`, function).
        let mut file = std::fs::read(&exe).unwrap();
        let mapped = map_file(&file).unwrap();
        let image = Image::from_mapped(&mapped).unwrap();
        let version = VersionFile::load(&root, "0.5.5e").unwrap();
        let table = Table::parse(
            &std::fs::read_to_string(version.signatures_path(&root).unwrap()).unwrap(),
        )
        .unwrap();
        let rva = resolve(&table, &image, Some(&["packets"]))
            .rva("read_message")
            .unwrap() as u32;
        let section = image
            .info
            .sections
            .iter()
            .find(|s| (s.rva..s.rva + s.virtual_size).contains(&rva))
            .unwrap();
        file[(section.raw_offset + rva - section.rva) as usize] ^= 0xff;
        let mutated = tmp.join("mutated.exe");
        std::fs::write(&mutated, &file).unwrap();
        let report = tmp.join("mutated.md");
        assert_eq!(
            run(&root, config.as_ref(), opts(&mutated, &report)).unwrap(),
            2
        );
        let md = std::fs::read_to_string(&report).unwrap();
        assert!(md.contains("| FAIL | `read_message` | missing"), "{md}");
        assert!(md.contains("| FAIL | `exe sha256` | DIFFERENT"));
        assert!(md.contains("| FAIL | `packets` | OFF"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
