//! Streamer-Link-Matcher — Port von `cogs/twitch/streamer_link_matcher.py`.
//!
//! Gleicht unverknüpfte Twitch-Streamer gegen die Discord-Memberliste ab:
//! Namens-Normalisierung + Fuzzy-Match (difflib-Algorithmus nachgebaut),
//! Score-Entscheidung Auto-Link / Review-Vorschlag (Buttons) / kein Treffer.
//!
//! Bewusste Lücke bis Phase 6: Das AI-Scoring (MiniMax via AIConnector) hängt
//! an dl-ai — bis dahin läuft der Heuristik-Modus, exakt wie das Original
//! ohne verfügbaren AIConnector (nur eindeutige Exakt-Treffer erreichen den
//! Auto-Bereich).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use dl_discord::{
    BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter, ModalField, ModalSpec,
};
use serde_json::{json, Map, Value};
use unicode_normalization::UnicodeNormalization;

use crate::twitch::TwitchApiClient;

pub const DEFAULT_NOTIFY_CHANNEL_ID: u64 = 1374364800817303632;
pub const DEFAULT_STREAMER_ROLE_ID: u64 = 1313624729466441769;
pub const DEFAULT_GUILD_ID: u64 = 1289721245281292288;
pub const REVIEW_PREFIX: &str = "slm:";

// ── Pure Funktionen (Referenzwerte aus CPython in den Tests) ───────────────

fn leet(ch: char) -> char {
    match ch {
        '0' => 'o',
        '1' => 'i',
        '3' => 'e',
        '4' => 'a',
        '5' => 's',
        '7' => 't',
        '$' => 's',
        '@' => 'a',
        '8' => 'b',
        other => other,
    }
}

fn deaccent(value: &str) -> String {
    // NFKD + ASCII-Filter wie `unicodedata.normalize("NFKD", …).encode("ascii","ignore")`
    value.nfkd().filter(char::is_ascii).collect()
}

fn tokens(value: &str) -> Vec<String> {
    let base: String = deaccent(value).to_lowercase().chars().map(leet).collect();
    let raw: Vec<String> = base
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    let kept: Vec<String> = raw
        .iter()
        .filter(|t| !PY_AFFIXES.contains(&t.as_str()))
        .cloned()
        .collect();
    if kept.is_empty() {
        raw
    } else {
        kept
    }
}

/// Affixe EXAKT wie das Python-Set (ohne die x/xx-Erweiterung oben — siehe Test).
const PY_AFFIXES: [&str; 16] = [
    "ttv", "live", "twitch", "stream", "streams", "streamer", "yt", "youtube", "tv", "official",
    "real", "the", "its", "im", "iam", "gg",
];

/// Normalisiert einen Namen zum Vergleichs-Schlüssel.
pub fn norm_key(value: &str) -> String {
    tokens(value).join("")
}

/// difflib `SequenceMatcher.ratio()` (Ratcliff/Obershelp): 2·M / (len(a)+len(b)).
pub fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let matches = matched_len(a.as_bytes(), b.as_bytes());
    2.0 * matches as f64 / (a.len() + b.len()) as f64
}

fn matched_len(a: &[u8], b: &[u8]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let (i, j, k) = longest_match(a, b);
    if k == 0 {
        return 0;
    }
    k + matched_len(&a[..i], &b[..j]) + matched_len(&a[i + k..], &b[j + k..])
}

fn longest_match(a: &[u8], b: &[u8]) -> (usize, usize, usize) {
    let mut best = (0usize, 0usize, 0usize);
    let mut j2len = vec![0usize; b.len() + 1];
    for (i, &byte_a) in a.iter().enumerate() {
        let mut new = vec![0usize; b.len() + 1];
        for (j, &byte_b) in b.iter().enumerate() {
            if byte_a == byte_b {
                let k = j2len[j] + 1;
                new[j + 1] = k;
                if k > best.2 {
                    best = (i + 1 - k, j + 1 - k, k);
                }
            }
        }
        j2len = new;
    }
    best
}

/// Konservativer Score ohne AI (nur exakte Unikate erreichen Auto-Bereich).
pub fn fallback_score(ratio: f64, exact_unique: bool) -> i64 {
    if exact_unique && ratio >= 0.999 {
        return 92;
    }
    if ratio >= 0.93 {
        return 80;
    }
    if ratio >= 0.82 {
        return 72;
    }
    dl_core::pyfloat::py_round(ratio * 70.0, 0) as i64
}

