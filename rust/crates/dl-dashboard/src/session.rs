//! Persistenter Store der Dashboard-Sessions (`master_dash_session`).
//!
//! Anders als die HMAC-signierten Stats-/Tierlist-Cookies (siehe
//! `dl-webcore::SessionCodec`) ist eine Dashboard-Session ein **opakes
//! Zufallstoken**. Ein kleiner In-Memory-Cache hält den Hot Path schnell;
//! `kv_store` ist die verbindliche Quelle über Prozessneustarts hinweg.
//! Bei jedem Zugriff verlängert sich die Gültigkeit (gleitende TTL).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::config::AccessLevel;
use crate::token;

const SESSION_KV_NAMESPACE: &str = "dl_dashboard_admin_session";

/// Grund, warum eine Session ausgestellt wurde (Diagnose/Audit, wie im
/// Original z. B. `owner_override`, `guild_admin:<id>`, `moderator_role:<id>`).
pub type SessionReason = String;

#[derive(Debug, Clone)]
pub struct Session {
    pub user_id: u64,
    pub username: String,
    pub display_name: String,
    pub reason: SessionReason,
    pub access_level: AccessLevel,
    pub csrf_token: String,
    pub created_at: f64,
    pub last_seen_at: f64,
    pub expires_at: f64,
}

impl Session {
    pub fn has_full_access(&self) -> bool {
        self.access_level == AccessLevel::Full
    }
}

/// Eingeloggte Identität nach erfolgreichem OAuth-Callback.
pub struct NewSession {
    pub user_id: u64,
    pub username: String,
    pub display_name: String,
    pub reason: SessionReason,
    pub access_level: AccessLevel,
}

#[derive(Clone)]
pub struct SessionStore {
    inner: Arc<Mutex<HashMap<String, Session>>>,
    ttl_secs: f64,
    db_path: Option<PathBuf>,
}

