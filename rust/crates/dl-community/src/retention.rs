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
const SERVER_LINK: &str = "https://discord.com/channels/1289721245281292288/1289721245281292291";
const VOICE_LINK: &str = "https://discord.com/channels/1289721245281292288/1501089974093873232";
const EXCLUDED_ROLE_IDS: [u64; 2] = [1304416311383818240, 1309741866098491479];
const RETENTION_MESSAGES_ID_LOCK: i64 = 0x4451_0008_0010_0002;

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
}

#[derive(Clone)]
pub struct RetentionTracker {
    pool: PgPool,
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
        if crate::privacy::is_opted_out(&self.pool, user_id).await {
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
        .fetch_optional(&self.pool)
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
                .execute(&self.pool)
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
                .execute(&self.pool)
                .await?;
            }
        }
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
    /// Spam-Schutz greift. Liefert `(user_id, guild_id, days_inactive)`, max. 50.
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
             LIMIT 50
            "#,
            now_dt,
            MIN_WEEKLY_SESSIONS,
            i64_to_i32(MIN_TOTAL_ACTIVE_DAYS, "MIN_TOTAL_ACTIVE_DAYS").unwrap_or(i32::MAX),
            inactivity_threshold,
            i64_to_i32(MAX_MISS_YOU_PER_USER, "MAX_MISS_YOU_PER_USER").unwrap_or(i32::MAX),
            min_gap,
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

        let now = now_dt.timestamp();
        for (user_id, guild_id, days_inactive) in self.find_inactive_users(now).await {
            self.send_miss_you(port, user_id, guild_id, days_inactive)
                .await;
        }
    }

    /// Eine Miss-You-DM aufbauen, senden und das Ergebnis protokollieren
    /// (Python `_send_miss_you_message`).
    async fn send_miss_you(
        &self,
        port: &Arc<dyn RetentionPort>,
        user_id: u64,
        guild_id: u64,
        days_inactive: i64,
    ) {
        // Globaler Privacy-Opt-out hat Vorrang vor der Retention-Spalte.
        let Ok(user_id_i64) = u64_to_i64(user_id, "user_id") else {
            return;
        };
        let Ok(guild_id_i64) = u64_to_i64(guild_id, "guild_id") else {
            return;
        };
        if crate::privacy::is_opted_out(&self.pool, user_id_i64).await {
            return;
        }
        // Name + Excluded-Rollen aus dem Cache; ausgeschlossene Rollen → kein DM.
        let member = port.member_info(guild_id, user_id).await;
        if let Some(m) = &member {
            if m.role_ids.iter().any(|r| EXCLUDED_ROLE_IDS.contains(r)) {
                return;
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
                let _ = sqlx::query!(
                    r#"
                    UPDATE activity.user_retention_tracking
                       SET last_miss_you_sent_at = $1,
                           miss_you_count = miss_you_count + 1,
                           updated_at = $1
                     WHERE user_id = $2
                    "#,
                    now,
                    user_id_i64,
                )
                .execute(&self.pool)
                .await;
                let _ = self
                    .insert_retention_message(
                        user_id_i64,
                        guild_id_i64,
                        "miss_you",
                        now,
                        "sent",
                        None,
                    )
                    .await;
            }
            MissYouDelivery::Blocked => {
                let _ = self
                    .insert_retention_message(
                        user_id_i64,
                        guild_id_i64,
                        "miss_you",
                        now,
                        "blocked",
                        Some("DMs disabled".to_string()),
                    )
                    .await;
            }
            MissYouDelivery::Failed(err) => {
                let _ = self
                    .insert_retention_message(
                        user_id_i64,
                        guild_id_i64,
                        "miss_you",
                        now,
                        "failed",
                        Some(err),
                    )
                    .await;
            }
        }
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
}
