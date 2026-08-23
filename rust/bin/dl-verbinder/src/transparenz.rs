//! Der Weg des Verbinders ins KI-Transparenz-Log.
//!
//! Der Verbinder laeuft als eigener Prozess am Timer, nicht im dl-bot. Die
//! Senke, die dort beim Start registriert wird, gibt es hier also nicht — ohne
//! dieses Modul faellt jedes Verbinder-Urteil aus dem Kanal heraus, obwohl es
//! ein Modell gefaellt hat.
//!
//! Discord wird direkt per REST angesprochen, so wie der Staff-Post es schon
//! tut: ein Gateway braucht der Verbinder fuer zwei Aufrufe nicht.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

/// Wie im dl-bot: einen Tag lesbar, dann archiviert.
const THREAD_AUTO_ARCHIVE_MINUTES: u32 = 1_440;

/// Discord antwortet auf einen haengenden Post irgendwann gar nicht mehr; der
/// Timer-Lauf soll daran nicht kleben bleiben.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

pub struct RestTransparencyMessenger {
    client: reqwest::Client,
    token: String,
}

impl RestTransparencyMessenger {
    pub fn new(token: String) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { client, token })
    }

    async fn post_und_id(&self, url: String, body: serde_json::Value) -> Result<u64, String> {
        let antwort = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .header("User-Agent", "dl-verbinder/0.1")
            .json(&body)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let status = antwort.status();
        if !status.is_success() {
            // Der Antworttext nennt den Grund (fehlende Rechte, falscher
            // Kanal); ohne ihn steht im Journal nur eine nackte Zahl.
            let text = antwort.text().await.unwrap_or_default();
            return Err(format!(
                "HTTP {status}: {}",
                text.chars().take(200).collect::<String>()
            ));
        }
        let daten: serde_json::Value = antwort.json().await.map_err(|error| error.to_string())?;
        daten
            .get("id")
            .and_then(|id| id.as_str())
            .and_then(|id| id.parse::<u64>().ok())
            .ok_or_else(|| "Antwort ohne verwertbare id".to_string())
    }
}

#[async_trait]
impl dl_ai::TransparencyMessenger for RestTransparencyMessenger {
    async fn send(&self, channel_id: u64, content: &str) -> Result<u64, String> {
        self.post_und_id(
            nachrichten_url(channel_id),
            json!({
                "content": content,
                // Zitierte Nutzertexte duerfen niemanden anpingen.
                "allowed_mentions": {"parse": []},
            }),
        )
        .await
    }

    async fn create_thread(
        &self,
        channel_id: u64,
        message_id: u64,
        name: &str,
    ) -> Result<u64, String> {
        self.post_und_id(
            thread_url(channel_id, message_id),
            json!({
                "name": name,
                "auto_archive_duration": THREAD_AUTO_ARCHIVE_MINUTES,
            }),
        )
        .await
    }
}

pub fn nachrichten_url(channel_id: u64) -> String {
    format!("https://discord.com/api/v10/channels/{channel_id}/messages")
}

pub fn thread_url(channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/api/v10/channels/{channel_id}/messages/{message_id}/threads")
}

/// Dieselbe Token-Kette wie der Staff-Post: wer den einen Weg konfiguriert
/// hat, hat den anderen damit auch.
pub fn transparenz_token(lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["DISCORD_TOKEN", "BOT_TOKEN"]
        .into_iter()
        .find_map(lookup)
        .map(|wert| wert.trim().to_string())
        .filter(|wert| !wert.is_empty())
}

