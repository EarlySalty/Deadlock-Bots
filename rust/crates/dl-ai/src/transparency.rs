//! KI-Transparenz: der Vertrag, mit dem jede Modellantwort sichtbar wird.
//!
//! Die Community-Leitung soll jede Antwort lesen koennen, die eine KI im Bot
//! gibt. Statt vierzehn Aufrufstellen einzeln zu instrumentieren, haengt hier
//! ein Decorator vor jedem [`ChatProvider`], den
//! [`crate::LlmProviderConfig::build_provider_for_env`] ausliefert. Damit ist
//! jeder Anwendungsfall erfasst, der ueber die gepruefte Fabrik laeuft.
//!
//! Drei Regeln haengen an dieser Datei:
//! - Das Melden darf einen `chat()`-Aufruf niemals verzoegern oder scheitern
//!   lassen. [`AiTransparencySink::record`] ist deshalb synchron und muss in
//!   der Implementierung sofort zurueckkehren.
//! - Ein KI-Ausfall ist selbst ein Befund, den der Kanal zeigen soll. Der
//!   Fehlerfall erzeugt darum genauso einen Record wie der Erfolg.
//! - Kein Sampling: jede erfolgreiche Antwort wird einzeln sichtbar. Nur
//!   wiederholte identische Fehler fasst die Senke zusammen, und auch dann
//!   wird jeder Fehler mindestens einmal einzeln gezeigt und jede
//!   Wiederholung als Anzahl ausgewiesen. Siehe
//!   [`crate::transparency_log`].

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use async_trait::async_trait;

use crate::chat_provider::{
    ChatMessage, ChatParams, ChatProvider, ChatProviderError, ChatResponse, ChatRole, LlmUseCase,
};

/// Fingerabdruck einer einzelnen Nutzer-Nachricht im uebergebenen Verlauf.
///
/// Der Wert ist rein inhaltsabgeleitet: derselbe Text im selben Anwendungsfall
/// ergibt bei jedem Aufruf denselben Fingerabdruck. Genau daraus naeht die
/// Senke zusammen, welche `chat()`-Aufrufe zur selben Konversation gehoeren,
/// ohne dass eine der vierzehn Aufrufstellen eine Session-ID durchreichen
/// muss.
pub type ConversationFingerprint = u64;

/// Eine einzelne KI-Interaktion, so wie sie im Transparenz-Kanal landet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiInteraction {
    pub use_case: LlmUseCase,
    /// Der Auslöser im Wortlaut: die letzte Nachricht mit User-Rolle, sonst
    /// die letzte Nachricht überhaupt.
    pub prompt_excerpt: String,
    pub system_excerpt: Option<String>,
    /// Die KI-Antwort im Wortlaut; `None`, wenn der Aufruf fehlschlug.
    pub response: Option<String>,
    pub error: Option<String>,
    /// Das Modell, unter dem der Aufruf angefragt wurde
    /// ([`ChatProvider::effective_model`]). Zugleich der Schluessel, an dem
    /// die Entprellung im Transparenz-Log ihre Serien wiedererkennt: er muss
    /// im Erfolgs- und im Fehlerfall aus derselben Quelle kommen.
    pub model: Option<String>,
    /// Das Modell, das laut Antwort-Body wirklich geantwortet hat. `None` im
    /// Fehlerfall und bei Anbietern, die es nicht zurueckmelden. Nur zur
    /// Anzeige, nie als Schluessel: sonst faende eine geglueckte Antwort die
    /// Serie ihres eigenen Ausfalls nicht wieder.
    pub antwort_modell: Option<String>,
    pub latency_ms: u64,
    /// Fingerabdrücke aller Nutzer-Nachrichten des übergebenen Verlaufs,
    /// älteste zuerst. Ein Verlauf mit mehr als einem Eintrag ist eine
    /// Konversation, ein einzelner Eintrag ist ein Einzelschuss.
    pub conversation_trail: Vec<ConversationFingerprint>,
}

impl AiInteraction {
    /// Gehört diese Interaktion zu einem mehrrundigen Gespräch?
    pub fn is_conversation(&self) -> bool {
        self.conversation_trail.len() > 1
    }
}

