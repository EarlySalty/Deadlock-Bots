//! Tag-System — Port von `cogs/tags/core.py` (TagService).
//!
//! User-Tags (`age`: 25+/u25, `tone`: banter_ok/ragebaiter_free) und
//! Mod-Tags (`ragebaiter`, Default-Laufzeit 14 Tage) in den Bestands-
//! Tabellen `user_tags`/`user_mod_tags`. Änderungen werden als
//! [`TagEvent`] gebroadcastet — das Pendant zu `bot.dispatch(...)`, an
//! dem TempVoice-Tag-Filter und Moderation hängen.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dl_db::Db;

pub const USER_TAG_KEYS: [&str; 2] = ["age", "tone"];
pub const AGE_TAGS: [&str; 2] = ["25+", "u25"];
pub const TONE_TAGS: [&str; 2] = ["banter_ok", "ragebaiter_free"];
pub const MOD_TAGS: [&str; 1] = ["ragebaiter"];
pub const RAGEBAITER_DEFAULT_DAYS: i64 = 14;
pub const CLEANUP_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagEvent {
    /// (user_id, key, old, new) — new=None bedeutet gelöscht.
    UserTagChanged {
        user_id: u64,
        key: String,
        old: Option<String>,
        new: Option<String>,
    },
    ModTagAdded {
        user_id: u64,
        tag: String,
        set_by: u64,
        reason: Option<String>,
    },
    ModTagRemoved {
        user_id: u64,
        tag: String,
        removed_by: u64,
    },
}

fn validate_user_key(key: &str) -> Result<&'static str, String> {
    let normalized = key.trim().to_lowercase();
    USER_TAG_KEYS
        .iter()
        .find(|k| **k == normalized)
        .copied()
        .ok_or_else(|| format!("unsupported user tag key: {key}"))
}

fn validate_user_tag(key: &str, value: &str) -> Result<(&'static str, String), String> {
    let key = validate_user_key(key)?;
    let value = value.trim().to_lowercase();
    let allowed: &[&str] = match key {
        "age" => &AGE_TAGS,
        _ => &TONE_TAGS,
    };
    if !allowed.contains(&value.as_str()) {
        return Err(format!("unsupported value for {key}: {value}"));
    }
    Ok((key, value))
}

fn validate_mod_tag(tag: &str) -> Result<&'static str, String> {
    let normalized = tag.trim().to_lowercase();
    MOD_TAGS
        .iter()
        .find(|t| **t == normalized)
        .copied()
        .ok_or_else(|| format!("unsupported mod tag: {tag}"))
}

