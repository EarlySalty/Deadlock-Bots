//! Einheitliche JSON-Antworten und Fehler-Envelopes.
//!
//! Zwei Vertrags-Formate aus dem Python-Original:
//! - Stats-Stil: `{"error": "<code>"}` (public_stats.py)
//! - Tierlist-Stil: `{"error": "<code>", "message": "<text>"}` (tierlist_public.py)

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Stats-Stil: `{"error": code}`.
pub fn error_code(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({ "error": code }))).into_response()
}

/// Tierlist-Stil: `{"error": code, "message": text}`.
pub fn error_message(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({ "error": code, "message": message }))).into_response()
}

/// 401 im Stats-Stil — exakt `{"error": "unauthenticated"}`.
pub fn unauthenticated() -> Response {
    error_code(StatusCode::UNAUTHORIZED, "unauthenticated")
}

/// 401 im Tierlist-Stil.
pub fn unauthorized_tierlist() -> Response {
    error_message(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "Anmeldung erforderlich.",
    )
}