/// Senke fuer KI-Interaktionen.
///
/// `record` laeuft im heissen Pfad eines `chat()`-Aufrufs: die Implementierung
/// schiebt in eine Queue und kehrt sofort zurueck, sie sendet nie selbst.
pub trait AiTransparencySink: Send + Sync {
    fn record(&self, interaction: AiInteraction);

    /// Wird dieser Anwendungsfall ueberhaupt gemeldet? Ein `false` spart dem
    /// Decorator die gesamte Aufbereitung.
    fn accepts(&self, _use_case: LlmUseCase) -> bool {
        true
    }
}

static GLOBAL_SINK: RwLock<Option<Arc<dyn AiTransparencySink>>> = RwLock::new(None);

/// Registriert die Senke global, damit
/// [`crate::LlmProviderConfig::build_provider_for_env`] sie erreicht, ohne die
/// Signatur aller Aufrufstellen zu aendern.
pub fn set_transparency_sink(sink: Arc<dyn AiTransparencySink>) {
    let mut guard = GLOBAL_SINK
        .write()
        .unwrap_or_else(|poison| poison.into_inner());
    *guard = Some(sink);
}

/// Entfernt die Senke wieder (Shutdown eines kurzlebigen Laufs, Tests).
pub fn clear_transparency_sink() {
    let mut guard = GLOBAL_SINK
        .write()
        .unwrap_or_else(|poison| poison.into_inner());
    *guard = None;
}

pub fn transparency_sink() -> Option<Arc<dyn AiTransparencySink>> {
    GLOBAL_SINK
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

/// Leitet an die global registrierte Senke weiter.
///
/// Die Aufloesung passiert bei jedem Aufruf, nicht beim Bauen des Providers:
/// sonst haette die Reihenfolge von Provider-Bau und Senken-Registrierung im
/// Start eines Binaers daraeber entschieden, ob ueberhaupt etwas geloggt wird.
struct GlobalSink;

impl AiTransparencySink for GlobalSink {
    fn record(&self, interaction: AiInteraction) {
        if let Some(sink) = transparency_sink() {
            sink.record(interaction);
        }
    }

    fn accepts(&self, use_case: LlmUseCase) -> bool {
        transparency_sink().is_some_and(|sink| sink.accepts(use_case))
    }
}

/// Legt den Transparenz-Decorator um einen Provider.
///
/// Ohne registrierte Senke lehnt [`GlobalSink::accepts`] jeden Anwendungsfall
/// ab; der Decorator reicht dann ohne jede Aufbereitung durch.
pub fn wrap_with_transparency(
    inner: Arc<dyn ChatProvider>,
    use_case: LlmUseCase,
) -> Arc<dyn ChatProvider> {
    TransparencyProvider::new(inner, Arc::new(GlobalSink), use_case) as Arc<dyn ChatProvider>
}

pub struct TransparencyProvider {
    inner: Arc<dyn ChatProvider>,
    sink: Arc<dyn AiTransparencySink>,
    use_case: LlmUseCase,
}

impl TransparencyProvider {
    pub fn new(
        inner: Arc<dyn ChatProvider>,
        sink: Arc<dyn AiTransparencySink>,
        use_case: LlmUseCase,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner,
            sink,
            use_case,
        })
    }
}