#[derive(Debug, Clone)]
struct ModTagState {
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct TagCache {
    user_tags: HashMap<u64, HashMap<String, String>>,
    mod_tags: HashMap<u64, HashMap<String, ModTagState>>,
}

pub struct TagService {
    db: Db,
    cache: tokio::sync::Mutex<TagCache>,
    events: tokio::sync::broadcast::Sender<TagEvent>,
}

impl TagService {
    pub fn new(db: Db) -> Arc<Self> {
        let (events, _) = tokio::sync::broadcast::channel(256);
        Arc::new(Self {
            db,
            cache: tokio::sync::Mutex::new(TagCache::default()),
            events,
        })
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TagEvent> {
        self.events.subscribe()
    }

    /// Cache aus der DB füllen (cog_load-Äquivalent) + abgelaufene räumen.
    #[allow(clippy::type_complexity)]
    pub async fn rehydrate(&self) {
        let rows: (
            Vec<(u64, String, String)>,
            Vec<(u64, String, Option<String>)>,
        ) = self
            .db
            .read(|conn| {
                let mut user_stmt =
                    conn.prepare("SELECT user_id, tag_key, tag_value FROM user_tags")?;
                let user_rows: Vec<(u64, String, String)> = user_stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<Result<_, _>>()?;
                let mut mod_stmt =
                    conn.prepare("SELECT user_id, mod_tag, expires_at FROM user_mod_tags")?;
                let mod_rows: Vec<(u64, String, Option<String>)> = mod_stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<Result<_, _>>()?;
                Ok((user_rows, mod_rows))
            })
            .await
            .unwrap_or_default();

        let mut cache = self.cache.lock().await;
        cache.user_tags.clear();
        cache.mod_tags.clear();
        for (user_id, key, value) in rows.0 {
            cache
                .user_tags
                .entry(user_id)
                .or_default()
                .insert(key, value);
        }
        for (user_id, tag, expires_raw) in rows.1 {
            let expires_at = expires_raw.as_deref().and_then(parse_expiry);
            cache
                .mod_tags
                .entry(user_id)
                .or_default()
                .insert(tag, ModTagState { expires_at });
        }
        drop(cache);
        self.cleanup_expired().await;
    }

    pub async fn get_user_tags(&self, user_id: u64) -> HashMap<String, String> {
        self.cache
            .lock()
            .await
            .user_tags
            .get(&user_id)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn set_user_tag(&self, user_id: u64, key: &str, value: &str) -> Result<(), String> {
        let (key, value) = validate_user_tag(key, value)?;
        let (key_owned, value_owned) = (key.to_string(), value.clone());
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO user_tags(user_id, tag_key, tag_value) VALUES(?1, ?2, ?3)
                     ON CONFLICT(user_id, tag_key) DO UPDATE SET
                       tag_value = excluded.tag_value, set_at = CURRENT_TIMESTAMP",
                    rusqlite::params![user_id, key_owned, value_owned],
                )
                .map(|_| ())
            })
            .await
            .map_err(|e| e.to_string())?;

        let old = {
            let mut cache = self.cache.lock().await;
            cache
                .user_tags
                .entry(user_id)
                .or_default()
                .insert(key.to_string(), value.clone())
        };
        if old.as_deref() != Some(value.as_str()) {
            let _ = self.events.send(TagEvent::UserTagChanged {
                user_id,
                key: key.to_string(),
                old,
                new: Some(value),
            });
        }
        Ok(())
    }

