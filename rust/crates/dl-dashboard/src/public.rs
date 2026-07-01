//! Öffentliche Endpunkte (`/api/public/...`) — KEINE Anmeldung, mit CORS für
//! die Community-Website. Hier zunächst die Patchnotes (reine DB); die
//! Live-Guild-Stats brauchen Bot-Daten und folgen separat.

use std::time::Duration;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::now_unix_f64;
use crate::web::DashboardApp;

const ALLOW_ORIGIN: &str = "https://deutsche-deadlock-community.de";

/// Abschnitts-Erkennung wie `_detect_sections` (Reihenfolge allgemein, items,
/// helden; case-insensitive Substring-Treffer).
fn detect_sections(content: &str) -> Vec<&'static str> {
    let lower = content.to_lowercase();
    let mut sections = Vec::new();
    if lower.contains("## allgemein") || lower.contains("## general") {
        sections.push("allgemein");
    }
    if lower.contains("## items") {
        sections.push("items");
    }
    if lower.contains("## helden") || lower.contains("## heroes") {
        sections.push("helden");
    }
    sections
}

/// Antwort mit CORS-/Cache-Headern (GET-Endpunkte).
fn with_cors(body: serde_json::Value, status: u16, max_age: u32) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
        [
            (
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                ALLOW_ORIGIN.to_string(),
            ),
            (header::ACCESS_CONTROL_ALLOW_METHODS, "GET".to_string()),
            (header::CACHE_CONTROL, format!("public, max-age={max_age}")),
        ],
        Json(body),
    )
        .into_response()
}

pub async fn patch_notes(State(app): State<DashboardApp>) -> Response {
    let result = sqlx::query!(
        r#"
        SELECT id, title, url,
               to_char(posted_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "posted_at?",
               translated_content
          FROM patchnotes.changelog_posts
         WHERE translated_content IS NOT NULL
           AND translated_content != ''
         ORDER BY id DESC
        "#
    )
    .fetch_all(app.pool())
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|row| {
                let content = row.translated_content.unwrap_or_default();
                json!({
                    "id": row.id,
                    "title": row.title,
                    "url": row.url,
                    "posted_at": row.posted_at.unwrap_or_default(),
                    "sections": detect_sections(&content),
                    "translated_content": content,
                })
            })
            .collect::<Vec<_>>()
    });

    match result {
        Ok(patches) => {
            let total = patches.len();
            with_cors(json!({ "patches": patches, "total": total }), 200, 120)
        }
        Err(err) => with_cors(json!({ "error": err.to_string() }), 500, 120),
    }
}

async fn fetch_guild_stats(base: &str) -> Option<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let url = format!(
        "{}/internal/master/v1/discord/guild-stats",
        base.trim_end_matches('/')
    );
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let data: Value = resp.json().await.ok()?;
    if data.get("found").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some(data)
}

pub async fn guild_stats(State(app): State<DashboardApp>) -> Response {
    let now = now_unix_f64();
    if let Some(cached) = app.guild_stats_cached(now) {
        return with_cors(cached, 200, 30);
    }
    let Some(data) = fetch_guild_stats(app.broker_base()).await else {
        // Kein Gateway/Guild → wie das Original 503.
        return with_cors(json!({ "error": "No guild data available" }), 503, 30);
    };
    let body = json!({
        "member_count": data.get("member_count").and_then(Value::as_u64).unwrap_or(0),
        "online_count": data.get("online_count").and_then(Value::as_u64).unwrap_or(0),
        "voice_count": data.get("voice_count").and_then(Value::as_u64).unwrap_or(0),
        "cached_at": chrono::Utc::now().to_rfc3339(),
    });
    app.set_guild_stats_cache(body.clone(), now);
    with_cors(body, 200, 30)
}

/// CORS-Preflight (OPTIONS) für die öffentlichen Endpunkte.
pub async fn public_cors() -> Response {
    (
        StatusCode::OK,
        [
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, ALLOW_ORIGIN),
            (header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS"),
            (header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type"),
            (header::ACCESS_CONTROL_MAX_AGE, "86400"),
        ],
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sektionen_erkennen() {
        assert_eq!(
            detect_sections("Text\n## Allgemein\n...\n## Items\n## Helden"),
            vec!["allgemein", "items", "helden"]
        );
        assert_eq!(detect_sections("## General only"), vec!["allgemein"]);
        assert_eq!(detect_sections("## Heroes"), vec!["helden"]);
        assert!(detect_sections("nichts").is_empty());
    }
}
