//! Namensauflösung User-ID → Anzeigename für die Analytics-Reads.
//!
//! Pythons `_resolve_display_names` liest den Bot-Member-Cache. dl-web hat
//! keinen Gateway, also geht das über den Master-Broker
//! (`GET /internal/master/v1/discord/resolve-names`, Bulk). Nicht gefundene
//! IDs füllt der Aufrufer selbst mit `User <id>` auf — wie im Original.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Löst mehrere User-IDs gebündelt zu Anzeigenamen auf.
#[async_trait::async_trait]
pub trait NameResolver: Send + Sync {
    /// Liefert nur die gefundenen Namen; fehlende IDs bleiben weg.
    async fn resolve(&self, user_ids: &[u64]) -> HashMap<u64, String>;
}

/// Hilfsfunktion: Map auffüllen wie `_resolve_display_names` (`User <id>`).
pub fn display_name_or_default(names: &HashMap<u64, String>, user_id: u64) -> String {
    names
        .get(&user_id)
        .cloned()
        .unwrap_or_else(|| format!("User {user_id}"))
}

/// Broker-gestützter Resolver.
#[derive(Clone)]
pub struct BrokerNameResolver {
    http: reqwest::Client,
    base: String,
}

impl BrokerNameResolver {
    pub fn new(base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("reqwest-Client bauen");
        Self {
            http,
            base: base.into().trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait::async_trait]
impl NameResolver for BrokerNameResolver {
    async fn resolve(&self, user_ids: &[u64]) -> HashMap<u64, String> {
        if user_ids.is_empty() {
            return HashMap::new();
        }
        let ids = user_ids
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let url = format!("{}/internal/master/v1/discord/resolve-names", self.base);
        let response = match self.http.get(&url).query(&[("user_ids", ids)]).send().await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(%err, "Broker resolve-names nicht erreichbar");
                return HashMap::new();
            }
        };
        if !response.status().is_success() {
            return HashMap::new();
        }
        let Ok(data) = response.json::<Value>().await else {
            return HashMap::new();
        };
        data.get("names")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(id, name)| {
                        Some((id.parse::<u64>().ok()?, name.as_str()?.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_fuellt_fehlende_auf() {
        let mut names = HashMap::new();
        names.insert(42u64, "Nani".to_string());
        assert_eq!(display_name_or_default(&names, 42), "Nani");
        assert_eq!(display_name_or_default(&names, 99), "User 99");
    }
}
