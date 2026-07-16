//! Zweitgehirn-Tab (:8766) — Feeder, Plan-Läufe und Second-Brain-Wiki.
//!
//! Alle Handler sind über [`DashboardApp::guard_full`] gegatet (Wiki-
//! und Report-Inhalte sind intern — TurnierOnly-Sessions bleiben draußen):
//! - `/api/brain/overview` — jüngster Wochenreport, letzte Feeder-Läufe,
//!   KI-Rechenschaft (7 Tage) und Top-Gründe je Quelle.
//! - `/api/brain/wiki` — `index.md`, `log.md` und die Liste aller `.md`-Seiten.
//! - `/api/brain/wiki/page?path=<rel>` — Inhalt einer einzelnen Wiki-Seite,
//!   streng auf das kanonisierte Wiki-Wurzelverzeichnis eingegrenzt.
//! - `/api/brain/plan` und `/api/brain/plan/runs` — aktueller Plan und Historie.
//! - `/api/brain/plan/item/:id` — Status und Kommentar eines Planpunkts.
//!
//! Es werden ausschliesslich Aggregate gelesen — keine einzelnen User-IDs.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, FromRow)]
struct PlanRun {
    id: i64,
    run_at: DateTime<Utc>,
    vorgeschlagen: i32,
    uebernommen: i32,
    verworfen: i32,
    verworfen_gruende: Option<Value>,
    modell: Option<String>,
    status: String,
    error: Option<String>,
    quellen_fehlend: Option<Vec<String>>,
    lage: Option<String>,
}

