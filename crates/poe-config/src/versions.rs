//! The supported client builds: `versions/<version>.toml` at the repository root.
//!
//! One file per build is the single definition of what a supported build is: its depots and
//! manifests, the exe's checksum, size and PE fingerprint, the key-file sizes, the opcode
//! folder, the signature table, the client hashes the login server accepts and the data
//! anchors `cargo xtask check-build` verifies. The servers, `cargo xtask` and (through a
//! byte-identical copy kept by `cargo xtask versions`) the launcher read the same file.
//! Procedure for a new build: `docs/guides/new-game-version.md`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Folder of the version files, relative to the repository root (the working directory).
pub const DIR: &str = "versions";

/// What `[client] hashes` says about the `client_hash` a client sent at login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientCheck {
    /// The hash is a known value of the server's version.
    Known,
    /// The version lists no hash, or lists unconfirmed ones (`hashes_confirmed = false`):
    /// the client cannot be judged. Logged, accepted.
    Unverified,
    /// A confirmed table that does not contain the hash: another build.
    Unknown,
    /// As `Unknown`, but `[security] allow_unknown_client` lets it in (only
    /// `Config::client_check` returns this).
    UnknownAllowed,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionFile {
    pub version: String,
    #[serde(default)]
    pub status: Status,
    pub app_id: u32,
    /// `Build Revision:` of `Client.txt`.
    pub build_revision: String,
    pub exe: Exe,
    pub depots: Vec<Depot>,
    #[serde(default)]
    pub verify: Verify,
    pub protocol: Protocol,
    #[serde(default)]
    pub client_hook: ClientHook,
    #[serde(default)]
    pub client: Client,
    #[serde(default)]
    pub data: Data,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Supported,
    Experimental,
    Legacy,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Exe {
    pub name: String,
    /// Lowercase hex; absent or `TODO` = unknown.
    pub sha256: Option<String>,
    #[serde(default)]
    pub sha256_enforce: bool,
    pub size: u64,
    pub pe_timestamp: u32,
    pub image_size: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Depot {
    pub id: u32,
    /// Steam manifest id (a u64, kept as text).
    pub manifest: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Verify {
    #[serde(default)]
    pub min_files: u64,
    #[serde(default)]
    pub min_bytes: u64,
    #[serde(default)]
    pub files: Vec<KeyFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Protocol {
    /// Folder with `login.toml`, `instance.toml`, `hnc.toml`, `inventories.tsv`.
    pub opcodes: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClientHook {
    /// The signature table of the client DLL for this build.
    pub signatures: Option<PathBuf>,
    /// Signature-table features whose failure is fatal for `check-build` (exit 2).
    #[serde(default)]
    pub required_features: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Client {
    /// Known `client_hash` values (lowercase hex, 32 bytes).
    #[serde(default)]
    pub hashes: Vec<String>,
    /// The hashes were seen on the wire (otherwise they are derived from the code).
    #[serde(default)]
    pub hashes_confirmed: bool,
    /// Strings `check-build` must find in the exe.
    #[serde(default)]
    pub strings: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Data {
    /// SHA-256 of the dat schema file the checks were made with.
    pub schema_sha256: Option<String>,
    #[serde(default)]
    pub row_counts: std::collections::BTreeMap<String, usize>,
    #[serde(default)]
    pub anchor: Vec<Anchor>,
}

/// A row the server addresses by index: `table` has `value` in `column` at `row`.
#[derive(Debug, Clone, Deserialize)]
pub struct Anchor {
    pub table: String,
    #[serde(default = "default_column")]
    pub column: String,
    pub value: String,
    pub row: usize,
    /// With `int`: another (unnamed) column of that row, by index, must hold that integer.
    pub column_index: Option<usize>,
    pub int: Option<i64>,
}

fn default_column() -> String {
    "Id".into()
}

#[derive(Debug, thiserror::Error)]
pub enum VersionError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("game version {version:?} is not a supported build: no {path} (known: {known})")]
    Unknown {
        version: String,
        path: PathBuf,
        known: String,
    },
}

impl VersionFile {
    /// Parses and validates one file's text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let v: VersionFile = toml::from_str(text).map_err(|e| e.to_string())?;
        v.validate()?;
        Ok(v)
    }

    /// `<root>/versions/<version>.toml`.
    pub fn path(root: &Path, version: &str) -> PathBuf {
        root.join(DIR).join(format!("{version}.toml"))
    }

    /// Loads the file of `version` below the repository root `root`.
    pub fn load(root: &Path, version: &str) -> Result<Self, VersionError> {
        // The name becomes a path component: no separators.
        if version.is_empty() || version.contains(['/', '\\']) || version.contains("..") {
            return Err(VersionError::Unknown {
                version: version.to_owned(),
                path: Self::path(root, version),
                known: known_versions(root).join(", "),
            });
        }
        let path = Self::path(root, version);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VersionError::Unknown {
                    version: version.to_owned(),
                    path,
                    known: known_versions(root).join(", "),
                });
            }
            Err(source) => return Err(VersionError::Io { path, source }),
        };
        let v = Self::parse(&text).map_err(|message| VersionError::Invalid {
            path: path.clone(),
            message,
        })?;
        if v.version != version {
            return Err(VersionError::Invalid {
                path,
                message: format!(
                    "`version` is {:?}, the file name says {version:?}",
                    v.version
                ),
            });
        }
        Ok(v)
    }

    /// The newest supported build (highest `build_revision`; for tests and tools that need
    /// "a version" without the config).
    pub fn latest(root: &Path) -> Result<Self, VersionError> {
        let mut all: Vec<Self> = known_versions(root)
            .iter()
            .map(|v| Self::load(root, v))
            .collect::<Result<_, _>>()?;
        all.sort_by_key(|v| v.build_revision.parse::<u64>().unwrap_or(0));
        all.pop().ok_or_else(|| VersionError::Unknown {
            version: String::new(),
            path: root.join(DIR),
            known: String::new(),
        })
    }

    /// The opcode folder below the repository root.
    pub fn opcodes_dir(&self, root: &Path) -> PathBuf {
        root.join(&self.protocol.opcodes)
    }

    /// The signature table below the repository root.
    pub fn signatures_path(&self, root: &Path) -> Option<PathBuf> {
        self.client_hook.signatures.as_ref().map(|p| root.join(p))
    }

    /// `dumps/<version>`: generated and datamined data of this build (git-ignored).
    pub fn dumps_dir(&self, root: &Path) -> PathBuf {
        root.join("dumps").join(&self.version)
    }

    /// The exe checksum, or `None` when unknown (`TODO`, empty, absent).
    pub fn exe_sha256(&self) -> Option<String> {
        match self.exe.sha256.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(t) if t.eq_ignore_ascii_case("todo") => None,
            Some(t) => Some(t.to_ascii_lowercase()),
        }
    }

    /// Judges the `client_hash` of a `0x0003`/`0x0005`.
    pub fn check_client_hash(&self, hash: &[u8; 32]) -> ClientCheck {
        let hex = hex32(hash);
        if self
            .client
            .hashes
            .iter()
            .any(|h| h.eq_ignore_ascii_case(&hex))
        {
            ClientCheck::Known
        } else if self.client.hashes.is_empty() || !self.client.hashes_confirmed {
            ClientCheck::Unverified
        } else {
            ClientCheck::Unknown
        }
    }

    fn validate(&self) -> Result<(), String> {
        let hex_ok = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
        if self.version.trim().is_empty() || self.version.contains(['/', '\\']) {
            return Err("`version` is empty or contains a path separator".into());
        }
        if self.app_id == 0 {
            return Err("`app_id` is missing".into());
        }
        if self.build_revision.is_empty()
            || !self.build_revision.chars().all(|c| c.is_ascii_digit())
        {
            return Err("`build_revision` must be digits".into());
        }
        if self.depots.is_empty() {
            return Err("no depots listed".into());
        }
        let mut seen = std::collections::HashSet::new();
        for d in &self.depots {
            if d.id == 0 || !seen.insert(d.id) {
                return Err(format!("depot {} is missing or listed twice", d.id));
            }
            if d.manifest.parse::<u64>().is_err() {
                return Err(format!(
                    "depot {}: manifest {:?} is not a 64-bit number",
                    d.id, d.manifest
                ));
            }
        }
        if let Some(h) = self.exe_sha256()
            && !hex_ok(&h)
        {
            return Err("`exe.sha256` must be 64 hex digits, TODO or absent".into());
        }
        for h in &self.client.hashes {
            if !hex_ok(h) {
                return Err(format!("`client.hashes` entry {h:?} is not 64 hex digits"));
            }
        }
        if let Some(h) = &self.data.schema_sha256
            && !hex_ok(h)
        {
            return Err("`data.schema_sha256` must be 64 hex digits".into());
        }
        Ok(())
    }
}

