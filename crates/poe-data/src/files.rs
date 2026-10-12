//! Virtual file system: override directories layered over the client's `Bundles2`.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use poe_data_tools::fs::{FS, FileSystem};

use crate::{DataError, Result};

/// Reads game files by their virtual path (e.g. `data/balance/worldareas.datc64`).
///
/// Paths are case-insensitive (the bundle index stores them lowercased). Override
/// directories are searched first, newest first, which is how server mods replace files.
pub struct GameFiles {
    bundles: FS,
    overrides: Vec<PathBuf>,
}

impl GameFiles {
    pub fn open(game_dir: &Path) -> Result<Self> {
        let bundles = FS::from_steam(game_dir.to_owned()).map_err(|e| DataError::Open {
            path: game_dir.to_owned(),
            message: e.to_string(),
        })?;
        Ok(Self {
            bundles,
            overrides: Vec::new(),
        })
    }

    /// Adds a directory whose files shadow bundle files with the same virtual path.
    /// Later additions take priority.
    pub fn push_override(&mut self, dir: PathBuf) {
        self.overrides.push(dir);
    }

    pub fn read(&self, path: &str) -> Result<Bytes> {
        let path = normalize(path);
        for dir in self.overrides.iter().rev() {
            let candidate = dir.join(&path);
            if candidate.is_file() {
                return std::fs::read(&candidate)
                    .map(Bytes::from)
                    .map_err(|e| DataError::Read {
                        path: candidate.display().to_string(),
                        message: e.to_string(),
                    });
            }
        }
        self.bundles.read(&path).map_err(|e| {
            let message = e.to_string();
            if message.to_ascii_lowercase().contains("not found") {
                DataError::FileNotFound(path.clone())
            } else {
                DataError::Read { path, message }
            }
        })
    }

    /// All virtual paths in the bundle index (overrides are not listed).
    pub fn list(&self) -> impl Iterator<Item = String> + '_ {
        self.bundles.list()
    }
}

fn normalize(path: &str) -> String {
    path.trim_start_matches('/')
        .replace('\\', "/")
        .to_ascii_lowercase()
}
