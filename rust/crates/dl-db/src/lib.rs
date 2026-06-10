//! SQLite-Zugriffsschicht auf die gemeinsame `data/deadlock.sqlite3`.
//!
//! Diese DB ist während der gesamten Strangler-Fig-Migration der Vertrag
//! zwischen Python-Original, Rust-Steam-Bot und diesem Rewrite
//! (siehe rust/docs/01-db-contract.md). Daraus folgen zwei harte Regeln:
//!
//! 1. **Kein Schema-Besitz vor Cutover.** Diese Crate legt keine Tabellen
//!    an und ändert keine — das macht weiterhin der Python-Bot, bis die
//!    jeweilige Domäne nach Rust geschnitten ist.
//! 2. **Kein stilles Neuanlegen.** [`Db::open`] schlägt fehl, wenn die
//!    Datei nicht existiert. Eine leere DB am falschen Pfad wäre der
//!    klassische Foot-Gun (vgl. Steam-Bot: "DB-Pfad NIE Code-Default").
//!
//! Nebenläufigkeitsmodell (WAL):
//! - Schreiben: genau EINE Writer-Verbindung, serialisiert über ein Mutex,
//!   ausgeführt auf Blocking-Threads.
//! - Lesen: pro Aufruf eine frische Read-only-Verbindung — unter WAL
//!   blockieren Leser weder den Writer noch einander.

mod kv;

pub use kv::KvError;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("DB-Datei nicht gefunden: {0} (Pfad prüfen — niemals stillschweigend neu anlegen)")]
    NotFound(PathBuf),
    #[error("SQLite-Fehler: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("DB-Task abgebrochen: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// Handle auf die gemeinsame SQLite-DB. Billig klonbar.
#[derive(Clone)]
pub struct Db {
    /// Einziger Schreibkanal — alle Writes laufen seriell hierüber.
    writer: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").field("path", &self.path).finish()
    }
}

impl Db {
    /// Öffnet eine EXISTIERENDE Datenbank. Fehlt die Datei, ist das ein
    /// Konfigurationsfehler und kein Grund, eine leere DB anzulegen.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(DbError::NotFound(path.to_path_buf()));
        }
        Self::open_unchecked(path)
    }

    /// Öffnet bzw. erstellt eine Datenbank. NUR für Tests und Werkzeuge —
    /// im Produktionspfad immer [`Db::open`] verwenden.
    pub fn open_creating(path: impl AsRef<Path>) -> Result<Self, DbError> {
        Self::open_unchecked(path.as_ref())
    }

    fn open_unchecked(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        // WAL ist in der Bestands-DB bereits aktiv; das Pragma ist idempotent
        // und stellt bei Test-DBs denselben Modus sicher.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        Ok(Self {
            writer: Arc::new(Mutex::new(conn)),
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Führt eine Schreiboperation seriell über die Writer-Verbindung aus.
    /// Die Closure läuft auf einem Blocking-Thread und darf blockieren.
    pub async fn write<T, F>(&self, f: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, rusqlite::Error> + Send + 'static,
    {
        let writer = Arc::clone(&self.writer);
        let result = tokio::task::spawn_blocking(move || {
            let mut conn = writer.lock().unwrap_or_else(|poisoned| {
                // Ein Panic in einer früheren Write-Closure vergiftet das Mutex.
                // Die Verbindung selbst ist davon unberührt — weiterverwenden.
                poisoned.into_inner()
            });
            f(&mut conn)
        })
        .await??;
        Ok(result)
    }

    /// Führt eine Leseoperation auf einer frischen Read-only-Verbindung aus.
    pub async fn read<T, F>(&self, f: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
    {
        let path = self.path.clone();
        let result = tokio::task::spawn_blocking(move || {
            let conn = Connection::open_with_flags(
                &path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            conn.busy_timeout(BUSY_TIMEOUT)?;
            f(&conn)
        })
        .await??;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DDL exakt wie in der Produktions-DB (Vertrag, siehe docs/db-schema.sql).
    const KV_DDL: &str = "CREATE TABLE kv_store(
              ns TEXT NOT NULL,
              k  TEXT NOT NULL,
              v  TEXT NOT NULL,
              PRIMARY KEY(ns, k)
            )";

    async fn test_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("test.sqlite3")).expect("open_creating");
        db.write(|c| c.execute(KV_DDL, []).map(|_| ()))
            .await
            .expect("kv_store anlegen");
        (dir, db)
    }

    #[tokio::test]
    async fn open_verweigert_fehlende_datei() {
        let err = Db::open("/nirgendwo/gibt-es-diese.sqlite3").expect_err("muss fehlschlagen");
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn write_und_read_roundtrip() {
        let (_dir, db) = test_db().await;
        db.write(|c| {
            c.execute(
                "INSERT INTO kv_store(ns, k, v) VALUES('test', 'a', '1')",
                [],
            )
            .map(|_| ())
        })
        .await
        .expect("insert");

        let value: String = db
            .read(|c| {
                c.query_row(
                    "SELECT v FROM kv_store WHERE ns='test' AND k='a'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("select");
        assert_eq!(value, "1");
    }

    #[tokio::test]
    async fn read_verbindung_ist_wirklich_readonly() {
        let (_dir, db) = test_db().await;
        let err = db
            .read(|c| {
                c.execute("INSERT INTO kv_store(ns, k, v) VALUES('x', 'y', 'z')", [])
                    .map(|_| ())
            })
            .await
            .expect_err("Schreiben über read() muss scheitern");
        assert!(matches!(err, DbError::Sqlite(_)));
    }

    #[tokio::test]
    async fn wal_modus_ist_aktiv() {
        let (_dir, db) = test_db().await;
        let mode: String = db
            .read(|c| c.query_row("PRAGMA journal_mode", [], |row| row.get(0)))
            .await
            .expect("pragma");
        assert_eq!(mode.to_lowercase(), "wal");
    }
}
