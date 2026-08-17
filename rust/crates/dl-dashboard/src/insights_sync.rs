//! Holt Discord-Server-Insights aus derselben Portal-API wie die CSV-Buttons
//! und spielt sie über [`crate::insights::import_csv_text`] ein.
//!
//! Kein HTML-Scraper: `GET /api/v9/guilds/{id}/analytics/...`.
//! Auth ist ein User-Token mit Recht „Server-Einblicke ansehen“, kein Bot-Token.
//! Env: `DISCORD_INSIGHTS_USER_TOKEN`.

use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use serde_json::Value;
use sqlx::PgPool;

use crate::insights::import_csv_text;

pub const DEFAULT_GUILD_ID: i64 = 1_289_721_245_281_292_288;
const API_BASE: &str = "https://discord.com/api/v9";
const TOKEN_ENV: &str = "DISCORD_INSIGHTS_USER_TOKEN";
const GUILD_ENV: &str = "DISCORD_INSIGHTS_GUILD_ID";
const ARCHIVE_ENV: &str = "INSIGHTS_ARCHIVE_DIR";

/// interval=1 täglich, 2 wöchentlich, 3 monatlich (Portal-UI).
struct Endpoint {
    path: &'static str,
    interval: u8,
    file_stem: &'static str,
}

const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        path: "engagement/base",
        interval: 2,
        file_stem: "guild-communicators",
    },
    Endpoint {
        path: "growth-activation/activation",
        interval: 2,
        file_stem: "guild-activation",
    },
    Endpoint {
        path: "growth-activation/retention",
        interval: 2,
        file_stem: "guild-retention",
    },
    Endpoint {
        path: "growth-activation/joins-by-source",
        interval: 2,
        file_stem: "guild-joins-by-source",
    },
    Endpoint {
        path: "growth-activation/leavers",
        interval: 2,
        file_stem: "guild-leavers",
    },
    Endpoint {
        path: "growth-activation/membership",
        interval: 1,
        file_stem: "guild-total-membership",
    },
    Endpoint {
        path: "growth-activation/joins-by-invite-link",
        interval: 3,
        file_stem: "guild-top-invites",
    },
    Endpoint {
        path: "growth-activation/joins-by-referrer",
        interval: 3,
        file_stem: "guild-referrers",
    },
    Endpoint {
        path: "engagement/text-channels",
        interval: 3,
        file_stem: "guild-text-channels",
    },
    Endpoint {
        path: "engagement/voice-channels",
        interval: 3,
        file_stem: "guild-voice-channels",
    },
];

#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub token: String,
    pub guild_id: i64,
    pub archive_dir: PathBuf,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub files: usize,
    pub rows: usize,
    pub skipped: Vec<String>,
    pub archive_dir: PathBuf,
}

impl SyncConfig {
    pub fn from_env() -> Result<Self, String> {
        let token = std::env::var(TOKEN_ENV)
            .map(|s| s.trim().to_string())
            .map_err(|_| format!("{TOKEN_ENV} fehlt"))?;
        if token.is_empty() {
            return Err(format!("{TOKEN_ENV} ist leer"));
        }
        let guild_id = std::env::var(GUILD_ENV)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_GUILD_ID);
        let archive_dir = std::env::var(ARCHIVE_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_archive_dir());
        Ok(Self {
            token,
            guild_id,
            archive_dir,
        })
    }
}

fn default_archive_dir() -> PathBuf {
    let today = Utc::now().date_naive();
    PathBuf::from(format!(
        "/home/nathanael/repos/Deadlock-Bots/docs/insights/discord/{today}"
    ))
}

pub async fn import_csv_dir(
    pool: &PgPool,
    guild_id: i64,
    dir: &Path,
) -> Result<SyncReport, String> {
    let mut report = SyncReport {
        archive_dir: dir.to_path_buf(),
        ..SyncReport::default()
    };
    let entries = std::fs::read_dir(dir).map_err(|err| err.to_string())?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("csv") {
            files.push(path);
        }
    }
    files.sort();
    for path in files {
        let text = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("upload.csv");
        match import_csv_text(pool, guild_id, &text).await {
            Ok((kind, rows)) => {
                tracing::info!(file = name, kind, rows, "Insights-CSV importiert");
                report.files += 1;
                report.rows += rows;
            }
            Err(err) => report.skipped.push(format!("{name}: {err}")),
        }
    }
    Ok(report)
}

