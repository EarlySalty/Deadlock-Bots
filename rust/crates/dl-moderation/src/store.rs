//! Persistenz: moderation.ai_moderation_cases + moderation.ai_moderation_ragebait_hits.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};

const RAGEBAIT_HITS_ID_LOCK_KEY: i64 = 0x5241_4745_4241_4954;

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

#[derive(Clone)]
pub struct ModerationStore {
    pub pool: PgPool,
}

impl ModerationStore {
    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            SELECT 1 AS "ok!"
            FROM moderation.ai_moderation_cases
            LIMIT 0
            "#
        )
        .fetch_optional(&self.pool)
        .await?;

        sqlx::query!(
            r#"
            SELECT 1 AS "ok!"
            FROM moderation.ai_moderation_ragebait_hits
            LIMIT 0
            "#
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(())
    }

    /// Case anlegen -> case_id nur bei erfolgreicher Persistenz.
    pub async fn insert_case(&self, draft: CaseDraft) -> Option<String> {
        let case_id = format!("{}-{}", draft.message_id, chrono::Utc::now().timestamp());
        let guild_id = discord_id_to_i64(draft.guild_id, "guild_id")?;
        let channel_id = discord_id_to_i64(draft.channel_id, "channel_id")?;
        let message_id = discord_id_to_i64(draft.message_id, "message_id")?;
        let user_id = discord_id_to_i64(draft.user_id, "user_id")?;
        let attachments_json =
            serde_json::to_string(&draft.attachments).unwrap_or_else(|_| "[]".to_string());
        let ai_raw_json = jsonb_text_or_string(&draft.ai_raw_json);

        let result = sqlx::query!(
            r#"
            INSERT INTO moderation.ai_moderation_cases(
                case_id, guild_id, channel_id, message_id, user_id, user_tag,
                original_content, attachments, ai_category, ai_confidence,
                ai_reason, ai_raw, escalated_with_context, action, created_at
            )
            VALUES(
                $1, $2, $3, $4, $5, $6,
                $7, $8::text::jsonb, $9, $10,
                $11, $12::text::jsonb, $13, $14, now()
            )
            "#,
            &case_id,
            guild_id,
            channel_id,
            message_id,
            user_id,
            draft.user_tag,
            draft.content,
            attachments_json,
            draft.category,
            draft.confidence,
            draft.reason,
            ai_raw_json,
            draft.escalated_with_context,
            draft.action,
        )
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Some(case_id),
            Err(err) => {
                tracing::warn!(%err, "Moderation: Case-Insert fehlgeschlagen");
                None
            }
        }
    }

    pub async fn set_review_message(&self, case_id: &str, message_id: u64) {
        let Some(message_id) = discord_id_to_i64(message_id, "mod_review_message_id") else {
            return;
        };
        if let Err(err) = sqlx::query!(
            r#"
            UPDATE moderation.ai_moderation_cases
            SET mod_review_message_id = $1
            WHERE case_id = $2
            "#,
            message_id,
            case_id,
        )
        .execute(&self.pool)
        .await
        {
            tracing::warn!(%err, case_id, "Moderation: Review-Message konnte nicht gespeichert werden");
        }
    }

    pub async fn set_log_message(&self, case_id: &str, message_id: u64) {
        let Some(message_id) = discord_id_to_i64(message_id, "log_message_id") else {
            return;
        };
        if let Err(err) = sqlx::query!(
            r#"
            UPDATE moderation.ai_moderation_cases
            SET log_message_id = $1
            WHERE case_id = $2
            "#,
            message_id,
            case_id,
        )
        .execute(&self.pool)
        .await
        {
            tracing::warn!(%err, case_id, "Moderation: Log-Message konnte nicht gespeichert werden");
        }
    }

    pub async fn update_case_action(&self, case_id: &str, action: &str) {
        if let Err(err) = sqlx::query!(
            r#"
            UPDATE moderation.ai_moderation_cases
            SET action = $1
            WHERE case_id = $2
            "#,
            action,
            case_id,
        )
        .execute(&self.pool)
        .await
        {
            tracing::warn!(%err, case_id, action, "Moderation: Case-Action konnte nicht aktualisiert werden");
        }
    }

    pub async fn resolve_case(&self, case_id: &str, action: &str, mod_id: u64) {
        let Some(mod_id) = discord_id_to_i64(mod_id, "mod_id") else {
            return;
        };
        if let Err(err) = sqlx::query!(
            r#"
            UPDATE moderation.ai_moderation_cases
            SET action = $1, mod_id = $2, mod_action_at = now()
            WHERE case_id = $3
            "#,
            action,
            mod_id,
            case_id,
        )
        .execute(&self.pool)
        .await
        {
            tracing::warn!(%err, case_id, action, "Moderation: Case konnte nicht resolved werden");
        }
    }

    /// Wie `resolve_case`, aber mit Pflicht-Begründung (`mod_deny_reason`) -
    /// für den Deny-Flow (Original: `_mark_case_denied_sync`).
    pub async fn resolve_case_denied(&self, case_id: &str, mod_id: u64, reason: &str) {
        let Some(mod_id) = discord_id_to_i64(mod_id, "mod_id") else {
            return;
        };
        if let Err(err) = sqlx::query!(
            r#"
            UPDATE moderation.ai_moderation_cases
            SET action = 'denied',
                mod_id = $1,
                mod_action_at = now(),
                mod_deny_reason = $2
            WHERE case_id = $3
            "#,
            mod_id,
            reason,
            case_id,
        )
        .execute(&self.pool)
        .await
        {
            tracing::warn!(%err, case_id, "Moderation: Case-Deny konnte nicht gespeichert werden");
        }
    }

    /// Case laden (für den Review-Flow). None, wenn nicht vorhanden.
    pub async fn fetch_case(&self, case_id: &str) -> Option<CaseRecord> {
        let row = match sqlx::query!(
            r#"
            SELECT
                case_id AS "case_id!",
                guild_id AS "guild_id!",
                channel_id AS "channel_id!",
                message_id AS "message_id!",
                user_id AS "user_id!",
                user_tag,
                ai_category,
                ai_confidence,
                ai_reason,
                action
            FROM moderation.ai_moderation_cases
            WHERE case_id = $1
            "#,
            case_id,
        )
        .fetch_optional(&self.pool)
        .await
        {
            Ok(row) => row?,
            Err(err) => {
                tracing::warn!(%err, case_id, "Moderation: Case konnte nicht geladen werden");
                return None;
            }
        };

        let guild_id = db_id_to_u64(row.guild_id, "guild_id")?;
        let channel_id = db_id_to_u64(row.channel_id, "channel_id")?;
        let message_id = db_id_to_u64(row.message_id, "message_id")?;
        let user_id = db_id_to_u64(row.user_id, "user_id")?;

        Some(CaseRecord {
            case_id: row.case_id,
            guild_id,
            channel_id,
            message_id,
            user_id,
            user_tag: row.user_tag.unwrap_or_default(),
            category: row.ai_category.unwrap_or_default(),
            confidence: row.ai_confidence.unwrap_or(0.0),
            reason: row.ai_reason.unwrap_or_default(),
            action: row.action.unwrap_or_default(),
        })
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
        let Some(guild_id) = discord_id_to_i64(guild_id, "guild_id") else {
            return (0, Vec::new());
        };
        let Some(user_id) = discord_id_to_i64(user_id, "user_id") else {
            return (0, Vec::new());
        };
        let Some(message_id) = discord_id_to_i64(message_id, "message_id") else {
            return (0, Vec::new());
        };
        let Some(channel_id) = discord_id_to_i64(channel_id, "channel_id") else {
            return (0, Vec::new());
        };
        let Ok(window_minutes) = i32::try_from(super::RAGEBAIT_WINDOW_MINUTES) else {
            tracing::warn!("Moderation: Ragebait-Fenster passt nicht in INTEGER");
            return (0, Vec::new());
        };
        let preview: String = content.chars().take(180).collect();

        let result: Result<(i64, Vec<String>), sqlx::Error> = async {
            let mut tx = self.pool.begin().await?;
            lock_ragebait_hit_ids(&mut tx).await?;
            let hit_id = next_ragebait_hit_id(&mut tx).await?;

            sqlx::query!(
                r#"
                INSERT INTO moderation.ai_moderation_ragebait_hits(
                    id, guild_id, user_id, message_id, channel_id, content_preview, created_at
                )
                VALUES($1, $2, $3, $4, $5, $6, now())
                "#,
                hit_id,
                guild_id,
                user_id,
                message_id,
                channel_id,
                preview,
            )
            .execute(&mut *tx)
            .await?;

            let rows = sqlx::query!(
                r#"
                SELECT content_preview
                FROM moderation.ai_moderation_ragebait_hits
                WHERE user_id = $1
                  AND guild_id = $2
                  AND created_at > now() - make_interval(mins => $3::int)
                ORDER BY created_at ASC, id ASC
                "#,
                user_id,
                guild_id,
                window_minutes,
            )
            .fetch_all(&mut *tx)
            .await?;

            let previews = rows
                .into_iter()
                .map(|row| row.content_preview.unwrap_or_default())
                .collect::<Vec<_>>();
            let count = i64::try_from(previews.len()).unwrap_or(0);
            tx.commit().await?;
            Ok((count, previews))
        }
        .await;

        match result {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(%err, "Moderation: Ragebait-Hit konnte nicht persistiert werden");
                (0, Vec::new())
            }
        }
    }
}

