//! `/api/repo-activity` — liefert das vom Rust-Collector `dl-repostats`
//! erzeugte Artefakt `data/repo_activity.json` verbatim aus (Session-Auth), plus
//! `POST /api/repo-activity/refresh`, der den Collector als Subprozess startet
//! (On-Demand „Neu sammeln") und das aktualisierte Artefakt zurückgibt.
//!
//! Re-Parsen unnötig: die Datei ist klein und lokal/vertrauenswürdig.

use std::time::Duration;

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
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    // `data/repo_activity.json` liegt im konfigurierten Datenverzeichnis.
    let path = app.data_dir().join("repo_activity.json");
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

/// POST `/api/repo-activity/refresh` — startet `dl-repostats` (cwd = Repo-Wurzel,
/// 60s-Timeout) und liefert danach das frische Artefakt (Port von
/// `_handle_repo_activity_refresh`). Voll-Zugriff + CSRF via `guard_mutate`.
pub async fn repo_activity_refresh(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, true).await {
        return resp;
    }
    // Binary unter repo/rust/target/release/, Repo-Wurzel = Data-Parent.
    let (bin, repo_root) = {
        let Some(rr) = app.repo_root() else {
            return err_text(500, "Repo-Pfad nicht auflösbar");
        };
        (
            rr.join("rust/target/release/dl-repostats"),
            rr.to_path_buf(),
        )
    };
    if !bin.exists() {
        return err_text(
            503,
            "dl-repostats-Binary nicht gebaut (cargo build --release -p dl-repostats).",
        );
    }
    tracing::info!("AUDIT master-dashboard repo-activity refresh");

    let mut cmd = tokio::process::Command::new(&bin);
    cmd.current_dir(&repo_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            tracing::error!(%err, "dl-repostats Start fehlgeschlagen");
            return err_text(500, "Collector-Start fehlgeschlagen");
        }
    };
    let out = match tokio::time::timeout(Duration::from_secs(60), child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(err)) => {
            tracing::error!(%err, "dl-repostats Lauf fehlgeschlagen");
            return err_text(500, "Collector-Lauf fehlgeschlagen");
        }
        Err(_) => return err_text(504, "Collector-Lauf hat zu lange gedauert (>60s)."),
    };
    if !out.status.success() {
        let detail: String = String::from_utf8_lossy(&out.stderr)
            .chars()
            .take(500)
            .collect();
        tracing::error!(detail = %detail, "dl-repostats exit != 0");
        return err_text(500, "Collector-Lauf fehlgeschlagen");
    }

    // Aktualisiertes Artefakt zurückgeben.
    repo_activity(State(app), headers).await
}