/// Lowercase hex of a 32-byte hash.
pub fn hex32(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// Versions that have a file in `<root>/versions/`, sorted by name.
pub fn known_versions(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root.join(DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "toml").then(|| p.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    #[test]
    fn the_repo_files_parse_and_name_their_version() {
        let root = Path::new(REPO);
        let known = known_versions(root);
        assert!(!known.is_empty());
        for v in &known {
            let f = VersionFile::load(root, v).unwrap();
            assert_eq!(&f.version, v);
            assert!(
                f.opcodes_dir(root).join("login.toml").exists(),
                "{v}: opcodes folder"
            );
            if let Some(sig) = f.signatures_path(root) {
                // A submodule that is not checked out is not an error here.
                assert!(
                    sig.exists() || !root.join("modules/client-hook/Cargo.toml").exists(),
                    "{v}: {sig:?}"
                );
            }
        }
        let latest = VersionFile::latest(root).unwrap();
        assert_eq!(latest.version, "0.5.5e");
        assert_eq!(latest.app_id, 2694490);
        assert_eq!(latest.exe.pe_timestamp, 0x6ac302e0);
    }

    #[test]
    fn unknown_version_names_the_known_ones() {
        let e = VersionFile::load(Path::new(REPO), "9.9.9z")
            .unwrap_err()
            .to_string();
        assert!(e.contains("9.9.9z") && e.contains("0.5.5e"), "{e}");
        assert!(VersionFile::load(Path::new(REPO), "../Cargo").is_err());
    }

    #[test]
    fn client_hash_judgement() {
        let mut v = VersionFile::latest(Path::new(REPO)).unwrap();
        let known: [u8; 32] = hex_to_32(&v.client.hashes[0]);
        let other = [7u8; 32];
        // Derived, not yet confirmed: nothing is refused.
        v.client.hashes_confirmed = false;
        assert_eq!(v.check_client_hash(&known), ClientCheck::Known);
        assert_eq!(v.check_client_hash(&other), ClientCheck::Unverified);
        v.client.hashes_confirmed = true;
        assert_eq!(v.check_client_hash(&known), ClientCheck::Known);
        assert_eq!(v.check_client_hash(&other), ClientCheck::Unknown);
        // No hash recorded at all: cannot judge.
        v.client.hashes.clear();
        assert_eq!(v.check_client_hash(&other), ClientCheck::Unverified);
    }

    fn hex_to_32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn bad_files_are_rejected() {
        assert!(VersionFile::parse("version = \"1\"").is_err());
        let base = |extra: &str| {
            format!(
                "version = \"1\"\napp_id = 5\nbuild_revision = \"7\"\n[exe]\nname = \"x\"\nsize = 1\npe_timestamp = 1\nimage_size = 1\n[[depots]]\nid = 6\nmanifest = \"123\"\n[protocol]\nopcodes = \"p\"\n{extra}"
            )
        };
        assert!(VersionFile::parse(&base("")).is_ok());
        assert!(VersionFile::parse(&base("[client]\nhashes = [\"zz\"]\n")).is_err());
        assert!(VersionFile::parse(&base("[client]\nhashes = [\"zz\"]\n")).is_err());
        assert!(VersionFile::parse(&base("").replace("123", "12x")).is_err());
    }
}
