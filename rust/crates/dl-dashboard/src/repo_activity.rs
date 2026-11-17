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

static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let Ok(data) = std::path::absolute(app.data_dir()) else {
        tracing::error!("repo_activity: data dir nicht absolut auflösbar");
        return err_text(500, "Repo-Pfad nicht auflösbar");
    };
    let Some(repo_root) = data.parent() else {
        tracing::error!(data = %data.display(), "repo_activity: data dir ohne Elternverzeichnis");
        return err_text(500, "Repo-Pfad nicht auflösbar");
    };
    let Some(base) = repo_root.parent() else {
        tracing::error!(root = %repo_root.display(), "repo_activity: Repo-Wurzel ohne Basis");
        return err_text(500, "Repo-Basis nicht auflösbar");
    };
    if data.to_str().is_none() {
        tracing::error!("repo_activity: data dir ist kein UTF-8");
        return err_text(500, "Repo-Pfad nicht auflösbar");
    }
    let bin = repo_root.join("rust/target/release/dl-repostats");
    if !bin.exists() {
        tracing::error!(bin = %bin.display(), "repo_activity: dl-repostats fehlt");
        return err_text(
            503,
            "dl-repostats-Binary nicht gebaut (cargo build --release -p dl-repostats).",
        );
    }
    tracing::info!("AUDIT master-dashboard repo-activity refresh");

    let _run = REFRESH_LOCK.lock().await;
    let artifact = data.join("repo_activity.json");
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.current_dir(repo_root)
        .env("REPOSTATS_BASE", base)
        .env("REPOSTATS_OUT", &artifact)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            tracing::error!(%err, "dl-repostats Start fehlgeschlagen");
            return err_text(500, "Collector-Start fehlgeschlagen");
        }
    };
    let run = match tokio::time::timeout(
        Duration::from_secs(app.cfg().repo_refresh_timeout_secs),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(err)) => {
            tracing::error!(%err, "dl-repostats Lauf fehlgeschlagen");
            return err_text(500, "Collector-Lauf fehlgeschlagen");
        }
        Err(_) => {
            tracing::error!("dl-repostats Lauf über 60s, Prozess wird beendet");
            return err_text(504, "Collector-Lauf hat zu lange gedauert (>60s).");
        }
    };
    if !run.status.success() {
        let detail: String = String::from_utf8_lossy(&run.stderr)
            .chars()
            .take(500)
            .collect();
        tracing::error!(detail = %detail, "dl-repostats exit != 0");
        return err_text(500, "Collector-Lauf fehlgeschlagen");
    }

    // Aktualisiertes Artefakt zurückgeben.
    repo_activity(State(app), headers).await
}
