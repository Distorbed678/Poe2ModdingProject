use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("opening game files at {path}: {message}")]
    Open { path: PathBuf, message: String },
    #[error("file not found in bundles or overrides: {0}")]
    FileNotFound(String),
    #[error("reading {path}: {message}")]
    Read { path: String, message: String },
    #[error("loading schema {path}: {message}")]
    Schema { path: PathBuf, message: String },
    #[error("table {0} is not in the schema")]
    UnknownTable(String),
    #[error("table {table}: column {column} not found")]
    UnknownColumn { table: String, column: String },
    #[error("table {table}: malformed data: {message}")]
    Malformed { table: String, message: String },
}

pub type Result<T> = std::result::Result<T, DataError>;
