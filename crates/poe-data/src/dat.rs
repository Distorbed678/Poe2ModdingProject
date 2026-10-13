//! `.datc64` table decoding.
//!
//! Layout: `u32 row_count`, then `row_count` fixed-width rows, then the variable-data section
//! which starts with eight `0xBB` bytes. Values are little-endian; strings and arrays in rows
//! are offsets into the variable section (relative to the `0xBB` marker). Null row
//! references are `0xFEFE…`.

use std::sync::Arc;

use bytes::Bytes;

use crate::{ColumnType, DataError, Result, TableSchema};

const MAGIC: [u8; 8] = [0xbb; 8];
const NULL_ROW: u64 = 0xfefe_fefe_fefe_fefe;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f32),
    String(String),
    /// Row index into this or another table.
    Row(usize),
    List(Vec<Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(v) => Some(*v),
            Self::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Int(v) => Some(*v != 0),
            _ => None,
        }
    }

    pub fn as_row(&self) -> Option<usize> {
        match self {
            Self::Row(r) => Some(*r),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(v) => Some(v),
            _ => None,
        }
    }
}

pub struct Table {
    name: String,
    data: Bytes,
    rows: usize,
    width: usize,
    /// Byte offset of the variable section (the `0xBB` marker).
    var_start: usize,
    schema: Arc<TableSchema>,
}

