//! Persistenz: ai_moderation_cases + ai_moderation_ragebait_hits
//! (Schema wie SCHEMA_SQL des Originals — das Original legt selbst an).

use dl_db::{Db, DbError};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseAttachment {
    pub url: String,
    pub content_type: String,
    pub filename: String,
}

#[derive(Debug, Clone)]
pub struct CaseDraft {
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub user_id: u64,
    pub user_tag: String,
    pub content: String,
    pub category: String,
    pub confidence: f64,
    pub reason: String,
    pub action: String,
    pub attachments: Vec<CaseAttachment>,
    pub ai_raw_json: String,
    pub escalated_with_context: bool,
}

/// Persistierter Case (für den Review-Flow: löschen/timeouten/bannen).
#[derive(Debug, Clone)]
pub struct CaseRecord {
    pub case_id: String,
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub user_id: u64,
    pub user_tag: String,
    pub category: String,
    pub confidence: f64,
    pub reason: String,
    pub action: String,
}

pub struct ModerationStore {
    pub db: Db,
}

impl ModerationStore {
    pub async fn ensure_schema(&self) -> Result<(), DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS ai_moderation_cases (
                        case_id TEXT PRIMARY KEY,
                        guild_id INTEGER NOT NULL,
                        channel_id INTEGER NOT NULL,
                        message_id INTEGER NOT NULL,
                        user_id INTEGER NOT NULL,
                        user_tag TEXT,
                        original_content TEXT,
                        attachments_json TEXT,
                        ai_category TEXT,
                        ai_confidence REAL,
                        ai_reason TEXT,
                        ai_raw_json TEXT,
                        escalated_with_context INTEGER DEFAULT 0,
                        action TEXT,
                        mod_id INTEGER,
                        mod_action_at TIMESTAMP,
                        mod_deny_reason TEXT,
                        mod_review_message_id INTEGER,
                        log_message_id INTEGER,
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                    );
                    CREATE INDEX IF NOT EXISTS idx_mod_cases_user ON ai_moderation_cases(user_id);
                    CREATE INDEX IF NOT EXISTS idx_mod_cases_created ON ai_moderation_cases(created_at);
                    CREATE TABLE IF NOT EXISTS ai_moderation_ragebait_hits (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        guild_id INTEGER NOT NULL,
                        user_id INTEGER NOT NULL,
                        message_id INTEGER NOT NULL,
                        channel_id INTEGER NOT NULL,
                        content_preview TEXT,
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                    );
                    CREATE INDEX IF NOT EXISTS idx_ragebait_user_time ON ai_moderation_ragebait_hits(user_id, created_at);",
                )
            })
            .await
    }

    /// Case anlegen → case_id (message_id-basiert, kollisionsfrei genug
    /// wie das Original mit Zeit-Suffix).
    pub async fn insert_case(&self, draft: CaseDraft) -> String {
        let case_id = format!("{}-{}", draft.message_id, chrono::Utc::now().timestamp());
        let id = case_id.clone();
        let result = self
            .db
            .write(move |conn| {
                let attachments_json =
                    serde_json::to_string(&draft.attachments).unwrap_or_else(|_| "[]".to_string());
                conn.execute(
                    "INSERT INTO ai_moderation_cases(
                       case_id, guild_id, channel_id, message_id, user_id, user_tag,
                       original_content, attachments_json, ai_category, ai_confidence,
                       ai_reason, ai_raw_json, escalated_with_context, action
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    rusqlite::params![
                        id,
                        draft.guild_id,
                        draft.channel_id,
                        draft.message_id,
                        draft.user_id,
                        draft.user_tag,
                        draft.content,
                        attachments_json,
                        draft.category,
                        draft.confidence,
                        draft.reason,
                        draft.ai_raw_json,
                        if draft.escalated_with_context {
                            1_i64
                        } else {
                            0_i64
                        },
                        draft.action,
                    ],
                )
                .map(|_| ())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, "Moderation: Case-Insert fehlgeschlagen");
        }
        case_id
    }

    pub async fn set_review_message(&self, case_id: &str, message_id: u64) {
        let case_id = case_id.to_string();
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE ai_moderation_cases SET mod_review_message_id = ?1 WHERE case_id = ?2",
                    rusqlite::params![message_id, case_id],
                )
                .map(|_| ())
            })
            .await;
    }

    pub async fn set_log_message(&self, case_id: &str, message_id: u64) {
        let case_id = case_id.to_string();
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE ai_moderation_cases SET log_message_id = ?1 WHERE case_id = ?2",
                    rusqlite::params![message_id, case_id],
                )
                .map(|_| ())
            })
            .await;
    }

    pub async fn update_case_action(&self, case_id: &str, action: &str) {
        let (case_id, action) = (case_id.to_string(), action.to_string());
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE ai_moderation_cases SET action = ?1 WHERE case_id = ?2",
                    rusqlite::params![action, case_id],
                )
                .map(|_| ())
            })
            .await;
    }

    pub async fn resolve_case(&self, case_id: &str, action: &str, mod_id: u64) {
        let (case_id, action) = (case_id.to_string(), action.to_string());
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE ai_moderation_cases
                        SET action = ?1, mod_id = ?2, mod_action_at = CURRENT_TIMESTAMP
                      WHERE case_id = ?3",
                    rusqlite::params![action, mod_id, case_id],
                )
                .map(|_| ())
            })
            .await;
    }

    /// Wie `resolve_case`, aber mit Pflicht-Begründung (`mod_deny_reason`) —
    /// für den Deny-Flow (Original: `_mark_case_denied_sync`).
    pub async fn resolve_case_denied(&self, case_id: &str, mod_id: u64, reason: &str) {
        let (case_id, reason) = (case_id.to_string(), reason.to_string());
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE ai_moderation_cases
                        SET action = 'denied', mod_id = ?1,
                            mod_action_at = CURRENT_TIMESTAMP, mod_deny_reason = ?2
                      WHERE case_id = ?3",
                    rusqlite::params![mod_id, reason, case_id],
                )
                .map(|_| ())
            })
            .await;
    }

    /// Case laden (für den Review-Flow). None, wenn nicht vorhanden.
    pub async fn fetch_case(&self, case_id: &str) -> Option<CaseRecord> {
        let case_id = case_id.to_string();
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT case_id, guild_id, channel_id, message_id, user_id, user_tag,
                            ai_category, ai_confidence, ai_reason, action
                       FROM ai_moderation_cases WHERE case_id = ?1",
                    [case_id],
                    |row| {
                        Ok(CaseRecord {
                            case_id: row.get(0)?,
                            guild_id: row.get::<_, i64>(1)? as u64,
                            channel_id: row.get::<_, i64>(2)? as u64,
                            message_id: row.get::<_, i64>(3)? as u64,
                            user_id: row.get::<_, i64>(4)? as u64,
                            user_tag: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                            category: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                            confidence: row.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
                            reason: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                            action: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                        })
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    /// Hit zählen → (Anzahl im 120-min-Fenster, Previews ältester→neuester).
    pub async fn insert_ragebait_hit(
        &self,
        guild_id: u64,
        user_id: u64,
        message_id: u64,
        channel_id: u64,
        content: &str,
    ) -> (i64, Vec<String>) {
        let preview: String = content.chars().take(180).collect();
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO ai_moderation_ragebait_hits(
                       guild_id, user_id, message_id, channel_id, content_preview
                     ) VALUES(?1,?2,?3,?4,?5)",
                    rusqlite::params![guild_id, user_id, message_id, channel_id, preview],
                )?;
                let mut stmt = conn.prepare(
                    "SELECT content_preview FROM ai_moderation_ragebait_hits
                      WHERE user_id = ?1
                        AND guild_id = ?2
                        AND created_at > datetime('now', ?3)
                      ORDER BY created_at ASC",
                )?;
                let window = format!("-{} minutes", super::RAGEBAIT_WINDOW_MINUTES);
                let previews: Vec<String> = stmt
                    .query_map(rusqlite::params![user_id, guild_id, window], |row| {
                        row.get::<_, Option<String>>(0)
                            .map(|p| p.unwrap_or_default())
                    })?
                    .collect::<Result<_, _>>()?;
                Ok((previews.len() as i64, previews))
            })
            .await
            .unwrap_or((0, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (tempfile::TempDir, ModerationStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        let store = ModerationStore { db };
        store.ensure_schema().await.expect("schema");
        (dir, store)
    }

    #[tokio::test]
    async fn case_lifecycle() {
        let (_dir, store) = store().await;
        let case_id = store
            .insert_case(CaseDraft {
                guild_id: 1,
                channel_id: 2,
                message_id: 3,
                user_id: 100,
                user_tag: "Anna".into(),
                content: "böse nachricht".into(),
                category: "scam".into(),
                confidence: 0.95,
                reason: "Scam".into(),
                action: "proposed".into(),
                attachments: vec![CaseAttachment {
                    url: "https://cdn.example/image.png".into(),
                    content_type: "image/png".into(),
                    filename: "image.png".into(),
                }],
                ai_raw_json: "{\"response_text\":\"raw\"}".into(),
                escalated_with_context: true,
            })
            .await;
        store.set_review_message(&case_id, 999).await;
        store
            .update_case_action(&case_id, "auto_delete_failed")
            .await;
        store.resolve_case(&case_id, "accepted", 777).await;
        let (action, mod_id, review): (String, u64, u64) = store
            .db
            .read(move |conn| {
                conn.query_row(
                    "SELECT action, mod_id, mod_review_message_id FROM ai_moderation_cases WHERE case_id = ?1",
                    [case_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
            })
            .await
            .expect("case");
        assert_eq!((action.as_str(), mod_id, review), ("accepted", 777, 999));

        let (attachments, raw, escalated): (String, String, i64) = store
            .db
            .read(move |conn| {
                conn.query_row(
                    "SELECT attachments_json, ai_raw_json, escalated_with_context
                       FROM ai_moderation_cases WHERE message_id = 3",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
            })
            .await
            .expect("case metadata");
        assert!(attachments.contains("https://cdn.example/image.png"));
        assert_eq!(raw, "{\"response_text\":\"raw\"}");
        assert_eq!(escalated, 1);
    }

    #[tokio::test]
    async fn fetch_und_deny_mit_grund() {
        let (_dir, store) = store().await;
        let case_id = store
            .insert_case(CaseDraft {
                guild_id: 1,
                channel_id: 2,
                message_id: 3,
                user_id: 100,
                user_tag: "Anna".into(),
                content: "x".into(),
                category: "scam".into(),
                confidence: 0.9,
                reason: "Scam".into(),
                action: "proposed".into(),
                attachments: Vec::new(),
                ai_raw_json: "{}".into(),
                escalated_with_context: false,
            })
            .await;
        let case = store.fetch_case(&case_id).await.expect("case");
        assert_eq!(case.action, "proposed");
        assert_eq!(
            (case.channel_id, case.message_id, case.user_id),
            (2, 3, 100)
        );

        store
            .resolve_case_denied(&case_id, 777, "kein Verstoss")
            .await;
        let denied = store.fetch_case(&case_id).await.expect("case");
        assert_eq!(denied.action, "denied");

        let reason: Option<String> = store
            .db
            .read({
                let case_id = case_id.clone();
                move |conn| {
                    conn.query_row(
                        "SELECT mod_deny_reason FROM ai_moderation_cases WHERE case_id = ?1",
                        [case_id],
                        |row| row.get(0),
                    )
                }
            })
            .await
            .expect("reason");
        assert_eq!(reason.as_deref(), Some("kein Verstoss"));

        assert!(store.fetch_case("gibtsnicht").await.is_none());
    }

    #[tokio::test]
    async fn ragebait_fenster_zaehlt() {
        let (_dir, store) = store().await;
        for i in 0..4 {
            let (count, previews) = store
                .insert_ragebait_hit(1, 100, 1000 + i, 2, &format!("bait {i}"))
                .await;
            assert_eq!(count, i as i64 + 1);
            assert_eq!(previews.len() as i64, count);
        }
        // anderer User zählt eigenes Fenster
        let (count, _) = store.insert_ragebait_hit(1, 200, 2000, 2, "x").await;
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn ragebait_fenster_ist_guild_spezifisch() {
        let (_dir, store) = store().await;
        for i in 0..3 {
            let (count, _) = store
                .insert_ragebait_hit(1, 100, 3000 + i, 2, &format!("g1 bait {i}"))
                .await;
            assert_eq!(count, i as i64 + 1);
        }

        let (count, previews) = store.insert_ragebait_hit(2, 100, 4000, 3, "g2 bait").await;
        assert_eq!(count, 1);
        assert_eq!(previews, vec!["g2 bait".to_string()]);
    }

    #[tokio::test]
    async fn ragebait_fenster_zaehlt_fensterrand_strikt_nicht_mit() {
        let (_dir, store) = store().await;

        let second = chrono::Utc::now().timestamp();
        while chrono::Utc::now().timestamp() == second {
            tokio::task::yield_now().await;
        }

        store
            .db
            .write(|conn| {
                let window = format!("-{} minutes", crate::RAGEBAIT_WINDOW_MINUTES);
                conn.execute(
                    "INSERT INTO ai_moderation_ragebait_hits(
                       guild_id, user_id, message_id, channel_id, content_preview, created_at
                     ) VALUES(?1,?2,?3,?4,?5,datetime('now', ?6))",
                    rusqlite::params![1_u64, 100_u64, 4998_u64, 2_u64, "genau rand", window],
                )?;
                conn.execute(
                    "INSERT INTO ai_moderation_ragebait_hits(
                       guild_id, user_id, message_id, channel_id, content_preview, created_at
                     ) VALUES(?1,?2,?3,?4,?5,datetime('now', ?6, '+1 second'))",
                    rusqlite::params![
                        1_u64,
                        100_u64,
                        4999_u64,
                        2_u64,
                        "knapp drin",
                        format!("-{} minutes", crate::RAGEBAIT_WINDOW_MINUTES)
                    ],
                )?;
                Ok(())
            })
            .await
            .expect("boundary hits");

        let (count, previews) = store.insert_ragebait_hit(1, 100, 5000, 2, "neu").await;
        assert_eq!(count, 2);
        assert_eq!(
            previews,
            vec!["knapp drin".to_string(), "neu".to_string()]
        );
    }
}
