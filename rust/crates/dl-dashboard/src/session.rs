//! In-Memory-Store der Dashboard-Sessions (`master_dash_session`).
//!
//! Anders als die HMAC-signierten Stats-/Tierlist-Cookies (siehe
//! `dl-webcore::SessionCodec`) ist eine Dashboard-Session ein **opakes
//! Zufallstoken**, das serverseitig in einer Map nachgeschlagen wird — exakt
//! wie `self._discord_sessions` im Python-Dashboard. Bei jedem Zugriff
//! verlängert sich die Gültigkeit (gleitende TTL), und ein fehlendes
//! CSRF-Token wird nachgezogen. Neustart leert den Store (wie im Original).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::config::AccessLevel;
use crate::token;

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
}

impl SessionStore {
    pub fn new(ttl_secs: i64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl_secs: ttl_secs.max(1) as f64,
        }
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
        map.insert(session_id.clone(), session);
        session_id
    }

    /// Importiert eine extern erzeugte Session (Twitch-Dashboard-SSO). Eine
    /// bereits vergebene ID wird abgelehnt (`false`).
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
        if map.contains_key(session_id) {
            return false;
        }
        map.insert(
            session_id.to_string(),
            Session {
                user_id: new.user_id,
                username: new.username,
                display_name: new.display_name,
                reason: new.reason,
                access_level: new.access_level,
                csrf_token: token::token_urlsafe(32),
                created_at: now,
                last_seen_at: now,
                expires_at: expires_at.unwrap_or(now + self.ttl_secs),
            },
        );
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
        Some(session.clone())
    }

    /// Entfernt eine Session (Logout).
    pub fn remove(&self, session_id: &str) {
        let session_id = session_id.trim();
        if !session_id.is_empty() {
            self.guard().remove(session_id);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.guard().len()
    }
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
    fn import_ist_einmalig_pro_id() {
        let store = SessionStore::new(100);
        let ok = store.import(
            "ext-id",
            login("twitch_dashboard_import", AccessLevel::Full),
            Some(9999.0),
            1000.0,
        );
        assert!(ok);
        // Gleiche ID erneut → abgelehnt.
        assert!(!store.import(
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
}
