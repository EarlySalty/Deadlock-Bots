use std::{collections::BTreeMap, path::Path};

use rusqlite::{types::ValueRef, Connection, OpenFlags};
use serde_json::{Number, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceColumn {
    pub name: String,
    pub declared_type: String,
    pub affinity: SqliteAffinity,
    pub primary_key_ordinal: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqliteAffinity {
    Integer,
    Text,
    Real,
    Numeric,
    Blob,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceRow {
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("nicht-finites SQLite-REAL: {value}")]
    NonFiniteReal { value: f64 },
    #[error("ungueltiger UTF-8-Text in SQLite-TEXT")]
    InvalidUtf8Text,
}

pub struct SourceSqlite {
    conn: Connection,
}

impl SourceSqlite {
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, SourceError> {
        // Echte ETL liest immer eine SNAPSHOT-Kopie, nie das SQLite-Original.
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        Ok(Self { conn })
    }

    pub fn table_columns(&self, table: &str) -> Result<Vec<String>, SourceError> {
        Ok(self
            .table_schema(table)?
            .into_iter()
            .map(|column| column.name)
            .collect())
    }

    pub fn table_schema(&self, table: &str) -> Result<Vec<SourceColumn>, SourceError> {
        let sql = format!("PRAGMA table_info({})", quote_sqlite_ident(table));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let name = row.get::<_, String>(1)?;
            let declared_type = row.get::<_, Option<String>>(2)?.unwrap_or_default();
            let affinity = SqliteAffinity::from_declared_type(&declared_type);
            let primary_key_ordinal = row.get::<_, i64>(5)?;

            Ok(SourceColumn {
                name,
                declared_type,
                affinity,
                primary_key_ordinal,
            })
        })?;

        let mut columns = Vec::new();
        for column in rows {
            columns.push(column?);
        }

        Ok(columns)
    }

    pub fn read_rows(&self, table: &str) -> Result<Vec<SourceRow>, SourceError> {
        let sql = format!("SELECT * FROM {}", quote_sqlite_ident(table));
        let mut stmt = self.conn.prepare(&sql)?;
        let columns = stmt
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut values = BTreeMap::new();

            for (index, column) in columns.iter().enumerate() {
                values.insert(
                    column.clone(),
                    sqlite_value_ref_to_json(row.get_ref(index)?)?,
                );
            }

            out.push(SourceRow { values });
        }

        Ok(out)
    }
}

impl SqliteAffinity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Integer => "INTEGER",
            Self::Text => "TEXT",
            Self::Real => "REAL",
            Self::Numeric => "NUMERIC",
            Self::Blob => "BLOB",
        }
    }

    fn from_declared_type(declared_type: &str) -> Self {
        let upper = declared_type.to_ascii_uppercase();
        if upper.contains("INT") {
            return Self::Integer;
        }

        if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
            return Self::Text;
        }

        if upper.contains("BLOB") || upper.trim().is_empty() {
            return Self::Blob;
        }

        if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
            return Self::Real;
        }

        Self::Numeric
    }
}

fn quote_sqlite_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn sqlite_value_ref_to_json(value: ValueRef<'_>) -> Result<Value, SourceError> {
    match value {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(value) => Ok(Value::Number(Number::from(value))),
        ValueRef::Real(value) => {
            if !value.is_finite() {
                return Err(SourceError::NonFiniteReal { value });
            }

            Number::from_f64(value)
                .map(Value::Number)
                .ok_or(SourceError::NonFiniteReal { value })
        }
        ValueRef::Text(value) => std::str::from_utf8(value)
            .map(|text| Value::String(text.to_string()))
            .map_err(|_| SourceError::InvalidUtf8Text),
        ValueRef::Blob(value) => Ok(Value::Array(
            value
                .iter()
                .map(|byte| Value::Number(Number::from(u64::from(*byte))))
                .collect(),
        )),
    }
}
