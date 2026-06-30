//! Öffentlicher Austritts-Umfrage-Flow (`/api/leave-survey/{token}`).
//!
//! Token-basiert, KEINE Admin-Anmeldung — wer den per DM verschickten Link
//! öffnet, sieht/füllt seine Umfrage. Token max. 30 Tage gültig. Reine
//! DB-/Dateisystem-Operationen (kein Bot). Anzeige (GET), Absenden inkl.
//! Bild-Upload (POST, multipart) und der Admin-geschützte Bild-Abruf
//! (GET image) — Port von `_handle_leave_survey_*` in dashboard.py.

use std::path::{Path as FsPath, PathBuf};

use axum::extract::{FromRequest, Multipart, Path, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{Duration, Utc};
use rand::Rng;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::db::{advisory_lock, DashboardDbError, DashboardDbResult};
use crate::web::{err_json, err_text, ok_json, DashboardApp};

const TOKEN_MAX_AGE_DAYS: i64 = 30;
const MEMBER_LEAVE_SURVEYS_TOKEN_LOCK: i64 = 0x4441_5348_5355_5256;
/// Wie `LEAVE_SURVEY_MAX_IMAGES`.
const MAX_IMAGES: usize = 5;
/// Wie `LEAVE_SURVEY_MAX_IMAGE_BYTES` (5 MiB pro Bild).
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Token-Format wie `_is_valid_leave_survey_token`: nicht leer, nur
/// `[A-Za-z0-9_-]`.
pub(crate) fn is_valid_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn token_cutoff() -> chrono::DateTime<Utc> {
    Utc::now() - Duration::days(TOKEN_MAX_AGE_DAYS)
}

pub async fn leave_survey_get(
    State(app): State<DashboardApp>,
    Path(token): Path<String>,
) -> Response {
    let token = token.trim().to_string();
    if !is_valid_token(&token) {
        return err_json(404, "not_found");
    }
    let cutoff = token_cutoff();
    let row = sqlx::query!(
        r#"
        SELECT display_name, user_bucket, reason_code,
               web_submitted_at IS NOT NULL AS "already_submitted!"
          FROM activity.member_leave_surveys
         WHERE survey_token = $1
           AND created_at >= $2
         ORDER BY id
         LIMIT 1
        "#,
        token,
        cutoff,
    )
    .fetch_optional(app.pool())
    .await;

    match row {
        Ok(Some(row)) => ok_json(json!({
            "display_name": row.display_name,
            "user_bucket": row.user_bucket,
            "reason_code": row.reason_code,
            "already_submitted": row.already_submitted,
        })),
        Ok(None) => err_json(404, "not_found"),
        Err(err) => {
            tracing::error!(%err, "leave_survey_get fehlgeschlagen");
            err_json(404, "not_found")
        }
    }
}

/// Dateinamen-Format wie `_is_valid_leave_survey_filename`: nicht leer, kein
/// führender Punkt, kein `/`/`\`/`..`, nur `[A-Za-z0-9._-]`.
pub(crate) fn is_valid_filename(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// Erlaubte Datei-Endungen je Content-Type (wie `LEAVE_SURVEY_ALLOWED_IMAGE_TYPES`).
fn allowed_exts(content_type: &str) -> Option<&'static [&'static str]> {
    match content_type {
        "image/jpeg" => Some(&[".jpg", ".jpeg"]),
        "image/png" => Some(&[".png"]),
        "image/webp" => Some(&[".webp"]),
        "image/gif" => Some(&[".gif"]),
        _ => None,
    }
}

/// Content-Type für die Auslieferung anhand der Datei-Endung.
fn mime_for_name(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else {
        "application/octet-stream"
    }
}

/// Upload-Verzeichnis = `<DB-Parent>/leave_survey_uploads/<token>` (entspricht
/// `repo/data/leave_survey_uploads/<token>`, da die DB unter `data/` liegt).
fn upload_dir(app: &DashboardApp, token: &str) -> PathBuf {
    app.data_dir().join("leave_survey_uploads").join(token)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmitOutcome {
    Updated,
    AlreadySubmitted,
    NotFound,
}

async fn submit_leave_survey(
    pool: &PgPool,
    token: &str,
    payload: &str,
    now: chrono::DateTime<Utc>,
) -> DashboardDbResult<SubmitOutcome> {
    let mut tx = pool.begin().await?;
    advisory_lock(&mut tx, MEMBER_LEAVE_SURVEYS_TOKEN_LOCK).await?;
    let cutoff = now - Duration::days(TOKEN_MAX_AGE_DAYS);
    let summary = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!",
               COALESCE(BOOL_OR(web_submitted_at IS NOT NULL), FALSE) AS "submitted!"
          FROM activity.member_leave_surveys
         WHERE survey_token = $1
           AND created_at >= $2
        "#,
        token,
        cutoff,
    )
    .fetch_one(&mut *tx)
    .await?;
    if summary.count == 0 {
        tx.commit().await?;
        return Ok(SubmitOutcome::NotFound);
    }
    if summary.count > 1 {
        return Err(DashboardDbError::UniqueViolation(
            "activity.member_leave_surveys.survey_token",
        ));
    }
    if summary.submitted {
        tx.commit().await?;
        return Ok(SubmitOutcome::AlreadySubmitted);
    }
    let updated = sqlx::query!(
        r#"
        UPDATE activity.member_leave_surveys
           SET web_payload = $1::text::jsonb,
               web_submitted_at = $2
         WHERE survey_token = $3
           AND created_at >= $4
           AND web_submitted_at IS NULL
        "#,
        payload,
        now,
        token,
        cutoff,
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    if updated > 0 {
        Ok(SubmitOutcome::Updated)
    } else {
        Ok(SubmitOutcome::AlreadySubmitted)
    }
}

/// `secrets.token_hex(n)`-Äquivalent: n Zufallsbytes als Hex.
fn rand_hex(nbytes: usize) -> String {
    let mut bytes = vec![0u8; nbytes];
    rand::thread_rng().fill(&mut bytes[..]);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// POST `/api/leave-survey/{token}` — Antworten + bis zu 5 Bilder (multipart).
///
/// Reihenfolge wie das Original: erst Token/Row prüfen (404/409), dann
/// Content-Type (400), dann Felder parsen, Bilder speichern, atomar
/// `web_payload`/`web_submitted_at` setzen (nur wenn noch nicht abgesendet).
pub async fn leave_survey_post(
    State(app): State<DashboardApp>,
    Path(token): Path<String>,
    request: Request,
) -> Response {
    let token = token.trim().to_string();
    if !is_valid_token(&token) {
        return err_json(404, "not_found");
    }

    // 1. Row prüfen (Existenz + bereits abgesendet) vor dem Body-Konsum.
    let token_check = token.clone();
    let cutoff = token_cutoff();
    let existing = sqlx::query!(
        r#"
        SELECT web_submitted_at
          FROM activity.member_leave_surveys
         WHERE survey_token = $1
           AND created_at >= $2
         ORDER BY id
         LIMIT 1
        "#,
        token_check,
        cutoff,
    )
    .fetch_optional(app.pool())
    .await;
    match existing {
        Ok(Some(row)) if row.web_submitted_at.is_some() => {
            return err_json(409, "already_submitted");
        }
        Ok(Some(_)) => {}
        Ok(None) => return err_json(404, "not_found"),
        Err(err) => {
            tracing::error!(%err, "leave_survey_post Row-Lookup fehlgeschlagen");
            return err_json(404, "not_found");
        }
    }

    // 2. Content-Type.
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !content_type.starts_with("multipart/") {
        return err_json(400, "invalid_content_type");
    }

    // 3. Felder parsen.
    let mut multipart = match Multipart::from_request(request, &()).await {
        Ok(m) => m,
        Err(_) => return err_json(400, "invalid_content_type"),
    };

    let mut answers: Option<Value> = None;
    let mut pending_images: Vec<(String, Vec<u8>)> = Vec::new();
    let mut image_count = 0usize;

    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return err_json(500, "submission_failed"),
        };
        let name = field.name().unwrap_or("").to_string();

        if name == "answers" {
            if answers.is_some() {
                return err_json(400, "duplicate_answers");
            }
            let raw = match field.text().await {
                Ok(t) => t,
                Err(_) => return err_json(400, "invalid_answers_json"),
            };
            let parsed: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(_) => return err_json(400, "invalid_answers_json"),
            };
            if !parsed.is_object() {
                return err_json(400, "invalid_answers_payload");
            }
            answers = Some(parsed);
            continue;
        }

        if name != "images" {
            // Unbekanntes Feld leeren und ignorieren.
            while let Ok(Some(_)) = field.chunk().await {}
            continue;
        }

        image_count += 1;
        if image_count > MAX_IMAGES {
            return err_json(413, "too_many_images");
        }
        let field_ct = field
            .content_type()
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let exts = match allowed_exts(&field_ct) {
            Some(e) => e,
            None => return err_json(400, "invalid_image_type"),
        };
        let orig_ext = field
            .file_name()
            .and_then(|f| FsPath::new(f).extension().and_then(|e| e.to_str()))
            .map(|e| format!(".{}", e.to_ascii_lowercase()))
            .unwrap_or_default();
        let ext = if exts.contains(&orig_ext.as_str()) {
            orig_ext
        } else {
            exts[0].to_string()
        };

        let mut data: Vec<u8> = Vec::new();
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if data.len() + chunk.len() > MAX_IMAGE_BYTES {
                        return err_json(413, "image_too_large");
                    }
                    data.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => return err_json(500, "upload_save_failed"),
            }
        }
        pending_images.push((ext, data));
    }

    let answers = match answers {
        Some(a) => a,
        None => return err_json(400, "missing_answers"),
    };

    // 4. Bilder speichern.
    let dir = upload_dir(&app, &token);
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        return err_json(500, "upload_save_failed");
    }
    let mut saved: Vec<String> = Vec::new();
    for (index, (ext, data)) in pending_images.iter().enumerate() {
        let filename = format!("image-{}-{}{}", index + 1, rand_hex(8), ext);
        if tokio::fs::write(dir.join(&filename), data).await.is_err() {
            // Bereits geschriebene Dateien aufräumen.
            for f in &saved {
                let _ = tokio::fs::remove_file(dir.join(f)).await;
            }
            return err_json(500, "upload_save_failed");
        }
        saved.push(filename);
    }

    // 5. Atomar setzen (nur wenn noch nicht abgesendet).
    let payload = json!({ "answers": answers, "images": saved }).to_string();
    let now = chrono::Utc::now();
    let token_db = token.clone();
    let updated = match submit_leave_survey(app.pool(), &token_db, &payload, now).await {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!(%err, "leave_survey_post UPDATE fehlgeschlagen");
            return err_json(500, "submission_failed");
        }
    };
    if updated != SubmitOutcome::Updated {
        for f in &saved {
            let _ = tokio::fs::remove_file(dir.join(f)).await;
        }
        return match updated {
            SubmitOutcome::AlreadySubmitted => err_json(409, "already_submitted"),
            SubmitOutcome::NotFound => err_json(404, "not_found"),
            SubmitOutcome::Updated => ok_json(json!({ "ok": true })),
        };
    }
    ok_json(json!({ "ok": true }))
}