#[async_trait]
impl ChatProvider for TransparencyProvider {
    fn effective_model(&self, params: &ChatParams) -> Option<String> {
        self.inner.effective_model(params)
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        params: ChatParams,
    ) -> Result<ChatResponse, ChatProviderError> {
        if !self.sink.accepts(self.use_case) {
            return self.inner.chat(messages, params).await;
        }

        let prompt_excerpt = redact_secrets(&prompt_excerpt(messages));
        let system_excerpt = system_excerpt(messages, params.system_prompt.as_deref())
            .map(|text| redact_secrets(&text));
        let conversation_trail = conversation_trail(self.use_case, messages);
        // Eine Quelle fuer beide Zweige.
        //
        // Frueher stand im Erfolgsfall das Modell aus dem Antwort-Body und im
        // Fehlerfall `params.model`. Weil fast jeder Aufrufer
        // `ChatParams::default()` benutzt, hiess das: Erfolg `Some(...)`,
        // Fehler `None`. Die Entprellung im Transparenz-Log schluesselt aber
        // ueber genau dieses Feld, also hat eine geglueckte Antwort die Serie
        // desselben Anbieters nie wiedergefunden und nie beendet: das Backoff
        // lief bis zum Deckel und blieb dort.
        //
        // Der Body bleibt deshalb aussen vor. Er kommt im Fehlerfall nicht an
        // und kann die Symmetrie darum nicht tragen; `effective_model` kann
        // sie, weil der Provider sie vor dem Aufruf kennt.
        let effective_model = self.inner.effective_model(&params);

        let started = Instant::now();
        let result = self.inner.chat(messages, params).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let private_image = messages.iter().any(|message| {
            message.role == ChatRole::User
                && serde_json::from_str::<serde_json::Value>(&message.content).is_ok_and(
                    |payload| {
                        payload
                            .get("image_context")
                            .is_some_and(serde_json::Value::is_string)
                    },
                )
        });
        let mut interaction = match &result {
            Ok(response) => AiInteraction {
                use_case: self.use_case,
                prompt_excerpt,
                system_excerpt,
                response: Some(redact_secrets(&response.content)),
                error: None,
                // Das angefragte Modell bleibt der Schluessel (siehe oben).
                // Was tatsaechlich geantwortet hat, steht daneben: loest der
                // Anbieter einen Alias auf ein anderes Modell auf, ist genau
                // das die Information, wegen der es den Kanal gibt.
                antwort_modell: response.model.clone(),
                model: effective_model,
                latency_ms,
                conversation_trail,
            },
            Err(error) => AiInteraction {
                use_case: self.use_case,
                prompt_excerpt,
                system_excerpt,
                response: None,
                // Der Fehlertext des Anbieters ist das einzige Feld, das frueher
                // ungefiltert in den Kanal ging. Manche Anbieter spiegeln bei
                // 400 den beanstandeten Nachrichteninhalt zurueck; schreibt ein
                // Nutzer seinen Schluessel in den Chat, stand er im `Fehler:`
                // im Klartext, waehrend im `Ausloeser:` sauber `[redigiert]`
                // stand.
                //
                // Dies ist der einzige Pfad, der den vollen Wortlaut lesen
                // darf: `Display` liefert nur den Statusteil, siehe
                // [`ChatProviderError::transparenz_wortlaut`].
                error: Some(redact_secrets(&error.transparenz_wortlaut())),
                // Im Fehlerfall gibt es keinen Antwort-Body und damit kein
                // geantwortetes Modell.
                antwort_modell: None,
                model: effective_model,
                latency_ms,
                conversation_trail,
            },
        };
        if private_image {
            interaction.prompt_excerpt = "Brain-Bildfrage (Inhalt nicht protokolliert)".into();
            interaction.system_excerpt = None;
            interaction.conversation_trail.clear();
            interaction.response = interaction
                .response
                .map(|_| "Bildantwort (Inhalt nicht protokolliert)".into());
            interaction.error = result.as_ref().err().map(ToString::to_string);
        }
        self.sink.record(interaction);
        result
    }
}

fn prompt_excerpt(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.role == ChatRole::User)
        .or_else(|| messages.last())
        .map(|message| message.content.clone())
        .unwrap_or_default()
}

