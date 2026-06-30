//! Coaching-Anfragen — Website-getriebener Einstieg mit Resten aus dem
//! früheren `cogs/coaching_panel.py`/`cogs/coaching_request.py`-Port.
//!
//! Das Panel verweist inzwischen per Link-Button auf die Coaching-Website.
//! Der Discord-interne Anfrage-/Analyse-/Rollen-Flow wird für neue Anfragen
//! bewusst nicht mehr gestartet.
//!
//! Die Feedback-Umfrage (`coaching_survey`) ist hier mit portiert: ein
//! 60-s-Poll plus ein Voice-Event-Listener erkennen das Ende einer Session
//! (User + Coach nicht mehr im selben Coaching-VC), vergeben die Reward-Rolle,
//! nehmen die Active-Rolle weg und schicken die Survey-DM.
//!
//! Dokumentierte Lücken: Website-Spiegelung der Sessions und der
//! Rollen-Ablauf-Manager (coaching_role_manager) — laufen vorerst in Python
//! weiter. Die beiden Owner-only-Slash-Befehle des Survey-Cogs
//! (`coaching-survey-senden`, `coaching-session-beenden`) sind manuelle
//! Overrides des Automatik-Flows und bleiben vorerst aus.

use std::sync::Arc;

use dl_ai::{GenerateRequest, TextGenerator};
use dl_central_db::kv;
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use serde_json::{json, Map, Value};
use sqlx::PgPool;

use crate::db::{
    advisory_lock, i64_to_i32, pg_i64_to_u64, u64_to_i64, unix_from_utc, utc_from_unix,
};

pub const COACHING_PANEL_CHANNEL_ID: u64 = 1494373349944459355;
pub const COACH_ROLE_ID: u64 = 1494372744286965941;
pub const COACHING_ACTIVE_ROLE_ID: u64 = 1371929762913587292;
pub const COACHING_REWARD_ROLE_ID: u64 = 1500793970714873927;
pub const REQUEST_CHANNEL_ID: u64 = 1461682293105229979;
/// Kategorie der Coaching-Voice-Channels (`settings.coaching_voice_category_id`).
pub const COACHING_VOICE_CATEGORY_ID: u64 = 1459526231686119600;
/// Feedback-Kanal, auf den die Survey-DM verlinkt (`coaching_feedback_channel_id`).
pub const COACHING_FEEDBACK_CHANNEL_ID: u64 = 1494756126644895885;
/// Reward-Rolle gilt 5 Tage (wie Python: `5 * 24 * 60 * 60`).
pub const REWARD_ROLE_DURATION_SECS: i64 = 5 * 24 * 60 * 60;
pub const OWNER_EXCLUDE_ID: u64 = 662995601738170389;
/// Coaches die NICHT automatisch per Round-Robin zugewiesen werden (claimen bleibt erlaubt).
pub const AUTO_ASSIGN_OPTOUT_IDS: &[u64] = &[907263048715239456];
pub const CLAIM_RESERVATION_HOURS: i64 = 24;
pub const ROLE_EXPIRY_HOURS: i64 = 168;
pub const COACHING_WEBSITE_URL: &str = "https://deutsche-deadlock-community.de/coaching";
pub const COACHING_WEBSITE_CTA_TEXT: &str = "👉 **Bereit loszulegen?** Stell deine Coaching-Anfrage direkt über den Button unten auf unserer Website — dort füllst du in einer Minute alles aus, der Rest läuft von selbst.";
pub const COACHING_WEBSITE_BUTTON_LABEL: &str = "Coaching-Anfrage starten";
pub const COACHING_WEBSITE_OPEN_BUTTON_LABEL: &str = "Auf der Website öffnen";
const PANEL_KV_NS: &str = "coaching";
const PANEL_KV_KEY: &str = "panel_message_id";
const COACHING_REQUESTS_ID_LOCK: i64 = 0x4451_0008_0004_0002;

pub const COACHING_ANALYSIS_SYSTEM: &str = r#"Du bist ein Deadlock Coaching Koordinator.

Der Coach sieht die Rohangaben des Spielers bereits (Rang, Hero, Games, Probleme).
Wiederhole diese NICHT – liefere direkt die Coach-Einschätzung.

Gib zurück:
1. **Key-Fokuspunkte** (3-5 Punkte): Was soll der Coach konkret trainieren? Sei spezifisch für Deadlock Gameplay (z.B. Lane-Phase, Itemreihenfolge, Positioning, Hero-Mechaniken, Map-Awareness).
2. **Coach-Ansatz** (1-2 Sätze): Welchen Einstieg empfiehlst du für die erste Session?

Keine Einleitung, kein "Der Spieler möchte..." – direkt mit den Fokuspunkten beginnen.

WICHTIG: Wenn die Anfrage offensichtlich nicht ernst gemeint ist (z. B. Nonsens-Text,
absichtlich falsche Angaben wie unmögliche Ränge, Beleidigungen, Spam oder kompletter
Blödsinn), antworte AUSSCHLIESSLICH mit dem Wort: INVALID_REQUEST
Keine Erklärung, kein weiterer Text."#;

// ── Pure Bausteine ─────────────────────────────────────────────────────────

/// Fairste Coach-Wahl (wie pick_fair_coach): am längsten nicht zugewiesen,
/// dann kleinste ID. Owner/Bots filtert
/// der Aufrufer beim Kandidaten-Sammeln.
pub fn pick_fair_coach(candidates: &[(u64, i64)]) -> Option<u64> {
    candidates
        .iter()
        .min_by_key(|(id, last_assigned_at)| (*last_assigned_at, *id))
        .map(|(id, _)| *id)
}

pub fn normalize_inline(value: &str, fallback: &str, limit: usize) -> String {
    let cleaned: String = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let base = if cleaned.is_empty() {
        fallback
    } else {
        &cleaned
    };
    if base.chars().count() <= limit {
        base.to_string()
    } else {
        let cut: String = base.chars().take(limit.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

pub fn format_ai_summary(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "Keine Analyse verfügbar.".to_string();
    }
    if trimmed.chars().count() <= 1024 {
        trimmed.to_string()
    } else {
        let cut: String = trimmed.chars().take(1023).collect();
        format!("{cut}…")
    }
}

fn combine_rank(rank: &str, subrank: &str) -> String {
    let rank = rank.trim();
    let subrank = subrank.trim();
    match (rank.is_empty(), subrank.is_empty()) {
        (true, true) => String::new(),
        (false, true) => rank.to_string(),
        (true, false) => subrank.to_string(),
        (false, false) => {
            if rank.split_whitespace().last() == Some(subrank) {
                rank.to_string()
            } else {
                format!("{rank} {subrank}")
            }
        }
    }
}

fn combine_games_hours(games_played: &str, hours_played: &str) -> String {
    let games_played = games_played.trim();
    let hours_played = hours_played.trim();
    match (games_played.is_empty(), hours_played.is_empty()) {
        (true, true) => String::new(),
        (false, true) => games_played.to_string(),
        (true, false) => hours_played.to_string(),
        (false, false) => format!("{games_played} / {hours_played}"),
    }
}

/// uuid4-Format aus Zufallsbytes (wie str(uuid.uuid4())).
pub fn new_session_id() -> String {
    let bytes: [u8; 16] = rand::random();
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-4{:01x}{:02x}-{:01x}{:01x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6] & 0x0f,
        bytes[7],
        8 + (bytes[8] & 0x03),
        bytes[8] & 0x0f,
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

/// Parameter für `mirror_to_website` (entspricht den Keyword-Args von
/// Python `_mirror_to_website`). Nur `request_id` ist Pflicht; die Rest-Felder
/// kommen je nach Lebenszyklus-Punkt dazu (Claim/Cancel/Survey).
#[derive(Default)]
struct MirrorOpts {
    request_id: i64,
    assigned_coach_username: Option<String>,
    coach_discord_id: Option<u64>,
    coach_username: Option<String>,
    session_status: Option<String>,
    bot_session_id: Option<String>,
}

/// Roh-Zeile aus `coaching_requests` für die Website-Spiegelung.
struct MirrorRow {
    id: i64,
    website_request_id: Option<String>,
    discord_user_id: i64,
    discord_username: String,
    rank: String,
    subrank: String,
    hero: Option<String>,
    games_played: Option<String>,
    hours_played: Option<String>,
    availability: Option<String>,
    current_problems: Option<String>,
    ai_summary: String,
    status: String,
    assigned_coach_id: Option<String>,
    reserved_until: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct RequestData {
    pub id: i64,
    pub user_id: u64,
    pub username: String,
    pub rank: String,
    pub hero: String,
    pub games_played: String,
    pub scheduled_slot: String,
    pub current_problems: String,
    pub ai_summary: String,
}

#[derive(Debug, Clone)]
struct RequestCreatedNotification {
    website_request_id: String,
    coachee_id: String,
    discord_user_id: u64,
    discord_username: String,
    rank: String,
    subrank: String,
    hero: Option<String>,
    games_played: Option<String>,
    hours_played: Option<String>,
    availability: Option<String>,
    current_problems: Option<String>,
    preferred_coach_id: Option<String>,
}

struct RequestCreatedUpsert {
    local_request_id: i64,
    already_posted: bool,
}

impl RequestCreatedNotification {
    fn from_item(item: &Value) -> Result<Self, String> {
        Ok(Self {
            website_request_id: required_string(item, "request_id")?,
            coachee_id: required_string(item, "coachee_id")?,
            discord_user_id: required_u64(item, "discord_user_id")?,
            discord_username: optional_string(item, "discord_username").unwrap_or_default(),
            rank: optional_string(item, "rank").unwrap_or_default(),
            subrank: optional_string(item, "subrank").unwrap_or_default(),
            hero: optional_string(item, "hero"),
            games_played: optional_string(item, "games_played"),
            hours_played: optional_string(item, "hours_played"),
            availability: optional_string(item, "availability"),
            current_problems: optional_string(item, "current_problems"),
            preferred_coach_id: optional_string(item, "preferred_coach_id"),
        })
    }

    fn request_data(&self, local_request_id: i64) -> RequestData {
        RequestData {
            id: local_request_id,
            user_id: self.discord_user_id,
            username: self.discord_username.clone(),
            rank: combine_rank(&self.rank, &self.subrank),
            hero: self.hero.clone().unwrap_or_default(),
            games_played: combine_games_hours(
                self.games_played.as_deref().unwrap_or_default(),
                self.hours_played.as_deref().unwrap_or_default(),
            ),
            scheduled_slot: self.availability.clone().unwrap_or_default(),
            current_problems: self.current_problems.clone().unwrap_or_default(),
            ai_summary: String::new(),
        }
    }
}

fn required_string(item: &Value, key: &str) -> Result<String, String> {
    optional_string(item, key).ok_or_else(|| format!("Notification ohne {key}"))
}

fn optional_string(item: &Value, key: &str) -> Option<String> {
    let value = item.get(key)?;
    let raw = match value {
        Value::Null => return None,
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => return None,
    };
    let trimmed = raw.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn required_u64(item: &Value, key: &str) -> Result<u64, String> {
    let Some(value) = item.get(key) else {
        return Err(format!("Notification ohne {key}"));
    };
    if let Some(value) = value.as_u64() {
        return Ok(value);
    }
    if let Some(value) = value.as_i64() {
        if value >= 0 {
            return Ok(value as u64);
        }
    }
    if let Some(value) = value.as_str().and_then(|raw| raw.trim().parse().ok()) {
        return Ok(value);
    }
    Err(format!("Notification-Feld {key} ist keine Discord-ID"))
}

fn build_request_embed_inner(
    request: &RequestData,
    assigned_coach_id: Option<u64>,
    reserved_until: Option<i64>,
    now_ts: i64,
    include_ai: bool,
    include_player: bool,
) -> Value {
    let mut fields = Vec::new();
    if include_player {
        fields.push(json!({
            "name": "Spieler",
            "value": format!("<@{}>", request.user_id),
            "inline": true,
        }));
    }
    fields.extend([
        json!({ "name": "Rang", "value": normalize_inline(&request.rank, "N/A", 256), "inline": true }),
        json!({ "name": "Hero", "value": normalize_inline(&request.hero, "Nicht angegeben", 256), "inline": true }),
        json!({ "name": "Games / Stunden", "value": normalize_inline(&request.games_played, "N/A", 256), "inline": true }),
        json!({ "name": "📅 Bevorzugter Slot", "value": normalize_inline(&request.scheduled_slot, "Nicht angegeben", 256), "inline": false }),
        json!({ "name": "📝 Probleme", "value": normalize_inline(&request.current_problems, "Keine Beschreibung", 1024), "inline": false }),
    ]);
    if include_ai {
        fields.push(
            json!({ "name": "🤖 AI Analyse", "value": format_ai_summary(&request.ai_summary), "inline": false }),
        );
    }
    if let (Some(coach), Some(until)) = (assigned_coach_id, reserved_until) {
        if now_ts < until {
            fields.push(json!({
                "name": "🎯 Reserviert für",
                "value": format!("<@{coach}> – claim bis <t:{until}:R>"),
                "inline": false,
            }));
        }
    }
    json!({
        "title": "🎮 Neue Coaching-Anfrage",
        "color": 0x3498DB,
        "author": { "name": request.username },
        "fields": fields,
    })
}

/// Anfrage-Embed (wie _build_request_embed).
pub fn build_request_embed(
    request: &RequestData,
    assigned_coach_id: Option<u64>,
    reserved_until: Option<i64>,
    now_ts: i64,
) -> Value {
    build_request_embed_inner(
        request,
        assigned_coach_id,
        reserved_until,
        now_ts,
        true,
        false,
    )
}

pub fn build_request_embed_no_ai(
    request: &RequestData,
    assigned_coach_id: Option<u64>,
    reserved_until: Option<i64>,
    now_ts: i64,
) -> Value {
    build_request_embed_inner(
        request,
        assigned_coach_id,
        reserved_until,
        now_ts,
        false,
        true,
    )
}

fn build_request_embed_for_existing_request(
    request: &RequestData,
    assigned_coach_id: Option<u64>,
    reserved_until: Option<i64>,
    now_ts: i64,
) -> Value {
    if request.ai_summary.trim().is_empty() {
        build_request_embed_no_ai(request, assigned_coach_id, reserved_until, now_ts)
    } else {
        build_request_embed(request, assigned_coach_id, reserved_until, now_ts)
    }
}

pub fn claim_components(request_id: i64, author_id: u64) -> Value {
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 3, "label": "Coaching übernehmen",
          "custom_id": format!("coach_claim_{request_id}") },
        { "type": 2, "style": 2, "label": "Freigeben",
          "custom_id": format!("coach_release_{request_id}_{author_id}") },
    ]}])
}

