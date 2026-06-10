//! Snapshot-Refresh gegen api.deadlock-api.com — Logik wie refresh_once()
//! im Python-Original inkl. der toleranten Stats-Normalisierung.

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::settings::{read_settings, BUCKETS, SNAPSHOT_RETENTION_PER_BUCKET};
use crate::util::{coerce_float, coerce_int, now_ts, parse_unix_or_iso, py_round4};
use crate::SharedApp;

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    /// Upstream-Probleme (Timeout, 5xx, Netzfehler) — nächster Tick darf es
    /// erneut versuchen; manueller Refresh antwortet 502.
    #[error("{0}")]
    Retryable(String),
    /// Alles andere (4xx, kaputtes Payload, DB-Fehler) — wird geloggt bzw. 500.
    #[error("{0}")]
    Fatal(String),
}

#[derive(Debug, Clone, Copy)]
struct HeroStats {
    hero_id: i64,
    matches: i64,
    wins: i64,
    losses: i64,
    winrate: f64,
}

async fn fetch_json(
    app: &SharedApp,
    url: &str,
    params: &[(&str, String)],
) -> Result<Value, RefreshError> {
    let response = app.api.get(url).query(params).send().await.map_err(|e| {
        if e.is_timeout() {
            RefreshError::Retryable("upstream request timed out".to_string())
        } else {
            RefreshError::Retryable(format!("upstream request failed: {e}"))
        }
    })?;
    let status = response.status();
    if status.as_u16() >= 500 {
        let body = response.text().await.unwrap_or_default();
        return Err(RefreshError::Retryable(format!(
            "upstream returned {}: {}",
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        )));
    }
    if status.as_u16() >= 400 {
        let body = response.text().await.unwrap_or_default();
        return Err(RefreshError::Fatal(format!(
            "upstream returned {}: {}",
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        )));
    }
    response
        .json::<Value>()
        .await
        .map_err(|e| RefreshError::Retryable(format!("upstream request failed: {e}")))
}

