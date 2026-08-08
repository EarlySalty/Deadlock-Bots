//! KI-Transparenz: der Discord-Weg der Senke und das Startinventar.
//!
//! Zwei Dinge haengen hier zusammen, weil sie dieselbe Frage beantworten:
//! „Wo redet gerade eine KI mit, und sagt sie ueberhaupt noch etwas?"
//! - Der [`DiscordTransparencyMessenger`] traegt jede Modellantwort in den
//!   Transparenz-Kanal; mehrrundige Gespraeche bekommen einen Thread.
//! - Das Startinventar schreibt beim Hochfahren in einer Zeile je Aussage,
//!   welcher Anbieter welchen Anwendungsfall bedient, ob das Transparenz-Log
//!   laeuft und wie weit der Concierge offen ist. Ein Pfad ohne nutzbaren
//!   Anbieter kommt als Warnung, denn genau der faellt sonst still aus.

use std::sync::Arc;

use serde_json::{json, Map};
use serenity::all::{ChannelId, MessageId};

/// Thread-Archivierung nach einem Tag: lange genug zum Nachlesen, kurz genug,
/// dass die Kanalliste nicht zulaeuft.
const THREAD_AUTO_ARCHIVE_MINUTES: u32 = 1_440;

pub struct DiscordTransparencyMessenger {
    pub adapter: Arc<dl_discord::DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_ai::TransparencyMessenger for DiscordTransparencyMessenger {
    async fn send(&self, channel_id: u64, content: &str) -> Result<u64, String> {
        let mut body = Map::new();
        body.insert("content".to_string(), json!(content));
        // Das Log zitiert Nutzertexte im Wortlaut — ohne diese Sperre pingt
        // eine zitierte Rollen-Erwaehnung den halben Server.
        body.insert("allowed_mentions".to_string(), json!({ "parse": [] }));
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn create_thread(
        &self,
        channel_id: u64,
        message_id: u64,
        name: &str,
    ) -> Result<u64, String> {
        let mut body = Map::new();
        body.insert("name".to_string(), json!(name));
        body.insert(
            "auto_archive_duration".to_string(),
            json!(THREAD_AUTO_ARCHIVE_MINUTES),
        );
        self.adapter
            .http
            .create_thread_from_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Some("KI-Transparenz-Log"),
            )
            .await
            .map(|channel| channel.id.get())
            .map_err(|err| err.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryLevel {
    Info,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryLine {
    pub level: InventoryLevel,
    pub text: String,
}

impl InventoryLine {
    fn info(text: impl Into<String>) -> Self {
        Self {
            level: InventoryLevel::Info,
            text: text.into(),
        }
    }

    fn warn(text: impl Into<String>) -> Self {
        Self {
            level: InventoryLevel::Warn,
            text: text.into(),
        }
    }
}

/// Baut das Startinventar der KI-Pfade.
///
/// Rein, damit der Vertrag testbar bleibt: keine User-IDs, ein Pfad ohne
/// Anbieter ist eine Warnung, und jede Aussage steht in einer eigenen Zeile.
pub fn ai_startup_inventory(
    provider_config: &Result<dl_ai::LlmProviderConfig, dl_ai::LlmProviderConfigError>,
    lookup: impl Fn(&str) -> Option<String> + Copy,
    transparency: &dl_ai::TransparencyConfig,
    concierge: &dl_community::concierge::ConciergeConfig,
) -> Vec<InventoryLine> {
    let mut lines = Vec::new();
    match provider_config {
        Ok(config) => {
            lines.push(InventoryLine::info(config.inventory_line(lookup)));
            for status in config.inventory(lookup) {
                if status.usable() {
                    continue;
                }
                lines.push(InventoryLine::warn(format!(
                    "KI-Pfad ohne nutzbaren Anbieter: {} (Anbieter {}) — Grund: {}. \
                     Der Pfad läuft still ohne KI auf seinem Vorlagentext weiter.",
                    status.use_case.as_str(),
                    status.provider_label(),
                    status.error.clone().unwrap_or_else(|| "unbekannt".into())
                )));
            }
        }
        Err(error) => lines.push(InventoryLine::warn(format!(
            "KI-Anbieterkonfiguration abgelehnt: {error}. Kein Anwendungsfall bekommt ein Modell."
        ))),
    }

    lines.push(InventoryLine::info(transparency.inventory_line()));
    if !transparency.enabled {
        lines.push(InventoryLine::warn(
            "KI-Transparenz-Log ist abgeschaltet: KI-Antworten sind nirgends mitzulesen."
                .to_string(),
        ));
    }
    lines.push(concierge_line(concierge));
    lines
}

fn concierge_line(concierge: &dl_community::concierge::ConciergeConfig) -> InventoryLine {
    if !concierge.enabled {
        return InventoryLine::info("Concierge: aus (DL_CONCIERGE_ENABLED)".to_string());
    }
    // Bewusst nur die Anzahl: eine User-ID im Journal waere ein Datenleck ohne
    // jeden Nutzen fuer die Frage „ist der Pfad offen oder eingeschraenkt?"
    let zugang = if concierge.test_user_allowlist.is_empty() {
        "offen für alle".to_string()
    } else if concierge.test_user_allowlist.len() == 1 {
        "Allowlist mit 1 Eintrag".to_string()
    } else {
        format!(
            "Allowlist mit {} Einträgen",
            concierge.test_user_allowlist.len()
        )
    };
    InventoryLine::info(format!(
        "Concierge: an, Zugang: {zugang}, proaktive DMs: {}",
        if concierge.proactive { "an" } else { "aus" }
    ))
}

/// Schreibt das Inventar ins Journal — eine Zeile je Aussage.
pub fn log_ai_startup_inventory(
    provider_config: &Result<dl_ai::LlmProviderConfig, dl_ai::LlmProviderConfigError>,
    lookup: impl Fn(&str) -> Option<String> + Copy,
    transparency: &dl_ai::TransparencyConfig,
    concierge: &dl_community::concierge::ConciergeConfig,
) {
    for line in ai_startup_inventory(provider_config, lookup, transparency, concierge) {
        match line.level {
            InventoryLevel::Info => tracing::info!("{}", line.text),
            InventoryLevel::Warn => tracing::warn!("{}", line.text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn concierge_config(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> dl_community::concierge::ConciergeConfig {
        dl_community::concierge::ConciergeConfig::from_env(lookup)
    }

    fn texte(lines: &[InventoryLine]) -> String {
        lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn inventar_nennt_anbieter_transparenzkanal_und_concierge_zugang() {
        let lookup = |key: &str| match key {
            "FIREWORK_API_KEY" | "OPENAI_API_KEY" => Some("key".to_string()),
            "DL_CONCIERGE_ENABLED" => Some("1".to_string()),
            _ => None,
        };
        let config = dl_ai::LlmProviderConfig::from_env(lookup);
        let lines = ai_startup_inventory(
            &config,
            lookup,
            &dl_ai::TransparencyConfig::default(),
            &concierge_config(lookup),
        );
        let text = texte(&lines);

        assert!(text.contains("KI-Anbieter je Anwendungsfall"), "{text}");
        assert!(text.contains("bot_pate=fireworks"), "{text}");
        assert!(text.contains("voice_hint=openai"), "{text}");
        assert!(text.contains("1374364800817303632"), "{text}");
        assert!(text.contains("Moderation gespiegelt: nein"), "{text}");
        assert!(text.contains("Concierge: an"), "{text}");
        assert!(text.contains("offen für alle"), "{text}");
        assert!(
            lines.iter().all(|line| line.level == InventoryLevel::Info),
            "mit beiden Schlüsseln darf nichts warnen: {text}"
        );
    }

    #[test]
    fn pfad_ohne_anbieter_wird_zur_warnung_mit_grund() {
        // Nur Fireworks liegt vor: die drei OpenAI-Pfade fallen still aus.
        let lookup = |key: &str| (key == "FIREWORK_API_KEY").then(|| "key".to_string());
        let config = dl_ai::LlmProviderConfig::from_env(lookup);
        let lines = ai_startup_inventory(
            &config,
            lookup,
            &dl_ai::TransparencyConfig::default(),
            &concierge_config(|_| None),
        );

        let warnungen: Vec<&InventoryLine> = lines
            .iter()
            .filter(|line| line.level == InventoryLevel::Warn)
            .collect();
        assert_eq!(warnungen.len(), 3, "{:?}", texte(&lines));
        for warnung in &warnungen {
            assert!(
                warnung.text.contains("ohne nutzbaren Anbieter"),
                "{warnung:?}"
            );
            assert!(warnung.text.contains("OPENAI_API_KEY"), "{warnung:?}");
        }
        let text = texte(&lines);
        assert!(text.contains("voice_hint"), "{text}");
        assert!(text.contains("turnier_vorschlag"), "{text}");
        assert!(text.contains("moderation_verify"), "{text}");
    }

    #[test]
    fn allowlist_erscheint_als_anzahl_und_nie_als_user_id() {
        let lookup = |key: &str| match key {
            "FIREWORK_API_KEY" | "OPENAI_API_KEY" => Some("key".to_string()),
            "DL_CONCIERGE_ENABLED" => Some("1".to_string()),
            "DL_CONCIERGE_TEST_USER_ALLOWLIST" => {
                Some("123456789012345678,987654321098765432".to_string())
            }
            _ => None,
        };
        let config = dl_ai::LlmProviderConfig::from_env(lookup);
        let text = texte(&ai_startup_inventory(
            &config,
            lookup,
            &dl_ai::TransparencyConfig::default(),
            &concierge_config(lookup),
        ));

        assert!(text.contains("Allowlist mit 2 Einträgen"), "{text}");
        assert!(
            !text.contains("123456789012345678"),
            "keine User-ID ins Journal: {text}"
        );
        assert!(!text.contains("987654321098765432"), "{text}");
    }

    #[test]
    fn abgeschaltete_transparenz_ist_eine_warnung() {
        let lookup = |key: &str| match key {
            "FIREWORK_API_KEY" | "OPENAI_API_KEY" => Some("key".to_string()),
            _ => None,
        };
        let config = dl_ai::LlmProviderConfig::from_env(lookup);
        let transparency = dl_ai::TransparencyConfig {
            enabled: false,
            ..dl_ai::TransparencyConfig::default()
        };
        let lines =
            ai_startup_inventory(&config, lookup, &transparency, &concierge_config(|_| None));
        assert!(
            lines.iter().any(|line| line.level == InventoryLevel::Warn
                && line.text.contains("nirgends mitzulesen")),
            "{:?}",
            texte(&lines)
        );
    }

    #[test]
    fn abgelehnte_anbieterkonfiguration_wird_gemeldet() {
        let lookup = |key: &str| (key == "DL_LLM_PROVIDER_FAQ").then(|| "unknown".to_string());
        let config = dl_ai::LlmProviderConfig::from_env(lookup);
        let lines = ai_startup_inventory(
            &config,
            lookup,
            &dl_ai::TransparencyConfig::default(),
            &concierge_config(|_| None),
        );
        assert!(
            lines.iter().any(|line| line.level == InventoryLevel::Warn
                && line.text.contains("Anbieterkonfiguration abgelehnt")),
            "{:?}",
            texte(&lines)
        );
    }
}
