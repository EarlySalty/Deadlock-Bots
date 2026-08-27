//! Website-Invites — Port von `cogs/website_invite_cog.py`.
//!
//! Pro Website-Unterseite (Landing, Streamer, Mitspieler, Coaching, Helden,
//! Guides) lebt ein permanenter Invite-Code im kv_store
//! (`website_invites`/slug, Landing gespiegelt auf `main`). Beim Start wird
//! geprüft, ob die Codes noch existieren — fehlende werden neu erstellt.
//! Dazu die Join-Quellen-Auswertung über `member_events` (Owner-Command).

use std::collections::HashMap;
use std::sync::Arc;

use dl_central_db::kv;
use serde_json::{json, Value};
use sqlx::PgPool;

pub const KV_NAMESPACE: &str = "website_invites";
pub const KV_KEY_MAIN: &str = "main";
pub const WEBSITE_SOURCE_LABEL: &str = "Website";
pub const DEFAULT_WELCOME_CHANNEL_ID: u64 = 1315684135175716975;

pub const WEBSITE_SUBPAGES: [(&str, &str); 6] = [
    ("landing", "Landing"),
    ("streamer", "Streamer"),
    ("mitspieler", "Mitspieler"),
    ("coaching", "Coaching"),
    ("helden", "Helden"),
    ("guides", "Guides"),
];

// ── Pure Klassifikation (wie cmd_join_sources) ─────────────────────────────

/// Rohe, ungeformatte Quell-Art eines Joins. Spiegelt die Entscheidung von
/// `classify_join_source`, liefert aber stabile Rohwerte statt Anzeige-Labels,
/// damit sich Konsumenten (z.B. das Verify-Gate) nicht an formatierte Strings
/// koppeln muessen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinSourceKind {
    /// Join ueber einen konfigurierten Website-Invite-Code.
    Website,
    /// Vanity-Link (Discord-Listings).
    Vanity,
    /// Join ueber einen Twitch-Streamer.
    TwitchStreamer,
    /// Bot-OAuth-Join.
    BotInvite,
    /// Persoenlicher Invite eines Mitglieds.
    PersonalInvite,
    /// Server-Entdeckung.
    ServerDiscovery,
    /// Nicht zuordenbar.
    Unknown,
}

