//! Discord-OAuth2-Client (Authorization-Code-Flow).
//!
//! Bildet `_build_discord_authorize_url`, `_exchange_discord_code`,
//! `_fetch_discord_user` und `_extract_steam_connection_ids` aus
//! `service/dashboard.py` nach. Netzwerkaufrufe liefern bei Fehler `None`
//! (mit Warn-Log) — die Route entscheidet dann über 401/Redirect, wie im
//! Original.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// Antwort des Token-Endpunkts (nur die genutzten Felder).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Discord-Nutzer aus `/users/@me`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordUser {
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub discriminator: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
}

impl DiscordUser {
    /// Anzeigename wie im Original: globaler Name, sonst `name#discriminator`
    /// (nur für Legacy-Discriminatoren ≠ "0"), sonst der reine Username.
    pub fn display_name(&self) -> String {
        if let Some(global) = self.global_name.as_deref().map(str::trim) {
            if !global.is_empty() {
                return global.to_string();
            }
        }
        match self.discriminator.as_deref() {
            Some(disc) if !disc.is_empty() && disc != "0" => {
                format!("{}#{}", self.username, disc)
            }
            _ => self.username.clone(),
        }
    }

    /// Name für die delegierten Endpunkte: globaler Name, sonst Username,
    /// sonst `User <id>` (KEIN Discriminator-Suffix — anders als
    /// [`display_name`](Self::display_name), das den Admin-Login betrifft).
    pub fn delegated_name(&self) -> String {
        if let Some(global) = self.global_name.as_deref().map(str::trim) {
            if !global.is_empty() {
                return global.to_string();
            }
        }
        let username = self.username.trim();
        if !username.is_empty() {
            return username.to_string();
        }
        format!("User {}", self.id)
    }

    /// Vollständige Avatar-URL (animierte Avatare als GIF) oder `None`.
    pub fn avatar_url(&self) -> Option<String> {
        let avatar = self.avatar.as_deref().filter(|a| !a.is_empty())?;
        let ext = if avatar.starts_with("a_") {
            "gif"
        } else {
            "png"
        };
        Some(format!(
            "https://cdn.discordapp.com/avatars/{}/{}.{}",
            self.id, avatar, ext
        ))
    }

    pub fn id_u64(&self) -> Option<u64> {
        self.id.trim().parse().ok()
    }
}

#[derive(Clone)]
pub struct OAuthClient {
    http: reqwest::Client,
    client_id: Option<String>,
    client_secret: Option<String>,
    api_base: String,
}

impl OAuthClient {
    pub fn new(
        client_id: Option<String>,
        client_secret: Option<String>,
        api_base: impl Into<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("reqwest-Client bauen");
        Self {
            http,
            client_id,
            client_secret,
            api_base: api_base.into().trim_end_matches('/').to_string(),
        }
    }

    /// Baut die Authorize-URL (Browser-Redirect zu Discord).
    pub fn authorize_url(&self, scope: &str, redirect_uri: &str, state: &str) -> String {
        let client_id = self.client_id.as_deref().unwrap_or_default();
        let query = form_urlencode(&[
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", scope),
            ("state", state),
        ]);
        format!("{}/oauth2/authorize?{}", self.api_base, query)
    }

    /// Authorize-URL OHNE `state` — für die turnier/twitch-`authorize-url`-
    /// Endpunkte, deren Aufrufer den State selbst verwaltet.
    pub fn authorize_url_no_state(&self, scope: &str, redirect_uri: &str) -> String {
        let client_id = self.client_id.as_deref().unwrap_or_default();
        let query = form_urlencode(&[
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", scope),
        ]);
        format!("{}/oauth2/authorize?{}", self.api_base, query)
    }

    /// Tauscht einen Authorization-Code gegen ein Access-Token.
    pub async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Option<TokenResponse> {
        let (Some(client_id), Some(client_secret)) =
            (self.client_id.as_deref(), self.client_secret.as_deref())
        else {
            tracing::warn!("OAuth-Token-Tausch ohne konfigurierte Client-Zugangsdaten");
            return None;
        };
        let url = format!("{}/oauth2/token", self.api_base);
        let form = [
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ];
        let response = match self.http.post(&url).form(&form).send().await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(%err, "Discord-Token-Endpunkt nicht erreichbar");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "Discord-Token-Tausch fehlgeschlagen");
            return None;
        }
        response.json::<TokenResponse>().await.ok()
    }

    /// Holt das Discord-Profil zum Access-Token.
    pub async fn fetch_user(&self, access_token: &str) -> Option<DiscordUser> {
        let value = self.get_authed("/users/@me", access_token).await?;
        serde_json::from_value(value).ok()
    }

    /// Holt die verknüpften Connections (z. B. Steam) zum Access-Token.
    pub async fn fetch_connections(&self, access_token: &str) -> Option<Vec<Value>> {
        let value = self
            .get_authed("/users/@me/connections", access_token)
            .await?;
        match value {
            Value::Array(items) => Some(items),
            _ => Some(Vec::new()),
        }
    }

    async fn get_authed(&self, path: &str, access_token: &str) -> Option<Value> {
        let url = format!("{}{}", self.api_base, path);
        let response = match self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
        {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(path, %err, "Discord-API nicht erreichbar");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(path, status = %response.status(), "Discord-API Fehler");
            return None;
        }
        response.json::<Value>().await.ok()
    }
}