fn system_excerpt(messages: &[ChatMessage], system_prompt: Option<&str>) -> Option<String> {
    system_prompt
        .map(ToString::to_string)
        .or_else(|| {
            messages
                .iter()
                .rev()
                .find(|message| message.role == ChatRole::System)
                .map(|message| message.content.clone())
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Fingerabdruecke aller Nutzer-Nachrichten, aelteste zuerst.
///
/// Der Concierge reicht seinen Verlauf als gleitendes Fenster durch
/// (`bot.concierge_conversations`, LIMIT 12). Zwei aufeinanderfolgende Runden
/// teilen sich deshalb immer mindestens eine Nutzer-Nachricht, und ueber diese
/// Ueberlappung findet die Senke den bereits angelegten Thread wieder, auch
/// wenn die aelteste Nachricht aus dem Fenster gefallen ist.
fn conversation_trail(
    use_case: LlmUseCase,
    messages: &[ChatMessage],
) -> Vec<ConversationFingerprint> {
    messages
        .iter()
        .filter(|message| message.role == ChatRole::User)
        .map(|message| fingerprint(use_case, &message.content))
        .collect()
}

fn fingerprint(use_case: LlmUseCase, content: &str) -> ConversationFingerprint {
    let mut hasher = DefaultHasher::new();
    use_case.as_str().hash(&mut hasher);
    content.trim().hash(&mut hasher);
    hasher.finish()
}

/// Moderation laeuft auf praktisch jeder Discord-Nachricht und hat bereits ein
/// eigenes, vom Owner abgenommenes Log. Eine Spiegelung in den
/// Transparenz-Kanal waere eine Flut und liefe in Discords Rate-Limit; sie ist
/// deshalb per Default aus und nur per Flag
/// (`DL_AI_TRANSPARENCY_INCLUDE_MODERATION`) zuschaltbar.
pub fn is_moderation_use_case(use_case: LlmUseCase) -> bool {
    matches!(
        use_case,
        LlmUseCase::ModerationText | LlmUseCase::ModerationVerify
    )
}

/// Klartextname des Anwendungsfalls fuer die Anzeige im Kanal.
pub fn use_case_label(use_case: LlmUseCase) -> &'static str {
    match use_case {
        LlmUseCase::BotPate => "Concierge/Pate",
        LlmUseCase::Faq => "FAQ",
        LlmUseCase::LfgFreitext => "LFG-Freitext",
        LlmUseCase::ScrimLagebild => "Scrim-Lagebild",
        LlmUseCase::VerbinderMatch => "Verbinder-Match",
        LlmUseCase::VerbinderKritik => "Verbinder-Kritik",
        LlmUseCase::AiOnboarding => "Onboarding",
        LlmUseCase::BrainAntwort => "Brain-Antwort",
        LlmUseCase::CoachingAnfrage => "Coaching-Anfrage",
        LlmUseCase::ModerationText => "Moderation (Text)",
        LlmUseCase::ModerationVerify => "Moderation (Prüfung)",
        LlmUseCase::StreamerMatcher => "Streamer-Matcher",
        LlmUseCase::TurnierVorschlag => "Turnier-Vorschlag",
        LlmUseCase::VoiceHint => "Voice-Hinweis",
    }
}

const REDACTED: &str = "[redigiert]";

/// Prefixe, an denen ein Zugangsschluessel eindeutig zu erkennen ist.
const SECRET_PREFIXES: &[&str] = &[
    "sk-",
    "sk_",
    "rk_",
    "fw_",
    "xoxb-",
    "xoxp-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "github_pat_",
    "hf_",
    "AKIA",
    "AIza",
    "mfa.",
    "eyJhbGciOi",
];

/// Schluesselnamen, hinter denen ein `=` oder `:` nie in den Kanal darf.
const SECRET_KEY_HINTS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwort",
    "api_key",
    "apikey",
    "api-key",
    "private_key",
    "dsn",
    "credential",
];

/// Woerter, nach denen das *naechste* Wort der Schluessel ist.
const SECRET_LEAD_WORDS: &[&str] = &["bearer", "bot", "authorization:", "x-api-key:", "basic"];

/// Entfernt Zugangsdaten aus einem Text, bevor er in den Kanal geht.
///
/// Bewusst konservativ: erkannt wird, was eindeutig nach Zugangsdaten aussieht
/// (bekannte Prefixe, `schluessel=wert`, `Bearer <wert>`, Verbindungs-URLs mit
/// Passwort). Freitext bleibt unangetastet, sonst waere das Log als
/// Arbeitsgrundlage wertlos.
pub fn redact_secrets(text: &str) -> String {
    text.split('\n')
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_line(line: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut previous_is_lead = false;
    for word in line.split(' ') {
        let redacted = if previous_is_lead && looks_like_value(word) {
            REDACTED.to_string()
        } else {
            redact_word(word)
        };
        previous_is_lead = SECRET_LEAD_WORDS.contains(&word.trim().to_ascii_lowercase().as_str());
        out.push(redacted);
    }
    out.join(" ")
}

fn looks_like_value(word: &str) -> bool {
    word.trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .chars()
        .count()
        >= 8
}

fn redact_word(word: &str) -> String {
    let trimmed = word.trim_end_matches([',', ';', '.', ')', ']', '"', '\'']);
    let tail = &word[trimmed.len()..];

    if let Some(redacted) = redact_connection_url(trimmed) {
        return format!("{redacted}{tail}");
    }
    if SECRET_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix) && trimmed.chars().count() > prefix.len() + 6)
    {
        return format!("{REDACTED}{tail}");
    }
    if let Some((key, value)) = split_assignment(trimmed) {
        let lowered = key.to_ascii_lowercase();
        if SECRET_KEY_HINTS.iter().any(|hint| lowered.contains(hint)) && !value.is_empty() {
            return format!("{key}={REDACTED}{tail}");
        }
    }
    word.to_string()
}