    pub async fn clear_user_tag(&self, user_id: u64, key: &str) -> Result<(), String> {
        let key = validate_user_key(key)?;
        let key_owned = key.to_string();
        self.db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM user_tags WHERE user_id = ?1 AND tag_key = ?2",
                    rusqlite::params![user_id, key_owned],
                )
                .map(|_| ())
            })
            .await
            .map_err(|e| e.to_string())?;
        let old = {
            let mut cache = self.cache.lock().await;
            let old = cache
                .user_tags
                .get_mut(&user_id)
                .and_then(|tags| tags.remove(key));
            if cache.user_tags.get(&user_id).map(HashMap::is_empty) == Some(true) {
                cache.user_tags.remove(&user_id);
            }
            old
        };
        if let Some(old) = old {
            let _ = self.events.send(TagEvent::UserTagChanged {
                user_id,
                key: key.to_string(),
                old: Some(old),
                new: None,
            });
        }
        Ok(())
    }

    pub async fn has_user_tag(&self, user_id: u64, key: &str, value: &str) -> bool {
        let Ok((key, value)) = validate_user_tag(key, value) else {
            return false;
        };
        self.cache
            .lock()
            .await
            .user_tags
            .get(&user_id)
            .and_then(|tags| tags.get(key))
            .map(|v| *v == value)
            .unwrap_or(false)
    }

    /// Mod-Tag setzen — ohne expires_at greift die Default-Laufzeit
    /// (ragebaiter: 14 Tage).
    pub async fn add_mod_tag(
        &self,
        user_id: u64,
        tag: &str,
        set_by: u64,
        reason: Option<String>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), String> {
        let tag = validate_mod_tag(tag)?;
        let expires_at = expires_at.or_else(|| {
            (tag == "ragebaiter")
                .then(|| Utc::now() + chrono::Duration::days(RAGEBAITER_DEFAULT_DAYS))
        });
        let payload = expires_at.map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Micros, true));
        let (tag_owned, reason_owned) = (tag.to_string(), reason.clone());
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO user_mod_tags(user_id, mod_tag, set_by, reason, expires_at)
                     VALUES(?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(user_id, mod_tag) DO UPDATE SET
                       set_by = excluded.set_by, reason = excluded.reason,
                       expires_at = excluded.expires_at, set_at = CURRENT_TIMESTAMP",
                    rusqlite::params![user_id, tag_owned, set_by, reason_owned, payload],
                )
                .map(|_| ())
            })
            .await
            .map_err(|e| e.to_string())?;
        self.cache
            .lock()
            .await
            .mod_tags
            .entry(user_id)
            .or_default()
            .insert(tag.to_string(), ModTagState { expires_at });
        let _ = self.events.send(TagEvent::ModTagAdded {
            user_id,
            tag: tag.to_string(),
            set_by,
            reason,
        });
        Ok(())
    }

    pub async fn remove_mod_tag(
        &self,
        user_id: u64,
        tag: &str,
        removed_by: u64,
    ) -> Result<(), String> {
        let tag = validate_mod_tag(tag)?;
        let tag_owned = tag.to_string();
        self.db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM user_mod_tags WHERE user_id = ?1 AND mod_tag = ?2",
                    rusqlite::params![user_id, tag_owned],
                )
                .map(|_| ())
            })
            .await
            .map_err(|e| e.to_string())?;
        let existed = {
            let mut cache = self.cache.lock().await;
            let existed = cache
                .mod_tags
                .get_mut(&user_id)
                .and_then(|tags| tags.remove(tag))
                .is_some();
            if cache.mod_tags.get(&user_id).map(HashMap::is_empty) == Some(true) {
                cache.mod_tags.remove(&user_id);
            }
            existed
        };
        if existed {
            let _ = self.events.send(TagEvent::ModTagRemoved {
                user_id,
                tag: tag.to_string(),
                removed_by,
            });
        }
        Ok(())
    }

    pub async fn has_active_mod_tag(&self, user_id: u64, tag: &str) -> bool {
        let Ok(tag) = validate_mod_tag(tag) else {
            return false;
        };
        let now = Utc::now();
        let mut cache = self.cache.lock().await;
        if let Some(tags) = cache.mod_tags.get_mut(&user_id) {
            tags.retain(|_, state| state.expires_at.map(|e| e > now).unwrap_or(true));
            return tags.contains_key(tag);
        }
        false
    }

    /// Abgelaufene Mod-Tags löschen → entfernte (user_id, tag)-Paare.
    pub async fn cleanup_expired(&self) -> Vec<(u64, String)> {
        let now_iso = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        let removed: Vec<(u64, String)> = self
            .db
            .write(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT user_id, mod_tag FROM user_mod_tags
                      WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                )?;
                let expired: Vec<(u64, String)> = stmt
                    .query_map([&now_iso], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<_, _>>()?;
                drop(stmt);
                conn.execute(
                    "DELETE FROM user_mod_tags
                      WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                    [&now_iso],
                )?;
                Ok(expired)
            })
            .await
            .unwrap_or_default();
        if !removed.is_empty() {
            let mut cache = self.cache.lock().await;
            for (user_id, tag) in &removed {
                if let Some(tags) = cache.mod_tags.get_mut(user_id) {
                    tags.remove(tag);
                }
            }
            for (user_id, tag) in &removed {
                let _ = self.events.send(TagEvent::ModTagRemoved {
                    user_id: *user_id,
                    tag: tag.clone(),
                    removed_by: 0, // System-Cleanup
                });
            }
        }
        removed
    }
}

/// fromisoformat-tolerantes Expiry-Parsing (mit/ohne Zeitzone → UTC).
fn parse_expiry(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f"))
                .ok()
                .map(|naive| naive.and_utc())
        })
}