/// Sammelt Steam-IDs aus Discord-Connections wie
/// `_extract_steam_connection_ids` (Kandidaten: `id`, `name`,
/// `metadata.steam_id`; Duplikate werden in Reihenfolge entfernt).
pub fn extract_steam_connection_ids(connections: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for conn in connections {
        let is_steam = conn
            .get("type")
            .and_then(Value::as_str)
            .map(|t| t.eq_ignore_ascii_case("steam"))
            .unwrap_or(false);
        if !is_steam {
            continue;
        }
        let candidates = [
            conn.get("id").and_then(Value::as_str),
            conn.get("name").and_then(Value::as_str),
            conn.get("metadata")
                .and_then(|m| m.get("steam_id"))
                .and_then(Value::as_str),
        ];
        for candidate in candidates.into_iter().flatten() {
            let value = candidate.trim();
            if !value.is_empty() && !out.iter().any(|e| e == value) {
                out.push(value.to_string());
            }
        }
    }
    out
}

/// `application/x-www-form-urlencoded` mit quote_plus-Semantik (Space → `+`),
/// passend zu Pythons `urllib.parse.urlencode`.
fn form_urlencode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn client() -> OAuthClient {
        OAuthClient::new(
            Some("12345".into()),
            Some("secret".into()),
            "https://discord.com/api/v10",
        )
    }

    #[test]
    fn authorize_url_kodiert_korrekt() {
        let url = client().authorize_url(
            "identify guilds.members.read",
            "https://deutsche-deadlock-community.de/callback/discord",
            "STATE-123",
        );
        assert!(url.starts_with("https://discord.com/api/v10/oauth2/authorize?"));
        assert!(url.contains("client_id=12345"));
        assert!(url.contains("response_type=code"));
        // Space → '+', '/' und ':' prozentkodiert.
        assert!(url.contains("scope=identify+guilds.members.read"));
        assert!(url.contains(
            "redirect_uri=https%3A%2F%2Fdeutsche-deadlock-community.de%2Fcallback%2Fdiscord"
        ));
        assert!(url.contains("state=STATE-123"));
    }

    #[test]
    fn display_name_prioritaet() {
        let global = DiscordUser {
            id: "1".into(),
            username: "nani".into(),
            global_name: Some("Nani".into()),
            discriminator: Some("0".into()),
            avatar: None,
        };
        assert_eq!(global.display_name(), "Nani");

        let legacy = DiscordUser {
            id: "1".into(),
            username: "nani".into(),
            global_name: None,
            discriminator: Some("1234".into()),
            avatar: None,
        };
        assert_eq!(legacy.display_name(), "nani#1234");

        let modern = DiscordUser {
            id: "1".into(),
            username: "nani".into(),
            global_name: None,
            discriminator: Some("0".into()),
            avatar: None,
        };
        assert_eq!(modern.display_name(), "nani");
    }

    #[test]
    fn avatar_url_animiert_vs_statisch() {
        let animated = DiscordUser {
            id: "77".into(),
            username: "x".into(),
            global_name: None,
            discriminator: None,
            avatar: Some("a_abc".into()),
        };
        assert_eq!(
            animated.avatar_url().as_deref(),
            Some("https://cdn.discordapp.com/avatars/77/a_abc.gif")
        );

        let none = DiscordUser {
            id: "77".into(),
            username: "x".into(),
            global_name: None,
            discriminator: None,
            avatar: None,
        };
        assert!(none.avatar_url().is_none());
    }

    #[test]
    fn steam_connections_extrahieren_und_deduplizieren() {
        let connections = vec![
            json!({"type": "steam", "id": "765", "name": "765"}),
            json!({"type": "STEAM", "metadata": {"steam_id": "888"}}),
            json!({"type": "twitch", "id": "ignore"}),
            json!({"type": "steam", "id": "765"}),
        ];
        let ids = extract_steam_connection_ids(&connections);
        assert_eq!(ids, vec!["765", "888"]);
    }
}
