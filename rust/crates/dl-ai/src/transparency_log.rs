//! Die Senke hinter dem Transparenz-Decorator: Queue, Drossel, Textform.
//!
//! Discord-frei mit Absicht — `dl-ai` bekommt keine Discord-Abhaengigkeit. Das
//! Binaer reicht einen [`TransparencyMessenger`] herein, der die zwei
//! Operationen kann, die dieses Log braucht: eine Nachricht senden und aus
//! einer Nachricht einen Thread machen.
//!
//! Leitlinien aus der Abnahme:
//! - Vollstaendigkeit schlaegt Sparsamkeit. Jede Interaktion wird einzeln
//!   sichtbar, kein Sampling, keine Aggregation.
//! - Bei Lastspitzen lieber verzoegern als verwerfen: die Queue ist gross, die
//!   Drossel ist eine Warteschlange und kein Filter. Verwerfen ist der letzte
//!   Ausweg und wird dann mit Anzahl ausgewiesen.
//! - Die KI-Antwort ist der Teil, den der Owner beurteilen will. Gekuerzt wird
//!   deshalb zuerst der Kontext, dann der Auslöser und erst zuletzt die
//!   Antwort.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::chat_provider::LlmUseCase;
use crate::transparency::{
    is_moderation_use_case, use_case_label, AiInteraction, AiTransparencySink,
    ConversationFingerprint,
};

/// Der Kanal, in dem die Community-Leitung die KI mitliest.
pub const DEFAULT_TRANSPARENCY_CHANNEL_ID: u64 = 1_374_364_800_817_303_632;

/// Discord nimmt hart 2000 Zeichen pro Nachricht; 100 Zeichen Puffer.
pub const DISCORD_CONTENT_LIMIT: usize = 1_900;

/// Discord nimmt hoechstens 100 Zeichen im Thread-Namen.
const THREAD_NAME_LIMIT: usize = 100;

/// Gross gewaehlt: bei einer Lastspitze soll gewartet und nicht verworfen
/// werden. 4096 Eintraege sind rund eine Stunde Vollast bei der Drossel unten.
const DEFAULT_QUEUE_CAPACITY: usize = 4_096;

/// Discord erlaubt rund 5 Nachrichten je 5 Sekunden und Kanal. Mit 1,1 s
/// Abstand bleibt Luft fuer die uebrigen Bot-Posts im selben Kanal.
const DEFAULT_MIN_SEND_INTERVAL: Duration = Duration::from_millis(1_100);

/// So viele Gespraeche behaelt die Thread-Zuordnung im Gedaechtnis.
const CONVERSATION_MEMORY: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransparencyConfig {
    pub enabled: bool,
    pub channel_id: u64,
    /// Moderation spiegeln? Default aus, siehe
    /// [`crate::transparency::is_moderation_use_case`].
    pub include_moderation: bool,
    pub queue_capacity: usize,
    pub min_send_interval: Duration,
}

impl Default for TransparencyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            channel_id: DEFAULT_TRANSPARENCY_CHANNEL_ID,
            include_moderation: false,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            min_send_interval: DEFAULT_MIN_SEND_INTERVAL,
        }
    }
}

impl TransparencyConfig {
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let default = Self::default();
        Self {
            enabled: env_bool(&lookup, "DL_AI_TRANSPARENCY_ENABLED", default.enabled),
            channel_id: read_env(&lookup, "DL_AI_TRANSPARENCY_CHANNEL_ID")
                .and_then(|value| value.parse().ok())
                .filter(|value| *value > 0)
                .unwrap_or(default.channel_id),
            include_moderation: env_bool(
                &lookup,
                "DL_AI_TRANSPARENCY_INCLUDE_MODERATION",
                default.include_moderation,
            ),
            ..default
        }
    }

    /// Eine Zeile fuer das Startinventar: was ist scharf und wo landet es?
    pub fn inventory_line(&self) -> String {
        if !self.enabled {
            return "KI-Transparenz-Log: aus (DL_AI_TRANSPARENCY_ENABLED)".to_string();
        }
        format!(
            "KI-Transparenz-Log: an, Kanal {}, Moderation gespiegelt: {}",
            self.channel_id,
            if self.include_moderation { "ja" } else { "nein" }
        )
    }
}

