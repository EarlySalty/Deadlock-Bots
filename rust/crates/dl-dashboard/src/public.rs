//! Öffentliche Endpunkte (`/api/public/...`) — KEINE Anmeldung, mit CORS für
//! die Community-Website. Hier zunächst die Patchnotes (reine DB); die
//! Live-Guild-Stats brauchen Bot-Daten und folgen separat.

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

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
    let result = app
        .db()
        .read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, title, url, posted_at, translated_content
                 FROM changelog_posts
                 WHERE translated_content IS NOT NULL AND translated_content != ''
                 ORDER BY id DESC",
            )?;
            let patches = stmt
                .query_map([], |row| {
                    let content: String = row.get::<_, Option<String>>(4)?.unwrap_or_default();
                    Ok(json!({
                        "id": row.get::<_, Option<i64>>(0)?,
                        "title": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        "url": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        "posted_at": row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        "sections": detect_sections(&content),
                        "translated_content": content,
                    }))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(patches)
        })
        .await;

    match result {
        Ok(patches) => {
            let total = patches.len();
            with_cors(json!({ "patches": patches, "total": total }), 200, 120)
        }
        Err(err) => with_cors(json!({ "error": err.to_string() }), 500, 120),
    }
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
