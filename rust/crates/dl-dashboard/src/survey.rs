//! Öffentlicher Austritts-Umfrage-Flow (`/api/leave-survey/{token}`).
//!
//! Token-basiert, KEINE Admin-Anmeldung — wer den per DM verschickten Link
//! öffnet, sieht/füllt seine Umfrage. Token max. 30 Tage gültig. Reine
//! DB-/Dateisystem-Operationen (kein Bot). Hier zunächst die Anzeige (GET);
//! das Absenden inkl. Bild-Upload folgt separat.

use axum::extract::{Path, State};
use axum::response::Response;
use rusqlite::{params, OptionalExtension};
use serde_json::json;

use crate::web::{err_json, ok_json, DashboardApp};

const TOKEN_MAX_AGE: &str = "-30 days";

/// Token-Format wie `_is_valid_leave_survey_token`: nicht leer, nur
/// `[A-Za-z0-9_-]`.
pub(crate) fn is_valid_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub async fn leave_survey_get(
    State(app): State<DashboardApp>,
    Path(token): Path<String>,
) -> Response {
    let token = token.trim().to_string();
    if !is_valid_token(&token) {
        return err_json(404, "not_found");
    }
    let row = app
        .db()
        .read(move |conn| {
            conn.query_row(
                "SELECT display_name, user_bucket, reason_code, web_submitted_at
                 FROM member_leave_surveys
                 WHERE survey_token = ? AND created_at >= datetime('now', ?)
                 LIMIT 1",
                params![token, TOKEN_MAX_AGE],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()
        })
        .await;

    match row {
        Ok(Some((display_name, user_bucket, reason_code, web_submitted_at))) => ok_json(json!({
            "display_name": display_name,
            "user_bucket": user_bucket,
            "reason_code": reason_code,
            "already_submitted": web_submitted_at.is_some(),
        })),
        Ok(None) => err_json(404, "not_found"),
        Err(err) => {
            tracing::error!(%err, "leave_survey_get fehlgeschlagen");
            err_json(404, "not_found")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_validierung() {
        assert!(is_valid_token("AbC-123_xyz"));
        assert!(!is_valid_token(""));
        assert!(!is_valid_token("hat/slash"));
        assert!(!is_valid_token("dot.dot"));
        assert!(!is_valid_token("space x"));
    }
}