fn read_env(lookup: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    lookup(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_bool(lookup: &impl Fn(&str) -> Option<String>, key: &str, default: bool) -> bool {
    match read_env(lookup, key) {
        None => default,
        Some(value) => !matches!(
            value.to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "nein" | "off" | "aus"
        ),
    }
}

/// Der Weg nach Discord, den das jeweilige Binaer schon hat.
#[async_trait]
pub trait TransparencyMessenger: Send + Sync {
    /// Sendet den Text und liefert die Discord-Message-ID zurueck.
    async fn send(&self, channel_id: u64, content: &str) -> Result<u64, String>;

    /// Macht aus einer bereits gesendeten Nachricht einen Thread und liefert
    /// dessen Kanal-ID. Binaere ohne Thread-Faehigkeit melden einen Fehler;
    /// die Senke faellt dann auf Einzelnachrichten zurueck.
    async fn create_thread(
        &self,
        channel_id: u64,
        message_id: u64,
        name: &str,
    ) -> Result<u64, String>;
}

/// Queue-Senke: `record` schiebt nur, gesendet wird im Worker.
pub struct QueuedTransparencySink {
    tx: mpsc::Sender<AiInteraction>,
    dropped: Arc<AtomicU64>,
    include_moderation: bool,
}

impl AiTransparencySink for QueuedTransparencySink {
    fn record(&self, interaction: AiInteraction) {
        // Zweiter Riegel neben `accepts`: die Senke darf nichts in den Kanal
        // schieben, was sie nicht annimmt — auch nicht, wenn ein Aufrufer den
        // Record an der Vorpruefung vorbei einreicht.
        if !self.accepts(interaction.use_case) {
            return;
        }
        // try_send statt send: ein blockierender Sender wuerde den
        // chat()-Aufruf aufhalten, den er nur beobachten soll.
        if self.tx.try_send(interaction).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn accepts(&self, use_case: LlmUseCase) -> bool {
        self.include_moderation || !is_moderation_use_case(use_case)
    }
}

/// Laufendes Transparenz-Log: Senke plus Worker.
pub struct TransparencyLog {
    sink: Arc<QueuedTransparencySink>,
    worker: JoinHandle<()>,
}

impl TransparencyLog {
    pub fn spawn(messenger: Arc<dyn TransparencyMessenger>, config: TransparencyConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.queue_capacity.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let sink = Arc::new(QueuedTransparencySink {
            tx,
            dropped: dropped.clone(),
            include_moderation: config.include_moderation,
        });
        let worker = tokio::spawn(run_worker(rx, messenger, config, dropped));
        Self { sink, worker }
    }

    pub fn sink(&self) -> Arc<dyn AiTransparencySink> {
        self.sink.clone() as Arc<dyn AiTransparencySink>
    }

    /// Leert die Queue und beendet den Worker. Kurzlebige Laeufe (Timer-Jobs)
    /// muessen das aufrufen, sonst endet der Prozess vor dem letzten Post.
    pub async fn shutdown(self) {
        crate::transparency::clear_transparency_sink();
        drop(self.sink);
        // Timeout, damit ein haengender Discord-Call den Prozessabschluss
        // nicht blockiert.
        let _ = tokio::time::timeout(Duration::from_secs(20), self.worker).await;
    }
}

/// Zustand eines Gespraechs im Kanal.
struct ConversationState {
    /// Die Nachricht im Kanal, an der der Thread haengt.
    anchor_message_id: Option<u64>,
    thread_id: Option<u64>,
    title: String,
    turn: u32,
}

/// Naeht die einzelnen `chat()`-Aufrufe zu Gespraechen zusammen.
#[derive(Default)]
struct ConversationRouter {
    by_fingerprint: HashMap<ConversationFingerprint, u64>,
    conversations: HashMap<u64, ConversationState>,
    order: VecDeque<u64>,
    next_id: u64,
}

impl ConversationRouter {
    fn resolve(&mut self, interaction: &AiInteraction) -> u64 {
        let existing = interaction
            .conversation_trail
            .iter()
            .find_map(|fingerprint| self.by_fingerprint.get(fingerprint).copied());
        let id = match existing {
            Some(id) if self.conversations.contains_key(&id) => id,
            _ => {
                self.next_id += 1;
                let id = self.next_id;
                self.conversations.insert(
                    id,
                    ConversationState {
                        anchor_message_id: None,
                        thread_id: None,
                        title: thread_title(interaction),
                        turn: 0,
                    },
                );
                self.order.push_back(id);
                self.evict_if_needed();
                id
            }
        };
        for fingerprint in &interaction.conversation_trail {
            self.by_fingerprint.insert(*fingerprint, id);
        }
        if let Some(state) = self.conversations.get_mut(&id) {
            state.turn += 1;
        }
        id
    }

    fn evict_if_needed(&mut self) {
        while self.order.len() > CONVERSATION_MEMORY {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            self.conversations.remove(&old);
            self.by_fingerprint.retain(|_, id| *id != old);
        }
    }
}

async fn run_worker(
    mut rx: mpsc::Receiver<AiInteraction>,
    messenger: Arc<dyn TransparencyMessenger>,
    config: TransparencyConfig,
    dropped: Arc<AtomicU64>,
) {
    let mut router = ConversationRouter::default();
    while let Some(interaction) = rx.recv().await {
        let missed = dropped.swap(0, Ordering::Relaxed);
        if missed > 0 {
            // Stille darf nie wie „keine KI-Aktivitaet" aussehen.
            if let Err(error) = messenger.send(config.channel_id, &drop_notice(missed)).await {
                tracing::warn!(%error, missed, "Hinweis auf verworfene KI-Eintraege nicht zustellbar");
                dropped.fetch_add(missed, Ordering::Relaxed);
            }
            throttle(&config).await;
        }

        let id = router.resolve(&interaction);
        deliver(&mut router, id, &interaction, messenger.as_ref(), &config).await;
        throttle(&config).await;
    }
}

async fn throttle(config: &TransparencyConfig) {
    if !config.min_send_interval.is_zero() {
        tokio::time::sleep(config.min_send_interval).await;
    }
}

async fn deliver(
    router: &mut ConversationRouter,
    id: u64,
    interaction: &AiInteraction,
    messenger: &dyn TransparencyMessenger,
    config: &TransparencyConfig,
) {
    let Some(state) = router.conversations.get(&id) else {
        return;
    };
    let turn = state.turn;
    let mut thread_id = state.thread_id;
    let anchor = state.anchor_message_id;
    let title = state.title.clone();

    // Ab der zweiten Runde gehoert das Gespraech in einen Thread: die erste
    // Frage bleibt als Ankernachricht im Kanal lesbar, der Verlauf haengt
    // darunter.
    if thread_id.is_none() && interaction.is_conversation() {
        if let Some(anchor) = anchor {
            match messenger
                .create_thread(config.channel_id, anchor, &title)
                .await
            {
                Ok(new_thread) => thread_id = Some(new_thread),
                Err(error) => {
                    tracing::warn!(%error, anchor, "KI-Transparenz-Thread nicht anlegbar, Verlauf bleibt im Kanal");
                }
            }
        }
    }

    let content = render_interaction(interaction, turn, thread_id.is_none());
    let target = thread_id.unwrap_or(config.channel_id);
    let sent = messenger.send(target, &content).await;
    let sent = match sent {
        Ok(message_id) => Some(message_id),
        Err(error) if target != config.channel_id => {
            // Der Thread ist weg (archiviert, geloescht). Nichts darf still
            // verschwinden: dann eben in den Kanal.
            tracing::warn!(%error, thread_id = target, "KI-Transparenz-Thread nicht erreichbar, Rueckfall auf den Kanal");
            thread_id = None;
            let fallback = render_interaction(interaction, turn, true);
            match messenger.send(config.channel_id, &fallback).await {
                Ok(message_id) => Some(message_id),
                Err(error) => {
                    tracing::warn!(%error, "KI-Transparenz-Post fehlgeschlagen");
                    None
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, "KI-Transparenz-Post fehlgeschlagen");
            None
        }
    };

    if let Some(state) = router.conversations.get_mut(&id) {
        state.thread_id = thread_id;
        if state.anchor_message_id.is_none() && thread_id.is_none() {
            state.anchor_message_id = sent;
        }
    }
}

pub fn drop_notice(count: u64) -> String {
    format!(
        "**Hinweis:** {count} KI-Interaktionen konnten wegen Überlast nicht angezeigt werden. \
         Die Nachrichten davor und danach sind vollständig."
    )
}

/// Thread-Titel: Anwendungsfall in Klartext plus Stichwort aus der Frage.
pub fn thread_title(interaction: &AiInteraction) -> String {
    let label = use_case_label(interaction.use_case);
    let keyword = interaction
        .prompt_excerpt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let keyword = if keyword.is_empty() {
        "ohne Frage".to_string()
    } else {
        keyword
    };
    let room = THREAD_NAME_LIMIT.saturating_sub(label.chars().count() + 2);
    format!("{label}: {}", cut(&keyword, room))
}

fn cut(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let taken: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{}…", taken.trim_end())
}

/// Kuerzungsstufen: (Kontext, Auslöser). `None` heisst „ungekuerzt", `Some(0)`
/// heisst „Zeile weglassen". Die Antwort wird erst in der letzten Stufe
/// angefasst — sie ist der Teil, den der Owner beurteilen will.
const SHRINK_STAGES: &[(Option<usize>, Option<usize>)] = &[
    (None, None),
    (Some(200), None),
    (Some(0), None),
    (Some(0), Some(500)),
    (Some(0), Some(200)),
    (Some(0), Some(80)),
];

/// Baut die Nachricht fuer den Kanal.
///
/// `turn` ist die Rundennummer im Gespraech, `standalone` sagt, ob die
/// Nachricht direkt im Kanal steht (dann braucht sie die Zuordnung im Text).
pub fn render_interaction(interaction: &AiInteraction, turn: u32, standalone: bool) -> String {
    for (system_budget, prompt_budget) in SHRINK_STAGES {
        let candidate = build_message(
            interaction,
            turn,
            standalone,
            *system_budget,
            *prompt_budget,
            None,
        );
        if candidate.chars().count() <= DISCORD_CONTENT_LIMIT {
            return candidate;
        }
    }
    // Auch die Antwort passt nicht mehr: knapp halten, aber kennzeichnen.
    let skeleton = build_message(interaction, turn, standalone, Some(0), Some(80), Some(0));
    let room = DISCORD_CONTENT_LIMIT.saturating_sub(skeleton.chars().count() + 40);
    build_message(interaction, turn, standalone, Some(0), Some(80), Some(room))
}

fn build_message(
    interaction: &AiInteraction,
    turn: u32,
    standalone: bool,
    system_budget: Option<usize>,
    prompt_budget: Option<usize>,
    answer_budget: Option<usize>,
) -> String {
    let status = if interaction.error.is_some() {
        "Fehler"
    } else {
        "Antwort"
    };
    let runde = if interaction.is_conversation() || turn > 1 {
        format!(" · Runde {turn}")
    } else {
        String::new()
    };
    let zuordnung = if standalone && (interaction.is_conversation() || turn > 1) {
        " · kein Thread verfügbar"
    } else {
        ""
    };
    let mut lines = vec![format!(
        "**KI · {}{runde} · {status}**{zuordnung}",
        use_case_label(interaction.use_case)
    )];

    if let Some(system) = interaction.system_excerpt.as_deref() {
        if system_budget != Some(0) {
            lines.push(format!("Kontext: {}", clip(system, system_budget)));
        }
    }
    lines.push(format!(
        "Auslöser: {}",
        clip(&interaction.prompt_excerpt, prompt_budget)
    ));
    match (&interaction.response, &interaction.error) {
        (Some(response), _) => lines.push(format!("Antwort: {}", clip(response, answer_budget))),
        (None, Some(error)) => lines.push(format!("Fehler: {error}")),
        (None, None) => lines.push("Antwort: (leer)".to_string()),
    }
    lines.push(format!(
        "Modell: {} · Dauer: {} ms",
        interaction.model.as_deref().unwrap_or("unbekannt"),
        interaction.latency_ms
    ));
    lines.join("\n")
}

/// Kuerzt und sagt es dazu. Ohne Kennzeichnung waere eine gekuerzte Antwort
/// nicht von einer abgebrochenen zu unterscheiden.
fn clip(text: &str, budget: Option<usize>) -> String {
    let total = text.chars().count();
    let Some(budget) = budget else {
        return text.to_string();
    };
    if total <= budget {
        return text.to_string();
    }
    let shown: String = text.chars().take(budget).collect();
    format!("{shown}… (gekürzt, {budget} von {total} Zeichen)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::chat_provider::{ChatMessage, ChatParams, ChatProvider, ChatResponse};
    use crate::transparency::TransparencyProvider;

    #[derive(Default)]
    struct FakeMessenger {
        sent: Mutex<Vec<(u64, String)>>,
        threads: Mutex<Vec<(u64, u64, String)>>,
        thread_erlaubt: bool,
        blockiert: Option<Arc<tokio::sync::Semaphore>>,
        next_id: AtomicU64,
    }

    impl FakeMessenger {
        fn arc(thread_erlaubt: bool) -> Arc<Self> {
            Arc::new(Self {
                thread_erlaubt,
                next_id: AtomicU64::new(1000),
                ..Self::default()
            })
        }

        fn blockierend(gate: Arc<tokio::sync::Semaphore>) -> Arc<Self> {
            Arc::new(Self {
                thread_erlaubt: false,
                blockiert: Some(gate),
                next_id: AtomicU64::new(1000),
                ..Self::default()
            })
        }

        fn sent(&self) -> Vec<(u64, String)> {
            self.sent.lock().map(|s| s.clone()).unwrap_or_default()
        }

        fn threads(&self) -> Vec<(u64, u64, String)> {
            self.threads.lock().map(|s| s.clone()).unwrap_or_default()
        }
    }

    #[async_trait]
    impl TransparencyMessenger for FakeMessenger {
        async fn send(&self, channel_id: u64, content: &str) -> Result<u64, String> {
            if let Some(gate) = self.blockiert.as_ref() {
                let permit = gate.acquire().await.map_err(|_| "gate".to_string())?;
                permit.forget();
            }
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut sent) = self.sent.lock() {
                sent.push((channel_id, content.to_string()));
            }
            Ok(id)
        }

        async fn create_thread(
            &self,
            channel_id: u64,
            message_id: u64,
            name: &str,
        ) -> Result<u64, String> {
            if let Ok(mut threads) = self.threads.lock() {
                threads.push((channel_id, message_id, name.to_string()));
            }
            if self.thread_erlaubt {
                Ok(message_id + 500_000)
            } else {
                Err("Threads hier nicht moeglich".to_string())
            }
        }
    }

    fn testkonfig() -> TransparencyConfig {
        TransparencyConfig {
            min_send_interval: Duration::ZERO,
            ..TransparencyConfig::default()
        }
    }

    fn interaktion(use_case: LlmUseCase, frage: &str, antwort: &str, trail: &[u64]) -> AiInteraction {
        AiInteraction {
            use_case,
            prompt_excerpt: frage.to_string(),
            system_excerpt: None,
            response: Some(antwort.to_string()),
            error: None,
            model: Some("test-modell".to_string()),
            latency_ms: 42,
            conversation_trail: trail.to_vec(),
        }
    }

    async fn warte_auf(pruefung: impl Fn() -> bool) -> bool {
        for _ in 0..400 {
            if pruefung() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        pruefung()
    }

    #[tokio::test]
    async fn config_from_env_hat_die_geforderten_defaults() {
        let default = TransparencyConfig::from_env(|_| None);
        assert!(default.enabled, "Transparenz ist per Default an");
        assert_eq!(default.channel_id, 1_374_364_800_817_303_632);
        assert!(
            !default.include_moderation,
            "Moderation ist per Default nicht gespiegelt"
        );

        let gesetzt = TransparencyConfig::from_env(|key| match key {
            "DL_AI_TRANSPARENCY_ENABLED" => Some("aus".to_string()),
            "DL_AI_TRANSPARENCY_CHANNEL_ID" => Some("42".to_string()),
            "DL_AI_TRANSPARENCY_INCLUDE_MODERATION" => Some("1".to_string()),
            _ => None,
        });
        assert!(!gesetzt.enabled);
        assert_eq!(gesetzt.channel_id, 42);
        assert!(gesetzt.include_moderation);
    }

    #[tokio::test]
    async fn moderation_ist_per_default_nicht_im_kanal() {
        let messenger = FakeMessenger::arc(false);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        assert!(
            !sink.accepts(LlmUseCase::ModerationText),
            "ModerationText darf per Default nicht angenommen werden"
        );
        assert!(!sink.accepts(LlmUseCase::ModerationVerify));
        assert!(sink.accepts(LlmUseCase::Faq));

        sink.record(interaktion(
            LlmUseCase::ModerationText,
            "wort",
            "ok",
            &[1],
        ));
        sink.record(interaktion(LlmUseCase::Faq, "frage", "antwort", &[2]));
        assert!(warte_auf(|| messenger.sent().len() == 1).await);

        let sent = messenger.sent();
        assert_eq!(sent.len(), 1, "nur die FAQ-Interaktion darf im Kanal sein");
        assert!(sent[0].1.contains("FAQ"), "{}", sent[0].1);
    }

    #[tokio::test]
    async fn moderation_mit_flag_landet_im_kanal() {
        let messenger = FakeMessenger::arc(false);
        let config = TransparencyConfig {
            include_moderation: true,
            ..testkonfig()
        };
        let log = TransparencyLog::spawn(messenger.clone(), config);
        let sink = log.sink();

        assert!(sink.accepts(LlmUseCase::ModerationText));
        sink.record(interaktion(
            LlmUseCase::ModerationText,
            "wort",
            "sauber",
            &[1],
        ));
        assert!(warte_auf(|| !messenger.sent().is_empty()).await);
        assert!(
            messenger.sent()[0].1.contains("Moderation (Text)"),
            "{}",
            messenger.sent()[0].1
        );
    }

    #[tokio::test]
    async fn verworfene_eintraege_werden_in_der_naechsten_nachricht_ausgewiesen() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let messenger = FakeMessenger::blockierend(gate.clone());
        let config = TransparencyConfig {
            queue_capacity: 2,
            ..testkonfig()
        };
        let log = TransparencyLog::spawn(messenger.clone(), config);
        let sink = log.sink();

        let gesamt = 12_u64;
        for index in 0..gesamt {
            sink.record(interaktion(
                LlmUseCase::Faq,
                &format!("frage {index}"),
                "antwort",
                &[index],
            ));
        }
        gate.add_permits(100);

        assert!(warte_auf(|| {
            let sent = messenger.sent();
            let gemeldet: u64 = sent
                .iter()
                .filter(|(_, text)| text.contains("wegen Überlast"))
                .filter_map(|(_, text)| zahl_aus(text))
                .sum();
            let interaktionen = sent
                .iter()
                .filter(|(_, text)| text.contains("Auslöser:"))
                .count() as u64;
            gemeldet + interaktionen == gesamt
        })
        .await);

        let sent = messenger.sent();
        let hinweise = sent
            .iter()
            .filter(|(_, text)| text.contains("wegen Überlast"))
            .count();
        assert!(
            hinweise > 0,
            "bei voller Queue muss der Verlust ausgewiesen werden, sent={sent:?}"
        );
    }

    fn zahl_aus(text: &str) -> Option<u64> {
        text.split_whitespace()
            .find_map(|wort| wort.parse::<u64>().ok())
    }

    #[tokio::test]
    async fn klemmende_senke_verzoegert_chat_nicht() {
        struct Sofort;
        #[async_trait]
        impl ChatProvider for Sofort {
            async fn chat(
                &self,
                _messages: &[ChatMessage],
                _params: ChatParams,
            ) -> Result<ChatResponse, crate::chat_provider::ChatProviderError> {
                Ok(ChatResponse::text("schnell"))
            }
        }

        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let messenger = FakeMessenger::blockierend(gate.clone());
        let config = TransparencyConfig {
            queue_capacity: 1,
            ..testkonfig()
        };
        let log = TransparencyLog::spawn(messenger.clone(), config);
        let provider = TransparencyProvider::new(Arc::new(Sofort), log.sink(), LlmUseCase::Faq);

        let start = std::time::Instant::now();
        for index in 0..50 {
            let antwort = provider
                .chat(
                    &[ChatMessage::user(format!("frage {index}"))],
                    ChatParams::default(),
                )
                .await
                .expect("Antwort");
            assert_eq!(antwort.content, "schnell");
        }
        let dauer = start.elapsed();
        assert!(
            dauer < Duration::from_millis(500),
            "50 Aufrufe mit klemmender Senke brauchten {dauer:?} — record() blockiert"
        );
        gate.add_permits(100);
    }

    #[tokio::test]
    async fn konversation_landet_ab_der_zweiten_runde_im_thread() {
        let messenger = FakeMessenger::arc(true);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        sink.record(interaktion(
            LlmUseCase::BotPate,
            "Wie finde ich Mitspieler?",
            "Schau in den LFG-Kanal.",
            &[7],
        ));
        assert!(warte_auf(|| messenger.sent().len() == 1).await);
        sink.record(interaktion(
            LlmUseCase::BotPate,
            "Und wenn niemand antwortet?",
            "Dann melde dich im Voice.",
            &[7, 8],
        ));
        sink.record(interaktion(
            LlmUseCase::BotPate,
            "Danke!",
            "Gerne.",
            &[7, 8, 9],
        ));
        assert!(warte_auf(|| messenger.sent().len() == 3).await);

        let sent = messenger.sent();
        let kanal = DEFAULT_TRANSPARENCY_CHANNEL_ID;
        assert_eq!(sent[0].0, kanal, "die erste Runde bleibt im Kanal");
        assert_ne!(sent[1].0, kanal, "ab Runde 2 geht es in den Thread");
        assert_eq!(sent[2].0, sent[1].0, "Runde 3 in denselben Thread");

        let threads = messenger.threads();
        assert_eq!(threads.len(), 1, "genau ein Thread je Gespraech");
        assert!(
            threads[0].2.starts_with("Concierge/Pate:"),
            "Titel nennt den Anwendungsfall: {}",
            threads[0].2
        );
        assert!(
            threads[0].2.contains("Mitspieler"),
            "Titel nennt ein Stichwort der ersten Frage: {}",
            threads[0].2
        );
        assert!(sent[1].1.contains("Runde 2"), "{}", sent[1].1);
    }

    #[tokio::test]
    async fn einzelschuesse_erzeugen_keinen_thread() {
        let messenger = FakeMessenger::arc(true);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        for index in 0..3_u64 {
            sink.record(interaktion(
                LlmUseCase::Faq,
                &format!("frage {index}"),
                "antwort",
                &[index],
            ));
        }
        assert!(warte_auf(|| messenger.sent().len() == 3).await);

        assert!(messenger.threads().is_empty(), "kein leerer Thread");
        for (channel, _) in messenger.sent() {
            assert_eq!(channel, DEFAULT_TRANSPARENCY_CHANNEL_ID);
        }
    }

    #[tokio::test]
    async fn ohne_thread_faehigkeit_bleibt_der_verlauf_im_kanal() {
        let messenger = FakeMessenger::arc(false);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        sink.record(interaktion(LlmUseCase::BotPate, "erste", "a1", &[7]));
        assert!(warte_auf(|| messenger.sent().len() == 1).await);
        sink.record(interaktion(LlmUseCase::BotPate, "zweite", "a2", &[7, 8]));
        assert!(warte_auf(|| messenger.sent().len() == 2).await);

        let sent = messenger.sent();
        assert_eq!(sent[1].0, DEFAULT_TRANSPARENCY_CHANNEL_ID);
        assert!(
            sent[1].1.contains("kein Thread verfügbar"),
            "die Zuordnung muss trotzdem erkennbar sein: {}",
            sent[1].1
        );
    }

    #[test]
    fn lange_texte_kuerzen_erst_den_kontext_dann_den_auslöser() {
        let interaction = AiInteraction {
            use_case: LlmUseCase::Faq,
            prompt_excerpt: "P".repeat(3_000),
            system_excerpt: Some("S".repeat(3_000)),
            response: Some("A".repeat(1_200)),
            error: None,
            model: Some("m".to_string()),
            latency_ms: 5,
            conversation_trail: vec![1],
        };

        let text = render_interaction(&interaction, 1, true);
        assert!(
            text.chars().count() <= DISCORD_CONTENT_LIMIT,
            "{} Zeichen",
            text.chars().count()
        );
        assert!(
            text.contains(&"A".repeat(1_200)),
            "die Antwort muss vollstaendig bleiben"
        );
        assert!(!text.contains("Kontext:"), "der Kontext faellt zuerst weg");
        assert!(text.contains("gekürzt"), "jede Kuerzung wird gekennzeichnet");
    }

    #[test]
    fn eine_riesige_antwort_wird_zuletzt_gekuerzt_und_gekennzeichnet() {
        let interaction = AiInteraction {
            use_case: LlmUseCase::Faq,
            prompt_excerpt: "P".repeat(3_000),
            system_excerpt: Some("S".repeat(3_000)),
            response: Some("A".repeat(9_000)),
            error: None,
            model: Some("m".to_string()),
            latency_ms: 5,
            conversation_trail: vec![1],
        };

        let text = render_interaction(&interaction, 1, true);
        assert!(
            text.chars().count() <= DISCORD_CONTENT_LIMIT,
            "{} Zeichen",
            text.chars().count()
        );
        assert!(text.contains("Antwort:"));
        assert!(text.contains("gekürzt"));
    }

    #[test]
    fn fehler_werden_als_fehler_ausgewiesen() {
        let interaction = AiInteraction {
            use_case: LlmUseCase::BotPate,
            prompt_excerpt: "Frage".to_string(),
            system_excerpt: None,
            response: None,
            error: Some("LLM provider rate limited".to_string()),
            model: None,
            latency_ms: 1_200,
            conversation_trail: vec![1],
        };
        let text = render_interaction(&interaction, 1, true);
        assert!(text.contains("Fehler"), "{text}");
        assert!(text.contains("rate limited"), "{text}");
        assert!(text.contains("unbekannt"), "{text}");
    }

    #[test]
    fn thread_titel_bleibt_unter_der_discord_grenze() {
        let interaction = interaktion(LlmUseCase::CoachingAnfrage, &"W".repeat(400), "a", &[1]);
        let titel = thread_title(&interaction);
        assert!(titel.chars().count() <= 100, "{} Zeichen", titel.chars().count());
        assert!(titel.starts_with("Coaching-Anfrage:"));
    }

    #[test]
    fn inventarzeile_nennt_kanal_und_moderationsstand() {
        let line = TransparencyConfig::default().inventory_line();
        assert!(line.contains("1374364800817303632"), "{line}");
        assert!(line.contains("Moderation gespiegelt: nein"), "{line}");

        let aus = TransparencyConfig {
            enabled: false,
            ..TransparencyConfig::default()
        };
        assert!(aus.inventory_line().contains("aus"), "{}", aus.inventory_line());
    }
}
