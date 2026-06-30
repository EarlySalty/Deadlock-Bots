//! Deadlock-Konfiguration und Helden-Übersicht (`/api/deadlock/...`).
//!
//! Full-Access-Bereich: globale Ziel-Build-Konfig aus dem KV-Store, Helden-Liste
//! samt Build-Snapshots (GET), sowie Anlegen/Ändern (Upsert) und Löschen von
//! Helden inklusive Build-Snapshot. SQL und JSON-Form 1:1 zum Original.
//!
//! NOCH NICHT portiert: der externe Steam-Build-Sync (`_sync_deadlock_hero_builds`,
//! MAINTAIN_BUILD_CATALOG) — er hat in Rust kein Äquivalent (eigenes Steam-Worker-
//! Subsystem). Upsert/Delete schreiben daher die DB konsistent und geben
//! `sync_summary: null` zurück (wie das Original bei `sync_on_save=false`); der
//! Steam-Abgleich wird separat verdrahtet.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};

use crate::db::{i64_to_i32, utc_to_json_unix, DashboardDbError, DashboardDbResult};
use crate::web::{err_text, ok_json, DashboardApp};

async fn global_target_build_name(app: &DashboardApp) -> String {
    dl_central_db::kv::get(app.pool(), "deadlock", "global_target_build_name")
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub async fn deadlock_config(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
        return resp;
    }
    ok_json(json!({ "global_target_build_name": global_target_build_name(&app).await }))
}

/// Validiert einen Ziel-Build-Namen wie `_normalize_deadlock_target_name`:
/// None/null → "", kein String → Fehler, max 120 Zeichen, keine Steuerzeichen.
fn normalize_target_name(raw: Option<&Value>, field: &str) -> Result<String, Response> {
    match raw {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => {
            let value = s.trim();
            if value.is_empty() {
                return Ok(String::new());
            }
            if value.chars().count() > 120 {
                return Err(err_text(
                    400,
                    &format!("{field} must be at most 120 characters"),
                ));
            }
            if value.chars().any(|c| (c as u32) < 32 || c as u32 == 127) {
                return Err(err_text(
                    400,
                    &format!("{field} contains unsupported control characters"),
                ));
            }
            Ok(value.to_string())
        }
        Some(_) => Err(err_text(400, &format!("{field} must be string"))),
    }
}

pub async fn deadlock_config_update(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return err_text(400, "Invalid JSON body"),
    };
    // dict.get(k1, dict.get(k2)): k1 nur fallen lassen, wenn der Schlüssel fehlt.
    let has_snake = payload
        .as_object()
        .map(|o| o.contains_key("global_target_build_name"))
        .unwrap_or(false);
    let raw = if has_snake {
        payload.get("global_target_build_name")
    } else {
        payload.get("globalTargetBuildName")
    };
    let name = match normalize_target_name(raw, "global_target_build_name") {
        Ok(n) => n,
        Err(resp) => return resp,
    };

    let previous = global_target_build_name(&app).await;
    if let Err(err) =
        dl_central_db::kv::set(app.pool(), "deadlock", "global_target_build_name", &name).await
    {
        tracing::error!(%err, "global_target_build_name speichern fehlgeschlagen");
        return err_text(500, "Saving config failed");
    }
    tracing::info!(
        target: "audit",
        action = "config.global_target_build_name",
        user_id = session.user_id,
        display_name = %session.display_name,
        access = session.access_level.as_str(),
        from = %previous,
        to = %name,
        "AUDIT deadlock"
    );
    ok_json(json!({ "global_target_build_name": name }))
}