/// Neuesten Patch per pub_date/pubDate bestimmen.
async fn fetch_current_patch(app: &SharedApp) -> Result<(String, i64), RefreshError> {
    let payload = fetch_json(app, &format!("{}/patches", app.api_base), &[]).await?;
    let candidates = match &payload {
        Value::Object(obj) => obj.get("patches").cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    let Value::Array(items) = candidates else {
        return Err(RefreshError::Fatal("unexpected patch payload".to_string()));
    };

    let mut latest_unix = -1i64;
    let mut latest_item: Option<&serde_json::Map<String, Value>> = None;
    let items_refs: Vec<&serde_json::Map<String, Value>> =
        items.iter().filter_map(Value::as_object).collect();
    for item in &items_refs {
        let pub_date = item.get("pub_date").or_else(|| item.get("pubDate"));
        let Some(patch_unix) = parse_unix_or_iso(pub_date) else {
            continue;
        };
        if patch_unix > latest_unix {
            latest_unix = patch_unix;
            latest_item = Some(item);
        }
    }
    let Some(item) = latest_item else {
        return Err(RefreshError::Fatal(
            "could not determine current patch".to_string(),
        ));
    };
    let patch_id = ["pub_date", "pubDate", "patch_id", "id"]
        .iter()
        .find_map(|k| item.get(*k))
        .map(value_to_display_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| latest_unix.to_string());
    Ok((patch_id, latest_unix))
}

fn value_to_display_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Tolerante Normalisierung der hero-stats-Antwort (viele Alias-Felder).
fn normalize_hero_stats(payload: &Value) -> Vec<HeroStats> {
    let raw_rows: Vec<&Value> = match payload {
        Value::Array(items) => items.iter().collect(),
        Value::Object(obj) => [
            "heroes",
            "hero_stats",
            "heroStats",
            "results",
            "data",
            "stats",
        ]
        .iter()
        .find_map(|k| obj.get(*k))
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default(),
        _ => Vec::new(),
    };

    let mut by_hero: std::collections::HashMap<i64, HeroStats> = std::collections::HashMap::new();
    for item in raw_rows {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let pick = |keys: &[&str]| keys.iter().find_map(|k| obj.get(*k));

        let Some(hero_id) = coerce_int(pick(&["hero_id", "heroId", "id"]), None) else {
            continue;
        };
        let mut matches = coerce_int(
            pick(&["matches", "matches_played", "match_count", "games_played"]),
            None,
        );
        let mut wins = coerce_int(pick(&["wins", "win_count"]), None);
        let mut losses = coerce_int(pick(&["losses", "loss_count"]), None);
        let mut winrate = coerce_float(pick(&["winrate", "win_rate", "wr"]), None);

        if matches.is_none() {
            if let (Some(w), Some(l)) = (wins, losses) {
                matches = Some(w + l);
            }
        }
        if losses.is_none() {
            if let (Some(w), Some(m)) = (wins, matches) {
                losses = Some((m - w).max(0));
            }
        }
        if wins.is_none() {
            if let (Some(l), Some(m)) = (losses, matches) {
                wins = Some((m - l).max(0));
            }
        }
        if winrate.is_none() {
            if let (Some(m), Some(w)) = (matches, wins) {
                if m != 0 {
                    winrate = Some((w as f64 / m as f64) * 100.0);
                }
            }
        } else if let Some(wr) = winrate {
            if wr <= 1.0 {
                winrate = Some(wr * 100.0);
            }
        }

        let (Some(matches), Some(wins), Some(losses), Some(winrate)) =
            (matches, wins, losses, winrate)
        else {
            continue;
        };
        if matches < 0 || wins < 0 || losses < 0 {
            continue;
        }
        by_hero.insert(
            hero_id,
            HeroStats {
                hero_id,
                matches,
                wins,
                losses,
                winrate: py_round4(winrate),
            },
        );
    }
    by_hero.into_values().collect()
}

fn allocate_fetched_at(conn: &Connection, bucket: &str, preferred: i64) -> rusqlite::Result<i64> {
    let mut fetched_at = preferred;
    loop {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM tierlist_snapshots WHERE bucket = ?1 AND fetched_at = ?2",
                rusqlite::params![bucket, fetched_at],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Ok(fetched_at);
        }
        fetched_at += 1;
    }
}

fn prune_snapshots(conn: &Connection, bucket: &str) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id FROM tierlist_snapshots WHERE bucket = ?1
          ORDER BY fetched_at DESC, id DESC LIMIT -1 OFFSET ?2",
    )?;
    let stale: Vec<i64> = stmt
        .query_map(
            rusqlite::params![bucket, SNAPSHOT_RETENTION_PER_BUCKET],
            |row| row.get(0),
        )?
        .collect::<Result<_, _>>()?;
    if stale.is_empty() {
        return Ok(());
    }
    let placeholders = stale.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    conn.execute(
        &format!("DELETE FROM tierlist_snapshots WHERE id IN ({placeholders})"),
        rusqlite::params_from_iter(stale),
    )?;
    Ok(())
}

fn insert_snapshot_bundle(
    conn: &Connection,
    bucket: &str,
    patch_id: &str,
    patch_unix: i64,
    fetched_at: i64,
    stats: &[HeroStats],
) -> rusqlite::Result<Value> {
    let snapshot_ts = allocate_fetched_at(conn, bucket, fetched_at)?;
    conn.execute(
        "INSERT INTO tierlist_snapshots(bucket, patch_id, patch_unix, fetched_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![bucket, patch_id, patch_unix, snapshot_ts],
    )?;
    let snapshot_id = conn.last_insert_rowid();
    for s in stats {
        conn.execute(
            "INSERT INTO tierlist_snapshot_heroes(
                 snapshot_id, hero_id, matches, wins, losses, winrate
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                snapshot_id,
                s.hero_id,
                s.matches,
                s.wins,
                s.losses,
                s.winrate
            ],
        )?;
    }
    prune_snapshots(conn, bucket)?;
    Ok(json!({
        "snapshot_id": snapshot_id,
        "bucket": bucket,
        "fetched_at": snapshot_ts,
        "heroes": stats.len(),
    }))
}

