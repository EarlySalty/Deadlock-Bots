//! Bug-/Ticket-Reports — Daten-Layer (Port von `service/issue_reports.py` +
//! der Domänen-Helfer aus `cogs/bug_reporter.py`).
//!
//! Reiner Store über `issue_reports` plus die Kategorie-Inferenz. Die
//! Discord-Seite (Modal/Button/Slash, Ticket-Channel, Panel) und die
//! Codex-Auto-Antwort folgen in eigenen Pässen; letztere ist an die lokale
//! Codex-CLI + Bot-interne Auto-Aktionen (Reload/Restart) gebunden und nicht
//! 1:1 nach dl-bot portierbar.

use dl_db::{Db, DbError};
use rusqlite::{params, OptionalExtension};

/// Auswahl im `/ticket`-Slash (Label, interner Wert) — wie `CATEGORY_CHOICES`.
pub const CATEGORY_CHOICES: [(&str, &str); 7] = [
    ("Steam-Verifizierung", "steam_verification"),
    ("Beta-Invite / Zugang", "beta_invite"),
    ("Bot-Command/Feature", "bot_command"),
    ("Build-Publishing", "build_publishing"),
    ("AI-Features (FAQ/Onboarding)", "ai_features"),
    ("User-Management/Beschwerde", "user_management"),
    ("Sonstiges", "other"),
];

/// Kategorien, die Codex automatisch bearbeiten darf (`CODEX_ALLOWED_CATEGORIES`).
pub const CODEX_ALLOWED_CATEGORIES: [&str; 7] = [
    "steam_verification",
    "beta_invite",
    "bot_command",
    "build_publishing",
    "ai_features",
    "user_management",
    "other",
];

const VALID_STATUSES: [&str; 5] = ["pending", "processing", "answered", "failed", "handoff"];
const DEFAULT_STATUS: &str = "pending";

/// Unbekannte/leere Status fallen auf `pending` zurück (Python `_normalize_status`).
fn normalize_status(status: &str) -> String {
    if VALID_STATUSES.contains(&status) {
        status.to_string()
    } else {
        DEFAULT_STATUS.to_string()
    }
}

/// Darf Codex diese Kategorie automatisch bearbeiten? (`_category_allows_codex`).
pub fn category_allows_codex(category: &str) -> bool {
    CODEX_ALLOWED_CATEGORIES.contains(&category.to_lowercase().as_str())
}

/// Heuristische Kategorie aus dem Freitext (Python `_infer_category`).
///
/// Bewusst 1:1 inkl. Eigenheiten: die erste Regel matcht u. a. die Teilstrings
/// `dm`/`pn`, weshalb z. B. „admin" (enthält `dm`) als `user_management` zählt.
pub fn infer_category(text: &str) -> &'static str {
    let low = text.to_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| low.contains(k));
    if has(&[
        "scam",
        "scammer",
        "scamer",
        "betrug",
        "fake",
        "phish",
        "phishing",
        "meldung",
        "beschwerde",
        "report",
        "gemeldet",
        "angeschrieben",
        "dm",
        "pn",
        "privatnachricht",
    ]) {
        return "user_management";
    }
    if has(&["steam", "verifiz", "verify"]) {
        return "steam_verification";
    }
    if has(&["beta", "invite", "zugang"]) {
        return "beta_invite";
    }
    if has(&["build", "publish", "mirror"]) {
        return "build_publishing";
    }
    if has(&["faq", "ai", "onboard", "ki"]) {
        return "ai_features";
    }
    if has(&["command", "befehl", "bot", "error", "traceback"]) {
        return "bot_command";
    }
    if has(&["ban", "kick", "mute", "user", "mod", "admin", "rolle"]) {
        return "user_management";
    }
    "other"
}

/// Felder eines neuen Reports (Python `create_report`-Argumente).
#[derive(Debug, Clone, Default)]
pub struct NewReport {
    pub user_id: Option<u64>,
    pub guild_id: Option<u64>,
    pub channel_id: Option<u64>,
    pub message_id: Option<u64>,
    pub category: Option<String>,
    pub title: Option<String>,
    pub description: String,
    pub status: String,
}

/// Ein vollständiger Report aus `issue_reports`.
#[derive(Debug, Clone)]
pub struct Report {
    pub id: i64,
    pub user_id: Option<i64>,
    pub guild_id: Option<i64>,
    pub channel_id: Option<i64>,
    pub message_id: Option<i64>,
    pub category: Option<String>,
    pub title: Option<String>,
    pub description: String,
    pub status: String,
    pub ai_response: Option<String>,
    pub ai_model: Option<String>,
    pub ai_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub answered_at: Option<i64>,
}

#[derive(Clone)]
pub struct BugReports {
    db: Db,
}

