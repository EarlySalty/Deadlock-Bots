//! Discord Verify-Gate gegen Scam-Accounts.
//!
//! Neue Mitglieder mit einem Konto juenger als die Schwelle, die nicht ueber
//! einen persoenlichen Invite kommen, landen sofort in Vollquarantaene (eigene
//! Rolle, keine Kanaele sichtbar) und muessen per Bot-DM eine feste Frage
//! beantworten ("Nenne einen Hero aus Deadlock"). Wer besteht, wird
//! freigeschaltet. Wer nach drei Versuchen oder binnen der Frist nicht besteht,
//! oder wessen DM nicht zustellbar ist, wird gekickt. Ein einmal gekickter
//! Account bekommt beim naechsten Join freien Zugang.
//!
//! Bestand: Join-Event `dl_discord::MemberEvent::Join`, DM-Message-Loop
//! `subscribe_messages` (Guild-ID None), Interaction-Router fuer den
//! Verifizieren-Knopf, Persistenz in `dl_central_db::verify_gate`, Judge ueber
//! den zentralen `dl_ai::TextGenerator` (Repo-Default DeepSeek v4 Flash).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use dl_ai::{GenerateRequest, TextGenerator};
use dl_central_db::verify_gate as store;
use dl_central_db::verify_gate::STATE_AWAITING_ANSWER;
use dl_central_db::verify_gate::STATE_AWAITING_START;
use dl_discord::{
    BridgeInteraction, BridgeReply, Dispatcher, InteractionHandler, InteractionRouter, MemberEvent,
    MessageEvent,
};
use serde_json::{json, Map, Value};
use sqlx::PgPool;

use crate::invites::{join_source_kind, InviteStore, JoinSourceKind};

/// Hauptgilde der Community (Deadlock).
pub const DEFAULT_GUILD_ID: u64 = 1_289_721_245_281_292_288;

// ── User-sichtbare Texte (natives Deutsch, echte Umlaute, keine Em-Dashes) ──

/// Start-DM: erklaert kurz und bietet den Verifizieren-Knopf an.
pub const START_TEXT: &str = "Willkommen. Dein Discord-Konto ist noch recht neu, darum eine kurze Sicherheitsfrage, damit wir Bots und Scam-Accounts draußen halten. Klick auf Verifizieren, um zu starten.";
/// Label des Verifizieren-Knopfes.
pub const START_BUTTON_LABEL: &str = "Verifizieren";
/// Feste Frage nach dem Verifizieren-Knopf.
pub const QUESTION_TEXT: &str =
    "Nenne einen Hero aus Deadlock. Antworte einfach hier in einem kurzen deutschen Satz.";
/// Abgelehnter Versuch, es sind noch Versuche offen.
pub const FAIL_TEXT: &str =
    "Das hat noch nicht gepasst. Versuch es bitte noch einmal mit einem Hero aus Deadlock, auf Deutsch.";
/// Freischalt-DM nach bestandener Antwort.
pub const PASS_TEXT: &str = "Danke, du bist freigeschaltet. Viel Spaß in der Community.";
/// Antwort auf den Knopf, wenn nichts offen ist (schon verifiziert o. nicht gegatet).
pub const NOTHING_OPEN_TEXT: &str =
    "Für dich ist gerade nichts offen. Du kannst die Community ganz normal nutzen.";

const VERIFY_START_CUSTOM_ID: &str = "verify:start";
const SCHEDULER_INTERVAL: StdDuration = StdDuration::from_secs(300);

/// custom_id-Praefix aller Verify-Gate-Komponenten (fuer die Router-Registrierung).
pub const CUSTOM_ID_PREFIX: &str = "verify:";

// ── Konfiguration ───────────────────────────────────────────────────────────

/// Konfiguration des Verify-Gates. Werte kommen zur Laufzeit aus dem zentralen
/// Config-Weg (Infisical), nichts ist hart verdrahtet. `enforce=false` ist der
/// Betriebs-Notaus: Rolle und DM laufen weiter, aber es wird nicht gekickt.
#[derive(Debug, Clone)]
pub struct VerifyGateConfig {
    pub enabled: bool,
    pub guild_id: u64,
    /// Vorkonfigurierte Quarantaene-Rolle; ist sie None, legt der Bot sie an.
    pub quarantine_role_id: Option<u64>,
    pub max_account_age_days: i64,
    pub deadline_hours: i64,
    pub max_attempts: i32,
    pub enforce: bool,
    /// Judge-Modell; None nutzt den Connector-Default (DeepSeek v4 Flash).
    pub model: Option<String>,
    pub judge_timeout: StdDuration,
}

impl Default for VerifyGateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            guild_id: DEFAULT_GUILD_ID,
            quarantine_role_id: None,
            max_account_age_days: 30,
            deadline_hours: 24,
            max_attempts: 3,
            enforce: true,
            model: None,
            judge_timeout: StdDuration::from_secs(15),
        }
    }
}