impl Table {
    pub fn parse(name: &str, data: Bytes, schema: Arc<TableSchema>) -> Result<Self> {
        let malformed = |message: String| DataError::Malformed {
            table: name.to_owned(),
            message,
        };
        if data.len() < 4 {
            return Err(malformed("file shorter than header".into()));
        }
        let rows = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
        let var_start = if rows == 0 {
            data[4..]
                .windows(8)
                .position(|w| w == MAGIC)
                .map(|p| p + 4)
                .ok_or_else(|| malformed("missing variable-data marker".into()))?
        } else {
            // Rows never contain the marker at a row boundary that also satisfies the size
            // equation, so take the first marker whose offset divides evenly into rows.
            (4..data.len().saturating_sub(7))
                .find(|&p| (p - 4) % rows == 0 && data[p..p + 8] == MAGIC)
                .ok_or_else(|| malformed("missing variable-data marker".into()))?
        };
        let width = if rows == 0 { 0 } else { (var_start - 4) / rows };
        if rows > 0 && schema.row_width() > width {
            tracing::warn!(
                table = name,
                schema_width = schema.row_width(),
                file_width = width,
                "schema row is wider than the file; trailing columns are unreadable"
            );
        } else if rows > 0 && schema.row_width() < width {
            tracing::debug!(
                table = name,
                schema_width = schema.row_width(),
                file_width = width,
                "file rows have unknown trailing bytes"
            );
        }
        Ok(Self {
            name: name.to_owned(),
            data,
            rows,
            width,
            var_start,
            schema,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn len(&self) -> usize {
        self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// Bytes per row in the file.
    pub fn row_width(&self) -> usize {
        self.width
    }

    pub fn schema(&self) -> &TableSchema {
        &self.schema
    }

    pub fn row(&self, index: usize) -> Option<Row<'_>> {
        (index < self.rows).then_some(Row { table: self, index })
    }

    pub fn rows(&self) -> impl Iterator<Item = Row<'_>> {
        (0..self.rows).map(|index| Row { table: self, index })
    }

    /// First row whose string column `column` equals `value`.
    pub fn find_by_str(&self, column: &str, value: &str) -> Result<Option<Row<'_>>> {
        let col = self.column_index(column)?;
        for row in self.rows() {
            if row.get_index(col)?.as_str() == Some(value) {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    /// Index of the first row whose string column equals `value`.
    pub fn find_index_by_str(&self, column: &str, value: &str) -> Result<Option<usize>> {
        let col = self.column_index(column)?;
        for (i, row) in self.rows().enumerate() {
            if row.get_index(col)?.as_str() == Some(value) {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    fn column_index(&self, column: &str) -> Result<usize> {
        self.schema
            .column_index(column)
            .ok_or_else(|| DataError::UnknownColumn {
                table: self.name.clone(),
                column: column.to_owned(),
            })
    }

    fn malformed(&self, message: String) -> DataError {
        DataError::Malformed {
            table: self.name.clone(),
            message,
        }
    }

    fn var(&self, offset: u64) -> Result<&[u8]> {
        let start = self.var_start + offset as usize;
        if offset < 8 || start > self.data.len() {
            return Err(self.malformed(format!("variable offset {offset} out of range")));
        }
        Ok(&self.data[start..])
    }

    fn read_string(&self, offset: u64) -> Result<String> {
        let bytes = self.var(offset)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        String::from_utf16(&units).map_err(|_| self.malformed("invalid utf-16 string".into()))
    }

    /// Decodes one scalar of type `ty` from `bytes` (which starts at the value).
    fn read_scalar(&self, ty: ColumnType, bytes: &[u8]) -> Result<Value> {
        let u64_at = |o: usize| u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        Ok(match ty {
            ColumnType::Bool => Value::Bool(bytes[0] != 0),
            ColumnType::I16 => Value::Int(i16::from_le_bytes([bytes[0], bytes[1]]) as i64),
            ColumnType::U16 => Value::Int(u16::from_le_bytes([bytes[0], bytes[1]]) as i64),
            ColumnType::I32 => {
                Value::Int(i32::from_le_bytes(bytes[..4].try_into().unwrap()) as i64)
            }
            ColumnType::U32 => {
                Value::Int(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as i64)
            }
            ColumnType::EnumRow => {
                Value::Row(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize)
            }
            ColumnType::F32 => Value::Float(f32::from_le_bytes(bytes[..4].try_into().unwrap())),
            ColumnType::String => Value::String(self.read_string(u64_at(0))?),
            ColumnType::Row | ColumnType::ForeignRow => match u64_at(0) {
                NULL_ROW => Value::Null,
                r => Value::Row(r as usize),
            },
            ColumnType::Array => Value::Null,
        })
    }
}

/// A borrowed view of one table row.
#[derive(Clone, Copy)]
pub struct Row<'a> {
    table: &'a Table,
    index: usize,
}

impl<'a> Row<'a> {
    pub fn index(&self) -> usize {
        self.index
    }

    /// Raw bytes of the whole row.
    pub fn raw(&self) -> &'a [u8] {
        let start = 4 + self.index * self.table.width;
        &self.table.data[start..start + self.table.width]
    }

    /// Value of a column by schema name (or `"_<n>"` for unnamed column `n`).
    pub fn get(&self, column: &str) -> Result<Value> {
        self.get_index(self.table.column_index(column)?)
    }

    pub fn get_index(&self, column: usize) -> Result<Value> {
        let t = self.table;
        let col = &t.schema.columns[column];
        let offset = t.schema.offset(column);
        let raw = self.raw();
        if offset + col.width() > raw.len() {
            return Err(t.malformed(format!(
                "column {column} ({:?}) lies outside the {}-byte row",
                col.name,
                raw.len()
            )));
        }
        let bytes = &raw[offset..offset + col.width()];
        if col.array {
            let count = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
            if count == 0 || col.ty == ColumnType::Array {
                return Ok(Value::List(Vec::new()));
            }
            let items = t.var(u64::from_le_bytes(bytes[8..].try_into().unwrap()))?;
            let w = col.ty.width();
            if items.len() < count * w {
                return Err(t.malformed(format!("array of {count} overruns data")));
            }
            return (0..count)
                .map(|i| t.read_scalar(col.ty, &items[i * w..]))
                .collect::<Result<_>>()
                .map(Value::List);
        }
        if col.interval {
            let w = col.ty.width();
            return Ok(Value::List(vec![
                t.read_scalar(col.ty, bytes)?,
                t.read_scalar(col.ty, &bytes[w..])?,
            ]));
        }
        t.read_scalar(col.ty, bytes)
    }

    pub fn str(&self, column: &str) -> Result<String> {
        Ok(self.get(column)?.as_str().unwrap_or_default().to_owned())
    }

    pub fn int(&self, column: &str) -> Result<i64> {
        Ok(self.get(column)?.as_int().unwrap_or_default())
    }

    pub fn bool(&self, column: &str) -> Result<bool> {
        Ok(self.get(column)?.as_bool().unwrap_or_default())
    }

    /// Referenced row index, `None` for a null reference.
    pub fn row_ref(&self, column: &str) -> Result<Option<usize>> {
        Ok(self.get(column)?.as_row())
    }
}
