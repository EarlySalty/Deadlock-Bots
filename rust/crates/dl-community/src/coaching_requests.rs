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
use dl_db::Db;
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use rusqlite::OptionalExtension;
use serde_json::{json, Map, Value};

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
pub const CLAIM_RESERVATION_HOURS: i64 = 24;
pub const ROLE_EXPIRY_HOURS: i64 = 48;
pub const COACHING_WEBSITE_URL: &str = "https://deutsche-deadlock-community.de/coaching";
pub const COACHING_WEBSITE_CTA_TEXT: &str =
    "👉 **Bereit loszulegen?** Stell deine Coaching-Anfrage direkt über den Button unten auf unserer Website — dort füllst du in einer Minute alles aus, der Rest läuft von selbst.";
pub const COACHING_WEBSITE_BUTTON_LABEL: &str = "Coaching-Anfrage starten";
const PANEL_KV_NS: &str = "coaching";
const PANEL_KV_KEY: &str = "panel_message_id";

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
/// dann wenigste aktive Sessions, dann kleinste ID. Owner/Bots filtert
/// der Aufrufer beim Kandidaten-Sammeln.
pub fn pick_fair_coach(candidates: &[(u64, i64, i64)]) -> Option<u64> {
    candidates
        .iter()
        .min_by_key(|(id, last_reserved, active)| (*last_reserved, *active, *id))
        .map(|(id, _, _)| *id)
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

/// uuid4-Format aus Zufallsbytes (wie str(uuid.uuid4())).
pub fn new_session_id() -> String {
    let bytes: [u8; 16] = rand::random();
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-4{:01x}{:02x}-{:01x}{:01x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
        bytes[6] & 0x0f, bytes[7],
        8 + (bytes[8] & 0x03), bytes[8] & 0x0f, bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
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

/// Anfrage-Embed (wie _build_request_embed).
pub fn build_request_embed(
    request: &RequestData,
    assigned_coach_id: Option<u64>,
    reserved_until: Option<i64>,
    now_ts: i64,
) -> Value {
    let mut fields = vec![
        json!({ "name": "Rang", "value": normalize_inline(&request.rank, "N/A", 256), "inline": true }),
        json!({ "name": "Hero", "value": normalize_inline(&request.hero, "Nicht angegeben", 256), "inline": true }),
        json!({ "name": "Games / Stunden", "value": normalize_inline(&request.games_played, "N/A", 256), "inline": true }),
        json!({ "name": "📅 Bevorzugter Slot", "value": normalize_inline(&request.scheduled_slot, "Nicht angegeben", 256), "inline": false }),
        json!({ "name": "📝 Probleme", "value": normalize_inline(&request.current_problems, "Keine Beschreibung", 1024), "inline": false }),
        json!({ "name": "🤖 AI Analyse", "value": format_ai_summary(&request.ai_summary), "inline": false }),
    ];
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

pub fn claim_components(request_id: i64, author_id: u64) -> Value {
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 3, "label": "Coaching übernehmen",
          "custom_id": format!("coach_claim_{request_id}") },
        { "type": 2, "style": 2, "label": "Freigeben",
          "custom_id": format!("coach_release_{request_id}_{author_id}") },
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
    pub db: Db,
    pub port: Arc<dyn CoachingPort>,
    pub ai: Option<Arc<dyn TextGenerator>>,
    pub guild_id: u64,
    /// Website-Spiegelung der Anfrage-/Session-Zustände (Python
    /// `_mirror_to_website`). `None` = inaktiv (kein interner Token), genau
    /// wie wenn `website_client._token()` leer ist.
    pub website: Option<Arc<crate::coaching::WebsiteClient>>,
}

impl CoachingRequests {
    pub fn new(
        db: Db,
        port: Arc<dyn CoachingPort>,
        ai: Option<Arc<dyn TextGenerator>>,
        guild_id: u64,
        website: Option<Arc<crate::coaching::WebsiteClient>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
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
                if let Err(err) = self
                    .db
                    .kv_set(PANEL_KV_NS, PANEL_KV_KEY, message_id.to_string())
                    .await
                {
                    tracing::warn!(%err, "Coaching-Panel-ID konnte nicht gespeichert werden");
                }
            }
            Err(err) => tracing::warn!(%err, "Coaching-Panel konnte nicht gepostet werden"),
        }
    }

    async fn panel_message_id(&self) -> Option<u64> {
        self.db
            .kv_get(PANEL_KV_NS, PANEL_KV_KEY)
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
        let db = self.db.clone();
        tokio::spawn(async move {
            let request_id = opts.request_id;
            // Volle Zeile lesen — wie `db.query_one("SELECT * ...")` in Python.
            let row: Option<MirrorRow> = db
                .read(move |conn| {
                    conn.query_row(
                        "SELECT id, discord_user_id, COALESCE(discord_username,''),
                                rank, subrank, hero, games_played, hours_played,
                                availability, current_problems, COALESCE(ai_summary,''),
                                status, assigned_coach_id, reserved_until
                           FROM coaching_requests WHERE id = ?1",
                        [request_id],
                        |r| {
                            Ok(MirrorRow {
                                id: r.get(0)?,
                                discord_user_id: r.get(1)?,
                                discord_username: r.get(2)?,
                                rank: r.get(3)?,
                                subrank: r.get(4)?,
                                hero: r.get::<_, Option<String>>(5)?,
                                games_played: r.get::<_, Option<String>>(6)?,
                                hours_played: r.get::<_, Option<String>>(7)?,
                                availability: r.get::<_, Option<String>>(8)?,
                                current_problems: r.get::<_, Option<String>>(9)?,
                                ai_summary: r.get(10)?,
                                status: r.get(11)?,
                                assigned_coach_id: r.get::<_, Option<String>>(12)?,
                                reserved_until: r.get::<_, Option<i64>>(13)?,
                            })
                        },
                    )
                    .optional()
                })
                .await
                .ok()
                .flatten();
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
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT id, discord_user_id, COALESCE(discord_username,''), rank,
                            COALESCE(hero,''), COALESCE(games_played,''),
                            COALESCE(scheduled_slot,''), COALESCE(current_problems,''),
                            COALESCE(ai_summary,''), status, assigned_coach_id,
                            reserved_until, message_id, role_expires_at
                       FROM coaching_requests WHERE id = ?1",
                    [request_id],
                    |row| {
                        Ok(Some((
                            RequestData {
                                id: row.get(0)?,
                                user_id: row.get(1)?,
                                username: row.get(2)?,
                                rank: row.get(3)?,
                                hero: row.get(4)?,
                                games_played: row.get(5)?,
                                scheduled_slot: row.get(6)?,
                                current_problems: row.get(7)?,
                                ai_summary: row.get(8)?,
                            },
                            row.get::<_, String>(9)?,
                            row.get::<_, Option<u64>>(10)?,
                            row.get::<_, Option<i64>>(11)?,
                            row.get::<_, Option<u64>>(12)?,
                            row.get::<_, Option<i64>>(13)?,
                        )))
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .flatten()
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

    /// Anfrage posten (wie _post_request_to_channel): faire Rotation +
    /// 24-h-Reservierung.
    async fn post_request(&self, request: &mut RequestData, ai_summary: String) {
        request.ai_summary = ai_summary.clone();
        let now_ts = chrono::Utc::now().timestamp();
        // Kandidaten + Rotations-Daten
        let coach_ids = self.port.coach_member_ids(self.guild_id).await;
        let stats: Vec<(u64, i64, i64)> = {
            let ids = coach_ids.clone();
            self.db
                .read(move |conn| {
                    let mut result = Vec::new();
                    for id in ids {
                        let last: i64 = conn
                            .query_row(
                                "SELECT COALESCE(MAX(reserved_until), 0) FROM coaching_requests
                                  WHERE assigned_coach_id = ?1",
                                [id],
                                |row| row.get(0),
                            )
                            .unwrap_or(0);
                        let active: i64 = conn
                            .query_row(
                                "SELECT COUNT(*) FROM coaching_sessions
                                  WHERE coach_id = ?1 AND status = 'active'",
                                [id],
                                |row| row.get(0),
                            )
                            .unwrap_or(0);
                        result.push((id, last, active));
                    }
                    Ok(result)
                })
                .await
                .unwrap_or_default()
        };
        let assigned = pick_fair_coach(&stats);
        let reserved_until = assigned.map(|_| now_ts + CLAIM_RESERVATION_HOURS * 3600);
        let embed = build_request_embed(request, assigned, reserved_until, now_ts);
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
        let components = claim_components(request.id, request.user_id);
        match self
            .port
            .send_request_message(REQUEST_CHANNEL_ID, &content, embed, components)
            .await
        {
            Ok(message_id) => {
                let request_id = request.id;
                let _ = self
                    .db
                    .write(move |conn| {
                        conn.execute(
                            "UPDATE coaching_requests SET message_id=?1, channel_id=?2,
                                    ai_summary=?3, status='analyzed', assigned_coach_id=?4,
                                    reserved_until=?5, updated_at=?6 WHERE id=?7",
                            rusqlite::params![
                                message_id,
                                REQUEST_CHANNEL_ID,
                                ai_summary,
                                assigned,
                                reserved_until,
                                chrono::Utc::now().timestamp(),
                                request_id
                            ],
                        )
                        .map(|_| ())
                    })
                    .await;
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
            }
            Err(err) => tracing::warn!(%err, "Coaching-Post fehlgeschlagen"),
        }
    }

    /// Reservierung aufheben + Nachricht aktualisieren (wie _open_request_to_all).
    pub async fn open_request_to_all(&self, request_id: i64, reason: &str) {
        let now_ts = chrono::Utc::now().timestamp();
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE coaching_requests SET assigned_coach_id=NULL, reserved_until=NULL,
                            updated_at=?1 WHERE id=?2 AND status='analyzed'",
                    rusqlite::params![now_ts, request_id],
                )
                .map(|_| ())
            })
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
        let embed = build_request_embed(&request, None, None, now_ts);
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
        let rows: Vec<i64> = self
            .db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id FROM coaching_requests
                      WHERE status='pending' AND current_problems IS NOT NULL
                        AND current_problems != ''
                        AND (ai_summary IS NULL OR ai_summary = '')
                      ORDER BY created_at ASC LIMIT 5",
                )?;
                let rows = stmt.query_map([], |row| row.get(0))?;
                rows.collect()
            })
            .await
            .unwrap_or_default();
        for request_id in rows {
            let claimed: usize = self
                .db
                .write(move |conn| {
                    conn.execute(
                        "UPDATE coaching_requests SET status='analyzing', updated_at=?1
                          WHERE id=?2 AND status='pending'",
                        rusqlite::params![chrono::Utc::now().timestamp(), request_id],
                    )
                })
                .await
                .unwrap_or(0);
            if claimed == 0 {
                continue;
            }
            let Some((mut request, ..)) = self.load_request(request_id).await else {
                continue;
            };
            let summary = self.analyze(&request).await;
            if summary.is_empty() {
                let _ = self
                    .db
                    .write(move |conn| {
                        conn.execute(
                            "UPDATE coaching_requests SET status='invalid', updated_at=?1 WHERE id=?2",
                            rusqlite::params![chrono::Utc::now().timestamp(), request_id],
                        )
                        .map(|_| ())
                    })
                    .await;
                continue;
            }
            self.post_request(&mut request, summary).await;
        }
    }

    /// Abgelaufene Reservierungen öffnen (60-s-Loop).
    pub async fn expire_reservations(&self) {
        let now_ts = chrono::Utc::now().timestamp();
        let expired: Vec<i64> = self
            .db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id FROM coaching_requests
                      WHERE status='analyzed' AND assigned_coach_id IS NOT NULL
                        AND reserved_until IS NOT NULL AND reserved_until < ?1",
                )?;
                let rows = stmt.query_map([now_ts], |row| row.get(0))?;
                rows.collect()
            })
            .await
            .unwrap_or_default();
        for request_id in expired {
            self.open_request_to_all(request_id, "expired").await;
        }
    }

    /// Abgelaufene Coaching-Rollen entfernen (Port `coaching_role_manager`):
    /// aktive Rolle nach 48 h (`coaching_requests.role_expires_at`), Reward-
    /// Rolle nach 5 Tagen (`coaching_sessions.reward_role_expires_at`).
    pub async fn expire_roles(&self) {
        let now = chrono::Utc::now().timestamp();
        let guild_id = self.guild_id;

        let active: Vec<(i64, u64)> = self
            .db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, discord_user_id FROM coaching_requests
                      WHERE role_removed_at IS NULL AND role_expires_at IS NOT NULL
                        AND role_expires_at < ?1",
                )?;
                let rows = stmt.query_map([now], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .await
            .unwrap_or_default();
        for (req_id, user_id) in active {
            self.port
                .remove_role(
                    guild_id,
                    user_id,
                    COACHING_ACTIVE_ROLE_ID,
                    "Coaching-Rolle abgelaufen (48h)",
                )
                .await;
            let _ = self
                .db
                .write(move |conn| {
                    conn.execute(
                        "UPDATE coaching_requests SET role_removed_at=?1, updated_at=?1 WHERE id=?2",
                        rusqlite::params![now, req_id],
                    )
                    .map(|_| ())
                })
                .await;
            let thread: Option<u64> = self
                .db
                .read(move |conn| {
                    conn.query_row(
                        "SELECT discord_thread_id FROM coaching_sessions
                          WHERE request_id=?1 AND status IN ('active','waiting_survey')
                          ORDER BY created_at DESC LIMIT 1",
                        [req_id],
                        |r| r.get::<_, Option<u64>>(0),
                    )
                    .optional()
                    .map(|o| o.flatten())
                })
                .await
                .ok()
                .flatten();
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

        let reward: Vec<(i64, u64)> = self
            .db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, discord_user_id FROM coaching_sessions
                      WHERE reward_role_removed_at IS NULL AND reward_role_expires_at IS NOT NULL
                        AND reward_role_expires_at < ?1",
                )?;
                let rows = stmt.query_map([now], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .await
            .unwrap_or_default();
        for (sess_id, user_id) in reward {
            self.port
                .remove_role(
                    guild_id,
                    user_id,
                    COACHING_REWARD_ROLE_ID,
                    "Coaching Reward-Rolle abgelaufen (5 Tage)",
                )
                .await;
            let _ = self
                .db
                .write(move |conn| {
                    conn.execute(
                        "UPDATE coaching_sessions SET reward_role_removed_at=?1 WHERE id=?2",
                        rusqlite::params![now, sess_id],
                    )
                    .map(|_| ())
                })
                .await;
        }
    }

    /// Survey-Poll (Python `_scan_active_sessions`, 60-s-Loop): alle aktiven
    /// Sessions ohne gesendete Umfrage prüfen.
    pub async fn scan_survey_sessions(&self) {
        for session in self.load_survey_sessions(None).await {
            self.process_survey_session(session).await;
        }
    }

    /// Voice-getriggerte Prüfung (Python `on_voice_state_update`): nur die
    /// Sessions, an denen dieses Mitglied als User oder Coach beteiligt ist.
    pub async fn survey_sessions_for_member(&self, member_id: u64) {
        for session in self.load_survey_sessions(Some(member_id)).await {
            self.process_survey_session(session).await;
        }
    }

    async fn load_survey_sessions(&self, member: Option<u64>) -> Vec<SurveySession> {
        self.db
            .read(move |conn| {
                let map = |row: &rusqlite::Row| {
                    Ok(SurveySession {
                        id: row.get(0)?,
                        coach_id: row
                            .get::<_, Option<String>>(1)?
                            .and_then(|s| s.parse::<u64>().ok()),
                        user_id: row.get::<_, i64>(2)? as u64,
                        voice_started_at: row.get(3)?,
                        request_id: row.get(4)?,
                    })
                };
                let mut out = Vec::new();
                match member {
                    Some(mid) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, coach_id, discord_user_id, voice_started_at, request_id
                               FROM coaching_sessions
                              WHERE status='active' AND survey_sent_at IS NULL
                                AND (discord_user_id=?1 OR coach_id=?2)",
                        )?;
                        let rows =
                            stmt.query_map(rusqlite::params![mid as i64, mid.to_string()], map)?;
                        for r in rows {
                            out.push(r?);
                        }
                    }
                    None => {
                        let mut stmt = conn.prepare(
                            "SELECT id, coach_id, discord_user_id, voice_started_at, request_id
                               FROM coaching_sessions
                              WHERE status='active' AND survey_sent_at IS NULL",
                        )?;
                        let rows = stmt.query_map([], map)?;
                        for r in rows {
                            out.push(r?);
                        }
                    }
                }
                Ok(out)
            })
            .await
            .unwrap_or_default()
    }

    /// Kern-Logik (Python `_process_session_voice_state`): sitzen User und Coach
    /// noch im selben Coaching-VC, läuft die Session (Tracking-Update). Sind sie
    /// es nicht mehr und war zuvor eine Voice-Session aktiv, gilt sie als beendet
    /// → Active-Rolle weg, Reward-Rolle (5 Tage) und Survey-DM.
    async fn process_survey_session(&self, session: SurveySession) {
        let guild = self.guild_id;
        let user_vc = self
            .port
            .member_voice_channel_in_category(guild, session.user_id, COACHING_VOICE_CATEGORY_ID)
            .await;
        let coach_vc = match session.coach_id {
            Some(cid) => {
                self.port
                    .member_voice_channel_in_category(guild, cid, COACHING_VOICE_CATEGORY_ID)
                    .await
            }
            None => None,
        };
        let now = chrono::Utc::now().timestamp();

        // Beide noch im selben Coaching-VC → Voice-Tracking aktualisieren.
        if user_vc.is_some() && user_vc == coach_vc {
            let vc = user_vc.unwrap_or_default() as i64;
            let id = session.id.clone();
            let _ = self
                .db
                .write(move |conn| {
                    conn.execute(
                        "UPDATE coaching_sessions
                            SET voice_channel_id=?1,
                                voice_started_at=COALESCE(voice_started_at, ?2),
                                voice_last_seen_at=?2
                          WHERE id=?3",
                        rusqlite::params![vc, now, id],
                    )
                    .map(|_| ())
                })
                .await;
            return;
        }

        // Noch nie gemeinsam im Voice → es gibt keine Session zu beenden.
        if session.voice_started_at.is_none() {
            return;
        }

        // Voice-Session beendet → atomar abschließen. Der WHERE-Filter macht den
        // Claim wettlaufsicher: Poll und Voice-Listener können dieselbe Session
        // gleichzeitig sehen, aber nur einer trifft `status='active'`.
        let reward_expiry = now + REWARD_ROLE_DURATION_SECS;
        let id = session.id.clone();
        let claimed = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE coaching_sessions
                        SET status='completed', completed_at=?1, survey_sent_at=?1,
                            reward_role_expires_at=?2, voice_last_seen_at=?1
                      WHERE id=?3 AND status='active' AND survey_sent_at IS NULL",
                    rusqlite::params![now, reward_expiry, id],
                )
            })
            .await
            .unwrap_or(0);
        if claimed == 0 {
            return;
        }

        let coach_name = match session.coach_id {
            Some(cid) => self.port.member_display_name(guild, cid).await,
            None => "Coach".to_string(),
        };

        self.port
            .remove_role(
                guild,
                session.user_id,
                COACHING_ACTIVE_ROLE_ID,
                "Coaching Session beendet",
            )
            .await;
        self.port
            .add_role(
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
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE coaching_requests
                        SET status='completed', role_removed_at=?1, updated_at=?1
                      WHERE id=?2",
                    rusqlite::params![now, rid],
                )
                .map(|_| ())
            })
            .await;
        // Website-Mirror (Python `coaching_survey.py`:311): Session als
        // 'completed' spiegeln, inkl. bot_session_id.
        self.mirror_to_website(MirrorOpts {
            request_id: rid,
            coach_discord_id: session.coach_id,
            coach_username: Some(coach_name),
            session_status: Some("completed".to_string()),
            bot_session_id: Some(session.id.clone()),
            ..MirrorOpts::default()
        });
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
            let user_id = interaction.user_id;
            let status: Option<String> =
                c.db.read(move |conn| {
                    conn.query_row(
                        "SELECT status FROM coaching_requests
                          WHERE discord_user_id = ?1 ORDER BY created_at DESC LIMIT 1",
                        rusqlite::params![user_id],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                })
                .await
                .ok()
                .flatten();
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
            let _ =
                c.db.write(move |conn| {
                    conn.execute(
                        "INSERT INTO coaching_requests (
                           discord_user_id, discord_username, rank, subrank, hero,
                           availability, scheduled_slot, games_played, hours_played,
                           current_problems, status, created_at, updated_at
                         ) VALUES (?1, ?2, ?3, '', ?4, ?5, ?5, ?6, '', ?7, 'pending', ?8, ?8)",
                        rusqlite::params![
                            user_id,
                            username,
                            rank,
                            hero,
                            availability,
                            games_hours,
                            problems,
                            chrono::Utc::now().timestamp()
                        ],
                    )
                    .map(|_| ())
                })
                .await;
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
            let (sid, cid, uid, uname) = (
                session_id.clone(),
                interaction.user_id,
                request.user_id,
                request.username.clone(),
            );
            let channel_id = interaction.channel_id;
            let _ =
                c.db.write(move |conn| {
                    conn.execute(
                        "INSERT INTO coaching_sessions (id, request_id, coach_id, discord_user_id,
                           discord_username, discord_channel_id, status,
                           role_assigned_at, role_expires_at, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?7)",
                        rusqlite::params![
                            sid, request_id, cid, uid, uname, channel_id, now_ts, expires_at
                        ],
                    )?;
                    conn.execute(
                        "UPDATE coaching_requests SET status='matched', updated_at=?1 WHERE id=?2",
                        rusqlite::params![now_ts, request_id],
                    )
                    .map(|_| ())
                })
                .await;
            c.port
                .add_role(
                    interaction.guild_id,
                    request.user_id,
                    COACHING_ACTIVE_ROLE_ID,
                    "Coaching Session gestartet",
                )
                .await;
            c.port
                .send_dm(
                    request.user_id,
                    &format!(
                        "🎉 Ein Coach hat sich für deine Anfrage gemeldet!\n\n**Coach:** {coach_name}\n\nSchau in den Coaching-Channel um euch abzustimmen und das Coaching innerhalb der nächsten **{ROLE_EXPIRY_HOURS} Stunden** durchzuführen."
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
                let embed = build_request_embed(&request, None, None, now_ts);
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
            return BridgeReply::ephemeral_text(
                "✅ Session gestartet — der Spieler wurde benachrichtigt.",
            );
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
            // {session_id}_{author_id} — session_id enthält Bindestriche, kein '_'
            let (session_id, author_id) = match rest.rsplit_once('_') {
                Some((sid, aid)) => (sid.to_string(), aid.parse::<u64>().unwrap_or(0)),
                None => (rest.to_string(), 0),
            };
            let sid = session_id.clone();
            let session: Option<(u64, i64)> =
                c.db.read(move |conn| {
                    conn.query_row(
                        "SELECT coach_id, request_id FROM coaching_sessions WHERE id = ?1",
                        [sid],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                })
                .await
                .ok()
                .flatten();
            let Some((coach_id, request_id)) = session else {
                return BridgeReply::ephemeral_text("❌ Session nicht gefunden.");
            };
            let is_owner = interaction.user_id == OWNER_EXCLUDE_ID
                || c.port
                    .member_is_admin(interaction.guild_id, interaction.user_id)
                    .await;
            if coach_id != interaction.user_id && !is_owner {
                return BridgeReply::ephemeral_text(
                    "❌ Nur der zugewiesene Coach oder ein Admin kann abbrechen.",
                );
            }
            let ban_expiry = now_ts + 7 * 24 * 3600;
            let sid2 = session_id.clone();
            let _ = c
                .db
                .write(move |conn| {
                    conn.execute(
                        "UPDATE coaching_sessions SET status='cancelled', completed_at=?1 WHERE id=?2",
                        rusqlite::params![now_ts, sid2],
                    )?;
                    conn.execute(
                        "UPDATE coaching_requests SET status='cancelled', updated_at=?1 WHERE id=?2",
                        rusqlite::params![now_ts, request_id],
                    )?;
                    conn.execute(
                        "INSERT INTO coaching_bans (discord_user_id, banned_at, expires_at, reason)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(discord_user_id) DO UPDATE SET
                           banned_at=excluded.banned_at, expires_at=excluded.expires_at,
                           reason=excluded.reason",
                        rusqlite::params![
                            author_id,
                            now_ts,
                            ban_expiry,
                            "User hat sich nicht gemeldet / Abbruch durch Coach"
                        ],
                    )
                    .map(|_| ())
                })
                .await;
            c.port
                .remove_role(
                    interaction.guild_id,
                    author_id,
                    COACHING_ACTIVE_ROLE_ID,
                    "Coaching abgebrochen - 7D Ban",
                )
                .await;
            // Website-Mirror (Python `CoachCancelButton`:213): Session als
            // 'cancelled' mit dem abbrechenden Coach spiegeln.
            let coach_name = c
                .port
                .member_display_name(interaction.guild_id, interaction.user_id)
                .await;
            c.mirror_to_website(MirrorOpts {
                request_id,
                coach_discord_id: Some(interaction.user_id),
                coach_username: Some(coach_name),
                session_status: Some("cancelled".to_string()),
                ..MirrorOpts::default()
            });
            return BridgeReply::ephemeral_text(
                "✅ Session abgebrochen — der Spieler ist 7 Tage fürs Coaching gesperrt.",
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
    // Website-driven intake: Der neue Panel-Button ist ein Link-Button ohne
    // Interaction. Der Legacy-custom_id bleibt nur als Redirect-Fallback.
    // #17/#18 entfallen bewusst; Rollen-/Analyse-/Stale-Flows übernimmt die Website.
}

/// Discord-seitige Intake-/Analyse-/Rollen-Loops sind im Website-Redesign aus.
pub fn spawn(
    _coaching: Arc<CoachingRequests>,
    _dispatcher: &dl_discord::Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
    tracing::info!(
        "Coaching: Discord-Intake-Loops deaktiviert (Website-driven intake, #17/#18 dropped)"
    );
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faire_rotation() {
        // am längsten nicht zugewiesen gewinnt
        let coaches = vec![(1u64, 100i64, 0i64), (2, 50, 3), (3, 50, 1)];
        assert_eq!(pick_fair_coach(&coaches), Some(3)); // 50 < 100, weniger aktive als 2
        assert_eq!(pick_fair_coach(&[]), None);
        // Gleichstand → kleinste ID
        let tie = vec![(9u64, 0i64, 0i64), (4, 0, 0)];
        assert_eq!(pick_fair_coach(&tie), Some(4));
    }

    #[test]
    fn texte_und_kappung() {
        assert_eq!(
            normalize_inline("  viel    raum  ", "N/A", 256),
            "viel raum"
        );
        assert_eq!(normalize_inline("", "N/A", 256), "N/A");
        let long = "x".repeat(300);
        assert_eq!(normalize_inline(&long, "N/A", 256).chars().count(), 256);
        assert_eq!(format_ai_summary(""), "Keine Analyse verfügbar.");
        let sid = new_session_id();
        assert_eq!(sid.len(), 36);
        assert_eq!(sid.chars().filter(|c| *c == '-').count(), 4);
        assert_eq!(sid.chars().nth(14), Some('4')); // uuid4-Version
    }

    #[test]
    fn embed_und_buttons() {
        let request = RequestData {
            id: 7,
            user_id: 42,
            username: "Nani".into(),
            rank: "Archon 3".into(),
            hero: "Haze".into(),
            games_played: "300 / 150".into(),
            scheduled_slot: "Montag 18:00".into(),
            current_problems: "Lane-Phase".into(),
            ai_summary: "Fokus: Last-Hits".into(),
        };
        let embed = build_request_embed(&request, Some(99), Some(1000), 500);
        let fields = embed["fields"].as_array().expect("fields");
        assert_eq!(fields.len(), 7); // 6 + Reservierung
        assert!(fields[6]["value"].as_str().expect("v").contains("<@99>"));
        // abgelaufene Reservierung → kein Feld
        let embed = build_request_embed(&request, Some(99), Some(1000), 2000);
        assert_eq!(embed["fields"].as_array().expect("fields").len(), 6);
        let claim = claim_components(7, 42);
        assert_eq!(claim[0]["components"][0]["custom_id"], "coach_claim_7");
        assert_eq!(claim[0]["components"][1]["custom_id"], "coach_release_7_42");
        let cancel = cancel_components("abc-def", 42);
        assert_eq!(
            cancel[0]["components"][0]["custom_id"],
            "coach_cancel_abc-def_42"
        );
    }

    #[test]
    fn panel_verweist_auf_website() {
        let embed = build_panel_embed();
        assert_eq!(embed["title"], "🎮  Deadlock Coaching");
        let description = embed["description"].as_str().unwrap_or_default();
        assert!(description
            .contains("Die Kommunikation findet **ausschließlich** im Coaching-Chat statt."));
        assert!(description.contains(
            "Bitte sende **keine** Freundschaftsanfragen (FAs) oder DMs an die Coaches."
        ));
        assert!(description.ends_with(COACHING_WEBSITE_CTA_TEXT));
        assert_eq!(embed["fields"][0]["name"], "📋 Ablauf");
        assert_eq!(
            embed["fields"][0]["value"],
            "1. Formular ausfüllen\n2. Du bekommst die Coaching-Rolle\n3. Ein Coach meldet sich bei dir"
        );
        assert_eq!(embed["fields"][1]["name"], "❓ Fragen nach dem Coaching?");
        assert_eq!(
            embed["fields"][1]["value"],
            "Hau sie einfach in <#1426220702054355077> raus statt per DM an deinen Coach. Dann sehen alle die Antwort und andere mit dem gleichen Thema lesen direkt mit."
        );

        let components = panel_components();
        let button = &components[0]["components"][0];
        assert_eq!(button["style"], 5);
        assert_eq!(button["label"], COACHING_WEBSITE_BUTTON_LABEL);
        assert_eq!(button["url"], COACHING_WEBSITE_URL);
    }
}