impl SessionStore {
    pub fn new(ttl_secs: i64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl_secs: ttl_secs.max(1) as f64,
            db_path: None,
        }
    }

    pub fn persistent(db_path: &Path, ttl_secs: i64, now: f64) -> rusqlite::Result<Self> {
        let db_path = db_path.to_path_buf();
        let sessions = load_persisted_sessions(&db_path, now)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(sessions)),
            ttl_secs: ttl_secs.max(1) as f64,
            db_path: Some(db_path),
        })
    }

    fn guard(&self) -> MutexGuard<'_, HashMap<String, Session>> {
        // Ein vergifteter Mutex (Panic in einem Handler) darf den Store nicht
        // dauerhaft sperren — wir übernehmen die Daten weiter.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Legt eine neue Session an und liefert ihre opake ID.
    pub fn create(&self, new: NewSession, now: f64) -> String {
        let session_id = token::session_token();
        let session = Session {
            user_id: new.user_id,
            username: new.username,
            display_name: new.display_name,
            reason: new.reason,
            access_level: new.access_level,
            csrf_token: token::token_urlsafe(32),
            created_at: now,
            last_seen_at: now,
            expires_at: now + self.ttl_secs,
        };
        let mut map = self.guard();
        prune_expired(&mut map, now);
        map.insert(session_id.clone(), session.clone());
        drop(map);
        self.persist(&session_id, &session);
        session_id
    }

    /// Importiert eine extern erzeugte Session (Twitch-Dashboard-SSO). Eine
    /// bereits vergebene ID wird überschrieben (Upsert, wie im Original).
    /// `false` nur bei leerer ID.
    pub fn import(
        &self,
        session_id: &str,
        new: NewSession,
        expires_at: Option<f64>,
        now: f64,
    ) -> bool {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return false;
        }
        let mut map = self.guard();
        prune_expired(&mut map, now);
        let session = Session {
            user_id: new.user_id,
            username: new.username,
            display_name: new.display_name,
            reason: new.reason,
            access_level: new.access_level,
            csrf_token: token::token_urlsafe(32),
            created_at: now,
            last_seen_at: now,
            expires_at: expires_at.unwrap_or(now + self.ttl_secs),
        };
        map.insert(session_id.to_string(), session.clone());
        drop(map);
        self.persist(session_id, &session);
        true
    }

    /// Schlägt eine Session nach, verlängert sie gleitend und liefert eine
    /// Kopie. `None`, wenn unbekannt oder abgelaufen.
    pub fn touch(&self, session_id: &str, now: f64) -> Option<Session> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return None;
        }
        let mut map = self.guard();
        prune_expired(&mut map, now);
        let session = map.get_mut(session_id)?;
        if session.expires_at <= now {
            map.remove(session_id);
            return None;
        }
        session.expires_at = now + self.ttl_secs;
        session.last_seen_at = now;
        let result = session.clone();
        drop(map);
        self.persist(session_id, &result);
        Some(result)
    }

    /// Entfernt eine Session (Logout).
    pub fn remove(&self, session_id: &str) {
        let session_id = session_id.trim();
        if !session_id.is_empty() {
            {
                self.guard().remove(session_id);
            }
            self.delete_persisted(session_id);
        }
    }

    fn persist(&self, session_id: &str, session: &Session) {
        let Some(path) = self.db_path.as_deref() else {
            return;
        };
        if let Err(error) = persist_session(path, session_id, session) {
            tracing::error!(%error, "Admin-Session konnte nicht persistiert werden");
        }
    }

    fn delete_persisted(&self, session_id: &str) {
        let Some(path) = self.db_path.as_deref() else {
            return;
        };
        if let Err(error) = delete_persisted_session(path, session_id) {
            tracing::warn!(%error, "Persistierte Admin-Session konnte nicht gelöscht werden");
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.guard().len()
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedSession {
    user_id: u64,
    username: String,
    display_name: String,
    reason: String,
    access_level: String,
    csrf_token: String,
    created_at: f64,
    last_seen_at: f64,
    expires_at: f64,
}

impl From<&Session> for PersistedSession {
    fn from(session: &Session) -> Self {
        Self {
            user_id: session.user_id,
            username: session.username.clone(),
            display_name: session.display_name.clone(),
            reason: session.reason.clone(),
            access_level: session.access_level.as_str().to_string(),
            csrf_token: session.csrf_token.clone(),
            created_at: session.created_at,
            last_seen_at: session.last_seen_at,
            expires_at: session.expires_at,
        }
    }
}

impl PersistedSession {
    fn into_session(self) -> Option<Session> {
        let access_level = match self.access_level.as_str() {
            "full" => AccessLevel::Full,
            "turnier_only" => AccessLevel::TurnierOnly,
            _ => return None,
        };
        Some(Session {
            user_id: self.user_id,
            username: self.username,
            display_name: self.display_name,
            reason: self.reason,
            access_level,
            csrf_token: self.csrf_token,
            created_at: self.created_at,
            last_seen_at: self.last_seen_at,
            expires_at: self.expires_at,
        })
    }
}

fn open_connection(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(connection)
}

fn persist_session(path: &Path, session_id: &str, session: &Session) -> rusqlite::Result<()> {
    let payload = serde_json::to_string(&PersistedSession::from(session))
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    open_connection(path)?.execute(
        "INSERT INTO kv_store(ns, k, v) VALUES(?1, ?2, ?3)
         ON CONFLICT(ns, k) DO UPDATE SET v = excluded.v",
        params![SESSION_KV_NAMESPACE, session_id, payload],
    )?;
    Ok(())
}

fn delete_persisted_session(path: &Path, session_id: &str) -> rusqlite::Result<()> {
    open_connection(path)?.execute(
        "DELETE FROM kv_store WHERE ns = ?1 AND k = ?2",
        params![SESSION_KV_NAMESPACE, session_id],
    )?;
    Ok(())
}

fn load_persisted_sessions(path: &Path, now: f64) -> rusqlite::Result<HashMap<String, Session>> {
    let connection = open_connection(path)?;
    let mut statement = connection.prepare("SELECT k, v FROM kv_store WHERE ns = ?1")?;
    let rows = statement.query_map([SESSION_KV_NAMESPACE], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut sessions = HashMap::new();
    for row in rows {
        let (session_id, payload) = row?;
        let Ok(persisted) = serde_json::from_str::<PersistedSession>(&payload) else {
            continue;
        };
        let Some(session) = persisted.into_session() else {
            continue;
        };
        if session.expires_at > now {
            sessions.insert(session_id, session);
        }
    }
    Ok(sessions)
}

fn prune_expired(map: &mut HashMap<String, Session>, now: f64) {
    map.retain(|_, session| session.expires_at > now);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(reason: &str, level: AccessLevel) -> NewSession {
        NewSession {
            user_id: 42,
            username: "nani".into(),
            display_name: "Nani".into(),
            reason: reason.into(),
            access_level: level,
        }
    }

    #[test]
    fn create_und_touch_mit_gleitender_ttl() {
        let store = SessionStore::new(100);
        let id = store.create(login("owner_override", AccessLevel::Full), 1000.0);

        let s = store.touch(&id, 1050.0).expect("gültig");
        assert_eq!(s.user_id, 42);
        assert!(s.has_full_access());
        assert!(!s.csrf_token.is_empty());
        // Gleitende Verlängerung: expires_at = now + ttl.
        assert_eq!(s.expires_at, 1150.0);
        assert_eq!(s.last_seen_at, 1050.0);
    }

    #[test]
    fn abgelaufene_session_verschwindet() {
        let store = SessionStore::new(100);
        let id = store.create(login("owner_override", AccessLevel::Full), 1000.0);
        // now nach Ablauf.
        assert!(store.touch(&id, 2000.0).is_none());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn logout_entfernt() {
        let store = SessionStore::new(100);
        let id = store.create(login("owner_override", AccessLevel::Full), 1000.0);
        store.remove(&id);
        assert!(store.touch(&id, 1000.0).is_none());
    }

    #[test]
    fn import_upsert_und_leere_id() {
        let store = SessionStore::new(100);
        let ok = store.import(
            "ext-id",
            login("twitch_dashboard_import", AccessLevel::Full),
            Some(9999.0),
            1000.0,
        );
        assert!(ok);
        // Leere ID → abgelehnt.
        assert!(!store.import(
            "  ",
            login("twitch_dashboard_import", AccessLevel::Full),
            None,
            1000.0
        ));
        // Gleiche ID erneut → Upsert (überschreibt, wie im Original).
        assert!(store.import(
            "ext-id",
            login("twitch_dashboard_import", AccessLevel::Full),
            None,
            1000.0
        ));
        let s = store.touch("ext-id", 1000.0).expect("gültig");
        assert_eq!(s.expires_at, 1100.0); // touch schiebt auf now+ttl
    }

    #[test]
    fn turnier_only_zugriff() {
        let store = SessionStore::new(100);
        let id = store.create(login("turnier_mod:1", AccessLevel::TurnierOnly), 1000.0);
        let s = store.touch(&id, 1000.0).expect("gültig");
        assert!(!s.has_full_access());
        assert_eq!(s.access_level.as_str(), "turnier_only");
    }

    #[test]
    fn persistente_session_ueberlebt_store_neustart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.sqlite3");
        let connection = Connection::open(&path).expect("db");
        connection
            .execute(
                "CREATE TABLE kv_store(
                    ns TEXT NOT NULL,
                    k TEXT NOT NULL,
                    v TEXT NOT NULL,
                    PRIMARY KEY(ns, k)
                )",
                [],
            )
            .expect("schema");
        drop(connection);

        let first = SessionStore::persistent(&path, 1209600, 1000.0).expect("store");
        let id = first.create(login("owner_override", AccessLevel::Full), 1000.0);
        drop(first);

        let restarted = SessionStore::persistent(&path, 1209600, 1001.0).expect("restart");
        let session = restarted.touch(&id, 1001.0).expect("persistiert");
        assert_eq!(session.user_id, 42);
        assert!(session.has_full_access());
    }
}
