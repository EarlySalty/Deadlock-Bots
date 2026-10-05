//! Verknüpft Twitch-Streamer nach einem kostenlosen Namensvergleich mit Discord.
//!
//! Gleicht unverknüpfte Twitch-Streamer gegen die Discord-Memberliste ab:
//! Namens-Normalisierung + Fuzzy-Match (difflib-Algorithmus nachgebaut),
//! Score-Entscheidung Auto-Link / Review-Vorschlag (Buttons) / kein Treffer.
//!
//! Nur eindeutige exakte Treffer werden automatisch verknüpft. Ähnliche Namen
//! benötigen eine Bestätigung. Automatische Versuche werden nach Twitch-ID gespeichert.

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
    fn is_available(&self) -> bool {
        true
    }

    async fn score(&self, login: &str, member: &MemberLite, ratio: f64) -> (Option<i64>, String);
}

pub struct NoAi;

#[async_trait::async_trait]
impl AiScorer for NoAi {
    fn is_available(&self) -> bool {
        false
    }

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
    /// Dauerhaft abgeschlossene automatische Versuche nach Twitch-ID.
    pub attempted_ids: Map<String, Value>,
    load_error: Option<String>,
    pub pending: Map<String, Value>,
    /// Offene manuelle Verknüpfungs-Prompts (kein Auto-Match) — für Neustart-Restore.
    pub manual_pending: Map<String, Value>,
}

impl LinkState {
    pub fn load(path: PathBuf) -> Self {
        let loaded = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str::<Value>(&raw)
                .map_err(|err| err.to_string())
                .and_then(|data| {
                    if !data.is_object()
                        || ["processed", "pending", "manual_pending", "attempted_ids"]
                            .iter()
                            .any(|key| data.get(key).is_some_and(|value| !value.is_object()))
                    {
                        Err("Ungültiges Format des Matcher-Zustands".to_string())
                    } else {
                        Ok(data)
                    }
                }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
            Err(err) => Err(err.to_string()),
        };
        let load_error = loaded.as_ref().err().cloned();
        let data = loaded.unwrap_or(Value::Null);
        let get = |key: &str| {
            data.get(key)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
        };
        Self {
            path,
            processed: get("processed"),
            attempted_ids: get("attempted_ids"),
            pending: get("pending"),
            manual_pending: get("manual_pending"),
            load_error,
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(error) = &self.load_error {
            return Err(std::io::Error::other(format!(
                "Matcher-Zustand konnte nicht geladen werden: {error}"
            )));
        }
        let payload = json!({
            "processed": self.processed,
            "attempted_ids": self.attempted_ids,
            "pending": self.pending,
            "manual_pending": self.manual_pending,
        });
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(parent)?;
        let tmp = self.path.with_extension("tmp");
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(payload.to_string().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, &self.path)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    fn claim(&mut self, twitch_id: &str, login: &str) -> std::io::Result<bool> {
        if let Some(record) = self.attempted_ids.get(twitch_id) {
            return Ok(matches!(
                record.get("status").and_then(Value::as_str),
                Some("retry_auto" | "evaluating")
            ));
        }
        let old_record = self
            .processed
            .get(login)
            .cloned()
            .or_else(|| self.manual_pending.get(login).cloned())
            .or_else(|| {
                self.pending
                    .values()
                    .find(|record| {
                        record.get("twitch_user_id").and_then(Value::as_str) == Some(twitch_id)
                            || record.get("login").and_then(Value::as_str) == Some(login)
                    })
                    .cloned()
            });
        let inherited = old_record.is_some();
        self.attempted_ids.insert(
            twitch_id.to_string(),
            json!({
                "login": login,
                "at": chrono::Utc::now().to_rfc3339(),
                "status": if inherited { "inherited" } else { "evaluating" },
                "evidence": old_record,
            }),
        );
        if let Err(err) = self.save() {
            self.attempted_ids.remove(twitch_id);
            return Err(err);
        }
        Ok(!inherited)
    }

    pub fn is_handled(&self, login: &str) -> bool {
        let login = login.to_lowercase();
        self.processed.contains_key(&login)
            || self.manual_pending.contains_key(&login)
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
        for attempt in self.attempted_ids.values_mut() {
            if attempt.get("login").and_then(Value::as_str) == Some(login.to_lowercase().as_str()) {
                attempt["status"] = json!(status);
            }
        }
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
    pub max_ai_per_scan: u64,
    pub scan_interval_hours: u64,
    pub state_path: PathBuf,
    pub ai_provider: String,
}

impl MatcherConfig {
    /// Bestehende Betriebsschlüssel aus der normalen Konfiguration.
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
            max_ai_per_scan: int("STREAMER_LINK_MAX_AI_PER_SCAN", 40).max(0) as u64,
            scan_interval_hours: int("STREAMER_LINK_SCAN_INTERVAL_HOURS", 6).max(0) as u64,
            state_path: PathBuf::from(
                get("STREAMER_LINK_STATE_PATH")
                    .unwrap_or_else(|| "data/streamer_link_state.json".to_string()),
            ),
            ai_provider: get("STREAMER_LINK_AI_PROVIDER")
                .unwrap_or_else(|| "minimax".to_string())
                .to_ascii_lowercase(),
        }
    }
}

type ExactIndex<'m> = HashMap<String, Vec<&'m MemberLite>>;
type BucketIndex<'m> = HashMap<String, Vec<(String, &'m MemberLite)>>;