/// `{"score":..,"reason":..}` aus der AI-Antwort (inkl. <think>-Strip).
pub fn parse_ai_score(text: Option<&str>) -> (Option<i64>, String) {
    let Some(text) = text else {
        return (None, String::new());
    };
    // <think>…</think> entfernen (case-insensitive)
    let lower = text.to_lowercase();
    let cleaned = match (lower.find("<think>"), lower.find("</think>")) {
        (Some(start), Some(end)) if end > start => {
            format!("{}{}", &text[..start], &text[end + "</think>".len()..])
        }
        _ => text.to_string(),
    };
    // erstes minimales {…}
    let Some(open) = cleaned.find('{') else {
        return (None, String::new());
    };
    let Some(close_rel) = cleaned[open..].find('}') else {
        return (None, String::new());
    };
    let candidate = &cleaned[open..=open + close_rel];
    let Ok(data) = serde_json::from_str::<Value>(candidate) else {
        return (None, String::new());
    };
    let reason: String = data
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .chars()
        .take(300)
        .collect();
    let score = data.get("score").and_then(|raw| match raw {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    });
    match score {
        Some(score) => (
            Some((dl_core::pyfloat::py_round(score, 0) as i64).clamp(0, 100)),
            reason,
        ),
        None => (None, reason),
    }
}

// ── Ports (testbar ohne Discord) ───────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct MemberLite {
    pub user_id: u64,
    pub name: String,
    pub global_name: Option<String>,
    pub nick: Option<String>,
    pub is_bot: bool,
}

impl MemberLite {
    pub fn display(&self) -> &str {
        self.global_name.as_deref().unwrap_or(&self.name)
    }
}

#[async_trait::async_trait]
pub trait GuildPort: Send + Sync {
    /// Alle Member der Guild (gechunkt) — None wenn Guild unbekannt.
    async fn members(&self, guild_id: u64) -> Option<Vec<MemberLite>>;
    /// Streamer-Rolle vergeben → Status-Notiz (Texte sind Vertrag).
    async fn grant_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> String;
}

#[async_trait::async_trait]
pub trait Notifier: Send + Sync {
    /// Embed (+ optionale Components) in den Ops-Kanal → (channel_id, message_id).
    async fn notify(&self, embed: Value, components: Option<Value>) -> Option<(u64, u64)>;
    /// Review-Nachricht finalisieren (Status-Feld anhängen, Buttons entfernen).
    async fn finalize_review(&self, channel_id: u64, message_id: u64, status: String, color: u32);
    /// Klartext-Antwort (für die !twitch_link_*-Kommandos).
    async fn send_text(&self, channel_id: u64, text: String);
}

/// AI-Bewertung — bis dl-ai (Phase 6) liefert der Default None → Heuristik.
#[async_trait::async_trait]
pub trait AiScorer: Send + Sync {
    async fn score(&self, login: &str, member: &MemberLite, ratio: f64) -> (Option<i64>, String);
}

pub struct NoAi;

#[async_trait::async_trait]
impl AiScorer for NoAi {
    async fn score(
        &self,
        _login: &str,
        _member: &MemberLite,
        _ratio: f64,
    ) -> (Option<i64>, String) {
        (None, String::new())
    }
}

// ── Persistenter Zustand ───────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct LinkState {
    path: PathBuf,
    pub processed: Map<String, Value>,
    pub pending: Map<String, Value>,
    /// Offene manuelle Verknüpfungs-Prompts (kein Auto-Match) — für Neustart-Restore.
    pub manual_pending: Map<String, Value>,
}

impl LinkState {
    pub fn load(path: PathBuf) -> Self {
        let (processed, pending, manual_pending) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .map(|data| {
                let get = |key: &str| {
                    data.get(key)
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default()
                };
                (get("processed"), get("pending"), get("manual_pending"))
            })
            .unwrap_or_default();
        Self {
            path,
            processed,
            pending,
            manual_pending,
        }
    }

    pub fn save(&self) {
        let payload = json!({
            "processed": self.processed,
            "pending": self.pending,
            "manual_pending": self.manual_pending,
        });
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, payload.to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    pub fn is_handled(&self, login: &str) -> bool {
        let login = login.to_lowercase();
        self.processed.contains_key(&login)
            || self
                .pending
                .values()
                .any(|p| p.get("login").and_then(Value::as_str) == Some(login.as_str()))
    }

    pub fn mark(&mut self, login: &str, status: &str, extra: Map<String, Value>) {
        let mut record = Map::new();
        record.insert("status".into(), json!(status));
        record.insert("at".into(), json!(chrono::Utc::now().to_rfc3339()));
        record.extend(extra);
        self.processed
            .insert(login.to_lowercase(), Value::Object(record));
    }
}

// ── Matcher ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MatcherConfig {
    pub enabled: bool,
    pub notify_channel_id: u64,
    pub role_id: u64,
    pub guild_id: u64,
    pub auto_threshold: i64,
    pub review_threshold: i64,
    pub fuzzy_floor: f64,
    pub scan_interval_hours: u64,
    pub state_path: PathBuf,
}

