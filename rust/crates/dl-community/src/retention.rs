//! Retention-Tracking — Daten-Layer (Port von `cogs/user_retention.py`).
//!
//! Schreibt `user_retention_tracking`: bei Voice-Join `last_active_at` +
//! `total_active_days` (nur ein neuer Tag zählt), und alle 30 min
//! `avg_weekly_sessions` aus `voice_session_log`. **Cutover-kritisch:** die
//! Leave-Survey-Einstufung (Bucket A/B/C, [`crate::leave_survey`]) LIEST
//! `avg_weekly_sessions` hier — ohne diesen Schreiber veraltet die Quelle.
//!
//! Die „Wir-vermissen-dich"-DM (`daily_retention_check`) ist hier mit portiert:
//! ein stündlicher Loop löst genau zur Check-Stunde (12 UTC, max. 1×/Tag) eine
//! Suche nach inaktiven Stamm-Usern aus und schickt ihnen eine Embed-DM mit
//! Link-Buttons + Feedback-Button. Der Feedback-Button öffnet ein Modal, dessen
//! Antwort als `message_type='feedback'` in `user_retention_messages` landet.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Timelike, Utc};
use dl_central_db::kv;
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::db::{
    advisory_lock, i64_to_i32, i64_to_u64, u64_to_i64, utc_from_unix, CommunityDbResult,
};

const SYNC_INTERVAL: Duration = Duration::from_secs(30 * 60);
const LOOKBACK_DAYS: i64 = 60;

// ── Miss-You-Konfiguration (Python `RetentionConfig`) ──────────────────────
const MIN_WEEKLY_SESSIONS: f64 = 0.5;
const MIN_TOTAL_ACTIVE_DAYS: i64 = 3;
const INACTIVITY_THRESHOLD_DAYS: i64 = 14;
const MIN_DAYS_BETWEEN_MESSAGES: i64 = 30;
const MAX_MISS_YOU_PER_USER: i64 = 1;
const CHECK_HOUR: u32 = 12;
// Genug Vorlauf, damit entfernte Mitglieder nicht das Zustelllimit blockieren.
const MISS_YOU_CANDIDATE_FETCH_LIMIT: i64 = 500;
const MAX_MISS_YOU_DELIVERIES_PER_RUN: usize = 50;
// Bewusst grobe Untergrenze gegen leere/halb gefüllte Caches nach Bot-Neustarts, kein Feintuning.
const MIN_PLAUSIBLE_GUILD_CACHE_MEMBERS: usize = 100;
/// Log-Meldung beim Abbruch wegen unvollständigem Guild-Cache. Als Konstante,
/// damit Tests auf die Meldung prüfen können, ohne am Wortlaut zu kleben.
pub const MISS_YOU_CACHE_ABORT_LOG: &str = "⚠️ Miss-You-Lauf abgebrochen: Der Server-Cache ist unvollständig, deshalb wurde heute niemand angeschrieben. Das repariert sich normalerweise von selbst, sobald der Bot durchgelaufen ist.";
const SERVER_LINK: &str = "https://discord.com/channels/1289721245281292288/1289721245281292291";
const VOICE_LINK: &str = "https://discord.com/channels/1289721245281292288/1501089974093873232";
const EXCLUDED_ROLE_IDS: [u64; 2] = [1304416311383818240, 1309741866098491479];
const RETENTION_MESSAGES_ID_LOCK: i64 = 0x4451_0008_0010_0002;
pub const LOG_CHANNEL_ID: u64 = 1374364800817303632;

/// Mitgliedsinfo für die Miss-You-Auswahl (Anzeigename + Rollen).
pub struct RetentionMember {
    pub display_name: String,
    pub role_ids: Vec<u64>,
}

/// Zustellergebnis der Miss-You-DM (→ `delivery_status`).
pub enum MissYouDelivery {
    Sent,
    Blocked,
    Failed(String),
}

/// Discord-Anbindung der Miss-You-DM (vom Bot über den Cache/HTTP erfüllt).
#[async_trait::async_trait]
pub trait RetentionPort: Send + Sync {
    /// Anzahl der im Guild-Cache vorhandenen Mitglieder; `None`, wenn die Guild fehlt.
    async fn guild_member_count(&self, guild_id: u64) -> Option<usize>;
    /// Ist der User aktuell Mitglied der Guild?
    async fn is_guild_member(&self, guild_id: u64, user_id: u64) -> bool;
    /// Anzeigename + Rollen aus dem Guild-Cache; `None`, wenn kein Mitglied.
    async fn member_info(&self, guild_id: u64, user_id: u64) -> Option<RetentionMember>;
    /// Fallback-Name via `fetch_user` (`global_name` | `name`).
    async fn fetch_user_name(&self, user_id: u64) -> Option<String>;
    /// Guild-Name + optionales Icon (Thumbnail-URL).
    async fn guild_label(&self, guild_id: u64) -> (String, Option<String>);
    /// DM mit Embed + Components senden; meldet den Zustellstatus.
    async fn send_miss_you_dm(
        &self,
        user_id: u64,
        embed: Value,
        components: Value,
    ) -> MissYouDelivery;
    /// Zusammenfassung eines Miss-You-Laufs in den Bot-Log-Channel senden.
    async fn send_log(&self, text: String);
}

#[derive(Clone)]
pub struct RetentionTracker {
    pool: PgPool,
}

#[derive(Default)]
struct MissYouRunStats {
    attempted: usize,
    delivered: usize,
    blocked: usize,
    not_member: usize,
    errors: usize,
}

enum MissYouDecision {
    Sent,
    Blocked,
    PermanentFailed,
    TransientFailed,
    Skipped,
}

