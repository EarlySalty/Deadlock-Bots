use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    #[serde(default)]
    pub tables: BTreeMap<String, TableLedger>,
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
    #[error("Ledger enthaelt keine Tabellen")]
    EmptyLedger,
    #[error("Ledger-Tabelle {table} enthaelt keine Spalten")]
    EmptyTableColumns { table: String },
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
