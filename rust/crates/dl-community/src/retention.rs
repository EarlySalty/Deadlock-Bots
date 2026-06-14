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

use chrono::Timelike;
use dl_db::{Db, DbError};
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

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
    db: Db,
}

impl RetentionTracker {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Legt die Tabelle an (idempotent), Schema wie `rust/docs/db-schema.sql`.
    pub async fn ensure_schema(&self) -> Result<(), DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS user_retention_tracking(
                       user_id INTEGER PRIMARY KEY,
                       guild_id INTEGER NOT NULL,
                       first_seen_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                       last_active_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                       total_active_days INTEGER NOT NULL DEFAULT 0,
                       avg_weekly_sessions REAL DEFAULT 0,
                       last_miss_you_sent_at INTEGER,
                       miss_you_count INTEGER NOT NULL DEFAULT 0,
                       opted_out INTEGER NOT NULL DEFAULT 0,
                       updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
                     );
                     CREATE TABLE IF NOT EXISTS user_retention_messages(
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       user_id INTEGER NOT NULL,
                       guild_id INTEGER NOT NULL,
                       message_type TEXT NOT NULL,
                       sent_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                       delivery_status TEXT NOT NULL DEFAULT 'sent',
                       error_message TEXT
                     );",
                )?;
                Ok(())
            })
            .await
    }

    /// Voice-Join/-Wechsel: `last_active_at` + `total_active_days` (nur ein
    /// neuer UTC-Tag erhöht den Zähler). Gated auf `user_privacy`-Opt-out.
    pub async fn update_user_activity(
        &self,
        user_id: u64,
        guild_id: u64,
        now: i64,
        today: String,
    ) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                // Opt-out-Gate (user_privacy, wertbasiert; fail-open bei Fehler).
                let opted = conn
                    .query_row(
                        "SELECT opted_out FROM user_privacy WHERE user_id=?1",
                        params![user_id],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()
                    .ok()
                    .flatten()
                    .filter(|v| *v != 0);
                if opted.is_some() {
                    return Ok(());
                }

                let row: Option<(i64, i64)> = conn
                    .query_row(
                        "SELECT last_active_at, total_active_days FROM user_retention_tracking WHERE user_id=?1",
                        params![user_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                match row {
                    Some((last_active_ts, total_days)) => {
                        let last_date = chrono::DateTime::from_timestamp(last_active_ts, 0)
                            .map(|d| d.format("%Y-%m-%d").to_string())
                            .unwrap_or_default();
                        let new_total = if last_date != today {
                            total_days + 1
                        } else {
                            total_days
                        };
                        conn.execute(
                            "UPDATE user_retention_tracking
                               SET last_active_at=?1, total_active_days=?2, updated_at=?1
                             WHERE user_id=?3",
                            params![now, new_total, user_id],
                        )?;
                    }
                    None => {
                        conn.execute(
                            "INSERT INTO user_retention_tracking
                               (user_id, guild_id, first_seen_at, last_active_at, total_active_days, updated_at)
                             VALUES(?1, ?2, ?3, ?3, 1, ?3)",
                            params![user_id, guild_id, now],
                        )?;
                    }
                }
                Ok(())
            })
            .await
    }

    /// 30-min-Lauf: `avg_weekly_sessions` aus `voice_session_log` (letzte 60
    /// Tage). `weeks_active = max(1, (now-first_session)/Woche)`. Gibt die Zahl
    /// aktualisierter User zurück.
    pub async fn sync_activity_data(&self, now: i64) -> Result<usize, DbError> {
        let cutoff = now - LOOKBACK_DAYS * 86400;
        self.db
            .write(move |conn| {
                let agg: Vec<(i64, i64, i64, i64, i64, i64)> = {
                    let mut stmt = conn.prepare(
                        "SELECT user_id, guild_id,
                                COUNT(DISTINCT date(started_at)) AS active_days,
                                COUNT(*) AS total_sessions,
                                CAST(MIN(strftime('%s', started_at)) AS INTEGER) AS first_s,
                                CAST(MAX(strftime('%s', started_at)) AS INTEGER) AS last_s
                           FROM voice_session_log
                          WHERE CAST(strftime('%s', started_at) AS INTEGER) > ?1
                          GROUP BY user_id",
                    )?;
                    let rows = stmt.query_map(params![cutoff], |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get::<_, Option<i64>>(4)?.unwrap_or(now),
                            r.get::<_, Option<i64>>(5)?.unwrap_or(now),
                        ))
                    })?;
                    rows.collect::<rusqlite::Result<_>>()?
                };
                let count = agg.len();
                for (user_id, guild_id, active_days, total_sessions, first_s, last_s) in agg {
                    let weeks = ((now - first_s) as f64 / (7.0 * 86400.0)).max(1.0);
                    let avg_weekly = total_sessions as f64 / weeks;
                    conn.execute(
                        "INSERT INTO user_retention_tracking
                           (user_id, guild_id, first_seen_at, last_active_at, total_active_days, avg_weekly_sessions, updated_at)
                         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT(user_id) DO UPDATE SET
                           last_active_at=MAX(user_retention_tracking.last_active_at, excluded.last_active_at),
                           total_active_days=MAX(user_retention_tracking.total_active_days, excluded.total_active_days),
                           avg_weekly_sessions=excluded.avg_weekly_sessions,
                           updated_at=excluded.updated_at",
                        params![user_id, guild_id, first_s, last_s, active_days, avg_weekly, now],
                    )?;
                }
                Ok(count)
            })
            .await
    }

    /// Opt-out setzen (Python `retention_optout`): legt den Tracking-Eintrag
    /// bei Bedarf an und setzt `opted_out=1`. Der Miss-You-Sender liest die
    /// Spalte bereits (siehe [`Self::find_inactive_users`]).
    pub async fn set_opted_out(&self, user_id: u64, guild_id: u64) -> Result<(), DbError> {
        let now = chrono::Utc::now().timestamp();
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO user_retention_tracking (user_id, guild_id, opted_out, updated_at)
                     VALUES (?1, ?2, 1, ?3)
                     ON CONFLICT(user_id) DO UPDATE SET opted_out = 1, updated_at = ?3",
                    params![user_id, guild_id, now],
                )
                .map(|_| ())
            })
            .await
    }

    /// Opt-in (Python `retention_optin`): setzt `opted_out=0` für einen
    /// bestehenden Eintrag. Wie das Original wird hier nichts neu angelegt.
    pub async fn clear_opted_out(&self, user_id: u64) -> Result<(), DbError> {
        let now = chrono::Utc::now().timestamp();
        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE user_retention_tracking
                       SET opted_out = 0, updated_at = ?1
                     WHERE user_id = ?2",
                    params![now, user_id],
                )
                .map(|_| ())
            })
            .await
    }

    /// Inaktive Stamm-User (Python `_find_inactive_regular_users`): regelmäßig
    /// aktiv gewesen, jetzt über der Schwelle inaktiv, nicht opted-out,
    /// Spam-Schutz greift. Liefert `(user_id, guild_id, days_inactive)`, max. 50.
    async fn find_inactive_users(&self, now: i64) -> Vec<(u64, u64, i64)> {
        let inactivity_threshold = now - INACTIVITY_THRESHOLD_DAYS * 86400;
        let min_gap = now - MIN_DAYS_BETWEEN_MESSAGES * 86400;
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT user_id, guild_id, (?1 - last_active_at) / 86400 AS days_inactive
                       FROM user_retention_tracking
                      WHERE avg_weekly_sessions >= ?2
                        AND total_active_days >= ?3
                        AND last_active_at < ?4
                        AND opted_out = 0
                        AND miss_you_count < ?5
                        AND (last_miss_you_sent_at IS NULL OR last_miss_you_sent_at < ?6)
                      ORDER BY days_inactive DESC
                      LIMIT 50",
                )?;
                let rows = stmt.query_map(
                    params![
                        now,
                        MIN_WEEKLY_SESSIONS,
                        MIN_TOTAL_ACTIVE_DAYS,
                        inactivity_threshold,
                        MAX_MISS_YOU_PER_USER,
                        min_gap
                    ],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)? as u64,
                            r.get::<_, i64>(1)? as u64,
                            r.get::<_, i64>(2)?,
                        ))
                    },
                )?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap_or_default()
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
        if self
            .db
            .kv_get("retention", "last_check_date")
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some(today.as_str())
        {
            return;
        }
        let _ = self.db.kv_set("retention", "last_check_date", today).await;

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
        if crate::privacy::is_opted_out(&self.db, user_id as i64).await {
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
        let now = chrono::Utc::now().timestamp();
        match delivery {
            MissYouDelivery::Sent => {
                let _ = self
                    .db
                    .write(move |conn| {
                        conn.execute(
                            "UPDATE user_retention_tracking
                                SET last_miss_you_sent_at=?1, miss_you_count=miss_you_count+1, updated_at=?1
                              WHERE user_id=?2",
                            params![now, user_id],
                        )?;
                        conn.execute(
                            "INSERT INTO user_retention_messages
                               (user_id, guild_id, message_type, sent_at, delivery_status)
                             VALUES(?1, ?2, 'miss_you', ?3, 'sent')",
                            params![user_id, guild_id, now],
                        )?;
                        Ok(())
                    })
                    .await;
            }
            MissYouDelivery::Blocked => {
                let _ = self
                    .db
                    .write(move |conn| {
                        conn.execute(
                            "INSERT INTO user_retention_messages
                               (user_id, guild_id, message_type, sent_at, delivery_status, error_message)
                             VALUES(?1, ?2, 'miss_you', ?3, 'blocked', 'DMs disabled')",
                            params![user_id, guild_id, now],
                        )
                        .map(|_| ())
                    })
                    .await;
            }
            MissYouDelivery::Failed(err) => {
                let _ = self
                    .db
                    .write(move |conn| {
                        conn.execute(
                            "INSERT INTO user_retention_messages
                               (user_id, guild_id, message_type, sent_at, delivery_status, error_message)
                             VALUES(?1, ?2, 'miss_you', ?3, 'failed', ?4)",
                            params![user_id, guild_id, now, err],
                        )
                        .map(|_| ())
                    })
                    .await;
            }
        }
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
    db: Db,
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
            let user_id = interaction.user_id;
            let now = chrono::Utc::now().timestamp();
            let _ = self
                .db
                .write(move |conn| {
                    conn.execute(
                        "INSERT INTO user_retention_messages
                           (user_id, guild_id, message_type, sent_at, delivery_status, error_message)
                         VALUES(?1, ?2, 'feedback', ?3, 'received', ?4)",
                        params![user_id, guild_id, now, text],
                    )
                    .map(|_| ())
                })
                .await;
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
pub fn register(router: &mut InteractionRouter, db: Db, port: Arc<dyn RetentionPort>) {
    let tracker = RetentionTracker::new(db.clone());
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

    let handler = Arc::new(FeedbackHandler { db, port });
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn mk() -> (tempfile::TempDir, RetentionTracker) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("r.sqlite3")).expect("db");
        let t = RetentionTracker::new(db.clone());
        t.ensure_schema().await.expect("schema");
        db.write(|c| {
            c.execute_batch(
                "CREATE TABLE user_privacy(user_id INTEGER PRIMARY KEY, opted_out INTEGER DEFAULT 0);
                 CREATE TABLE voice_session_log(user_id INTEGER, guild_id INTEGER, started_at DATETIME, duration_seconds INTEGER);",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        (dir, t)
    }

    fn day(ts: i64) -> String {
        chrono::DateTime::from_timestamp(ts, 0)
            .unwrap()
            .format("%Y-%m-%d")
            .to_string()
    }

    #[tokio::test]
    async fn update_zaehlt_nur_neuen_tag() {
        let (_d, t) = mk().await;
        // now+today konsistent (last_date wird aus dem gespeicherten ts abgeleitet).
        let d1a = 1_780_000_000_i64; // 20:26 UTC an Tag X
        let d1b = d1a + 3600; // selber Tag X
        let d2 = d1a + 86400; // Tag X+1
        t.update_user_activity(5, 1, d1a, day(d1a)).await.unwrap(); // → 1
        t.update_user_activity(5, 1, d1b, day(d1b)).await.unwrap(); // selber Tag → 1
        t.update_user_activity(5, 1, d2, day(d2)).await.unwrap(); // neuer Tag → 2
        let total: i64 =
            t.db.read(|c| {
                c.query_row(
                    "SELECT total_active_days FROM user_retention_tracking WHERE user_id=5",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn update_respektiert_opt_out() {
        let (_d, t) = mk().await;
        t.db.write(|c| {
            c.execute(
                "INSERT INTO user_privacy(user_id, opted_out) VALUES(9, 1)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        t.update_user_activity(9, 1, 1000, "2026-06-01".into())
            .await
            .unwrap();
        let n: i64 =
            t.db.read(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM user_retention_tracking WHERE user_id=9",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(n, 0, "Opt-out-User wird nicht getrackt");
    }

    #[tokio::test]
    async fn sync_berechnet_avg_weekly() {
        let (_d, t) = mk().await;
        // 2 Sessions am selben Start; first=last → weeks_active=max(1,…)=1 → avg=2.
        t.db.write(|c| {
            c.execute_batch(
                "INSERT INTO voice_session_log(user_id, guild_id, started_at, duration_seconds)
                 VALUES (7, 1, '2026-06-01 10:00:00', 600),
                        (7, 1, '2026-06-01 12:00:00', 600);",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        // now nahe an started_at → weeks ~1.
        let now =
            chrono::DateTime::parse_from_str("2026-06-01 13:00:00 +0000", "%Y-%m-%d %H:%M:%S %z")
                .unwrap()
                .timestamp();
        let updated = t.sync_activity_data(now).await.unwrap();
        assert_eq!(updated, 1);
        let avg: f64 =
            t.db.read(|c| {
                c.query_row(
                    "SELECT avg_weekly_sessions FROM user_retention_tracking WHERE user_id=7",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert!((avg - 2.0).abs() < 0.01, "avg_weekly = {avg}");
    }

    #[tokio::test]
    async fn optout_legt_an_und_optin_setzt_zurueck() {
        let (_d, t) = mk().await;
        // Opt-out ohne Vor-Eintrag → legt Zeile mit opted_out=1 an.
        t.set_opted_out(42, 7).await.unwrap();
        let (gid, opted): (i64, i64) = t
            .db
            .read(|c| {
                c.query_row(
                    "SELECT guild_id, opted_out FROM user_retention_tracking WHERE user_id=42",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!((gid, opted), (7, 1));

        // Opt-in → setzt opted_out=0 für den bestehenden Eintrag.
        t.clear_opted_out(42).await.unwrap();
        let opted: i64 = t
            .db
            .read(|c| {
                c.query_row(
                    "SELECT opted_out FROM user_retention_tracking WHERE user_id=42",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(opted, 0);
    }

    #[tokio::test]
    async fn optin_ohne_eintrag_legt_nichts_an() {
        let (_d, t) = mk().await;
        // Wie das Python-UPDATE: ohne bestehende Zeile passiert nichts.
        t.clear_opted_out(999).await.unwrap();
        let n: i64 = t
            .db
            .read(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM user_retention_tracking WHERE user_id=999",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn find_inactive_filtert_jede_bedingung() {
        let (_d, t) = mk().await;
        let now = 2_000_000_000_i64;
        let inactive = now - 20 * 86400; // 20 Tage inaktiv (> 14)
        let recent = now - 2 * 86400; // erst 2 Tage (noch aktiv)
        let recent_msg = now - 5 * 86400; // vor 5 Tagen schon angeschrieben (< 30)
        // user 1 erfüllt alle Kriterien; 2–7 fallen je an einer Bedingung raus.
        t.db
            .write(move |c| {
                c.execute_batch(&format!(
                    "INSERT INTO user_retention_tracking
                       (user_id,guild_id,last_active_at,total_active_days,avg_weekly_sessions,opted_out,miss_you_count,last_miss_you_sent_at)
                     VALUES
                       (1,1,{inactive},5,1.0,0,0,NULL),        -- berechtigt
                       (2,1,{inactive},5,1.0,1,0,NULL),        -- opted_out
                       (3,1,{inactive},5,1.0,0,1,NULL),        -- schon 1 Nachricht
                       (4,1,{recent},5,1.0,0,0,NULL),          -- noch aktiv
                       (5,1,{inactive},1,1.0,0,0,NULL),        -- zu wenige aktive Tage
                       (6,1,{inactive},5,0.1,0,0,NULL),        -- zu selten (avg < 0.5)
                       (7,1,{inactive},5,1.0,0,0,{recent_msg}) -- Spam-Sperre (< 30 Tage)"
                ))?;
                Ok(())
            })
            .await
            .unwrap();
        let found: Vec<u64> = t
            .find_inactive_users(now)
            .await
            .into_iter()
            .map(|(u, _, _)| u)
            .collect();
        assert_eq!(found, vec![1], "nur der berechtigte User darf übrig bleiben");
    }
}
