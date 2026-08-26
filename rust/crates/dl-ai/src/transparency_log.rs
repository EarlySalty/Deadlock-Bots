//! Die Senke hinter dem Transparenz-Decorator: Queue, Drossel, Textform.
//!
//! Discord-frei mit Absicht — `dl-ai` bekommt keine Discord-Abhaengigkeit. Das
//! Binaer reicht einen [`TransparencyMessenger`] herein, der die zwei
//! Operationen kann, die dieses Log braucht: eine Nachricht senden und aus
//! einer Nachricht einen Thread machen.
//!
//! Leitlinien aus der Abnahme:
//! - Erfolgreiche Antworten werden einzeln sichtbar, kein Sampling.
//! - Fehler werden mindestens einmal einzeln sichtbar. Wiederholt derselbe
//!   Provider denselben Fehler, zeigt der Kanal ihn einmal und danach nur noch
//!   die Anzahl. Keine unterdrueckte Wiederholung verschwindet ohne Zahl: die
//!   Anzahl kommt entweder an der naechsten gezeigten Meldung mit, als
//!   Nachtrag, sobald die Fehlerserie zur Ruhe kommt, oder spaetestens beim
//!   geordneten Herunterfahren.
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
// Tokios Uhr statt std: nur so laesst sich das Wiederholungsfenster im Test
// mit `tokio::time::advance` ueberspringen, statt es abzuwarten.
use tokio::time::Instant;

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

