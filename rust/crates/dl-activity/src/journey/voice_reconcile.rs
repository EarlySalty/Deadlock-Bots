//! Abschluss verwaister offener Metadaten-Sessions ohne Änderung geschlossener History.

use super::*;
use dl_discord::voice_cache::GuildVoiceSnapshot;

/// Schließt offene Sessions leerer oder verschwundener Kanäle am letzten
/// belegten Zeitpunkt. Die Journey schreibt dabei wie beim normalen Leave in
/// voice_metadata_events, nicht in den getrennten, punktebasierten Session-Log.
pub async fn reconcile_voice_open_sessions(
    pool: &PgPool,
    snapshot: &GuildVoiceSnapshot,
) -> ActivityDbResult<u64> {
    let guild_id = discord_id_to_i64(snapshot.guild_id, "voice_open_sessions.guild_id")?;
    let occupied: Vec<i64> = snapshot
        .members
        .values()
        .copied()
        .map(|id| discord_id_to_i64(id, "voice_open_sessions.channel_id"))
        .collect::<Result<_, _>>()?;
    let observed_at = snapshot.observed_at.min(Utc::now());
    let candidates = sqlx::query(
        "SELECT user_id, channel_id, joined_at, updated_at
         FROM activity.voice_open_sessions
         WHERE guild_id = $1 AND NOT (channel_id = ANY($2)) AND updated_at <= $3
         ORDER BY user_id",
    )
    .bind(guild_id)
    .bind(&occupied)
    .bind(observed_at)
    .fetch_all(pool)
    .await?;
    let mut closed = 0;
    for row in candidates {
        let user_id: i64 = row.try_get("user_id")?;
        let channel_id: i64 = row.try_get("channel_id")?;
        let joined_at: DateTime<Utc> = row.try_get("joined_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;
        let mut tx = pool.begin().await?;
        // Gleiche Sperrreihenfolge wie Event-Ingestion und Privacy-Erase.
        let opted_out = dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, user_id).await?;
        let removed = sqlx::query(
            "DELETE FROM activity.voice_open_sessions
             WHERE user_id = $1 AND guild_id = $2 AND channel_id = $3
               AND joined_at = $4 AND updated_at = $5",
        )
        .bind(user_id)
        .bind(guild_id)
        .bind(channel_id)
        .bind(joined_at)
        .bind(updated_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if removed != 0 {
            if !opted_out {
                let ended_at = updated_at.min(observed_at);
                insert_voice_metadata_tx(
                    &mut tx,
                    user_id,
                    guild_id,
                    channel_id,
                    "leave",
                    ended_at,
                    Some(duration_seconds(joined_at, ended_at)),
                    None,
                    None,
                )
                .await?;
            }
            closed += 1;
        }
        tx.commit().await?;
    }

    // Ein Heartbeat gilt für die tatsächlich beobachtete User/Kanal-Paarung.
    // Keine Verlängerung eines Geists, dessen alter Kanal inzwischen besetzt ist.
    let mut users = Vec::with_capacity(snapshot.members.len());
    let mut channels = Vec::with_capacity(snapshot.members.len());
    for (&user, &channel) in &snapshot.members {
        users.push(discord_id_to_i64(user, "voice_open_sessions.user_id")?);
        channels.push(discord_id_to_i64(
            channel,
            "voice_open_sessions.channel_id",
        )?);
    }
    sqlx::query(
        "UPDATE activity.voice_open_sessions AS session SET updated_at = $4
         FROM UNNEST($2::bigint[], $3::bigint[]) AS live(user_id, channel_id)
         WHERE session.guild_id = $1 AND session.user_id = live.user_id
           AND session.channel_id = live.channel_id AND session.updated_at < $4
           AND NOT EXISTS (SELECT 1 FROM core.user_privacy privacy
               WHERE privacy.user_id = session.user_id
                 AND (privacy.opted_out = TRUE OR privacy.deleted_at IS NOT NULL))",
    )
    .bind(guild_id)
    .bind(users)
    .bind(channels)
    .bind(observed_at)
    .execute(pool)
    .await?;
    Ok(closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn reconcile_einmal_sql_ist_eng_begrenzt_und_wiederholbar() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        sqlx::query("INSERT INTO activity.voice_open_sessions(user_id, guild_id, channel_id, joined_at, updated_at)
            VALUES(754808003257172048, 1289721245281292288, 1470126503252721845,
            '2026-08-02 20:44:20.594256Z', '2026-08-02 20:46:20.594256Z'),
            (100, 1289721245281292288, 1470126503252721845, '2026-08-02 20:44:20.594256Z', '2026-08-02 20:46:20.594256Z')")
            .execute(db.pool()).await.expect("seed");
        sqlx::query("INSERT INTO activity.voice_metadata_events(user_id, guild_id, channel_id, event_type, occurred_at, duration_seconds)
            VALUES(999, 1, 10, 'leave', '2026-08-01 20:00Z', 321)")
            .execute(db.pool()).await.expect("closed sentinel");
        let repair = include_str!("../../../../ops/repair_voice_open_session_20260802.sql");
        sqlx::raw_sql(repair)
            .execute(db.pool())
            .await
            .expect("repair");
        sqlx::raw_sql(repair)
            .execute(db.pool())
            .await
            .expect("repeat");
        let remaining: Vec<i64> =
            sqlx::query_scalar("SELECT user_id FROM activity.voice_open_sessions")
                .fetch_all(db.pool())
                .await
                .expect("remaining");
        assert_eq!(remaining, vec![100]);
        let rows: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT user_id, duration_seconds FROM activity.voice_metadata_events ORDER BY id",
        )
        .fetch_all(db.pool())
        .await
        .expect("closed");
        assert_eq!(rows, vec![(999, 321), (754808003257172048, 120)]);
        // Eine inzwischen beobachtete, gleich alte Session wird nicht repariert.
        sqlx::query("INSERT INTO activity.voice_open_sessions(user_id, guild_id, channel_id, joined_at, updated_at)
            VALUES(754808003257172048, 1289721245281292288, 1470126503252721845,
            '2026-08-02 20:44:20.594256Z', '2026-09-21 12:00Z')")
            .execute(db.pool()).await.expect("renewed");
        sqlx::raw_sql(repair)
            .execute(db.pool())
            .await
            .expect("protected");
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM activity.voice_open_sessions")
                .fetch_one(db.pool())
                .await
                .expect("remaining");
        assert_eq!(remaining, 2);
    }

    #[tokio::test]
    async fn reconcile_opt_out_entfernt_offen_ohne_history_aufzufuellen() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        sqlx::query("INSERT INTO core.user_privacy(user_id, opted_out) VALUES(100, TRUE)")
            .execute(db.pool())
            .await
            .expect("privacy");
        sqlx::query("INSERT INTO activity.voice_open_sessions(user_id, guild_id, channel_id, joined_at, updated_at)
            VALUES(100, 42, 10, '2026-08-02 20:00Z', '2026-08-02 20:01Z')")
            .execute(db.pool()).await.expect("seed");
        let mut snapshot = GuildVoiceSnapshot {
            guild_id: 42,
            observed_at: Utc::now(),
            channels: HashMap::from([(10, None)]),
            members: HashMap::from([(100, 10)]),
        };
        assert_eq!(
            reconcile_voice_open_sessions(db.pool(), &snapshot)
                .await
                .expect("occupied"),
            0
        );
        let updated: DateTime<Utc> = sqlx::query_scalar(
            "SELECT updated_at FROM activity.voice_open_sessions WHERE user_id = 100",
        )
        .fetch_one(db.pool())
        .await
        .expect("no heartbeat for opt-out");
        assert_eq!(updated.to_rfc3339(), "2026-08-02T20:01:00+00:00");
        snapshot.members.clear();
        assert_eq!(
            reconcile_voice_open_sessions(db.pool(), &snapshot)
                .await
                .expect("reconcile"),
            1
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM activity.voice_metadata_events")
            .fetch_one(db.pool())
            .await
            .expect("no refill");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn reconcile_schliesst_leer_und_fehlend_am_updated_at_idempotent() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO activity.voice_open_sessions
             (user_id, guild_id, channel_id, joined_at, updated_at) VALUES
             (100, 42, 10, '2026-08-02 20:00Z', '2026-08-02 20:01Z'),
             (101, 42, 11, '2026-08-02 20:00Z', '2026-08-02 20:02Z'),
             (102, 42, 12, '2026-08-02 20:00Z', '2026-08-02 20:03Z'),
             (103, 43, 10, '2026-08-02 20:00Z', '2026-08-02 20:04Z')",
        )
        .execute(pool)
        .await
        .expect("seed");
        let snapshot = GuildVoiceSnapshot {
            guild_id: 42,
            observed_at: Utc::now(),
            channels: HashMap::from([(10, None), (12, None)]),
            members: HashMap::from([(102, 12)]),
        };
        assert_eq!(
            reconcile_voice_open_sessions(pool, &snapshot)
                .await
                .expect("reconcile"),
            2
        );
        assert_eq!(
            reconcile_voice_open_sessions(pool, &snapshot)
                .await
                .expect("repeat"),
            0
        );
        let rows: Vec<(i64, i64, DateTime<Utc>)> = sqlx::query_as(
            "SELECT user_id, duration_seconds, occurred_at
             FROM activity.voice_metadata_events WHERE event_type = 'leave' ORDER BY user_id",
        )
        .fetch_all(pool)
        .await
        .expect("closed");
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].0, rows[0].1), (100, 60));
        assert_eq!((rows[1].0, rows[1].1), (101, 120));
        assert_eq!(rows[0].2.to_rfc3339(), "2026-08-02T20:01:00+00:00");
        let remaining: Vec<i64> =
            sqlx::query_scalar("SELECT user_id FROM activity.voice_open_sessions ORDER BY user_id")
                .fetch_all(pool)
                .await
                .expect("remaining");
        assert_eq!(remaining, vec![102, 103]);
        let logs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM activity.voice_session_log")
            .fetch_one(pool)
            .await
            .expect("separate history");
        assert_eq!(
            logs, 0,
            "Journey darf keinen zweiten Session-Recorder erfinden"
        );
    }

    #[tokio::test]
    async fn reconcile_schuetzt_neuere_zeile_und_geist_im_besetzten_kanal() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO activity.voice_open_sessions
             (user_id, guild_id, channel_id, joined_at, updated_at) VALUES
             (100, 42, 10, $1, $2), (101, 42, 11, $1, $1)",
        )
        .bind(now - chrono::Duration::minutes(5))
        .bind(now + chrono::Duration::seconds(1))
        .execute(db.pool())
        .await
        .expect("seed");
        let snapshot = GuildVoiceSnapshot {
            guild_id: 42,
            observed_at: now,
            channels: HashMap::from([(10, None), (11, None)]),
            members: HashMap::from([(999, 11)]),
        };
        assert_eq!(
            reconcile_voice_open_sessions(db.pool(), &snapshot)
                .await
                .expect("reconcile"),
            0
        );
        let updated: DateTime<Utc> = sqlx::query_scalar(
            "SELECT updated_at FROM activity.voice_open_sessions WHERE user_id = 101",
        )
        .fetch_one(db.pool())
        .await
        .expect("unchanged");
        // Postgres speichert Mikrosekunden, chrono kann Nanosekunden enthalten.
        assert_eq!(
            updated.timestamp_micros(),
            (now - chrono::Duration::minutes(5)).timestamp_micros()
        );
    }
}