pub async fn deadlock_heroes(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
        return resp;
    }
    let config_name = global_target_build_name(&app).await;

    let result: DashboardDbResult<Vec<Value>> = async {
        let build_rows = sqlx::query!(
            r#"
            SELECT id, hero_id, build_id, build_name, author_name,
                   is_active AS "is_active!", sort_order,
                   sync_status, sync_message, last_checked_at, last_synced_at,
                   last_alerted_at, source_version, source_last_updated_at,
                   clone_build_id, clone_version, created_at, updated_at
              FROM tierlist.deadlock_hero_builds
             ORDER BY hero_id ASC, sort_order ASC, build_id ASC
            "#
        )
        .fetch_all(app.pool())
        .await?;
        let mut builds_by_hero: std::collections::HashMap<i64, Vec<Value>> =
            std::collections::HashMap::new();
        for row in build_rows {
            let hero_id = row.hero_id;
            builds_by_hero
                .entry(hero_id)
                .or_default()
                .push(build_json(HeroBuildDbRow {
                    id: row.id,
                    hero_id,
                    build_id: row.build_id,
                    build_name: row.build_name,
                    author_name: row.author_name,
                    is_active: row.is_active,
                    sort_order: row.sort_order,
                    sync_status: row.sync_status,
                    sync_message: row.sync_message,
                    last_checked_at: row.last_checked_at,
                    last_synced_at: row.last_synced_at,
                    last_alerted_at: row.last_alerted_at,
                    source_version: row.source_version,
                    source_last_updated_at: row.source_last_updated_at,
                    clone_build_id: row.clone_build_id,
                    clone_version: row.clone_version,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                }));
        }

        let hero_rows = sqlx::query!(
            r#"
            SELECT id, hero_id, name, origin_build_id, target_build_name_override,
                   is_active AS "is_active!", created_at, updated_at
              FROM tierlist.deadlock_heroes
             ORDER BY hero_id
            "#
        )
        .fetch_all(app.pool())
        .await?;
        let mut heroes = Vec::new();
        for row in hero_rows {
            let hero_id = row.hero_id;
            let origin_build_id = row.origin_build_id;
            let is_active = row.is_active;
            let mut hero = hero_json_base(HeroDbRow {
                id: row.id,
                hero_id,
                name: row.name,
                origin_build_id,
                target_build_name_override: row.target_build_name_override,
                is_active,
                created_at: row.created_at,
                updated_at: row.updated_at,
            });
            let hero_builds = builds_by_hero.remove(&hero_id).unwrap_or_default();
            let has_builds = !hero_builds.is_empty();
            hero["builds"] = json!(hero_builds);
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
    }
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

// ── Schreib-Seite: Upsert + Delete ──────────────────────────────────────────

/// Zwei-Schlüssel-Lookup (snake_case ODER camelCase), wie Pythons `dict.get`.
fn get2<'a>(obj: &'a Value, k1: &str, k2: &str) -> Option<&'a Value> {
    obj.get(k1).or_else(|| obj.get(k2))
}

/// Ob mindestens einer der beiden Schlüssel im Objekt existiert.
fn has2(obj: &Value, k1: &str, k2: &str) -> bool {
    obj.as_object()
        .map(|o| o.contains_key(k1) || o.contains_key(k2))
        .unwrap_or(false)
}

/// `_coerce_int`: Integer direkt, numerischer String geparst, sonst None.
fn coerce_int(raw: Option<&Value>) -> Option<i64> {
    match raw {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

/// `_coerce_request_bool`: None/null→default, bool direkt, int 0/1→bool
/// (sonst Fehler), String über true/false/1/0/yes/no/on/off.
fn coerce_req_bool(raw: Option<&Value>, default: bool, field: &str) -> Result<bool, Response> {
    match raw {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(err_text(400, &format!("{field} must be boolean"))),
        },
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(err_text(400, &format!("{field} must be boolean"))),
        },
        Some(_) => Err(err_text(400, &format!("{field} must be boolean"))),
    }
}

/// Eine geparste Build-Zeile aus dem Upsert-Payload.
struct BuildRow {
    build_id: i64,
    build_name: String,
    author_name: String,
    is_active: bool,
    sort_order: i64,
}

