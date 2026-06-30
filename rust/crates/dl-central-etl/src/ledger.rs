use std::{
    collections::{btree_map::Entry, BTreeMap},
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    #[serde(default)]
    pub tables: BTreeMap<String, TableLedger>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerSet {
    pub sources: BTreeMap<String, Ledger>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableLedger {
    #[serde(default)]
    pub columns: BTreeMap<String, ColumnStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ColumnStatus {
    Mapped { to: String },
    Dropped { reason: String },
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    #[error("Ledger-Verzeichnis {path} konnte nicht gelesen werden")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Ledger-Pfad {path} ist kein gueltiger UTF-8-Name")]
    NonUtf8Path { path: PathBuf },
    #[error("Ledger enthaelt keine Tabellen")]
    EmptyLedger,
    #[error("Ledger-Set enthaelt keine Quellen")]
    EmptyLedgerSet,
    #[error("Ledger-Tabelle {table} enthaelt keine Spalten")]
    EmptyTableColumns { table: String },
    #[error("Ledger-Quelle {source_db} definiert Tabelle {table} mehrfach")]
    DuplicateSourceTable { source_db: String, table: String },
    #[error("Ledger-Mapping fuer {table}.{column} hat ein leeres Ziel")]
    EmptyMappedTarget { table: String, column: String },
    #[error("Ledger-Drop fuer {table}.{column} hat eine leere Begruendung")]
    EmptyDropReason { table: String, column: String },
}

impl Ledger {
    pub fn from_toml_str(input: &str) -> Result<Self, LedgerError> {
        let ledger: Self = toml::from_str(input)?;
        ledger.validate()?;
        Ok(ledger)
    }

    pub fn table(&self, name: &str) -> Option<&TableLedger> {
        self.tables.get(name)
    }

    pub fn validate(&self) -> Result<(), LedgerError> {
        if self.tables.is_empty() {
            return Err(LedgerError::EmptyLedger);
        }

        for (table_name, table) in &self.tables {
            table.validate(table_name)?;
        }
        Ok(())
    }

    fn merge_source_fragment(&mut self, source: &str, fragment: Ledger) -> Result<(), LedgerError> {
        for (table_name, table_ledger) in fragment.tables {
            match self.tables.entry(table_name) {
                Entry::Vacant(entry) => {
                    entry.insert(table_ledger);
                }
                Entry::Occupied(entry) => {
                    return Err(LedgerError::DuplicateSourceTable {
                        source_db: source.to_string(),
                        table: entry.key().clone(),
                    });
                }
            }
        }

        Ok(())
    }
}

impl LedgerSet {
    pub fn from_dir(root: impl AsRef<Path>) -> Result<Self, LedgerError> {
        let root = root.as_ref();
        let mut set = Self {
            sources: BTreeMap::new(),
        };

        for source_entry in sorted_dir_entries(root)? {
            let source_path = source_entry.path();
            if !source_entry
                .file_type()
                .map_err(|source| LedgerError::Io {
                    path: source_path.clone(),
                    source,
                })?
                .is_dir()
            {
                continue;
            }

            let source_name = path_file_name(&source_path)?;
            let mut source_ledger = Ledger {
                tables: BTreeMap::new(),
            };

            for fragment_entry in sorted_dir_entries(&source_path)? {
                let fragment_path = fragment_entry.path();
                if !fragment_entry
                    .file_type()
                    .map_err(|source| LedgerError::Io {
                        path: fragment_path.clone(),
                        source,
                    })?
                    .is_file()
                {
                    continue;
                }

                if fragment_path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                    continue;
                }

                let contents =
                    fs::read_to_string(&fragment_path).map_err(|source| LedgerError::Io {
                        path: fragment_path.clone(),
                        source,
                    })?;
                let fragment = Ledger::from_toml_str(&contents)?;
                source_ledger.merge_source_fragment(&source_name, fragment)?;
            }

            if !source_ledger.tables.is_empty() {
                source_ledger.validate()?;
                set.sources.insert(source_name, source_ledger);
            }
        }

        if set.sources.is_empty() {
            return Err(LedgerError::EmptyLedgerSet);
        }

        Ok(set)
    }

    pub fn source(&self, name: &str) -> Option<&Ledger> {
        self.sources.get(name)
    }

    pub fn validate(&self) -> Result<(), LedgerError> {
        if self.sources.is_empty() {
            return Err(LedgerError::EmptyLedgerSet);
        }

        for ledger in self.sources.values() {
            ledger.validate()?;
        }

        Ok(())
    }
}

fn sorted_dir_entries(path: &Path) -> Result<Vec<fs::DirEntry>, LedgerError> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| LedgerError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| LedgerError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    entries.sort_by_key(|entry| entry.path());
    Ok(entries)
}

fn path_file_name(path: &Path) -> Result<String, LedgerError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| LedgerError::NonUtf8Path {
            path: path.to_path_buf(),
        })
}

impl TableLedger {
    pub fn column_status(&self, column: &str) -> Option<&ColumnStatus> {
        self.columns.get(column)
    }

    fn validate(&self, table_name: &str) -> Result<(), LedgerError> {
        if self.columns.is_empty() {
            return Err(LedgerError::EmptyTableColumns {
                table: table_name.to_string(),
            });
        }

        for (column_name, status) in &self.columns {
            match status {
                ColumnStatus::Mapped { to } if to.trim().is_empty() => {
                    return Err(LedgerError::EmptyMappedTarget {
                        table: table_name.to_string(),
                        column: column_name.to_string(),
                    });
                }
                ColumnStatus::Dropped { reason } if reason.trim().is_empty() => {
                    return Err(LedgerError::EmptyDropReason {
                        table: table_name.to_string(),
                        column: column_name.to_string(),
                    });
                }
                ColumnStatus::Mapped { .. } | ColumnStatus::Dropped { .. } => {}
            }
        }

        Ok(())
    }
}
