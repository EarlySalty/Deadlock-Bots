//! Coaching-Plattform-Brücke — Port von `cogs/coaching_platform_sync.py`
//! + `service/website_client.py`.
//!
//! Zwei Loops halten Discord und die Website-Coaching-Plattform synchron:
//! - **Rollen-Sync** (alle 10 min): Mitglieder mit der Coach-Rolle →
//!   `POST /coaching/platform/coaches/sync` — mit dem Roster-Wipe-Schutz
//!   des Originals (leere Liste wird NIE gesendet).
//! - **Notification-Poller** (alle 60 s): fällige Termin-DMs abholen
//!   (`/notifications/due`), zustellen, bestätigen (`/notifications/ack`).
//!   DM-Texte wortgleich; deaktivierte DMs werden geackt statt endlos
//!   wiederholt.

use std::sync::Arc;
use std::time::Duration;

use chrono::TimeZone;
use serde_json::{json, Value};

pub const COACH_ROLE_ID: u64 = 1494372744286965941;
pub const ROLE_SYNC_INTERVAL: Duration = Duration::from_secs(600);
pub const NOTIFICATION_INTERVAL: Duration = Duration::from_secs(60);

const WEEKDAYS_DE: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];

/// ISO-UTC → `"Mi, 11.06. um 19:00 Uhr"` (Europe/Berlin), wie `_fmt_dt`.
pub fn fmt_dt(iso_utc: &str) -> String {
    let normalized = iso_utc.replace('Z', "+00:00");
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&normalized) else {
        return iso_utc.to_string();
    };
    let berlin = chrono_tz::Europe::Berlin.from_utc_datetime(&parsed.naive_utc());
    let weekday = WEEKDAYS_DE[berlin
        .format("%u")
        .to_string()
        .parse::<usize>()
        .unwrap_or(1)
        - 1];
    format!(
        "{weekday}, {} um {} Uhr",
        berlin.format("%d.%m."),
        berlin.format("%H:%M")
    )
}

/// DM-Text je Notification-Typ (wortgleich); None bei unbekanntem Typ.
pub fn build_dm_text(item: &Value) -> Option<String> {
    let ntype = item.get("type").and_then(Value::as_str).unwrap_or("");
    let coach = item
        .get("coach_display")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("dein Coach");
    let scheduled = item
        .get("scheduled_at")
        .and_then(Value::as_str)
        .unwrap_or("");
    let datum = if scheduled.is_empty() {
        "unbekannter Zeitpunkt".to_string()
    } else {
        fmt_dt(scheduled)
    };
    let title = item.get("title").and_then(Value::as_str).unwrap_or("");
    let note = item.get("note").and_then(Value::as_str).unwrap_or("");
    let title_line = if title.is_empty() {
        String::new()
    } else {
        format!("\n{title}")
    };
    let note_line = if note.is_empty() {
        String::new()
    } else {
        format!("\n{note}")
    };
    match ntype {
        "created" => {
            let dur_str = item
                .get("duration_minutes")
                .and_then(Value::as_i64)
                .map(|d| format!(" (ca. {d} Min.)"))
                .unwrap_or_default();
            Some(format!(
                "📅 **Coaching-Termin geplant** — {coach} hat ein Coaching mit dir angesetzt: \
**{datum}**{dur_str}.{title_line}{note_line}\n\
Details findest du auf https://deutsche-deadlock-community.de/coaching unter \"Mein Coaching\"."
            ))
        }
        "reminder" => Some(format!(
            "⏰ **Erinnerung** — dein Coaching mit {coach} startet **{datum}** (in unter 2 Stunden)."
        )),
        "cancelled" => Some(format!(
            "❌ **Termin abgesagt** — dein Coaching mit {coach} am {datum} findet nicht statt. \
Bei Fragen melde dich beim Coach."
        )),
        other => {
            tracing::warn!(ntype = other, "Unbekannter Notification-Typ");
            None
        }
    }
}

// ── Website-Client ─────────────────────────────────────────────────────────