impl MatcherConfig {
    /// ENV-Namen wie das Original (+ STREAMER_LINK_STATE_PATH für den Pfad).
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let int = |key: &str, default: i64| {
            get(key)
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(default)
        };
        Self {
            enabled: get("STREAMER_LINK_ENABLED")
                .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(true),
            notify_channel_id: int(
                "STREAMER_LINK_NOTIFY_CHANNEL_ID",
                DEFAULT_NOTIFY_CHANNEL_ID as i64,
            ) as u64,
            role_id: int("STREAMER_ROLE_ID", DEFAULT_STREAMER_ROLE_ID as i64) as u64,
            guild_id: get("STREAMER_GUILD_ID")
                .or_else(|| get("MAIN_GUILD_ID"))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(DEFAULT_GUILD_ID),
            auto_threshold: int("STREAMER_LINK_AUTO_THRESHOLD", 90),
            review_threshold: int("STREAMER_LINK_REVIEW_THRESHOLD", 70),
            fuzzy_floor: get("STREAMER_LINK_FUZZY_FLOOR")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.62),
            scan_interval_hours: int("STREAMER_LINK_SCAN_INTERVAL_HOURS", 6).max(0) as u64,
            state_path: PathBuf::from(
                get("STREAMER_LINK_STATE_PATH")
                    .unwrap_or_else(|| "data/streamer_link_state.json".to_string()),
            ),
        }
    }
}

type ExactIndex<'m> = HashMap<String, Vec<&'m MemberLite>>;
type BucketIndex<'m> = HashMap<String, Vec<(String, &'m MemberLite)>>;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanStats {
    pub checked: u64,
    pub auto: u64,
    pub review: u64,
    pub skipped: u64,
    pub errors: u64,
    /// Alle in diesem Lauf erstmals geprüften Logins (für das Summary-Embed).
    pub new_logins: Vec<String>,
}

pub struct Matcher {
    pub config: MatcherConfig,
    pub client: Arc<TwitchApiClient>,
    pub guild: Arc<dyn GuildPort>,
    pub notifier: Arc<dyn Notifier>,
    pub scorer: Arc<dyn AiScorer>,
    pub state: tokio::sync::Mutex<LinkState>,
    scan_lock: tokio::sync::Mutex<()>,
}