fn split_assignment(word: &str) -> Option<(&str, &str)> {
    let index = word.find('=')?;
    Some((&word[..index], &word[index + 1..]))
}

/// `schema://benutzer:passwort@host/db` — das Passwort muss weg, der Rest
/// darf stehen bleiben, sonst ist die Meldung nicht mehr zuzuordnen.
fn redact_connection_url(word: &str) -> Option<String> {
    let scheme_end = word.find("://")?;
    let rest = &word[scheme_end + 3..];
    let at = rest.find('@')?;
    let userinfo = &rest[..at];
    let colon = userinfo.find(':')?;
    Some(format!(
        "{}://{}:{REDACTED}@{}",
        &word[..scheme_end],
        &userinfo[..colon],
        &rest[at + 1..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::chat_provider::TokenUsage;

    #[derive(Default)]
    struct RecordingSink {
        records: Mutex<Vec<AiInteraction>>,
    }

    impl RecordingSink {
        fn arc() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn records(&self) -> Vec<AiInteraction> {
            self.records
                .lock()
                .map(|records| records.clone())
                .unwrap_or_default()
        }
    }

    impl AiTransparencySink for RecordingSink {
        fn record(&self, interaction: AiInteraction) {
            if let Ok(mut records) = self.records.lock() {
                records.push(interaction);
            }
        }
    }

    struct FixedProvider(Result<ChatResponse, ChatProviderError>);

    #[async_trait]
    impl ChatProvider for FixedProvider {
        /// Wie jeder echte Provider: er kennt sein Modell, auch wenn der
        /// Aufrufer keins gesetzt hat. Ohne das prueft der Test einen
        /// Zustand, den es in der Verdrahtung nicht gibt.
        fn effective_model(&self, params: &ChatParams) -> Option<String> {
            Some(
                params
                    .model
                    .clone()
                    .unwrap_or_else(|| "test-modell".to_string()),
            )
        }

        async fn chat(
            &self,
            _messages: &[ChatMessage],
            _params: ChatParams,
        ) -> Result<ChatResponse, ChatProviderError> {
            self.0.clone()
        }
    }

    fn antwort(content: &str) -> ChatResponse {
        ChatResponse {
            content: content.to_string(),
            model: Some("test-modell".to_string()),
            usage: TokenUsage::default(),
        }
    }

    #[tokio::test]
    async fn brain_image_context_and_answer_do_not_enter_content_log() {
        let inner = Arc::new(FixedProvider(Ok(antwort("PRIVATE_IMAGE_OBSERVATION"))));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink.clone(), LlmUseCase::BotPate);
        let payload = serde_json::json!({"question":"PRIVATE_IMAGE_QUESTION","image_context":"PRIVATE_IMAGE_OBSERVATION"});
        let response = provider
            .chat(
                &[ChatMessage::user(payload.to_string())],
                ChatParams::default(),
            )
            .await
            .expect("unveränderte Bildantwort");
        assert_eq!(response.content, "PRIVATE_IMAGE_OBSERVATION");
        let records = sink.records();
        assert_eq!(records.len(), 1);
        assert!(!records[0].prompt_excerpt.contains("PRIVATE_IMAGE"));
        assert!(!records[0]
            .response
            .as_deref()
            .expect("inhaltlich ausgeblendete Antwort im Protokoll")
            .contains("PRIVATE_IMAGE"));
        assert!(records[0].system_excerpt.is_none());
        assert!(records[0].conversation_trail.is_empty());
        assert!(records[0].model.is_some());
    }

    #[tokio::test]
    async fn antwort_wird_unveraendert_durchgereicht() {
        let inner = Arc::new(FixedProvider(Ok(antwort("Hallo aus dem Modell"))));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink, LlmUseCase::Faq);

        let result = provider
            .chat(&[ChatMessage::user("Frage")], ChatParams::default())
            .await
            .expect("Antwort");

        assert_eq!(result, antwort("Hallo aus dem Modell"));
    }

    #[tokio::test]
    async fn fehler_wird_unveraendert_durchgereicht() {
        let inner = Arc::new(FixedProvider(Err(ChatProviderError::RateLimit)));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink, LlmUseCase::Faq);

        let error = provider
            .chat(&[ChatMessage::user("Frage")], ChatParams::default())
            .await
            .expect_err("Fehler");

        assert_eq!(error, ChatProviderError::RateLimit);
    }

    #[tokio::test]
    async fn erfolg_erzeugt_genau_einen_record_im_wortlaut() {
        let inner = Arc::new(FixedProvider(Ok(antwort("Die volle Antwort"))));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink.clone(), LlmUseCase::BotPate);

        provider
            .chat(
                &[
                    ChatMessage::system("Systemregeln"),
                    ChatMessage::user("Die volle Frage"),
                ],
                ChatParams::default(),
            )
            .await
            .expect("Antwort");

        let records = sink.records();
        assert_eq!(records.len(), 1, "Erfolg muss genau einen Record erzeugen");
        assert_eq!(records[0].use_case, LlmUseCase::BotPate);
        assert_eq!(records[0].prompt_excerpt, "Die volle Frage");
        assert_eq!(records[0].response.as_deref(), Some("Die volle Antwort"));
        assert_eq!(records[0].error, None);
        assert_eq!(records[0].model.as_deref(), Some("test-modell"));
        assert_eq!(records[0].system_excerpt.as_deref(), Some("Systemregeln"));
    }

    #[tokio::test]
    async fn fehler_erzeugt_genau_einen_record_mit_fehlertext() {
        let inner = Arc::new(FixedProvider(Err(ChatProviderError::Timeout)));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink.clone(), LlmUseCase::VerbinderMatch);

        let _ = provider
            .chat(&[ChatMessage::user("Frage")], ChatParams::default())
            .await;

        let records = sink.records();
        assert_eq!(records.len(), 1, "Fehler muss genau einen Record erzeugen");
        assert_eq!(records[0].response, None);
        assert_eq!(
            records[0].error.as_deref(),
            Some("LLM provider request timed out")
        );
    }

    #[tokio::test]
    async fn nur_der_transparenz_pfad_sieht_den_anbieter_wortlaut() {
        // Der Kern des Funds: `Display` gibt den Wortlaut des Anbieters nicht
        // mehr her, damit ihn keine Logstelle mit `%error` versehentlich ins
        // Anwendungslog schreibt. Der Kanal braucht ihn und holt ihn sich
        // ueber den einen erlaubten Weg.
        let hinweis = crate::chat_provider::anbieter_hinweis(
            400,
            r#"{"error":{"message":"verbotenes wort: sk-geheim-123"}}"#,
        );
        let fehler = ChatProviderError::Provider(hinweis);
        assert!(
            !fehler.to_string().contains("verbotenes wort"),
            "Display darf den Wortlaut nicht tragen: {fehler}"
        );

        let inner = Arc::new(FixedProvider(Err(fehler)));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink.clone(), LlmUseCase::Faq);

        let _ = provider
            .chat(&[ChatMessage::user("Frage")], ChatParams::default())
            .await;

        let records = sink.records();
        let text = records[0].error.as_deref().expect("Fehlertext");
        assert!(
            text.contains("verbotenes wort"),
            "der Kanal braucht den vollen Wortlaut: {text}"
        );
        assert!(
            text.contains("HTTP 400"),
            "der Statusteil bleibt vorne: {text}"
        );
    }

    #[tokio::test]
    async fn der_record_traegt_das_modell_aus_dem_antwort_body() {
        // `model` bleibt der Schluessel (angefragtes Modell), `antwort_modell`
        // zeigt, was wirklich geantwortet hat.
        let mut response = antwort("ok");
        response.model = Some("aufgeloestes-modell".to_string());
        let inner = Arc::new(FixedProvider(Ok(response)));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink.clone(), LlmUseCase::Faq);

        provider
            .chat(&[ChatMessage::user("Frage")], ChatParams::default())
            .await
            .expect("Antwort");

        let records = sink.records();
        assert_eq!(records[0].model.as_deref(), Some("test-modell"));
        assert_eq!(
            records[0].antwort_modell.as_deref(),
            Some("aufgeloestes-modell")
        );
    }

    #[tokio::test]
    async fn systemprompt_aus_den_params_landet_im_record() {
        let inner = Arc::new(FixedProvider(Ok(antwort("ok"))));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink.clone(), LlmUseCase::Faq);

        let params = ChatParams {
            system_prompt: Some("Du bist knapp".to_string()),
            ..ChatParams::default()
        };
        provider
            .chat(&[ChatMessage::user("Frage")], params)
            .await
            .expect("Antwort");

        assert_eq!(
            sink.records()[0].system_excerpt.as_deref(),
            Some("Du bist knapp")
        );
    }

    #[tokio::test]
    async fn ohne_user_nachricht_zaehlt_die_letzte_nachricht() {
        let inner = Arc::new(FixedProvider(Ok(antwort("ok"))));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink.clone(), LlmUseCase::ScrimLagebild);

        provider
            .chat(
                &[
                    ChatMessage::system("Regeln"),
                    ChatMessage::assistant("Letzter Stand"),
                ],
                ChatParams::default(),
            )
            .await
            .expect("Antwort");

        assert_eq!(sink.records()[0].prompt_excerpt, "Letzter Stand");
    }

    #[tokio::test]
    async fn verlauf_ergibt_eine_stabile_konversationsspur() {
        let sink = RecordingSink::arc();
        let einzel = TransparencyProvider::new(
            Arc::new(FixedProvider(Ok(antwort("a1")))),
            sink.clone(),
            LlmUseCase::BotPate,
        );
        einzel
            .chat(
                &[ChatMessage::system("s"), ChatMessage::user("Runde 1")],
                ChatParams::default(),
            )
            .await
            .expect("Antwort");

        let zweite = TransparencyProvider::new(
            Arc::new(FixedProvider(Ok(antwort("a2")))),
            sink.clone(),
            LlmUseCase::BotPate,
        );
        zweite
            .chat(
                &[
                    ChatMessage::system("s"),
                    ChatMessage::user("Runde 1"),
                    ChatMessage::assistant("a1"),
                    ChatMessage::user("Runde 2"),
                ],
                ChatParams::default(),
            )
            .await
            .expect("Antwort");

        let records = sink.records();
        assert_eq!(records[0].conversation_trail.len(), 1);
        assert!(
            !records[0].is_conversation(),
            "erste Runde ist Einzelschuss"
        );
        assert_eq!(records[1].conversation_trail.len(), 2);
        assert!(records[1].is_conversation());
        assert_eq!(
            records[1].conversation_trail[0], records[0].conversation_trail[0],
            "die Ueberlappung muss beide Runden verbinden"
        );
    }

    #[tokio::test]
    async fn gleicher_text_in_anderem_use_case_ist_eine_andere_konversation() {
        let sink = RecordingSink::arc();
        for use_case in [LlmUseCase::Faq, LlmUseCase::BotPate] {
            let provider = TransparencyProvider::new(
                Arc::new(FixedProvider(Ok(antwort("ok")))),
                sink.clone(),
                use_case,
            );
            provider
                .chat(&[ChatMessage::user("gleiche Frage")], ChatParams::default())
                .await
                .expect("Antwort");
        }
        let records = sink.records();
        assert_ne!(
            records[0].conversation_trail[0],
            records[1].conversation_trail[0]
        );
    }

    #[tokio::test]
    async fn abgelehnter_use_case_erzeugt_keinen_record_und_reicht_trotzdem_durch() {
        struct AblehnendeSenke(Arc<RecordingSink>);
        impl AiTransparencySink for AblehnendeSenke {
            fn record(&self, interaction: AiInteraction) {
                self.0.record(interaction);
            }
            fn accepts(&self, _use_case: LlmUseCase) -> bool {
                false
            }
        }

        let inner = Arc::new(FixedProvider(Ok(antwort("durchgereicht"))));
        let recorder = RecordingSink::arc();
        let provider = TransparencyProvider::new(
            inner,
            Arc::new(AblehnendeSenke(recorder.clone())),
            LlmUseCase::ModerationText,
        );

        let result = provider
            .chat(&[ChatMessage::user("Frage")], ChatParams::default())
            .await
            .expect("Antwort");

        assert_eq!(result.content, "durchgereicht");
        assert!(recorder.records().is_empty());
    }

    #[tokio::test]
    async fn ohne_registrierte_senke_verhaelt_sich_der_wrapper_wie_der_provider() {
        clear_transparency_sink();
        let inner = Arc::new(FixedProvider(Ok(antwort("unveraendert"))));
        let wrapped = wrap_with_transparency(inner, LlmUseCase::Faq);

        let result = wrapped
            .chat(&[ChatMessage::user("Frage")], ChatParams::default())
            .await
            .expect("Antwort");

        assert_eq!(result, antwort("unveraendert"));
    }

    #[test]
    fn moderation_use_cases_sind_benannt() {
        assert!(is_moderation_use_case(LlmUseCase::ModerationText));
        assert!(is_moderation_use_case(LlmUseCase::ModerationVerify));
        for use_case in LlmUseCase::all() {
            if matches!(
                use_case,
                LlmUseCase::ModerationText | LlmUseCase::ModerationVerify
            ) {
                continue;
            }
            assert!(
                !is_moderation_use_case(*use_case),
                "{} darf nicht als Moderation gelten",
                use_case.as_str()
            );
        }
    }

    #[test]
    fn jeder_use_case_hat_einen_klartextnamen() {
        for use_case in LlmUseCase::all() {
            let label = use_case_label(*use_case);
            assert!(!label.is_empty());
            assert_ne!(
                label,
                use_case.as_str(),
                "der Klartextname darf nicht der Slug sein"
            );
        }
    }

    #[test]
    fn secrets_werden_vor_dem_kanal_redigiert() {
        let text = "Nutze OPENAI_API_KEY=sk-liveGEHEIM1234567890 und DSN \
                    postgres://bot:supergeheim@10.0.0.5:5432/central, Header \
                    Authorization: Bearer eyJhbGciOiJIUzI1NiJ9geheim";
        let redacted = redact_secrets(text);

        assert!(!redacted.contains("sk-liveGEHEIM1234567890"), "{redacted}");
        assert!(!redacted.contains("supergeheim"), "{redacted}");
        assert!(
            !redacted.contains("eyJhbGciOiJIUzI1NiJ9geheim"),
            "{redacted}"
        );
        assert!(redacted.contains(REDACTED), "{redacted}");
        assert!(
            redacted.contains("Nutze"),
            "harmloser Text muss stehen bleiben: {redacted}"
        );
        assert!(
            redacted.contains("10.0.0.5:5432/central"),
            "der Rest der Verbindung darf stehen bleiben: {redacted}"
        );
    }

    #[test]
    fn freitext_bleibt_unangetastet() {
        let text = "Wie melde ich mich fuer das Turnier an? Ich habe gestern \
                    schon gefragt, aber niemand hat geantwortet.";
        assert_eq!(redact_secrets(text), text);
    }

    #[tokio::test]
    async fn secrets_im_prompt_und_in_der_antwort_landen_nicht_im_record() {
        let inner = Arc::new(FixedProvider(Ok(antwort(
            "nimm den key sk-antwortGEHEIM12345",
        ))));
        let sink = RecordingSink::arc();
        let provider = TransparencyProvider::new(inner, sink.clone(), LlmUseCase::Faq);

        provider
            .chat(
                &[ChatMessage::user(
                    "mein token=sk-abcdefghijklmnop1234 bitte pruefen",
                )],
                ChatParams::default(),
            )
            .await
            .expect("Antwort");

        let record = sink.records().remove(0);
        assert!(
            !record.prompt_excerpt.contains("sk-abcdefghijklmnop1234"),
            "{}",
            record.prompt_excerpt
        );
        let response = record.response.unwrap_or_default();
        assert!(!response.contains("sk-antwortGEHEIM12345"), "{response}");
    }
}
