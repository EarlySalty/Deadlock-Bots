//! `/api/repo-activity` — liefert das vom Rust-Collector `dl-repostats`
//! erzeugte Artefakt `data/repo_activity.json` verbatim aus (Admin-Auth).
//!
//! Re-Parsen unnötig: die Datei ist klein und lokal/vertrauenswürdig. Der
//! On-Demand-Refresh (`POST /api/repo-activity/refresh`, der den Collector als
//! Subprozess startet) gehört zum Steuerungs-Cluster (systemd/Subprozess) und
//! folgt dort — er braucht das tokio-`process`-Feature.

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use crate::web::{err_text, ok_json, DashboardApp};

/// Antwort, wenn der Collector noch nie lief (Datei fehlt) — wie das Original.
fn unavailable() -> Value {
    json!({
        "available": false,
        "repos": [],
        "excludes": [],
        "error": "Noch keine Daten — der Collector (dl-repostats) lief noch nicht.",
    })
}

pub async fn repo_activity(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers) {
        return resp;
    }
    // `data/repo_activity.json` liegt neben der DB (= repo/data/).
    let Some(dir) = app.db().path().parent() else {
        return ok_json(unavailable());
    };
    let path = dir.join("repo_activity.json");
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            raw,
        )
            .into_response(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => ok_json(unavailable()),
        Err(err) => {
            tracing::error!(%err, "repo_activity.json unlesbar");
            err_text(500, "repo_activity.json unlesbar")
        }
    }
}