/// `_parse_deadlock_build_payload_row`.
fn parse_build_row(raw: &Value, index: usize) -> Result<BuildRow, Response> {
    if !raw.is_object() {
        return Err(err_text(400, &format!("builds[{index}] must be an object")));
    }
    let build_id = coerce_int(get2(raw, "build_id", "buildId"))
        .ok_or_else(|| err_text(400, &format!("builds[{index}].build_id must be integer")))?;
    let build_name = normalize_target_name(
        get2(raw, "build_name", "buildName"),
        &format!("builds[{index}].build_name"),
    )?;
    let author_name = get2(raw, "author_name", "authorName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if author_name.is_empty() {
        return Err(err_text(
            400,
            &format!("builds[{index}].author_name is required"),
        ));
    }
    let is_active = coerce_req_bool(
        get2(raw, "is_active", "isActive"),
        true,
        &format!("builds[{index}].is_active"),
    )?;
    let raw_sort = get2(raw, "sort_order", "sortOrder");
    let sort_order = match raw_sort {
        None | Some(Value::Null) => 100,
        Some(Value::String(s)) if s.trim().is_empty() => 100,
        other => coerce_int(other)
            .ok_or_else(|| err_text(400, &format!("builds[{index}].sort_order must be integer")))?,
    };
    Ok(BuildRow {
        build_id,
        build_name,
        author_name,
        is_active,
        sort_order,
    })
}

/// `_extract_deadlock_builds_payload`: None wenn kein builds-Schlüssel; leere
/// Liste bei null; sonst je Zeile geparst + Duplikat-build_id-Prüfung.
fn extract_builds(payload: &Value) -> Result<Option<Vec<BuildRow>>, Response> {
    if !(has2(payload, "builds", "hero_builds") || payload.get("heroBuilds").is_some()) {
        return Ok(None);
    }
    let raw = payload
        .get("builds")
        .or_else(|| payload.get("hero_builds"))
        .or_else(|| payload.get("heroBuilds"));
    match raw {
        None | Some(Value::Null) => Ok(Some(Vec::new())),
        Some(Value::Array(arr)) => {
            let mut seen = std::collections::HashSet::new();
            let mut builds = Vec::with_capacity(arr.len());
            for (i, rb) in arr.iter().enumerate() {
                let parsed = parse_build_row(rb, i + 1)?;
                if !seen.insert(parsed.build_id) {
                    return Err(err_text(
                        400,
                        &format!("Duplicate build_id in builds: {}", parsed.build_id),
                    ));
                }
                builds.push(parsed);
            }
            Ok(Some(builds))
        }
        Some(_) => Err(err_text(400, "builds must be an array")),
    }
}