// ── Port (von der dl-bot-Glue implementiert) ────────────────────────────────

/// Ergebnis eines DM-Versands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DmOutcome {
    Sent {
        channel_id: u64,
        message_id: u64,
    },
    /// Discord verweigert die DM (Fehler 50007, DMs zu). Gilt als nicht zustellbar.
    Undeliverable,
    /// Technischer Fehler ohne Urteil ueber die Zustellbarkeit.
    Failed(String),
}

/// Discord-Aktionen, die das Gate braucht. In dl-bot gegen den DiscordAdapter
/// implementiert, in Tests gegen einen Mock.
#[async_trait]
pub trait VerifyGatePort: Send + Sync {
    /// Quarantaene-Rolle sicherstellen: existiert `configured` nicht, wird eine
    /// Rolle angelegt; danach wird VIEW_CHANNEL auf JEDEM Kanal der Gilde
    /// (Kategorien, Kinder, unkategorisierte) fuer diese Rolle verweigert.
    /// Liefert die effektive Rollen-ID. Idempotent.
    async fn ensure_quarantine_role(
        &self,
        guild_id: u64,
        configured: Option<u64>,
    ) -> Result<u64, String>;
    async fn assign_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), String>;
    async fn remove_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), String>;
    async fn kick(&self, guild_id: u64, user_id: u64, reason: &str) -> Result<(), String>;
    async fn send_dm(&self, user_id: u64, body: Map<String, Value>) -> DmOutcome;
}

// ── Reine Entscheidungslogik (ohne DB, unit-testbar) ────────────────────────

/// Ist das Konto juenger als die Schwelle?
pub fn account_is_young(account_created_at: i64, now: i64, max_age_days: i64) -> bool {
    let age_secs = now.saturating_sub(account_created_at);
    age_secs < max_age_days.saturating_mul(86_400)
}

/// Ist die Join-Quelle ein persoenlicher Invite eines Mitglieds? Prueft die rohe
/// Quell-Art aus dem metadata-JSON statt eines formatierten Anzeige-Labels,
/// damit eine Wortaenderung am Label die Ausnahme nicht still bricht. Ein
/// Website-Invite-Code hat Vorrang und zaehlt NICHT als persoenlich.
pub fn is_personal_invite(meta: &Value, website_codes: &HashMap<String, String>) -> bool {
    join_source_kind(meta, website_codes) == JoinSourceKind::PersonalInvite
}

/// Greift das Gate fuer diesen Join?
pub fn should_gate(
    is_bot: bool,
    account_young: bool,
    personal_invite: bool,
    already_kicked: bool,
) -> bool {
    !is_bot && account_young && !personal_invite && !already_kicked
}

// ── M3 Heroliste-Abgleich ───────────────────────────────────────────────────

/// Normalisiert Text auf kleingeschriebene, durch Leerzeichen getrennte Tokens
/// und rahmt ihn mit Leerzeichen, damit Vergleiche an Wortgrenzen greifen.
fn normalize_for_match(text: &str) -> String {
    let mut buf = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                buf.push(lower);
            }
        } else {
            buf.push(' ');
        }
    }
    let collapsed = buf.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(" {collapsed} ")
}

/// Enthaelt der Text einen der Heronamen an einer Wortgrenze?
pub fn text_contains_hero(text: &str, hero_names: &[String]) -> bool {
    let haystack = normalize_for_match(text);
    hero_names.iter().any(|name| {
        let needle = normalize_for_match(name);
        needle.trim().len() >= 2 && haystack.contains(&needle)
    })
}

async fn fetch_hero_names(pool: &PgPool) -> Vec<String> {
    match sqlx::query_scalar::<_, String>(
        "SELECT name FROM tierlist.deadlock_heroes WHERE is_active = TRUE",
    )
    .fetch_all(pool)
    .await
    {
        Ok(names) => names,
        Err(err) => {
            tracing::warn!(%err, "Verify-Gate: Heroliste nicht ladbar, nur Judge-Pfad");
            Vec::new()
        }
    }
}

/// Nennt die Antwort einen bekannten Hero (Listenabgleich)?
pub async fn answer_matches_hero(pool: &PgPool, text: &str) -> bool {
    let names = fetch_hero_names(pool).await;
    text_contains_hero(text, &names)
}

// ── M4 Verify-Judge ─────────────────────────────────────────────────────────

/// Urteil des Judges. `pass=true` schaltet frei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub pass: bool,
}

const JUDGE_SYSTEM: &str =
    "Du pruefst Antworten in einem Verifizierungs-Gate fuer eine deutschsprachige Deadlock-Community.";