/// 5-min-Cleanup-Loop wie das Original.
pub fn spawn_cleanup(service: Arc<TagService>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(CLEANUP_INTERVAL).await;
            let removed = service.cleanup_expired().await;
            if !removed.is_empty() {
                tracing::info!(count = removed.len(), "Tag-Cleanup: Mod-Tags abgelaufen");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDLS: [&str; 2] = [
        "CREATE TABLE user_tags(user_id INTEGER NOT NULL, tag_key TEXT NOT NULL, tag_value TEXT NOT NULL, set_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, PRIMARY KEY(user_id, tag_key))",
        "CREATE TABLE user_mod_tags(user_id INTEGER NOT NULL, mod_tag TEXT NOT NULL, set_by INTEGER NOT NULL, reason TEXT, set_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, expires_at TIMESTAMP, PRIMARY KEY(user_id, mod_tag))",
    ];

    async fn service() -> (tempfile::TempDir, Arc<TagService>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        for ddl in DDLS {
            db.write(move |c| c.execute(ddl, []).map(|_| ()))
                .await
                .expect("ddl");
        }
        (dir, TagService::new(db))
    }

    #[test]
    fn validierung_wie_python() {
        assert!(validate_user_tag("age", "25+").is_ok());
        assert!(validate_user_tag("AGE", " U25 ").is_ok()); // normalisiert
        assert!(validate_user_tag("age", "banter_ok").is_err());
        assert!(validate_user_tag("tone", "ragebaiter_free").is_ok());
        assert!(validate_user_key("style").is_err());
        assert!(validate_mod_tag("Ragebaiter").is_ok());
        assert!(validate_mod_tag("spammer").is_err());
    }

    #[tokio::test]
    async fn user_tags_mit_events() {
        let (_dir, service) = service().await;
        let mut events = service.subscribe();
        service.set_user_tag(100, "age", "25+").await.expect("set");
        assert!(service.has_user_tag(100, "age", "25+").await);
        assert_eq!(
            events.recv().await.expect("event"),
            TagEvent::UserTagChanged {
                user_id: 100,
                key: "age".into(),
                old: None,
                new: Some("25+".into()),
            }
        );
        // gleicher Wert → kein Event; Wechsel → Event mit old
        service.set_user_tag(100, "age", "25+").await.expect("set");
        service.set_user_tag(100, "age", "u25").await.expect("set");
        assert_eq!(
            events.recv().await.expect("event"),
            TagEvent::UserTagChanged {
                user_id: 100,
                key: "age".into(),
                old: Some("25+".into()),
                new: Some("u25".into()),
            }
        );
        service.clear_user_tag(100, "age").await.expect("clear");
        assert!(!service.has_user_tag(100, "age", "u25").await);
    }

    #[tokio::test]
    async fn mod_tag_default_laufzeit_und_cleanup() {
        let (_dir, service) = service().await;
        // Default-Expiry 14 Tage → aktiv
        service
            .add_mod_tag(100, "ragebaiter", 999, Some("Spam".into()), None)
            .await
            .expect("add");
        assert!(service.has_active_mod_tag(100, "ragebaiter").await);

        // Abgelaufener Tag → Cleanup entfernt ihn + Event
        let past = Utc::now() - chrono::Duration::hours(1);
        service
            .add_mod_tag(200, "ragebaiter", 999, None, Some(past))
            .await
            .expect("add");
        let removed = service.cleanup_expired().await;
        assert_eq!(removed, vec![(200, "ragebaiter".to_string())]);
        assert!(!service.has_active_mod_tag(200, "ragebaiter").await);
        assert!(service.has_active_mod_tag(100, "ragebaiter").await);
    }

    #[tokio::test]
    async fn rehydrate_aus_db() {
        let (_dir, service) = service().await;
        service
            .set_user_tag(100, "tone", "banter_ok")
            .await
            .expect("set");
        service
            .add_mod_tag(100, "ragebaiter", 999, None, None)
            .await
            .expect("add");
        // Frischer Service über dieselbe DB
        let fresh = TagService::new(service.db.clone());
        fresh.rehydrate().await;
        assert!(fresh.has_user_tag(100, "tone", "banter_ok").await);
        assert!(fresh.has_active_mod_tag(100, "ragebaiter").await);
    }
}