/// Was der Scan mit einem erstmals gesehenen Streamer gemacht hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    AutoLinked { discord_user_id: u64 },
    Review { discord_user_id: u64 },
    NoMatch,
    Failed,
}

/// Ein in diesem Lauf erstmals geprüfter Streamer samt Ergebnis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEntryResult {
    pub login: String,
    pub outcome: ScanOutcome,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanStats {
    pub checked: u64,
    pub auto: u64,
    pub review: u64,
    pub skipped: u64,
    pub errors: u64,
    pub ai_calls: u64,
    /// Alle in diesem Lauf erstmals geprüften Streamer mit Ausgang (Summary-Embed).
    pub new_streamers: Vec<ScanEntryResult>,
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
        self.run_scan_inner(trigger, false).await
    }

    /// Wie [`Self::run_scan`], schweigt aber bei einem Lauf ohne neuen Streamer
    /// und ohne Fehler (der 6h-Loop soll den Kanal nicht mit Nullen fluten).
    pub async fn run_scan_quiet(self: &Arc<Self>, trigger: &str) -> ScanStats {
        self.run_scan_inner(trigger, true).await
    }

    async fn run_scan_inner(self: &Arc<Self>, trigger: &str, quiet_when_empty: bool) -> ScanStats {
        let _guard = self.scan_lock.lock().await;
        let mut stats = ScanStats::default();
        if !self.config.enabled {
            return stats;
        }
        if let Err(err) = self.state.lock().await.save() {
            tracing::error!(%err, "Matcher: Speicherprüfung fehlgeschlagen, Abgleich gesperrt");
            stats.errors += 1;
            return stats;
        }

        let Some(members) = self.guild.members(self.config.guild_id).await else {
            stats.errors += 1;
            self.notifier
                .notify(
                    error_embed("Keine Guild gefunden – Abgleich abgebrochen."),
                    None,
                )
                .await;
            return stats;
        };

        if members.is_empty() {
            tracing::error!("Matcher: Mitgliederbestand ist leer, Abgleich gesperrt");
            stats.errors += 1;
            return stats;
        }

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

        self.deliver_saved_prompts().await;
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
            let twitch_id = entry.get("twitch_user_id").and_then(|value| match value {
                Value::String(id)
                    if !id.is_empty()
                        && id.chars().all(|c| c.is_ascii_digit())
                        && id.parse::<u64>().ok().is_some_and(|id| id > 0) =>
                {
                    Some(id.clone())
                }
                Value::Number(id) => id.as_u64().filter(|id| *id > 0).map(|id| id.to_string()),
                _ => None,
            });
            let Some(twitch_id) = twitch_id else {
                tracing::error!(login, "Matcher: Twitch-ID fehlt oder ist ungültig");
                stats.errors += 1;
                continue;
            };
            match self.state.lock().await.claim(&twitch_id, &login) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(err) => {
                    tracing::error!(%err, "Matcher: dauerhafte Speicherung fehlgeschlagen, Abgleich beendet");
                    stats.errors += 1;
                    break;
                }
            }
            stats.checked += 1;
            let entry_idx = stats.new_streamers.len();
            stats.new_streamers.push(ScanEntryResult {
                login: login.clone(),
                outcome: ScanOutcome::NoMatch,
            });
            let is_monitored = entry
                .get("is_monitored_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let login_key = norm_key(&login);
            if login_key.is_empty() {
                if !self
                    .mark(&login, "no_match", json!({"reason": "leerer Schlüssel"}))
                    .await
                {
                    stats.errors += 1;
                    stats.new_streamers[entry_idx].outcome = ScanOutcome::Failed;
                    continue;
                }
                stats.skipped += 1;
                continue;
            }

            let (member, ratio, exact_unique) = Self::best_member(&login_key, &exact, &bucket);
            let Some(member) = member.filter(|_| ratio >= self.config.fuzzy_floor) else {
                if !self
                    .mark(
                        &login,
                        "no_match",
                        json!({"reason": format!("kein Member (beste Ähnlichkeit {ratio:.2})")}),
                    )
                    .await
                {
                    stats.errors += 1;
                    stats.new_streamers[entry_idx].outcome = ScanOutcome::Failed;
                    continue;
                }
                stats.skipped += 1;
                self.post_manual_link_prompt(&login).await;
                continue;
            };
            if used_member_ids.contains(&member.user_id) {
                if !self
                    .mark(
                        &login,
                        "no_match",
                        json!({"reason": "Member-Kollision im Lauf"}),
                    )
                    .await
                {
                    stats.errors += 1;
                    stats.new_streamers[entry_idx].outcome = ScanOutcome::Failed;
                    continue;
                }
                stats.skipped += 1;
                continue;
            }

            let score = fallback_score(ratio, exact_unique);
            let reason = format!("Namensvergleich (Ähnlichkeit {ratio:.2})");
            let can_auto = exact_unique && score >= self.config.auto_threshold && !is_monitored;
            if can_auto {
                {
                    let mut state = self.state.lock().await;
                    state
                        .attempted_ids
                        .get_mut(&twitch_id)
                        .expect("Versuch vorhanden")["status"] = json!("retry_auto");
                    if let Err(err) = state.save() {
                        tracing::error!(%err, "Matcher: Auto-Verknüpfung konnte nicht vorbereitet werden");
                        stats.errors += 1;
                        stats.new_streamers[entry_idx].outcome = ScanOutcome::Failed;
                        break;
                    }
                }
                if self.auto_link(&login, member, score, &reason).await {
                    used_member_ids.insert(member.user_id);
                    stats.auto += 1;
                    stats.new_streamers[entry_idx].outcome = ScanOutcome::AutoLinked {
                        discord_user_id: member.user_id,
                    };
                } else {
                    let mut state = self.state.lock().await;
                    state.attempted_ids.remove(&twitch_id);
                    if let Err(err) = state.save() {
                        tracing::error!(%err, "Matcher: technischer Fehler konnte nicht wieder geöffnet werden");
                    }
                    stats.errors += 1;
                    stats.new_streamers[entry_idx].outcome = ScanOutcome::Failed;
                }
            } else if score >= self.config.review_threshold {
                if !self
                    .post_review(&login, entry, member, score, &reason, is_monitored)
                    .await
                {
                    stats.errors += 1;
                    stats.new_streamers[entry_idx].outcome = ScanOutcome::Failed;
                    continue;
                }
                used_member_ids.insert(member.user_id);
                stats.review += 1;
                stats.new_streamers[entry_idx].outcome = ScanOutcome::Review {
                    discord_user_id: member.user_id,
                };
            } else {
                if !self.mark(
                    &login,
                    "no_match",
                    json!({"reason": format!("Score {score} < {}", self.config.review_threshold)}),
                )
                .await {
                    stats.errors += 1;
                    stats.new_streamers[entry_idx].outcome = ScanOutcome::Failed;
                    continue;
                }
                stats.skipped += 1;
                self.post_manual_link_prompt(&login).await;
            }
        }

        {
            let state = self.state.lock().await;
            if let Err(err) = state.save() {
                tracing::error!(%err, "Matcher: Zustand konnte nicht gespeichert werden");
            }
        }
        let nothing_happened = stats.new_streamers.is_empty() && stats.errors == 0;
        if !(quiet_when_empty && nothing_happened) {
            self.notifier
                .notify(summary_embed(&stats, trigger), None)
                .await;
        }
        stats
    }

    async fn deliver_saved_prompts(self: &Arc<Self>) {
        let (manual, reviews) = {
            let state = self.state.lock().await;
            let unsent =
                |record: &&Value| record.get("delivered").and_then(Value::as_bool) == Some(false);
            (
                state
                    .manual_pending
                    .values()
                    .filter(unsent)
                    .filter_map(|record| {
                        record
                            .get("login")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect::<Vec<_>>(),
                state
                    .pending
                    .iter()
                    .filter(|(_, record)| {
                        record.get("delivered").and_then(Value::as_bool) == Some(false)
                    })
                    .map(|(token, record)| (token.clone(), record.clone()))
                    .collect::<Vec<_>>(),
            )
        };
        for login in manual {
            self.post_manual_link_prompt(&login).await;
        }
        for (token, record) in reviews {
            let Some(embed) = record.get("embed").cloned() else {
                continue;
            };
            {
                let mut state = self.state.lock().await;
                if let Some(record) = state.pending.get_mut(&token) {
                    record["delivered"] = json!(true);
                }
                if let Err(err) = state.save() {
                    if let Some(record) = state.pending.get_mut(&token) {
                        record["delivered"] = json!(false);
                    }
                    tracing::error!(%err, "Matcher: Versand konnte nicht dauerhaft vorbereitet werden");
                    continue;
                }
            }
            let Some((channel_id, message_id)) = self
                .notifier
                .notify(embed, record.get("components").cloned())
                .await
            else {
                let mut state = self.state.lock().await;
                if let Some(record) = state.pending.get_mut(&token) {
                    record["delivered"] = json!(false);
                }
                if let Err(err) = state.save() {
                    tracing::error!(%err, "Matcher: fehlgeschlagener Versand konnte nicht gespeichert werden");
                }
                continue;
            };
            let mut state = self.state.lock().await;
            if let Some(record) = state.pending.get_mut(&token).and_then(Value::as_object_mut) {
                record.insert("delivered".into(), json!(true));
                record.insert("channel_id".into(), json!(channel_id));
                record.insert("message_id".into(), json!(message_id));
            }
            if let Err(err) = state.save() {
                tracing::error!(%err, "Matcher: gespeicherter Vorschlag konnte nicht finalisiert werden");
            }
        }
    }

    async fn mark(&self, login: &str, status: &str, extra: Value) -> bool {
        let mut state = self.state.lock().await;
        let previous_processed = state.processed.clone();
        let previous_attempts = state.attempted_ids.clone();
        let previous_manual = state.manual_pending.clone();
        state.mark(
            login,
            status,
            extra.as_object().cloned().unwrap_or_default(),
        );
        if status == "no_match" {
            state
                .manual_pending
                .entry(login.to_lowercase())
                .or_insert_with(|| json!({"login": login, "delivered": false}));
        }
        if let Err(err) = state.save() {
            state.processed = previous_processed;
            state.attempted_ids = previous_attempts;
            state.manual_pending = previous_manual;
            tracing::error!(%err, "Matcher: Ergebnis konnte nicht gespeichert werden");
            return false;
        }
        true
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
                "**Twitch:** `{login}`\n**Discord:** <@{}> (`{}`)\n**Namensvergleich:** {score} von 100\n**Grund:** {reason}\n{role_note}",
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
    ) -> bool {
        let token = hex::encode(rand::random::<[u8; 8]>());
        let note = if monitored {
            "\n*(nur überwachter Kanal – nie automatisch)*"
        } else {
            ""
        };
        let embed = json!({
            "title": "❓ Möglicher Streamer-Match",
            "description": format!(
                "**Twitch:** `{login}`\n**Discord:** <@{}> (`{}`)\n**Namensvergleich:** {score} von 100\n**Grund:** {reason}{note}",
                member.user_id, member.name
            ),
            "color": 0xF1C40F,
        });
        let components = json!([{ "type": 1, "components": [
            { "type": 2, "style": 3, "label": "Verknüpfen", "custom_id": format!("slm:link:{token}") },
            { "type": 2, "style": 4, "label": "Ablehnen", "custom_id": format!("slm:reject:{token}") },
        ]}]);
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
        record.insert("delivered".into(), json!(true));
        record.insert("embed".into(), embed.clone());
        record.insert("components".into(), components.clone());
        record.insert("message_id".into(), Value::Null);
        record.insert("channel_id".into(), Value::Null);
        {
            let mut state = self.state.lock().await;
            let previous_processed = state.processed.clone();
            let previous_attempts = state.attempted_ids.clone();
            state.pending.insert(token.clone(), Value::Object(record));
            state.mark(login, "review", Map::new());
            if let Err(err) = state.save() {
                state.pending.remove(&token);
                state.processed = previous_processed;
                state.attempted_ids = previous_attempts;
                tracing::error!(%err, "Matcher: Vorschlag konnte nicht gespeichert werden");
                return false;
            }
        }
        let posted = self.notifier.notify(embed, Some(components)).await;
        if posted.is_none() {
            let mut state = self.state.lock().await;
            if let Some(record) = state.pending.get_mut(&token) {
                record["delivered"] = json!(false);
            }
            if let Err(err) = state.save() {
                tracing::error!(%err, "Matcher: fehlgeschlagener Versand konnte nicht gespeichert werden");
            }
        }
        if let Some((channel_id, message_id)) = posted {
            let mut state = self.state.lock().await;
            if let Some(record) = state.pending.get_mut(&token).and_then(Value::as_object_mut) {
                record.insert("delivered".into(), json!(true));
                record.insert("channel_id".into(), json!(channel_id));
                record.insert("message_id".into(), json!(message_id));
            }
            if let Err(err) = state.save() {
                tracing::error!(%err, "Matcher: Nachrichtenadresse konnte nicht gespeichert werden");
            }
        }
        posted.is_some()
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
                if let Err(err) = state.save() {
                    tracing::error!(%err, "Matcher: Zustand konnte nicht gespeichert werden");
                }
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
            if let Err(err) = state.save() {
                tracing::error!(%err, "Matcher: Zustand konnte nicht gespeichert werden");
            }
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
        {
            let mut state = self.state.lock().await;
            state
                .manual_pending
                .entry(login.to_lowercase())
                .or_insert_with(|| json!({"login": login, "delivered": false}));
            state
                .manual_pending
                .get_mut(&login.to_lowercase())
                .expect("manuelle Eingabe vorhanden")["delivered"] = json!(true);
            if let Err(err) = state.save() {
                tracing::error!(%err, "Matcher: manuelle Eingabe konnte nicht gespeichert werden");
                return;
            }
        }
        let posted = self.notifier.notify(embed, Some(components)).await;
        let (channel_val, message_val) = match posted {
            Some((ch, msg)) => (json!(ch), json!(msg)),
            None => (Value::Null, Value::Null),
        };
        let mut state = self.state.lock().await;
        state.manual_pending.insert(
            login.to_lowercase(),
            json!({"login": login, "message_id": message_val, "channel_id": channel_val, "delivered": posted.is_some()}),
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
            let moderator = format!("<@{}>", interaction.user_id);
            state.mark(
                login,
                "linked",
                json!({"discord_user_id": user_id.to_string(), "by": moderator})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            );
            state.manual_pending.remove(&login.to_lowercase());
            if let Err(err) = state.save() {
                tracing::error!(%err, "Matcher: Zustand konnte nicht gespeichert werden");
            }
        }

        if let (Some(channel_id), Some(message_id)) = (channel_id, message_id) {
            let moderator = format!("<@{}>", interaction.user_id);
            self.notifier
                .finalize_review(
                    channel_id,
                    message_id,
                    format!(
                        "Manuell verknüpft von {} → <@{user_id}>. {role_note}",
                        moderator
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

/// Ein Zeile je neuem Streamer: wer es ist, wo er sendet, was der Scan entschied.
fn new_streamers_text(stats: &ScanStats) -> String {
    if stats.new_streamers.is_empty() {
        return String::new();
    }
    const MAX_SHOWN: usize = 10;
    let mut lines: Vec<String> = stats
        .new_streamers
        .iter()
        .take(MAX_SHOWN)
        .map(|entry| {
            let link = format!("[{}](https://twitch.tv/{})", entry.login, entry.login);
            match &entry.outcome {
                ScanOutcome::AutoLinked { discord_user_id } => {
                    format!("✅ {link} → <@{discord_user_id}>")
                }
                ScanOutcome::Review { discord_user_id } => {
                    format!("❓ {link} → <@{discord_user_id}> (Vorschlag, bitte bestätigen)")
                }
                ScanOutcome::NoMatch => format!("❌ {link} (kein Discord-Treffer)"),
                ScanOutcome::Failed => format!("⚠️ {link} (Verknüpfung fehlgeschlagen)"),
            }
        })
        .collect();
    let rest = stats.new_streamers.len().saturating_sub(MAX_SHOWN);
    if rest > 0 {
        lines.push(format!("… +{rest} weitere"));
    }
    format!("\n\n**Neue Streamer:**\n{}", lines.join("\n"))
}

fn summary_embed(stats: &ScanStats, trigger: &str) -> Value {
    let logins_text = new_streamers_text(stats);
    json!({
        "title": "📊 Streamer-Abgleich gelaufen",
        "description": format!(
            "**Auslöser:** {trigger}\n**Geprüft:** {}\n**Auto-verknüpft:** {}\n**Vorschläge:** {}\n**Ohne Treffer:** {}\n**AI-Aufrufe:** {}\n**Fehler:** {}{}",
            stats.checked,
            stats.auto,
            stats.review,
            stats.skipped,
            stats.ai_calls,
            stats.errors,
            logins_text
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
        let moderator = format!("<@{}>", interaction.user_id);
        let rest = interaction
            .custom_id
            .strip_prefix(REVIEW_PREFIX)
            .unwrap_or_default();
        let (action, token) = rest.split_once(':').unwrap_or(("", ""));
        match action {
            "link" => self.matcher.confirm_pending(token, true, &moderator).await,
            "reject" => self.matcher.confirm_pending(token, false, &moderator).await,
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
                            value: None,
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

/// Stiller Startscan und danach der konfigurierte regelmäßige Abgleich.
pub fn spawn_scan_loop(matcher: Arc<Matcher>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let hours = matcher.config.scan_interval_hours;
        if !matcher.config.enabled || hours == 0 {
            return;
        }
        tracing::info!(state_path = %matcher.config.state_path.display(), "Matcher: dauerhafter Zustand");
        let interval = std::time::Duration::from_secs(hours * 3600);
        loop {
            let stats = matcher.run_scan_quiet("Auto-Scan (neue Streamer)").await;
            tracing::info!(
                checked = stats.checked,
                auto = stats.auto,
                review = stats.review,
                skipped = stats.skipped,
                errors = stats.errors,
                ai_calls = stats.ai_calls,
                "Matcher: automatischer Abgleich abgeschlossen"
            );
            tokio::time::sleep(interval).await;
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
                    let _scan_guard = matcher.scan_lock.lock().await;
                    let key = login.trim().to_lowercase();
                    let candidates = match matcher.client.link_candidates().await {
                        Ok(entries) => entries,
                        Err(err) => {
                            tracing::error!(%err, "Matcher: erneuter Abgleich konnte nicht vorbereitet werden");
                            matcher.notifier.send_text(event.channel_id, "Der erneute Abgleich konnte nicht vorbereitet werden. Bitte später erneut versuchen.".to_string()).await;
                            continue;
                        }
                    };
                    let twitch_id = candidates
                        .iter()
                        .find(|entry| {
                            entry
                                .get("twitch_login")
                                .and_then(Value::as_str)
                                .is_some_and(|login| login.eq_ignore_ascii_case(&key))
                        })
                        .and_then(|entry| entry.get("twitch_user_id"))
                        .map(|id| {
                            id.as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| id.to_string())
                        });
                    {
                        let mut state = matcher.state.lock().await;
                        state.processed.remove(&key);
                        state.attempted_ids.retain(|_, record| {
                            record.get("login").and_then(Value::as_str) != Some(key.as_str())
                        });
                        if let Some(twitch_id) = &twitch_id {
                            state.attempted_ids.remove(twitch_id);
                        }
                        state.manual_pending.remove(&key);
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
                        if let Err(err) = state.save() {
                            tracing::error!(%err, "Matcher: Zustand konnte nicht gespeichert werden");
                            drop(state);
                            matcher
                                .notifier
                                .send_text(
                                    event.channel_id,
                                    "Der erneute Abgleich konnte nicht gespeichert werden."
                                        .to_string(),
                                )
                                .await;
                            continue;
                        }
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
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

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
    fn config_und_summary_ai_budget_wie_python() {
        let cfg = MatcherConfig::from_env(|key| match key {
            "STREAMER_LINK_MAX_AI_PER_SCAN" => Some("7".to_string()),
            "STREAMER_LINK_AI_PROVIDER" => Some("openai".to_string()),
            _ => None,
        });
        assert_eq!(cfg.max_ai_per_scan, 7);
        assert_eq!(cfg.ai_provider, "openai");
        let stats = ScanStats {
            checked: 3,
            ai_calls: 2,
            errors: 1,
            ..ScanStats::default()
        };
        let embed = summary_embed(&stats, "Test");
        let desc = embed["description"].as_str().expect("description");
        assert!(desc.contains("**AI-Aufrufe:** 2"));
        assert!(desc.contains("**Fehler:** 1"));
    }

    #[test]
    fn summary_nennt_neue_streamer_mit_namen_und_ausgang() {
        let stats = ScanStats {
            checked: 3,
            auto: 1,
            review: 1,
            skipped: 1,
            new_streamers: vec![
                ScanEntryResult {
                    login: "neuerstreamer".into(),
                    outcome: ScanOutcome::AutoLinked {
                        discord_user_id: 42,
                    },
                },
                ScanEntryResult {
                    login: "unklar".into(),
                    outcome: ScanOutcome::Review {
                        discord_user_id: 43,
                    },
                },
                ScanEntryResult {
                    login: "ohnetreffer".into(),
                    outcome: ScanOutcome::NoMatch,
                },
            ],
            ..ScanStats::default()
        };
        let embed = summary_embed(&stats, "Test");
        let desc = embed["description"].as_str().expect("description");
        assert!(desc.contains("**Neue Streamer:**"));
        assert!(desc.contains("✅ [neuerstreamer](https://twitch.tv/neuerstreamer) → <@42>"));
        assert!(desc.contains("❓ [unklar](https://twitch.tv/unklar) → <@43>"));
        assert!(desc.contains("❌ [ohnetreffer](https://twitch.tv/ohnetreffer)"));
    }

    #[test]
    fn summary_ohne_neue_streamer_bleibt_ohne_liste() {
        let embed = summary_embed(&ScanStats::default(), "Test");
        let desc = embed["description"].as_str().expect("description");
        assert!(!desc.contains("Neue Streamer"));
    }

    struct MockGuild {
        members: Vec<MemberLite>,
    }

    #[async_trait::async_trait]
    impl GuildPort for MockGuild {
        async fn members(&self, _guild_id: u64) -> Option<Vec<MemberLite>> {
            Some(self.members.clone())
        }

        async fn grant_role(&self, _guild_id: u64, _user_id: u64, _role_id: u64) -> String {
            "Rolle gesetzt".to_string()
        }
    }

    struct MockNotifier {
        embeds: Mutex<Vec<Value>>,
        statuses: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Notifier for MockNotifier {
        async fn notify(&self, embed: Value, _components: Option<Value>) -> Option<(u64, u64)> {
            self.embeds.lock().expect("lock").push(embed);
            Some((10, 20))
        }

        async fn finalize_review(
            &self,
            _channel_id: u64,
            _message_id: u64,
            _status: String,
            _color: u32,
        ) {
            self.statuses.lock().expect("lock").push(_status);
        }

        async fn send_text(&self, _channel_id: u64, _text: String) {}
    }

    struct CountingAi {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl AiScorer for CountingAi {
        async fn score(
            &self,
            login: &str,
            _member: &MemberLite,
            _ratio: f64,
        ) -> (Option<i64>, String) {
            self.calls.lock().expect("lock").push(login.to_string());
            (Some(50), "AI".to_string())
        }
    }

    async fn mock_link_candidates(entries: Value) -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().route(
            "/internal/twitch/v1/streamers/link-candidates",
            axum::routing::get(move || {
                let entries = entries.clone();
                async move { axum::Json(json!({ "entries": entries })) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn matcher_verwendet_auch_mit_scorer_keine_ki() {
        let entries = json!([
            { "twitch_login": "alice", "twitch_user_id": "1" },
            { "twitch_login": "bob", "twitch_user_id": "2" },
            { "twitch_login": "carol", "twitch_user_id": "3" }
        ]);
        let (url, server) = mock_link_candidates(entries).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = MatcherConfig {
            enabled: true,
            notify_channel_id: DEFAULT_NOTIFY_CHANNEL_ID,
            role_id: DEFAULT_STREAMER_ROLE_ID,
            guild_id: DEFAULT_GUILD_ID,
            auto_threshold: 90,
            review_threshold: 70,
            fuzzy_floor: 0.62,
            max_ai_per_scan: 2,
            scan_interval_hours: 6,
            state_path: dir.path().join("state.json"),
            ai_provider: "minimax".to_string(),
        };
        let guild = Arc::new(MockGuild {
            members: vec![
                MemberLite {
                    user_id: 1,
                    name: "alice".to_string(),
                    ..MemberLite::default()
                },
                MemberLite {
                    user_id: 2,
                    name: "bob".to_string(),
                    ..MemberLite::default()
                },
                MemberLite {
                    user_id: 3,
                    name: "carol".to_string(),
                    ..MemberLite::default()
                },
            ],
        });
        let notifier = Arc::new(MockNotifier {
            embeds: Mutex::new(Vec::new()),
            statuses: Mutex::new(Vec::new()),
        });
        let scorer = Arc::new(CountingAi {
            calls: Mutex::new(Vec::new()),
        });
        let matcher = Matcher::new(
            config,
            TwitchApiClient::new(url, "tok", std::time::Duration::from_secs(2)),
            guild,
            notifier.clone(),
            scorer.clone(),
        );

        let stats = matcher.run_scan("Test").await;
        assert_eq!(stats.checked, 3);
        assert_eq!(stats.ai_calls, 0);
        assert_eq!(stats.errors, 3);
        assert_eq!(
            scorer.calls.lock().expect("lock").clone(),
            Vec::<String>::new()
        );
        let second = matcher.run_scan_quiet("Noch einmal").await;
        assert_eq!(second.checked, 3);
        assert_eq!(second.errors, 3);
        assert_eq!(second.ai_calls, 0);
        let embeds = notifier.embeds.lock().expect("lock");
        let summary = embeds.last().expect("summary");
        assert!(summary["description"]
            .as_str()
            .expect("description")
            .contains("**AI-Aufrufe:** 0"));
        server.abort();
    }

    #[tokio::test]
    async fn review_status_verwendet_moderator_mention() {
        let dir = tempfile::tempdir().expect("tempdir");
        let notifier = Arc::new(MockNotifier {
            embeds: Mutex::new(Vec::new()),
            statuses: Mutex::new(Vec::new()),
        });
        let matcher = Matcher::new(
            MatcherConfig {
                enabled: true,
                notify_channel_id: DEFAULT_NOTIFY_CHANNEL_ID,
                role_id: DEFAULT_STREAMER_ROLE_ID,
                guild_id: DEFAULT_GUILD_ID,
                auto_threshold: 90,
                review_threshold: 70,
                fuzzy_floor: 0.62,
                max_ai_per_scan: 40,
                scan_interval_hours: 6,
                state_path: dir.path().join("state.json"),
                ai_provider: "minimax".to_string(),
            },
            TwitchApiClient::new(
                "http://127.0.0.1:9",
                "tok",
                std::time::Duration::from_secs(1),
            ),
            Arc::new(MockGuild {
                members: Vec::new(),
            }),
            notifier.clone(),
            Arc::new(NoAi),
        );
        matcher.state.lock().await.pending.insert(
            "tok".to_string(),
            json!({
                "login": "dragskope",
                "channel_id": 10,
                "message_id": 20,
            }),
        );
        let handler = ReviewHandler {
            matcher: matcher.clone(),
        };
        let _ = handler
            .handle(BridgeInteraction {
                custom_id: "slm:reject:tok".to_string(),
                user_id: 99,
                author_name: "Moderator".to_string(),
                author_can_manage_roles: true,
                ..BridgeInteraction::default()
            })
            .await;

        let statuses = notifier.statuses.lock().expect("lock");
        assert!(statuses[0].contains("<@99>"));
        assert!(!statuses[0].contains("Moderator"));
    }

    #[tokio::test]
    async fn manuelle_verknuepfung_bleibt_nach_negativem_versuch_benutzbar() {
        let app = axum::Router::new().route(
            "/internal/twitch/v1/streamers/ohnetreffer/discord-profile",
            axum::routing::post(|| async { axum::Json(json!({"status":"linked"})) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let mut config = MatcherConfig::from_env(|_| None);
        config.state_path = dir.path().join("state.json");
        let matcher = Matcher::new(
            config,
            TwitchApiClient::new(
                format!("http://{address}"),
                "tok",
                std::time::Duration::from_secs(2),
            ),
            Arc::new(MockGuild {
                members: vec![MemberLite {
                    user_id: 42,
                    name: "discordname".into(),
                    ..Default::default()
                }],
            }),
            Arc::new(MockNotifier {
                embeds: Mutex::new(Vec::new()),
                statuses: Mutex::new(Vec::new()),
            }),
            Arc::new(NoAi),
        );
        {
            let mut state = matcher.state.lock().await;
            assert!(state.claim("123", "ohnetreffer").unwrap());
            state
                .manual_pending
                .insert("ohnetreffer".into(), json!({"login":"ohnetreffer"}));
        }
        matcher
            .handle_manual_submit(
                "ohnetreffer",
                &BridgeInteraction {
                    author_can_manage_roles: true,
                    user_id: 99,
                    options: HashMap::from([("discord_input".to_string(), json!("42"))]),
                    ..Default::default()
                },
            )
            .await;
        let state = matcher.state.lock().await;
        assert_eq!(state.processed["ohnetreffer"]["status"], "linked");
        assert!(!state.manual_pending.contains_key("ohnetreffer"));
        assert_eq!(
            LinkState::load(state.path.clone()).attempted_ids["123"]["status"],
            "linked"
        );
        server.abort();
    }

    #[tokio::test]
    async fn mehrdeutige_exakte_namen_brauchen_bestaetigung() {
        let (url, server) =
            mock_link_candidates(json!([{"twitch_login":"alice", "twitch_user_id":"123"}])).await;
        let dir = tempfile::tempdir().unwrap();
        let mut config = MatcherConfig::from_env(|_| None);
        config.state_path = dir.path().join("state.json");
        config.auto_threshold = 70;
        let matcher = Matcher::new(
            config,
            TwitchApiClient::new(url, "tok", std::time::Duration::from_secs(2)),
            Arc::new(MockGuild {
                members: vec![
                    MemberLite {
                        user_id: 1,
                        name: "alice".into(),
                        ..Default::default()
                    },
                    MemberLite {
                        user_id: 2,
                        name: "Alice TTV".into(),
                        ..Default::default()
                    },
                ],
            }),
            Arc::new(MockNotifier {
                embeds: Mutex::new(Vec::new()),
                statuses: Mutex::new(Vec::new()),
            }),
            Arc::new(NoAi),
        );
        let stats = matcher.run_scan_quiet("Test").await;
        assert_eq!(stats.auto, 0);
        assert_eq!(stats.review, 1);
        let state = LinkState::load(matcher.config.state_path.clone());
        assert_eq!(state.pending.len(), 1);
        assert!(state.attempted_ids.contains_key("123"));
        server.abort();
    }

    #[test]
    fn dateiname_ohne_verzeichnis_ist_speicherbar() {
        let path = PathBuf::from(format!(
            "matcher-test-{}.json",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let state = LinkState::load(path.clone());
        state.save().unwrap();
        assert!(path.exists());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn unterbrochene_auswertung_bleibt_nach_neustart_wiederholbar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = LinkState::load(path.clone());
        assert!(state.claim("123", "name").unwrap());
        let mut state = LinkState::load(path);
        assert!(state.claim("123", "name").unwrap());
    }

    struct FlakyNotifier {
        manual_calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Notifier for FlakyNotifier {
        async fn notify(&self, embed: Value, _: Option<Value>) -> Option<(u64, u64)> {
            if embed["title"] == "🔗 Kein Discord-Match"
                && self
                    .manual_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    == 0
            {
                return None;
            }
            Some((10, 20))
        }
        async fn finalize_review(&self, _: u64, _: u64, _: String, _: u32) {}
        async fn send_text(&self, _: u64, _: String) {}
    }

    #[tokio::test]
    async fn fehlgeschlagener_versand_wird_ohne_neue_entscheidung_nachgeholt() {
        let (url, server) =
            mock_link_candidates(json!([{"twitch_login":"ohnetreffer", "twitch_user_id":"123"}]))
                .await;
        let dir = tempfile::tempdir().unwrap();
        let mut config = MatcherConfig::from_env(|_| None);
        config.state_path = dir.path().join("state.json");
        let client = TwitchApiClient::new(url, "tok", std::time::Duration::from_secs(2));
        let guild = Arc::new(MockGuild {
            members: vec![MemberLite {
                user_id: 999,
                name: "zebra".into(),
                ..Default::default()
            }],
        });
        let notifier = Arc::new(FlakyNotifier {
            manual_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let matcher = Matcher::new(
            config.clone(),
            client.clone(),
            guild.clone(),
            notifier.clone(),
            Arc::new(NoAi),
        );
        assert_eq!(matcher.run_scan_quiet("Test").await.checked, 1);
        let matcher = Matcher::new(config, client, guild, notifier.clone(), Arc::new(NoAi));
        assert_eq!(matcher.run_scan_quiet("Test").await.checked, 0);
        assert_eq!(matcher.run_scan_quiet("Test").await.checked, 0);
        assert_eq!(
            notifier
                .manual_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            2
        );
        server.abort();
    }

    #[test]
    fn versuch_bleibt_nach_neustart_und_umbenennung_geschlossen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = LinkState::load(path.clone());
        assert!(state.claim("123", "altername").unwrap());
        state.mark("altername", "no_match", Map::new());
        state.save().unwrap();
        assert!(!state.claim("123", "altername").unwrap());
        let mut state = LinkState::load(path);
        assert!(!state.claim("123", "neuername").unwrap());
        assert!(state.claim("456", "anderer").unwrap());
    }

    #[test]
    fn alte_negative_und_manuelle_faelle_werden_ohne_neuen_versuch_uebernommen() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = LinkState::load(dir.path().join("state.json"));
        state.mark("negativ", "no_match", Map::new());
        state.manual_pending.insert(
            "manuell".into(),
            json!({"login":"manuell", "message_id": 7}),
        );
        assert!(!state.claim("1", "negativ").unwrap());
        assert!(!state.claim("2", "manuell").unwrap());
        assert_eq!(state.manual_pending["manuell"]["message_id"], 7);
    }

    #[test]
    fn speicherfehler_erlaubt_keine_nebenwirkung_und_beschaedigte_datei_bleibt_erhalten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "kaputt").unwrap();
        let mut state = LinkState::load(path.clone());
        assert!(state.claim("1", "name").is_err());
        assert!(state.attempted_ids.is_empty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "kaputt");
    }

    #[tokio::test]
    async fn negativer_scan_sendet_nur_einmal_auch_nach_neustart() {
        let (url, server) =
            mock_link_candidates(json!([{"twitch_login":"ohnetreffer", "twitch_user_id":"123"}]))
                .await;
        let dir = tempfile::tempdir().unwrap();
        let mut config = MatcherConfig::from_env(|_| None);
        config.state_path = dir.path().join("state.json");
        let client = TwitchApiClient::new(url, "tok", std::time::Duration::from_secs(2));
        let guild = Arc::new(MockGuild {
            members: vec![MemberLite {
                user_id: 999,
                name: "zebra".into(),
                ..Default::default()
            }],
        });
        let notifier = Arc::new(MockNotifier {
            embeds: Mutex::new(Vec::new()),
            statuses: Mutex::new(Vec::new()),
        });
        let scorer = Arc::new(CountingAi {
            calls: Mutex::new(Vec::new()),
        });
        let matcher = Matcher::new(
            config.clone(),
            client.clone(),
            guild.clone(),
            notifier.clone(),
            scorer.clone(),
        );
        assert_eq!(matcher.run_scan_quiet("Test").await.skipped, 1);
        let count = notifier.embeds.lock().unwrap().len();
        assert_eq!(matcher.run_scan_quiet("Test").await.checked, 0);
        let matcher = Matcher::new(config, client, guild, notifier.clone(), scorer.clone());
        assert_eq!(matcher.run_scan_quiet("Test").await.checked, 0);
        assert_eq!(notifier.embeds.lock().unwrap().len(), count);
        assert!(scorer.calls.lock().unwrap().is_empty());
        server.abort();
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
        if let Err(err) = state.save() {
            tracing::error!(%err, "Matcher: Zustand konnte nicht gespeichert werden");
        }

        let reloaded = LinkState::load(path);
        assert!(reloaded.is_handled("NANI"));
        assert!(reloaded.is_handled("other")); // pending zählt als handled
        assert!(!reloaded.is_handled("dritter"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