impl Matcher {
    pub fn new(
        config: MatcherConfig,
        client: Arc<TwitchApiClient>,
        guild: Arc<dyn GuildPort>,
        notifier: Arc<dyn Notifier>,
        scorer: Arc<dyn AiScorer>,
    ) -> Arc<Self> {
        let state = LinkState::load(config.state_path.clone());
        Arc::new(Self {
            config,
            client,
            guild,
            notifier,
            scorer,
            state: tokio::sync::Mutex::new(state),
            scan_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn scan_running(&self) -> bool {
        self.scan_lock.try_lock().is_err()
    }

    /// Member-Index: exakte Schlüssel + 2-Zeichen-Buckets fürs Fuzzy-Matching.
    fn build_index(members: &[MemberLite]) -> (ExactIndex<'_>, BucketIndex<'_>) {
        let mut exact: HashMap<String, Vec<&MemberLite>> = HashMap::new();
        let mut bucket: HashMap<String, Vec<(String, &MemberLite)>> = HashMap::new();
        for member in members.iter().filter(|m| !m.is_bot) {
            let mut keys: Vec<String> = vec![
                norm_key(&member.name),
                norm_key(member.global_name.as_deref().unwrap_or("")),
                norm_key(member.nick.as_deref().unwrap_or("")),
            ];
            keys.sort();
            keys.dedup();
            for key in keys.into_iter().filter(|k| !k.is_empty()) {
                let prefix: String = key.chars().take(2).collect();
                exact.entry(key.clone()).or_default().push(member);
                bucket.entry(prefix).or_default().push((key, member));
            }
        }
        (exact, bucket)
    }

    fn best_member<'m>(
        login_key: &str,
        exact: &ExactIndex<'m>,
        bucket: &BucketIndex<'m>,
    ) -> (Option<&'m MemberLite>, f64, bool) {
        if let Some(members) = exact.get(login_key) {
            return (members.first().copied(), 1.0, members.len() == 1);
        }
        let prefix: String = login_key.chars().take(2).collect();
        let mut best: (Option<&MemberLite>, f64) = (None, 0.0);
        if let Some(candidates) = bucket.get(&prefix) {
            for (key, member) in candidates {
                let ratio = similarity(login_key, key);
                if ratio > best.1 {
                    best = (Some(member), ratio);
                }
            }
        }
        (best.0, best.1, false)
    }

    pub async fn run_scan(self: &Arc<Self>, trigger: &str) -> ScanStats {
        let _guard = self.scan_lock.lock().await;
        let mut stats = ScanStats::default();
        if !self.config.enabled {
            return stats;
        }

        let Some(members) = self.guild.members(self.config.guild_id).await else {
            self.notifier
                .notify(
                    error_embed("Keine Guild gefunden – Abgleich abgebrochen."),
                    None,
                )
                .await;
            return stats;
        };

        let candidates = match self.client.link_candidates().await {
            Ok(candidates) => candidates,
            Err(err) => {
                tracing::error!(%err, "Matcher: Kandidaten-Abruf fehlgeschlagen");
                self.notifier
                    .notify(
                        error_embed("Konnte unverknüpfte Streamer nicht laden (interne API)."),
                        None,
                    )
                    .await;
                stats.errors += 1;
                return stats;
            }
        };

        let (exact, bucket) = Self::build_index(&members);
        let mut used_member_ids: std::collections::HashSet<u64> = std::collections::HashSet::new();

        for entry in &candidates {
            let login = entry
                .get("twitch_login")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            if login.is_empty() {
                continue;
            }
            {
                let state = self.state.lock().await;
                if state.is_handled(&login) {
                    continue;
                }
            }
            stats.checked += 1;
            stats.new_logins.push(login.clone());
            let is_monitored = entry
                .get("is_monitored_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let login_key = norm_key(&login);
            if login_key.is_empty() {
                self.mark(&login, "no_match", json!({"reason": "leerer Schlüssel"}))
                    .await;
                stats.skipped += 1;
                continue;
            }

            let (member, ratio, exact_unique) = Self::best_member(&login_key, &exact, &bucket);
            let Some(member) = member.filter(|_| ratio >= self.config.fuzzy_floor) else {
                self.mark(
                    &login,
                    "no_match",
                    json!({"reason": format!("kein Member (beste Ähnlichkeit {ratio:.2})")}),
                )
                .await;
                stats.skipped += 1;
                self.post_manual_link_prompt(&login).await;
                continue;
            };
            if used_member_ids.contains(&member.user_id) {
                self.mark(
                    &login,
                    "no_match",
                    json!({"reason": "Member-Kollision im Lauf"}),
                )
                .await;
                stats.skipped += 1;
                continue;
            }

            let (ai_score, ai_reason) = self.scorer.score(&login, member, ratio).await;
            let (score, reason) = match ai_score {
                Some(score) => (score, ai_reason),
                None => (
                    fallback_score(ratio, exact_unique),
                    if ai_reason.is_empty() {
                        format!("Heuristik (Ähnlichkeit {ratio:.2})")
                    } else {
                        ai_reason
                    },
                ),
            };

            let can_auto = score >= self.config.auto_threshold && !is_monitored;
            if can_auto {
                if self.auto_link(&login, member, score, &reason).await {
                    used_member_ids.insert(member.user_id);
                    stats.auto += 1;
                } else {
                    stats.errors += 1;
                }
            } else if score >= self.config.review_threshold {
                self.post_review(&login, entry, member, score, &reason, is_monitored)
                    .await;
                used_member_ids.insert(member.user_id);
                stats.review += 1;
            } else {
                self.mark(
                    &login,
                    "no_match",
                    json!({"reason": format!("Score {score} < {}", self.config.review_threshold)}),
                )
                .await;
                stats.skipped += 1;
                self.post_manual_link_prompt(&login).await;
            }
        }

        {
            let state = self.state.lock().await;
            state.save();
        }
        self.notifier
            .notify(summary_embed(&stats, trigger), None)
            .await;
        stats
    }

    async fn mark(&self, login: &str, status: &str, extra: Value) {
        let mut state = self.state.lock().await;
        state.mark(
            login,
            status,
            extra.as_object().cloned().unwrap_or_default(),
        );
    }

    async fn auto_link(&self, login: &str, member: &MemberLite, score: i64, reason: &str) -> bool {
        if let Err(err) = self
            .client
            .link_discord_profile(login, member.user_id, member.display())
            .await
        {
            tracing::error!(%err, login, "Matcher: Auto-Link DB-Write fehlgeschlagen");
            self.notifier
                .notify(
                    error_embed(&format!(
                        "Auto-Link für **{login}** → <@{}> fehlgeschlagen: `{}`",
                        member.user_id,
                        err.to_string().chars().take(160).collect::<String>()
                    )),
                    None,
                )
                .await;
            return false;
        }
        let role_note = self
            .guild
            .grant_role(self.config.guild_id, member.user_id, self.config.role_id)
            .await;
        self.mark(
            login,
            "auto_linked",
            json!({"discord_user_id": member.user_id.to_string(), "score": score}),
        )
        .await;
        let embed = json!({
            "title": "✅ Auto-verknüpft",
            "description": format!(
                "**Twitch:** `{login}`\n**Discord:** <@{}> (`{}`)\n**Wahrscheinlichkeit:** {score}%\n**Grund:** {reason}\n{role_note}",
                member.user_id, member.name
            ),
            "color": 0x2ECC71,
        });
        self.notifier.notify(embed, None).await;
        true
    }

    async fn post_review(
        &self,
        login: &str,
        entry: &Value,
        member: &MemberLite,
        score: i64,
        reason: &str,
        monitored: bool,
    ) {
        let token = hex::encode(rand::random::<[u8; 8]>());
        let note = if monitored {
            "\n*(nur überwachter Kanal – nie automatisch)*"
        } else {
            ""
        };
        let embed = json!({
            "title": "❓ Möglicher Streamer-Match",
            "description": format!(
                "**Twitch:** `{login}`\n**Discord:** <@{}> (`{}`)\n**Wahrscheinlichkeit:** {score}%\n**Grund:** {reason}{note}",
                member.user_id, member.name
            ),
            "color": 0xF1C40F,
        });
        let components = json!([{ "type": 1, "components": [
            { "type": 2, "style": 3, "label": "Verknüpfen", "custom_id": format!("slm:link:{token}") },
            { "type": 2, "style": 4, "label": "Ablehnen", "custom_id": format!("slm:reject:{token}") },
        ]}]);
        let posted = self.notifier.notify(embed, Some(components)).await;

        let mut record = Map::new();
        record.insert("login".into(), json!(login));
        record.insert(
            "twitch_user_id".into(),
            json!(entry
                .get("twitch_user_id")
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default()),
        );
        record.insert("discord_user_id".into(), json!(member.user_id.to_string()));
        record.insert(
            "discord_display_name".into(),
            json!(member.display().to_string()),
        );
        record.insert("score".into(), json!(score));
        record.insert("reason".into(), json!(reason));
        record.insert(
            "message_id".into(),
            posted.map(|(_, m)| json!(m)).unwrap_or(Value::Null),
        );
        record.insert(
            "channel_id".into(),
            posted.map(|(c, _)| json!(c)).unwrap_or(Value::Null),
        );
        let mut state = self.state.lock().await;
        state.pending.insert(token, Value::Object(record));
    }

    /// Review-Button bestätigt/abgelehnt (slm:link:* / slm:reject:*).
    pub async fn confirm_pending(
        &self,
        token: &str,
        approve: bool,
        moderator: &str,
    ) -> BridgeReply {
        let record = {
            let state = self.state.lock().await;
            state.pending.get(token).cloned()
        };
        let Some(record) = record else {
            return BridgeReply::ephemeral_text("Dieser Vorschlag ist nicht mehr offen.");
        };
        let login = record
            .get("login")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let channel_id = record.get("channel_id").and_then(Value::as_u64);
        let message_id = record.get("message_id").and_then(Value::as_u64);

        if !approve {
            {
                let mut state = self.state.lock().await;
                state.pending.remove(token);
                state.mark(
                    &login,
                    "rejected",
                    json!({"by": moderator})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                );
                state.save();
            }
            if let (Some(channel_id), Some(message_id)) = (channel_id, message_id) {
                self.notifier
                    .finalize_review(
                        channel_id,
                        message_id,
                        format!("❌ Abgelehnt von {moderator} – **{login}**"),
                        0x95A5A6,
                    )
                    .await;
            }
            return BridgeReply::ephemeral_text(format!("Abgelehnt: {login}"));
        }

        let user_id = record
            .get("discord_user_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let display_name = record
            .get("discord_display_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Err(err) = self
            .client
            .link_discord_profile(&login, user_id, &display_name)
            .await
        {
            return BridgeReply::ephemeral_text(format!(
                "DB-Write fehlgeschlagen: `{}`",
                err.to_string().chars().take(160).collect::<String>()
            ));
        }
        let role_note = self
            .guild
            .grant_role(self.config.guild_id, user_id, self.config.role_id)
            .await;
        {
            let mut state = self.state.lock().await;
            state.pending.remove(token);
            state.mark(
                &login,
                "linked",
                json!({"discord_user_id": user_id.to_string(), "by": moderator})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            );
            state.save();
        }
        if let (Some(channel_id), Some(message_id)) = (channel_id, message_id) {
            self.notifier
                .finalize_review(
                    channel_id,
                    message_id,
                    format!(
                        "✅ Bestätigt von {moderator} – **{login}** → <@{user_id}>. {role_note}"
                    ),
                    0x2ECC71,
                )
                .await;
        }
        BridgeReply::ephemeral_text(format!("Verknüpft: {login}"))
    }

    async fn post_manual_link_prompt(self: &Arc<Self>, login: &str) {
        let embed = json!({
            "title": "🔗 Kein Discord-Match",
            "description": format!(
                "**Twitch:** `{login}`\nKein Discord-Account automatisch gefunden.\nDiscord-Name oder numerische ID eingeben, um manuell zu verknüpfen."
            ),
            "color": 0xE67E22u32,
        });
        let components = json!([{ "type": 1, "components": [{
            "type": 2,
            "style": 1,
            "label": "Discord eingeben",
            "custom_id": format!("slm:manual:{login}"),
        }]}]);
        let posted = self.notifier.notify(embed, Some(components)).await;
        let (channel_val, message_val) = match posted {
            Some((ch, msg)) => (json!(ch), json!(msg)),
            None => (Value::Null, Value::Null),
        };
        let mut state = self.state.lock().await;
        state.manual_pending.insert(
            login.to_lowercase(),
            json!({"login": login, "message_id": message_val, "channel_id": channel_val}),
        );
    }

    /// Modaleingabe verarbeiten: Member via Name oder ID suchen, Profil verknüpfen.
    pub async fn handle_manual_submit(
        self: &Arc<Self>,
        login: &str,
        interaction: &BridgeInteraction,
    ) -> BridgeReply {
        if !interaction.author_can_manage_roles {
            return BridgeReply::ephemeral_text("Nur Mods mit Rollen-Rechten.");
        }
        let value = interaction
            .options
            .get("discord_input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();

        let (channel_id, message_id) = {
            let state = self.state.lock().await;
            let rec = state.manual_pending.get(&login.to_lowercase());
            (
                rec.and_then(|r| r.get("channel_id"))
                    .and_then(Value::as_u64),
                rec.and_then(|r| r.get("message_id"))
                    .and_then(Value::as_u64),
            )
        };

        let Some(members) = self.guild.members(self.config.guild_id).await else {
            return BridgeReply::ephemeral_text("Guild nicht gefunden.");
        };

        let member = if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
            let id: u64 = value.parse().unwrap_or(0);
            members.iter().find(|m| m.user_id == id).cloned()
        } else {
            let vl = value.to_lowercase();
            members
                .iter()
                .find(|m| {
                    !m.is_bot
                        && (m.name.to_lowercase() == vl
                            || m.global_name
                                .as_deref()
                                .map(|n| n.to_lowercase() == vl)
                                .unwrap_or(false)
                            || m.nick
                                .as_deref()
                                .map(|n| n.to_lowercase() == vl)
                                .unwrap_or(false))
                })
                .cloned()
        };

        let Some(member) = member else {
            return BridgeReply::ephemeral_text(format!(
                "Kein Member für `{value}` gefunden. Bitte numerische Discord-ID eingeben."
            ));
        };

        let display = member.display().to_string();
        let user_id = member.user_id;

        if let Err(err) = self
            .client
            .link_discord_profile(login, user_id, &display)
            .await
        {
            return BridgeReply::ephemeral_text(format!(
                "DB-Write fehlgeschlagen: `{}`",
                err.to_string().chars().take(160).collect::<String>()
            ));
        }

        let role_note = self
            .guild
            .grant_role(self.config.guild_id, user_id, self.config.role_id)
            .await;
        {
            let mut state = self.state.lock().await;
            state.processed.remove(&login.to_lowercase());
            state.mark(
                login,
                "linked",
                json!({"discord_user_id": user_id.to_string(), "by": interaction.author_name})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            );
            state.manual_pending.remove(&login.to_lowercase());
            state.save();
        }

        if let (Some(channel_id), Some(message_id)) = (channel_id, message_id) {
            self.notifier
                .finalize_review(
                    channel_id,
                    message_id,
                    format!(
                        "Manuell verknüpft von {} → <@{user_id}>. {role_note}",
                        interaction.author_name
                    ),
                    0x2ECC71,
                )
                .await;
        }

        BridgeReply::ephemeral_text(format!(
            "✅ **{login}** → <@{user_id}> verknüpft. {role_note}"
        ))
    }
}

fn summary_embed(stats: &ScanStats, trigger: &str) -> Value {
    let logins_text = if stats.new_logins.is_empty() {
        String::new()
    } else {
        let shown: Vec<&str> = stats
            .new_logins
            .iter()
            .map(String::as_str)
            .take(10)
            .collect();
        let rest = stats.new_logins.len().saturating_sub(10);
        let names = shown
            .iter()
            .map(|l| format!("`{l}`"))
            .collect::<Vec<_>>()
            .join(", ");
        if rest > 0 {
            format!("\n**Neu:** {names} +{rest} weitere")
        } else {
            format!("\n**Neu:** {names}")
        }
    };
    json!({
        "title": "📊 Streamer-Abgleich gelaufen",
        "description": format!(
            "**Auslöser:** {trigger}\n**Geprüft:** {}\n**Auto-verknüpft:** {}\n**Vorschläge:** {}\n**Ohne Treffer:** {}\n**Fehler:** {}{}",
            stats.checked, stats.auto, stats.review, stats.skipped, stats.errors, logins_text
        ),
        "color": 0x3498DB,
    })
}

fn error_embed(text: &str) -> Value {
    json!({ "title": "⚠️ Streamer-Abgleich", "description": text, "color": 0xE74C3C })
}

// ── Router-Anbindung + Listener + Loop ─────────────────────────────────────

struct ReviewHandler {
    matcher: Arc<Matcher>,
}

#[async_trait::async_trait]
impl InteractionHandler for ReviewHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if !interaction.author_can_manage_roles {
            return BridgeReply::ephemeral_text(
                "Nur Mods mit Rollen-Rechten können das bestätigen.",
            );
        }
        let rest = interaction
            .custom_id
            .strip_prefix(REVIEW_PREFIX)
            .unwrap_or_default();
        let (action, token) = rest.split_once(':').unwrap_or(("", ""));
        match action {
            "link" => {
                self.matcher
                    .confirm_pending(token, true, &interaction.author_name)
                    .await
            }
            "reject" => {
                self.matcher
                    .confirm_pending(token, false, &interaction.author_name)
                    .await
            }
            "manual" => {
                // token = login; Button öffnet Modal
                BridgeReply {
                    modal: Some(ModalSpec {
                        custom_id: format!("slm:manual_submit:{token}"),
                        title: "Discord-Account verknüpfen".to_string(),
                        fields: vec![ModalField {
                            custom_id: "discord_input".to_string(),
                            label: "Discord-Name oder ID".to_string(),
                            placeholder: "z.B. username oder 123456789012345678".to_string(),
                            required: true,
                            min_length: 2,
                            max_length: 100,
                            paragraph: false,
                        }],
                    }),
                    ..BridgeReply::default()
                }
            }
            "manual_submit" => {
                // token = login; Modaleingabe verarbeiten
                self.matcher.handle_manual_submit(token, &interaction).await
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}

pub fn register(router: &mut InteractionRouter, matcher: Arc<Matcher>) {
    router.on_prefix(REVIEW_PREFIX, Arc::new(ReviewHandler { matcher }));
}

/// 6h-Loop (erste Iteration wird übersprungen — Backfill nur manuell).
pub fn spawn_scan_loop(matcher: Arc<Matcher>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let hours = matcher.config.scan_interval_hours;
        if !matcher.config.enabled || hours == 0 {
            return;
        }
        let interval = std::time::Duration::from_secs(hours * 3600);
        loop {
            tokio::time::sleep(interval).await;
            let _ = matcher.run_scan("Auto-Scan (neue Streamer)").await;
        }
    })
}

/// !twitch_link_scan + !twitch_link_rescan_login (Admin, via Message-Listener).
pub fn spawn_command_listener(
    dispatcher: &dl_discord::Dispatcher,
    matcher: Arc<Matcher>,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let content = event.content.trim();
            if !content.starts_with("!twitch_link_") || !event.author_is_admin {
                continue;
            }
            let mut parts = content.split_whitespace();
            match parts.next() {
                Some("!twitch_link_scan") => {
                    if !matcher.config.enabled {
                        matcher
                            .notifier
                            .send_text(
                                event.channel_id,
                                "Matcher ist inaktiv (kein interner API-Token?).".to_string(),
                            )
                            .await;
                        continue;
                    }
                    if matcher.scan_running() {
                        matcher
                            .notifier
                            .send_text(
                                event.channel_id,
                                "Es läuft bereits ein Abgleich.".to_string(),
                            )
                            .await;
                        continue;
                    }
                    matcher
                        .notifier
                        .send_text(
                            event.channel_id,
                            "Starte vollständigen Streamer-Abgleich … Ergebnisse landen im Ops-Kanal."
                                .to_string(),
                        )
                        .await;
                    let stats = matcher
                        .run_scan(&format!("Manuell ({})", event.author_display_name))
                        .await;
                    matcher
                        .notifier
                        .send_text(
                            event.channel_id,
                            format!(
                                "Fertig: {} auto, {} Vorschläge, {} ohne Treffer, {} Fehler.",
                                stats.auto, stats.review, stats.skipped, stats.errors
                            ),
                        )
                        .await;
                }
                Some("!twitch_link_rescan_login") => {
                    let Some(login) = parts.next() else { continue };
                    let key = login.trim().to_lowercase();
                    {
                        let mut state = matcher.state.lock().await;
                        state.processed.remove(&key);
                        let stale: Vec<String> = state
                            .pending
                            .iter()
                            .filter(|(_, rec)| {
                                rec.get("login").and_then(Value::as_str) == Some(key.as_str())
                            })
                            .map(|(token, _)| token.clone())
                            .collect();
                        for token in stale {
                            state.pending.remove(&token);
                        }
                        state.save();
                    }
                    matcher
                        .notifier
                        .send_text(
                            event.channel_id,
                            format!("`{key}` ist wieder offen für den nächsten Abgleich."),
                        )
                        .await;
                }
                _ => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Referenzwerte aus CPython (cogs/twitch/streamer_link_matcher.py)
    #[test]
    fn norm_key_wie_python() {
        assert_eq!(norm_key("DragSkope"), "dragskope");
        assert_eq!(norm_key("drag_skope | TTV"), "dragskope");
        assert_eq!(norm_key("Müller LIVE"), "muller");
        assert_eq!(norm_key("N4ni"), "nani");
        assert_eq!(norm_key("xX_n0ob_Xx"), "xxnoobxx");
        assert_eq!(norm_key("ttv"), "ttv"); // nur Affixe → raw-Fallback
        assert_eq!(norm_key("🔥Flame🔥"), "flame");
    }

    #[test]
    fn similarity_wie_difflib() {
        assert_eq!(similarity("dragskope", "dragskope"), 1.0);
        assert!((similarity("dragskope", "dragscope") - 0.888_888_888_888_888_8).abs() < 1e-12);
        assert!((similarity("nani", "nani2003") - 0.666_666_666_666_666_6).abs() < 1e-12);
        assert!((similarity("gabelogan", "newell") - 0.266_666_666_666_666_66).abs() < 1e-12);
        assert!((similarity("abcdef", "abdcef") - 0.833_333_333_333_333_4).abs() < 1e-12);
        assert_eq!(similarity("", "x"), 0.0);
    }

    #[test]
    fn fallback_scores_wie_python() {
        assert_eq!(fallback_score(1.0, true), 92);
        assert_eq!(fallback_score(1.0, false), 80);
        assert_eq!(fallback_score(0.95, false), 80);
        assert_eq!(fallback_score(0.85, false), 72);
        assert_eq!(fallback_score(0.5, false), 35);
    }

    #[test]
    fn ai_score_parsing_wie_python() {
        assert_eq!(
            parse_ai_score(Some(
                "<think>blah</think> {\"score\": 87.6, \"reason\": \"passt\"}"
            )),
            (Some(88), "passt".to_string())
        );
        assert_eq!(parse_ai_score(Some("kein json")), (None, String::new()));
        assert_eq!(
            parse_ai_score(Some("{\"score\": 150, \"reason\": \"x\"}")),
            (Some(100), "x".to_string())
        );
        assert_eq!(
            parse_ai_score(Some("{\"score\": \"nope\", \"reason\": \"y\"}")),
            (None, "y".to_string())
        );
        assert_eq!(parse_ai_score(None), (None, String::new()));
    }

    #[test]
    fn state_roundtrip_und_handled() {
        let dir = std::env::temp_dir().join(format!("slm-test-{}", std::process::id()));
        let path = dir.join("state.json");
        let mut state = LinkState::load(path.clone());
        assert!(!state.is_handled("nani"));
        state.mark("Nani", "auto_linked", Map::new());
        state
            .pending
            .insert("tok1".into(), json!({"login": "other"}));
        state.save();

        let reloaded = LinkState::load(path);
        assert!(reloaded.is_handled("NANI"));
        assert!(reloaded.is_handled("other")); // pending zählt als handled
        assert!(!reloaded.is_handled("dritter"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