#[derive(Debug, FromRow)]
struct PlanItem {
    id: i64,
    run_id: i64,
    run_at: DateTime<Utc>,
    prioritaet: i16,
    bereich: String,
    titel: String,
    begruendung: String,
    aktion: String,
    beleg: String,
    beleg_art: String,
    belegt_gemessen: bool,
    status: String,
    kommentar: Option<String>,
    entschieden_am: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct RejectedPlanItem {
    id: i64,
    run_id: i64,
    prioritaet: i16,
    bereich: String,
    titel: String,
    begruendung: String,
    aktion: String,
    beleg: String,
    beleg_art: String,
    verworfen_grund: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePlanItem {
    status: String,
    kommentar: Option<String>,
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
    if let Err(resp) = app.guard_full(&headers).await {
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

pub async fn plan(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
        return resp;
    }
    let pool = app.pool();
    let run = match sqlx::query_as::<_, PlanRun>(
        "SELECT id, run_at, vorgeschlagen, uebernommen, verworfen,
                verworfen_gruende, modell, status, error, quellen_fehlend, lage
           FROM brain.plan_runs
          ORDER BY run_at DESC, id DESC
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    {
        Ok(run) => run,
        Err(err) => {
            tracing::error!(%err, "brain.plan_runs konnte nicht gelesen werden");
            return err_text(500, "brain plan unavailable");
        }
    };

    let Some(run) = run else {
        return ok_json(json!({
            "run": null,
            "items": [],
            "verworfen_items": [],
            "older_open_items": [],
            "stale": true,
        }));
    };
    let items = match load_plan_items(pool, run.id).await {
        Ok(items) => items,
        Err(err) => {
            tracing::error!(%err, "brain.plan_items konnte nicht gelesen werden");
            return err_text(500, "brain plan unavailable");
        }
    };
    let older_open_items = match load_older_open_items(pool, run.id).await {
        Ok(items) => items,
        Err(err) => {
            tracing::error!(%err, "ältere brain.plan_items konnten nicht gelesen werden");
            return err_text(500, "brain plan unavailable");
        }
    };
    let verworfen_items = match load_rejected_plan_items(pool, run.id).await {
        Ok(items) => items,
        Err(err) => {
            tracing::error!(%err, "verworfene brain.plan_items konnten nicht gelesen werden");
            return err_text(500, "brain plan unavailable");
        }
    };
    let stale = Utc::now() - run.run_at > Duration::days(8);

    ok_json(json!({
        "run": plan_run_json(&run),
        "items": items.into_iter().map(plan_item_json).collect::<Vec<_>>(),
        "verworfen_items": verworfen_items.into_iter().map(rejected_plan_item_json).collect::<Vec<_>>(),
        "older_open_items": older_open_items.into_iter().map(plan_item_json).collect::<Vec<_>>(),
        "stale": stale,
    }))
}

fn rejected_plan_item_json(item: RejectedPlanItem) -> Value {
    json!({
        "id": item.id,
        "run_id": item.run_id,
        "prioritaet": item.prioritaet,
        "bereich": item.bereich,
        "titel": item.titel,
        "begruendung": item.begruendung,
        "aktion": item.aktion,
        "beleg": item.beleg,
        "beleg_art": item.beleg_art,
        "verworfen_grund": item.verworfen_grund,
        "created_at": item.created_at.to_rfc3339(),
    })
}

fn plan_run_json(run: &PlanRun) -> Value {
    json!({
        "id": run.id,
        "run_at": run.run_at.to_rfc3339(),
        "vorgeschlagen": run.vorgeschlagen,
        "uebernommen": run.uebernommen,
        "verworfen": run.verworfen,
        "verworfen_gruende": run.verworfen_gruende,
        "modell": run.modell,
        "status": run.status,
        "error": run.error,
        "quellen_fehlend": run.quellen_fehlend,
        "lage": run.lage,
    })
}

fn plan_item_json(item: PlanItem) -> Value {
    json!({
        "id": item.id,
        "run_id": item.run_id,
        "run_at": item.run_at.to_rfc3339(),
        "prioritaet": item.prioritaet,
        "bereich": item.bereich,
        "titel": item.titel,
        "begruendung": item.begruendung,
        "aktion": item.aktion,
        "beleg": item.beleg,
        "beleg_art": item.beleg_art,
        "belegt_gemessen": item.belegt_gemessen,
        "status": item.status,
        "kommentar": item.kommentar,
        "entschieden_am": item.entschieden_am.map(|value| value.to_rfc3339()),
        "created_at": item.created_at.to_rfc3339(),
    })
}

async fn load_plan_items(pool: &PgPool, run_id: i64) -> Result<Vec<PlanItem>, sqlx::Error> {
    sqlx::query_as::<_, PlanItem>(
        "SELECT item.id, item.run_id, run.run_at, item.prioritaet, item.bereich,
                item.titel, item.begruendung, item.aktion, item.beleg, item.beleg_art,
                item.belegt_gemessen, item.status, item.kommentar, item.entschieden_am,
                item.created_at
           FROM brain.plan_items item
           JOIN brain.plan_runs run ON run.id = item.run_id
          WHERE item.run_id = $1
          ORDER BY item.prioritaet, item.created_at, item.id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}

async fn load_rejected_plan_items(
    pool: &PgPool,
    run_id: i64,
) -> Result<Vec<RejectedPlanItem>, sqlx::Error> {
    sqlx::query_as::<_, RejectedPlanItem>(
        "SELECT id, run_id, prioritaet, bereich, titel, begruendung, aktion,
                beleg, beleg_art, verworfen_grund, created_at
           FROM brain.plan_items_verworfen
          WHERE run_id = $1
          ORDER BY created_at, id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}

async fn load_older_open_items(
    pool: &PgPool,
    current_run_id: i64,
) -> Result<Vec<PlanItem>, sqlx::Error> {
    sqlx::query_as::<_, PlanItem>(
        "SELECT item.id, item.run_id, run.run_at, item.prioritaet, item.bereich,
                item.titel, item.begruendung, item.aktion, item.beleg, item.beleg_art,
                item.belegt_gemessen, item.status, item.kommentar, item.entschieden_am,
                item.created_at
           FROM brain.plan_items item
           JOIN brain.plan_runs run ON run.id = item.run_id
          WHERE item.run_id <> $1 AND item.status = 'offen'
          ORDER BY item.prioritaet, run.run_at, item.created_at, item.id",
    )
    .bind(current_run_id)
    .fetch_all(pool)
    .await
}

pub async fn plan_runs(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
        return resp;
    }
    match sqlx::query_as::<_, PlanRun>(
        "SELECT id, run_at, vorgeschlagen, uebernommen, verworfen,
                verworfen_gruende, modell, status, error, quellen_fehlend, lage
           FROM brain.plan_runs
          ORDER BY run_at DESC, id DESC",
    )
    .fetch_all(app.pool())
    .await
    {
        Ok(runs) => ok_json(json!({
            "runs": runs.iter().map(plan_run_json).collect::<Vec<_>>()
        })),
        Err(err) => {
            tracing::error!(%err, "Historie aus brain.plan_runs konnte nicht gelesen werden");
            err_text(500, "brain plan runs unavailable")
        }
    }
}

pub async fn update_plan_item(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    axum::Json(body): axum::Json<UpdatePlanItem>,
) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
        return resp;
    }
    if let Err(resp) = app.guard_mutate(&headers, true).await {
        return resp;
    }
    if !matches!(
        body.status.as_str(),
        "offen" | "angenommen" | "abgelehnt" | "erledigt"
    ) {
        return err_text(400, "invalid status");
    }

    match sqlx::query_as::<_, (String, Option<String>, DateTime<Utc>)>(
        "UPDATE brain.plan_items
            SET status = $2, kommentar = $3, entschieden_am = now()
          WHERE id = $1
          RETURNING status, kommentar, entschieden_am",
    )
    .bind(id)
    .bind(&body.status)
    .bind(&body.kommentar)
    .fetch_optional(app.pool())
    .await
    {
        Ok(Some((status, kommentar, entschieden_am))) => ok_json(json!({
            "id": id,
            "status": status,
            "kommentar": kommentar,
            "entschieden_am": entschieden_am.to_rfc3339(),
        })),
        Ok(None) => err_text(404, "plan item not found"),
        Err(err) => {
            tracing::error!(%err, "brain.plan_items konnte nicht aktualisiert werden");
            err_text(500, "brain plan item unavailable")
        }
    }
}

pub async fn wiki(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
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
    if let Err(resp) = app.guard_full(&headers).await {
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

    #[test]
    fn plan_panel_escaped_alle_llm_texte() {
        let html = include_str!("../../../../service/static/dashboard.html");
        for sink in [
            "${esc(run.lage ?? '')}",
            "${esc(item.titel)}",
            "${esc(item.begruendung)}",
            "${esc(item.aktion)}",
            "${esc(item.beleg)}",
            "${esc(item.kommentar ?? '')}",
        ] {
            assert!(html.contains(sink), "fehlender esc()-Sink: {sink}");
        }
        assert!(!html.contains("${item.titel}"));
    }

    #[test]
    fn plan_panel_escaped_verworfene_items() {
        let html = include_str!("../../../../service/static/dashboard.html");
        let rejected = html
            .split_once("function rejectedPlanItemHtml")
            .expect("Renderer für verworfene Items")
            .1
            .split_once("\n        function ")
            .expect("Ende des Renderers")
            .0;

        for sink in [
            "${esc(item.titel)}",
            "${esc(item.beleg)}",
            "${esc(item.beleg_art)}",
            "${esc(item.verworfen_grund)}",
        ] {
            assert!(rejected.contains(sink), "fehlender esc()-Sink: {sink}");
        }
    }

    #[test]
    fn plan_panel_verworfene_items_hat_finale_beschriftung() {
        let html = include_str!("../../../../service/static/dashboard.html");
        assert!(!html.contains("PLATZHALTER"));
        for label in [
            "Vom Beleg-Gate verworfene Vorschläge",
            "Vorgeschlagene Begründung:",
            "Vorgeschlagene Aktion:",
            "Angegebener Beleg:",
            "Verworfen, weil:",
        ] {
            assert!(
                html.contains(label),
                "fehlende finale Beschriftung: {label}"
            );
        }
    }
}

#[cfg(all(test, feature = "testing"))]
mod plan_tests {
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::authority::{MemberAccessInfo, MemberLookup};
    use crate::config::{AccessLevel, DashboardConfig};
    use crate::names::NameResolver;
    use crate::now_unix_f64;
    use crate::web::{router, SESSION_COOKIE};

    struct NoMemberLookup;

    #[async_trait::async_trait]
    impl MemberLookup for NoMemberLookup {
        async fn member_access(
            &self,
            _guild_id: Option<u64>,
            _user_id: u64,
        ) -> Option<MemberAccessInfo> {
            None
        }
    }

    struct NoNameResolver;

    #[async_trait::async_trait]
    impl NameResolver for NoNameResolver {
        async fn resolve(&self, _user_ids: &[u64]) -> HashMap<u64, String> {
            HashMap::new()
        }
    }

    async fn plan_app(
        with_session: bool,
    ) -> Result<
        (
            dl_central_db::TestDb,
            axum::Router,
            Option<(String, String)>,
        ),
        Box<dyn std::error::Error>,
    > {
        let db = dl_central_db::testing::test_pool().await?;
        sqlx::query("CREATE SCHEMA IF NOT EXISTS brain")
            .execute(db.pool())
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS brain.plan_runs (
                id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                run_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                period_start TIMESTAMPTZ NOT NULL,
                period_end TIMESTAMPTZ NOT NULL,
                plan_path TEXT,
                modell TEXT,
                lage TEXT,
                vorgeschlagen INTEGER NOT NULL DEFAULT 0,
                uebernommen INTEGER NOT NULL DEFAULT 0,
                verworfen INTEGER NOT NULL DEFAULT 0,
                verworfen_gruende JSONB,
                quellen_fehlend TEXT[],
                status TEXT NOT NULL,
                error TEXT,
                committed BOOLEAN NOT NULL DEFAULT false,
                pushed BOOLEAN NOT NULL DEFAULT false
            )",
        )
        .execute(db.pool())
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS brain.plan_items (
                id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                run_id BIGINT NOT NULL REFERENCES brain.plan_runs(id) ON DELETE CASCADE,
                prioritaet SMALLINT NOT NULL,
                bereich TEXT NOT NULL,
                titel TEXT NOT NULL,
                begruendung TEXT NOT NULL,
                aktion TEXT NOT NULL,
                beleg TEXT NOT NULL,
                beleg_art TEXT NOT NULL,
                belegt_gemessen BOOLEAN NOT NULL,
                fingerprint TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'offen',
                kommentar TEXT,
                entschieden_am TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            )",
        )
        .execute(db.pool())
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS brain.plan_items_verworfen (
                id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                run_id BIGINT NOT NULL REFERENCES brain.plan_runs(id) ON DELETE CASCADE,
                prioritaet SMALLINT NOT NULL,
                bereich TEXT NOT NULL,
                titel TEXT NOT NULL,
                begruendung TEXT NOT NULL,
                aktion TEXT NOT NULL,
                beleg TEXT NOT NULL,
                beleg_art TEXT NOT NULL,
                verworfen_grund TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            )",
        )
        .execute(db.pool())
        .await?;

        let session = if with_session {
            let session_id = "brain-plan-test-session".to_string();
            let csrf = "brain-plan-test-csrf".to_string();
            let now = now_unix_f64();
            dl_central_db::kv::set(
                db.pool(),
                "dl_dashboard_admin_session",
                &session_id,
                &json!({
                    "user_id": 42,
                    "username": "admin",
                    "display_name": "Admin",
                    "reason": "test",
                    "access_level": AccessLevel::Full.as_str(),
                    "csrf_token": csrf,
                    "created_at": now,
                    "last_seen_at": now,
                    "expires_at": now + 3600.0,
                })
                .to_string(),
            )
            .await?;
            Some((session_id, csrf))
        } else {
            None
        };

        let cfg = DashboardConfig::from_lookup(|key| match key {
            "DISCORD_OAUTH_CLIENT_ID" => Some("id".to_string()),
            "DISCORD_OAUTH_CLIENT_SECRET" => Some("secret".to_string()),
            _ => None,
        });
        let app = DashboardApp::new(
            cfg,
            db.pool().clone(),
            Arc::new(NoMemberLookup),
            Arc::new(NoNameResolver),
        )
        .await?;
        Ok((db, router(app), session))
    }

    fn auth_request(
        method: &str,
        uri: &str,
        session_id: &str,
        csrf: &str,
        body: Value,
    ) -> Result<Request<Body>, axum::http::Error> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::ORIGIN,
                "https://admin.deutsche-deadlock-community.de",
            )
            .header("X-CSRF-Token", csrf)
            .header(header::COOKIE, format!("{SESSION_COOKIE}={session_id}"))
            .body(Body::from(body.to_string()))
    }

    async fn insert_plan_item(pool: &PgPool) -> Result<i64, sqlx::Error> {
        let run_id: i64 = sqlx::query_scalar(
            "INSERT INTO brain.plan_runs(period_start, period_end, status)
             VALUES(now() - interval '7 days', now(), 'ok') RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        sqlx::query_scalar(
            "INSERT INTO brain.plan_items(
                run_id, prioritaet, bereich, titel, begruendung, aktion,
                beleg, beleg_art, belegt_gemessen, fingerprint
             ) VALUES($1, 1, 'discord', 'Titel', 'Grund', 'Aktion',
                      'Discord', 'digest', true, 'fingerprint')
             RETURNING id",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await
    }

    async fn insert_rejected_plan_item(pool: &PgPool, run_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO brain.plan_items_verworfen(
                run_id, prioritaet, bereich, titel, begruendung, aktion,
                beleg, beleg_art, verworfen_grund
             ) VALUES($1, 9, 'kaputt', '<Titel>', 'Grund', 'Aktion',
                      '## Erfunden', 'falsch', 'beleg_unaufloesbar')",
        )
        .bind(run_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "benötigt CENTRAL_TEST_DSN"]
    async fn plan_endpunkte_verlangen_vollzugriff() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, app, _) = plan_app(false).await?;
        for (method, uri) in [
            ("GET", "/api/brain/plan"),
            ("GET", "/api/brain/plan/runs"),
            ("POST", "/api/brain/plan/item/1"),
        ] {
            let body = if method == "POST" {
                r#"{"status":"offen"}"#
            } else {
                "{}"
            };
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body))?,
                )
                .await?;
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "benötigt CENTRAL_TEST_DSN"]
    async fn plan_get_ist_bei_leeren_tabellen_sauber() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, app, session) = plan_app(true).await?;
        let (session_id, csrf) = session.expect("session");
        let response = app
            .oneshot(auth_request(
                "GET",
                "/api/brain/plan",
                &session_id,
                &csrf,
                json!({}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
        assert!(body["run"].is_null());
        assert_eq!(body["items"], json!([]));
        assert_eq!(body["older_open_items"], json!([]));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "benötigt CENTRAL_TEST_DSN"]
    async fn ungueltiger_status_schreibt_nicht() -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session) = plan_app(true).await?;
        let item_id = insert_plan_item(db.pool()).await?;
        let (session_id, csrf) = session.expect("session");
        let response = app
            .oneshot(auth_request(
                "POST",
                &format!("/api/brain/plan/item/{item_id}"),
                &session_id,
                &csrf,
                json!({"status": "spaeter", "kommentar": "nein"}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let stored: (String, Option<String>, Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT status, kommentar, entschieden_am FROM brain.plan_items WHERE id = $1",
        )
        .bind(item_id)
        .fetch_one(db.pool())
        .await?;
        assert_eq!(stored, ("offen".to_string(), None, None));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "benötigt CENTRAL_TEST_DSN"]
    async fn post_setzt_entschieden_am() -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session) = plan_app(true).await?;
        let item_id = insert_plan_item(db.pool()).await?;
        let (session_id, csrf) = session.expect("session");
        let response = app
            .oneshot(auth_request(
                "POST",
                &format!("/api/brain/plan/item/{item_id}"),
                &session_id,
                &csrf,
                json!({"status": "angenommen", "kommentar": "Montag"}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let entschieden_am: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT entschieden_am FROM brain.plan_items WHERE id = $1")
                .bind(item_id)
                .fetch_one(db.pool())
                .await?;
        assert!(entschieden_am.is_some());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "benötigt CENTRAL_TEST_DSN"]
    async fn plan_get_trennt_verworfene_von_normalen_items(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session) = plan_app(true).await?;
        let item_id = insert_plan_item(db.pool()).await?;
        let run_id: i64 = sqlx::query_scalar("SELECT run_id FROM brain.plan_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await?;
        insert_rejected_plan_item(db.pool(), run_id).await?;
        let (session_id, csrf) = session.expect("session");

        let response = app
            .oneshot(auth_request(
                "GET",
                "/api/brain/plan",
                &session_id,
                &csrf,
                json!({}),
            )?)
            .await?;
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;

        assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["items"][0]["titel"], "Titel");
        assert_eq!(body["verworfen_items"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["verworfen_items"][0]["beleg"], "## Erfunden");
        assert_eq!(
            body["verworfen_items"][0]["verworfen_grund"],
            "beleg_unaufloesbar"
        );
        Ok(())
    }
}