/// Startet das Log und registriert die Senke global.
///
/// `None` heisst: es wird nichts mitgeschrieben. Beide Gruende dafuer stehen
/// als Warnung im Journal, denn ein stummes Log sieht von aussen aus wie ein
/// Lauf ohne KI.
pub fn starte(
    config: dl_ai::TransparencyConfig,
    token: Option<String>,
) -> Option<dl_ai::TransparencyLog> {
    if !config.enabled {
        tracing::warn!(
            "KI-Transparenz-Log aus (DL_AI_TRANSPARENCY_ENABLED): Verbinder-Urteile sind nirgends mitzulesen"
        );
        return None;
    }
    let Some(token) = token else {
        tracing::warn!(
            "DISCORD_TOKEN und BOT_TOKEN fehlen: Verbinder-Urteile laufen ohne Transparenz-Log"
        );
        return None;
    };
    let messenger = match RestTransparencyMessenger::new(token) {
        Ok(messenger) => Arc::new(messenger) as Arc<dyn dl_ai::TransparencyMessenger>,
        Err(error) => {
            tracing::warn!(%error, "HTTP-Client fuer das Transparenz-Log nicht baubar");
            return None;
        }
    };
    let log = dl_ai::TransparencyLog::spawn(messenger, config);
    dl_ai::set_transparency_sink(log.sink());
    Some(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_kette_nimmt_discord_token_vor_bot_token() {
        let beide = transparenz_token(|key| match key {
            "DISCORD_TOKEN" => Some("erst".to_string()),
            "BOT_TOKEN" => Some("zweit".to_string()),
            _ => None,
        });
        assert_eq!(beide.as_deref(), Some("erst"));

        let nur_bot = transparenz_token(|key| (key == "BOT_TOKEN").then(|| "zweit".to_string()));
        assert_eq!(nur_bot.as_deref(), Some("zweit"));
    }

    #[test]
    fn leeres_token_zaehlt_als_fehlend() {
        // Ein gesetzter, aber leerer Schluessel ist der haeufigere Fall als ein
        // fehlender — er kommt aus einem Drop-in ohne Wert.
        assert_eq!(transparenz_token(|_| Some("   ".to_string())), None);
        assert_eq!(transparenz_token(|_| None), None);
    }

    #[test]
    fn urls_treffen_die_discord_endpunkte() {
        assert_eq!(
            nachrichten_url(1_374_364_800_817_303_632),
            "https://discord.com/api/v10/channels/1374364800817303632/messages"
        );
        assert_eq!(
            thread_url(1_374_364_800_817_303_632, 42),
            "https://discord.com/api/v10/channels/1374364800817303632/messages/42/threads"
        );
    }

    /// Die Transparenz-Senke ist prozessweit (`dl_ai::set_transparency_sink`
    /// schreibt in ein `static`). libtest faehrt die Tests dieses Binaries
    /// parallel, also muessen sich alle Senken-Tests seriell abwechseln.
    static SENKE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn senke_exklusiv() -> std::sync::MutexGuard<'static, ()> {
        SENKE_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[tokio::test]
    async fn ohne_token_wird_nichts_gestartet_und_keine_senke_registriert() {
        let _guard = senke_exklusiv();
        dl_ai::clear_transparency_sink();
        let log = starte(dl_ai::TransparencyConfig::default(), None);
        assert!(log.is_none(), "ohne Token darf kein Log laufen");
        assert!(
            dl_ai::transparency_sink().is_none(),
            "ohne Log darf keine Senke stehenbleiben"
        );
    }

    #[tokio::test]
    async fn abgeschaltete_konfiguration_startet_nichts() {
        let _guard = senke_exklusiv();
        dl_ai::clear_transparency_sink();
        let config = dl_ai::TransparencyConfig {
            enabled: false,
            ..dl_ai::TransparencyConfig::default()
        };
        assert!(starte(config, Some("token".to_string())).is_none());
        assert!(dl_ai::transparency_sink().is_none());
    }

    #[tokio::test]
    async fn mit_token_laeuft_das_log_und_die_senke_steht() {
        let _guard = senke_exklusiv();
        dl_ai::clear_transparency_sink();
        let log = starte(
            dl_ai::TransparencyConfig::default(),
            Some("token".to_string()),
        )
        .expect("mit Token muss das Log laufen");
        assert!(
            dl_ai::transparency_sink().is_some(),
            "der Decorator findet sonst keine Senke und verwirft jede Interaktion"
        );
        log.shutdown().await;
        assert!(
            dl_ai::transparency_sink().is_none(),
            "shutdown muss die Senke wieder abmelden"
        );
    }
}
