//! Key-Value-Repository über die Bestands-Tabelle `kv_store`.
//!
//! Vertrag mit dem Python-Original (service/db.py: get_kv/set_kv):
//! `kv_store(ns TEXT, k TEXT, v TEXT, PRIMARY KEY(ns, k))` — Werte sind
//! rohe Strings (häufig JSON), die Interpretation gehört den Domänen.

use rusqlite::OptionalExtension;

use crate::{Db, DbError};

#[derive(Debug, thiserror::Error)]
pub enum KvError {
    #[error(transparent)]
    Db(#[from] DbError),
}

impl Db {
    /// Liest einen Wert; `None` wenn der Schlüssel nicht existiert.
    pub async fn kv_get(
        &self,
        ns: impl Into<String>,
        key: impl Into<String>,
    ) -> Result<Option<String>, KvError> {
        let (ns, key) = (ns.into(), key.into());
        let value = self
            .read(move |conn| {
                conn.query_row(
                    "SELECT v FROM kv_store WHERE ns = ?1 AND k = ?2",
                    (&ns, &key),
                    |row| row.get(0),
                )
                .optional()
            })
            .await?;
        Ok(value)
    }

    /// Setzt einen Wert (Insert oder Update, wie Python set_kv).
    pub async fn kv_set(
        &self,
        ns: impl Into<String>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), KvError> {
        let (ns, key, value) = (ns.into(), key.into(), value.into());
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO kv_store(ns, k, v) VALUES (?1, ?2, ?3)
                 ON CONFLICT(ns, k) DO UPDATE SET v = excluded.v",
                (&ns, &key, &value),
            )
            .map(|_| ())
        })
        .await?;
        Ok(())
    }

    /// Löscht einen Schlüssel; meldet zurück, ob er existierte.
    pub async fn kv_delete(
        &self,
        ns: impl Into<String>,
        key: impl Into<String>,
    ) -> Result<bool, KvError> {
        let (ns, key) = (ns.into(), key.into());
        let affected = self
            .write(move |conn| {
                conn.execute("DELETE FROM kv_store WHERE ns = ?1 AND k = ?2", (&ns, &key))
            })
            .await?;
        Ok(affected > 0)
    }
}

#[cfg(test)]
mod tests {
    use crate::Db;

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
    async fn get_auf_unbekannten_schluessel_ist_none() {
        let (_dir, db) = test_db().await;
        assert_eq!(db.kv_get("ns", "fehlt").await.expect("get"), None);
    }

    #[tokio::test]
    async fn set_get_update_roundtrip() {
        let (_dir, db) = test_db().await;
        db.kv_set("panel", "message_id", "123").await.expect("set");
        assert_eq!(
            db.kv_get("panel", "message_id").await.expect("get"),
            Some("123".to_string())
        );

        // Upsert: zweites Set überschreibt
        db.kv_set("panel", "message_id", "456").await.expect("set");
        assert_eq!(
            db.kv_get("panel", "message_id").await.expect("get"),
            Some("456".to_string())
        );
    }

    #[tokio::test]
    async fn namespaces_sind_getrennt() {
        let (_dir, db) = test_db().await;
        db.kv_set("a", "k", "wert-a").await.expect("set");
        db.kv_set("b", "k", "wert-b").await.expect("set");
        assert_eq!(
            db.kv_get("a", "k").await.expect("get"),
            Some("wert-a".to_string())
        );
        assert_eq!(
            db.kv_get("b", "k").await.expect("get"),
            Some("wert-b".to_string())
        );
    }

    #[tokio::test]
    async fn delete_meldet_existenz() {
        let (_dir, db) = test_db().await;
        db.kv_set("ns", "k", "v").await.expect("set");
        assert!(db.kv_delete("ns", "k").await.expect("delete"));
        assert!(!db.kv_delete("ns", "k").await.expect("delete"));
        assert_eq!(db.kv_get("ns", "k").await.expect("get"), None);
    }
}
