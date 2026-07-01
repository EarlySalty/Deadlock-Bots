//! Persistenter Store der Dashboard-Sessions (`master_dash_session`).
//!
//! Anders als die HMAC-signierten Stats-/Tierlist-Cookies (siehe
//! `dl-webcore::SessionCodec`) ist eine Dashboard-Session ein **opakes
//! Zufallstoken**. Ein kleiner In-Memory-Cache hält den Hot Path schnell;
//! `kv_store` ist die verbindliche Quelle über Prozessneustarts hinweg.
//! Bei jedem Zugriff verlängert sich die Gültigkeit (gleitende TTL).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::config::AccessLevel;
use crate::db::DashboardDbResult;
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
    pool: Option<PgPool>,
}

impl SessionStore {
    pub fn new(ttl_secs: i64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl_secs: ttl_secs.max(1) as f64,
            pool: None,
        }
    }

    pub async fn persistent(pool: PgPool, ttl_secs: i64, now: f64) -> DashboardDbResult<Self> {
        let sessions = load_persisted_sessions(&pool, now).await?;
        Ok(Self {
            inner: Arc::new(Mutex::new(sessions)),
            ttl_secs: ttl_secs.max(1) as f64,
            pool: Some(pool),
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
    pub async fn create(&self, new: NewSession, now: f64) -> DashboardDbResult<String> {
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
        {
            let mut map = self.guard();
            prune_expired(&mut map, now);
        }
        self.persist(&session_id, &session).await?;
        self.guard().insert(session_id.clone(), session);
        Ok(session_id)
    }

    /// Importiert eine extern erzeugte Session (Twitch-Dashboard-SSO). Eine
    /// bereits vergebene ID wird überschrieben (Upsert, wie im Original).
    /// `false` nur bei leerer ID.
    pub async fn import(
        &self,
        session_id: &str,
        new: NewSession,
        expires_at: Option<f64>,
        now: f64,
    ) -> DashboardDbResult<bool> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Ok(false);
        }
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
        {
            let mut map = self.guard();
            prune_expired(&mut map, now);
        }
        self.persist(session_id, &session).await?;
        self.guard().insert(session_id.to_string(), session);
        Ok(true)
    }

    /// Schlägt eine Session nach, verlängert sie gleitend und liefert eine
    /// Kopie. `None`, wenn unbekannt oder abgelaufen.
    pub async fn touch(&self, session_id: &str, now: f64) -> DashboardDbResult<Option<Session>> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Ok(None);
        }
        let result = {
            let mut map = self.guard();
            prune_expired(&mut map, now);
            let Some(session) = map.get_mut(session_id) else {
                return Ok(None);
            };
            if session.expires_at <= now {
                map.remove(session_id);
                return Ok(None);
            }
            session.expires_at = now + self.ttl_secs;
            session.last_seen_at = now;
            session.clone()
        };
        self.persist(session_id, &result).await?;
        Ok(Some(result))
    }

    /// Entfernt eine Session (Logout).
    pub async fn remove(&self, session_id: &str) -> DashboardDbResult<()> {
        let session_id = session_id.trim();
        if !session_id.is_empty() {
            self.delete_persisted(session_id).await?;
            self.guard().remove(session_id);
        }
        Ok(())
    }

    async fn persist(&self, session_id: &str, session: &Session) -> DashboardDbResult<()> {
        if let Some(pool) = self.pool.as_ref() {
            persist_session(pool, session_id, session).await?;
        };
        Ok(())
    }

    async fn delete_persisted(&self, session_id: &str) -> DashboardDbResult<()> {
        if let Some(pool) = self.pool.as_ref() {
            delete_persisted_session(pool, session_id).await?;
        };
        Ok(())
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

async fn persist_session(
    pool: &PgPool,
    session_id: &str,
    session: &Session,
) -> DashboardDbResult<()> {
    let payload = serde_json::to_string(&PersistedSession::from(session))?;
    dl_central_db::kv::set(pool, SESSION_KV_NAMESPACE, session_id, &payload).await?;
    Ok(())
}

async fn delete_persisted_session(pool: &PgPool, session_id: &str) -> DashboardDbResult<()> {
    dl_central_db::kv::delete(pool, SESSION_KV_NAMESPACE, session_id).await?;
    Ok(())
}

async fn load_persisted_sessions(
    pool: &PgPool,
    now: f64,
) -> DashboardDbResult<HashMap<String, Session>> {
    let rows = sqlx::query!(
        r#"
        SELECT k, v
          FROM bot.kv_store
         WHERE ns = $1
        "#,
        SESSION_KV_NAMESPACE,
    )
    .fetch_all(pool)
    .await?;
    let mut sessions = HashMap::new();
    for row in rows {
        let Ok(persisted) = serde_json::from_str::<PersistedSession>(&row.v) else {
            continue;
        };
        let Some(session) = persisted.into_session() else {
            continue;
        };
        if session.expires_at > now {
            sessions.insert(row.k, session);
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

    #[tokio::test]
    async fn create_und_touch_mit_gleitender_ttl() {
        let store = SessionStore::new(100);
        let id = store
            .create(login("owner_override", AccessLevel::Full), 1000.0)
            .await
            .expect("create");

        let s = store
            .touch(&id, 1050.0)
            .await
            .expect("touch")
            .expect("gültig");
        assert_eq!(s.user_id, 42);
        assert!(s.has_full_access());
        assert!(!s.csrf_token.is_empty());
        // Gleitende Verlängerung: expires_at = now + ttl.
        assert_eq!(s.expires_at, 1150.0);
        assert_eq!(s.last_seen_at, 1050.0);
    }

    #[tokio::test]
    async fn abgelaufene_session_verschwindet() {
        let store = SessionStore::new(100);
        let id = store
            .create(login("owner_override", AccessLevel::Full), 1000.0)
            .await
            .expect("create");
        // now nach Ablauf.
        assert!(store.touch(&id, 2000.0).await.expect("touch").is_none());
        assert_eq!(store.len(), 0);
    }

    #[tokio::test]
    async fn logout_entfernt() {
        let store = SessionStore::new(100);
        let id = store
            .create(login("owner_override", AccessLevel::Full), 1000.0)
            .await
            .expect("create");
        store.remove(&id).await.expect("remove");
        assert!(store.touch(&id, 1000.0).await.expect("touch").is_none());
    }

    #[tokio::test]
    async fn import_upsert_und_leere_id() {
        let store = SessionStore::new(100);
        let ok = store
            .import(
                "ext-id",
                login("twitch_dashboard_import", AccessLevel::Full),
                Some(9999.0),
                1000.0,
            )
            .await
            .expect("import");
        assert!(ok);
        // Leere ID → abgelehnt.
        assert!(!store
            .import(
                "  ",
                login("twitch_dashboard_import", AccessLevel::Full),
                None,
                1000.0
            )
            .await
            .expect("import"));
        // Gleiche ID erneut → Upsert (überschreibt, wie im Original).
        assert!(store
            .import(
                "ext-id",
                login("twitch_dashboard_import", AccessLevel::Full),
                None,
                1000.0
            )
            .await
            .expect("import"));
        let s = store
            .touch("ext-id", 1000.0)
            .await
            .expect("touch")
            .expect("gültig");
        assert_eq!(s.expires_at, 1100.0); // touch schiebt auf now+ttl
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn persistente_session_ueberlebt_store_neustart() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let first = SessionStore::persistent(db.pool().clone(), 1209600, 1000.0)
            .await
            .expect("store");
        let id = first
            .create(login("owner_override", AccessLevel::Full), 1000.0)
            .await
            .expect("create");
        assert!(dl_central_db::kv::get(db.pool(), SESSION_KV_NAMESPACE, &id)
            .await
            .expect("kv get")
            .is_some());
        drop(first);

        let restarted = SessionStore::persistent(db.pool().clone(), 1209600, 1001.0)
            .await
            .expect("restart");
        let session = restarted
            .touch(&id, 1001.0)
            .await
            .expect("touch")
            .expect("persistiert");
        assert_eq!(session.user_id, 42);
        assert!(session.has_full_access());
        restarted.remove(&id).await.expect("remove");
        assert!(dl_central_db::kv::get(db.pool(), SESSION_KV_NAMESPACE, &id)
            .await
            .expect("kv get")
            .is_none());
    }
}