async fn lock_ragebait_hit_ids(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        SELECT 1 AS "locked!"
        FROM pg_advisory_xact_lock($1)
        "#,
        RAGEBAIT_HITS_ID_LOCK_KEY,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

async fn next_ragebait_hit_id(tx: &mut Transaction<'_, Postgres>) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int8 AS "next_id!"
        FROM moderation.ai_moderation_ragebait_hits
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.next_id)
}

fn discord_id_to_i64(value: u64, field: &'static str) -> Option<i64> {
    match i64::try_from(value) {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::warn!(%err, field, value, "Discord-ID passt nicht in PostgreSQL BIGINT");
            None
        }
    }
}

fn db_id_to_u64(value: i64, field: &'static str) -> Option<u64> {
    match u64::try_from(value) {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::warn!(%err, field, value, "PostgreSQL BIGINT ist keine gueltige Discord-ID");
            None
        }
    }
}

fn jsonb_text_or_string(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| serde_json::Value::String(raw.to_string()).to_string())
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    async fn store() -> Result<(dl_central_db::TestDb, ModerationStore), Box<dyn std::error::Error>>
    {
        let db = dl_central_db::testing::test_pool().await?;
        let store = ModerationStore {
            pool: db.pool().clone(),
        };
        store.ensure_schema().await?;
        Ok((db, store))
    }

    fn draft() -> CaseDraft {
        CaseDraft {
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
        }
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn case_lifecycle() -> TestResult {
        let (db, store) = store().await?;
        let case_id = store.insert_case(draft()).await.expect("case insert");
        store.set_review_message(&case_id, 999).await;
        store
            .update_case_action(&case_id, "auto_delete_failed")
            .await;
        store.resolve_case(&case_id, "accepted", 777).await;

        let row = sqlx::query!(
            r#"
            SELECT action, mod_id, mod_review_message_id
            FROM moderation.ai_moderation_cases
            WHERE case_id = $1
            "#,
            &case_id,
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(row.action.as_deref(), Some("accepted"));
        assert_eq!(row.mod_id, Some(777));
        assert_eq!(row.mod_review_message_id, Some(999));

        let row = sqlx::query!(
            r#"
            SELECT
                attachments::text AS "attachments!",
                ai_raw::text AS "raw!",
                escalated_with_context
            FROM moderation.ai_moderation_cases
            WHERE message_id = $1
            "#,
            3_i64,
        )
        .fetch_one(db.pool())
        .await?;
        assert!(row.attachments.contains("https://cdn.example/image.png"));
        assert_eq!(row.raw, r#"{"response_text": "raw"}"#);
        assert_eq!(row.escalated_with_context, Some(true));

        let duplicate = sqlx::query!(
            r#"
            INSERT INTO moderation.ai_moderation_cases
                (case_id, guild_id, channel_id, message_id, user_id, created_at)
            VALUES ($1, $2, $3, $4, $5, now())
            "#,
            &case_id,
            1_i64,
            2_i64,
            3_i64,
            100_i64,
        )
        .execute(db.pool())
        .await
        .expect_err("case_id primary key must reject duplicates");
        let code = duplicate.as_database_error().and_then(|err| err.code());
        assert_eq!(code.as_deref(), Some("23505"));

        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn fetch_und_deny_mit_grund() -> TestResult {
        let (db, store) = store().await?;
        let mut draft = draft();
        draft.content = "x".into();
        draft.attachments = Vec::new();
        draft.ai_raw_json = "{}".into();
        draft.escalated_with_context = false;

        let case_id = store.insert_case(draft).await.expect("case insert");
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

        let row = sqlx::query!(
            r#"
            SELECT mod_deny_reason
            FROM moderation.ai_moderation_cases
            WHERE case_id = $1
            "#,
            &case_id,
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(row.mod_deny_reason.as_deref(), Some("kein Verstoss"));

        assert!(store.fetch_case("gibtsnicht").await.is_none());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn ragebait_fenster_zaehlt() -> TestResult {
        let (db, store) = store().await?;
        for i in 0..4 {
            let (count, previews) = store
                .insert_ragebait_hit(1, 100, 1000 + i, 2, &format!("bait {i}"))
                .await;
            assert_eq!(count, i as i64 + 1);
            assert_eq!(previews.len() as i64, count);
        }

        let row = sqlx::query!(
            r#"
            SELECT
                COUNT(*) AS "count!",
                COUNT(DISTINCT id) AS "distinct_count!"
            FROM moderation.ai_moderation_ragebait_hits
            WHERE guild_id = $1 AND user_id = $2
            "#,
            1_i64,
            100_i64,
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(row.count, 4);
        assert_eq!(row.distinct_count, 4);

        // anderer User zählt eigenes Fenster
        let (count, _) = store.insert_ragebait_hit(1, 200, 2000, 2, "x").await;
        assert_eq!(count, 1);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn ragebait_fenster_ist_guild_spezifisch() -> TestResult {
        let (_db, store) = store().await?;
        for i in 0..3 {
            let (count, _) = store
                .insert_ragebait_hit(1, 100, 3000 + i, 2, &format!("g1 bait {i}"))
                .await;
            assert_eq!(count, i as i64 + 1);
        }

        let (count, previews) = store.insert_ragebait_hit(2, 100, 4000, 3, "g2 bait").await;
        assert_eq!(count, 1);
        assert_eq!(previews, vec!["g2 bait".to_string()]);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn ragebait_fenster_zaehlt_fensterrand_strikt_nicht_mit() -> TestResult {
        let (db, store) = store().await?;
        let window_minutes = i32::try_from(crate::RAGEBAIT_WINDOW_MINUTES)?;

        sqlx::query!(
            r#"
            INSERT INTO moderation.ai_moderation_ragebait_hits(
                id, guild_id, user_id, message_id, channel_id, content_preview, created_at
            )
            VALUES
                ($1, $2, $3, $4, $5, $6, now() - make_interval(mins => $7::int)),
                ($8, $2, $3, $9, $5, $10, now() - make_interval(mins => $7::int) + interval '1 second')
            "#,
            1_i64,
            1_i64,
            100_i64,
            4998_i64,
            2_i64,
            "genau rand",
            window_minutes,
            2_i64,
            4999_i64,
            "knapp drin",
        )
        .execute(db.pool())
        .await?;

        let (count, previews) = store.insert_ragebait_hit(1, 100, 5000, 2, "neu").await;
        assert_eq!(count, 2);
        assert_eq!(previews, vec!["knapp drin".to_string(), "neu".to_string()]);
        Ok(())
    }
}