impl BugReports {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Legt die Tabelle an (idempotent), Schema wie `service/db.py:1056`.
    pub async fn ensure_schema(&self) -> Result<(), DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS issue_reports(
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       user_id INTEGER,
                       guild_id INTEGER,
                       channel_id INTEGER,
                       message_id INTEGER,
                       category TEXT,
                       title TEXT,
                       description TEXT NOT NULL,
                       status TEXT NOT NULL DEFAULT 'pending',
                       ai_response TEXT,
                       ai_model TEXT,
                       ai_error TEXT,
                       created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                       updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                       answered_at INTEGER
                     );",
                )?;
                Ok(())
            })
            .await
    }

    /// Erzeugt einen Report; gibt die neue ID zurück (`None` bei Fehler, wie
    /// Pythons `0`-Rückgabe, nur typsicher).
    pub async fn create_report(&self, report: NewReport) -> Option<i64> {
        let status = normalize_status(&report.status);
        let now = chrono::Utc::now().timestamp();
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO issue_reports(
                       user_id, guild_id, channel_id, message_id, category,
                       title, description, status, created_at, updated_at)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                    params![
                        report.user_id,
                        report.guild_id,
                        report.channel_id,
                        report.message_id,
                        report.category,
                        report.title,
                        report.description,
                        status,
                        now,
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .ok()
    }

    /// Setzt Status + AI-Ergebnis. `answered_at` wird gesetzt, sobald der Status
    /// nicht mehr `pending` ist (Python `update_status`).
    pub async fn update_status(
        &self,
        report_id: i64,
        status: &str,
        ai_response: Option<String>,
        ai_model: Option<String>,
        ai_error: Option<String>,
    ) {
        let status = normalize_status(status);
        let now = chrono::Utc::now().timestamp();
        let answered_at = (status != DEFAULT_STATUS).then_some(now);
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE issue_reports
                        SET status=?1, ai_response=?2, ai_model=?3, ai_error=?4,
                            updated_at=?5, answered_at=?6
                      WHERE id=?7",
                    params![
                        status,
                        ai_response,
                        ai_model,
                        ai_error,
                        now,
                        answered_at,
                        report_id
                    ],
                )
                .map(|_| ())
            })
            .await;
    }

    /// Liefert einen Report oder `None` (Python `fetch_report`).
    pub async fn fetch_report(&self, report_id: i64) -> Option<Report> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT id, user_id, guild_id, channel_id, message_id, category,
                            title, description, status, ai_response, ai_model, ai_error,
                            created_at, updated_at, answered_at
                       FROM issue_reports WHERE id=?1",
                    params![report_id],
                    |r| {
                        Ok(Report {
                            id: r.get(0)?,
                            user_id: r.get(1)?,
                            guild_id: r.get(2)?,
                            channel_id: r.get(3)?,
                            message_id: r.get(4)?,
                            category: r.get(5)?,
                            title: r.get(6)?,
                            description: r.get(7)?,
                            status: r.get(8)?,
                            ai_response: r.get(9)?,
                            ai_model: r.get(10)?,
                            ai_error: r.get(11)?,
                            created_at: r.get(12)?,
                            updated_at: r.get(13)?,
                            answered_at: r.get(14)?,
                        })
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mk() -> (tempfile::TempDir, BugReports) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("b.sqlite3")).expect("db");
        let store = BugReports::new(db);
        store.ensure_schema().await.expect("schema");
        (dir, store)
    }

    #[test]
    fn infer_deckt_die_zweige_ab() {
        assert_eq!(infer_category("Steam Verifizierung klappt nicht"), "steam_verification");
        assert_eq!(infer_category("brauche einen beta invite"), "beta_invite");
        assert_eq!(infer_category("build publishing kaputt"), "build_publishing");
        assert_eq!(infer_category("FAQ onboarding bug"), "ai_features");
        assert_eq!(infer_category("command wirft error"), "bot_command");
        assert_eq!(infer_category("komplett harmloser text"), "other");
        // Beschwerde-Schlüsselwörter haben Vorrang.
        assert_eq!(infer_category("ein scammer hat mich angeschrieben"), "user_management");
        // Eigenheit: „admin" enthält „dm" → erste Regel greift (faithful zu Python).
        assert_eq!(infer_category("der admin soll das fixen"), "user_management");
    }

    #[test]
    fn codex_erlaubte_kategorien() {
        assert!(category_allows_codex("steam_verification"));
        assert!(category_allows_codex("OTHER")); // case-insensitive
        assert!(!category_allows_codex("nicht_existent"));
    }

    #[tokio::test]
    async fn create_und_fetch_roundtrip() {
        let (_d, store) = mk().await;
        let id = store
            .create_report(NewReport {
                user_id: Some(42),
                guild_id: Some(1),
                channel_id: Some(2),
                message_id: None,
                category: Some("bot_command".into()),
                title: Some("Titel".into()),
                description: "Beschreibung".into(),
                status: "processing".into(),
            })
            .await
            .expect("create");
        let report = store.fetch_report(id).await.expect("fetch");
        assert_eq!(report.user_id, Some(42));
        assert_eq!(report.category.as_deref(), Some("bot_command"));
        assert_eq!(report.description, "Beschreibung");
        assert_eq!(report.status, "processing");
        assert_eq!(report.answered_at, None);
        assert!(store.fetch_report(99999).await.is_none());
    }

    #[tokio::test]
    async fn unbekannter_status_faellt_auf_pending() {
        let (_d, store) = mk().await;
        let id = store
            .create_report(NewReport {
                description: "x".into(),
                status: "voellig_ungueltig".into(),
                ..Default::default()
            })
            .await
            .expect("create");
        let report = store.fetch_report(id).await.expect("fetch");
        assert_eq!(report.status, "pending");
    }

    #[tokio::test]
    async fn update_setzt_answered_at_nur_bei_nicht_pending() {
        let (_d, store) = mk().await;
        let id = store
            .create_report(NewReport {
                description: "x".into(),
                ..Default::default()
            })
            .await
            .expect("create");

        store
            .update_status(id, "answered", Some("Antwort".into()), Some("gpt".into()), None)
            .await;
        let report = store.fetch_report(id).await.expect("fetch");
        assert_eq!(report.status, "answered");
        assert_eq!(report.ai_response.as_deref(), Some("Antwort"));
        assert!(report.answered_at.is_some());

        // Zurück auf pending → answered_at wieder NULL.
        store.update_status(id, "pending", None, None, None).await;
        let report = store.fetch_report(id).await.expect("fetch");
        assert!(report.answered_at.is_none());
    }
}
