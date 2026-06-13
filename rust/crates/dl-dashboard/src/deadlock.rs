//! Deadlock-Konfiguration und Helden-Übersicht (`/api/deadlock/...`).
//!
//! Read-Seite des Konfig-Bereichs (Full-Access): die globale Ziel-Build-Konfig
//! aus dem KV-Store und die Helden samt Build-Snapshots. SQL und JSON-Form
//! 1:1 zum Original. Die Schreib-/Sync-Pfade (Upsert/Delete/Sync gegen die
//! externe Build-API) folgen separat.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use rusqlite::types::Value as SqlValue;
use serde_json::{json, Value};

use crate::web::{err_text, ok_json, DashboardApp};

/// SQLite-Wert → JSON (für unbekannt typisierte Durchreich-Felder).
fn sql_to_json(value: SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(i) => json!(i),
        SqlValue::Real(f) => json!(f),
        SqlValue::Text(s) => json!(s),
        SqlValue::Blob(_) => Value::Null,
    }
}

/// origin_build_id-Coercion: Integer direkt, numerischer Text geparst, sonst
/// `None` (wie `_map_deadlock_hero_row`).
fn coerce_opt_i64(value: SqlValue) -> Option<i64> {
    match value {
        SqlValue::Integer(i) => Some(i),
        SqlValue::Real(f) => Some(f as i64),
        SqlValue::Text(s) => s.trim().parse().ok(),
        _ => None,
    }
}

async fn global_target_build_name(app: &DashboardApp) -> String {
    app.db()
        .kv_get("deadlock", "global_target_build_name")
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub async fn deadlock_config(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers) {
        return resp;
    }
    ok_json(json!({ "global_target_build_name": global_target_build_name(&app).await }))
}

pub async fn deadlock_heroes(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers) {
        return resp;
    }
    let config_name = global_target_build_name(&app).await;

    let result = app
        .db()
        .read(move |conn| {
            // Build-Snapshots je Held gruppieren.
            let mut builds_stmt = conn.prepare(
                "SELECT id, hero_id, build_id, build_name, author_name, is_active, sort_order,
                        sync_status, sync_message, last_checked_at, last_synced_at,
                        last_alerted_at, source_version, source_last_updated_ts,
                        clone_build_id, clone_version, created_at, updated_at
                 FROM deadlock_hero_builds
                 ORDER BY hero_id ASC, sort_order ASC, build_id ASC",
            )?;
            let mut builds_by_hero: std::collections::HashMap<i64, Vec<Value>> =
                std::collections::HashMap::new();
            let mut brows = builds_stmt.query([])?;
            while let Some(r) = brows.next()? {
                let hero_id: i64 = r.get::<_, Option<i64>>(1)?.unwrap_or(0);
                let build = json!({
                    "id": r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                    "hero_id": hero_id,
                    "build_id": r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    "build_name": r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    "author_name": r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    "is_active": r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                    "sort_order": r.get::<_, Option<i64>>(6)?.unwrap_or(100),
                    "sync_status": sql_to_json(r.get(7)?),
                    "sync_message": sql_to_json(r.get(8)?),
                    "last_checked_at": sql_to_json(r.get(9)?),
                    "last_synced_at": sql_to_json(r.get(10)?),
                    "last_alerted_at": sql_to_json(r.get(11)?),
                    "source_version": sql_to_json(r.get(12)?),
                    "source_last_updated_ts": sql_to_json(r.get(13)?),
                    "clone_build_id": sql_to_json(r.get(14)?),
                    "clone_version": sql_to_json(r.get(15)?),
                    "created_at": sql_to_json(r.get(16)?),
                    "updated_at": sql_to_json(r.get(17)?),
                });
                builds_by_hero.entry(hero_id).or_default().push(build);
            }

            let mut heroes_stmt = conn.prepare(
                "SELECT id, hero_id, name, origin_build_id, target_build_name_override,
                        is_active, created_at, updated_at
                 FROM deadlock_heroes
                 ORDER BY hero_id",
            )?;
            let mut heroes = Vec::new();
            let mut hrows = heroes_stmt.query([])?;
            while let Some(r) = hrows.next()? {
                let hero_id: i64 = r.get::<_, Option<i64>>(1)?.unwrap_or(0);
                let origin_build_id = coerce_opt_i64(r.get(3)?);
                let is_active = r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0;
                let mut hero = json!({
                    "id": r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                    "hero_id": hero_id,
                    "name": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    "origin_build_id": origin_build_id,
                    "target_build_name_override": r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    "is_active": is_active,
                    "created_at": sql_to_json(r.get(6)?),
                    "updated_at": sql_to_json(r.get(7)?),
                });
                let hero_builds = builds_by_hero.remove(&hero_id).unwrap_or_default();
                let has_builds = !hero_builds.is_empty();
                hero["builds"] = json!(hero_builds);
                // Ohne Builds: Legacy-Vorschlag aus origin_build_id.
                if !has_builds {
                    if let Some(legacy_id) = origin_build_id {
                        hero["legacy_build_suggestion"] = json!({
                            "build_id": legacy_id,
                            "build_name": "",
                            "author_name": "Legacy",
                            "is_active": is_active,
                            "sort_order": 100,
                            "suggested": true,
                        });
                    }
                }
                heroes.push(hero);
            }
            Ok(heroes)
        })
        .await;

    match result {
        Ok(heroes) => ok_json(json!({
            "heroes": heroes,
            "config": { "global_target_build_name": config_name },
        })),
        Err(err) => {
            tracing::error!(%err, "deadlock_heroes fehlgeschlagen");
            err_text(500, "Deadlock heroes unavailable")
        }
    }
}
