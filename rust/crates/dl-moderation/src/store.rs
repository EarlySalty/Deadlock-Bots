//! Persistenz: ai_moderation_cases + ai_moderation_ragebait_hits
//! (Schema wie SCHEMA_SQL des Originals — das Original legt selbst an).

use dl_db::{Db, DbError};

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
                conn.execute(
                    "INSERT INTO ai_moderation_cases(
                       case_id, guild_id, channel_id, message_id, user_id, user_tag,
                       original_content, ai_category, ai_confidence, ai_reason, action
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    rusqlite::params![
                        id,
                        draft.guild_id,
                        draft.channel_id,
                        draft.message_id,
                        draft.user_id,
                        draft.user_tag,
                        draft.content,
                        draft.category,
                        draft.confidence,
                        draft.reason,
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
                        AND created_at >= datetime('now', ?2)
                      ORDER BY created_at ASC",
                )?;
                let window = format!("-{} minutes", super::RAGEBAIT_WINDOW_MINUTES);
                let previews: Vec<String> = stmt
                    .query_map(rusqlite::params![user_id, window], |row| {
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
            })
            .await;
        store.set_review_message(&case_id, 999).await;
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
}
