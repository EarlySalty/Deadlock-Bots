//! Client für die interne API des (noch) Python-Dashboards.
//!
//! Das Dashboard bleibt während der Migration der OAuth-Besitzer; dl-web
//! delegiert dorthin — exakt wie public_stats._call_dashboard_api bzw. der
//! Tierlist-Session-Check. Alle Aufrufe: POST JSON, `X-Internal-Token`,
//! 20-Sekunden-Timeout, Nicht-200 → None (mit Warn-Log).

use std::time::Duration;

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitiateResult {
    pub authorize_url: String,
    pub state_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumeResult {
    pub discord_id: String,
    pub discord_name: String,
    pub discord_avatar: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedSession {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub expires_at: f64,
}

#[derive(Clone)]
pub struct DashboardClient {
    http: reqwest::Client,
    base: String,
    relay_token: Option<String>,
    twitch_token: Option<String>,
}

impl DashboardClient {
    pub fn new(base: String, relay_token: Option<String>, twitch_token: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest-Client bauen");
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            relay_token,
            twitch_token,
        }
    }

    async fn post(&self, token: Option<&str>, path: &str, payload: Value) -> Option<Value> {
        let Some(token) = token.map(str::trim).filter(|t| !t.is_empty()) else {
            tracing::warn!(path, "kein Dashboard-Internal-Token konfiguriert");
            return None;
        };
        let url = format!("{}{}", self.base, path);
        let response = match self
            .http
            .post(&url)
            .header("X-Internal-Token", token)
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(path, %err, "Dashboard-API nicht erreichbar");
                return None;
            }
        };
        let status = response.status();
        if status != reqwest::StatusCode::OK {
            let body = response.text().await.unwrap_or_default();
            tracing::warn!(path, %status, body = %body.chars().take(200).collect::<String>(),
                "Dashboard-API Fehler");
            return None;
        }
        match response.json::<Value>().await {
            Ok(v) if v.is_object() => Some(v),
            _ => None,
        }
    }

    /// `POST /internal/v1/discord/initiate` → Authorize-URL + State-ID.
    pub async fn discord_initiate(
        &self,
        redirect_after: &str,
        requesting_service: &str,
    ) -> Option<InitiateResult> {
        let data = self
            .post(
                self.relay_token.as_deref(),
                "/internal/v1/discord/initiate",
                json!({
                    "scope": "identify",
                    "redirect_after": redirect_after,
                    "requesting_service": requesting_service,
                }),
            )
            .await?;
        let authorize_url = non_empty_str(&data, "authorize_url")?;
        let state_id = non_empty_str(&data, "state_id")?;
        Some(InitiateResult {
            authorize_url,
            state_id,
        })
    }

    /// `POST /internal/v1/discord/consume-result` → Discord-Identität.
    pub async fn discord_consume(&self, state_id: &str) -> Option<ConsumeResult> {
        let data = self
            .post(
                self.relay_token.as_deref(),
                "/internal/v1/discord/consume-result",
                json!({ "state_id": state_id }),
            )
            .await?;
        let discord_id = non_empty_str(&data, "discord_id")?;
        Some(ConsumeResult {
            discord_id,
            discord_name: data
                .get("discord_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            discord_avatar: data
                .get("discord_avatar")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty()),
        })
    }

    /// `POST /internal/twitch/v1/discord/validate-session` — prüft einen
    /// `master_dash_session`-Cookie-Wert. Sliding-TTL passiert serverseitig.
    /// `None` bei ungültig/abgelaufen/Fehler.
    pub async fn validate_session(&self, session_id: &str) -> Option<ValidatedSession> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return None;
        }
        let data = self
            .post(
                self.twitch_token.as_deref(),
                "/internal/twitch/v1/discord/validate-session",
                json!({ "session_id": session_id }),
            )
            .await?;
        if data.get("valid").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        Some(ValidatedSession {
            user_id: data
                .get("user_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            username: data
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            display_name: data
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            expires_at: data
                .get("expires_at")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
        })
    }
}

fn non_empty_str(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
