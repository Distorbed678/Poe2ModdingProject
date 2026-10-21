//! Game data access for the server.
//!
//! [`GameData`] reads files straight from the client's `Bundles2` (through `poe_data_tools`),
//! with an optional stack of override directories in front of it so mods can replace
//! individual files. Tables (`.datc64`) are decoded with the community `dat-schema`, and
//! columns are addressed by name, so code never depends on raw column offsets.

mod dat;
mod error;
mod files;
pub mod hash;
pub mod rooms;
mod schema;
pub mod template;
pub mod used;
pub mod views;

pub use dat::{Row, Table, Value};
pub use error::{DataError, Result};
pub use files::GameFiles;
pub use schema::{ColumnSchema, ColumnType, Schema, TableSchema};

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

/// Directory (inside the bundle file system) holding the base-language tables.
pub const TABLE_DIR: &str = "data/balance";

/// Entry point for all game data lookups. Cheap to share via `Arc`.
pub struct GameData {
    files: GameFiles,
    schema: Schema,
    tables: Mutex<HashMap<String, Arc<Table>>>,
}

impl GameData {
    /// Opens the game install at `game_dir` (the folder containing `Bundles2/`) and loads the
    /// dat schema JSON at `schema_path`.
    pub fn open(game_dir: &Path, schema_path: &Path) -> Result<Self> {
        Ok(Self {
            files: GameFiles::open(game_dir)?,
            schema: Schema::load(schema_path)?,
            tables: Mutex::new(HashMap::new()),
        })
    }

    pub fn files(&self) -> &GameFiles {
        &self.files
    }

    pub fn files_mut(&mut self) -> &mut GameFiles {
        &mut self.files
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Loads (and caches) a table by its schema name, e.g. `"WorldAreas"`.
    pub fn table(&self, name: &str) -> Result<Arc<Table>> {
        if let Some(t) = self.tables.lock().unwrap().get(name) {
            return Ok(t.clone());
        }
        let schema = self
            .schema
            .table(name)
            .ok_or_else(|| DataError::UnknownTable(name.to_owned()))?;
        let path = format!("{TABLE_DIR}/{}.datc64", name.to_ascii_lowercase());
        let bytes = self.files.read(&path)?;
        let table = Arc::new(Table::parse(name, bytes, schema.clone())?);
        self.tables
            .lock()
            .unwrap()
            .insert(name.to_owned(), table.clone());
        Ok(table)
    }
}