pub struct WebsiteClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl WebsiteClient {
    /// Token-Kette wie website_client.py: TWITCH_INTERNAL_API_TOKEN →
    /// MASTER_BROKER_TOKEN; None ohne Token (Loops laufen dann leer).
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Option<Arc<Self>> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let token = get("TWITCH_INTERNAL_API_TOKEN")
            .or_else(|| get("MASTER_BROKER_TOKEN"))
            .or_else(|| get("COACHING_BOT_TOKEN"))?;
        let base_url = get("WEBSITE_API_BASE")
            .unwrap_or_else(|| "https://deutsche-deadlock-community.de/api".to_string());
        Some(Arc::new(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
        }))
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base_url))
            .header("X-Internal-Token", &self.token)
            .header("X-Bot-Token", &self.token)
    }

    /// Coach-Roster übermitteln — true bei Erfolg (best-effort).
    pub async fn sync_coaches(&self, coaches: &[Value]) -> bool {
        match self
            .request(reqwest::Method::POST, "/coaching/platform/coaches/sync")
            .json(&json!({ "coaches": coaches }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => true,
            Ok(response) => {
                tracing::warn!(status = %response.status(), "Coach-Sync fehlgeschlagen");
                false
            }
            Err(err) => {
                tracing::warn!(%err, "Coach-Sync Fehler");
                false
            }
        }
    }

    /// Fällige Termin-Benachrichtigungen; bei Fehler leer (nie Abbruch).
    pub async fn due_notifications(&self) -> Vec<Value> {
        match self
            .request(reqwest::Method::GET, "/coaching/platform/notifications/due")
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => response
                .json::<Value>()
                .await
                .ok()
                .and_then(|data| data.get("notifications").and_then(Value::as_array).cloned())
                .unwrap_or_default(),
            Ok(response) => {
                tracing::warn!(status = %response.status(), "Notification-Poll fehlgeschlagen");
                Vec::new()
            }
            Err(err) => {
                tracing::warn!(%err, "Notification-Poll Fehler");
                Vec::new()
            }
        }
    }

    pub async fn ack_notifications(&self, items: &[Value]) -> bool {
        if items.is_empty() {
            return true;
        }
        match self
            .request(
                reqwest::Method::POST,
                "/coaching/platform/notifications/ack",
            )
            .json(&json!({ "items": items }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => true,
            Ok(response) => {
                tracing::warn!(status = %response.status(), "Notification-Ack fehlgeschlagen");
                false
            }
            Err(err) => {
                tracing::warn!(%err, "Notification-Ack Fehler");
                false
            }
        }
    }
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait CoachingPort: Send + Sync {
    /// Mitglieder mit der Coach-Rolle: (user_id, username, display_name, avatar_url).
    async fn coach_members(&self, role_id: u64) -> Vec<(u64, String, String, String)>;
    /// DM senden — Ok(false) = DMs deaktiviert (ackbar), Err = später erneut.
    async fn send_dm(&self, user_id: u64, text: String) -> Result<bool, String>;
}

pub struct CoachingSync {
    pub client: Arc<WebsiteClient>,
    pub port: Arc<dyn CoachingPort>,
}

impl CoachingSync {
    pub async fn run_role_sync(&self) {
        let members = self.port.coach_members(COACH_ROLE_ID).await;
        if members.is_empty() {
            // Roster-Wipe-Schutz wie das Original
            tracing::warn!("Coach-Rolle hat keine Mitglieder – Sync wird NICHT ausgeführt");
            return;
        }
        let coaches: Vec<Value> = members
            .iter()
            .map(|(user_id, username, display_name, avatar_url)| {
                json!({
                    "discord_user_id": user_id,
                    "discord_username": username,
                    "display_name": display_name,
                    "avatar_url": avatar_url,
                })
            })
            .collect();
        if self.client.sync_coaches(&coaches).await {
            tracing::info!(count = coaches.len(), "Coach-Sync erfolgreich");
        }
    }

    pub async fn process_notifications(&self) {
        let items = self.client.due_notifications().await;
        if items.is_empty() {
            return;
        }
        let mut to_ack: Vec<Value> = Vec::new();
        for item in items {
            let Some(user_id) = item.get("discord_user_id").and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            }) else {
                tracing::warn!("Notification ohne discord_user_id – übersprungen");
                continue;
            };
            let Some(text) = build_dm_text(&item) else {
                continue;
            };
            match self.port.send_dm(user_id, text).await {
                Ok(true) => to_ack.push(item),
                Ok(false) => {
                    tracing::warn!(user_id, "DMs deaktiviert – wird geackt ohne Zustellung");
                    to_ack.push(item);
                }
                Err(err) => {
                    tracing::warn!(%err, user_id, "DM-Versand fehlgeschlagen (wird erneut versucht)");
                }
            }
        }
        if !to_ack.is_empty() && self.client.ack_notifications(&to_ack).await {
            tracing::info!(count = to_ack.len(), "Notifications geackt");
        }
    }
}

pub fn spawn(sync: Arc<CoachingSync>) -> Vec<tokio::task::JoinHandle<()>> {
    let role_sync = sync.clone();
    let role_task = tokio::spawn(async move {
        loop {
            role_sync.run_role_sync().await;
            tokio::time::sleep(ROLE_SYNC_INTERVAL).await;
        }
    });
    let notification_task = tokio::spawn(async move {
        loop {
            sync.process_notifications().await;
            tokio::time::sleep(NOTIFICATION_INTERVAL).await;
        }
    });
    vec![role_task, notification_task]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datum_formatierung_berlin() {
        // 2026-06-10T17:00:00Z = Mittwoch 19:00 CEST
        assert_eq!(fmt_dt("2026-06-10T17:00:00Z"), "Mi, 10.06. um 19:00 Uhr");
        // Winterzeit: 2026-01-05T18:00:00Z = Montag 19:00 CET
        assert_eq!(fmt_dt("2026-01-05T18:00:00Z"), "Mo, 05.01. um 19:00 Uhr");
        // kaputter String bleibt unverändert
        assert_eq!(fmt_dt("quatsch"), "quatsch");
    }

    #[test]
    fn dm_texte_wie_python() {
        let created = json!({
            "type": "created",
            "coach_display": "Nani",
            "scheduled_at": "2026-06-10T17:00:00Z",
            "duration_minutes": 60,
            "title": "Laning-Review",
        });
        let text = build_dm_text(&created).expect("created");
        assert!(text.contains("📅 **Coaching-Termin geplant** — Nani"));
        assert!(text.contains("**Mi, 10.06. um 19:00 Uhr** (ca. 60 Min.)"));
        assert!(text.contains("\nLaning-Review"));
        assert!(text.contains("deutsche-deadlock-community.de/coaching"));

        let reminder = json!({ "type": "reminder", "scheduled_at": "2026-06-10T17:00:00Z" });
        let text = build_dm_text(&reminder).expect("reminder");
        assert!(text.contains("⏰ **Erinnerung** — dein Coaching mit dein Coach"));
        assert!(text.contains("(in unter 2 Stunden)"));

        let cancelled = json!({ "type": "cancelled", "coach_display": "Nani" });
        let text = build_dm_text(&cancelled).expect("cancelled");
        assert!(text.contains("❌ **Termin abgesagt**"));
        assert!(text.contains("unbekannter Zeitpunkt"));

        assert!(build_dm_text(&json!({ "type": "weird" })).is_none());
    }
}