fn build_judge_prompt(answer: &str) -> String {
    format!(
        "Der Nutzer sollte einen Helden aus dem Spiel Deadlock nennen oder beschreiben. \
Beschreibt oder nennt der folgende Text plausibel einen Deadlock-Helden, UND ist der Text auf Deutsch geschrieben? \
Antworte ausschliesslich mit einem JSON-Objekt der Form {{\"hero\": true|false, \"deutsch\": true|false}}, ohne weiteren Text.\n\nText: {answer}"
    )
}

/// Parst das Judge-JSON. Ein klar erkennbares Urteil entscheidet; alles Unsichere
/// (kein JSON, fehlende Felder) faellt fail-open auf `pass=true`.
pub fn parse_verdict(raw: &str) -> Verdict {
    let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) else {
        return Verdict { pass: true };
    };
    if end < start {
        return Verdict { pass: true };
    }
    let value: Value = match serde_json::from_str(&raw[start..=end]) {
        Ok(value) => value,
        Err(_) => return Verdict { pass: true },
    };
    match (
        value.get("hero").and_then(Value::as_bool),
        value.get("deutsch").and_then(Value::as_bool),
    ) {
        (Some(hero), Some(deutsch)) => Verdict {
            pass: hero && deutsch,
        },
        // Unvollstaendiges Urteil: fail-open.
        _ => Verdict { pass: true },
    }
}

/// Bewertet die Antwort ueber den Judge. Timeout, Provider-Fehler oder ein
/// unsicheres Urteil fallen fail-open auf `pass=true` (REQ-08).
pub async fn judge_answer(
    generator: &Arc<dyn TextGenerator>,
    model: Option<String>,
    timeout: StdDuration,
    answer: &str,
) -> Verdict {
    let request = GenerateRequest {
        prompt: build_judge_prompt(answer),
        system_prompt: Some(JUDGE_SYSTEM.to_string()),
        model,
        max_output_tokens: Some(80),
        reasoning_effort: None,
        temperature: 0.0,
    };
    match tokio::time::timeout(timeout, generator.generate_text(request)).await {
        Ok(Some(raw)) => parse_verdict(&raw),
        // Kein Text vom Provider oder Timeout: fail-open.
        Ok(None) | Err(_) => Verdict { pass: true },
    }
}

// ── DM-Bodies ───────────────────────────────────────────────────────────────

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// Start-DM inklusive Verifizieren-Knopf.
pub fn start_dm_body() -> Map<String, Value> {
    object(json!({
        "content": START_TEXT,
        "components": [{
            "type": 1,
            "components": [{
                "type": 2,
                "style": 1,
                "label": START_BUTTON_LABEL,
                "custom_id": VERIFY_START_CUSTOM_ID,
            }],
        }],
    }))
}

fn text_dm_body(content: &str) -> Map<String, Value> {
    object(json!({ "content": content }))
}

// ── Gate ────────────────────────────────────────────────────────────────────

pub struct VerifyGate {
    pool: PgPool,
    port: Arc<dyn VerifyGatePort>,
    judge: Option<Arc<dyn TextGenerator>>,
    config: VerifyGateConfig,
    /// Aufgeloeste Quarantaene-Rollen-ID (0 = noch nicht aufgeloest).
    role_id: AtomicU64,
}