pub async fn refresh_once(app: &SharedApp) -> Result<Value, RefreshError> {
    let _guard = app.refresh_lock.lock().await;

    let settings = app
        .db
        .write(|c| read_settings(c))
        .await
        .map_err(|e| RefreshError::Fatal(e.to_string()))?;

    let (patch_id, fetched_patch_unix) = fetch_current_patch(app).await?;
    // Python: int(settings["patch_override_unix"] or patch["patch_unix"]) — 0 ist falsy.
    let patch_unix = settings
        .patch_override_unix
        .filter(|v| *v != 0)
        .unwrap_or(fetched_patch_unix);
    let fetched_at = now_ts();

    let known: std::collections::HashSet<i64> = app
        .db
        .read(|conn| {
            let mut stmt =
                conn.prepare("SELECT hero_id FROM deadlock_heroes WHERE is_active = 1")?;
            let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
            rows.collect()
        })
        .await
        .map_err(|e| RefreshError::Fatal(e.to_string()))?;

    let mut bucket_results: Vec<(&str, Vec<HeroStats>)> = Vec::new();
    for (bucket, min_badge, max_badge) in BUCKETS {
        let payload = fetch_json(
            app,
            &format!("{}/analytics/hero-stats", app.api_base),
            &[
                ("min_average_badge", min_badge.to_string()),
                ("max_average_badge", max_badge.to_string()),
                ("min_unix_timestamp", patch_unix.to_string()),
                ("game_mode", "normal".to_string()),
            ],
        )
        .await?;
        let stats: Vec<HeroStats> = normalize_hero_stats(&payload)
            .into_iter()
            .filter(|s| known.contains(&s.hero_id))
            .collect();
        bucket_results.push((bucket, stats));
    }

    let patch_id_for_db = patch_id.clone();
    let inserted = app
        .db
        .write(move |conn| {
            let tx = conn.transaction()?;
            let mut inserted = Vec::new();
            for (bucket, stats) in &bucket_results {
                inserted.push(insert_snapshot_bundle(
                    &tx,
                    bucket,
                    &patch_id_for_db,
                    patch_unix,
                    fetched_at,
                    stats,
                )?);
            }
            tx.commit()?;
            Ok(inserted)
        })
        .await
        .map_err(|e| RefreshError::Fatal(e.to_string()))?;

    tracing::info!(%patch_id, patch_unix, buckets = inserted.len(), "Tierlist-Refresh abgeschlossen");
    Ok(json!({
        "ok": true,
        "patch_id": patch_id,
        "patch_unix": patch_unix,
        "fetched_at": fetched_at,
        "snapshots": inserted,
    }))
}

/// Hintergrund-Loop: refresh, dann Intervall aus den Settings schlafen.
pub async fn refresh_loop(app: SharedApp) {
    loop {
        match refresh_once(&app).await {
            Ok(_) => {}
            Err(RefreshError::Retryable(msg)) => {
                tracing::warn!(%msg, "Tierlist-Refresh übersprungen (retryable)");
            }
            Err(RefreshError::Fatal(msg)) => {
                tracing::error!(%msg, "Tierlist-Refresh fehlgeschlagen");
            }
        }
        let interval = app
            .db
            .write(|c| read_settings(c))
            .await
            .map(|s| s.refresh_interval_seconds)
            .unwrap_or(crate::settings::REFRESH_DEFAULT_SECONDS)
            .max(1) as u64;
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_normalisierung_mit_aliassen_und_ableitungen() {
        let payload = json!({
            "data": [
                {"heroId": 1, "wins": 60, "losses": 40},
                {"hero_id": 2, "matches": 100, "wins": 55, "losses": 45, "winrate": 0.55},
                {"hero_id": 3},
                "kein-objekt"
            ]
        });
        let mut stats = normalize_hero_stats(&payload);
        stats.sort_by_key(|s| s.hero_id);
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].matches, 100);
        assert_eq!(stats[0].winrate, 60.0);
        // winrate <= 1.0 wird als Anteil interpretiert und ×100 genommen
        assert_eq!(stats[1].winrate, 55.0);
    }
}