pub async fn sync_insights(pool: &PgPool, cfg: &SyncConfig) -> Result<SyncReport, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| err.to_string())?;
    let end = Utc::now();
    let start = end - Duration::days(119);
    let start_s = start.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let end_s = end.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    std::fs::create_dir_all(&cfg.archive_dir).map_err(|err| err.to_string())?;

    let mut report = SyncReport {
        archive_dir: cfg.archive_dir.clone(),
        ..SyncReport::default()
    };
    for endpoint in ENDPOINTS {
        match fetch_csv(&client, cfg, endpoint, &start_s, &end_s).await {
            Ok(None) => report.skipped.push(format!("{}: leer", endpoint.file_stem)),
            Ok(Some(csv)) => {
                let path = cfg.archive_dir.join(format!("{}.csv", endpoint.file_stem));
                std::fs::write(&path, &csv).map_err(|err| err.to_string())?;
                match import_csv_text(pool, cfg.guild_id, &csv).await {
                    Ok((kind, rows)) => {
                        tracing::info!(
                            file = endpoint.file_stem,
                            kind,
                            rows,
                            "Insights-CSV importiert"
                        );
                        report.files += 1;
                        report.rows += rows;
                    }
                    Err(err) => {
                        report
                            .skipped
                            .push(format!("{}: Import {err}", endpoint.file_stem));
                    }
                }
            }
            Err(err) => report
                .skipped
                .push(format!("{}: {err}", endpoint.file_stem)),
        }
    }
    Ok(report)
}

async fn fetch_csv(
    client: &reqwest::Client,
    cfg: &SyncConfig,
    endpoint: &Endpoint,
    start: &str,
    end: &str,
) -> Result<Option<String>, String> {
    let url = format!(
        "{API_BASE}/guilds/{}/analytics/{}?start={}&end={}&interval={}",
        cfg.guild_id, endpoint.path, start, end, endpoint.interval
    );
    let response = client
        .get(&url)
        .header("Authorization", &cfg.token)
        .header("User-Agent", "DeadlockInsightsSync/1.0")
        .send()
        .await
        .map_err(|err| err.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|err| err.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let json: Value = serde_json::from_str(&text).map_err(|err| err.to_string())?;
    Ok(json_to_csv(&json))
}

fn json_to_csv(value: &Value) -> Option<String> {
    let rows = json_rows(value)?;
    if rows.is_empty() {
        return None;
    }
    let mut headers = Vec::new();
    for row in &rows {
        if let Value::Object(map) = row {
            for key in map.keys() {
                if !headers.iter().any(|h| h == key) {
                    headers.push(key.clone());
                }
            }
        }
    }
    if headers.is_empty() {
        return None;
    }
    let mut out = csv_row(&headers);
    for row in &rows {
        let Value::Object(map) = row else {
            continue;
        };
        let cells = headers
            .iter()
            .map(|key| json_cell(map.get(key).unwrap_or(&Value::Null)))
            .collect::<Vec<_>>();
        out.push('\n');
        out.push_str(&csv_row(&cells));
    }
    Some(out)
}

fn json_rows(value: &Value) -> Option<Vec<Value>> {
    match value {
        Value::Array(items) => Some(items.clone()),
        Value::Object(map) => map.values().find_map(|inner| inner.as_array().cloned()),
        _ => None,
    }
}

fn json_cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn csv_row(cells: &[String]) -> String {
    cells
        .iter()
        .map(|cell| {
            if cell.contains(',') || cell.contains('"') || cell.contains('\n') {
                format!("\"{}\"", cell.replace('"', "\"\""))
            } else {
                cell.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub fn archive_readme(dir: &Path) -> String {
    format!(
        "# Discord Insights {}\n\nAutomatischer Wochenexport aus der Portal-API.\n",
        dir.file_name().and_then(|s| s.to_str()).unwrap_or("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_array_wird_zu_csv_mit_stabilen_headern() {
        let csv = json_to_csv(&json!([
            {"day_pt": "2026-08-01T00:00:00+00:00", "visitors": 517, "pct_communicated": 21.08},
            {"day_pt": "2026-08-08T00:00:00+00:00", "visitors": 468, "pct_communicated": 23.72}
        ]))
        .expect("csv");
        let header = csv.lines().next().expect("header");
        assert!(header.contains("day_pt"));
        assert!(header.contains("visitors"));
        assert!(header.contains("pct_communicated"));
        assert!(csv.contains("2026-08-01"));
        assert!(csv.contains("517"));
    }

    #[test]
    fn json_wrapper_mit_data_feld_wird_entpackt() {
        let csv =
            json_to_csv(&json!({"data": [{"day_pt": "2026-08-01", "leavers": 9}]})).expect("csv");
        assert!(csv.contains("leavers"));
        assert!(csv.contains("9"));
    }
}
