//! Zweitgehirn-Tab (:8766) — read-only Blick auf den Phase-2-Feeder.
//!
//! Drei GET-Handler, alle über [`DashboardApp::guard_read`] gegatet:
//! - `/api/brain/overview` — jüngster Wochenreport, letzte Feeder-Läufe,
//!   KI-Rechenschaft (7 Tage) und Top-Gründe je Quelle.
//! - `/api/brain/wiki` — `index.md`, `log.md` und die Liste aller `.md`-Seiten.
//! - `/api/brain/wiki/page?path=<rel>` — Inhalt einer einzelnen Wiki-Seite,
//!   streng auf das kanonisierte Wiki-Wurzelverzeichnis eingegrenzt.
//!
//! Es werden ausschliesslich Aggregate gelesen — keine einzelnen User-IDs.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::FromRow;
use sqlx::PgPool;

use crate::web::{err_text, ok_json, DashboardApp};

/// Standard-Wurzel des Second-Brain-Wikis (gleicher Default wie beim Feeder).
const DEFAULT_WIKI_ROOT: &str = "/home/naniadm/Documents/Deadlock-2nd-Brain";

fn wiki_root() -> PathBuf {
    std::env::var("DL_BRAIN_WIKI_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_WIKI_ROOT))
}

/// Ein Lauf-Protokoll aus `brain.feeder_runs`.
#[derive(Debug, FromRow)]
struct FeederRun {
    run_at: DateTime<Utc>,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    digest_path: Option<String>,
    gesehen: Option<i32>,
    relevant: Option<i32>,
    kategorien: Option<Vec<String>>,
    status: String,
    error: Option<String>,
    committed: bool,
    pushed: bool,
}

#[derive(Debug, FromRow)]
struct ReportRow {
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    report_text: String,
    kpis: Value,
}

#[derive(Debug, FromRow, Serialize)]
struct LedgerAgg {
    source: String,
    decision: String,
    count: i64,
}

#[derive(Debug, FromRow, Serialize)]
struct ReasonAgg {
    source: String,
    reason: String,
    count: i64,
}

/// `true`, wenn die Tabelle/das Schema `brain.feeder_runs` noch fehlt
/// (Migration nicht eingespielt) — dann kein 500, sondern leeres Ergebnis.
fn is_missing_schema(err: &sqlx::Error) -> bool {
    matches!(
        err,
        sqlx::Error::Database(db)
            if matches!(db.code().as_deref(), Some("42P01") | Some("3F000"))
    )
}

pub async fn overview(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let pool = app.pool();
    let now = Utc::now();
    let start = now - Duration::days(7);

    let report = match sqlx::query_as::<_, ReportRow>(
        "SELECT period_start, period_end, report_text, kpis
           FROM bot.brain_reports
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(err) => {
            tracing::error!(%err, "bot.brain_reports konnte nicht gelesen werden");
            return err_text(500, "brain overview unavailable");
        }
    };

    let (feeder_runs, schema_missing) = match sqlx::query_as::<_, FeederRun>(
        "SELECT run_at, period_start, period_end, digest_path, gesehen, relevant,
                kategorien, status, error, committed, pushed
           FROM brain.feeder_runs
          ORDER BY run_at DESC
          LIMIT 10",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => (rows, false),
        Err(err) if is_missing_schema(&err) => (Vec::new(), true),
        Err(err) => {
            tracing::error!(%err, "brain.feeder_runs konnte nicht gelesen werden");
            return err_text(500, "brain overview unavailable");
        }
    };

    // Überfällig, wenn die Tabelle existiert, aber nie oder seit >8 Tagen kein
    // Lauf protokolliert wurde. Fehlt das Schema, reicht der Migrations-Hinweis.
    let feeder_stale = !schema_missing
        && feeder_runs
            .first()
            .is_none_or(|latest| now - latest.run_at > Duration::days(8));

    let ledger = match load_ledger(pool, start, now).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(%err, "bot.ai_decision_ledger konnte nicht aggregiert werden");
            return err_text(500, "brain overview unavailable");
        }
    };
    let top_reasons = match load_reasons(pool, start, now).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(%err, "bot.ai_decision_ledger Gründe unlesbar");
            return err_text(500, "brain overview unavailable");
        }
    };

    let report = report.map(|r| {
        json!({
            "period_start": r.period_start.to_rfc3339(),
            "period_end": r.period_end.to_rfc3339(),
            "report_text": r.report_text,
            "kpis": r.kpis,
        })
    });
    let feeder_runs = feeder_runs
        .into_iter()
        .map(|run| {
            json!({
                "run_at": run.run_at.to_rfc3339(),
                "period_start": run.period_start.to_rfc3339(),
                "period_end": run.period_end.to_rfc3339(),
                "digest_path": run.digest_path,
                "gesehen": run.gesehen,
                "relevant": run.relevant,
                "kategorien": run.kategorien,
                "status": run.status,
                "error": run.error,
                "committed": run.committed,
                "pushed": run.pushed,
            })
        })
        .collect::<Vec<_>>();

    ok_json(json!({
        "report": report,
        "feeder_runs": feeder_runs,
        "ledger_week": ledger,
        "top_reasons": top_reasons,
        "schema_missing": schema_missing,
        "feeder_stale": feeder_stale,
    }))
}

