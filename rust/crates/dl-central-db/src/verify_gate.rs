//! Persistenz des Discord Verify-Gates.
//!
//! Zwei Tabellen (Migration `2026082701_verify_gate.sql`):
//! - `bot.verify_gate_pending`: offener Zustand je Mitglied in Quarantaene
//!   (Versuche, Zustand, Frist).
//! - `bot.verify_gate_kicked`: wer ueber das Gate gekickt wurde, bekommt beim
//!   naechsten Join freien Zugang.
//!
//! Die Queries sind bewusst laufzeitgeprueft (`sqlx::query`/`query_as`), weil
//! der Offline-Cache (`.sqlx`) fuer neue Statements ohne Live-DB nicht erzeugt
//! werden kann. Das ist im Repo gaengig (siehe `testing.rs`,
//! `concierge.rs::query_scalar`).

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::CentralDbError;

/// Zustand direkt nach Quarantaene: die Frage wurde noch nicht gestartet.
pub const STATE_AWAITING_START: &str = "awaiting_start";
/// Zustand nach dem Verifizieren-Knopf: der Bot wartet auf die Hero-Antwort.
pub const STATE_AWAITING_ANSWER: &str = "awaiting_answer";

/// Offener Verify-Zustand eines Mitglieds.
#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PendingVerify {
    pub guild_id: i64,
    pub user_id: i64,
    pub dm_channel_id: Option<i64>,
    pub attempts: i32,
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
}

