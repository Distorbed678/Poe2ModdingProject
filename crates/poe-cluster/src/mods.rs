//! What the web API needs from the server's mod host (`crates/poe-mod-host`): the mods offered
//! to clients (`GET /api/v1/server`, `GET /api/v1/mods/{id}/{version}.zip`) and the admin's
//! list of loaded mods. A trait so the API does not depend on the loader and tests can
//! substitute a mock.

/// One mod the server offers to clients (a client or both half), as advertised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModAdvert {
    pub id: String,
    pub name: String,
    pub version: String,
    /// `client` or `both` (a server-only mod is never advertised).
    pub side: String,
    /// SHA-256 of the package (lower-case hex).
    pub sha256: String,
    /// Size of the package in bytes.
    pub size: u64,
    /// The login check requires it (every advertised mod is required today).
    pub required: bool,
    pub hook_api: Option<u32>,
    pub game_versions: Vec<String>,
    /// The client half contains a native library (a DLL).
    pub contains_code: bool,
    pub description: String,
    pub authors: Vec<String>,
}

/// A package the server serves.
#[derive(Debug, Clone)]
pub struct ModPackage {
    pub sha256: String,
    pub bytes: Vec<u8>,
}

/// One row of the admin's mod list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModStatus {
    pub id: String,
    pub name: String,
    pub version: String,
    pub side: String,
    pub enabled: bool,
    /// `loaded`, `skipped`, `disabled` (enabled by the owner, not loaded), `failed`
    /// (disabled at run time after repeated callback failures) or `installed` (not enabled).
    pub state: String,
    /// Why the mod is not loaded, or a note.
    pub detail: String,
    /// Chat commands the mod registered.
    pub commands: Vec<String>,
}

pub trait ModDirectory: Send + Sync {
    /// The client-relevant mods (side client or both) that are enabled and have a package,
    /// in load order.
    fn adverts(&self) -> Vec<ModAdvert>;
    /// How many enabled mods are server-only (not offered to clients).
    fn server_only(&self) -> usize {
        0
    }
    /// The verified package bytes of an advertised mod; `None` for an unknown id or version.
    fn package(&self, id: &str, version: &str) -> Option<ModPackage>;
    /// The hash of an advertised package without reading it (for `ETag` / `304`).
    fn package_sha256(&self, id: &str, version: &str) -> Option<String> {
        self.package(id, version).map(|p| p.sha256)
    }
    /// Every mod in the mods folder with its state.
    fn status(&self) -> Vec<ModStatus>;
    /// The mod system is on (the mods folder has a `state.toml`).
    fn active(&self) -> bool {
        false
    }
    /// The mods folder changed since the host loaded it (a restart applies the change).
    fn restart_required(&self) -> bool {
        false
    }
}

/// A server without mods.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMods;

impl ModDirectory for NoMods {
    fn adverts(&self) -> Vec<ModAdvert> {
        Vec::new()
    }
    fn package(&self, _: &str, _: &str) -> Option<ModPackage> {
        None
    }
    fn status(&self) -> Vec<ModStatus> {
        Vec::new()
    }
}
