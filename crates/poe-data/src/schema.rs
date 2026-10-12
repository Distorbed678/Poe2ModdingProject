//! The community dat schema (`schema-poe2.min.json`, github.com/poe-tool-dev/dat-schema).

use std::{collections::HashMap, path::Path, sync::Arc};

use serde::Deserialize;

use crate::{DataError, Result};

/// `validFor` bit for Path of Exile 2.
const VALID_POE2: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    Bool,
    I16,
    U16,
    I32,
    U32,
    F32,
    String,
    /// Index into the same table (8 bytes).
    Row,
    /// Index into another table (16 bytes).
    ForeignRow,
    /// Index into an enumeration (4 bytes).
    EnumRow,
    /// Array of unknown element type (only valid with `array: true`).
    Array,
}

impl ColumnType {
    /// Size of one value inside a row (arrays always take 16 bytes: count + offset).
    pub fn width(self) -> usize {
        match self {
            Self::Bool => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 | Self::EnumRow => 4,
            Self::String | Self::Row => 8,
            Self::ForeignRow | Self::Array => 16,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ColumnSchema {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub ty: ColumnType,
    #[serde(default)]
    pub array: bool,
    #[serde(default)]
    pub interval: bool,
    #[serde(default)]
    pub references: Option<Reference>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reference {
    pub table: String,
}

impl ColumnSchema {
    /// Bytes this column occupies in the fixed-size row.
    pub fn width(&self) -> usize {
        if self.array {
            16
        } else if self.interval {
            self.ty.width() * 2
        } else {
            self.ty.width()
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawTable {
    name: String,
    #[serde(rename = "validFor")]
    valid_for: u32,
    columns: Vec<ColumnSchema>,
}

#[derive(Debug, Deserialize)]
struct RawSchema {
    version: u32,
    tables: Vec<RawTable>,
}

/// One table's layout with precomputed column offsets.
#[derive(Debug)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnSchema>,
    offsets: Vec<usize>,
    by_name: HashMap<String, usize>,
}

impl TableSchema {
    fn new(name: String, columns: Vec<ColumnSchema>) -> Self {
        let mut offsets = Vec::with_capacity(columns.len());
        let mut by_name = HashMap::new();
        let mut offset = 0;
        for (i, col) in columns.iter().enumerate() {
            offsets.push(offset);
            offset += col.width();
            if let Some(n) = &col.name {
                by_name.entry(n.clone()).or_insert(i);
            }
        }
        Self {
            name,
            columns,
            offsets,
            by_name,
        }
    }

    /// Column index by name; unnamed columns can be addressed as `"_<index>"`.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied().or_else(|| {
            name.strip_prefix('_')
                .and_then(|i| i.parse().ok())
                .filter(|&i: &usize| i < self.columns.len())
        })
    }

    pub fn offset(&self, column: usize) -> usize {
        self.offsets[column]
    }

    /// Bytes per row described by the schema (the file may have more, unknown trailing bytes).
    pub fn row_width(&self) -> usize {
        self.columns.iter().map(ColumnSchema::width).sum()
    }
}

pub struct Schema {
    pub version: u32,
    tables: HashMap<String, Arc<TableSchema>>,
}

impl Schema {
    pub fn load(path: &Path) -> Result<Self> {
        let err = |message: String| DataError::Schema {
            path: path.to_owned(),
            message,
        };
        let text = std::fs::read_to_string(path).map_err(|e| err(e.to_string()))?;
        Self::parse(&text).map_err(|e| err(e.to_string()))
    }

    pub fn parse(text: &str) -> serde_json::Result<Self> {
        let raw: RawSchema = serde_json::from_str(text)?;
        let tables = raw
            .tables
            .into_iter()
            .filter(|t| t.valid_for & VALID_POE2 != 0)
            .map(|t| {
                (
                    t.name.clone(),
                    Arc::new(TableSchema::new(t.name, t.columns)),
                )
            })
            .collect();
        Ok(Self {
            version: raw.version,
            tables,
        })
    }

    pub fn table(&self, name: &str) -> Option<&Arc<TableSchema>> {
        self.tables.get(name)
    }

    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }
}