impl VerifyGate {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn VerifyGatePort>,
        judge: Option<Arc<dyn TextGenerator>>,
        config: VerifyGateConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            judge,
            config,
            role_id: AtomicU64::new(0),
        })
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    fn guild(&self) -> i64 {
        self.config.guild_id as i64
    }

    /// Stellt die Quarantaene-Rolle sicher und cached ihre ID.
    pub async fn ensure_role(&self) -> Option<u64> {
        let cached = self.role_id.load(Ordering::Acquire);
        if cached != 0 {
            return Some(cached);
        }
        match self
            .port
            .ensure_quarantine_role(self.config.guild_id, self.config.quarantine_role_id)
            .await
        {
            Ok(role_id) => {
                self.role_id.store(role_id, Ordering::Release);
                Some(role_id)
            }
            Err(err) => {
                tracing::error!(%err, "Verify-Gate: Quarantaene-Rolle konnte nicht eingerichtet werden");
                None
            }
        }
    }

    /// Kickt, sofern `enforce=true`; sonst nur Log. Bei erfolgtem Kick werden
    /// Kick-Liste und pending-Zustand nachgezogen.
    async fn enforce_kick(&self, user_id: u64, reason: &str) {
        if !self.config.enforce {
            tracing::warn!(
                user_id,
                reason,
                "Verify-Gate: enforce=false, Kick uebersprungen (Rolle bleibt)"
            );
            return;
        }
        match self.port.kick(self.config.guild_id, user_id, reason).await {
            Ok(()) => {
                if let Err(err) = store::mark_kicked(&self.pool, self.guild(), user_id as i64).await
                {
                    tracing::error!(%err, user_id, "Verify-Gate: mark_kicked fehlgeschlagen");
                }
                if let Err(err) =
                    store::delete_pending(&self.pool, self.guild(), user_id as i64).await
                {
                    tracing::error!(%err, user_id, "Verify-Gate: delete_pending nach Kick fehlgeschlagen");
                }
            }
            Err(err) => {
                tracing::error!(%err, user_id, reason, "Verify-Gate: Kick fehlgeschlagen");
            }
        }
    }

    /// Join-Handler: entscheidet, quarantaeniert und startet die DM.
    pub async fn handle_join(
        &self,
        guild_id: u64,
        user_id: u64,
        is_bot: bool,
        account_created_at: i64,
        metadata: &Value,
    ) {
        if guild_id != self.config.guild_id {
            return;
        }
        let now = Utc::now().timestamp();
        let young = account_is_young(account_created_at, now, self.config.max_account_age_days);
        if is_bot || !young {
            return;
        }

        let website_codes = InviteStore {
            pool: self.pool.clone(),
        }
        .website_code_map()
        .await;
        let personal = is_personal_invite(metadata, &website_codes);

        let already_kicked = store::was_kicked(&self.pool, self.guild(), user_id as i64)
            .await
            .unwrap_or_else(|err| {
                tracing::error!(%err, user_id, "Verify-Gate: was_kicked-Abfrage fehlgeschlagen");
                false
            });

        if !should_gate(is_bot, young, personal, already_kicked) {
            return;
        }

        let Some(role_id) = self.ensure_role().await else {
            tracing::error!(
                user_id,
                "Verify-Gate: kein Rollen-Setup, Join nicht gegatet"
            );
            return;
        };

        // Quarantaene zuerst (Kanaele sofort weg), dann DM.
        if let Err(err) = self
            .port
            .assign_role(guild_id, user_id, role_id, "Verify-Gate: Quarantaene")
            .await
        {
            tracing::error!(%err, user_id, "Verify-Gate: Quarantaene-Rolle nicht vergeben");
        }

        let deadline = Utc::now() + Duration::hours(self.config.deadline_hours);
        if let Err(err) = store::upsert_pending(
            &self.pool,
            self.guild(),
            user_id as i64,
            None,
            STATE_AWAITING_START,
            deadline,
        )
        .await
        {
            tracing::error!(%err, user_id, "Verify-Gate: pending-Zustand nicht gespeichert");
            return;
        }

        match self.port.send_dm(user_id, start_dm_body()).await {
            DmOutcome::Sent { channel_id, .. } => {
                if let Err(err) = store::set_dm_channel(
                    &self.pool,
                    self.guild(),
                    user_id as i64,
                    channel_id as i64,
                )
                .await
                {
                    tracing::error!(%err, user_id, "Verify-Gate: DM-Kanal nicht gespeichert");
                }
            }
            DmOutcome::Undeliverable => {
                tracing::info!(user_id, "Verify-Gate: DM nicht zustellbar, Kick");
                self.enforce_kick(user_id, "Verify-Gate: DM nicht zustellbar")
                    .await;
            }
            DmOutcome::Failed(err) => {
                tracing::error!(%err, user_id, "Verify-Gate: Start-DM fehlgeschlagen, Frist-Kick greift spaeter");
            }
        }
    }

    /// Interaction-Handler fuer den Verifizieren-Knopf. Liefert die Frage-DM.
    pub async fn handle_verify_start(&self, user_id: u64) -> BridgeReply {
        match store::get_pending(&self.pool, self.guild(), user_id as i64).await {
            Ok(Some(_)) => {
                if let Err(err) = store::set_state(
                    &self.pool,
                    self.guild(),
                    user_id as i64,
                    STATE_AWAITING_ANSWER,
                )
                .await
                {
                    tracing::error!(%err, user_id, "Verify-Gate: Zustandswechsel fehlgeschlagen");
                }
                BridgeReply {
                    content: Some(QUESTION_TEXT.to_string()),
                    ..BridgeReply::default()
                }
            }
            Ok(None) => BridgeReply::ephemeral_text(NOTHING_OPEN_TEXT),
            Err(err) => {
                tracing::error!(%err, user_id, "Verify-Gate: pending-Abfrage fehlgeschlagen");
                BridgeReply::ephemeral_text(NOTHING_OPEN_TEXT)
            }
        }
    }

    /// DM-Antwort eines Mitglieds im Zustand `awaiting_answer` bewerten.
    pub async fn handle_answer(&self, user_id: u64, text: &str) {
        let pending = match store::get_pending(&self.pool, self.guild(), user_id as i64).await {
            Ok(Some(pending)) => pending,
            Ok(None) => return,
            Err(err) => {
                tracing::error!(%err, user_id, "Verify-Gate: pending-Abfrage fehlgeschlagen");
                return;
            }
        };
        // Erst nach dem Verifizieren-Knopf werden Antworten bewertet.
        if pending.state != STATE_AWAITING_ANSWER {
            return;
        }

        let passed = if answer_matches_hero(&self.pool, text).await {
            true
        } else {
            match &self.judge {
                Some(generator) => {
                    judge_answer(
                        generator,
                        self.config.model.clone(),
                        self.config.judge_timeout,
                        text,
                    )
                    .await
                    .pass
                }
                // Kein Judge verdrahtet: fail-open.
                None => true,
            }
        };

        if passed {
            self.pass_member(user_id).await;
        } else {
            self.fail_member(user_id).await;
        }
    }

    async fn pass_member(&self, user_id: u64) {
        if let Some(role_id) = self.ensure_role().await {
            if let Err(err) = self
                .port
                .remove_role(
                    self.config.guild_id,
                    user_id,
                    role_id,
                    "Verify-Gate: bestanden",
                )
                .await
            {
                tracing::error!(%err, user_id, "Verify-Gate: Quarantaene-Rolle nicht entfernt");
            }
        }
        match self.port.send_dm(user_id, text_dm_body(PASS_TEXT)).await {
            DmOutcome::Sent { .. } => {}
            other => tracing::warn!(
                user_id,
                ?other,
                "Verify-Gate: Freischalt-DM nicht zugestellt"
            ),
        }
        if let Err(err) = store::delete_pending(&self.pool, self.guild(), user_id as i64).await {
            tracing::error!(%err, user_id, "Verify-Gate: pending nach Bestehen nicht geloescht");
        }
    }

    async fn fail_member(&self, user_id: u64) {
        let attempts = match store::incr_attempt(&self.pool, self.guild(), user_id as i64).await {
            Ok(attempts) => attempts,
            Err(err) => {
                tracing::error!(%err, user_id, "Verify-Gate: Versuchszaehler nicht erhoeht");
                return;
            }
        };
        if attempts >= self.config.max_attempts {
            self.enforce_kick(user_id, "Verify-Gate: drei Fehlversuche")
                .await;
        } else {
            match self.port.send_dm(user_id, text_dm_body(FAIL_TEXT)).await {
                DmOutcome::Sent { .. } => {}
                other => {
                    tracing::warn!(user_id, ?other, "Verify-Gate: Fehler-DM nicht zugestellt")
                }
            }
        }
    }

    /// Frist-Kick: alle abgelaufenen offenen Zustaende kicken.
    pub async fn run_deadline_sweep(&self) {
        let now = Utc::now();
        let expired = match store::list_expired(&self.pool, now).await {
            Ok(expired) => expired,
            Err(err) => {
                tracing::error!(%err, "Verify-Gate: list_expired fehlgeschlagen");
                return;
            }
        };
        for pending in expired {
            if pending.guild_id != self.guild() {
                continue;
            }
            self.enforce_kick(pending.user_id as u64, "Verify-Gate: Frist abgelaufen")
                .await;
        }
    }
}