struct HeroBuildDbRow {
    id: i64,
    hero_id: i64,
    build_id: i64,
    build_name: String,
    author_name: String,
    is_active: bool,
    sort_order: i32,
    sync_status: Option<String>,
    sync_message: Option<String>,
    last_checked_at: Option<DateTime<Utc>>,
    last_synced_at: Option<DateTime<Utc>>,
    last_alerted_at: Option<DateTime<Utc>>,
    source_version: Option<i64>,
    source_last_updated_at: Option<DateTime<Utc>>,
    clone_build_id: Option<i64>,
    clone_version: Option<i64>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn build_json(row: HeroBuildDbRow) -> Value {
    json!({
        "id": row.id,
        "hero_id": row.hero_id,
        "build_id": row.build_id,
        "build_name": row.build_name,
        "author_name": row.author_name,
        "is_active": row.is_active,
        "sort_order": row.sort_order,
        "sync_status": row.sync_status,
        "sync_message": row.sync_message,
        "last_checked_at": utc_to_json_unix(row.last_checked_at),
        "last_synced_at": utc_to_json_unix(row.last_synced_at),
        "last_alerted_at": utc_to_json_unix(row.last_alerted_at),
        "source_version": row.source_version,
        "source_last_updated_ts": utc_to_json_unix(row.source_last_updated_at),
        "clone_build_id": row.clone_build_id,
        "clone_version": row.clone_version,
        "created_at": row.created_at.timestamp(),
        "updated_at": row.updated_at.timestamp(),
    })
}

struct HeroDbRow {
    id: i64,
    hero_id: i64,
    name: String,
    origin_build_id: Option<i64>,
    target_build_name_override: Option<String>,
    is_active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn hero_json_base(row: HeroDbRow) -> Value {
    json!({
        "id": row.id,
        "hero_id": row.hero_id,
        "name": row.name,
        "origin_build_id": row.origin_build_id,
        "target_build_name_override": row.target_build_name_override.unwrap_or_default(),
        "is_active": row.is_active,
        "created_at": row.created_at.timestamp(),
        "updated_at": row.updated_at.timestamp(),
    })
}

/// `_apply_deadlock_build_snapshot`: Builds upserten, dann nicht übermittelte
/// build_ids des Helden löschen (Replace-Semantik; leere Liste → alle löschen).
async fn apply_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    hero_id: i64,
    builds: &[BuildRow],
    ts: DateTime<Utc>,
) -> DashboardDbResult<()> {
    for b in builds {
        let sort_order = i64_to_i32(b.sort_order, "sort_order")?;
        sqlx::query!(
            r#"
            INSERT INTO tierlist.deadlock_hero_builds (
                hero_id, build_id, build_name, author_name, is_active, sort_order, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
             ON CONFLICT(hero_id, build_id) DO UPDATE SET
                build_name = EXCLUDED.build_name,
                author_name = EXCLUDED.author_name,
                is_active = EXCLUDED.is_active,
                sort_order = EXCLUDED.sort_order,
                updated_at = EXCLUDED.updated_at
            "#,
            hero_id,
            b.build_id,
            b.build_name,
            b.author_name,
            b.is_active,
            sort_order,
            ts,
        )
        .execute(&mut **tx)
        .await?;
    }
    if builds.is_empty() {
        sqlx::query!(
            r#"
            DELETE FROM tierlist.deadlock_hero_builds WHERE hero_id = $1
            "#,
            hero_id,
        )
        .execute(&mut **tx)
        .await?;
    } else {
        let build_ids: Vec<i64> = builds.iter().map(|b| b.build_id).collect();
        sqlx::query!(
            r#"
            DELETE FROM tierlist.deadlock_hero_builds
             WHERE hero_id = $1
               AND NOT (build_id = ANY($2))
            "#,
            hero_id,
            &build_ids,
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// `_load_deadlock_hero_payload`: ein Held samt Builds (+ Legacy-Vorschlag),
/// JSON-Form 1:1 wie der GET-Handler.
async fn load_hero_json(
    tx: &mut Transaction<'_, Postgres>,
    hero_id: i64,
) -> DashboardDbResult<Option<Value>> {
    let builds = sqlx::query!(
        r#"
        SELECT id, hero_id, build_id, build_name, author_name,
               is_active AS "is_active!", sort_order,
               sync_status, sync_message, last_checked_at, last_synced_at,
               last_alerted_at, source_version, source_last_updated_at,
               clone_build_id, clone_version, created_at, updated_at
          FROM tierlist.deadlock_hero_builds
         WHERE hero_id = $1
         ORDER BY sort_order ASC, build_id ASC
        "#,
        hero_id,
    )
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|row| {
        build_json(HeroBuildDbRow {
            id: row.id,
            hero_id: row.hero_id,
            build_id: row.build_id,
            build_name: row.build_name,
            author_name: row.author_name,
            is_active: row.is_active,
            sort_order: row.sort_order,
            sync_status: row.sync_status,
            sync_message: row.sync_message,
            last_checked_at: row.last_checked_at,
            last_synced_at: row.last_synced_at,
            last_alerted_at: row.last_alerted_at,
            source_version: row.source_version,
            source_last_updated_at: row.source_last_updated_at,
            clone_build_id: row.clone_build_id,
            clone_version: row.clone_version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    })
    .collect::<Vec<_>>();

    let Some(row) = sqlx::query!(
        r#"
        SELECT id, hero_id, name, origin_build_id, target_build_name_override,
               is_active AS "is_active!", created_at, updated_at
          FROM tierlist.deadlock_heroes
         WHERE hero_id = $1
        "#,
        hero_id,
    )
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    let origin = row.origin_build_id;
    let is_active = row.is_active;
    let mut hero = hero_json_base(HeroDbRow {
        id: row.id,
        hero_id: row.hero_id,
        name: row.name,
        origin_build_id: origin,
        target_build_name_override: row.target_build_name_override,
        is_active,
        created_at: row.created_at,
        updated_at: row.updated_at,
    });
    let has_builds = !builds.is_empty();
    hero["builds"] = json!(builds);
    if !has_builds {
        if let Some(legacy_id) = origin {
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
    Ok(Some(hero))
}

fn is_unique_violation(err: &DashboardDbError) -> bool {
    match err {
        DashboardDbError::Sqlx(sqlx::Error::Database(db)) => db.code().as_deref() == Some("23505"),
        _ => false,
    }
}

/// POST `/api/deadlock/heroes` — Held anlegen/aktualisieren (+ Build-Snapshot).
/// Port von `_handle_deadlock_upsert_hero`; Steam-Sync deferred (`sync_summary: null`).
pub async fn deadlock_upsert_hero(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return err_text(400, "Invalid JSON body"),
    };

    let hero_id = match coerce_int(get2(&payload, "hero_id", "heroId")) {
        Some(i) => i,
        None => return err_text(400, "hero_id must be an integer"),
    };
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return err_text(400, "name is required");
    }

    let has_origin = has2(&payload, "origin_build_id", "originBuildId");
    let parsed_origin: Option<i64> = if has_origin {
        match get2(&payload, "origin_build_id", "originBuildId") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            other => match coerce_int(other) {
                Some(i) => Some(i),
                None => return err_text(400, "origin_build_id must be integer or null"),
            },
        }
    } else {
        None
    };

    let has_is_active = has2(&payload, "is_active", "isActive");
    let parsed_is_active: Option<bool> = if has_is_active {
        match coerce_req_bool(get2(&payload, "is_active", "isActive"), true, "is_active") {
            Ok(b) => Some(b),
            Err(resp) => return resp,
        }
    } else {
        None
    };

    let has_target = has2(
        &payload,
        "target_build_name_override",
        "targetBuildNameOverride",
    );
    let target_override = match normalize_target_name(
        get2(
            &payload,
            "target_build_name_override",
            "targetBuildNameOverride",
        ),
        "target_build_name_override",
    ) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let builds = match extract_builds(&payload) {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let ts = chrono::Utc::now();
    let name_c = name.clone();
    let target_c = target_override.clone();
    let result: DashboardDbResult<(Option<Value>, String, String)> = async {
        let mut tx = app.pool().begin().await?;
        let existing_row = sqlx::query!(
            r#"
            SELECT name, origin_build_id, is_active AS "is_active!",
                   target_build_name_override
              FROM tierlist.deadlock_heroes
             WHERE hero_id = $1
            "#,
            hero_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let existing = existing_row.map(|r| {
            (
                r.name,
                r.origin_build_id,
                r.is_active,
                r.target_build_name_override.unwrap_or_default(),
            )
        });

        let origin_build_id = if has_origin {
            parsed_origin
        } else {
            existing.as_ref().and_then(|e| e.1)
        };
        let target = if has_target {
            target_c.clone()
        } else {
            existing.as_ref().map(|e| e.3.clone()).unwrap_or_default()
        };
        let is_active = if has_is_active {
            parsed_is_active.unwrap_or(true)
        } else {
            existing.as_ref().map(|e| e.2).unwrap_or(true)
        };

        sqlx::query!(
            r#"
            INSERT INTO tierlist.deadlock_heroes (
                    hero_id, name, origin_build_id, target_build_name_override,
                    is_active, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $6)
                 ON CONFLICT(hero_id) DO UPDATE SET
                    name = EXCLUDED.name,
                    origin_build_id = EXCLUDED.origin_build_id,
                    target_build_name_override = EXCLUDED.target_build_name_override,
                    is_active = EXCLUDED.is_active,
                    updated_at = EXCLUDED.updated_at
            "#,
            hero_id,
            name_c,
            origin_build_id,
            target,
            is_active,
            ts,
        )
        .execute(&mut *tx)
        .await?;

        if let Some(b) = &builds {
            apply_snapshot(&mut tx, hero_id, b, ts).await?;
        }
        let saved = load_hero_json(&mut tx, hero_id).await?;
        let prev_name = existing.as_ref().map(|e| e.0.clone()).unwrap_or_default();
        let prev_override = existing.map(|e| e.3).unwrap_or_default();
        tx.commit().await?;
        Ok((saved, prev_name, prev_override))
    }
    .await;

    match result {
        Ok((Some(hero), prev_name, prev_override)) => {
            tracing::info!(
                target: "audit",
                action = "hero.upsert",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                hero_id,
                name_from = %prev_name,
                name_to = %name,
                override_from = %prev_override,
                override_to = %target_override,
                "AUDIT deadlock"
            );
            ok_json(json!({ "hero": hero, "sync_summary": Value::Null }))
        }
        Ok((None, _, _)) => err_text(500, "Saving hero failed"),
        Err(err) if is_unique_violation(&err) => {
            err_text(409, "Hero with same ID or name already exists")
        }
        Err(err) => {
            tracing::error!(%err, "Deadlock-Held-Upsert fehlgeschlagen");
            err_text(500, "Saving hero failed")
        }
    }
}

/// DELETE `/api/deadlock/heroes/{hero_id}` — Held + Build-Snapshot löschen.
/// Port von `_handle_deadlock_delete_hero`; Steam-Sync deferred.
pub async fn deadlock_delete_hero(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(hero_id_raw): Path<String>,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let hero_id: i64 = match hero_id_raw.trim().parse() {
        Ok(i) => i,
        Err(_) => return err_text(400, "hero_id must be integer"),
    };
    let result: DashboardDbResult<u64> = async {
        let mut tx = app.pool().begin().await?;
        sqlx::query!(
            r#"
            DELETE FROM tierlist.deadlock_hero_builds WHERE hero_id = $1
            "#,
            hero_id,
        )
        .execute(&mut *tx)
        .await?;
        let deleted = sqlx::query!(
            r#"
            DELETE FROM tierlist.deadlock_heroes WHERE hero_id = $1
            "#,
            hero_id,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(deleted)
    }
    .await;
    match result {
        Ok(deleted) => {
            tracing::info!(
                target: "audit",
                action = "hero.delete",
                user_id = session.user_id,
                access = session.access_level.as_str(),
                hero_id,
                deleted = deleted > 0,
                "AUDIT deadlock"
            );
            ok_json(json!({ "deleted": deleted > 0, "sync_summary": Value::Null }))
        }
        Err(err) => {
            tracing::error!(%err, "Deadlock-Held-Löschen fehlgeschlagen");
            err_text(500, "Delete hero failed")
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn normalize_regeln() {
        assert_eq!(normalize_target_name(None, "f").ok(), Some(String::new()));
        assert_eq!(
            normalize_target_name(Some(&Value::Null), "f").ok(),
            Some(String::new())
        );
        assert_eq!(
            normalize_target_name(Some(&json!("  Build A  ")), "f").ok(),
            Some("Build A".to_string())
        );
        assert_eq!(
            normalize_target_name(Some(&json!("   ")), "f").ok(),
            Some(String::new())
        );
        assert!(normalize_target_name(Some(&json!(5)), "f").is_err());
        assert!(normalize_target_name(Some(&json!(true)), "f").is_err());
        let long = "x".repeat(121);
        assert!(normalize_target_name(Some(&json!(long)), "f").is_err());
        assert!(normalize_target_name(Some(&json!("a\u{0007}b")), "f").is_err());
    }

    #[test]
    fn coerce_int_varianten() {
        assert_eq!(coerce_int(Some(&json!(42))), Some(42));
        assert_eq!(coerce_int(Some(&json!("17"))), Some(17));
        assert_eq!(coerce_int(Some(&json!(" 5 "))), Some(5));
        assert_eq!(coerce_int(Some(&json!("abc"))), None);
        assert_eq!(coerce_int(Some(&Value::Null)), None);
        assert_eq!(coerce_int(None), None);
    }

    #[test]
    fn coerce_bool_varianten() {
        assert_eq!(coerce_req_bool(None, true, "f").ok(), Some(true));
        assert_eq!(
            coerce_req_bool(Some(&json!(false)), true, "f").ok(),
            Some(false)
        );
        assert_eq!(
            coerce_req_bool(Some(&json!(1)), false, "f").ok(),
            Some(true)
        );
        assert_eq!(
            coerce_req_bool(Some(&json!("no")), true, "f").ok(),
            Some(false)
        );
        assert!(coerce_req_bool(Some(&json!(2)), false, "f").is_err());
        assert!(coerce_req_bool(Some(&json!("maybe")), false, "f").is_err());
    }

    #[test]
    fn build_row_parsing() {
        let ok = parse_build_row(
            &json!({"build_id": 9, "build_name": " B ", "author_name": " Nani "}),
            1,
        )
        .expect("ok");
        assert_eq!(ok.build_id, 9);
        assert_eq!(ok.build_name, "B");
        assert_eq!(ok.author_name, "Nani");
        assert!(ok.is_active); // default true
        assert_eq!(ok.sort_order, 100); // default
                                        // fehlende author_name -> Fehler
        assert!(parse_build_row(&json!({"build_id": 1, "build_name": "x"}), 1).is_err());
        // fehlende build_id -> Fehler
        assert!(parse_build_row(&json!({"build_name": "x", "author_name": "y"}), 1).is_err());
    }

    #[test]
    fn extract_builds_semantik() {
        // kein Schlüssel -> None
        assert!(extract_builds(&json!({"hero_id": 1})).unwrap().is_none());
        // null -> leere Liste
        assert_eq!(
            extract_builds(&json!({"builds": Value::Null}))
                .unwrap()
                .map(|v| v.len()),
            Some(0)
        );
        // Duplikat-build_id -> Fehler
        assert!(extract_builds(&json!({"builds": [
            {"build_id": 1, "build_name": "a", "author_name": "x"},
            {"build_id": 1, "build_name": "b", "author_name": "y"}
        ]}))
        .is_err());
        // camelCase-Schlüssel akzeptiert
        let camel = extract_builds(&json!({"heroBuilds": [
            {"buildId": 3, "buildName": "c", "authorName": "z", "sortOrder": 5}
        ]}))
        .unwrap()
        .unwrap();
        assert_eq!(camel.len(), 1);
        assert_eq!(camel[0].build_id, 3);
        assert_eq!(camel[0].sort_order, 5);
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn apply_snapshot_ersetzt_builds_pro_held() -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let mut tx = db.pool().begin().await?;
        let now = Utc::now();
        sqlx::query!(
            r#"
            INSERT INTO tierlist.deadlock_heroes(
                hero_id, name, is_active, created_at, updated_at
            )
            VALUES (10, 'Hero A', TRUE, $1, $1)
            "#,
            now,
        )
        .execute(&mut *tx)
        .await?;
        apply_snapshot(
            &mut tx,
            10,
            &[
                BuildRow {
                    build_id: 100,
                    build_name: "Build 100".to_string(),
                    author_name: "Nani".to_string(),
                    is_active: true,
                    sort_order: 1,
                },
                BuildRow {
                    build_id: 200,
                    build_name: "Build 200".to_string(),
                    author_name: "Nani".to_string(),
                    is_active: true,
                    sort_order: 2,
                },
            ],
            now,
        )
        .await?;
        tx.commit().await?;

        let mut tx = db.pool().begin().await?;
        apply_snapshot(
            &mut tx,
            10,
            &[BuildRow {
                build_id: 100,
                build_name: "Build 100b".to_string(),
                author_name: "Nani".to_string(),
                is_active: false,
                sort_order: 5,
            }],
            Utc::now(),
        )
        .await?;
        tx.commit().await?;

        let rows = sqlx::query!(
            r#"
            SELECT build_id, build_name, is_active AS "is_active!", sort_order
              FROM tierlist.deadlock_hero_builds
             WHERE hero_id = 10
             ORDER BY build_id
            "#
        )
        .fetch_all(db.pool())
        .await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].build_id, 100);
        assert_eq!(rows[0].build_name, "Build 100b");
        assert!(!rows[0].is_active);
        assert_eq!(rows[0].sort_order, 5);

        let mut tx = db.pool().begin().await?;
        apply_snapshot(&mut tx, 10, &[], Utc::now()).await?;
        tx.commit().await?;
        let remaining = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM tierlist.deadlock_hero_builds
             WHERE hero_id = 10
            "#
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(remaining, 0);
        Ok(())
    }
}