/// Legt einen offenen Verify-Zustand an. Idempotent: ein erneutes Join-Event
/// fuer denselben Nutzer laesst einen bestehenden Zustand (inkl. Versuche)
/// unangetastet.
pub async fn upsert_pending(
    pool: &PgPool,
    guild_id: i64,
    user_id: i64,
    dm_channel_id: Option<i64>,
    state: &str,
    deadline_at: DateTime<Utc>,
) -> Result<(), CentralDbError> {
    sqlx::query(
        r#"
        INSERT INTO bot.verify_gate_pending
            (guild_id, user_id, dm_channel_id, attempts, state, deadline_at)
        VALUES ($1, $2, $3, 0, $4, $5)
        ON CONFLICT (guild_id, user_id) DO NOTHING
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(dm_channel_id)
    .bind(state)
    .bind(deadline_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Liest den offenen Verify-Zustand, falls vorhanden.
pub async fn get_pending(
    pool: &PgPool,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<PendingVerify>, CentralDbError> {
    let row = sqlx::query_as::<_, PendingVerify>(
        r#"
        SELECT guild_id, user_id, dm_channel_id, attempts, state, created_at, deadline_at
          FROM bot.verify_gate_pending
         WHERE guild_id = $1 AND user_id = $2
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Merkt sich den DM-Kanal des Mitglieds, sobald die Start-DM zugestellt wurde.
pub async fn set_dm_channel(
    pool: &PgPool,
    guild_id: i64,
    user_id: i64,
    dm_channel_id: i64,
) -> Result<(), CentralDbError> {
    sqlx::query(
        r#"
        UPDATE bot.verify_gate_pending
           SET dm_channel_id = $3
         WHERE guild_id = $1 AND user_id = $2
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(dm_channel_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Setzt den Zustand (z. B. von `awaiting_start` auf `awaiting_answer`).
pub async fn set_state(
    pool: &PgPool,
    guild_id: i64,
    user_id: i64,
    state: &str,
) -> Result<(), CentralDbError> {
    sqlx::query(
        r#"
        UPDATE bot.verify_gate_pending
           SET state = $3
         WHERE guild_id = $1 AND user_id = $2
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(())
}

/// Erhoeht den Versuchszaehler und liefert den neuen Stand zurueck.
pub async fn incr_attempt(
    pool: &PgPool,
    guild_id: i64,
    user_id: i64,
) -> Result<i32, CentralDbError> {
    let attempts: i32 = sqlx::query_scalar(
        r#"
        UPDATE bot.verify_gate_pending
           SET attempts = attempts + 1
         WHERE guild_id = $1 AND user_id = $2
        RETURNING attempts
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(attempts)
}

/// Entfernt den offenen Verify-Zustand (nach Bestehen oder Kick).
pub async fn delete_pending(
    pool: &PgPool,
    guild_id: i64,
    user_id: i64,
) -> Result<(), CentralDbError> {
    sqlx::query(
        r#"
        DELETE FROM bot.verify_gate_pending
         WHERE guild_id = $1 AND user_id = $2
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Alle offenen Zustaende, deren Frist bis `now` abgelaufen ist.
pub async fn list_expired(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<PendingVerify>, CentralDbError> {
    let rows = sqlx::query_as::<_, PendingVerify>(
        r#"
        SELECT guild_id, user_id, dm_channel_id, attempts, state, created_at, deadline_at
          FROM bot.verify_gate_pending
         WHERE deadline_at <= $1
         ORDER BY deadline_at ASC
        "#,
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Vermerkt einen ueber das Gate gekickten Account (fuer den Rejoin-Freipass).
pub async fn mark_kicked(
    pool: &PgPool,
    guild_id: i64,
    user_id: i64,
) -> Result<(), CentralDbError> {
    sqlx::query(
        r#"
        INSERT INTO bot.verify_gate_kicked (guild_id, user_id)
        VALUES ($1, $2)
        ON CONFLICT (guild_id, user_id) DO NOTHING
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Wurde dieser Account schon einmal ueber das Gate gekickt?
pub async fn was_kicked(
    pool: &PgPool,
    guild_id: i64,
    user_id: i64,
) -> Result<bool, CentralDbError> {
    let found: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
              FROM bot.verify_gate_kicked
             WHERE guild_id = $1 AND user_id = $2
        )
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(found)
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use crate::testing::test_pool;
    use chrono::Duration;

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN"]
    async fn insert_select_and_expiry() {
        let db = test_pool().await.expect("test pool");
        let pool: &PgPool = &db;
        let guild = 1289721245281292288_i64;
        let user = 424242_i64;
        let now = Utc::now();

        // Frist in der Zukunft -> nicht abgelaufen.
        upsert_pending(pool, guild, user, None, STATE_AWAITING_START, now + Duration::hours(24))
            .await
            .expect("upsert");
        let pending = get_pending(pool, guild, user).await.expect("get").expect("row");
        assert_eq!(pending.attempts, 0);
        assert_eq!(pending.state, STATE_AWAITING_START);
        assert!(list_expired(pool, now).await.expect("expired").is_empty());

        // Idempotenz: zweiter Upsert laesst den Zustand unveraendert.
        set_state(pool, guild, user, STATE_AWAITING_ANSWER).await.expect("state");
        let bumped = incr_attempt(pool, guild, user).await.expect("incr");
        assert_eq!(bumped, 1);
        upsert_pending(pool, guild, user, None, STATE_AWAITING_START, now + Duration::hours(1))
            .await
            .expect("upsert idempotent");
        let again = get_pending(pool, guild, user).await.expect("get").expect("row");
        assert_eq!(again.attempts, 1, "Versuche bleiben erhalten");
        assert_eq!(again.state, STATE_AWAITING_ANSWER, "Zustand bleibt erhalten");

        // Abgelaufene Frist wird gelistet.
        set_state(pool, guild, user, STATE_AWAITING_ANSWER).await.expect("state");
        sqlx::query("UPDATE bot.verify_gate_pending SET deadline_at = $3 WHERE guild_id = $1 AND user_id = $2")
            .bind(guild)
            .bind(user)
            .bind(now - Duration::minutes(5))
            .execute(pool)
            .await
            .expect("expire");
        let expired = list_expired(pool, now).await.expect("expired");
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].user_id, user);

        // Kick-Liste.
        assert!(!was_kicked(pool, guild, user).await.expect("was_kicked"));
        mark_kicked(pool, guild, user).await.expect("mark");
        assert!(was_kicked(pool, guild, user).await.expect("was_kicked"));

        delete_pending(pool, guild, user).await.expect("delete");
        assert!(get_pending(pool, guild, user).await.expect("get").is_none());
    }
}