fn meta_str(meta: &Value, key: &str) -> String {
    meta.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Rohe Quell-Art eines member_events-Metadatensatzes. Ein Website-Invite-Code
/// hat Vorrang vor der `join_source_kind`, genau wie in `classify_join_source`.
pub fn join_source_kind(meta: &Value, website_codes: &HashMap<String, String>) -> JoinSourceKind {
    let invite_code = meta_str(meta, "invite_code");
    if !invite_code.is_empty() && website_codes.contains_key(&invite_code.to_lowercase()) {
        return JoinSourceKind::Website;
    }
    match meta_str(meta, "join_source_kind").as_str() {
        "vanity" => JoinSourceKind::Vanity,
        "twitch_streamer" => JoinSourceKind::TwitchStreamer,
        "bot_invite" => JoinSourceKind::BotInvite,
        "invite_link" => JoinSourceKind::PersonalInvite,
        "server_discovery" => JoinSourceKind::ServerDiscovery,
        _ => JoinSourceKind::Unknown,
    }
}

/// Join-Quelle eines member_events-Metadatensatzes benennen.
pub fn classify_join_source(meta: &Value, website_codes: &HashMap<String, String>) -> String {
    match join_source_kind(meta, website_codes) {
        JoinSourceKind::Website => {
            let invite_code = meta_str(meta, "invite_code");
            let label = website_codes
                .get(&invite_code.to_lowercase())
                .map_or("", String::as_str);
            format!("{WEBSITE_SOURCE_LABEL}: {label}")
        }
        JoinSourceKind::Vanity => "Vanity-Link (Discord-Listings)".to_string(),
        JoinSourceKind::TwitchStreamer => {
            let login = meta_str(meta, "twitch_streamer_login");
            if login.is_empty() {
                "Twitch-Streamer".to_string()
            } else {
                format!("Twitch: {login}")
            }
        }
        JoinSourceKind::BotInvite => "Bot-Invite".to_string(),
        JoinSourceKind::PersonalInvite => {
            let inviter = meta_str(meta, "inviter_name");
            format!(
                "Persönlicher Invite ({})",
                if inviter.is_empty() { "?" } else { &inviter }
            )
        }
        JoinSourceKind::ServerDiscovery => "Server entdecken".to_string(),
        JoinSourceKind::Unknown => {
            let label_existing = meta_str(meta, "join_source_label");
            if label_existing.is_empty() {
                "Unbekannt".to_string()
            } else {
                label_existing
            }
        }
    }
}

/// Anzeigezeilen wie das Original (Top 12, 5 %-Balken, Rest-Zeile).
pub fn format_bucket_lines(buckets: &HashMap<String, u64>, total: u64) -> Vec<String> {
    let mut sorted: Vec<(&String, &u64)> = buckets.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let max_show = 12;
    let mut lines: Vec<String> = sorted
        .iter()
        .take(max_show)
        .map(|(key, count)| {
            let pct = **count as f64 / total as f64 * 100.0;
            let bar_units = ((pct / 5.0).round() as usize).max(1);
            let bar = "▰".repeat(bar_units) + &"▱".repeat(4usize.saturating_sub(bar_units));
            format!("`{count:>4}` ({pct:5.1}%) {bar}  {key}")
        })
        .collect();
    if sorted.len() > max_show {
        let rest: u64 = sorted.iter().skip(max_show).map(|(_, c)| **c).sum();
        lines.push(format!(
            "`{rest:>4}` (rest) — {} weitere Quellen",
            sorted.len() - max_show
        ));
    }
    lines
}

// ── Store ──────────────────────────────────────────────────────────────────

pub struct InviteStore {
    pub pool: PgPool,
}

impl InviteStore {
    /// Gespeicherter Invite je Unterseite (Landing fällt auf `main` zurück).
    pub async fn invite_for_subpage(&self, slug: &str) -> Option<Value> {
        if let Some(stored) = self.load_raw(slug).await {
            return Some(stored);
        }
        if slug == "landing" {
            return self.load_raw(KV_KEY_MAIN).await;
        }
        None
    }

    async fn load_raw(&self, key: &str) -> Option<Value> {
        let raw = kv::get(&self.pool, KV_NAMESPACE, key)
            .await
            .ok()
            .flatten()?;
        let data: Value = serde_json::from_str(&raw).ok()?;
        let code = data
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        (!code.is_empty()).then_some(data.clone())
    }

    pub async fn save_invite(&self, slug: &str, code: &str, channel_id: u64) {
        let payload = serde_json::to_string(&json!({
            "code": code,
            "channel_id": channel_id,
            "created_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        }))
        .unwrap_or_default();
        let _ = kv::set(&self.pool, KV_NAMESPACE, slug, &payload).await;
        if slug == "landing" {
            let _ = kv::set(&self.pool, KV_NAMESPACE, KV_KEY_MAIN, &payload).await;
        }
    }

    /// Code → Label aller konfigurierten Website-Invites.
    pub async fn website_code_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (slug, label) in WEBSITE_SUBPAGES {
            if let Some(stored) = self.invite_for_subpage(slug).await {
                if let Some(code) = stored.get("code").and_then(Value::as_str) {
                    map.insert(code.to_lowercase(), label.to_string());
                }
            }
        }
        map
    }

    /// Join-Quellen der letzten N Tage aus member_events aggregieren.
    pub async fn join_source_buckets(
        &self,
        days: i64,
        guild_id: Option<u64>,
    ) -> (HashMap<String, u64>, u64) {
        let days = days.clamp(1, 365);
        let since = chrono::Utc::now() - chrono::Duration::days(days);
        let guild_id = guild_id.and_then(|id| i64::try_from(id).ok());
        let rows: Vec<Option<String>> = sqlx::query!(
            r#"
            SELECT metadata::text AS metadata
              FROM activity.member_events
             WHERE event_type = 'join'
               AND occurred_at >= $1
               AND ($2::bigint IS NULL OR guild_id = $2)
            "#,
            since,
            guild_id,
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(|row| row.metadata).collect())
        .unwrap_or_default();
        let website_codes = self.website_code_map().await;
        let mut buckets: HashMap<String, u64> = HashMap::new();
        let total = rows.len() as u64;
        for raw in rows {
            let meta: Value = raw
                .as_deref()
                .and_then(|r| serde_json::from_str(r).ok())
                .unwrap_or(Value::Null);
            let key = classify_join_source(&meta, &website_codes);
            *buckets.entry(key).or_default() += 1;
        }
        (buckets, total)
    }
}

// ── Lifecycle ──────────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait InvitePort: Send + Sync {
    /// Aktive Invite-Codes der Guild.
    async fn guild_invite_codes(&self, guild_id: u64) -> Vec<String>;
    /// Permanenten Invite erstellen (max_age=0, max_uses=0) → Code.
    async fn create_permanent_invite(
        &self,
        channel_id: u64,
        reason: &str,
    ) -> Result<String, String>;
}

pub struct WebsiteInvites {
    pub store: InviteStore,
    pub port: Arc<dyn InvitePort>,
    pub guild_id: u64,
    pub welcome_channel_id: u64,
}

impl WebsiteInvites {
    /// Beim Start: tote/fehlende Codes neu erstellen (wie _ensure_invite).
    pub async fn ensure_invites(&self) {
        let alive: std::collections::HashSet<String> = self
            .port
            .guild_invite_codes(self.guild_id)
            .await
            .into_iter()
            .map(|c| c.to_lowercase())
            .collect();
        for (slug, label) in WEBSITE_SUBPAGES {
            let stored_code = self
                .store
                .invite_for_subpage(slug)
                .await
                .and_then(|s| s.get("code").and_then(Value::as_str).map(str::to_string));
            let needs_new = match &stored_code {
                Some(code) => !alive.contains(&code.to_lowercase()),
                None => true,
            };
            if !needs_new {
                continue;
            }
            match self
                .port
                .create_permanent_invite(
                    self.welcome_channel_id,
                    &format!("Website-Invite: {label}"),
                )
                .await
            {
                Ok(code) => {
                    self.store
                        .save_invite(slug, &code, self.welcome_channel_id)
                        .await;
                    tracing::info!(slug, code, "Website-Invite neu erstellt");
                }
                Err(err) => {
                    tracing::warn!(%err, slug, "Website-Invite-Erstellung fehlgeschlagen")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klassifikation_wie_python() {
        let mut codes = HashMap::new();
        codes.insert("abc123".to_string(), "Streamer".to_string());
        let case = |meta: Value| classify_join_source(&meta, &codes);
        assert_eq!(
            case(json!({ "invite_code": "ABC123" })),
            "Website: Streamer"
        );
        assert_eq!(
            case(json!({ "join_source_kind": "vanity" })),
            "Vanity-Link (Discord-Listings)"
        );
        assert_eq!(
            case(json!({ "join_source_kind": "twitch_streamer", "twitch_streamer_login": "nani" })),
            "Twitch: nani"
        );
        assert_eq!(
            case(json!({ "join_source_kind": "invite_link", "inviter_name": "Ben" })),
            "Persönlicher Invite (Ben)"
        );
        assert_eq!(
            case(json!({ "join_source_kind": "invite_link" })),
            "Persönlicher Invite (?)"
        );
        assert_eq!(
            case(json!({ "join_source_kind": "server_discovery" })),
            "Server entdecken"
        );
        assert_eq!(case(json!({})), "Unbekannt");
        assert_eq!(
            case(json!({ "join_source_label": "Sonstiges" })),
            "Sonstiges"
        );
    }

    #[test]
    fn balken_zeilen() {
        let mut buckets = HashMap::new();
        buckets.insert("Website: Landing".to_string(), 60u64);
        buckets.insert("Unbekannt".to_string(), 40u64);
        let lines = format_bucket_lines(&buckets, 100);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("60") && lines[0].contains("60.0%"));
        assert!(lines[0].contains("Website: Landing"));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn kv_vertrag_mit_landing_spiegel() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let store = InviteStore {
            pool: db.pool().clone(),
        };
        store.save_invite("landing", "xYz", 42).await;
        // Landing wird auf main gespiegelt
        let main = store.invite_for_subpage("landing").await.expect("landing");
        assert_eq!(main["code"], "xYz");
        let raw_main = store.load_raw(KV_KEY_MAIN).await.expect("main");
        assert_eq!(raw_main["code"], "xYz");
        // Andere Subpage ohne Eintrag → None
        assert!(store.invite_for_subpage("guides").await.is_none());
        let codes = store.website_code_map().await;
        assert_eq!(codes.get("xyz").map(String::as_str), Some("Landing"));
    }
}