async fn load_ledger(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<LedgerAgg>, sqlx::Error> {
    sqlx::query_as::<_, LedgerAgg>(
        "SELECT source, decision, COUNT(*)::BIGINT AS count
           FROM bot.ai_decision_ledger
          WHERE decided_at >= $1 AND decided_at < $2
          GROUP BY source, decision
          ORDER BY source, decision",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
}

async fn load_reasons(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<ReasonAgg>, sqlx::Error> {
    sqlx::query_as::<_, ReasonAgg>(
        "SELECT source, reason, count FROM (
           SELECT source, reason, COUNT(*)::BIGINT AS count,
                  ROW_NUMBER() OVER (PARTITION BY source ORDER BY COUNT(*) DESC, reason) AS rn
             FROM bot.ai_decision_ledger
            WHERE decided_at >= $1 AND decided_at < $2
            GROUP BY source, reason
         ) ranked
         WHERE rn <= 5
         ORDER BY source, count DESC, reason",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
}

pub async fn wiki(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let root = wiki_root();
    // ponytail: kleines Markdown-Repo, synchroner Walk genügt; spawn_blocking erst bei >10k Dateien.
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap_or_default();
    let pages = match collect_md_pages(&root) {
        Ok(pages) => pages,
        Err(err) => {
            tracing::warn!(%err, "Wiki-Seiten konnten nicht gelistet werden");
            Vec::new()
        }
    };
    ok_json(json!({
        "index": read("index.md"),
        "log": read("log.md"),
        "pages": pages,
    }))
}

/// Sammelt alle `.md`-Dateien unter `root` (ausser `.git`) als relative Pfade.
fn collect_md_pages(root: &Path) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

pub async fn wiki_page(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let Some(rel) = params.get("path") else {
        return err_text(400, "path required");
    };
    let root = wiki_root();
    let canon = match resolve_wiki_page(&root, rel) {
        Ok(path) => path,
        Err(_) => return err_text(400, "invalid path"),
    };
    match std::fs::read_to_string(&canon) {
        Ok(content) => ok_json(json!({ "path": rel, "content": content })),
        Err(_) => err_text(404, "page not found"),
    }
}

/// Validiert einen relativen Wiki-Pfad und gibt den kanonisierten,
/// wurzel-gebundenen Zielpfad zurück. Lehnt absolute Pfade, `..`-Segmente,
/// Nicht-`.md`-Dateien und jeden Symlink-/Canonicalize-Ausbruch ab.
fn resolve_wiki_page(root: &Path, rel: &str) -> Result<PathBuf, ()> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Err(());
    }
    let rel_path = Path::new(rel);
    if rel_path.extension().and_then(|e| e.to_str()) != Some("md") {
        return Err(());
    }
    // Nur echte Unterpfade: keine Wurzel-, Prefix- oder Parent-Segmente.
    if rel_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(());
    }
    // Konsistenz mit collect_md_pages: das .git-Verzeichnis ist tabu.
    if rel_path.components().any(|c| c.as_os_str() == ".git") {
        return Err(());
    }
    let canon_root = root.canonicalize().map_err(|_| ())?;
    let canon = canon_root.join(rel_path).canonicalize().map_err(|_| ())?;
    if !canon.starts_with(&canon_root) {
        return Err(());
    }
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_wiki() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("Projekte")).expect("subdir");
        fs::write(dir.path().join("index.md"), "# Index").expect("index");
        fs::write(dir.path().join("Projekte/foo.md"), "# Foo").expect("foo");
        dir
    }

    #[test]
    fn accepts_valid_relative_md_page() {
        let dir = tmp_wiki();
        let resolved = resolve_wiki_page(dir.path(), "Projekte/foo.md").expect("gültiger Pfad");
        assert!(resolved.ends_with("Projekte/foo.md"));
        assert!(resolved.starts_with(dir.path().canonicalize().expect("canon root")));
    }

    #[test]
    fn rejects_parent_traversal() {
        let dir = tmp_wiki();
        assert!(resolve_wiki_page(dir.path(), "../secret.md").is_err());
        assert!(resolve_wiki_page(dir.path(), "Projekte/../../secret.md").is_err());
    }

    #[test]
    fn rejects_absolute_path() {
        let dir = tmp_wiki();
        assert!(resolve_wiki_page(dir.path(), "/etc/passwd.md").is_err());
        assert!(resolve_wiki_page(dir.path(), "/etc/passwd").is_err());
    }

    #[test]
    fn rejects_non_md_extension() {
        let dir = tmp_wiki();
        fs::write(dir.path().join("secret.txt"), "x").expect("txt");
        assert!(resolve_wiki_page(dir.path(), "secret.txt").is_err());
        assert!(resolve_wiki_page(dir.path(), "index").is_err());
    }

    #[test]
    fn rejects_empty_path() {
        let dir = tmp_wiki();
        assert!(resolve_wiki_page(dir.path(), "").is_err());
        assert!(resolve_wiki_page(dir.path(), "   ").is_err());
    }

    #[test]
    fn rejects_symlink_escape() {
        let dir = tmp_wiki();
        let outside = tempfile::tempdir().expect("outside");
        fs::write(outside.path().join("secret.md"), "leak").expect("secret");
        let link = dir.path().join("escape.md");
        // Symlink auf eine Datei ausserhalb der Wurzel: canonicalize löst ihn
        // auf, der starts_with-Check muss ihn abweisen.
        if std::os::unix::fs::symlink(outside.path().join("secret.md"), &link).is_ok() {
            assert!(resolve_wiki_page(dir.path(), "escape.md").is_err());
        }
    }

    #[test]
    fn rejects_git_directory() {
        let dir = tmp_wiki();
        fs::create_dir(dir.path().join(".git")).expect("git dir");
        fs::write(dir.path().join(".git/config.md"), "leak").expect("git file");
        assert!(resolve_wiki_page(dir.path(), ".git/config.md").is_err());
    }

    #[test]
    fn collects_md_pages_without_git() {
        let dir = tmp_wiki();
        fs::create_dir(dir.path().join(".git")).expect("git dir");
        fs::write(dir.path().join(".git/config.md"), "x").expect("git file");
        let pages = collect_md_pages(dir.path()).expect("walk");
        assert!(pages.contains(&"index.md".to_string()));
        assert!(pages.contains(&"Projekte/foo.md".to_string()));
        assert!(!pages.iter().any(|p| p.starts_with(".git")));
    }
}