/// Ein kaputter Provider liefert denselben Fehler bei jedem Versuch. Einmal
/// muss der Owner ihn sehen, danach reicht die Anzahl: innerhalb dieses
/// Fensters wird derselbe Fehler nur gezaehlt und beim naechsten Durchlass als
/// Anzahl mitgeschickt. 15 Minuten bis zur ersten Wiederholung, danach greift
/// das Backoff aus [`fenster_mit_backoff`].
/// Erfolgreiche Antworten bleiben davon unberuehrt.
const DEFAULT_ERROR_REPEAT_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Obergrenze des Backoffs. Ein Ausfall, der einen ganzen Tag dauert (gesperrtes
/// Anbieter-Konto, abgelaufene Rechnung), soll den Kanal nicht im Viertelstunden-
/// takt zumuellen; alle sechs Stunden eine Erinnerung reicht, und die Meldung
/// bleibt trotzdem eine Erinnerung und kein Stummschalter.
const MAX_ERROR_REPEAT_WINDOW: Duration = Duration::from_secs(6 * 60 * 60);

/// Nachtraege als eigene Nachricht sind standardmaessig aus. Sie waren selbst
/// der Spam, ueber den sich der Owner beschwert hat: erst die Fehlermeldung,
/// dann zwei Minuten spaeter eine zweite Nachricht, die nur „danach noch 3 mal
/// dasselbe" sagt. Die Anzahl geht ohne eigene Nachricht raus, naemlich an der
/// naechsten gezeigten Meldung oder an der ersten Antwort, die wieder klappt.
const DEFAULT_ERROR_FOLLOWUP_QUIET: Duration = Duration::ZERO;

/// Takt, in dem der Worker nach faelligen Nachtraegen schaut.
const FLUSH_TICK: Duration = Duration::from_secs(60);

/// So viele verschiedene Fehlerarten behaelt die Entprellung im Gedaechtnis.
const ERROR_MEMORY: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransparencyConfig {
    pub enabled: bool,
    pub channel_id: u64,
    /// Moderation spiegeln? Default aus, siehe
    /// [`crate::transparency::is_moderation_use_case`].
    pub include_moderation: bool,
    pub queue_capacity: usize,
    pub min_send_interval: Duration,
    /// Wie lange derselbe Fehler nach der ersten Meldung nur gezaehlt wird.
    /// `Duration::ZERO` schaltet die Entprellung ab.
    pub error_repeat_window: Duration,
    /// Ruhefrist, nach der die gezaehlten Wiederholungen als Nachtrag in den
    /// Kanal gehen. `Duration::ZERO` schaltet den Nachtrag ab; die Anzahl
    /// kommt dann nur an der naechsten gezeigten Meldung und beim
    /// Herunterfahren.
    pub error_followup_quiet: Duration,
}

impl Default for TransparencyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            channel_id: DEFAULT_TRANSPARENCY_CHANNEL_ID,
            include_moderation: false,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            min_send_interval: DEFAULT_MIN_SEND_INTERVAL,
            error_repeat_window: DEFAULT_ERROR_REPEAT_WINDOW,
            error_followup_quiet: DEFAULT_ERROR_FOLLOWUP_QUIET,
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
            // Im Vorfall muss ein Operator die Entprellung abschalten koennen,
            // ohne ein neues Binaer zu bauen: 0 heisst „jeden Fehler zeigen".
            error_repeat_window: env_seconds(
                &lookup,
                "DL_AI_TRANSPARENCY_ERROR_REPEAT_SECONDS",
                default.error_repeat_window,
            ),
            error_followup_quiet: env_seconds(
                &lookup,
                "DL_AI_TRANSPARENCY_ERROR_FOLLOWUP_SECONDS",
                default.error_followup_quiet,
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
            if self.include_moderation {
                "ja"
            } else {
                "nein"
            }
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

/// Sekundenwert aus der Umgebung. Unlesbares faellt auf den Default zurueck,
/// `0` ist ein gueltiger Wert und heisst „abgeschaltet".
fn env_seconds(lookup: &impl Fn(&str) -> Option<String>, key: &str, default: Duration) -> Duration {
    read_env(lookup, key)
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(default)
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

/// Modell plus Fehlertext des Anbieters. Bewusst ohne Anwendungsfall: ein
/// gesperrtes Konto trifft jeden Anwendungsfall gleichzeitig, und mit dem
/// Anwendungsfall im Schluessel meldete derselbe Ausfall einmal fuer das
/// Scrim-Lagebild, einmal fuer Runde 2, einmal fuer den Verbinder und so
/// weiter.
///
/// Das Modell gehoert dagegen hinein: ohne es sind ein 500 von Fireworks und
/// ein 500 von Gemini derselbe Schluessel, und der zweite, unabhaengige
/// Ausfall bliebe bis zu sechs Stunden unsichtbar. Es ist zugleich das
/// Kriterium, an dem eine wieder geglueckte Antwort ihre eigene Serie
/// wiedererkennt.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ErrorKey {
    /// Das Modell, unter dem die Serie lief. `None` heisst „nicht bekannt";
    /// der Fehlerpfad traegt oft kein Modell, weil der Aufrufer keins gesetzt
    /// hat und der Anbieter im Fehlerfall keins zurueckmeldet.
    model: Option<String>,
    /// Der entstoerte Fehlertext, siehe [`fehler_schluessel`].
    text: String,
}

/// Entprellt Wiederholungen desselben Fehlers.
///
/// Der Schluessel ist Modell plus Fehlertext. Die erste Meldung geht durch, jede
/// weitere im Fenster wird gezaehlt, und das Fenster waechst mit jeder
/// gezeigten Meldung ([`fenster_mit_backoff`]). Die gezaehlte Anzahl erreicht
/// den Owner, ohne dass eine zusaetzliche Nachricht noetig waere:
/// 1. an der naechsten gezeigten Meldung derselben Fehlerart,
/// 2. an der ersten Antwort desselben Modells, die wieder klappt: der Erfolg
///    raeumt genau die Zaehler dieses Modells ab und traegt deren Anzahl mit,
/// 3. als Nachtrag beim Verdraengen aus dem Gedaechtnis und beim
///    Herunterfahren ([`ErrorThrottle::offene_nachtraege`]).
///
/// Nicht abgedeckt bleibt der harte Abbruch (Absturz, SIGKILL): dort geht der
/// laufende Zaehler verloren. Persistenz waere dafuer noetig und steht
/// bewusst nicht im Verhaeltnis zum Nutzen.
#[derive(Default)]
struct ErrorThrottle {
    streaks: HashMap<ErrorKey, ErrorStreak>,
    /// Zuletzt benutzter Schluessel steht hinten (LRU): verdraengt wird, was
    /// am laengsten nicht mehr vorkam, nicht was zuerst eintrat.
    order: VecDeque<ErrorKey>,
    /// Nachtraege, die durch Verdraengung faellig wurden: Schluessel,
    /// Anwendungsfall der letzten Meldung, Anzahl.
    verdraengt: Vec<(ErrorKey, LlmUseCase, u64)>,
}

struct ErrorStreak {
    /// Wann diese Fehlerart zuletzt tatsaechlich im Kanal stand.
    last_posted: Instant,
    /// Wann sie zuletzt auftrat, auch unterdrueckt.
    last_seen: Instant,
    suppressed: u64,
    /// Wie oft diese Fehlerart schon im Kanal stand. Treibt das Backoff.
    gezeigt: u32,
    /// Anwendungsfall der letzten Meldung, nur fuer den Nachtragstext.
    letzter_use_case: LlmUseCase,
}

/// Der Schluessel, unter dem zwei Fehler als "derselbe" gelten.
///
/// Der Fehlertext traegt seit dem Anbieter-Hinweis den Wortlaut des Anbieters,
/// und der enthaelt oft eine Request-ID, eine Restlaufzeit oder einen
/// Zeitstempel. Am rohen Text zu deduplizieren hiesse: jeder Aufruf ist ein
/// neuer Fehler, und der Kanal ist wieder voll. Deshalb zwei Stufen:
///
/// 1. Alles ab [`crate::chat_provider::ANBIETER_MARKER`] faellt weg. Davor
///    steht der Teil, den wir selbst bauen, und der ist stabil.
/// 2. Was uebrig bleibt, wird von langen Ziffernfolgen befreit, damit auch
///    Fehlertexte anderer Herkunft nicht an einer ID auseinanderfallen.
///
/// Das Modell kommt unveraendert dazu: es unterscheidet zwei Anbieter, die
/// denselben Statuscode liefern.
fn fehler_schluessel(error: &str, model: Option<&str>) -> ErrorKey {
    ErrorKey {
        model: model.map(str::to_string),
        text: fehler_text_schluessel(error),
    }
}

/// Der entstoerte Teil des Fehlertexts, siehe [`fehler_schluessel`].
fn fehler_text_schluessel(error: &str) -> String {
    let kopf = match error.find(crate::chat_provider::ANBIETER_MARKER) {
        Some(pos) => &error[..pos],
        None => error,
    };

    let mut key = String::with_capacity(kopf.len());
    let mut lauf = String::new();

    fn spuelen(lauf: &mut String, key: &mut String) {
        if lauf.is_empty() {
            return;
        }
        // Vier Zeichen mit Ziffer: Jahreszahl, Uhrzeit, Request-ID. Ein
        // HTTP-Status hat drei und bleibt damit unterscheidbar.
        let hat_ziffer = lauf.chars().any(|c| c.is_ascii_digit());
        if hat_ziffer && lauf.chars().count() >= 4 {
            key.push('#');
        } else {
            key.push_str(lauf);
        }
        lauf.clear();
    }

    for zeichen in kopf.chars() {
        if zeichen.is_alphanumeric() {
            lauf.push(zeichen);
            continue;
        }
        spuelen(&mut lauf, &mut key);
        key.push(zeichen);
    }
    spuelen(&mut lauf, &mut key);
    key
}

/// Das Wiederholungsfenster waechst mit jeder gezeigten Meldung: nach der
/// ersten Meldung die volle Basis, danach jeweils das Doppelte, gedeckelt bei
/// [`MAX_ERROR_REPEAT_WINDOW`].
///
/// Ohne das meldet ein Dauerausfall alle 15 Minuten neu, und ein Abend mit
/// gesperrtem Anbieter-Konto sind zwanzig gleiche Nachrichten.
fn fenster_mit_backoff(basis: Duration, gezeigt: u32) -> Duration {
    if basis.is_zero() || basis >= MAX_ERROR_REPEAT_WINDOW {
        // Der Deckel gilt dem Backoff, nicht der Vorgabe. Wer bewusst zwoelf
        // Stunden konfiguriert, bekommt zwoelf Stunden und nicht stillschweigend
        // sechs.
        return basis;
    }
    let faktor = 1u32 << gezeigt.saturating_sub(1).min(16);
    basis.saturating_mul(faktor).min(MAX_ERROR_REPEAT_WINDOW)
}

/// Was nach einem bestaetigten Versand am Gedaechtnis nachzuziehen ist.
///
/// Nichts davon passiert in [`ErrorThrottle::decide`]: schlaegt der Post fehl,
/// bleibt der Stand unveraendert, und keine gezaehlte Wiederholung verschwindet
/// still.
enum Nachhalten {
    /// Entprellung abgeschaltet: es gibt kein Gedaechtnis zu pflegen.
    Nichts,
    /// Eine gezeigte Fehlermeldung: Fenster neu starten, Backoff erhoehen.
    Fehler(ErrorKey),
    /// Eine geglueckte Antwort: die Fehlerserien genau dieses Modells
    /// abraeumen.
    Erfolg(Option<String>),
}

/// `Show`: senden. `nachhalten` sagt, was nach erfolgreichem Senden am
/// Gedaechtnis nachzuziehen ist.
enum ErrorDecision {
    Show {
        nachhalten: Nachhalten,
        suppressed: u64,
    },
    Suppress,
}

/// Gehoert eine Fehlerserie zu dem Modell, das gerade wieder geantwortet hat?
///
/// Verglichen wird das Modell als Ganzes, `None` eingeschlossen. Ein Erfolg mit
/// bekanntem Modell raeumt also nie eine Serie ab, deren Modell unbekannt
/// blieb: wir wissen dann schlicht nicht, ob es dieselbe Strecke war, und ein
/// zu grosszuegiges Abraeumen ist genau der Spam, den die Entprellung
/// verhindern soll. Die verbleibende Unschaerfe (unbekannt raeumt unbekannt ab)
/// ist eng: eine geglueckte Antwort traegt das Modell des Anbieters fast immer
/// mit, nur der Fehlerpfad hat oft keins.
fn passt_zum_modell(key: &ErrorKey, model: Option<&str>) -> bool {
    key.model.as_deref() == model
}

impl ErrorThrottle {
    fn decide(
        &mut self,
        interaction: &AiInteraction,
        window: Duration,
        now: Instant,
        immer_zeigen: bool,
    ) -> ErrorDecision {
        let Some(error) = interaction.error.as_deref() else {
            // Dieses Modell antwortet wieder: seine Serie ist vorbei. Die
            // offenen Zaehler reisen auf dieser Antwort mit, statt eine eigene
            // Nachricht zu bekommen, und sein Backoff faengt bei null an.
            //
            // Nur seine: raeumte der Erfolg das ganze Gedaechtnis ab, wuerde
            // jede geglueckte Antwort eines anderen Modells die Serie des
            // kaputten Anbieters loeschen, und dessen naechster Fehler stuende
            // sofort wieder als Erstmeldung im Kanal.
            return ErrorDecision::Show {
                nachhalten: Nachhalten::Erfolg(interaction.model.clone()),
                suppressed: self.summe_fuer_modell(interaction.model.as_deref()),
            };
        };
        if window.is_zero() {
            return ErrorDecision::Show {
                nachhalten: Nachhalten::Nichts,
                suppressed: 0,
            };
        }
        let key = fehler_schluessel(error, interaction.model.as_deref());
        self.beruehren(&key);
        if let Some(streak) = self.streaks.get_mut(&key) {
            streak.last_seen = now;
            streak.letzter_use_case = interaction.use_case;
            let fenster = fenster_mit_backoff(window, streak.gezeigt);
            if !immer_zeigen && now.duration_since(streak.last_posted) < fenster {
                streak.suppressed += 1;
                return ErrorDecision::Suppress;
            }
            // `last_posted` bleibt stehen, bis das Senden geglueckt ist:
            // sonst verschluckt ein fehlgeschlagener Post die einzige
            // sichtbare Instanz und das ganze Fenster bleibt stumm.
            let carried = streak.suppressed;
            return ErrorDecision::Show {
                nachhalten: Nachhalten::Fehler(key),
                suppressed: carried,
            };
        }
        // Erstes Auftreten: der Eintrag entsteht erst mit dem Senden. Kommt
        // der Post nicht durch, gibt es auch nichts zu entprellen.
        ErrorDecision::Show {
            nachhalten: Nachhalten::Fehler(key),
            suppressed: 0,
        }
    }

    /// Bestaetigt einen erfolgreich gesendeten Fehler: Fenster neu starten,
    /// Zaehler auf null.
    fn gesendet(&mut self, key: ErrorKey, use_case: LlmUseCase, now: Instant) {
        match self.streaks.get_mut(&key) {
            Some(streak) => {
                streak.last_posted = now;
                streak.last_seen = now;
                streak.suppressed = 0;
                streak.gezeigt = streak.gezeigt.saturating_add(1);
                streak.letzter_use_case = use_case;
            }
            None => {
                self.streaks.insert(
                    key.clone(),
                    ErrorStreak {
                        last_posted: now,
                        last_seen: now,
                        suppressed: 0,
                        gezeigt: 1,
                        letzter_use_case: use_case,
                    },
                );
                self.order.push_back(key);
                self.verdraengen();
            }
        }
    }

    /// Was an offener Anzahl auf dem Konto genau dieses Modells steht, ohne
    /// etwas zu veraendern. Die Summe geht als Anzahl an die erfolgreiche
    /// Meldung; abgeraeumt wird erst, wenn die auch wirklich im Kanal steht
    /// ([`ErrorThrottle::erfolg_bestaetigt`]).
    fn summe_fuer_modell(&self, model: Option<&str>) -> u64 {
        let summe = self
            .verdraengt
            .iter()
            .filter(|(key, _, _)| passt_zum_modell(key, model))
            .fold(0u64, |summe, (_, _, anzahl)| summe.saturating_add(*anzahl));
        self.streaks
            .iter()
            .filter(|(key, _)| passt_zum_modell(key, model))
            .fold(summe, |summe, (_, streak)| {
                summe.saturating_add(streak.suppressed)
            })
    }

    /// Der Erfolg steht im Kanal: die Fehlerserien dieses Modells sind damit
    /// erzaehlt und verschwinden, Backoff inklusive. Alles andere bleibt
    /// stehen, sonst faengt eine fremde, weiterlaufende Serie wieder bei der
    /// Erstmeldung an.
    fn erfolg_bestaetigt(&mut self, model: Option<&str>) {
        self.verdraengt
            .retain(|(key, _, _)| !passt_zum_modell(key, model));
        self.streaks.retain(|key, _| !passt_zum_modell(key, model));
        self.order.retain(|key| !passt_zum_modell(key, model));
    }

    /// Schiebt einen bekannten Schluessel ans Ende der LRU-Reihe.
    fn beruehren(&mut self, key: &ErrorKey) {
        if let Some(pos) = self.order.iter().position(|vorhanden| vorhanden == key) {
            if let Some(gefunden) = self.order.remove(pos) {
                self.order.push_back(gefunden);
            }
        }
    }

    fn verdraengen(&mut self) {
        while self.order.len() > ERROR_MEMORY {
            let Some(alt) = self.order.pop_front() else {
                break;
            };
            if let Some(streak) = self.streaks.remove(&alt) {
                // Ein offener Zaehler darf nicht mit dem Eintrag verschwinden.
                if streak.suppressed > 0 {
                    self.verdraengt
                        .push((alt, streak.letzter_use_case, streak.suppressed));
                }
            }
        }
    }

    /// Fehlerserien, die seit der Ruhefrist nicht mehr auftraten und noch
    /// einen offenen Zaehler haben. Der Zaehler wird dabei geleert.
    fn faellige_nachtraege(
        &mut self,
        quiet: Duration,
        now: Instant,
    ) -> Vec<(ErrorKey, LlmUseCase, u64)> {
        let mut faellig = std::mem::take(&mut self.verdraengt);
        if quiet.is_zero() {
            return faellig;
        }
        for (key, streak) in self.streaks.iter_mut() {
            if streak.suppressed > 0 && now.duration_since(streak.last_seen) >= quiet {
                faellig.push((key.clone(), streak.letzter_use_case, streak.suppressed));
                streak.suppressed = 0;
            }
        }
        faellig
    }

    /// Alles, was noch offen ist — fuer den geordneten Abschluss.
    fn offene_nachtraege(&mut self) -> Vec<(ErrorKey, LlmUseCase, u64)> {
        let mut faellig = std::mem::take(&mut self.verdraengt);
        for (key, streak) in self.streaks.iter_mut() {
            if streak.suppressed > 0 {
                faellig.push((key.clone(), streak.letzter_use_case, streak.suppressed));
                streak.suppressed = 0;
            }
        }
        faellig
    }
}

/// Haengt die Anzahl der unterdrueckten Wiederholungen an die Meldung.
fn with_repeat_notice(content: String, suppressed: u64) -> String {
    if suppressed == 0 {
        return content;
    }
    let notice = format!("\n_Davor {suppressed} mal derselbe Fehler, nicht einzeln angezeigt._");
    let room = DISCORD_CONTENT_LIMIT.saturating_sub(notice.chars().count());
    format!("{}{notice}", cut(&content, room))
}

/// Der Nachtrag: die Serie ist vorbei, die Anzahl geht trotzdem raus.
fn repeat_followup(key: &ErrorKey, use_case: LlmUseCase, suppressed: u64) -> String {
    let text = format!(
        "**KI · {} · Fehler**\nDanach noch {suppressed} mal derselbe Fehler, nicht einzeln angezeigt.\nFehler: {}",
        use_case_label(use_case),
        key.text,
    );
    cut(&text, DISCORD_CONTENT_LIMIT)
}

async fn run_worker(
    mut rx: mpsc::Receiver<AiInteraction>,
    messenger: Arc<dyn TransparencyMessenger>,
    config: TransparencyConfig,
    dropped: Arc<AtomicU64>,
) {
    let mut router = ConversationRouter::default();
    let mut errors = ErrorThrottle::default();
    let mut flush = tokio::time::interval(FLUSH_TICK);
    loop {
        let interaction = tokio::select! {
            eingang = rx.recv() => match eingang {
                Some(interaction) => interaction,
                // Die Senke ist weg: was noch offen ist, geht jetzt raus.
                None => break,
            },
            _ = flush.tick() => {
                let faellig = errors.faellige_nachtraege(config.error_followup_quiet, Instant::now());
                sende_nachtraege(faellig, messenger.as_ref(), &config).await;
                continue;
            }
        };
        let missed = dropped.swap(0, Ordering::Relaxed);
        if missed > 0 {
            // Stille darf nie wie „keine KI-Aktivitaet" aussehen.
            if let Err(error) = messenger
                .send(config.channel_id, &drop_notice(missed))
                .await
            {
                tracing::warn!(%error, missed, "Hinweis auf verworfene KI-Eintraege nicht zustellbar");
                dropped.fetch_add(missed, Ordering::Relaxed);
            }
            throttle(&config).await;
        }

        // Auch unterdrueckte Eintraege laufen durch den Router: er haelt die
        // Fingerprints des Gespraechs frisch und zaehlt die Runde weiter. Ohne
        // das reisst eine laengere Fehlerserie den Thread ab und die naechste
        // gezeigte Meldung traegt eine zu niedrige Rundennummer.
        let id = router.resolve(&interaction);

        // Ein laufendes Gespraech, von dem noch nichts im Kanal steht, braucht
        // seine Ankernachricht: ohne sie gibt es keinen Thread und der Verlauf
        // liegt verstreut im Kanal. Diese eine Meldung geht darum immer durch.
        let ohne_anker = router
            .conversations
            .get(&id)
            .is_some_and(|state| state.anchor_message_id.is_none() && state.thread_id.is_none());
        let braucht_anker = interaction.is_conversation() && ohne_anker;

        let decision = errors.decide(
            &interaction,
            config.error_repeat_window,
            Instant::now(),
            braucht_anker,
        );
        let (nachhalten, suppressed) = match decision {
            ErrorDecision::Suppress => continue,
            ErrorDecision::Show {
                nachhalten,
                suppressed,
            } => (nachhalten, suppressed),
        };
        let gesendet = deliver(
            &mut router,
            id,
            &interaction,
            suppressed,
            messenger.as_ref(),
            &config,
        )
        .await;
        // Erst der bestaetigte Versand veraendert das Gedaechtnis: sonst
        // nimmt ein fehlgeschlagener Post die gesammelte Anzahl mit ins Nichts.
        if gesendet {
            match nachhalten {
                Nachhalten::Nichts => {}
                Nachhalten::Fehler(key) => {
                    errors.gesendet(key, interaction.use_case, Instant::now());
                }
                Nachhalten::Erfolg(model) => errors.erfolg_bestaetigt(model.as_deref()),
            }
        }
        throttle(&config).await;
    }

    // Geordnetes Ende: kein gezaehlter Fehler verschwindet ungenannt.
    let offen = errors.offene_nachtraege();
    sende_nachtraege(offen, messenger.as_ref(), &config).await;
}

async fn sende_nachtraege(
    faellig: Vec<(ErrorKey, LlmUseCase, u64)>,
    messenger: &dyn TransparencyMessenger,
    config: &TransparencyConfig,
) {
    for (key, use_case, suppressed) in faellig {
        if let Err(error) = messenger
            .send(
                config.channel_id,
                &repeat_followup(&key, use_case, suppressed),
            )
            .await
        {
            tracing::warn!(%error, suppressed, "Nachtrag zu unterdrueckten KI-Fehlern nicht zustellbar");
        }
        throttle(config).await;
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
    suppressed: u64,
    messenger: &dyn TransparencyMessenger,
    config: &TransparencyConfig,
) -> bool {
    let Some(state) = router.conversations.get(&id) else {
        return false;
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

    let content = with_repeat_notice(
        render_interaction(interaction, turn, thread_id.is_none()),
        suppressed,
    );
    let target = thread_id.unwrap_or(config.channel_id);
    let sent = messenger.send(target, &content).await;
    let sent = match sent {
        Ok(message_id) => Some(message_id),
        Err(error) if target != config.channel_id => {
            // Der Thread ist weg (archiviert, geloescht). Nichts darf still
            // verschwinden: dann eben in den Kanal.
            tracing::warn!(%error, thread_id = target, "KI-Transparenz-Thread nicht erreichbar, Rueckfall auf den Kanal");
            thread_id = None;
            let fallback =
                with_repeat_notice(render_interaction(interaction, turn, true), suppressed);
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
    sent.is_some()
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

/// Kuerzungsstufen fuer eine Fehlermeldung. Hier gibt es keine Antwort zu
/// beurteilen, und der Auslöser ist im Fehlerfall wertlos: was im Kanal zaehlt,
/// ist der Fehlertext. Ein Scrim-Lagebild schleppt 23.000 Zeichen Rohdaten im
/// Auslöser mit, und davon standen vorher 500 in jeder Fehlermeldung.
const ERROR_SHRINK_STAGES: &[(Option<usize>, Option<usize>)] =
    &[(Some(0), Some(160)), (Some(0), Some(80))];

/// Baut die Nachricht fuer den Kanal.
///
/// `turn` ist die Rundennummer im Gespraech, `standalone` sagt, ob die
/// Nachricht direkt im Kanal steht (dann braucht sie die Zuordnung im Text).
pub fn render_interaction(interaction: &AiInteraction, turn: u32, standalone: bool) -> String {
    let stages = if interaction.error.is_some() {
        ERROR_SHRINK_STAGES
    } else {
        SHRINK_STAGES
    };
    for (system_budget, prompt_budget) in stages {
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
        /// So viele der naechsten Sendeversuche scheitern.
        fehlschlaege: AtomicU64,
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

        fn mit_fehlschlaegen(anzahl: u64) -> Arc<Self> {
            Arc::new(Self {
                thread_erlaubt: false,
                fehlschlaege: AtomicU64::new(anzahl),
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
            if self
                .fehlschlaege
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |offen| {
                    offen.checked_sub(1)
                })
                .is_ok()
            {
                return Err("Discord antwortet nicht".to_string());
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

    fn fehler_interaktion(use_case: LlmUseCase, frage: &str, fehler: &str) -> AiInteraction {
        fehler_interaktion_mit_modell(use_case, frage, fehler, None)
    }

    /// Wie [`fehler_interaktion`], nur mit bekanntem Modell. Der Fehlerpfad
    /// traegt in der Praxis oft keins, die Entprellung muss aber beide Faelle
    /// auseinanderhalten.
    fn fehler_interaktion_mit_modell(
        use_case: LlmUseCase,
        frage: &str,
        fehler: &str,
        modell: Option<&str>,
    ) -> AiInteraction {
        AiInteraction {
            use_case,
            prompt_excerpt: frage.to_string(),
            system_excerpt: None,
            response: None,
            error: Some(fehler.to_string()),
            model: modell.map(str::to_string),
            latency_ms: 4,
            conversation_trail: vec![],
        }
    }

    #[test]
    fn eine_wechselnde_request_id_macht_keinen_neuen_fehler() {
        // Wortlaut wie von Fireworks, nur die Request-ID wechselt je Aufruf.
        let erster = crate::chat_provider::anbieter_hinweis(
            412,
            r#"{"error":{"message":"Account mail-01rvneuz61yq is suspended (request chatcmpl-1e4b56f248b24bbb9e7cd3ea2add56ab)"}}"#,
        );
        let zweiter = crate::chat_provider::anbieter_hinweis(
            412,
            r#"{"error":{"message":"Account mail-01rvneuz61yq is suspended (request chatcmpl-99887766554433221100aabbccddeeff)"}}"#,
        );
        assert_ne!(erster, zweiter, "der Testaufbau muss variieren");
        assert_eq!(
            fehler_schluessel(&erster, None),
            fehler_schluessel(&zweiter, None),
            "eine wechselnde ID darf die Entprellung nicht aushebeln"
        );
    }

    #[test]
    fn verschiedene_statuscodes_bleiben_verschiedene_fehler() {
        let gesperrt = crate::chat_provider::anbieter_hinweis(412, "");
        let unbekannt = crate::chat_provider::anbieter_hinweis(404, "");
        assert_ne!(
            fehler_schluessel(&gesperrt, None),
            fehler_schluessel(&unbekannt, None),
            "412 und 404 sind zwei Befunde, keiner"
        );
    }

    #[tokio::test]
    async fn eine_serie_mit_wechselnder_id_wird_trotzdem_entprellt() {
        let messenger = FakeMessenger::arc(false);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        for index in 0..5 {
            let fehler = crate::chat_provider::anbieter_hinweis(
                412,
                &format!(
                    r#"{{"error":{{"message":"Account suspended (request req-{index}0000000000{index})"}}}}"#
                ),
            );
            let mut interaktion = fehler_interaktion(LlmUseCase::ScrimLagebild, "team", &fehler);
            interaktion.conversation_trail = vec![];
            sink.record(interaktion);
        }
        assert!(warte_auf(|| !messenger.sent().is_empty()).await);
        tokio::time::sleep(Duration::from_millis(80)).await;

        assert_eq!(
            messenger.sent().len(),
            1,
            "wechselnde IDs sind derselbe Ausfall: {:?}",
            messenger.sent()
        );
    }

    #[tokio::test]
    async fn derselbe_fehler_wird_einmal_gezeigt_und_danach_nur_gezaehlt() {
        let messenger = FakeMessenger::arc(false);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        for index in 0..5 {
            sink.record(fehler_interaktion(
                LlmUseCase::ScrimLagebild,
                &format!("team {index}"),
                "LLM provider error: HTTP 412",
            ));
        }
        // Ein anderer Fehler muss trotzdem durchkommen.
        sink.record(fehler_interaktion(
            LlmUseCase::ScrimLagebild,
            "team 9",
            "LLM provider error: HTTP 404",
        ));
        assert!(warte_auf(|| messenger.sent().len() == 2).await);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let sent = messenger.sent();
        assert_eq!(sent.len(), 2, "vier Wiederholungen bleiben stumm: {sent:?}");
        assert!(sent[0].1.contains("HTTP 412"), "{}", sent[0].1);
        assert!(sent[1].1.contains("HTTP 404"), "{}", sent[1].1);
    }

    #[tokio::test]
    async fn erfolgreiche_antworten_werden_nie_entprellt() {
        let messenger = FakeMessenger::arc(false);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        for index in 0..4_u64 {
            sink.record(interaktion(
                LlmUseCase::Faq,
                "gleiche frage",
                "gleiche antwort",
                &[index],
            ));
        }
        assert!(warte_auf(|| messenger.sent().len() == 4).await);
    }

    /// Pausierte Uhr: das Wiederholungsfenster wird per `advance` uebersprungen
    /// statt es gegen die echte Uhr abzuwarten.
    #[tokio::test(start_paused = true)]
    async fn nach_dem_fenster_meldet_der_fehler_die_unterdrueckte_anzahl() {
        let messenger = FakeMessenger::arc(false);
        let config = TransparencyConfig {
            error_repeat_window: Duration::from_secs(600),
            // Der Nachtrag ist hier aus: geprueft wird der Weg ueber die
            // naechste gezeigte Meldung.
            error_followup_quiet: Duration::ZERO,
            ..testkonfig()
        };
        let log = TransparencyLog::spawn(messenger.clone(), config);
        let sink = log.sink();

        for _ in 0..3 {
            sink.record(fehler_interaktion(
                LlmUseCase::ScrimLagebild,
                "team 1",
                "LLM provider error: HTTP 412",
            ));
        }
        assert!(warte_auf(|| messenger.sent().len() == 1).await);
        tokio::time::advance(Duration::from_secs(700)).await;
        sink.record(fehler_interaktion(
            LlmUseCase::ScrimLagebild,
            "team 1",
            "LLM provider error: HTTP 412",
        ));
        assert!(warte_auf(|| messenger.sent().len() == 2).await);

        let sent = messenger.sent();
        assert!(
            sent[1].1.contains("Davor 2 mal derselbe Fehler"),
            "die Anzahl muss mitkommen: {}",
            sent[1].1
        );
    }

    #[tokio::test]
    async fn unterdrueckte_runden_zaehlen_im_gespraech_weiter() {
        let messenger = FakeMessenger::arc(true);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        let mut fehler = fehler_interaktion(LlmUseCase::BotPate, "runde 1", "HTTP 412");
        fehler.conversation_trail = vec![1];
        sink.record(fehler);
        assert!(warte_auf(|| messenger.sent().len() == 1).await);

        let mut wiederholung = fehler_interaktion(LlmUseCase::BotPate, "runde 2", "HTTP 412");
        wiederholung.conversation_trail = vec![1, 2];
        sink.record(wiederholung);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(messenger.sent().len(), 1, "Runde 2 bleibt stumm");

        sink.record(interaktion(
            LlmUseCase::BotPate,
            "runde 3",
            "endlich eine Antwort",
            &[2, 3],
        ));
        assert!(warte_auf(|| messenger.sent().len() == 2).await);

        let sent = messenger.sent();
        assert!(
            sent[1].1.contains("Runde 3"),
            "die unterdrueckte Runde zaehlt mit: {}",
            sent[1].1
        );
        assert_eq!(
            messenger.threads().len(),
            1,
            "das Gespraech behaelt seinen Thread"
        );
    }

    #[test]
    fn die_anzahl_sprengt_das_zeichenlimit_nicht() {
        let lang = "X".repeat(DISCORD_CONTENT_LIMIT);
        let text = with_repeat_notice(lang, 7);
        assert!(
            text.chars().count() <= DISCORD_CONTENT_LIMIT,
            "{}",
            text.chars().count()
        );
        assert!(text.contains("Davor 7 mal derselbe Fehler"));
    }

    /// Nimmt eine Entscheidung ab, die „senden" lauten muss, und bestaetigt
    /// den Versand. Liefert die mitgeschickte Anzahl.
    fn zeigen(
        throttle: &mut ErrorThrottle,
        interaktion: &AiInteraction,
        fenster: Duration,
        jetzt: Instant,
    ) -> u64 {
        match throttle.decide(interaktion, fenster, jetzt, false) {
            ErrorDecision::Show {
                nachhalten,
                suppressed,
            } => {
                match nachhalten {
                    Nachhalten::Nichts => {}
                    Nachhalten::Fehler(key) => throttle.gesendet(key, interaktion.use_case, jetzt),
                    Nachhalten::Erfolg(model) => throttle.erfolg_bestaetigt(model.as_deref()),
                }
                suppressed
            }
            ErrorDecision::Suppress => panic!("die Meldung haette gezeigt werden muessen"),
        }
    }

    fn unterdruecken(
        throttle: &mut ErrorThrottle,
        interaktion: &AiInteraction,
        fenster: Duration,
        jetzt: Instant,
    ) {
        assert!(
            matches!(
                throttle.decide(interaktion, fenster, jetzt, false),
                ErrorDecision::Suppress
            ),
            "die Wiederholung haette stumm bleiben muessen"
        );
    }

    #[test]
    fn das_standardfenster_ist_kurz_genug_um_wieder_sichtbar_zu_werden() {
        let default = TransparencyConfig::default();
        assert_eq!(
            default.error_repeat_window,
            Duration::from_secs(15 * 60),
            "ein Fenster von Stunden waere ein Stummschalter"
        );
        assert!(
            default.error_followup_quiet.is_zero(),
            "der Nachtrag als eigene Nachricht ist der Spam, ueber den sich der \
             Owner beschwert hat; die Anzahl reist an der naechsten Meldung mit"
        );
    }

    #[test]
    fn die_entprellung_laesst_sich_per_umgebung_abschalten() {
        let aus = TransparencyConfig::from_env(|key| match key {
            "DL_AI_TRANSPARENCY_ERROR_REPEAT_SECONDS" => Some("0".to_string()),
            _ => None,
        });
        assert!(
            aus.error_repeat_window.is_zero(),
            "0 muss die Entprellung im Vorfall abschalten"
        );

        let gesetzt = TransparencyConfig::from_env(|key| match key {
            "DL_AI_TRANSPARENCY_ERROR_REPEAT_SECONDS" => Some("90".to_string()),
            "DL_AI_TRANSPARENCY_ERROR_FOLLOWUP_SECONDS" => Some("30".to_string()),
            _ => None,
        });
        assert_eq!(gesetzt.error_repeat_window, Duration::from_secs(90));
        assert_eq!(gesetzt.error_followup_quiet, Duration::from_secs(30));

        let unlesbar = TransparencyConfig::from_env(|key| match key {
            "DL_AI_TRANSPARENCY_ERROR_REPEAT_SECONDS" => Some("bald".to_string()),
            _ => None,
        });
        assert_eq!(
            unlesbar.error_repeat_window, DEFAULT_ERROR_REPEAT_WINDOW,
            "Unsinn faellt auf den Default zurueck"
        );
    }

    #[test]
    fn ein_fehlgeschlagener_post_startet_das_fenster_nicht() {
        let mut throttle = ErrorThrottle::default();
        let jetzt = Instant::now();
        let fenster = Duration::from_secs(600);
        let fehler = fehler_interaktion(LlmUseCase::Faq, "frage", "HTTP 412");

        // Erster Versuch: senden erlaubt, aber der Versand scheitert, also
        // wird nicht bestaetigt.
        assert!(matches!(
            throttle.decide(&fehler, fenster, jetzt, false),
            ErrorDecision::Show { .. }
        ));
        // Ohne Bestaetigung muss der naechste Versuch wieder durchgehen,
        // sonst ist die einzige sichtbare Instanz verloren.
        assert!(
            matches!(
                throttle.decide(&fehler, fenster, jetzt, false),
                ErrorDecision::Show { .. }
            ),
            "ohne erfolgreichen Versand darf das Fenster nicht zugehen"
        );

        // Erst der bestaetigte Versand macht die Entprellung scharf.
        zeigen(&mut throttle, &fehler, fenster, jetzt);
        unterdruecken(&mut throttle, &fehler, fenster, jetzt);
    }

    #[test]
    fn verdraengt_wird_nach_aktualitaet_nicht_nach_ersteintritt() {
        let mut throttle = ErrorThrottle::default();
        let jetzt = Instant::now();
        let fenster = Duration::from_secs(600);

        for index in 0..ERROR_MEMORY {
            let fehler = fehler_interaktion(LlmUseCase::Faq, "frage", &format!("fehler {index}"));
            zeigen(&mut throttle, &fehler, fenster, jetzt);
        }

        // Der aelteste Eintrag kommt gerade wieder vor.
        let alt = fehler_interaktion(LlmUseCase::Faq, "frage", "fehler 0");
        unterdruecken(&mut throttle, &alt, fenster, jetzt);

        // Ein neuer Fehler verdraengt jetzt den laengst ungenutzten Eintrag.
        let neu = fehler_interaktion(LlmUseCase::Faq, "frage", "ganz neuer fehler");
        zeigen(&mut throttle, &neu, fenster, jetzt);

        assert!(
            throttle
                .streaks
                .contains_key(&fehler_schluessel("fehler 0", None)),
            "der zuletzt benutzte Eintrag darf nicht verdraengt werden"
        );
        assert!(
            !throttle
                .streaks
                .contains_key(&fehler_schluessel("fehler 1", None)),
            "verdraengt wird der laengst ungenutzte Eintrag"
        );
    }

    #[test]
    fn ein_ausfall_meldet_sich_nicht_einmal_pro_anwendungsfall() {
        // Ein gesperrtes Anbieter-Konto trifft jeden Anwendungsfall
        // gleichzeitig. Mit dem Anwendungsfall im Schluessel meldete derselbe
        // Ausfall einmal fuer das Scrim-Lagebild, einmal fuer den Verbinder
        // und einmal fuer die FAQ, obwohl es ein einziges Problem ist.
        let mut throttle = ErrorThrottle::default();
        let jetzt = Instant::now();
        let fenster = Duration::from_secs(900);
        let text = "HTTP 412: Anbieter-Konto gesperrt";

        zeigen(
            &mut throttle,
            &fehler_interaktion(LlmUseCase::ScrimLagebild, "team 1", text),
            fenster,
            jetzt,
        );
        unterdruecken(
            &mut throttle,
            &fehler_interaktion(LlmUseCase::Faq, "frage", text),
            fenster,
            jetzt,
        );
        unterdruecken(
            &mut throttle,
            &fehler_interaktion(LlmUseCase::BotPate, "pate", text),
            fenster,
            jetzt,
        );
    }

    #[test]
    fn ein_dauerausfall_meldet_sich_immer_seltener() {
        // Ohne Backoff sind zwoelf Stunden gesperrtes Konto achtundvierzig
        // gleiche Nachrichten im Kanal.
        let mut throttle = ErrorThrottle::default();
        let mut jetzt = Instant::now();
        let fenster = Duration::from_secs(900);
        let fehler = fehler_interaktion(LlmUseCase::ScrimLagebild, "team 1", "HTTP 412");

        zeigen(&mut throttle, &fehler, fenster, jetzt);

        // Zweite Meldung nach dem vollen Grundfenster.
        jetzt += fenster;
        zeigen(&mut throttle, &fehler, fenster, jetzt);

        // Dritte erst nach dem doppelten: nach einem weiteren Grundfenster
        // bleibt sie stumm.
        jetzt += fenster;
        unterdruecken(&mut throttle, &fehler, fenster, jetzt);
        jetzt += fenster;
        zeigen(&mut throttle, &fehler, fenster, jetzt);
    }

    #[test]
    fn das_backoff_bleibt_unter_dem_deckel() {
        // Ohne Deckel waere das Fenster nach zwanzig Meldungen laenger als ein
        // Menschenleben und die Entprellung ein Stummschalter.
        let basis = Duration::from_secs(900);
        assert_eq!(fenster_mit_backoff(basis, 1), basis);
        assert_eq!(fenster_mit_backoff(basis, 2), basis * 2);
        assert_eq!(fenster_mit_backoff(basis, 3), basis * 4);
        assert_eq!(fenster_mit_backoff(basis, 40), MAX_ERROR_REPEAT_WINDOW);
        assert!(fenster_mit_backoff(Duration::ZERO, 40).is_zero());
    }

    #[test]
    fn ein_erfolg_beendet_die_serie_und_traegt_die_anzahl() {
        let mut throttle = ErrorThrottle::default();
        let jetzt = Instant::now();
        let fenster = Duration::from_secs(900);
        // Dasselbe Modell auf beiden Seiten: nur dann weiss der Erfolg, dass
        // er genau diese Serie beendet.
        let fehler = fehler_interaktion_mit_modell(
            LlmUseCase::ScrimLagebild,
            "team 1",
            "HTTP 412",
            Some("test-modell"),
        );

        zeigen(&mut throttle, &fehler, fenster, jetzt);
        for _ in 0..3 {
            unterdruecken(&mut throttle, &fehler, fenster, jetzt);
        }

        let erfolg = interaktion(LlmUseCase::ScrimLagebild, "team 1", "geht wieder", &[]);
        let getragen = zeigen(&mut throttle, &erfolg, fenster, jetzt);
        assert_eq!(getragen, 3, "die Anzahl reist auf der Erfolgsmeldung mit");
        assert!(
            throttle.streaks.is_empty(),
            "der Erfolg raeumt das Gedaechtnis ab"
        );

        // Und weil das Backoff mit zurueckgesetzt ist, ist der naechste Fehler
        // sofort wieder sichtbar.
        zeigen(&mut throttle, &fehler, fenster, jetzt);
    }

    #[test]
    fn ein_erfolg_eines_anderen_modells_bricht_die_entprellung_nicht() {
        // Der Vorfall: Fireworks liefert bei jedem Versuch 412 (Konto
        // gesperrt), waehrend Gemini-FAQ und OpenAI normal weiter antworten.
        // Raeumte jede dieser Erfolgsmeldungen das ganze Gedaechtnis ab,
        // stuende jeder 412 sofort wieder als Erstmeldung im Kanal.
        let mut throttle = ErrorThrottle::default();
        let jetzt = Instant::now();
        let fenster = Duration::from_secs(900);
        let kaputt = fehler_interaktion_mit_modell(
            LlmUseCase::ScrimLagebild,
            "team 1",
            "HTTP 412: Anbieter-Konto gesperrt",
            Some("fireworks/deepseek"),
        );
        let mut heil = interaktion(LlmUseCase::Faq, "frage", "antwort", &[]);
        heil.model = Some("gemini-flash".to_string());

        zeigen(&mut throttle, &kaputt, fenster, jetzt);

        for runde in 0..5 {
            let getragen = zeigen(&mut throttle, &heil, fenster, jetzt);
            assert_eq!(
                getragen, 0,
                "der Erfolg eines anderen Modells traegt keine fremde Anzahl (Runde {runde})"
            );
            unterdruecken(&mut throttle, &kaputt, fenster, jetzt);
        }

        // Und die Serie hat weitergezaehlt, statt bei jedem Erfolg neu zu
        // beginnen: die Anzahl reist an der naechsten gezeigten Meldung mit.
        let spaeter = jetzt + Duration::from_secs(1_000);
        let getragen = zeigen(&mut throttle, &kaputt, fenster, spaeter);
        assert_eq!(
            getragen, 5,
            "keine der unterdrueckten Wiederholungen darf verloren gehen"
        );
    }

    #[test]
    fn ein_erfolg_raeumt_erst_nach_bestaetigtem_versand_ab() {
        // Der Drain darf nicht in `decide` passieren: schlaegt der Post fehl,
        // waere die gesammelte Anzahl weg, obwohl nie jemand sie gelesen hat.
        let mut throttle = ErrorThrottle::default();
        let jetzt = Instant::now();
        let fenster = Duration::from_secs(900);
        let fehler = fehler_interaktion_mit_modell(
            LlmUseCase::ScrimLagebild,
            "team 1",
            "HTTP 412",
            Some("test-modell"),
        );

        zeigen(&mut throttle, &fehler, fenster, jetzt);
        for _ in 0..3 {
            unterdruecken(&mut throttle, &fehler, fenster, jetzt);
        }

        // Entscheidung abholen, aber den Versand NICHT bestaetigen.
        let erfolg = interaktion(LlmUseCase::ScrimLagebild, "team 1", "geht wieder", &[]);
        let angekuendigt = match throttle.decide(&erfolg, fenster, jetzt, false) {
            ErrorDecision::Show { suppressed, .. } => suppressed,
            ErrorDecision::Suppress => panic!("ein Erfolg wird nie entprellt"),
        };
        assert_eq!(angekuendigt, 3);
        assert_eq!(
            throttle.offene_nachtraege().len(),
            1,
            "ohne bestaetigten Versand bleibt der Zaehler stehen"
        );
    }

    #[test]
    fn zwei_anbieter_mit_demselben_status_sind_zwei_befunde() {
        let text = crate::chat_provider::anbieter_hinweis(500, "upstream connect error");
        assert_ne!(
            fehler_schluessel(&text, Some("fireworks/deepseek")),
            fehler_schluessel(&text, Some("gemini-flash")),
            "ein zweiter, unabhaengiger Ausfall darf nicht hinter dem ersten verschwinden"
        );
    }

    #[test]
    fn ein_konfiguriertes_fenster_ueber_dem_deckel_bleibt_stehen() {
        // Der Deckel gehoert dem Backoff. Wer zwoelf Stunden konfiguriert,
        // bekommt zwoelf Stunden und nicht stillschweigend sechs.
        let basis = Duration::from_secs(12 * 60 * 60);
        assert!(basis > MAX_ERROR_REPEAT_WINDOW);
        assert_eq!(fenster_mit_backoff(basis, 1), basis);
        assert_eq!(fenster_mit_backoff(basis, 9), basis);
    }

    #[test]
    fn eine_fehlermeldung_schleppt_den_ausloeser_nicht_mit() {
        // Das Scrim-Lagebild reicht 23.000 Zeichen Rohdaten als Auslöser
        // herein. Im Fehlerfall gibt es keine Antwort zu beurteilen, und was
        // zaehlt, ist der Fehlertext.
        let mut fehler = fehler_interaktion(
            LlmUseCase::ScrimLagebild,
            &"{\"evidenzen\":[]}".repeat(2_000),
            "HTTP 412: Anbieter-Konto gesperrt",
        );
        fehler.system_excerpt = Some("X".repeat(4_000));
        let text = render_interaction(&fehler, 1, true);
        assert!(
            text.chars().count() < 500,
            "eine Fehlermeldung bleibt kurz, war {} Zeichen",
            text.chars().count()
        );
        assert!(
            text.contains("Anbieter-Konto gesperrt"),
            "der Fehlertext muss bleiben: {text}"
        );
        assert!(
            !text.contains("Kontext:"),
            "der Kontext hat im Fehlerfall nichts zu suchen: {text}"
        );
    }

    #[test]
    fn ein_verdraengter_zaehler_geht_als_nachtrag_raus() {
        let mut throttle = ErrorThrottle::default();
        let jetzt = Instant::now();
        let fenster = Duration::from_secs(600);
        let alt = fehler_interaktion(LlmUseCase::Faq, "frage", "alter fehler");

        zeigen(&mut throttle, &alt, fenster, jetzt);
        for _ in 0..4 {
            unterdruecken(&mut throttle, &alt, fenster, jetzt);
        }
        // Genug neue Fehlerarten, um den alten Eintrag zu verdraengen.
        for index in 0..ERROR_MEMORY {
            let neu = fehler_interaktion(LlmUseCase::Faq, "frage", &format!("neu {index}"));
            zeigen(&mut throttle, &neu, fenster, jetzt);
        }

        let nachtraege = throttle.faellige_nachtraege(Duration::from_secs(120), jetzt);
        assert_eq!(
            nachtraege,
            vec![(fehler_schluessel("alter fehler", None), LlmUseCase::Faq, 4)],
            "der offene Zaehler darf nicht mit dem Eintrag verschwinden"
        );
        assert!(
            repeat_followup(&nachtraege[0].0, nachtraege[0].1, nachtraege[0].2)
                .contains("Danach noch 4 mal derselbe Fehler")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn nach_der_ruhefrist_kommt_die_anzahl_als_nachtrag() {
        let messenger = FakeMessenger::arc(false);
        let config = TransparencyConfig {
            error_repeat_window: Duration::from_secs(3_600),
            error_followup_quiet: Duration::from_secs(120),
            ..testkonfig()
        };
        let log = TransparencyLog::spawn(messenger.clone(), config);
        let sink = log.sink();

        for _ in 0..4 {
            sink.record(fehler_interaktion(
                LlmUseCase::ScrimLagebild,
                "team 1",
                "LLM provider error: HTTP 412",
            ));
        }
        assert!(warte_auf(|| messenger.sent().len() == 1).await);

        // Die Fehlerserie ist vorbei, das Fenster laeuft aber noch lange.
        tokio::time::advance(Duration::from_secs(200)).await;
        assert!(
            warte_auf(|| messenger.sent().len() == 2).await,
            "die Anzahl muss den Owner auch ohne neuen Fehler erreichen"
        );

        let sent = messenger.sent();
        assert!(
            sent[1].1.contains("Danach noch 3 mal derselbe Fehler"),
            "{}",
            sent[1].1
        );
        assert!(sent[1].1.contains("HTTP 412"), "{}", sent[1].1);
    }

    #[tokio::test]
    async fn beim_herunterfahren_kommt_die_offene_anzahl_noch_raus() {
        let messenger = FakeMessenger::arc(false);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        for _ in 0..3 {
            sink.record(fehler_interaktion(
                LlmUseCase::ScrimLagebild,
                "team 1",
                "LLM provider error: HTTP 412",
            ));
        }
        assert!(warte_auf(|| messenger.sent().len() == 1).await);
        drop(sink);
        log.shutdown().await;

        let sent = messenger.sent();
        assert_eq!(
            sent.len(),
            2,
            "ein geordneter Neustart darf die Zaehler nicht verschlucken: {sent:?}"
        );
        assert!(
            sent[1].1.contains("Danach noch 2 mal derselbe Fehler"),
            "{}",
            sent[1].1
        );
    }

    #[tokio::test]
    async fn die_ankerrunde_eines_gespraechs_wird_nie_unterdrueckt() {
        let messenger = FakeMessenger::arc(true);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        // Der Fehler ist schon bekannt, das Fenster steht.
        sink.record(fehler_interaktion(
            LlmUseCase::BotPate,
            "irgendwo",
            "HTTP 412",
        ));
        assert!(warte_auf(|| messenger.sent().len() == 1).await);

        // Runde 1 eines neuen Gespraechs faellt in die Entprellung.
        let mut runde1 = fehler_interaktion(LlmUseCase::BotPate, "runde 1", "HTTP 412");
        runde1.conversation_trail = vec![7];
        sink.record(runde1);

        // Runde 2 darf nicht auch noch stumm bleiben, sonst bekommt das
        // Gespraech nie eine Ankernachricht und keinen Thread.
        let mut runde2 = fehler_interaktion(LlmUseCase::BotPate, "runde 2", "HTTP 412");
        runde2.conversation_trail = vec![7, 8];
        sink.record(runde2);
        assert!(
            warte_auf(|| messenger.sent().len() == 2).await,
            "das Gespraech braucht seine Ankernachricht: {:?}",
            messenger.sent()
        );

        let sent = messenger.sent();
        assert!(sent[1].1.contains("Runde 2"), "{}", sent[1].1);

        // Und die Runde danach haengt am Anker im Thread.
        sink.record(interaktion(
            LlmUseCase::BotPate,
            "runde 3",
            "endlich eine Antwort",
            &[8, 9],
        ));
        assert!(warte_auf(|| messenger.sent().len() == 3).await);
        assert_eq!(
            messenger.threads().len(),
            1,
            "die Ankernachricht traegt jetzt einen Thread"
        );
    }

    #[tokio::test]
    async fn ein_fehlgeschlagener_post_haelt_den_kanal_nicht_stunden_stumm() {
        let messenger = FakeMessenger::mit_fehlschlaegen(1);
        let log = TransparencyLog::spawn(messenger.clone(), testkonfig());
        let sink = log.sink();

        sink.record(fehler_interaktion(
            LlmUseCase::ScrimLagebild,
            "team 1",
            "LLM provider error: HTTP 412",
        ));
        sink.record(fehler_interaktion(
            LlmUseCase::ScrimLagebild,
            "team 2",
            "LLM provider error: HTTP 412",
        ));

        assert!(
            warte_auf(|| messenger.sent().len() == 1).await,
            "der zweite Versuch muss durchkommen, der erste ging verloren"
        );
        let sent = messenger.sent();
        assert!(sent[0].1.contains("team 2"), "{}", sent[0].1);
    }

    fn testkonfig() -> TransparencyConfig {
        TransparencyConfig {
            min_send_interval: Duration::ZERO,
            ..TransparencyConfig::default()
        }
    }

    fn interaktion(
        use_case: LlmUseCase,
        frage: &str,
        antwort: &str,
        trail: &[u64],
    ) -> AiInteraction {
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

        sink.record(interaktion(LlmUseCase::ModerationText, "wort", "ok", &[1]));
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

        assert!(
            warte_auf(|| {
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
            .await
        );

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
        assert!(
            text.contains("gekürzt"),
            "jede Kuerzung wird gekennzeichnet"
        );
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
        assert!(
            titel.chars().count() <= 100,
            "{} Zeichen",
            titel.chars().count()
        );
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
        assert!(
            aus.inventory_line().contains("aus"),
            "{}",
            aus.inventory_line()
        );
    }
}