pub fn coachee_website_url(coachee_id: &str) -> String {
    let coachee_id = coachee_id.trim().trim_matches('/');
    format!("{COACHING_WEBSITE_URL}/coachees/{coachee_id}")
}

pub fn claim_components_with_website_link(
    request_id: i64,
    author_id: u64,
    coachee_id: &str,
) -> Value {
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 3, "label": "Coaching übernehmen",
          "custom_id": format!("coach_claim_{request_id}") },
        { "type": 2, "style": 2, "label": "Freigeben",
          "custom_id": format!("coach_release_{request_id}_{author_id}") },
        { "type": 2, "style": 5, "label": COACHING_WEBSITE_OPEN_BUTTON_LABEL,
          "url": coachee_website_url(coachee_id) },
    ]}])
}

pub fn cancel_components(session_id: &str, author_id: u64) -> Value {
    json!([{ "type": 1, "components": [{
        "type": 2, "style": 4, "label": "Abbrechen (User meldet sich nicht)",
        "custom_id": format!("coach_cancel_{session_id}_{author_id}"),
    }]}])
}

pub fn build_panel_embed() -> Value {
    json!({
        "title": "🎮  Deadlock Coaching",
        "description": format!(
            "Du willst besser werden? Unsere Coaches helfen dir, dein Spiel gezielt zu verbessern.\n\n\
             **⚠️ Wichtige Regeln:**\n\
             • Die Kommunikation findet **ausschließlich** im Coaching-Chat statt.\n\
             • Bitte sende **keine** Freundschaftsanfragen (FAs) oder DMs an die Coaches.\n\
             • Sei bereit zu antworten, wenn sich ein Coach meldet.\n\n\
             {COACHING_WEBSITE_CTA_TEXT}"
        ),
        "color": 0x3498DB,
        "fields": [
            {
                "name": "📋 Ablauf",
                "value": "1. Formular ausfüllen\n2. Du bekommst die Coaching-Rolle\n3. Ein Coach meldet sich bei dir",
                "inline": false,
            },
            {
                "name": "❓ Fragen nach dem Coaching?",
                "value": "Hau sie einfach in <#1426220702054355077> raus statt per DM an deinen Coach. Dann sehen alle die Antwort und andere mit dem gleichen Thema lesen direkt mit.",
                "inline": false,
            },
        ],
    })
}

pub fn panel_components() -> Value {
    json!([{ "type": 1, "components": [{
        "type": 2,
        "style": 5,
        "label": COACHING_WEBSITE_BUTTON_LABEL,
        "url": COACHING_WEBSITE_URL,
    }]}])
}

fn panel_body() -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("embeds".into(), json!([build_panel_embed()]));
    body.insert("components".into(), panel_components());
    body
}

fn website_cta_reply() -> BridgeReply {
    BridgeReply {
        content: Some(COACHING_WEBSITE_CTA_TEXT.to_string()),
        components: Some(panel_components()),
        ephemeral: true,
        ..BridgeReply::default()
    }
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait CoachingPort: Send + Sync {
    async fn post_panel(&self, channel_id: u64, body: Map<String, Value>) -> Result<u64, String>;
    async fn edit_panel(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
    ) -> Result<(), String>;
    /// Coach-Kandidaten: Mitglieder der Coach-Rolle (ohne Bots/Owner).
    async fn coach_member_ids(&self, guild_id: u64) -> Vec<u64>;
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> String;
    async fn member_is_admin(&self, guild_id: u64, user_id: u64) -> bool;
    async fn send_request_message(
        &self,
        channel_id: u64,
        content: &str,
        embed: Value,
        components: Value,
    ) -> Result<u64, String>;
    async fn edit_request_message(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
        embed: Value,
        components: Value,
    );
    async fn send_channel_text(&self, channel_id: u64, content: &str);
    async fn send_dm(&self, user_id: u64, content: &str) -> bool;
    async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str);
    async fn remove_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str);
    /// Voice-Channel des Mitglieds, falls es in einem VC unter `category_id`
    /// sitzt; sonst `None` (für die Coaching-Voice-Erkennung).
    async fn member_voice_channel_in_category(
        &self,
        guild_id: u64,
        user_id: u64,
        category_id: u64,
    ) -> Option<u64>;
    /// DM mit Embed senden (Survey-Aufforderung); `true` bei Erfolg.
    async fn send_dm_embed(&self, user_id: u64, embed: Value) -> bool;
}

pub struct CoachingRequests {
    pub pool: PgPool,
    pub port: Arc<dyn CoachingPort>,
    pub ai: Option<Arc<dyn TextGenerator>>,
    pub guild_id: u64,
    /// Website-Spiegelung der Anfrage-/Session-Zustände (Python
    /// `_mirror_to_website`). `None` = inaktiv (kein interner Token), genau
    /// wie wenn `website_client._token()` leer ist.
    pub website: Option<Arc<dyn crate::coaching::CoachingWebsiteSyncClient>>,
}

