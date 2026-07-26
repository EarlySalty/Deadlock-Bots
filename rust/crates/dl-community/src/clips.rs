//! Clip-Einsendungen — Port von `cogs/clip_submission.py`.
//!
//! Interface-Embed mit „Clip einsenden"-Button im Submit-Kanal →
//! Erlaubnis-Bestätigung → Modal (Link/Credit/Info, URL-Check,
//! 60-s-Cooldown). Einsendungen laufen in ein wöchentliches Fenster
//! (Sonntag 00:00 → Samstag 23:00 Europe/Berlin); nach Ablauf geht genau
//! einmal ein TXT-Dump aller Einsendungen per DM an den Clip-Kurator.
//! custom_ids (`clip_submit_btn_v1`, `clip_perm_yes_v1`) unverändert.

use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::json;
use sqlx::PgPool;

use crate::db::{i64_to_u64, u64_to_i64, unix_from_utc, utc_from_unix};

pub const SUBMIT_CHANNEL_ID: u64 = 1425215762460835931;
pub const SEND_TO_USER_ID: u64 = 388772056717590539;
pub const GUILD_ID: u64 = 1289721245281292288;
pub const VIEW_TYPE: &str = "clip_submission_v1";
pub const COOLDOWN_SECONDS: u64 = 60;

pub const INTERFACE_TITLE: &str = "🎥 Deadlock Gameplay-Clips einsenden";
pub const RULES_TEXT: &str = "• Reiche einen Gameplay-Clip in mind. 1080p ein.\n\
• Füge **Link**, **Credit/Username** (Overlay) und **Kontext/Info** hinzu.\n\
• Durch das Absenden bestätigst du, dass die Einverständnis des Erstellers vorliegt.\n\
• Durch das Absenden dürfen wir den Clip frei verwenden; Credits erscheinen im Video.\n";

// ── Pure Logik ─────────────────────────────────────────────────────────────

/// Wochenfenster wie `_compute_week_window_berlin`: Start Sonntag 00:00,
/// Ende Samstag 23:00 (Europe/Berlin) — als Unix-Timestamps.
pub fn compute_week_window(now_utc: chrono::DateTime<chrono::Utc>) -> (i64, i64) {
    let berlin = now_utc.with_timezone(&chrono_tz::Europe::Berlin);
    let local = berlin
        .date_naive()
        .and_hms_opt(chrono::Timelike::hour(&berlin), 0, 0)
        .unwrap_or(berlin.naive_local());
    // weekday(): Mo=0 … So=6; Fensterstart ist Sonntag (6)
    let days_since_sun =
        (chrono::Datelike::weekday(&local).num_days_from_monday() as i64 - 6).rem_euclid(7);
    let start_local = (local - chrono::Duration::days(days_since_sun))
        .date()
        .and_hms_opt(0, 0, 0)
        .expect("00:00 existiert");
    let end_local = (start_local + chrono::Duration::days(6))
        .date()
        .and_hms_opt(23, 0, 0)
        .expect("23:00 existiert");
    let to_ts = |naive: chrono::NaiveDateTime| {
        chrono_tz::Europe::Berlin
            .from_local_datetime(&naive)
            .earliest()
            .map(|dt| dt.timestamp())
            .unwrap_or_default()
    };
    (to_ts(start_local), to_ts(end_local))
}

/// Fenster-Statuszeile mit Discord-Timestamps (wie `_window_line`).
pub fn window_line(start_ts: i64, end_ts: i64, now_ts: i64) -> String {
    if start_ts <= now_ts && now_ts <= end_ts {
        format!(
            "🏁 **Teilnahmefenster aktiv**: <t:{start_ts}:f> – <t:{end_ts}:f> (endet <t:{end_ts}:R>)"
        )
    } else {
        format!("🗓️ Nächstes Fenster: <t:{start_ts}:f> – <t:{end_ts}:f> (startet <t:{start_ts}:R>)")
    }
}