impl RetentionTracker {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Schema gehört zentral `0010_activity_moderation_content_patchnotes.sql`.
    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            SELECT to_regclass('activity.user_retention_tracking') IS NOT NULL AS "exists!"
            "#
        )
        .fetch_one(&self.pool)
        .await
        .map(|_| ())
    }

    /// Voice-Join/-Wechsel: `last_active_at` + `total_active_days` (nur ein
    /// neuer UTC-Tag erhöht den Zähler). Gated auf `user_privacy`-Opt-out.
    pub async fn update_user_activity(
        &self,
        user_id: u64,
        guild_id: u64,
        now: i64,
        today: String,
    ) -> CommunityDbResult<()> {
        let user_id = u64_to_i64(user_id, "user_id")?;
        let guild_id = u64_to_i64(guild_id, "guild_id")?;
        let mut tx = self.pool.begin().await?;
        if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, user_id).await? {
            tracing::info!(
                writer = "activity.user_retention_tracking",
                user_id,
                "übersprungen wegen Opt-out"
            );
            tx.commit().await?;
            return Ok(());
        }
        let now_dt = utc_from_unix(now)?;
        let row = sqlx::query!(
            r#"
            SELECT last_active_at, total_active_days
              FROM activity.user_retention_tracking
             WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        match row {
            Some(row) => {
                let last_date = row.last_active_at.format("%Y-%m-%d").to_string();
                let new_total = if last_date != today {
                    row.total_active_days + 1
                } else {
                    row.total_active_days
                };
                sqlx::query!(
                    r#"
                    UPDATE activity.user_retention_tracking
                       SET last_active_at = $1,
                           total_active_days = $2,
                           updated_at = $1
                     WHERE user_id = $3
                    "#,
                    now_dt,
                    new_total,
                    user_id,
                )
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query!(
                    r#"
                    INSERT INTO activity.user_retention_tracking(
                        user_id, guild_id, first_seen_at, last_active_at, total_active_days, updated_at
                    )
                    VALUES ($1, $2, $3, $3, 1, $3)
                    "#,
                    user_id,
                    guild_id,
                    now_dt,
                )
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// 30-min-Lauf: `avg_weekly_sessions` aus `voice_session_log` (letzte 60
    /// Tage). `weeks_active = max(1, (now-first_session)/Woche)`. Gibt die Zahl
    /// aktualisierter User zurück.
    pub async fn sync_activity_data(&self, now: i64) -> CommunityDbResult<usize> {
        let now_dt = utc_from_unix(now)?;
        let cutoff = now_dt - chrono::Duration::days(LOOKBACK_DAYS);
        let agg = sqlx::query!(
            r#"
            SELECT user_id,
                   guild_id,
                   COUNT(DISTINCT (started_at AT TIME ZONE 'UTC')::date)::BIGINT AS "active_days!",
                   COUNT(*)::BIGINT AS "total_sessions!",
                   MIN(started_at) AS "first_at?",
                   MAX(started_at) AS "last_at?"
              FROM activity.voice_session_log
             WHERE started_at > $1
             GROUP BY user_id, guild_id
            "#,
            cutoff,
        )
        .fetch_all(&self.pool)
        .await?;
        let count = agg.len();
        for row in agg {
            let first_at = row.first_at.unwrap_or(now_dt);
            let last_at = row.last_at.unwrap_or(now_dt);
            let weeks =
                ((now_dt.timestamp() - first_at.timestamp()) as f64 / (7.0 * 86400.0)).max(1.0);
            let avg_weekly = row.total_sessions as f64 / weeks;
            let active_days = i64_to_i32(row.active_days, "active_days")?;
            sqlx::query!(
                r#"
                INSERT INTO activity.user_retention_tracking(
                    user_id, guild_id, first_seen_at, last_active_at,
                    total_active_days, avg_weekly_sessions, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT(user_id) DO UPDATE SET
                  last_active_at = GREATEST(activity.user_retention_tracking.last_active_at, excluded.last_active_at),
                  total_active_days = GREATEST(activity.user_retention_tracking.total_active_days, excluded.total_active_days),
                  avg_weekly_sessions = excluded.avg_weekly_sessions,
                  updated_at = excluded.updated_at
                "#,
                row.user_id,
                row.guild_id,
                first_at,
                last_at,
                active_days,
                avg_weekly,
                now_dt,
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(count)
    }

    /// Opt-out setzen (Python `retention_optout`): legt den Tracking-Eintrag
    /// bei Bedarf an und setzt `opted_out=1`. Der Miss-You-Sender liest die
    /// Spalte bereits (siehe [`Self::find_inactive_users`]).
    pub async fn set_opted_out(&self, user_id: u64, guild_id: u64) -> CommunityDbResult<()> {
        let user_id = u64_to_i64(user_id, "user_id")?;
        let guild_id = u64_to_i64(guild_id, "guild_id")?;
        let now = Utc::now();
        sqlx::query!(
            r#"
            INSERT INTO activity.user_retention_tracking(
                user_id, guild_id, first_seen_at, last_active_at, total_active_days, opted_out, updated_at
            )
            VALUES ($1, $2, $3, $3, 0, TRUE, $3)
            ON CONFLICT(user_id) DO UPDATE SET
              opted_out = TRUE,
              updated_at = excluded.updated_at
            "#,
            user_id,
            guild_id,
            now,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Opt-in (Python `retention_optin`): setzt `opted_out=0` für einen
    /// bestehenden Eintrag. Wie das Original wird hier nichts neu angelegt.
    pub async fn clear_opted_out(&self, user_id: u64) -> CommunityDbResult<()> {
        let user_id = u64_to_i64(user_id, "user_id")?;
        let now = Utc::now();
        sqlx::query!(
            r#"
            UPDATE activity.user_retention_tracking
               SET opted_out = FALSE,
                   updated_at = $1
             WHERE user_id = $2
            "#,
            now,
            user_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Inaktive Stamm-User (Python `_find_inactive_regular_users`): regelmäßig
    /// aktiv gewesen, jetzt über der Schwelle inaktiv, nicht opted-out,
    /// Spam-Schutz greift. Liefert `(user_id, guild_id, days_inactive)`.
    async fn find_inactive_users(&self, now: i64) -> Vec<(u64, u64, i64)> {
        let Ok(now_dt) = utc_from_unix(now) else {
            return Vec::new();
        };
        let inactivity_threshold = now_dt - chrono::Duration::days(INACTIVITY_THRESHOLD_DAYS);
        let min_gap = now_dt - chrono::Duration::days(MIN_DAYS_BETWEEN_MESSAGES);
        let rows = sqlx::query!(
            r#"
            SELECT user_id,
                   guild_id,
                   FLOOR(EXTRACT(EPOCH FROM ($1 - last_active_at)) / 86400)::BIGINT AS "days_inactive!"
              FROM activity.user_retention_tracking
             WHERE avg_weekly_sessions >= $2
               AND total_active_days >= $3
               AND last_active_at < $4
               AND opted_out = FALSE
               AND miss_you_count < $5
               AND (last_miss_you_sent_at IS NULL OR last_miss_you_sent_at < $6)
             ORDER BY 3 DESC
             LIMIT $7
            "#,
            now_dt,
            MIN_WEEKLY_SESSIONS,
            i64_to_i32(MIN_TOTAL_ACTIVE_DAYS, "MIN_TOTAL_ACTIVE_DAYS").unwrap_or(i32::MAX),
            inactivity_threshold,
            i64_to_i32(MAX_MISS_YOU_PER_USER, "MAX_MISS_YOU_PER_USER").unwrap_or(i32::MAX),
            min_gap,
            MISS_YOU_CANDIDATE_FETCH_LIMIT,
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        rows.into_iter()
            .filter_map(|row| {
                Some((
                    i64_to_u64(row.user_id, "user_id")?,
                    i64_to_u64(row.guild_id, "guild_id")?,
                    row.days_inactive,
                ))
            })
            .collect()
    }

    /// Stündlicher Miss-You-Check (Python `daily_retention_check`): nur zur
    /// Check-Stunde (12 UTC) und max. 1×/Tag. Das Datum liegt im `kv_store`,
    /// damit ein Neustart am Check-Tag nicht doppelt sendet.
    pub async fn run_miss_you_check(&self, port: &Arc<dyn RetentionPort>) {
        let now_dt = chrono::Utc::now();
        if now_dt.hour() != CHECK_HOUR {
            return;
        }
        let today = now_dt.format("%Y-%m-%d").to_string();
        if kv::get(&self.pool, "retention", "last_check_date")
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some(today.as_str())
        {
            return;
        }
        let _ = kv::set(&self.pool, "retention", "last_check_date", &today).await;

        self.run_miss_you_candidates(port, now_dt.timestamp()).await;
    }

    async fn run_miss_you_candidates(&self, port: &Arc<dyn RetentionPort>, now: i64) {
        let mut stats = MissYouRunStats::default();
        let candidates = self.find_inactive_users(now).await;
        for &(_, guild_id, _) in &candidates {
            let member_count = port.guild_member_count(guild_id).await;
            if !matches!(
                member_count,
                Some(count) if count >= MIN_PLAUSIBLE_GUILD_CACHE_MEMBERS
            ) {
                tracing::warn!(
                    guild_id,
                    member_count,
                    entscheidung = "abgebrochen",
                    grund = "cache_unplausibel",
                    "Miss-You-Lauf abgebrochen"
                );
                port.send_log(MISS_YOU_CACHE_ABORT_LOG.to_string()).await;
                return;
            }
        }

        for (user_id, guild_id, days_inactive) in candidates {
            if !port.is_guild_member(guild_id, user_id).await {
                tracing::info!(
                    user_id,
                    guild_id,
                    entscheidung = "uebersprungen",
                    grund = "cache_unsicher",
                    "Miss-You-Zustellentscheidung"
                );
                stats.not_member += 1;
                continue;
            }

            if stats.attempted >= MAX_MISS_YOU_DELIVERIES_PER_RUN {
                break;
            }
            match self
                .send_miss_you(port, user_id, guild_id, days_inactive)
                .await
            {
                MissYouDecision::Sent => {
                    stats.attempted += 1;
                    stats.delivered += 1;
                }
                MissYouDecision::Blocked => {
                    stats.attempted += 1;
                    stats.blocked += 1;
                }
                MissYouDecision::PermanentFailed | MissYouDecision::TransientFailed => {
                    stats.attempted += 1;
                    stats.errors += 1;
                }
                MissYouDecision::Skipped => {}
            }
        }
        let delivery_rate = (stats.delivered * 100)
            .checked_div(stats.attempted)
            .unwrap_or(100);
        let warning = if stats.attempted > 0 && delivery_rate < 50 {
            " ⚠️ WARNUNG: Zustellquote unter 50 %."
        } else {
            ""
        };
        port.send_log(format!(
            "🔔 Miss-You-Lauf — versucht: {}, zugestellt: {}, DMs zu: {}, nicht mehr auf dem Server: {}, Fehler: {}, Zustellquote: {} %.{}",
            stats.attempted,
            stats.delivered,
            stats.blocked,
            stats.not_member,
            stats.errors,
            delivery_rate,
            warning,
        ))
        .await;
    }

    /// Eine Miss-You-DM aufbauen, senden und das Ergebnis protokollieren
    /// (Python `_send_miss_you_message`).
    async fn send_miss_you(
        &self,
        port: &Arc<dyn RetentionPort>,
        user_id: u64,
        guild_id: u64,
        days_inactive: i64,
    ) -> MissYouDecision {
        // Globaler Privacy-Opt-out hat Vorrang vor der Retention-Spalte.
        let Ok(user_id_i64) = u64_to_i64(user_id, "user_id") else {
            return MissYouDecision::Skipped;
        };
        let Ok(guild_id_i64) = u64_to_i64(guild_id, "guild_id") else {
            return MissYouDecision::Skipped;
        };
        if crate::privacy::is_opted_out(&self.pool, user_id_i64).await {
            tracing::info!(
                writer = "activity.user_retention_messages",
                user_id = user_id_i64,
                entscheidung = "übersprungen",
                grund = "privacy_opt_out",
                "übersprungen wegen Opt-out"
            );
            return MissYouDecision::Skipped;
        }
        // Name + Excluded-Rollen aus dem Cache; ausgeschlossene Rollen → kein DM.
        let member = port.member_info(guild_id, user_id).await;
        if let Some(m) = &member {
            if m.role_ids.iter().any(|r| EXCLUDED_ROLE_IDS.contains(r)) {
                tracing::info!(
                    user_id,
                    guild_id,
                    entscheidung = "übersprungen",
                    grund = "ausgeschlossene_rolle",
                    "Miss-You-Zustellentscheidung"
                );
                return MissYouDecision::Skipped;
            }
        }
        let display_name = match member.map(|m| m.display_name) {
            Some(name) => name,
            None => port
                .fetch_user_name(user_id)
                .await
                .unwrap_or_else(|| "Unbekannt".to_string()),
        };
        let (guild_name, icon) = port.guild_label(guild_id).await;

        let embed = miss_you_embed(&display_name, days_inactive, &guild_name, icon.as_deref());
        let delivery = port
            .send_miss_you_dm(user_id, embed, miss_you_components(guild_id))
            .await;
        let now = chrono::Utc::now();
        match delivery {
            MissYouDelivery::Sent => {
                if let Err(err) = self.update_miss_you_sent(user_id_i64, now).await {
                    tracing::warn!(%err, user_id, "Miss-You-Zähler konnte nicht aktualisiert werden");
                }
                if let Err(err) = self
                    .insert_retention_message(
                        user_id_i64,
                        guild_id_i64,
                        "miss_you",
                        now,
                        "sent",
                        None,
                    )
                    .await
                {
                    tracing::warn!(%err, user_id, "Miss-You-Erfolg konnte nicht protokolliert werden");
                }
                tracing::info!(
                    user_id,
                    guild_id,
                    entscheidung = "zugestellt",
                    grund = "discord_dm_sent",
                    "Miss-You-Zustellentscheidung"
                );
                MissYouDecision::Sent
            }
            MissYouDelivery::Blocked => {
                if let Err(err) = self.mark_miss_you_terminal(user_id_i64, now).await {
                    tracing::warn!(%err, user_id, "Miss-You-Zähler konnte nicht aktualisiert werden");
                }
                if let Err(err) = self
                    .insert_retention_message(
                        user_id_i64,
                        guild_id_i64,
                        "miss_you",
                        now,
                        "blocked",
                        Some("DMs disabled".to_string()),
                    )
                    .await
                {
                    tracing::warn!(%err, user_id, "Miss-You-Blockierung konnte nicht protokolliert werden");
                }
                tracing::info!(
                    user_id,
                    guild_id,
                    entscheidung = "blocked",
                    grund = "dms_disabled",
                    "Miss-You-Zustellentscheidung"
                );
                MissYouDecision::Blocked
            }
            MissYouDelivery::Failed(err) => {
                if is_blocked_delivery_error(&err) {
                    if let Err(db_err) = self.mark_miss_you_terminal(user_id_i64, now).await {
                        tracing::warn!(%db_err, user_id, "Miss-You-Zähler konnte nicht aktualisiert werden");
                    }
                    if let Err(db_err) = self
                        .insert_retention_message(
                            user_id_i64,
                            guild_id_i64,
                            "miss_you",
                            now,
                            "blocked",
                            Some(err.clone()),
                        )
                        .await
                    {
                        tracing::warn!(%db_err, user_id, "Miss-You-Blockierung konnte nicht protokolliert werden");
                    }
                    tracing::info!(
                        user_id,
                        guild_id,
                        entscheidung = "blocked",
                        grund = %err,
                        "Miss-You-Zustellentscheidung"
                    );
                    return MissYouDecision::Blocked;
                }

                let permanent = is_permanent_delivery_error(&err);
                if permanent {
                    if let Err(db_err) = self.mark_miss_you_terminal(user_id_i64, now).await {
                        tracing::warn!(%db_err, user_id, "Miss-You-Zähler konnte nicht aktualisiert werden");
                    }
                }
                if let Err(db_err) = self
                    .insert_retention_message(
                        user_id_i64,
                        guild_id_i64,
                        "miss_you",
                        now,
                        "failed",
                        Some(err.clone()),
                    )
                    .await
                {
                    tracing::warn!(%db_err, user_id, "Miss-You-Fehler konnte nicht protokolliert werden");
                }
                if permanent {
                    tracing::warn!(
                        user_id,
                        guild_id,
                        entscheidung = "permanent_failed",
                        grund = %err,
                        "Miss-You-Zustellentscheidung"
                    );
                    MissYouDecision::PermanentFailed
                } else {
                    tracing::warn!(
                        user_id,
                        guild_id,
                        entscheidung = "transient_failed",
                        grund = %err,
                        "Miss-You-Zustellentscheidung"
                    );
                    MissYouDecision::TransientFailed
                }
            }
        }
    }

    async fn mark_miss_you_terminal(
        &self,
        user_id: i64,
        now: DateTime<Utc>,
    ) -> CommunityDbResult<()> {
        let mut tx = self.pool.begin().await?;
        if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, user_id).await? {
            tx.commit().await?;
            return Ok(());
        }
        sqlx::query(
            r#"
            UPDATE activity.user_retention_tracking
               SET miss_you_count = miss_you_count + 1,
                   updated_at = $1
             WHERE user_id = $2
            "#,
        )
        .bind(now)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn update_miss_you_sent(
        &self,
        user_id: i64,
        now: DateTime<Utc>,
    ) -> CommunityDbResult<()> {
        let mut tx = self.pool.begin().await?;
        if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, user_id).await? {
            tracing::info!(
                writer = "activity.user_retention_tracking.miss_you",
                user_id,
                "übersprungen wegen Opt-out"
            );
            tx.commit().await?;
            return Ok(());
        }
        sqlx::query!(
            r#"
                    UPDATE activity.user_retention_tracking
                       SET last_miss_you_sent_at = $1,
                           miss_you_count = miss_you_count + 1,
                           updated_at = $1
                     WHERE user_id = $2
                    "#,
            now,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn insert_retention_message(
        &self,
        user_id: i64,
        guild_id: i64,
        message_type: &str,
        sent_at: DateTime<Utc>,
        delivery_status: &str,
        error_message: Option<String>,
    ) -> CommunityDbResult<i64> {
        let mut tx = self.pool.begin().await?;
        if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, user_id).await? {
            tracing::info!(
                writer = "activity.user_retention_messages",
                user_id,
                "übersprungen wegen Opt-out"
            );
            tx.commit().await?;
            return Ok(0);
        }
        advisory_lock(&mut tx, RETENTION_MESSAGES_ID_LOCK).await?;
        let next_id = sqlx::query_scalar!(
            r#"
            SELECT COALESCE(MAX(id), 0) + 1 AS "next_id!: i64"
              FROM activity.user_retention_messages
            "#
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO activity.user_retention_messages(
                id, user_id, guild_id, message_type, sent_at, delivery_status, error_message
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            next_id,
            user_id,
            guild_id,
            message_type,
            sent_at,
            delivery_status,
            error_message,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(next_id)
    }
}

fn is_blocked_delivery_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("dms disabled")
        || (error.contains("50007") && !error.contains("no mutual guild"))
}

fn is_permanent_delivery_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("no mutual guild") || error.contains("cannot send messages to this user")
}

/// Miss-You-Embed (Python `_send_miss_you_message`): blau, optional Guild-Icon.
fn miss_you_embed(
    display_name: &str,
    days_inactive: i64,
    guild_name: &str,
    icon: Option<&str>,
) -> Value {
    let mut embed = json!({
        "title": format!("Hey {display_name}, wir vermissen dich! :("),
        "description": format!(
            "Dir ist bestimmt aufgefallen, dass du schon **{days_inactive} Tage** \
             nicht mehr aktiv in der **{guild_name}** warst.\n\n\
             Wir würden uns freuen, dich mal wieder im Voice oder Chat zu sehen.\n\n\
             Falls dich etwas stört oder du Feedback hast, lass es uns bitte wissen \
             - wir wollen den Server für dich besser machen."
        ),
        // discord.Color.blue()
        "color": 0x3498DB,
    });
    if let Some(url) = icon {
        embed["thumbnail"] = json!({ "url": url });
    }
    embed
}

/// Action-Row der Miss-You-DM: Link-Buttons (Server/Voice) + Feedback-Button.
/// Die `guild_id` reist in der `custom_id` mit, weil der Button in einer DM
/// geklickt wird (dort gäbe es sonst keinen Guild-Kontext).
fn miss_you_components(guild_id: u64) -> Value {
    json!([{
        "type": 1,
        "components": [
            { "type": 2, "style": 5, "label": "Zum Server", "emoji": { "name": "🏠" }, "url": SERVER_LINK },
            { "type": 2, "style": 5, "label": "Zum Voice", "emoji": { "name": "🎧" }, "url": VOICE_LINK },
            { "type": 2, "style": 1, "label": "Feedback geben", "emoji": { "name": "💬" }, "custom_id": format!("{FEEDBACK_BTN_PREFIX}{guild_id}") },
        ],
    }])
}

const FEEDBACK_BTN_PREFIX: &str = "retention_feedback:";
const FEEDBACK_MODAL_PREFIX: &str = "retention_feedback_modal:";

struct FeedbackHandler {
    pool: PgPool,
    port: Arc<dyn RetentionPort>,
}

#[async_trait::async_trait]
impl InteractionHandler for FeedbackHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        // Modal-Submit → Feedback in user_retention_messages ablegen.
        if let Some(gid) = interaction.custom_id.strip_prefix(FEEDBACK_MODAL_PREFIX) {
            let guild_id = gid.parse::<u64>().unwrap_or(0);
            let text = interaction
                .options
                .get("feedback")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let (Ok(user_id), Ok(guild_id_i64)) = (
                u64_to_i64(interaction.user_id, "interaction.user_id"),
                u64_to_i64(guild_id, "guild_id"),
            ) {
                let tracker = RetentionTracker::new(self.pool.clone());
                let _ = tracker
                    .insert_retention_message(
                        user_id,
                        guild_id_i64,
                        "feedback",
                        Utc::now(),
                        "received",
                        Some(text),
                    )
                    .await;
            }
            let (guild_name, _) = self.port.guild_label(guild_id).await;
            return BridgeReply::ephemeral_text(format!(
                "Danke für dein Feedback! Wir werden es uns anschauen und versuchen, \
                 **{guild_name}** für dich zu verbessern."
            ));
        }

        // Feedback-Button → Modal öffnen (guild_id wandert in die Modal-custom_id).
        if let Some(gid) = interaction.custom_id.strip_prefix(FEEDBACK_BTN_PREFIX) {
            return BridgeReply {
                modal: Some(ModalSpec {
                    custom_id: format!("{FEEDBACK_MODAL_PREFIX}{gid}"),
                    title: "Feedback geben".to_string(),
                    fields: vec![ModalField {
                        custom_id: "feedback".to_string(),
                        label: "Was können wir verbessern?".to_string(),
                        placeholder:
                            "Erzähl uns, was dich stört oder was wir besser machen können..."
                                .to_string(),
                        required: true,
                        min_length: 0,
                        max_length: 1000,
                        paragraph: true,
                    }],
                }),
                ..BridgeReply::default()
            };
        }

        BridgeReply::ephemeral_text("Unbekannte Aktion.")
    }
}

/// Self-Service-Slash-Commands `/retention-optout` + `/retention-optin`
/// (Python `retention_optout`/`retention_optin`): der User steuert selbst, ob
/// er die „Wir-vermissen-dich"-DMs bekommt.
struct OptOutHandler {
    tracker: RetentionTracker,
    opt_out: bool,
}

#[async_trait::async_trait]
impl InteractionHandler for OptOutHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if self.opt_out {
            let _ = self
                .tracker
                .set_opted_out(interaction.user_id, interaction.guild_id)
                .await;
            BridgeReply::ephemeral_text(
                "✅ Du erhältst ab jetzt keine 'Wir vermissen dich'-Nachrichten mehr.",
            )
        } else {
            let _ = self.tracker.clear_opted_out(interaction.user_id).await;
            BridgeReply::ephemeral_text(
                "✅ Du erhältst wieder 'Wir vermissen dich'-Nachrichten wenn du länger inaktiv bist.",
            )
        }
    }
}

fn optout_command_spec(name: &str, description: &str) -> CommandSpec {
    CommandSpec {
        definition: json!({
            "name": name,
            "description": description,
            "type": 1,
        }),
    }
}

/// Registriert den Feedback-Button + das Feedback-Modal der Miss-You-DM sowie
/// die Opt-out-/Opt-in-Slash-Commands.
pub fn register(router: &mut InteractionRouter, pool: PgPool, port: Arc<dyn RetentionPort>) {
    let tracker = RetentionTracker::new(pool.clone());
    router.on_command(
        "retention-optout",
        optout_command_spec(
            "retention-optout",
            "Deaktiviere 'Wir vermissen dich'-Nachrichten",
        ),
        Arc::new(OptOutHandler {
            tracker: tracker.clone(),
            opt_out: true,
        }),
    );
    router.on_command(
        "retention-optin",
        optout_command_spec(
            "retention-optin",
            "Aktiviere 'Wir vermissen dich'-Nachrichten wieder",
        ),
        Arc::new(OptOutHandler {
            tracker,
            opt_out: false,
        }),
    );

    let handler = Arc::new(FeedbackHandler { pool, port });
    router.on_prefix(FEEDBACK_BTN_PREFIX, handler.clone());
    router.on_prefix(FEEDBACK_MODAL_PREFIX, handler);
}

/// Spawnt den Voice-Join-Subscriber, den 30-min-Sync-Loop und den stündlichen
/// Miss-You-Check.
pub fn spawn(
    tracker: RetentionTracker,
    port: Arc<dyn RetentionPort>,
    dispatcher: &dl_discord::Dispatcher,
) {
    {
        let tracker = tracker.clone();
        tokio::spawn(async move {
            let _ = tracker.ensure_schema().await;
            loop {
                let now = chrono::Utc::now().timestamp();
                if let Err(e) = tracker.sync_activity_data(now).await {
                    tracing::warn!(%e, "retention-sync fehlgeschlagen");
                }
                tokio::time::sleep(SYNC_INTERVAL).await;
            }
        });
    }

    // Miss-You-Loop: stündlich aufwachen, die Tagesschranke prüft die Methode.
    {
        let tracker = tracker.clone();
        tokio::spawn(async move {
            loop {
                tracker.run_miss_you_check(&port).await;
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });
    }

    let mut events = dispatcher.subscribe_voice();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                // Join (aus dem Nichts) ODER Kanalwechsel zählt als Aktivität.
                Ok(dl_discord::VoiceEvent::Join {
                    guild_id, user_id, ..
                })
                | Ok(dl_discord::VoiceEvent::Move {
                    guild_id, user_id, ..
                }) => {
                    let now = chrono::Utc::now();
                    let _ = tracker
                        .update_user_activity(
                            user_id,
                            guild_id,
                            now.timestamp(),
                            now.format("%Y-%m-%d").to_string(),
                        )
                        .await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use dl_central_db::testing::{test_pool, TestDb};
    use std::collections::{HashSet, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockRetentionPort {
        guild_member: bool,
        guild_members: Option<HashSet<u64>>,
        guild_member_count: Option<usize>,
        deliveries: tokio::sync::Mutex<VecDeque<MissYouDelivery>>,
        send_calls: AtomicUsize,
        logs: tokio::sync::Mutex<Vec<String>>,
    }

    impl MockRetentionPort {
        fn new(
            guild_member: bool,
            guild_member_count: Option<usize>,
            deliveries: impl IntoIterator<Item = MissYouDelivery>,
        ) -> Self {
            Self {
                guild_member,
                guild_members: None,
                guild_member_count,
                deliveries: tokio::sync::Mutex::new(deliveries.into_iter().collect()),
                send_calls: AtomicUsize::new(0),
                logs: tokio::sync::Mutex::new(Vec::new()),
            }
        }

        fn with_guild_members(mut self, guild_members: impl IntoIterator<Item = u64>) -> Self {
            self.guild_members = Some(guild_members.into_iter().collect());
            self
        }

        fn is_member(&self, user_id: u64) -> bool {
            self.guild_members
                .as_ref()
                .map_or(self.guild_member, |members| members.contains(&user_id))
        }
    }

    #[async_trait::async_trait]
    impl RetentionPort for MockRetentionPort {
        async fn guild_member_count(&self, _guild_id: u64) -> Option<usize> {
            self.guild_member_count
        }

        async fn is_guild_member(&self, _guild_id: u64, user_id: u64) -> bool {
            self.is_member(user_id)
        }

        async fn member_info(&self, _guild_id: u64, user_id: u64) -> Option<RetentionMember> {
            self.is_member(user_id).then(|| RetentionMember {
                display_name: "Test".to_string(),
                role_ids: Vec::new(),
            })
        }

        async fn fetch_user_name(&self, _user_id: u64) -> Option<String> {
            Some("Test".to_string())
        }

        async fn guild_label(&self, _guild_id: u64) -> (String, Option<String>) {
            ("Test-Guild".to_string(), None)
        }

        async fn send_miss_you_dm(
            &self,
            _user_id: u64,
            _embed: Value,
            _components: Value,
        ) -> MissYouDelivery {
            self.send_calls.fetch_add(1, Ordering::SeqCst);
            self.deliveries
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| MissYouDelivery::Failed("keine Testzustellung".to_string()))
        }

        async fn send_log(&self, text: String) {
            self.logs.lock().await.push(text);
        }
    }

    async fn mk() -> (TestDb, RetentionTracker) {
        let db = test_pool().await.expect("test_pool");
        let tracker = RetentionTracker::new(db.pool().clone());
        tracker.ensure_schema().await.expect("schema");
        (db, tracker)
    }

    fn day(ts: i64) -> String {
        chrono::DateTime::from_timestamp(ts, 0)
            .expect("timestamp")
            .format("%Y-%m-%d")
            .to_string()
    }

    async fn insert_inactive_candidate(db: &TestDb, user_id: i64, now: DateTime<Utc>) {
        insert_inactive_candidates(db, &[user_id], now, 20).await;
    }

    async fn insert_inactive_candidates(
        db: &TestDb,
        user_ids: &[i64],
        now: DateTime<Utc>,
        days_inactive: i64,
    ) {
        let inactive = now - chrono::Duration::days(days_inactive);
        sqlx::query(
            r#"
            INSERT INTO activity.user_retention_tracking(
                user_id, guild_id, first_seen_at, last_active_at, total_active_days,
                avg_weekly_sessions, miss_you_count, opted_out, updated_at
            )
            SELECT user_id, 1, $2, $2, 5, 1.0, 0, FALSE, $3
              FROM UNNEST($1::BIGINT[]) AS candidate(user_id)
            "#,
        )
        .bind(user_ids)
        .bind(inactive)
        .bind(now)
        .execute(db.pool())
        .await
        .expect("insert candidate");
    }

    #[tokio::test]
    async fn update_zaehlt_nur_neuen_tag() {
        let (db, tracker) = mk().await;
        let d1a = 1_780_000_000_i64;
        let d1b = d1a + 3600;
        let d2 = d1a + 86400;
        tracker
            .update_user_activity(5, 1, d1a, day(d1a))
            .await
            .expect("update");
        tracker
            .update_user_activity(5, 1, d1b, day(d1b))
            .await
            .expect("update");
        tracker
            .update_user_activity(5, 1, d2, day(d2))
            .await
            .expect("update");
        let total = sqlx::query_scalar!(
            "SELECT total_active_days FROM activity.user_retention_tracking WHERE user_id = 5"
        )
        .fetch_one(db.pool())
        .await
        .expect("total");
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn update_respektiert_opt_out() {
        let (db, tracker) = mk().await;
        sqlx::query!(
            "INSERT INTO core.user_privacy(user_id, opted_out, updated_at) VALUES (9, TRUE, now())"
        )
        .execute(db.pool())
        .await
        .expect("privacy");
        tracker
            .update_user_activity(9, 1, 1000, "2026-06-01".into())
            .await
            .expect("update");
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM activity.user_retention_tracking WHERE user_id = 9"
        )
        .fetch_one(db.pool())
        .await
        .expect("count");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn privacy_lock_blockiert_retention_refill_nach_loeschung() {
        let (db, tracker) = mk().await;
        let pool = db.pool().clone();
        let mut erase_tx = pool.begin().await.expect("erase tx");
        dl_central_db::lock_user_privacy(&mut erase_tx, 9)
            .await
            .expect("privacy lock");
        sqlx::query(
            "INSERT INTO core.user_privacy(user_id, opted_out, updated_at)
             VALUES(9, TRUE, now())",
        )
        .execute(&mut *erase_tx)
        .await
        .expect("privacy tombstone");

        let write_tracker = tracker.clone();
        let write = tokio::spawn(async move {
            write_tracker
                .update_user_activity(9, 1, 1_780_000_000, day(1_780_000_000))
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        erase_tx.commit().await.expect("erase commit");
        write.await.expect("writer task").expect("writer result");

        tracker
            .insert_retention_message(9, 1, "miss_you", Utc::now(), "sent", None)
            .await
            .expect("message writer");
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT
                (SELECT COUNT(*) FROM activity.user_retention_tracking WHERE user_id = 9) +
                (SELECT COUNT(*) FROM activity.user_retention_messages WHERE user_id = 9)",
        )
        .fetch_one(&pool)
        .await
        .expect("retention refill count");
        assert_eq!(count, 0, "Löschung darf Retention-Profil nicht neu anlegen");
    }

    #[tokio::test]
    async fn sync_berechnet_avg_weekly_utc_bucket() {
        let (db, tracker) = mk().await;
        let started1 = chrono::DateTime::parse_from_rfc3339("2026-06-01T10:00:00Z")
            .expect("dt")
            .with_timezone(&Utc);
        let started2 = chrono::DateTime::parse_from_rfc3339("2026-06-01T12:00:00Z")
            .expect("dt")
            .with_timezone(&Utc);
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_session_log(
                id, user_id, guild_id, started_at, ended_at, duration_seconds, points
            )
            VALUES (1, 7, 1, $1, $1, 600, 1),
                   (2, 7, 1, $2, $2, 600, 1)
            "#,
            started1,
            started2,
        )
        .execute(db.pool())
        .await
        .expect("voice sessions");
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-01T13:00:00Z")
            .expect("dt")
            .timestamp();
        let updated = tracker.sync_activity_data(now).await.expect("sync");
        assert_eq!(updated, 1);
        let avg = sqlx::query_scalar!(
            "SELECT avg_weekly_sessions FROM activity.user_retention_tracking WHERE user_id = 7"
        )
        .fetch_one(db.pool())
        .await
        .expect("avg")
        .unwrap_or_default();
        assert!((avg - 2.0).abs() < 0.01, "avg_weekly = {avg}");
    }

    #[tokio::test]
    async fn optout_legt_an_und_optin_setzt_zurueck() {
        let (db, tracker) = mk().await;
        tracker.set_opted_out(42, 7).await.expect("optout");
        let row = sqlx::query!(
            "SELECT guild_id, opted_out FROM activity.user_retention_tracking WHERE user_id = 42"
        )
        .fetch_one(db.pool())
        .await
        .expect("row");
        assert_eq!(row.guild_id, 7);
        assert!(row.opted_out);

        tracker.clear_opted_out(42).await.expect("optin");
        let opted = sqlx::query_scalar!(
            "SELECT opted_out FROM activity.user_retention_tracking WHERE user_id = 42"
        )
        .fetch_one(db.pool())
        .await
        .expect("opted");
        assert!(!opted);
    }

    #[tokio::test]
    async fn optin_ohne_eintrag_legt_nichts_an() {
        let (db, tracker) = mk().await;
        tracker.clear_opted_out(999).await.expect("optin");
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM activity.user_retention_tracking WHERE user_id = 999"
        )
        .fetch_one(db.pool())
        .await
        .expect("count");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn find_inactive_filtert_jede_bedingung() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        let inactive = now - chrono::Duration::days(20);
        let recent = now - chrono::Duration::days(2);
        let recent_msg = now - chrono::Duration::days(5);
        for (user_id, last_active, active_days, avg, opted_out, miss_count, last_sent) in [
            (1_i64, inactive, 5, 1.0, false, 0, None),
            (2, inactive, 5, 1.0, true, 0, None),
            (3, inactive, 5, 1.0, false, 1, None),
            (4, recent, 5, 1.0, false, 0, None),
            (5, inactive, 1, 1.0, false, 0, None),
            (6, inactive, 5, 0.1, false, 0, None),
            (7, inactive, 5, 1.0, false, 0, Some(recent_msg)),
        ] {
            sqlx::query!(
                r#"
                INSERT INTO activity.user_retention_tracking(
                    user_id, guild_id, first_seen_at, last_active_at, total_active_days,
                    avg_weekly_sessions, last_miss_you_sent_at, miss_you_count, opted_out, updated_at
                )
                VALUES ($1, 1, $2, $2, $3, $4, $5, $6, $7, $8)
                "#,
                user_id,
                last_active,
                active_days,
                avg,
                last_sent,
                miss_count,
                opted_out,
                now,
            )
            .execute(db.pool())
            .await
            .expect("insert tracking");
        }
        let found: Vec<u64> = tracker
            .find_inactive_users(now.timestamp())
            .await
            .into_iter()
            .map(|(user_id, _, _)| user_id)
            .collect();
        assert_eq!(found, vec![1]);
    }

    #[tokio::test]
    async fn nicht_mitglieder_verbrauchen_keinen_sendeplatz() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        let non_members: Vec<i64> = (1_000..1_060).collect();
        let members: Vec<i64> = (2_000..2_050).collect();
        insert_inactive_candidates(&db, &non_members, now, 30).await;
        insert_inactive_candidates(&db, &members, now, 20).await;
        let mock = Arc::new(
            MockRetentionPort::new(
                false,
                Some(100),
                std::iter::repeat_with(|| MissYouDelivery::Sent).take(50),
            )
            .with_guild_members(members.iter().map(|&id| id as u64)),
        );
        let port: Arc<dyn RetentionPort> = mock.clone();

        tracker
            .run_miss_you_candidates(&port, now.timestamp())
            .await;

        assert_eq!(mock.send_calls.load(Ordering::SeqCst), 50);
    }

    #[tokio::test]
    async fn laufbilanz_zaehlt_per_cache_uebersprungene_nicht_mitglieder() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        insert_inactive_candidates(&db, &[3_000, 3_001, 3_002], now, 30).await;
        insert_inactive_candidate(&db, 4_000, now).await;
        let mock = Arc::new(
            MockRetentionPort::new(false, Some(100), [MissYouDelivery::Sent])
                .with_guild_members([4_000]),
        );
        let port: Arc<dyn RetentionPort> = mock.clone();

        tracker
            .run_miss_you_candidates(&port, now.timestamp())
            .await;

        let logs = mock.logs.lock().await;
        assert!(
            logs.last()
                .is_some_and(|log| log.contains("nicht mehr auf dem Server: 3")),
            "Laufbilanz: {logs:?}"
        );
    }

    #[tokio::test]
    async fn sendelimit_gilt_bei_500_geladenen_kandidaten() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        let candidates: Vec<i64> = (5_000..5_500).collect();
        insert_inactive_candidates(&db, &candidates, now, 20).await;
        assert_eq!(
            tracker.find_inactive_users(now.timestamp()).await.len(),
            500,
            "SQL-Auswahl muss 500 Kandidaten für den Membership-Filter laden"
        );
        let mock = Arc::new(MockRetentionPort::new(
            true,
            Some(500),
            std::iter::repeat_with(|| MissYouDelivery::Sent).take(500),
        ));
        let port: Arc<dyn RetentionPort> = mock.clone();

        tracker
            .run_miss_you_candidates(&port, now.timestamp())
            .await;

        assert_eq!(mock.send_calls.load(Ordering::SeqCst), 50);
    }

    #[tokio::test]
    async fn cache_miss_veraendert_zaehler_nicht_und_bleibt_kandidat() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        insert_inactive_candidate(&db, 10, now).await;
        let mock = Arc::new(MockRetentionPort::new(false, Some(100), []));
        let port: Arc<dyn RetentionPort> = mock.clone();

        tracker
            .run_miss_you_candidates(&port, now.timestamp())
            .await;

        assert_eq!(mock.send_calls.load(Ordering::SeqCst), 0);
        let miss_you_count = sqlx::query_scalar::<_, i32>(
            "SELECT miss_you_count FROM activity.user_retention_tracking WHERE user_id = 10",
        )
        .fetch_one(db.pool())
        .await
        .expect("miss_you_count");
        assert_eq!(miss_you_count, 0);
        assert!(
            !tracker
                .find_inactive_users((now + chrono::Duration::days(1)).timestamp())
                .await
                .is_empty(),
            "Cache-Miss muss am Folgetag erneut Kandidat sein"
        );
        let message_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM activity.user_retention_messages WHERE user_id = 10",
        )
        .fetch_one(db.pool())
        .await
        .expect("message count");
        assert_eq!(message_count, 0);
    }

    #[tokio::test]
    async fn permanenter_fehler_verhindert_folgeversuch() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        insert_inactive_candidate(&db, 11, now).await;
        let mock = Arc::new(MockRetentionPort::new(
            true,
            Some(100),
            [
                MissYouDelivery::Failed(
                    "Cannot send messages to this user due to having no mutual guilds".to_string(),
                ),
                MissYouDelivery::Sent,
            ],
        ));
        let port: Arc<dyn RetentionPort> = mock.clone();

        tracker
            .run_miss_you_candidates(&port, now.timestamp())
            .await;
        tracker
            .run_miss_you_candidates(&port, (now + chrono::Duration::days(1)).timestamp())
            .await;

        assert_eq!(mock.send_calls.load(Ordering::SeqCst), 1);
        let miss_you_count = sqlx::query_scalar::<_, i32>(
            "SELECT miss_you_count FROM activity.user_retention_tracking WHERE user_id = 11",
        )
        .fetch_one(db.pool())
        .await
        .expect("miss_you_count");
        assert_eq!(miss_you_count, 1);
    }

    #[tokio::test]
    async fn transienter_fehler_erlaubt_folgeversuch() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        insert_inactive_candidate(&db, 12, now).await;
        let mock = Arc::new(MockRetentionPort::new(
            true,
            Some(100),
            [
                MissYouDelivery::Failed("HTTP 503 Service Unavailable".to_string()),
                MissYouDelivery::Sent,
            ],
        ));
        let port: Arc<dyn RetentionPort> = mock.clone();

        tracker
            .run_miss_you_candidates(&port, now.timestamp())
            .await;
        tracker
            .run_miss_you_candidates(&port, (now + chrono::Duration::days(1)).timestamp())
            .await;

        assert_eq!(mock.send_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn blocked_wird_als_blocked_statt_failed_protokolliert() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        insert_inactive_candidate(&db, 13, now).await;
        let mock = Arc::new(MockRetentionPort::new(
            true,
            Some(100),
            [MissYouDelivery::Failed(
                "Discord API 50007: Cannot send messages to this user (DMs disabled)".to_string(),
            )],
        ));
        let port: Arc<dyn RetentionPort> = mock;

        tracker
            .run_miss_you_candidates(&port, now.timestamp())
            .await;

        let statuses = sqlx::query_scalar::<_, String>(
            "SELECT delivery_status FROM activity.user_retention_messages WHERE user_id = 13",
        )
        .fetch_all(db.pool())
        .await
        .expect("delivery statuses");
        assert_eq!(statuses, vec!["blocked"]);
    }

    #[tokio::test]
    async fn fehlende_guild_im_cache_bricht_lauf_ohne_aenderungen_ab() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        insert_inactive_candidate(&db, 14, now).await;
        let mock = Arc::new(MockRetentionPort::new(true, None, [MissYouDelivery::Sent]));
        let port: Arc<dyn RetentionPort> = mock.clone();

        tracker
            .run_miss_you_candidates(&port, now.timestamp())
            .await;

        assert_eq!(mock.send_calls.load(Ordering::SeqCst), 0);
        let miss_you_count = sqlx::query_scalar::<_, i32>(
            "SELECT miss_you_count FROM activity.user_retention_tracking WHERE user_id = 14",
        )
        .fetch_one(db.pool())
        .await
        .expect("miss_you_count");
        assert_eq!(miss_you_count, 0);
        let logs = mock.logs.lock().await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0], MISS_YOU_CACHE_ABORT_LOG);
    }

    #[tokio::test]
    async fn zu_kleiner_guild_cache_bricht_lauf_ohne_aenderungen_ab() {
        let (db, tracker) = mk().await;
        let now = utc_from_unix(2_000_000_000).expect("now");
        insert_inactive_candidate(&db, 15, now).await;
        let mock = Arc::new(MockRetentionPort::new(
            true,
            Some(99),
            [MissYouDelivery::Sent],
        ));
        let port: Arc<dyn RetentionPort> = mock.clone();

        tracker
            .run_miss_you_candidates(&port, now.timestamp())
            .await;

        assert_eq!(mock.send_calls.load(Ordering::SeqCst), 0);
        let miss_you_count = sqlx::query_scalar::<_, i32>(
            "SELECT miss_you_count FROM activity.user_retention_tracking WHERE user_id = 15",
        )
        .fetch_one(db.pool())
        .await
        .expect("miss_you_count");
        assert_eq!(miss_you_count, 0);
        let logs = mock.logs.lock().await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0], MISS_YOU_CACHE_ABORT_LOG);
    }
}