impl CoachingRequests {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn CoachingPort>,
        ai: Option<Arc<dyn TextGenerator>>,
        guild_id: u64,
        website: Option<Arc<dyn crate::coaching::CoachingWebsiteSyncClient>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            ai,
            guild_id,
            website,
        })
    }

    /// Postet/editiert das persistente Coaching-Panel im Panel-Channel.
    /// Idempotent über denselben KV-Vertrag wie Python:
    /// `kv_store(ns='coaching', k='panel_message_id')`.
    pub async fn ensure_panel(&self) {
        let body = panel_body();
        if let Some(message_id) = self.panel_message_id().await {
            match self
                .port
                .edit_panel(COACHING_PANEL_CHANNEL_ID, message_id, body.clone())
                .await
            {
                Ok(()) => return,
                Err(err) => tracing::info!(
                    %err,
                    message_id,
                    "Coaching-Panel konnte nicht editiert werden, poste neu"
                ),
            }
        }

        match self.port.post_panel(COACHING_PANEL_CHANNEL_ID, body).await {
            Ok(message_id) => {
                if let Err(err) = kv::set(
                    &self.pool,
                    PANEL_KV_NS,
                    PANEL_KV_KEY,
                    &message_id.to_string(),
                )
                .await
                {
                    tracing::warn!(%err, "Coaching-Panel-ID konnte nicht gespeichert werden");
                }
            }
            Err(err) => tracing::warn!(%err, "Coaching-Panel konnte nicht gepostet werden"),
        }
    }

    async fn panel_message_id(&self) -> Option<u64> {
        kv::get(&self.pool, PANEL_KV_NS, PANEL_KV_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u64>().ok())
    }

    /// Spiegelt den aktuellen Anfrage-/Session-Stand best-effort an die Website
    /// (Port von `_mirror_to_website`). Liest die `coaching_requests`-Zeile,
    /// füllt das EXAKTE Payload-Schema und feuert den POST in einem
    /// `tokio::spawn`, damit der Discord-Flow nie blockiert. Fehler nur
    /// `tracing`. Ohne Website-Client (kein Token) ein No-op.
    fn mirror_to_website(&self, opts: MirrorOpts) {
        let Some(client) = self.website.clone() else {
            return;
        };
        let pool = self.pool.clone();
        tokio::spawn(async move {
            let request_id = opts.request_id;
            let Ok(request_id_i32) = i64_to_i32(request_id, "request_id") else {
                tracing::warn!(
                    request_id,
                    "Coaching-Mirror: request_id passt nicht in INTEGER"
                );
                return;
            };
            // Volle Zeile lesen — wie `db.query_one("SELECT * ...")` in Python.
            let row = sqlx::query!(
                r#"
                SELECT bot_request_id AS "id!: i32",
                       website_request_id,
                       discord_user_id,
                       COALESCE(discord_username, '') AS "discord_username!",
                       rank,
                       subrank,
                       hero,
                       games_played,
                       hours_played,
                       availability,
                       current_problems,
                       COALESCE(ai_summary, '') AS "ai_summary!",
                       COALESCE(status, '') AS "status!",
                       assigned_coach_id,
                       reserved_until
                  FROM coaching.requests
                 WHERE bot_request_id = $1
                "#,
                request_id_i32,
            )
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten()
            .map(|r| MirrorRow {
                id: i64::from(r.id),
                website_request_id: r.website_request_id,
                discord_user_id: r.discord_user_id,
                discord_username: r.discord_username,
                rank: r.rank,
                subrank: r.subrank,
                hero: r.hero,
                games_played: r.games_played,
                hours_played: r.hours_played,
                availability: r.availability,
                current_problems: r.current_problems,
                ai_summary: r.ai_summary,
                status: r.status,
                assigned_coach_id: r.assigned_coach_id,
                reserved_until: r.reserved_until.map(unix_from_utc),
            });
            let Some(row) = row else {
                tracing::debug!(
                    request_id,
                    "Coaching-Mirror: Zeile nicht gefunden (ignoriert)"
                );
                return;
            };
            // assigned_coach_id ist TEXT in der DB — wie Python int(assigned) → numerisch.
            let assigned = row
                .assigned_coach_id
                .as_deref()
                .and_then(|s| s.parse::<i64>().ok());
            let mut payload = json!({
                "bot_request_id": row.id,
                "discord_user_id": row.discord_user_id,
                "discord_username": row.discord_username,
                "rank": row.rank,
                "subrank": row.subrank,
                "hero": row.hero,
                "games_played": row.games_played,
                "hours_played": row.hours_played,
                "availability": row.availability,
                "current_problems": row.current_problems,
                "ai_summary": row.ai_summary,
                "status": row.status,
                "assigned_coach_discord_id": assigned,
                "assigned_coach_username": opts.assigned_coach_username,
                "reserved_until": row.reserved_until,
            });
            if let Some(website_request_id) = row.website_request_id {
                if let Some(map) = payload.as_object_mut() {
                    map.insert("website_request_id".into(), json!(website_request_id));
                }
            }
            if let (Some(coach_id), Some(session_status)) =
                (opts.coach_discord_id, opts.session_status.as_ref())
            {
                if let Some(map) = payload.as_object_mut() {
                    map.insert("coach_discord_id".into(), json!(coach_id));
                    map.insert("coach_username".into(), json!(opts.coach_username));
                    map.insert("session_status".into(), json!(session_status));
                }
            }
            if let Some(session_id) = opts.bot_session_id {
                if let Some(map) = payload.as_object_mut() {
                    map.insert("bot_session_id".into(), json!(session_id));
                }
            }
            client.sync_coaching(&payload).await;
        });
    }

    async fn load_request(
        &self,
        request_id: i64,
    ) -> Option<(
        RequestData,
        String,
        Option<u64>,
        Option<i64>,
        Option<u64>,
        Option<i64>,
    )> {
        let request_id_i32 = i64_to_i32(request_id, "request_id").ok()?;
        let row = sqlx::query!(
            r#"
            SELECT bot_request_id AS "id!: i32",
                   discord_user_id,
                   COALESCE(discord_username, '') AS "discord_username!",
                   rank,
                   COALESCE(subrank, '') AS "subrank!",
                   COALESCE(hero, '') AS "hero!",
                   COALESCE(games_played, '') AS "games_played!",
                   COALESCE(hours_played, '') AS "hours_played!",
                   COALESCE(NULLIF(scheduled_slot, ''), availability, '') AS "scheduled_slot!",
                   COALESCE(current_problems, '') AS "current_problems!",
                   COALESCE(ai_summary, '') AS "ai_summary!",
                   COALESCE(status, '') AS "status!",
                   assigned_coach_id,
                   reserved_until,
                   message_id,
                   role_expires_at
              FROM coaching.requests
             WHERE bot_request_id = $1
            "#,
            request_id_i32,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()??;
        let user_id = pg_i64_to_u64(row.discord_user_id, "discord_user_id").ok()?;
        let message_id = row
            .message_id
            .and_then(|value| pg_i64_to_u64(value, "message_id").ok());
        let rank = combine_rank(&row.rank, &row.subrank);
        let games_played = combine_games_hours(&row.games_played, &row.hours_played);
        Some((
            RequestData {
                id: i64::from(row.id),
                user_id,
                username: row.discord_username,
                rank,
                hero: row.hero,
                games_played,
                scheduled_slot: row.scheduled_slot,
                current_problems: row.current_problems,
                ai_summary: row.ai_summary,
            },
            row.status,
            row.assigned_coach_id
                .and_then(|raw| raw.parse::<u64>().ok()),
            row.reserved_until.map(unix_from_utc),
            message_id,
            row.role_expires_at.map(unix_from_utc),
        ))
    }

    /// AI-Analyse (Prompt wie _analyze_with_ai); "" = INVALID_REQUEST.
    async fn analyze(&self, request: &RequestData) -> String {
        let Some(ai) = &self.ai else {
            return format!("**Analyse:**\n{}", request.current_problems);
        };
        let or_na = |v: &str| {
            if v.is_empty() {
                "N/A".to_string()
            } else {
                v.to_string()
            }
        };
        let prompt = format!(
            "Analysiere diese Deadlock Coaching-Anfrage. Die Felder sind Rohtext vom User,\n\
interpretiere Rang/Subrank und Game/Stunden-Angaben selbst.\n\n\
- Rang: {}\n- Hero: {}\n- Games / Stunden: {}\n- Verfügbarkeit: {}\n- Probleme: {}\n\n\
Erstelle eine präzise, hilfreiche Zusammenfassung für den Coach.",
            or_na(&request.rank),
            or_na(&request.hero),
            or_na(&request.games_played),
            or_na(&request.scheduled_slot),
            or_na(&request.current_problems),
        );
        let answer = ai
            .generate_text(GenerateRequest {
                prompt,
                system_prompt: Some(COACHING_ANALYSIS_SYSTEM.to_string()),
                model: Some("MiniMax-M3".to_string()),
                max_output_tokens: Some(500),
                temperature: 0.7,
            })
            .await
            .unwrap_or_default();
        if answer.trim() == "INVALID_REQUEST" {
            return String::new();
        }
        if answer.trim().is_empty() {
            format!("**Analyse:**\n{}", request.current_problems)
        } else {
            answer
        }
    }

    async fn auto_assign_stats(&self) -> Vec<(u64, i64)> {
        let mut coach_ids = self.port.coach_member_ids(self.guild_id).await;
        coach_ids.retain(|id| !AUTO_ASSIGN_OPTOUT_IDS.contains(id));
        let mut result = Vec::new();
        for id in coach_ids {
            let coach_id = id.to_string();
            let last_assigned_at = sqlx::query_scalar!(
                r#"
                SELECT last_assigned_at
                  FROM coaching.coach_rotation
                 WHERE coach_id = $1
                "#,
                coach_id,
            )
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
            .map(unix_from_utc)
            .unwrap_or(0);
            result.push((id, last_assigned_at));
        }
        result
    }

    async fn post_request_to_channel(
        &self,
        request: &mut RequestData,
        ai_summary: String,
        include_ai: bool,
        components: Value,
    ) -> Result<(), String> {
        request.ai_summary = ai_summary.clone();
        let now_ts = chrono::Utc::now().timestamp();
        // Kandidaten + Rotations-Daten
        let stats = self.auto_assign_stats().await;
        let assigned = pick_fair_coach(&stats);
        let reserved_until = assigned.map(|_| now_ts + CLAIM_RESERVATION_HOURS * 3600);
        let embed = if include_ai {
            build_request_embed(request, assigned, reserved_until, now_ts)
        } else {
            build_request_embed_no_ai(request, assigned, reserved_until, now_ts)
        };
        let content = match assigned {
            Some(coach) => format!(
                "📥 Anfrage von <@{}> – 🎯 reserviert für <@{coach}> ({CLAIM_RESERVATION_HOURS}h)",
                request.user_id
            ),
            None => format!(
                "📥 Anfrage von <@{}> – 🟢 offen für alle Coaches",
                request.user_id
            ),
        };
        let message_id = self
            .port
            .send_request_message(REQUEST_CHANNEL_ID, &content, embed, components)
            .await?;
        let request_id = request.id;
        let assigned_coach_id = assigned.map(|coach| coach.to_string());
        let request_id_i32 = i64_to_i32(request_id, "request_id").map_err(|err| err.to_string())?;
        let message_id_i64 = u64_to_i64(message_id, "message_id").map_err(|err| err.to_string())?;
        let channel_id_i64 =
            u64_to_i64(REQUEST_CHANNEL_ID, "REQUEST_CHANNEL_ID").map_err(|err| err.to_string())?;
        let reserved_until_dt = reserved_until
            .map(utc_from_unix)
            .transpose()
            .map_err(|err| err.to_string())?;
        let updated_at = chrono::Utc::now();
        sqlx::query!(
            r#"
            UPDATE coaching.requests
               SET message_id = $1,
                   channel_id = $2,
                   ai_summary = $3,
                   status = 'analyzed',
                   assigned_coach_id = $4,
                   reserved_until = $5,
                   updated_at = $6
             WHERE bot_request_id = $7
            "#,
            message_id_i64,
            channel_id_i64,
            ai_summary,
            assigned_coach_id,
            reserved_until_dt,
            updated_at,
            request_id_i32,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        if let Some(coach) = assigned {
            sqlx::query!(
                r#"
                INSERT INTO coaching.coach_rotation(coach_id, last_assigned_at)
                VALUES ($1, $2)
                ON CONFLICT(coach_id) DO UPDATE SET
                  last_assigned_at = excluded.last_assigned_at
                "#,
                coach.to_string(),
                updated_at,
            )
            .execute(&self.pool)
            .await
            .map_err(|err| err.to_string())?;
        }
        // Website-Mirror (Python `_post_request_to_channel`:852) — mit
        // dem Display-Namen des reservierten Coaches, falls einer
        // zugewiesen wurde.
        let assigned_coach_username = match assigned {
            Some(coach) => Some(self.port.member_display_name(self.guild_id, coach).await),
            None => None,
        };
        self.mirror_to_website(MirrorOpts {
            request_id: request.id,
            assigned_coach_username,
            ..MirrorOpts::default()
        });
        Ok(())
    }

    /// Anfrage posten (wie _post_request_to_channel): faire Rotation +
    /// 24-h-Reservierung.
    async fn post_request(&self, request: &mut RequestData, ai_summary: String) {
        let components = claim_components(request.id, request.user_id);
        if let Err(err) = self
            .post_request_to_channel(request, ai_summary, true, components)
            .await
        {
            tracing::warn!(%err, "Coaching-Post fehlgeschlagen");
        }
    }

    async fn upsert_request_created_notification(
        &self,
        data: &RequestCreatedNotification,
    ) -> Result<RequestCreatedUpsert, String> {
        let website_request_id = data.website_request_id.clone();
        let coachee_id = data.coachee_id.clone();
        let discord_user_id = data.discord_user_id;
        let discord_username = data.discord_username.clone();
        let rank = data.rank.clone();
        let subrank = data.subrank.clone();
        let hero = data.hero.clone();
        let games_played = data.games_played.clone();
        let hours_played = data.hours_played.clone();
        let availability = data.availability.clone();
        let scheduled_slot = data.availability.clone();
        let current_problems = data.current_problems.clone();
        let preferred_coach_id = data.preferred_coach_id.clone();
        let discord_user_id =
            u64_to_i64(discord_user_id, "discord_user_id").map_err(|err| err.to_string())?;
        let now = chrono::Utc::now();
        let mut tx = self.pool.begin().await.map_err(|err| err.to_string())?;
        advisory_lock(&mut tx, COACHING_REQUESTS_ID_LOCK)
            .await
            .map_err(|err| err.to_string())?;
        let existing = sqlx::query!(
            r#"
            SELECT bot_request_id AS "bot_request_id?: i32", message_id
              FROM coaching.requests
             WHERE website_request_id = $1
            "#,
            website_request_id,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
        if let Some(existing) = existing {
            let local_request_id = if let Some(local_request_id) = existing.bot_request_id {
                local_request_id
            } else {
                sqlx::query_scalar!(
                    r#"
                    SELECT COALESCE(MAX(bot_request_id), 0) + 1 AS "next_id!: i32"
                      FROM coaching.requests
                    "#
                )
                .fetch_one(&mut *tx)
                .await
                .map_err(|err| err.to_string())?
            };
            sqlx::query!(
                r#"
                UPDATE coaching.requests
                   SET bot_request_id = $1,
                       coachee_id = $2,
                       discord_user_id = $3,
                       discord_username = $4,
                       rank = $5,
                       subrank = $6,
                       hero = $7,
                       games_played = $8,
                       hours_played = $9,
                       availability = $10,
                       scheduled_slot = $11,
                       current_problems = $12,
                       preferred_coach_id = $13,
                       updated_at = $14
                 WHERE website_request_id = $15
                "#,
                local_request_id,
                coachee_id,
                discord_user_id,
                discord_username,
                rank,
                subrank,
                hero,
                games_played,
                hours_played,
                availability,
                scheduled_slot,
                current_problems,
                preferred_coach_id,
                now,
                website_request_id,
            )
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
            tx.commit().await.map_err(|err| err.to_string())?;
            return Ok(RequestCreatedUpsert {
                local_request_id: i64::from(local_request_id),
                already_posted: existing.message_id.is_some(),
            });
        }

        let next_id = sqlx::query_scalar!(
            r#"
            SELECT COALESCE(MAX(bot_request_id), 0) + 1 AS "next_id!: i32"
              FROM coaching.requests
            "#
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
        let request_uid = format!("website:{website_request_id}");
        sqlx::query!(
            r#"
            INSERT INTO coaching.requests(
                request_uid, bot_request_id, website_request_id, coachee_id,
                discord_user_id, discord_username, rank, subrank, hero, games_played,
                hours_played, availability, scheduled_slot, current_problems,
                preferred_coach_id, ai_summary, status, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                    $11, $12, $13, $14, $15, '', 'pending', $16, $16)
            "#,
            request_uid,
            next_id,
            website_request_id,
            coachee_id,
            discord_user_id,
            discord_username,
            rank,
            subrank,
            hero,
            games_played,
            hours_played,
            availability,
            scheduled_slot,
            current_problems,
            preferred_coach_id,
            now,
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
        tx.commit().await.map_err(|err| err.to_string())?;
        Ok(RequestCreatedUpsert {
            local_request_id: i64::from(next_id),
            already_posted: false,
        })
    }

    pub async fn post_request_created_notification(&self, item: &Value) -> Result<(), String> {
        let data = RequestCreatedNotification::from_item(item)?;
        if let Some(preferred_coach_id) = data.preferred_coach_id.as_deref() {
            tracing::debug!(
                request_id = %data.website_request_id,
                preferred_coach_id,
                "request_created preferred_coach_id gelesen"
            );
        }
        let upsert = self.upsert_request_created_notification(&data).await?;
        if upsert.already_posted {
            return Ok(());
        }

        let mut request = data.request_data(upsert.local_request_id);
        let components =
            claim_components_with_website_link(request.id, request.user_id, &data.coachee_id);
        self.post_request_to_channel(&mut request, String::new(), false, components)
            .await
    }

    /// Reservierung aufheben + Nachricht aktualisieren (wie _open_request_to_all).
    pub async fn open_request_to_all(&self, request_id: i64, reason: &str) {
        let now = chrono::Utc::now();
        let Ok(request_id_i32) = i64_to_i32(request_id, "request_id") else {
            return;
        };
        let _ = sqlx::query!(
            r#"
            UPDATE coaching.requests
               SET assigned_coach_id = NULL,
                   reserved_until = NULL,
                   updated_at = $1
             WHERE bot_request_id = $2 AND status = 'analyzed'
            "#,
            now,
            request_id_i32,
        )
        .execute(&self.pool)
        .await;
        let Some((request, status, _, _, message_id, _)) = self.load_request(request_id).await
        else {
            return;
        };
        let (Some(message_id), true) = (message_id, status == "analyzed") else {
            return;
        };
        let prefix = if reason == "expired" {
            "⏰ Reservierung abgelaufen – "
        } else {
            "🟢 Freigegeben – "
        };
        let content = format!(
            "📥 Anfrage von <@{}> – {prefix}jetzt für alle Coaches offen",
            request.user_id
        );
        let embed = build_request_embed_for_existing_request(&request, None, None, now.timestamp());
        self.port
            .edit_request_message(
                REQUEST_CHANNEL_ID,
                message_id,
                &content,
                embed,
                claim_components(request.id, request.user_id),
            )
            .await;
        // Website-Mirror (Python `_open_request_to_all`:742) — nur die Anfrage,
        // ohne Coach-/Session-Felder; status ist inzwischen wieder 'analyzed'
        // mit geleerter Reservierung.
        self.mirror_to_website(MirrorOpts {
            request_id,
            ..MirrorOpts::default()
        });
    }

    /// Analyse-Loop (wie _analyze_pending_requests, Claim via rowcount).
    pub async fn analyze_pending(&self) {
        let rows = sqlx::query_scalar!(
            r#"
            SELECT bot_request_id AS "bot_request_id!: i32"
              FROM coaching.requests
             WHERE status = 'pending'
               AND current_problems IS NOT NULL
               AND current_problems != ''
               AND (ai_summary IS NULL OR ai_summary = '')
             ORDER BY created_at ASC
             LIMIT 5
            "#
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        for request_id in rows {
            let request_id_i64 = i64::from(request_id);
            let claimed = sqlx::query!(
                r#"
                UPDATE coaching.requests
                   SET status = 'analyzing',
                       updated_at = $1
                 WHERE bot_request_id = $2 AND status = 'pending'
                "#,
                chrono::Utc::now(),
                request_id,
            )
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
            .unwrap_or(0);
            if claimed == 0 {
                continue;
            }
            let Some((mut request, ..)) = self.load_request(request_id_i64).await else {
                continue;
            };
            let summary = self.analyze(&request).await;
            if summary.is_empty() {
                let _ = sqlx::query!(
                    r#"
                    UPDATE coaching.requests
                       SET status = 'invalid',
                           updated_at = $1
                     WHERE bot_request_id = $2
                    "#,
                    chrono::Utc::now(),
                    request_id,
                )
                .execute(&self.pool)
                .await;
                continue;
            }
            self.post_request(&mut request, summary).await;
        }
    }

    /// Abgelaufene Reservierungen öffnen (60-s-Loop).
    pub async fn expire_reservations(&self) {
        let expired = sqlx::query_scalar!(
            r#"
            SELECT bot_request_id AS "bot_request_id!: i32"
              FROM coaching.requests
             WHERE status = 'analyzed'
               AND assigned_coach_id IS NOT NULL
               AND reserved_until IS NOT NULL
               AND reserved_until < $1
            "#,
            chrono::Utc::now(),
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        for request_id in expired {
            self.open_request_to_all(i64::from(request_id), "expired")
                .await;
        }
    }

    /// Abgelaufene Coaching-Rollen entfernen (Port `coaching_role_manager`):
    /// aktive Rolle nach 48 h (`coaching_requests.role_expires_at`), Reward-
    /// Rolle nach 5 Tagen (`coaching_sessions.reward_role_expires_at`).
    pub async fn expire_roles(&self) {
        let now = chrono::Utc::now();
        let guild_id = self.guild_id;

        let active = sqlx::query!(
            r#"
            SELECT bot_request_id AS "bot_request_id!: i32", discord_user_id
              FROM coaching.requests
             WHERE role_removed_at IS NULL
               AND role_expires_at IS NOT NULL
               AND role_expires_at < $1
            "#,
            now,
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        for row in active {
            let Some(user_id) = pg_i64_to_u64(row.discord_user_id, "discord_user_id").ok() else {
                continue;
            };
            self.remove_role_if_present(
                guild_id,
                user_id,
                COACHING_ACTIVE_ROLE_ID,
                "Coaching-Rolle abgelaufen (48h)",
            )
            .await;
            let _ = sqlx::query!(
                r#"
                UPDATE coaching.requests
                   SET role_removed_at = $1,
                       updated_at = $1
                 WHERE bot_request_id = $2
                "#,
                now,
                row.bot_request_id,
            )
            .execute(&self.pool)
            .await;
            let thread = sqlx::query_scalar!(
                r#"
                SELECT discord_thread_id
                  FROM coaching.sessions
                 WHERE bot_request_id = $1
                   AND status IN ('active', 'waiting_survey')
                 ORDER BY created_at DESC
                 LIMIT 1
                "#,
                row.bot_request_id,
            )
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
            .flatten()
            .and_then(|id| pg_i64_to_u64(id, "discord_thread_id").ok());
            if let Some(tid) = thread {
                self.port
                    .send_channel_text(
                        tid,
                        "⏰ Die 48h Coaching-Phase ist abgelaufen. Falls ihr noch keine \
                         Voice-Session hattet, müsst ihr eine neue Anfrage stellen.",
                    )
                    .await;
            }
        }

        let reward = sqlx::query!(
            r#"
            SELECT id, discord_user_id
              FROM coaching.sessions
             WHERE reward_role_removed_at IS NULL
               AND reward_role_expires_at IS NOT NULL
               AND reward_role_expires_at < $1
            "#,
            now,
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        for row in reward {
            let Some(user_id_i64) = row.discord_user_id else {
                continue;
            };
            let Some(user_id) = pg_i64_to_u64(user_id_i64, "discord_user_id").ok() else {
                continue;
            };
            self.remove_role_if_present(
                guild_id,
                user_id,
                COACHING_REWARD_ROLE_ID,
                "Coaching Reward-Rolle abgelaufen (5 Tage)",
            )
            .await;
            let _ = sqlx::query!(
                r#"
                UPDATE coaching.sessions
                   SET reward_role_removed_at = $1
                 WHERE id = $2
                "#,
                now,
                row.id,
            )
            .execute(&self.pool)
            .await;
        }
    }

    /// Survey-Poll (Python `_scan_active_sessions`, 60-s-Loop): alle aktiven
    /// Sessions ohne gesendete Umfrage prüfen.
    pub async fn scan_survey_sessions(&self) {
        for session in self.load_survey_sessions(None).await {
            self.process_survey_session(session, SurveyTrigger::Poll)
                .await;
        }
    }

    /// Voice-getriggerte Prüfung (Python `on_voice_state_update`): nur die
    /// Sessions, an denen dieses Mitglied als User oder Coach beteiligt ist.
    pub async fn survey_sessions_for_member(&self, member_id: u64, voice_transition_can_end: bool) {
        let trigger = if voice_transition_can_end {
            SurveyTrigger::VoiceTransition
        } else {
            SurveyTrigger::Poll
        };
        for session in self.load_survey_sessions(Some(member_id)).await {
            self.process_survey_session(session, trigger).await;
        }
    }

    async fn load_survey_sessions(&self, member: Option<u64>) -> Vec<SurveySession> {
        let member_i64 = member.and_then(|value| u64_to_i64(value, "member_id").ok());
        let member_text = member.map(|value| value.to_string());
        let rows = sqlx::query!(
            r#"
            SELECT id, coach_id, discord_user_id, voice_started_at, bot_request_id
              FROM coaching.sessions
             WHERE status = 'active'
               AND survey_sent_at IS NULL
               AND (
                    $1::bigint IS NULL
                    OR discord_user_id = $1
                    OR coach_id = $2
               )
            "#,
            member_i64,
            member_text,
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        rows.into_iter()
            .filter_map(|row| {
                Some(SurveySession {
                    id: row.id,
                    coach_id: row.coach_id.and_then(|s| s.parse::<u64>().ok()),
                    user_id: pg_i64_to_u64(row.discord_user_id?, "discord_user_id").ok()?,
                    voice_started_at: row.voice_started_at.map(unix_from_utc),
                    request_id: i64::from(row.bot_request_id?),
                })
            })
            .collect()
    }

    /// Kern-Logik (Python `_process_session_voice_state`): sitzen User und Coach
    /// noch im selben Coaching-VC, läuft die Session (Tracking-Update). Sind sie
    /// es nicht mehr und war zuvor eine Voice-Session aktiv, gilt sie als beendet
    /// → Active-Rolle weg, Reward-Rolle (5 Tage) und Survey-DM.
    async fn process_survey_session(&self, session: SurveySession, trigger: SurveyTrigger) {
        let guild = self.guild_id;
        let Some(coach_id) = session.coach_id else {
            tracing::warn!(
                session_id = %session.id,
                "Coaching-Survey: aktive Session ohne gueltige coach_id uebersprungen"
            );
            return;
        };
        let user_vc = self
            .port
            .member_voice_channel_in_category(guild, session.user_id, COACHING_VOICE_CATEGORY_ID)
            .await;
        let coach_vc = self
            .port
            .member_voice_channel_in_category(guild, coach_id, COACHING_VOICE_CATEGORY_ID)
            .await;
        let now = chrono::Utc::now();

        // Beide noch im selben Coaching-VC → Voice-Tracking aktualisieren.
        if user_vc.is_some() && user_vc == coach_vc {
            let Ok(vc) = u64_to_i64(user_vc.unwrap_or_default(), "voice_channel_id") else {
                return;
            };
            let _ = sqlx::query!(
                r#"
                UPDATE coaching.sessions
                   SET voice_channel_id = $1,
                       voice_started_at = COALESCE(voice_started_at, $2),
                       voice_last_seen_at = $2
                 WHERE id = $3
                "#,
                vc,
                now,
                session.id,
            )
            .execute(&self.pool)
            .await;
            return;
        }

        // Noch nie gemeinsam im Voice → es gibt keine Session zu beenden.
        if session.voice_started_at.is_none() {
            return;
        }

        let has_end_evidence = match trigger {
            // Der Poll darf nicht "beide unbekannt" als Ende interpretieren:
            // direkt nach Start kann der Gateway-Cache leer sein. Sicher ist
            // nur: mindestens ein Beteiligter wurde in einem anderen Coaching-
            // Zustand gesehen, aber beide sind nicht mehr im selben VC.
            SurveyTrigger::Poll => user_vc.is_some() || coach_vc.is_some(),
            // Leave/Move eines Beteiligten ist ein echtes Voice-Endsignal, auch
            // wenn der Cache danach fuer beide Mitglieder leer ist.
            SurveyTrigger::VoiceTransition => true,
        };
        if !has_end_evidence {
            tracing::debug!(
                session_id = %session.id,
                user_id = session.user_id,
                coach_id,
                "Coaching-Survey: Poll ohne eindeutigen Voice-Cache, Session bleibt aktiv"
            );
            return;
        }

        // Voice-Session beendet → atomar abschließen. Der WHERE-Filter macht den
        // Claim wettlaufsicher: Poll und Voice-Listener können dieselbe Session
        // gleichzeitig sehen, aber nur einer trifft `status='active'`.
        let reward_expiry = now + chrono::Duration::seconds(REWARD_ROLE_DURATION_SECS);
        let claimed = sqlx::query!(
            r#"
            UPDATE coaching.sessions
               SET status = 'completed',
                   completed_at = $1,
                   survey_sent_at = $1,
                   reward_role_expires_at = $2,
                   voice_last_seen_at = $1
             WHERE id = $3 AND status = 'active' AND survey_sent_at IS NULL
            "#,
            now,
            reward_expiry,
            session.id,
        )
        .execute(&self.pool)
        .await
        .map(|result| result.rows_affected())
        .unwrap_or(0);
        if claimed == 0 {
            return;
        }

        let coach_name = self.port.member_display_name(guild, coach_id).await;

        self.remove_role_if_present(
            guild,
            session.user_id,
            COACHING_ACTIVE_ROLE_ID,
            "Coaching Session beendet",
        )
        .await;
        self.add_role_if_missing(
            guild,
            session.user_id,
            COACHING_REWARD_ROLE_ID,
            "Coaching abgeschlossen - Feedback-Berechtigung",
        )
        .await;
        if !self
            .port
            .send_dm_embed(session.user_id, survey_embed(&coach_name, guild))
            .await
        {
            tracing::warn!(
                user_id = session.user_id,
                "Coaching-Survey-DM konnte nicht zugestellt werden"
            );
        }

        let rid = session.request_id;
        if let Ok(rid_i32) = i64_to_i32(rid, "request_id") {
            let _ = sqlx::query!(
                r#"
                UPDATE coaching.requests
                   SET status = 'completed',
                       role_removed_at = $1,
                       updated_at = $1
                 WHERE bot_request_id = $2
                "#,
                now,
                rid_i32,
            )
            .execute(&self.pool)
            .await;
        }
        // Website-Mirror (Python `coaching_survey.py`:311): Session als
        // 'completed' spiegeln, inkl. bot_session_id.
        self.mirror_to_website(MirrorOpts {
            request_id: rid,
            coach_discord_id: Some(coach_id),
            coach_username: Some(coach_name),
            session_status: Some("completed".to_string()),
            bot_session_id: Some(session.id.clone()),
            ..MirrorOpts::default()
        });
    }

    async fn member_has_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> bool {
        self.port
            .member_role_ids(guild_id, user_id)
            .await
            .contains(&role_id)
    }

    async fn add_role_if_missing(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str) {
        if !self.member_has_role(guild_id, user_id, role_id).await {
            self.port.add_role(guild_id, user_id, role_id, reason).await;
        }
    }

    async fn remove_role_if_present(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) {
        if self.member_has_role(guild_id, user_id, role_id).await {
            self.port
                .remove_role(guild_id, user_id, role_id, reason)
                .await;
        }
    }
}

#[async_trait::async_trait]
impl crate::coaching::RequestNotificationSink for CoachingRequests {
    async fn post_request_created_notification(&self, item: &Value) -> Result<(), String> {
        CoachingRequests::post_request_created_notification(self, item).await
    }
}

/// Eine aktive Coaching-Session, deren Feedback-Umfrage noch aussteht.
struct SurveySession {
    id: String,
    coach_id: Option<u64>,
    user_id: u64,
    voice_started_at: Option<i64>,
    request_id: i64,
}

#[derive(Clone, Copy)]
enum SurveyTrigger {
    Poll,
    VoiceTransition,
}

/// Survey-Embed wie Python `send_survey_dm` (grün, Link in den Feedback-Kanal).
fn survey_embed(coach_name: &str, guild_id: u64) -> Value {
    let url = format!("https://discord.com/channels/{guild_id}/{COACHING_FEEDBACK_CHANNEL_ID}");
    json!({
        "title": "🎮 Coaching abgeschlossen!",
        "description": format!(
            "Deine Coaching-Session mit **{coach_name}** ist beendet. \
             Wir hoffen, es hat dir geholfen!"
        ),
        // discord.Color.green()
        "color": 0x2ECC71,
        "fields": [{
            "name": "⭐ Gib uns Feedback",
            "value": format!(
                "Du hast nun für **5 Tage** Zugriff auf unseren Feedback-Kanal. \
                 Bitte teile deine Erfahrungen dort mit uns:\n\n\
                 👉 [**HIER FEEDBACK ABGEBEN**]({url})\n\n\
                 Dein Feedback hilft uns die Qualität der Coaches sicherzustellen!"
            ),
            "inline": false,
        }],
    })
}

// ── Interaction-Handler ────────────────────────────────────────────────────

struct CoachingHandler {
    coaching: Arc<CoachingRequests>,
}

#[async_trait::async_trait]
impl InteractionHandler for CoachingHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let c = &self.coaching;
        let now_ts = chrono::Utc::now().timestamp();

        // /coaching-status — Status der letzten Anfrage (reiner Read).
        if interaction.command == "coaching-status" {
            let status = match u64_to_i64(interaction.user_id, "interaction.user_id") {
                Ok(user_id) => sqlx::query_scalar!(
                    r#"
                    SELECT status
                      FROM coaching.requests
                     WHERE discord_user_id = $1
                     ORDER BY created_at DESC
                     LIMIT 1
                    "#,
                    user_id,
                )
                .fetch_optional(&c.pool)
                .await
                .ok()
                .flatten()
                .flatten(),
                Err(_) => None,
            };
            let msg = match status.as_deref() {
                None => "Du hast keine Coaching-Anfrage gestellt.".to_string(),
                Some("pending") => {
                    "⏳ Deine Anfrage wird gerade analysiert. Bitte warte.".to_string()
                }
                Some("analyzed") => {
                    "✅ Deine Anfrage wurde analysiert und wartet auf einen Coach.".to_string()
                }
                Some("matched") => {
                    "🎉 Ein Coach hat sich für dich gemeldet. Check deine DMs.".to_string()
                }
                Some("active") => "🎮 Deine Coaching-Session läuft gerade.".to_string(),
                Some("completed") => "✅ Deine letzte Session ist abgeschlossen.".to_string(),
                Some("cancelled") => "❌ Deine Anfrage wurde abgebrochen.".to_string(),
                Some(other) => format!("Status: {other}"),
            };
            return BridgeReply::ephemeral_text(msg);
        }

        // Website-driven intake: Legacy-Button und Slash-Command verweisen nur
        // noch auf die Website; der Discord-Modal-/Analyse-Flow bleibt aus.
        if interaction.custom_id == "coaching_panel_start"
            || interaction.command == "coaching-anfrage"
        {
            return website_cta_reply();
        }

        if interaction.custom_id == "coaching_request_modal" {
            return website_cta_reply();
        }

        if interaction.custom_id == "__disabled_coaching_request_modal" {
            let get = |key: &str| {
                interaction
                    .options
                    .get(key)
                    .and_then(|v| v.as_str())
                    .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| "Nicht angegeben".to_string())
            };
            let (user_id, username) = (
                interaction.user_id,
                c.port
                    .member_display_name(interaction.guild_id, interaction.user_id)
                    .await,
            );
            let (rank, hero, availability, games_hours) = (
                get("rank"),
                get("hero"),
                get("availability"),
                get("games_hours"),
            );
            let problems = interaction
                .options
                .get("problems")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if let Ok(user_id_i64) = u64_to_i64(user_id, "user_id") {
                if let Ok(mut tx) = c.pool.begin().await {
                    if advisory_lock(&mut tx, COACHING_REQUESTS_ID_LOCK)
                        .await
                        .is_ok()
                    {
                        if let Ok(next_id) = sqlx::query_scalar!(
                            r#"
                            SELECT COALESCE(MAX(bot_request_id), 0) + 1 AS "next_id!: i32"
                              FROM coaching.requests
                            "#
                        )
                        .fetch_one(&mut *tx)
                        .await
                        {
                            let request_uid = format!("bot:{next_id}");
                            let _ = sqlx::query!(
                                r#"
                                INSERT INTO coaching.requests(
                                    request_uid, bot_request_id, discord_user_id,
                                    discord_username, rank, subrank, hero, availability,
                                    scheduled_slot, games_played, hours_played,
                                    current_problems, status, created_at, updated_at
                                )
                                VALUES ($1, $2, $3, $4, $5, '', $6, $7, $7, $8, '',
                                        $9, 'pending', $10, $10)
                                "#,
                                request_uid,
                                next_id,
                                user_id_i64,
                                username,
                                rank,
                                hero,
                                availability,
                                games_hours,
                                problems,
                                chrono::Utc::now(),
                            )
                            .execute(&mut *tx)
                            .await;
                            let _ = tx.commit().await;
                        }
                    }
                }
            }
            return BridgeReply::ephemeral_text(
                "✅ Deine Coaching-Anfrage wurde gespeichert. Die AI analysiert sie jetzt und postet sie automatisch im Coaching-Channel.",
            );
        }

        // coach_claim_{id} / coach_release_{id}_{author} / coach_cancel_{session}_{author}
        if let Some(rest) = interaction.custom_id.strip_prefix("coach_claim_") {
            let request_id: i64 = rest.parse().unwrap_or(0);
            let roles = c
                .port
                .member_role_ids(interaction.guild_id, interaction.user_id)
                .await;
            if !roles.contains(&COACH_ROLE_ID) {
                return BridgeReply::ephemeral_text(
                    "❌ Nur Coaches können sich für Sessions melden!",
                );
            }
            let Some((request, status, assigned, reserved_until, message_id, role_expires)) =
                c.load_request(request_id).await
            else {
                return BridgeReply::ephemeral_text("❌ Request nicht gefunden.");
            };
            if status == "matched" {
                return BridgeReply::ephemeral_text(
                    "❌ Diese Anfrage wurde bereits von einem Coach geclaimt.",
                );
            }
            let is_owner = interaction.user_id == OWNER_EXCLUDE_ID
                || c.port
                    .member_is_admin(interaction.guild_id, interaction.user_id)
                    .await;
            if let (Some(coach), Some(until)) = (assigned, reserved_until) {
                if now_ts < until && coach != interaction.user_id && !is_owner {
                    return BridgeReply::ephemeral_text(format!(
                        "⏳ Dieses Coaching ist noch für <@{coach}> reserviert (bis <t:{until}:R>). Danach kannst du es übernehmen."
                    ));
                }
            }
            // Session anlegen + Rolle + Benachrichtigungen
            let session_id = new_session_id();
            let expires_at = role_expires.unwrap_or(now_ts + ROLE_EXPIRY_HOURS * 3600);
            let coach_name = c
                .port
                .member_display_name(interaction.guild_id, interaction.user_id)
                .await;
            let request_id_i32 = match i64_to_i32(request_id, "request_id") {
                Ok(value) => value,
                Err(_) => return BridgeReply::ephemeral_text("❌ Request-ID ungültig."),
            };
            let uid = match u64_to_i64(request.user_id, "request.user_id") {
                Ok(value) => value,
                Err(_) => return BridgeReply::ephemeral_text("❌ User-ID ungültig."),
            };
            let channel_id = match u64_to_i64(interaction.channel_id, "interaction.channel_id") {
                Ok(value) => value,
                Err(_) => return BridgeReply::ephemeral_text("❌ Channel-ID ungültig."),
            };
            let assigned_at = chrono::Utc::now();
            let expires_at_dt = match utc_from_unix(expires_at) {
                Ok(value) => value,
                Err(_) => return BridgeReply::ephemeral_text("❌ Ablaufzeit ungültig."),
            };
            let _ = sqlx::query!(
                r#"
                INSERT INTO coaching.sessions(
                    id, bot_request_id, coach_id, discord_user_id, discord_username,
                    discord_channel_id, status, role_assigned_at, role_expires_at, created_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, 'active', $7, $8, $7)
                "#,
                &session_id,
                request_id_i32,
                interaction.user_id.to_string(),
                uid,
                request.username.clone(),
                channel_id,
                assigned_at,
                expires_at_dt,
            )
            .execute(&c.pool)
            .await;
            let _ = sqlx::query!(
                r#"
                UPDATE coaching.requests
                   SET status = 'matched',
                       updated_at = $1
                 WHERE bot_request_id = $2
                "#,
                assigned_at,
                request_id_i32,
            )
            .execute(&c.pool)
            .await;
            c.add_role_if_missing(
                interaction.guild_id,
                request.user_id,
                COACHING_ACTIVE_ROLE_ID,
                "Coaching Session gestartet",
            )
            .await;
            let dm_ok = c
                .port
                .send_dm(
                    request.user_id,
                    &format!(
                        "🎉 Ein Coach hat sich für deine Anfrage gemeldet!\n\n**Coach:** {coach_name}\n\nSchau in den Coaching-Channel um euch abzustimmen und das Coaching innerhalb der nächsten **7 Tage** durchzuführen."
                    ),
                )
                .await;
            c.port
                .send_channel_text(
                    interaction.channel_id,
                    &format!(
                        "✅ <@{}> – **{coach_name}** ist jetzt dein Coach! Stimmt euch hier ab.",
                        request.user_id
                    ),
                )
                .await;
            if let Some(message_id) = message_id {
                let embed = build_request_embed_for_existing_request(&request, None, None, now_ts);
                c.port
                    .edit_request_message(
                        REQUEST_CHANNEL_ID,
                        message_id,
                        &format!("📥 Anfrage von <@{}> – ✅ geclaimt", request.user_id),
                        embed,
                        cancel_components(&session_id, request.user_id),
                    )
                    .await;
            }
            // Website-Mirror (Python `CoachClaimButton.callback`:380):
            // Session als 'active' mit dem claimenden Coach spiegeln.
            c.mirror_to_website(MirrorOpts {
                request_id,
                coach_discord_id: Some(interaction.user_id),
                coach_username: Some(coach_name.clone()),
                session_status: Some("active".to_string()),
                ..MirrorOpts::default()
            });
            let dm_note = if dm_ok {
                ""
            } else {
                " (DM an User fehlgeschlagen – bitte im Channel anpingen.)"
            };
            return BridgeReply::ephemeral_text(format!(
                "✅ Session mit {} gestartet!{dm_note}",
                request.username
            ));
        }

        if let Some(rest) = interaction.custom_id.strip_prefix("coach_release_") {
            let request_id: i64 = rest
                .split('_')
                .next()
                .and_then(|r| r.parse().ok())
                .unwrap_or(0);
            let Some((_, status, assigned, _, _, _)) = c.load_request(request_id).await else {
                return BridgeReply::ephemeral_text("❌ Request nicht gefunden.");
            };
            if status == "matched" {
                return BridgeReply::ephemeral_text(
                    "❌ Schon geclaimt – kann nicht mehr freigegeben werden.",
                );
            }
            let Some(assigned) = assigned else {
                return BridgeReply::ephemeral_text(
                    "ℹ️ Dieses Coaching ist bereits für alle offen.",
                );
            };
            let is_owner = interaction.user_id == OWNER_EXCLUDE_ID
                || c.port
                    .member_is_admin(interaction.guild_id, interaction.user_id)
                    .await;
            if assigned != interaction.user_id && !is_owner {
                return BridgeReply::ephemeral_text(
                    "❌ Nur der reservierte Coach oder ein Admin kann freigeben.",
                );
            }
            c.open_request_to_all(request_id, "manual").await;
            return BridgeReply::ephemeral_text(
                "✅ Coaching freigegeben – jetzt für alle Coaches offen.",
            );
        }

        if let Some(rest) = interaction.custom_id.strip_prefix("coach_cancel_") {
            // Python-Legacy: {session_id}; Rust-neu: {session_id}_{author_id}.
            // Die DB bleibt Quelle für die betroffene User-ID.
            let session_id = match rest.rsplit_once('_') {
                Some((sid, aid)) if aid.parse::<u64>().is_ok() => sid.to_string(),
                _ => rest.to_string(),
            };
            let session = sqlx::query!(
                r#"
                SELECT coach_id, bot_request_id, discord_user_id
                  FROM coaching.sessions
                 WHERE id = $1
                "#,
                session_id,
            )
            .fetch_optional(&c.pool)
            .await
            .ok()
            .flatten()
            .and_then(|row| {
                Some((
                    row.coach_id.and_then(|raw| raw.parse::<u64>().ok()),
                    i64::from(row.bot_request_id?),
                    pg_i64_to_u64(row.discord_user_id?, "discord_user_id").ok()?,
                ))
            });
            let Some((coach_id, request_id, author_id)) = session else {
                return BridgeReply::ephemeral_text("❌ Session nicht gefunden.");
            };
            let is_owner = interaction.user_id == OWNER_EXCLUDE_ID
                || c.port
                    .member_is_admin(interaction.guild_id, interaction.user_id)
                    .await;
            if coach_id != Some(interaction.user_id) && !is_owner {
                return BridgeReply::ephemeral_text(
                    "❌ Nur der zugewiesene Coach oder ein Admin kann abbrechen.",
                );
            }
            let now_dt = chrono::Utc::now();
            let ban_expiry = now_dt + chrono::Duration::days(7);
            let _ = sqlx::query!(
                r#"
                UPDATE coaching.sessions
                   SET status = 'cancelled',
                       completed_at = $1
                 WHERE id = $2
                "#,
                now_dt,
                session_id,
            )
            .execute(&c.pool)
            .await;
            if let Ok(request_id_i32) = i64_to_i32(request_id, "request_id") {
                let _ = sqlx::query!(
                    r#"
                    UPDATE coaching.requests
                       SET status = 'cancelled',
                           updated_at = $1
                     WHERE bot_request_id = $2
                    "#,
                    now_dt,
                    request_id_i32,
                )
                .execute(&c.pool)
                .await;
            }
            if let Ok(author_id_i64) = u64_to_i64(author_id, "author_id") {
                let _ = sqlx::query!(
                    r#"
                    INSERT INTO coaching.bans(discord_user_id, banned_at, expires_at, reason)
                    VALUES ($1, $2, $3, $4)
                    ON CONFLICT(discord_user_id) DO UPDATE SET
                      banned_at = excluded.banned_at,
                      expires_at = excluded.expires_at,
                      reason = excluded.reason
                    "#,
                    author_id_i64,
                    now_dt,
                    ban_expiry,
                    "User hat sich nicht gemeldet / Abbruch durch Coach",
                )
                .execute(&c.pool)
                .await;
            }
            c.remove_role_if_present(
                interaction.guild_id,
                author_id,
                COACHING_ACTIVE_ROLE_ID,
                "Coaching abgebrochen - 7D Ban",
            )
            .await;
            let coach_name = c
                .port
                .member_display_name(interaction.guild_id, interaction.user_id)
                .await;
            let expiry_text = ban_expiry
                .with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string();
            c.port
                .send_dm(
                    author_id,
                    &format!(
                        "❌ Dein Coaching wurde vom Coach abgebrochen (z.B. weil du dich nicht gemeldet hast).\n\
                         Du wurdest für **7 Tage** für neue Coaching-Anfragen gesperrt.\n\
                         Sperre endet am: {expiry_text}"
                    ),
                )
                .await;
            c.port
                .send_channel_text(
                    interaction.channel_id,
                    &format!(
                        "⚠️ Das Coaching für <@{author_id}> wurde von {coach_name} abgebrochen. Der User wurde für 7 Tage gesperrt."
                    ),
                )
                .await;
            // Website-Mirror (Python `CoachCancelButton`:213): Session als
            // 'cancelled' mit dem abbrechenden Coach spiegeln.
            c.mirror_to_website(MirrorOpts {
                request_id,
                coach_discord_id: Some(interaction.user_id),
                coach_username: Some(coach_name),
                session_status: Some("cancelled".to_string()),
                ..MirrorOpts::default()
            });
            return BridgeReply::ephemeral_text(
                "✅ Coaching erfolgreich abgebrochen und User für 7 Tage gesperrt.",
            );
        }

        BridgeReply::ephemeral_text("Unbekannte Aktion.")
    }
}

pub fn register(router: &mut InteractionRouter, coaching: Arc<CoachingRequests>) {
    let handler = Arc::new(CoachingHandler { coaching });
    router.on_command(
        "coaching-anfrage",
        CommandSpec {
            definition: json!({
                "name": "coaching-anfrage",
                "description": "Stelle eine Coaching-Anfrage",
                "type": 1,
            }),
        },
        handler.clone(),
    );
    router.on_command(
        "coaching-status",
        CommandSpec {
            definition: json!({
                "name": "coaching-status",
                "description": "Pruefe den Status deiner Anfrage",
                "type": 1,
            }),
        },
        handler.clone(),
    );
    router.on_custom_id("coaching_panel_start", handler.clone());
    router.on_prefix("coach_", handler);
    // Website-driven intake: Der neue Panel-Button ist ein Link-Button ohne
    // Interaction. Der Legacy-custom_id bleibt nur als Redirect-Fallback.
    // #17/#18 entfallen bewusst; Rollen-/Analyse-/Stale-Flows übernimmt die Website.
}

/// Survey-Poll + Voice-Event-Listener starten. Der alte Discord-Intake bleibt
/// im Website-Redesign aus; dieser Worker deckt nur das Python-`coaching_survey`
/// Verhalten ab.
pub fn spawn(
    coaching: Arc<CoachingRequests>,
    dispatcher: &dl_discord::Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
    tracing::info!(
        "Coaching: Survey-Poll und Voice-Ende-Listener aktiv; Discord-Intake bleibt website-driven"
    );
    let poll = coaching.clone();
    let poll_task = tokio::spawn(async move {
        // main.rs startet Gateway-Worker vor `client.start()`. Der erste Scan
        // wartet kurz, damit der Voice-Cache nicht leer als "alle weg" zählt.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        loop {
            poll.scan_survey_sessions().await;
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });

    let mut voice_events = dispatcher.subscribe_voice();
    let voice_task = tokio::spawn(async move {
        loop {
            match voice_events.recv().await {
                Ok(event) => {
                    let (guild_id, user_id, voice_transition_can_end) = match event {
                        dl_discord::VoiceEvent::Join {
                            guild_id, user_id, ..
                        } => (guild_id, user_id, false),
                        dl_discord::VoiceEvent::Leave {
                            guild_id, user_id, ..
                        }
                        | dl_discord::VoiceEvent::Move {
                            guild_id, user_id, ..
                        } => (guild_id, user_id, true),
                        dl_discord::VoiceEvent::Update {
                            guild_id, user_id, ..
                        } => (guild_id, user_id, false),
                    };
                    if guild_id == coaching.guild_id {
                        coaching
                            .survey_sessions_for_member(user_id, voice_transition_can_end)
                            .await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Coaching-Survey: Voice-Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    vec![poll_task, voice_task]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_rank_und_games_hours_bleiben_python_kompatibel() {
        assert_eq!(combine_rank("Phantom", "III"), "Phantom III");
        assert_eq!(combine_rank("Phantom III", "III"), "Phantom III");
        assert_eq!(combine_rank("", "IV"), "IV");
        assert_eq!(combine_games_hours("120 games", "300h"), "120 games / 300h");
        assert_eq!(combine_games_hours("", "300h"), "300h");
    }

    #[test]
    fn survey_dm_text_entspricht_python_verbatim() {
        let embed = survey_embed("CoachName", 1);
        assert_eq!(embed["title"], "🎮 Coaching abgeschlossen!");
        assert_eq!(
            embed["description"],
            "Deine Coaching-Session mit **CoachName** ist beendet. Wir hoffen, es hat dir geholfen!"
        );
        assert_eq!(embed["fields"][0]["name"], "⭐ Gib uns Feedback");
        assert_eq!(
            embed["fields"][0]["value"],
            "Du hast nun für **5 Tage** Zugriff auf unseren Feedback-Kanal. Bitte teile deine Erfahrungen dort mit uns:\n\n👉 [**HIER FEEDBACK ABGEBEN**](https://discord.com/channels/1/1494756126644895885)\n\nDein Feedback hilft uns die Qualität der Coaches sicherzustellen!"
        );
        assert_eq!(embed["fields"][0]["inline"], false);
    }
}

#[cfg(all(test, feature = "testing"))]
mod pg_tests {
    use super::*;
    use dl_central_db::testing::{test_pool, TestDb};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    struct RequestMessageCall {
        channel_id: u64,
        content: String,
        embed: Value,
        components: Value,
    }

    #[derive(Default)]
    struct MockCoachingPort {
        coach_ids: Mutex<Vec<u64>>,
        request_messages: Mutex<Vec<RequestMessageCall>>,
        role_ids: Mutex<Vec<(u64, Vec<u64>)>>,
        channel_texts: Mutex<Vec<(u64, String)>>,
        dm_texts: Mutex<Vec<(u64, String)>>,
        dm_embeds: Mutex<Vec<(u64, Value)>>,
        added_roles: Mutex<Vec<(u64, u64, u64, String)>>,
        removed_roles: Mutex<Vec<(u64, u64, u64, String)>>,
    }

    #[async_trait::async_trait]
    impl CoachingPort for MockCoachingPort {
        async fn post_panel(
            &self,
            _channel_id: u64,
            _body: Map<String, Value>,
        ) -> Result<u64, String> {
            Ok(42)
        }

        async fn edit_panel(
            &self,
            _channel_id: u64,
            _message_id: u64,
            _body: Map<String, Value>,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn coach_member_ids(&self, _guild_id: u64) -> Vec<u64> {
            self.coach_ids.lock().expect("coach_ids lock").clone()
        }

        async fn member_role_ids(&self, _guild_id: u64, user_id: u64) -> Vec<u64> {
            self.role_ids
                .lock()
                .expect("role_ids lock")
                .iter()
                .find_map(|(id, roles)| (*id == user_id).then(|| roles.clone()))
                .unwrap_or_default()
        }

        async fn member_display_name(&self, _guild_id: u64, user_id: u64) -> String {
            format!("Coach {user_id}")
        }

        async fn member_is_admin(&self, _guild_id: u64, _user_id: u64) -> bool {
            false
        }

        async fn send_request_message(
            &self,
            channel_id: u64,
            content: &str,
            embed: Value,
            components: Value,
        ) -> Result<u64, String> {
            let mut messages = self.request_messages.lock().expect("request_messages lock");
            let message_id = 9000 + u64::try_from(messages.len()).expect("message count fits u64");
            messages.push(RequestMessageCall {
                channel_id,
                content: content.to_string(),
                embed,
                components,
            });
            Ok(message_id)
        }

        async fn edit_request_message(
            &self,
            _channel_id: u64,
            _message_id: u64,
            _content: &str,
            _embed: Value,
            _components: Value,
        ) {
        }

        async fn send_channel_text(&self, channel_id: u64, content: &str) {
            self.channel_texts
                .lock()
                .expect("channel_texts lock")
                .push((channel_id, content.to_string()));
        }

        async fn send_dm(&self, user_id: u64, content: &str) -> bool {
            self.dm_texts
                .lock()
                .expect("dm_texts lock")
                .push((user_id, content.to_string()));
            true
        }

        async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str) {
            self.added_roles.lock().expect("added_roles lock").push((
                guild_id,
                user_id,
                role_id,
                reason.to_string(),
            ));
        }

        async fn remove_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str) {
            self.removed_roles
                .lock()
                .expect("removed_roles lock")
                .push((guild_id, user_id, role_id, reason.to_string()));
        }

        async fn member_voice_channel_in_category(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _category_id: u64,
        ) -> Option<u64> {
            None
        }

        async fn send_dm_embed(&self, user_id: u64, embed: Value) -> bool {
            self.dm_embeds
                .lock()
                .expect("dm_embeds lock")
                .push((user_id, embed));
            true
        }
    }

    async fn test_coaching() -> (TestDb, Arc<MockCoachingPort>, Arc<CoachingRequests>) {
        let db = test_pool().await.expect("test pool");
        let port = Arc::new(MockCoachingPort::default());
        port.coach_ids
            .lock()
            .expect("coach_ids lock")
            .extend([AUTO_ASSIGN_OPTOUT_IDS[0], 12345]);
        port.role_ids
            .lock()
            .expect("role_ids lock")
            .push((12345, vec![COACH_ROLE_ID]));
        let coaching = CoachingRequests::new(db.pool().clone(), port.clone(), None, 1, None);
        (db, port, coaching)
    }

    fn notification(website_request_id: &str, user_id: u64) -> RequestCreatedNotification {
        RequestCreatedNotification {
            website_request_id: website_request_id.to_string(),
            coachee_id: format!("coachee-{website_request_id}"),
            discord_user_id: user_id,
            discord_username: format!("Player{user_id}"),
            rank: "Phantom".to_string(),
            subrank: "III".to_string(),
            hero: Some("Ivy".to_string()),
            games_played: Some("120 games".to_string()),
            hours_played: Some("300h".to_string()),
            availability: Some("abends".to_string()),
            current_problems: Some("Laning und Map Movement".to_string()),
            preferred_coach_id: Some("coach-web-1".to_string()),
        }
    }

    #[tokio::test]
    async fn request_created_emuliert_bot_id_und_rundet_website_spalten() {
        let (db, _port, coaching) = test_coaching().await;
        let first = notification("web-1", 777);
        let second = notification("web-2", 778);

        let upsert_1 = coaching
            .upsert_request_created_notification(&first)
            .await
            .expect("first upsert");
        let upsert_2 = coaching
            .upsert_request_created_notification(&second)
            .await
            .expect("second upsert");

        assert_eq!(upsert_1.local_request_id, 1);
        assert_eq!(upsert_2.local_request_id, 2);
        assert!(!upsert_1.already_posted);
        assert!(!upsert_2.already_posted);

        let row = sqlx::query!(
            r#"
            SELECT request_uid,
                   bot_request_id AS "bot_request_id!: i32",
                   website_request_id,
                   discord_user_id,
                   rank,
                   subrank,
                   hero,
                   games_played,
                   hours_played,
                   availability,
                   current_problems,
                   preferred_coach_id,
                   COALESCE(status, '') AS "status!"
              FROM coaching.requests
             WHERE website_request_id = 'web-1'
            "#
        )
        .fetch_one(db.pool())
        .await
        .expect("request row");

        assert_eq!(row.request_uid, "website:web-1");
        assert_eq!(row.bot_request_id, 1);
        assert_eq!(row.discord_user_id, 777);
        assert_eq!(row.rank, "Phantom");
        assert_eq!(row.subrank, "III");
        assert_eq!(row.hero.as_deref(), Some("Ivy"));
        assert_eq!(row.games_played.as_deref(), Some("120 games"));
        assert_eq!(row.hours_played.as_deref(), Some("300h"));
        assert_eq!(row.availability.as_deref(), Some("abends"));
        assert_eq!(
            row.current_problems.as_deref(),
            Some("Laning und Map Movement")
        );
        assert_eq!(row.preferred_coach_id.as_deref(), Some("coach-web-1"));
        assert_eq!(row.status, "pending");

        let insights = json!({
            "lane": "mid",
            "focus": ["positioning", "tempo"],
            "score": 7
        });
        sqlx::query!(
            r#"
            UPDATE coaching.requests
               SET ai_insights_json = $1::text::jsonb
             WHERE bot_request_id = $2
            "#,
            insights.to_string(),
            row.bot_request_id,
        )
        .execute(db.pool())
        .await
        .expect("write jsonb insights");
        let json_row = sqlx::query!(
            r#"
            SELECT ai_insights_json AS "ai_insights_json!: serde_json::Value"
              FROM coaching.requests
             WHERE bot_request_id = $1
            "#,
            row.bot_request_id,
        )
        .fetch_one(db.pool())
        .await
        .expect("read jsonb insights");
        assert_eq!(json_row.ai_insights_json, insights);
    }

    #[tokio::test]
    async fn request_created_duplikat_nutzt_bestehende_id_und_already_posted() {
        let (db, _port, coaching) = test_coaching().await;
        let mut item = notification("web-dupe", 800);
        let inserted = coaching
            .upsert_request_created_notification(&item)
            .await
            .expect("insert");
        let message_id = 55_i64;
        sqlx::query!(
            r#"
            UPDATE coaching.requests
               SET message_id = $1
             WHERE bot_request_id = $2
            "#,
            message_id,
            i32::try_from(inserted.local_request_id).expect("request id fits i32"),
        )
        .execute(db.pool())
        .await
        .expect("mark posted");

        item.discord_username = "UpdatedName".to_string();
        let updated = coaching
            .upsert_request_created_notification(&item)
            .await
            .expect("update");
        assert_eq!(updated.local_request_id, inserted.local_request_id);
        assert!(updated.already_posted);

        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!",
                   MAX(discord_username) AS "discord_username?"
              FROM coaching.requests
             WHERE website_request_id = 'web-dupe'
            "#
        )
        .fetch_one(db.pool())
        .await
        .expect("dupe row count");
        assert_eq!(row.count, 1);
        assert_eq!(row.discord_username.as_deref(), Some("UpdatedName"));
    }

    #[tokio::test]
    async fn request_created_vergibt_bot_id_fuer_website_union_zeile() {
        let (db, _port, coaching) = test_coaching().await;
        sqlx::query!(
            r#"
            INSERT INTO coaching.requests(
                request_uid, website_request_id, discord_user_id, discord_username,
                rank, subrank, status, created_at, updated_at
            )
            VALUES ('website-union:seed', 'web-union', 801, 'WebsiteOnly',
                    'Phantom', 'III', 'pending', now(), now())
            "#
        )
        .execute(db.pool())
        .await
        .expect("seed website row");

        let item = notification("web-union", 801);
        let upsert = coaching
            .upsert_request_created_notification(&item)
            .await
            .expect("upsert website union row");
        assert_eq!(upsert.local_request_id, 1);
        assert!(!upsert.already_posted);

        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!",
                   MIN(bot_request_id) AS "bot_request_id?",
                   MAX(discord_username) AS "discord_username?"
              FROM coaching.requests
             WHERE website_request_id = 'web-union'
            "#
        )
        .fetch_one(db.pool())
        .await
        .expect("website union row");
        assert_eq!(row.count, 1);
        assert_eq!(row.bot_request_id, Some(1));
        assert_eq!(row.discord_username.as_deref(), Some("Player801"));

        let duplicate_bot_id = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM coaching.requests
             WHERE bot_request_id = $1
            "#,
            i32::try_from(upsert.local_request_id).expect("request id fits i32"),
        )
        .fetch_one(db.pool())
        .await
        .expect("bot id count");
        assert_eq!(duplicate_bot_id, 1);
    }

    #[tokio::test]
    async fn post_request_to_channel_schreibt_rotation_und_post_status() {
        let (db, port, coaching) = test_coaching().await;
        let data = notification("web-post", 900);
        let upsert = coaching
            .upsert_request_created_notification(&data)
            .await
            .expect("upsert");
        let mut request = data.request_data(upsert.local_request_id);

        coaching
            .post_request_to_channel(
                &mut request,
                "**Analyse:** Map Movement".to_string(),
                true,
                json!([{ "type": 1, "components": [] }]),
            )
            .await
            .expect("post request");

        let messages = port.request_messages.lock().expect("request_messages lock");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].channel_id, REQUEST_CHANNEL_ID);
        assert!(messages[0].content.contains("reserviert"));
        assert_eq!(messages[0].embed["title"], "🎮 Neue Coaching-Anfrage");
        assert_eq!(messages[0].components[0]["type"], 1);
        drop(messages);

        let row = sqlx::query!(
            r#"
            SELECT message_id,
                   channel_id,
                   COALESCE(status, '') AS "status!",
                   assigned_coach_id,
                   ai_summary,
                   reserved_until
              FROM coaching.requests
             WHERE bot_request_id = $1
            "#,
            i32::try_from(upsert.local_request_id).expect("request id fits i32"),
        )
        .fetch_one(db.pool())
        .await
        .expect("posted row");
        assert_eq!(row.message_id, Some(9000));
        assert_eq!(
            row.channel_id,
            Some(i64::try_from(REQUEST_CHANNEL_ID).expect("channel id"))
        );
        assert_eq!(row.status, "analyzed");
        assert_eq!(row.assigned_coach_id.as_deref(), Some("12345"));
        assert_eq!(
            row.ai_summary,
            Some("**Analyse:** Map Movement".to_string())
        );
        assert!(row.reserved_until.is_some());

        let rotation = sqlx::query!(
            r#"
            SELECT coach_id, last_assigned_at
              FROM coaching.coach_rotation
             WHERE coach_id = '12345'
            "#
        )
        .fetch_one(db.pool())
        .await
        .expect("rotation row");
        assert_eq!(rotation.coach_id, "12345");
        assert!(rotation.last_assigned_at <= chrono::Utc::now());
    }

    #[tokio::test]
    async fn ensure_panel_speichert_message_id_in_kv_store() {
        let (db, _port, coaching) = test_coaching().await;

        coaching.ensure_panel().await;

        let value = dl_central_db::kv::get(db.pool(), PANEL_KV_NS, PANEL_KV_KEY)
            .await
            .expect("kv get");
        assert_eq!(value.as_deref(), Some("42"));
    }
}