/// Einfache URL-Prüfung wie `URL_RE.fullmatch` (https?://ohne-Whitespace).
pub fn is_valid_url(link: &str) -> bool {
    let lower = link.to_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && link.len() > 8
        && !link.chars().any(char::is_whitespace)
}

#[derive(Debug, Clone)]
pub struct DumpRow {
    pub id: i64,
    pub user_id: u64,
    pub created_at: String,
    pub credit: String,
    pub link: String,
    pub permission: String,
    pub info: String,
}

/// TXT-Dump-Inhalt (Format wie `_send_window_dump`).
pub fn dump_text(
    guild_name: &str,
    guild_id: u64,
    start_ts: i64,
    end_ts: i64,
    rows: &[DumpRow],
) -> String {
    if rows.is_empty() {
        return "# (keine Einsendungen in diesem Fenster)".to_string();
    }
    let fmt = |ts: i64| {
        chrono_tz::Europe::Berlin
            .timestamp_opt(ts, 0)
            .single()
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default()
    };
    let mut lines = vec![
        format!(
            "# Deadlock Clips – Fenster {} → {} (Europe/Berlin)",
            fmt(start_ts),
            fmt(end_ts)
        ),
        format!("# Guild: {guild_name} ({guild_id})"),
        String::new(),
        "id | user_id | created_at | credit | link | permission | info".to_string(),
        "-".repeat(120),
    ];
    for row in rows {
        lines.push(format!(
            "{} | {} | {} | {} | {} | {} | {}",
            row.id,
            row.user_id,
            row.created_at,
            row.credit,
            row.link,
            row.permission,
            row.info.replace('\n', " ")
        ));
    }
    lines.join("\n")
}

// ── Store ──────────────────────────────────────────────────────────────────

pub struct ClipStore {
    pub pool: PgPool,
}