// ── Interaction-Handler ─────────────────────────────────────────────────────

struct VerifyStartHandler {
    gate: Arc<VerifyGate>,
}

#[async_trait]
impl InteractionHandler for VerifyStartHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        self.gate.handle_verify_start(interaction.user_id).await
    }
}

/// Registriert den Verifizieren-Knopf am Interaction-Router.
pub fn register(router: &mut InteractionRouter, gate: Arc<VerifyGate>) {
    router.on_custom_id(
        VERIFY_START_CUSTOM_ID,
        Arc::new(VerifyStartHandler { gate }),
    );
}

// ── Spawn (Join-Loop, DM-Loop, Frist-Scheduler) ─────────────────────────────

/// Startet die Hintergrundaufgaben des Verify-Gates.
pub fn spawn(gate: Arc<VerifyGate>, dispatcher: &Dispatcher) -> Vec<tokio::task::JoinHandle<()>> {
    if !gate.enabled() {
        return Vec::new();
    }
    let mut handles = Vec::new();

    // Rolle beim Start einrichten (idempotent).
    {
        let gate = gate.clone();
        handles.push(tokio::spawn(async move {
            gate.ensure_role().await;
        }));
    }

    // Join-Loop.
    {
        let gate = gate.clone();
        let mut members = dispatcher.subscribe_members();
        handles.push(tokio::spawn(async move {
            loop {
                match members.recv().await {
                    Ok(MemberEvent::Join {
                        guild_id,
                        user_id,
                        account_created_at,
                        is_bot,
                        metadata,
                        ..
                    }) => {
                        gate.handle_join(guild_id, user_id, is_bot, account_created_at, &metadata)
                            .await;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::warn!(missed, "Verify-Gate: Member-Events verpasst");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
    }

    // DM-Antwort-Loop (Guild-ID None = Direktnachricht).
    {
        let gate = gate.clone();
        let mut messages = dispatcher.subscribe_messages();
        handles.push(tokio::spawn(async move {
            loop {
                match messages.recv().await {
                    Ok(MessageEvent {
                        guild_id: None,
                        author_id,
                        content,
                        ..
                    }) => {
                        if !content.trim().is_empty() {
                            gate.handle_answer(author_id, &content).await;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::warn!(missed, "Verify-Gate: Message-Events verpasst");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
    }

    // Frist-Kick-Scheduler.
    {
        let gate = gate.clone();
        handles.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(SCHEDULER_INTERVAL);
            loop {
                interval.tick().await;
                gate.run_deadline_sweep().await;
            }
        }));
    }

    handles
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    // ── Reine Logik ─────────────────────────────────────────────────────────

    #[test]
    fn account_age_threshold() {
        let now = 1_000_000_000_i64;
        // 29 Tage alt -> jung.
        assert!(account_is_young(now - 29 * 86_400, now, 30));
        // exakt 30 Tage -> nicht mehr jung.
        assert!(!account_is_young(now - 30 * 86_400, now, 30));
        // 40 Tage -> nicht jung.
        assert!(!account_is_young(now - 40 * 86_400, now, 30));
    }

    #[test]
    fn personal_invite_detection() {
        let no_codes = HashMap::new();
        assert!(is_personal_invite(
            &json!({ "join_source_kind": "invite_link", "inviter_name": "nani" }),
            &no_codes,
        ));
        assert!(!is_personal_invite(
            &json!({ "join_source_kind": "vanity" }),
            &no_codes,
        ));
        assert!(!is_personal_invite(
            &json!({ "join_source_kind": "server_discovery" }),
            &no_codes,
        ));
        // Website-Invite (invite_link mit Website-Code) zaehlt NICHT als
        // persoenlich, obwohl die rohe kind "invite_link" ist.
        let mut website_codes = HashMap::new();
        website_codes.insert("abc123".to_string(), "Landing".to_string());
        assert!(!is_personal_invite(
            &json!({ "join_source_kind": "invite_link", "invite_code": "ABC123" }),
            &website_codes,
        ));
    }

    #[test]
    fn gate_decision_matrix() {
        // Frischer Account, keine Ausnahme -> gaten.
        assert!(should_gate(false, true, false, false));
        // Bot -> nie.
        assert!(!should_gate(true, true, false, false));
        // Alter Account -> nein.
        assert!(!should_gate(false, false, false, false));
        // Persoenlicher Invite -> nein.
        assert!(!should_gate(false, true, true, false));
        // Schon einmal gekickt -> nein.
        assert!(!should_gate(false, true, false, true));
    }

    #[test]
    fn hero_list_match_word_boundary() {
        let heroes = vec![
            "Abrams".to_string(),
            "Grey Talon".to_string(),
            "Mo & Krill".to_string(),
        ];
        assert!(text_contains_hero("Ich spiele gerne Abrams!", &heroes));
        assert!(text_contains_hero("grey talon ist stark", &heroes));
        // Teilwort darf nicht matchen.
        assert!(!text_contains_hero("Abramsson ist kein Hero", &heroes));
        assert!(!text_contains_hero("banane", &heroes));
        // Leere Liste -> nie.
        assert!(!text_contains_hero("Abrams", &[]));
    }

    #[test]
    fn verdict_parsing() {
        // Klares Ja.
        assert!(parse_verdict("{\"hero\": true, \"deutsch\": true}").pass);
        // Englisch -> deutsch=false -> durchgefallen.
        assert!(!parse_verdict("{\"hero\": true, \"deutsch\": false}").pass);
        // Kein Hero -> durchgefallen.
        assert!(!parse_verdict("{\"hero\": false, \"deutsch\": true}").pass);
        // JSON eingebettet in Fliesstext.
        assert!(!parse_verdict("Antwort: {\"hero\": false, \"deutsch\": true} fertig").pass);
        // Kaputt/kein JSON -> fail-open.
        assert!(parse_verdict("weiss nicht").pass);
        // Feld fehlt -> fail-open.
        assert!(parse_verdict("{\"hero\": true}").pass);
    }

    // ── Judge mit fixem Provider ─────────────────────────────────────────────

    struct FixedJudge(Option<String>);

    #[async_trait]
    impl TextGenerator for FixedJudge {
        async fn generate_text(&self, _request: GenerateRequest) -> Option<String> {
            self.0.clone()
        }
    }

    fn judge(reply: Option<&str>) -> Arc<dyn TextGenerator> {
        Arc::new(FixedJudge(reply.map(str::to_string)))
    }

    #[tokio::test]
    async fn judge_passes_german_hero() {
        let generator = judge(Some("{\"hero\": true, \"deutsch\": true}"));
        let verdict = judge_answer(
            &generator,
            None,
            StdDuration::from_secs(5),
            "der mit der Kette",
        )
        .await;
        assert!(verdict.pass);
    }

    #[tokio::test]
    async fn judge_fails_english() {
        let generator = judge(Some("{\"hero\": true, \"deutsch\": false}"));
        let verdict = judge_answer(
            &generator,
            None,
            StdDuration::from_secs(5),
            "the one with hooks",
        )
        .await;
        assert!(!verdict.pass);
    }

    #[tokio::test]
    async fn judge_fail_open_on_provider_error() {
        // Provider liefert nichts (Fehler/Timeout-Aequivalent) -> fail-open.
        let generator = judge(None);
        let verdict = judge_answer(&generator, None, StdDuration::from_secs(5), "irgendwas").await;
        assert!(verdict.pass);
    }

    // ── Flow gegen Mock-Port + Test-DB (ignored) ─────────────────────────────

    #[cfg(feature = "testing")]
    mod flow {
        use super::*;
        use dl_central_db::testing::test_pool;
        use std::sync::Mutex;

        #[derive(Default)]
        struct Recorder {
            assigned: Vec<u64>,
            removed: Vec<u64>,
            kicked: Vec<u64>,
            dms: Vec<u64>,
        }

        struct MockPort {
            role_id: u64,
            dm: DmOutcome,
            rec: Mutex<Recorder>,
        }

        impl MockPort {
            fn new(dm: DmOutcome) -> Arc<Self> {
                Arc::new(Self {
                    role_id: 999,
                    dm,
                    rec: Mutex::new(Recorder::default()),
                })
            }
        }

        #[async_trait]
        impl VerifyGatePort for MockPort {
            async fn ensure_quarantine_role(
                &self,
                _guild_id: u64,
                _configured: Option<u64>,
            ) -> Result<u64, String> {
                Ok(self.role_id)
            }
            async fn assign_role(
                &self,
                _g: u64,
                user_id: u64,
                _r: u64,
                _reason: &str,
            ) -> Result<(), String> {
                self.rec.lock().unwrap().assigned.push(user_id);
                Ok(())
            }
            async fn remove_role(
                &self,
                _g: u64,
                user_id: u64,
                _r: u64,
                _reason: &str,
            ) -> Result<(), String> {
                self.rec.lock().unwrap().removed.push(user_id);
                Ok(())
            }
            async fn kick(&self, _g: u64, user_id: u64, _reason: &str) -> Result<(), String> {
                self.rec.lock().unwrap().kicked.push(user_id);
                Ok(())
            }
            async fn send_dm(&self, user_id: u64, _body: Map<String, Value>) -> DmOutcome {
                self.rec.lock().unwrap().dms.push(user_id);
                self.dm.clone()
            }
        }

        fn config() -> VerifyGateConfig {
            VerifyGateConfig {
                enabled: true,
                guild_id: 42,
                quarantine_role_id: Some(999),
                enforce: true,
                ..VerifyGateConfig::default()
            }
        }

        fn young_join_meta() -> Value {
            json!({ "join_source_kind": "vanity" })
        }

        #[tokio::test]
        #[ignore = "requires CENTRAL_TEST_DSN"]
        async fn join_quarantines_and_dms() {
            let db = test_pool().await.expect("test pool");
            let port = MockPort::new(DmOutcome::Sent {
                channel_id: 7,
                message_id: 8,
            });
            let gate = VerifyGate::new((*db).clone(), port.clone(), Some(judge(None)), config());

            let now = Utc::now().timestamp();
            gate.handle_join(42, 100, false, now - 3 * 86_400, &young_join_meta())
                .await;

            {
                let rec = port.rec.lock().unwrap();
                assert_eq!(rec.assigned, vec![100]);
                assert_eq!(rec.dms, vec![100]);
            }
            let pending = store::get_pending(&db, 42, 100).await.unwrap().unwrap();
            assert_eq!(pending.state, STATE_AWAITING_START);
        }

        #[tokio::test]
        #[ignore = "requires CENTRAL_TEST_DSN"]
        async fn personal_invite_and_kicked_skip_gate() {
            let db = test_pool().await.expect("test pool");
            let port = MockPort::new(DmOutcome::Sent {
                channel_id: 7,
                message_id: 8,
            });
            let gate = VerifyGate::new((*db).clone(), port.clone(), Some(judge(None)), config());
            let now = Utc::now().timestamp();

            // Persoenlicher Invite -> kein Gate.
            gate.handle_join(
                42,
                200,
                false,
                now - 86_400,
                &json!({ "join_source_kind": "invite_link", "inviter_name": "nani" }),
            )
            .await;
            assert!(store::get_pending(&db, 42, 200).await.unwrap().is_none());

            // Bereits gekickt -> kein Gate.
            store::mark_kicked(&db, 42, 300).await.unwrap();
            gate.handle_join(42, 300, false, now - 86_400, &young_join_meta())
                .await;
            assert!(store::get_pending(&db, 42, 300).await.unwrap().is_none());

            assert!(port.rec.lock().unwrap().assigned.is_empty());
        }

        #[tokio::test]
        #[ignore = "requires CENTRAL_TEST_DSN"]
        async fn correct_answer_passes() {
            let db = test_pool().await.expect("test pool");
            let port = MockPort::new(DmOutcome::Sent {
                channel_id: 7,
                message_id: 8,
            });
            // Judge sagt Ja.
            let gate = VerifyGate::new(
                (*db).clone(),
                port.clone(),
                Some(judge(Some("{\"hero\": true, \"deutsch\": true}"))),
                config(),
            );
            let now = Utc::now().timestamp();
            gate.handle_join(42, 400, false, now - 86_400, &young_join_meta())
                .await;
            gate.handle_verify_start(400).await;
            gate.handle_answer(400, "der grosse Roboter mit der Bombe")
                .await;

            assert_eq!(port.rec.lock().unwrap().removed, vec![400]);
            assert!(store::get_pending(&db, 42, 400).await.unwrap().is_none());
            assert!(!store::was_kicked(&db, 42, 400).await.unwrap());
        }

        #[tokio::test]
        #[ignore = "requires CENTRAL_TEST_DSN"]
        async fn three_failures_kick() {
            let db = test_pool().await.expect("test pool");
            let port = MockPort::new(DmOutcome::Sent {
                channel_id: 7,
                message_id: 8,
            });
            // Judge sagt immer Nein.
            let gate = VerifyGate::new(
                (*db).clone(),
                port.clone(),
                Some(judge(Some("{\"hero\": false, \"deutsch\": true}"))),
                config(),
            );
            let now = Utc::now().timestamp();
            gate.handle_join(42, 500, false, now - 86_400, &young_join_meta())
                .await;
            gate.handle_verify_start(500).await;
            gate.handle_answer(500, "keine ahnung").await;
            gate.handle_answer(500, "immer noch nicht").await;
            assert!(
                port.rec.lock().unwrap().kicked.is_empty(),
                "erst nach drittem Versuch"
            );
            gate.handle_answer(500, "auch nicht").await;

            assert_eq!(port.rec.lock().unwrap().kicked, vec![500]);
            assert!(store::was_kicked(&db, 42, 500).await.unwrap());
            assert!(store::get_pending(&db, 42, 500).await.unwrap().is_none());
        }

        #[tokio::test]
        #[ignore = "requires CENTRAL_TEST_DSN"]
        async fn undeliverable_dm_kicks() {
            let db = test_pool().await.expect("test pool");
            let port = MockPort::new(DmOutcome::Undeliverable);
            let gate = VerifyGate::new((*db).clone(), port.clone(), Some(judge(None)), config());
            let now = Utc::now().timestamp();
            gate.handle_join(42, 600, false, now - 86_400, &young_join_meta())
                .await;

            assert_eq!(port.rec.lock().unwrap().kicked, vec![600]);
            assert!(store::was_kicked(&db, 42, 600).await.unwrap());
            assert!(store::get_pending(&db, 42, 600).await.unwrap().is_none());
        }

        #[tokio::test]
        #[ignore = "requires CENTRAL_TEST_DSN"]
        async fn expired_deadline_kicks() {
            let db = test_pool().await.expect("test pool");
            let port = MockPort::new(DmOutcome::Sent {
                channel_id: 7,
                message_id: 8,
            });
            let gate = VerifyGate::new((*db).clone(), port.clone(), Some(judge(None)), config());
            // Abgelaufener Zustand direkt anlegen.
            store::upsert_pending(
                &db,
                42,
                700,
                None,
                STATE_AWAITING_ANSWER,
                Utc::now() - Duration::minutes(5),
            )
            .await
            .unwrap();

            gate.run_deadline_sweep().await;

            assert_eq!(port.rec.lock().unwrap().kicked, vec![700]);
            assert!(store::was_kicked(&db, 42, 700).await.unwrap());
        }

        #[tokio::test]
        #[ignore = "requires CENTRAL_TEST_DSN"]
        async fn enforce_off_does_not_kick() {
            let db = test_pool().await.expect("test pool");
            let port = MockPort::new(DmOutcome::Undeliverable);
            let mut cfg = config();
            cfg.enforce = false;
            let gate = VerifyGate::new((*db).clone(), port.clone(), Some(judge(None)), cfg);
            let now = Utc::now().timestamp();
            gate.handle_join(42, 800, false, now - 86_400, &young_join_meta())
                .await;

            // Rolle vergeben, aber kein Kick trotz undeliverable DM.
            assert_eq!(port.rec.lock().unwrap().assigned, vec![800]);
            assert!(port.rec.lock().unwrap().kicked.is_empty());
            assert!(!store::was_kicked(&db, 42, 800).await.unwrap());
        }
    }
}