/// GET `/api/leave-surveys/image/{token}/{filename}` — sessiongeschützter Abruf
/// eines hochgeladenen Bildes (wie `_handle_leave_survey_image`, `_check_auth`).
pub async fn leave_survey_image(
    State(app): State<DashboardApp>,
    Path((token, filename)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let token = token.trim().to_string();
    let filename = filename.trim().to_string();
    if !is_valid_token(&token) || !is_valid_filename(&filename) {
        return err_text(404, "Image not found");
    }
    let path = upload_dir(&app, &token).join(&filename);
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime_for_name(&filename))],
            bytes,
        )
            .into_response(),
        Err(_) => err_text(404, "Image not found"),
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

    #[test]
    fn filename_validierung() {
        assert!(is_valid_filename("image-1-abcdef0123456789.png"));
        assert!(!is_valid_filename(""));
        assert!(!is_valid_filename(".hidden"));
        assert!(!is_valid_filename("../escape.png"));
        assert!(!is_valid_filename("dir/file.png"));
        assert!(!is_valid_filename("back\\slash.png"));
    }

    #[test]
    fn ext_mapping() {
        assert_eq!(allowed_exts("image/png"), Some(&[".png"][..]));
        assert_eq!(allowed_exts("image/jpeg"), Some(&[".jpg", ".jpeg"][..]));
        assert!(allowed_exts("image/svg+xml").is_none());
        assert_eq!(mime_for_name("x.JPG"), "image/jpeg");
        assert_eq!(mime_for_name("x.bin"), "application/octet-stream");
    }

    #[test]
    fn rand_hex_laenge() {
        assert_eq!(rand_hex(8).len(), 16);
        assert!(rand_hex(8).chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn submit_leave_survey_emuliert_token_unique_constraint(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let now = Utc::now();
        for id in [1_i64, 2_i64] {
            sqlx::query!(
                r#"
                INSERT INTO activity.member_leave_surveys(
                    id, user_id, guild_id, left_at, display_name, user_bucket,
                    survey_token, created_at
                )
                VALUES ($1, $2, 42, $3, 'Nani', 'regular', 'dup-token', $3)
                "#,
                id,
                1000_i64 + id,
                now,
            )
            .execute(db.pool())
            .await?;
        }

        let err = submit_leave_survey(db.pool(), "dup-token", r#"{"answers":{},"images":[]}"#, now)
            .await
            .expect_err("duplicate token must fail");

        assert!(matches!(
            err,
            DashboardDbError::UniqueViolation("activity.member_leave_surveys.survey_token")
        ));
        let updated = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.member_leave_surveys
             WHERE web_submitted_at IS NOT NULL
            "#
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(updated, 0);
        Ok(())
    }
}