impl ClipStore {
    /// Schema gehört zentral `0008_clips.sql`; der Hook bleibt als Start-Guard.
    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            SELECT to_regclass('clips.clip_submissions') IS NOT NULL AS "exists!"
            "#
        )
        .fetch_one(&self.pool)
        .await
        .map(|_| ())
    }

    /// Aktuelles Wochenfenster anlegen/holen → (id, start_ts, end_ts, status, dump_sent_ts).
    pub async fn ensure_window(
        &self,
        guild_id: u64,
    ) -> Option<(i64, i64, i64, String, Option<i64>)> {
        let (start_ts, end_ts) = compute_week_window(chrono::Utc::now());
        let guild_id = u64_to_i64(guild_id, "guild_id").ok()?;
        let start_at = utc_from_unix(start_ts).ok()?;
        let end_at = utc_from_unix(end_ts).ok()?;
        sqlx::query!(
            r#"
            INSERT INTO clips.clip_windows(guild_id, start_at, end_at, status)
            VALUES ($1, $2, $3, 'running')
            ON CONFLICT(guild_id, start_at, end_at) DO NOTHING
            "#,
            guild_id,
            start_at,
            end_at,
        )
        .execute(&self.pool)
        .await
        .ok()?;

        let row = sqlx::query!(
            r#"
            SELECT id, start_at, end_at, status, dump_sent_at
              FROM clips.clip_windows
             WHERE guild_id = $1 AND start_at = $2 AND end_at = $3
             LIMIT 1
            "#,
            guild_id,
            start_at,
            end_at,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()??;
        Some((
            row.id,
            unix_from_utc(row.start_at),
            unix_from_utc(row.end_at),
            row.status,
            row.dump_sent_at.map(unix_from_utc),
        ))
    }

    /// Einsendung speichern + laufendem Fenster zuordnen.
    pub async fn insert_submission(
        &self,
        guild_id: u64,
        user_id: u64,
        link: String,
        credit: String,
        permission: String,
        info: String,
    ) -> Option<i64> {
        let window = self.ensure_window(guild_id).await;
        let now_ts = chrono::Utc::now().timestamp();
        let running_window_id = window
            .filter(|(_, start, end, _, _)| *start <= now_ts && now_ts <= *end)
            .map(|(id, _, _, _, _)| id);
        let guild_id = u64_to_i64(guild_id, "guild_id").ok()?;
        let user_id_i64 = u64_to_i64(user_id, "user_id").ok()?;
        let row = sqlx::query!(
            r#"
            INSERT INTO clips.clip_submissions(guild_id, user_id, link, credit, permission, info)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
            guild_id,
            user_id_i64,
            link,
            credit,
            permission,
            info,
        )
        .fetch_one(&self.pool)
        .await
        .ok()?;
        if let Some(window_id) = running_window_id {
            let _ = sqlx::query!(
                r#"
                INSERT INTO clips.clip_window_submissions(window_id, submission_id, user_id)
                VALUES ($1, $2, $3)
                ON CONFLICT(window_id, submission_id) DO NOTHING
                "#,
                window_id,
                row.id,
                user_id_i64,
            )
            .execute(&self.pool)
            .await;
        }
        Some(row.id)
    }

    pub async fn dump_rows(&self, guild_id: u64, start_ts: i64, end_ts: i64) -> Vec<DumpRow> {
        let Ok(guild_id) = u64_to_i64(guild_id, "guild_id") else {
            return Vec::new();
        };
        let Ok(start_at) = utc_from_unix(start_ts) else {
            return Vec::new();
        };
        let Ok(end_at) = utc_from_unix(end_ts) else {
            return Vec::new();
        };
        let rows = sqlx::query!(
            r#"
            SELECT id, user_id, link, credit, permission, info, created_at
              FROM clips.clip_submissions
             WHERE guild_id = $1
               AND created_at >= $2
               AND created_at <= $3
             ORDER BY created_at ASC
            "#,
            guild_id,
            start_at,
            end_at,
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        rows.into_iter()
            .filter_map(|row| {
                let user_id = i64_to_u64(row.user_id, "user_id")?;
                Some(DumpRow {
                    id: row.id,
                    user_id,
                    link: row.link,
                    credit: row.credit,
                    permission: row.permission,
                    info: row.info.unwrap_or_default(),
                    created_at: row
                        .created_at
                        .map(|dt| dt.naive_utc().format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_default(),
                })
            })
            .collect()
    }

    pub async fn mark_dumped(&self, window_id: i64) {
        let _ = sqlx::query!(
            r#"
            UPDATE clips.clip_windows
               SET status = 'dumped', dump_sent_at = $1
             WHERE id = $2
            "#,
            Utc::now(),
            window_id,
        )
        .execute(&self.pool)
        .await;
    }

    /// Interface-Message aus bot.persistent_views.
    pub async fn interface_message(&self, guild_id: u64) -> Option<(u64, u64)> {
        let guild_id = u64_to_i64(guild_id, "guild_id").ok()?;
        let row = sqlx::query!(
            r#"
            SELECT channel_id, message_id
              FROM bot.persistent_views
             WHERE guild_id = $1 AND view_type = $2
             ORDER BY created_at DESC
             LIMIT 1
            "#,
            guild_id,
            VIEW_TYPE,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()??;
        Some((
            i64_to_u64(row.channel_id, "channel_id")?,
            i64_to_u64(row.message_id, "message_id")?,
        ))
    }

    pub async fn save_interface_message(&self, guild_id: u64, channel_id: u64, message_id: u64) {
        let (Ok(guild_id), Ok(channel_id), Ok(message_id)) = (
            u64_to_i64(guild_id, "guild_id"),
            u64_to_i64(channel_id, "channel_id"),
            u64_to_i64(message_id, "message_id"),
        ) else {
            return;
        };
        let _ = sqlx::query!(
            r#"
            DELETE FROM bot.persistent_views
             WHERE guild_id = $1 AND view_type = $2
            "#,
            guild_id,
            VIEW_TYPE,
        )
        .execute(&self.pool)
        .await;
        let _ = sqlx::query!(
            r#"
            INSERT INTO bot.persistent_views(message_id, channel_id, guild_id, view_type)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT(message_id) DO UPDATE SET
              channel_id = excluded.channel_id,
              guild_id = excluded.guild_id,
              view_type = excluded.view_type,
              created_at = now()
            "#,
            message_id,
            channel_id,
            guild_id,
            VIEW_TYPE,
        )
        .execute(&self.pool)
        .await;
    }
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait ClipPort: Send + Sync {
    /// Interface-Embed posten/aktualisieren → message_id.
    async fn upsert_interface(
        &self,
        channel_id: u64,
        existing_message_id: Option<u64>,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> Result<u64, String>;
    /// TXT-Datei per DM an den Kurator (Fallback: Submit-Kanal).
    async fn send_dump(
        &self,
        user_id: u64,
        fallback_channel_id: u64,
        caption: String,
        filename: String,
        content: String,
    );
    async fn guild_name(&self, guild_id: u64) -> String;
}

pub struct ClipSubmission {
    pub store: ClipStore,
    pub port: Arc<dyn ClipPort>,
    cooldown: tokio::sync::Mutex<std::collections::HashMap<u64, std::time::Instant>>,
}

impl ClipSubmission {
    pub fn new(pool: PgPool, port: Arc<dyn ClipPort>) -> Arc<Self> {
        Arc::new(Self {
            store: ClipStore { pool },
            port,
            cooldown: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    fn interface_embed(&self, start_ts: i64, end_ts: i64) -> serde_json::Value {
        let line = window_line(start_ts, end_ts, chrono::Utc::now().timestamp());
        json!({
            "title": INTERFACE_TITLE,
            "description": format!("{RULES_TEXT}\n\n{line}"),
            "color": 0x2ECC71,
            "footer": { "text": "Mit dem Button unten kannst du deinen Clip einreichen." },
        })
    }

    /// Interface posten/aktualisieren (wie `upsert_interface`).
    pub async fn refresh_interface(&self, guild_id: u64) {
        let Some((_, start_ts, end_ts, _, _)) = self.store.ensure_window(guild_id).await else {
            return;
        };
        let existing = self.store.interface_message(guild_id).await;
        let components = json!([{ "type": 1, "components": [{
            "type": 2, "style": 1, "label": "Clip einsenden", "custom_id": "clip_submit_btn_v1",
        }]}]);
        match self
            .port
            .upsert_interface(
                SUBMIT_CHANNEL_ID,
                existing.map(|(_, message_id)| message_id),
                self.interface_embed(start_ts, end_ts),
                components,
            )
            .await
        {
            Ok(message_id) => {
                if existing.map(|(_, m)| m) != Some(message_id) {
                    self.store
                        .save_interface_message(guild_id, SUBMIT_CHANNEL_ID, message_id)
                        .await;
                }
            }
            Err(err) => tracing::warn!(%err, "Clip-Interface-Upsert fehlgeschlagen"),
        }
    }

    /// Abgelaufenes Fenster genau einmal dumpen (wie `weekly_window_manager`).
    pub async fn process_window(&self, guild_id: u64) {
        let Some((window_id, start_ts, end_ts, status, dumped)) =
            self.store.ensure_window(guild_id).await
        else {
            return;
        };
        let now_ts = chrono::Utc::now().timestamp();
        if now_ts > end_ts && dumped.unwrap_or(0) == 0 && status == "running" {
            let rows = self.store.dump_rows(guild_id, start_ts, end_ts).await;
            let guild_name = self.port.guild_name(guild_id).await;
            let content = dump_text(&guild_name, guild_id, start_ts, end_ts, &rows);
            self.port
                .send_dump(
                    SEND_TO_USER_ID,
                    SUBMIT_CHANNEL_ID,
                    format!("📦 **Wochen-Dump (Clips)** – {guild_name}"),
                    format!("deadlock_clips_{guild_id}_{start_ts}_{end_ts}.txt"),
                    content,
                )
                .await;
            self.store.mark_dumped(window_id).await;
        }
    }
}

// ── Interaction-Handler ────────────────────────────────────────────────────

struct ClipHandler {
    clips: Arc<ClipSubmission>,
}

#[async_trait::async_trait]
impl InteractionHandler for ClipHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        match interaction.custom_id.as_str() {
            "clip_submit_btn_v1" => BridgeReply {
                content: Some("Bitte bestätige zunächst die Verwendungserlaubnis:".to_string()),
                components: Some(json!([{ "type": 1, "components": [{
                    "type": 2, "style": 3,
                    "label": "Ich bin Ersteller oder Erlaubnis liegt vor",
                    "custom_id": "clip_perm_yes_v1",
                }]}])),
                ephemeral: true,
                ..BridgeReply::default()
            },
            "clip_perm_yes_v1" => BridgeReply {
                modal: Some(ModalSpec {
                    custom_id: "clip_submit_modal_v1".to_string(),
                    title: "Gameplay-Clip einreichen".to_string(),
                    fields: vec![
                        ModalField {
                            custom_id: "clip_link".to_string(),
                            label: "Clip-Link (YouTube/Twitch/etc.)".to_string(),
                            placeholder: "https://…".to_string(),
                            value: None,
                            required: true,
                            min_length: 0,
                            max_length: 400,
                            paragraph: false,
                        },
                        ModalField {
                            custom_id: "credit".to_string(),
                            label: "Credit/Username (Overlay)".to_string(),
                            placeholder: "@DeadlockPlayer123".to_string(),
                            value: None,
                            required: true,
                            min_length: 0,
                            max_length: 100,
                            paragraph: false,
                        },
                        ModalField {
                            custom_id: "info".to_string(),
                            label: "Info (Kontext/Zeitstempel)".to_string(),
                            placeholder: "z. B. Held, Map, Timestamp 00:36, Besonderheiten"
                                .to_string(),
                            value: None,
                            required: false,
                            min_length: 0,
                            max_length: 1000,
                            paragraph: true,
                        },
                    ],
                }),
                ..BridgeReply::default()
            },
            "clip_submit_modal_v1" => {
                let get = |key: &str| {
                    interaction
                        .options
                        .get(key)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string()
                };
                let link = get("clip_link");
                let credit = get("credit");
                let info = get("info");
                if !is_valid_url(&link) {
                    return BridgeReply::ephemeral_text(
                        "❌ Der Link sieht nicht wie eine gültige URL aus. Bitte erneut versuchen.",
                    );
                }
                {
                    let mut cooldown = self.clips.cooldown.lock().await;
                    let now = std::time::Instant::now();
                    if let Some(last) = cooldown.get(&interaction.user_id) {
                        if now.duration_since(*last) < Duration::from_secs(COOLDOWN_SECONDS) {
                            return BridgeReply::ephemeral_text(
                                "⏱️ Bitte warte kurz bevor du erneut einsendest (60 Sek. Cooldown).",
                            );
                        }
                    }
                    cooldown.insert(interaction.user_id, now);
                }
                let saved = self
                    .clips
                    .store
                    .insert_submission(
                        interaction.guild_id,
                        interaction.user_id,
                        link,
                        credit,
                        // über den Bestätigungs-Button kommt immer diese Stufe
                        "owner_or_permission".to_string(),
                        info,
                    )
                    .await;
                match saved {
                    Some(_) => BridgeReply::ephemeral_text(
                        "✅ Danke! Dein Clip wurde eingereicht und nimmt am aktuellen Fenster teil.",
                    ),
                    None => BridgeReply::ephemeral_text(
                        "❌ Speichern fehlgeschlagen — bitte versuch es gleich nochmal.",
                    ),
                }
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}

pub fn register(router: &mut InteractionRouter, clips: Arc<ClipSubmission>) {
    let handler = Arc::new(ClipHandler { clips });
    router.on_custom_id("clip_submit_btn_v1", handler.clone());
    router.on_custom_id("clip_perm_yes_v1", handler.clone());
    router.on_custom_id("clip_submit_modal_v1", handler);
}

/// Loops wie das Original: Interface-Refresh 5 min, Fenster-Manager 2 min.
pub fn spawn(clips: Arc<ClipSubmission>) -> Vec<tokio::task::JoinHandle<()>> {
    let refresher = {
        let clips = clips.clone();
        tokio::spawn(async move {
            if let Err(err) = clips.store.ensure_schema().await {
                tracing::warn!(%err, "Clip-Schema-Anlage fehlgeschlagen");
            }
            loop {
                clips.refresh_interface(GUILD_ID).await;
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
        })
    };
    let window_manager = tokio::spawn(async move {
        // kurz versetzt starten, damit das Schema sicher steht
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            clips.process_window(GUILD_ID).await;
            tokio::time::sleep(Duration::from_secs(120)).await;
        }
    });
    vec![refresher, window_manager]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wochenfenster_berlin() {
        // Mittwoch, 10.06.2026 12:00 UTC → Fenster So 07.06. 00:00 bis Sa 13.06. 23:00 Berlin
        let now = chrono::Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();
        let (start, end) = compute_week_window(now);
        let berlin_start = chrono_tz::Europe::Berlin.timestamp_opt(start, 0).unwrap();
        let berlin_end = chrono_tz::Europe::Berlin.timestamp_opt(end, 0).unwrap();
        assert_eq!(
            berlin_start.format("%Y-%m-%d %H:%M %a").to_string(),
            "2026-06-07 00:00 Sun"
        );
        assert_eq!(
            berlin_end.format("%Y-%m-%d %H:%M %a").to_string(),
            "2026-06-13 23:00 Sat"
        );
        // Sonntag selbst ist Fensterstart
        let sunday = chrono::Utc.with_ymd_and_hms(2026, 6, 7, 10, 0, 0).unwrap();
        let (start2, _) = compute_week_window(sunday);
        assert_eq!(start2, start);
        assert!(start < end);
    }

    #[test]
    fn fensterzeile_und_url() {
        let line = window_line(100, 200, 150);
        assert!(line.starts_with("🏁") && line.contains("<t:200:R>"));
        let line = window_line(100, 200, 50);
        assert!(line.starts_with("🗓️") && line.contains("<t:100:R>"));
        assert!(is_valid_url("https://clips.twitch.tv/abc"));
        assert!(!is_valid_url("ftp://nope"));
        assert!(!is_valid_url("https://mit leerzeichen"));
    }

    #[test]
    fn dump_format() {
        assert_eq!(
            dump_text("G", 1, 0, 1, &[]),
            "# (keine Einsendungen in diesem Fenster)"
        );
        let rows = vec![DumpRow {
            id: 7,
            user_id: 42,
            created_at: "2026-06-08 18:00:00".to_string(),
            credit: "@Nani".to_string(),
            link: "https://x".to_string(),
            permission: "owner_or_permission".to_string(),
            info: "Zeile1\nZeile2".to_string(),
        }];
        let text = dump_text("Guild", 1, 0, 1, &rows);
        assert!(text.contains("# Guild: Guild (1)"));
        assert!(text.contains("7 | 42 | 2026-06-08 18:00:00 | @Nani | https://x | owner_or_permission | Zeile1 Zeile2"));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn fenster_und_einsendung() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let store = ClipStore {
            pool: db.pool().clone(),
        };
        store.ensure_schema().await.expect("schema");
        let (id1, start, end, status, dumped) = store.ensure_window(1).await.expect("window");
        assert_eq!(status, "running");
        assert!(dumped.is_none());
        // idempotent
        let (id2, ..) = store.ensure_window(1).await.expect("window2");
        assert_eq!(id1, id2);
        assert!(start <= chrono::Utc::now().timestamp());
        assert!(end > start);
        let submission = store
            .insert_submission(
                1,
                42,
                "https://x".into(),
                "@n".into(),
                "owner_or_permission".into(),
                "".into(),
            )
            .await;
        assert!(submission.is_some());
        let rows = store
            .dump_rows(1, start, chrono::Utc::now().timestamp() + 10)
            .await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].user_id, 42);
        store.mark_dumped(id1).await;
        let (_, _, _, status, dumped) = store.ensure_window(1).await.expect("window3");
        assert_eq!(status, "dumped");
        assert!(dumped.is_some());
    }
}
