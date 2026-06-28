//! Dashboard-CRUD für generische Reaction-Rollen.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use dl_community::reaction_roles::canonical_emoji_input;
use dl_db::DbError;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::web::{err_text, ok_json, DashboardApp};

fn get2<'a>(obj: &'a Value, k1: &str, k2: &str) -> Option<&'a Value> {
    obj.get(k1).or_else(|| obj.get(k2))
}

fn sqlite_positive_u64(value: u64, field: &str) -> Result<u64, Response> {
    if value == 0 || value > i64::MAX as u64 {
        return Err(err_text(
            400,
            &format!("{field} must be u64 between 1 and i64::MAX"),
        ));
    }
    Ok(value)
}

fn parse_u64(raw: Option<&Value>, field: &str) -> Result<u64, Response> {
    let value = match raw {
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| err_text(400, &format!("{field} must be u64")))?,
        Some(Value::String(s)) => s
            .trim()
            .parse::<u64>()
            .map_err(|_| err_text(400, &format!("{field} must be u64")))?,
        _ => return Err(err_text(400, &format!("{field} must be u64"))),
    };
    sqlite_positive_u64(value, field)
}

fn parse_optional_id(raw: Option<&Value>, field: &str) -> Result<Option<i64>, Response> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(_) => parse_u64(raw, field).map(|value| Some(value as i64)),
    }
}

fn parse_path_id(raw: &str, field: &str) -> Result<i64, Response> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| err_text(400, &format!("{field} must be u64")))
        .and_then(|value| sqlite_positive_u64(value, field))
        .map(|value| value as i64)
}

fn parse_bool(raw: Option<&Value>, default: bool, field: &str) -> Result<bool, Response> {
    match raw {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
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

fn optional_string(raw: Option<&Value>, field: &str) -> Result<Option<String>, Response> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.to_string())),
        Some(_) => Err(err_text(400, &format!("{field} must be string"))),
    }
}

struct ReactionRolePayload {
    id: Option<i64>,
    guild_id: u64,
    source_channel_id: u64,
    message_id: u64,
    emoji: String,
    role_id: u64,
    dm_enabled: bool,
    dm_text: Option<String>,
    remove_on_unreact: bool,
    backfill: bool,
}

fn parse_payload(payload: &Value) -> Result<ReactionRolePayload, Response> {
    let id = parse_optional_id(payload.get("id"), "id")?;
    let guild_id = parse_u64(get2(payload, "guild_id", "guildId"), "guild_id")?;
    let source_channel_id = parse_u64(
        get2(payload, "source_channel_id", "sourceChannelId"),
        "source_channel_id",
    )?;
    let message_id = parse_u64(get2(payload, "message_id", "messageId"), "message_id")?;
    let emoji = canonical_emoji_input(payload.get("emoji").and_then(Value::as_str).unwrap_or(""));
    if emoji.is_empty() {
        return Err(err_text(400, "emoji is required"));
    }
    let role_id = parse_u64(get2(payload, "role_id", "roleId"), "role_id")?;
    Ok(ReactionRolePayload {
        id,
        guild_id,
        source_channel_id,
        message_id,
        emoji,
        role_id,
        dm_enabled: parse_bool(
            get2(payload, "dm_enabled", "dmEnabled"),
            false,
            "dm_enabled",
        )?,
        dm_text: optional_string(get2(payload, "dm_text", "dmText"), "dm_text")?,
        remove_on_unreact: parse_bool(
            get2(payload, "remove_on_unreact", "removeOnUnreact"),
            true,
            "remove_on_unreact",
        )?,
        backfill: parse_bool(payload.get("backfill"), false, "backfill")?,
    })
}

fn row_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "guild_id": row.get::<_, u64>(1)?,
        "source_channel_id": row.get::<_, u64>(2)?,
        "message_id": row.get::<_, u64>(3)?,
        "emoji": row.get::<_, String>(4)?,
        "role_id": row.get::<_, u64>(5)?,
        "dm_enabled": row.get::<_, i64>(6)? != 0,
        "dm_text": row.get::<_, Option<String>>(7)?,
        "remove_on_unreact": row.get::<_, i64>(8)? != 0,
        "backfill_pending": row.get::<_, i64>(9)? != 0,
        "active": row.get::<_, i64>(10)? != 0,
        "created_at": row.get::<_, i64>(11)?,
        "updated_at": row.get::<_, i64>(12)?,
    }))
}

fn load_mapping(conn: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    conn.query_row(
        "SELECT id, guild_id, source_channel_id, message_id, emoji, role_id,
                dm_enabled, dm_text, remove_on_unreact, backfill_pending,
                active, created_at, updated_at
           FROM reaction_role_mappings
          WHERE id = ?1",
        params![id],
        row_json,
    )
    .optional()
}

fn save_mapping(
    conn: &mut Connection,
    payload: ReactionRolePayload,
    ts: i64,
) -> rusqlite::Result<Option<Value>> {
    let tx = conn.transaction()?;
    let saved_id = if let Some(id) = payload.id {
        let exists = tx
            .query_row(
                "SELECT 1 FROM reaction_role_mappings WHERE id = ?1",
                params![id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .is_some();
        if !exists {
            tx.commit()?;
            return Ok(None);
        }
        tx.execute(
            "UPDATE reaction_role_mappings
                SET guild_id = ?1,
                    source_channel_id = ?2,
                    message_id = ?3,
                    emoji = ?4,
                    role_id = ?5,
                    dm_enabled = ?6,
                    dm_text = ?7,
                    remove_on_unreact = ?8,
                    backfill_pending = ?9,
                    active = 1,
                    updated_at = ?10
              WHERE id = ?11",
            params![
                payload.guild_id,
                payload.source_channel_id,
                payload.message_id,
                payload.emoji,
                payload.role_id,
                payload.dm_enabled as i64,
                payload.dm_text,
                payload.remove_on_unreact as i64,
                payload.backfill as i64,
                ts,
                id,
            ],
        )?;
        id
    } else {
        tx.execute(
            "INSERT INTO reaction_role_mappings(
                guild_id, source_channel_id, message_id, emoji, role_id,
                dm_enabled, dm_text, remove_on_unreact, backfill_pending,
                active, created_at, updated_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10)
             ON CONFLICT(message_id, emoji) DO UPDATE SET
                guild_id = excluded.guild_id,
                source_channel_id = excluded.source_channel_id,
                role_id = excluded.role_id,
                dm_enabled = excluded.dm_enabled,
                dm_text = excluded.dm_text,
                remove_on_unreact = excluded.remove_on_unreact,
                backfill_pending = excluded.backfill_pending,
                active = 1,
                updated_at = excluded.updated_at",
            params![
                payload.guild_id,
                payload.source_channel_id,
                payload.message_id,
                payload.emoji,
                payload.role_id,
                payload.dm_enabled as i64,
                payload.dm_text,
                payload.remove_on_unreact as i64,
                payload.backfill as i64,
                ts,
            ],
        )?;
        tx.query_row(
            "SELECT id FROM reaction_role_mappings WHERE message_id = ?1 AND emoji = ?2",
            params![payload.message_id, payload.emoji],
            |row| row.get(0),
        )?
    };
    let saved = load_mapping(&tx, saved_id)?;
    tx.commit()?;
    Ok(saved)
}

pub async fn reaction_roles(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers) {
        return resp;
    }
    let result = app
        .db()
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, guild_id, source_channel_id, message_id, emoji, role_id,
                        dm_enabled, dm_text, remove_on_unreact, backfill_pending,
                        active, created_at, updated_at
                   FROM reaction_role_mappings
                  ORDER BY id DESC",
            )?;
            let rows = stmt.query_map([], row_json)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await;
    match result {
        Ok(mappings) => ok_json(json!(mappings)),
        Err(err) => {
            tracing::error!(%err, "Reaction-Roles laden fehlgeschlagen");
            err_text(500, "Reaction roles unavailable")
        }
    }
}

pub async fn reaction_role_upsert(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let raw = match serde_json::from_slice::<Value>(&body) {
        Ok(value) if value.is_object() => value,
        _ => return err_text(400, "Invalid JSON body"),
    };
    let payload = match parse_payload(&raw) {
        Ok(payload) => payload,
        Err(resp) => return resp,
    };
    let ts = chrono::Utc::now().timestamp();
    let result = app
        .db()
        .write(move |conn| save_mapping(conn, payload, ts))
        .await;

    match result {
        Ok(Some(mapping)) => {
            tracing::info!(
                target: "audit",
                action = "reaction_role.upsert",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                "AUDIT reaction_roles"
            );
            ok_json(json!({ "mapping": mapping }))
        }
        Ok(None) => err_text(404, "Reaction role mapping not found"),
        Err(DbError::Sqlite(rusqlite::Error::SqliteFailure(e, _)))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            err_text(409, "Reaction role mapping already exists")
        }
        Err(err) => {
            tracing::error!(%err, "Reaction-Role-Upsert fehlgeschlagen");
            err_text(500, "Saving reaction role failed")
        }
    }
}

pub async fn reaction_role_delete(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(id_raw): Path<String>,
) -> Response {
    let session = match app.guard_mutate(&headers, true) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let id = match parse_path_id(&id_raw, "id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let result = app
        .db()
        .write(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "DELETE FROM reaction_role_dm_log WHERE mapping_id = ?1",
                params![id],
            )?;
            let deleted = tx.execute(
                "DELETE FROM reaction_role_mappings WHERE id = ?1",
                params![id],
            )?;
            tx.commit()?;
            Ok(deleted)
        })
        .await;
    match result {
        Ok(deleted) => {
            tracing::info!(
                target: "audit",
                action = "reaction_role.delete",
                user_id = session.user_id,
                access = session.access_level.as_str(),
                mapping_id = id,
                deleted = deleted > 0,
                "AUDIT reaction_roles"
            );
            ok_json(json!({ "deleted": deleted > 0 }))
        }
        Err(err) => {
            tracing::error!(%err, "Reaction-Role-Löschen fehlgeschlagen");
            err_text(500, "Delete reaction role failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;

    fn payload_json(id: Value) -> Value {
        json!({
            "id": id,
            "guild_id": "42",
            "source_channel_id": "43",
            "message_id": "100",
            "emoji": "✅",
            "role_id": "500",
        })
    }

    fn payload(id: Option<i64>, message_id: u64, role_id: u64) -> ReactionRolePayload {
        ReactionRolePayload {
            id,
            guild_id: 42,
            source_channel_id: 43,
            message_id,
            emoji: "✅".to_string(),
            role_id,
            dm_enabled: true,
            dm_text: Some("DM".to_string()),
            remove_on_unreact: true,
            backfill: false,
        }
    }

    #[test]
    fn canonical_emoji_input_akzeptiert_unicode_name_id_und_discord_custom() {
        assert_eq!(canonical_emoji_input(" ✅ "), "✅");
        assert_eq!(canonical_emoji_input("pog:123"), "pog:123");
        assert_eq!(canonical_emoji_input("<:pog:123>"), "pog:123");
        assert_eq!(canonical_emoji_input("<a:spin:456>"), "spin:456");
    }

    #[test]
    fn parse_payload_validiert_ids_positiv_und_sqlite_kompatibel() {
        for (field, value) in [
            ("id", json!(0)),
            ("id", json!(-1)),
            ("id", json!((i64::MAX as u64 + 1).to_string())),
            ("guild_id", json!(0)),
            ("source_channel_id", json!(-1)),
            ("message_id", json!((i64::MAX as u64 + 1).to_string())),
            ("role_id", json!("0")),
        ] {
            let mut body = payload_json(Value::Null);
            body.as_object_mut()
                .expect("object")
                .insert(field.to_string(), value);
            let response = match parse_payload(&body) {
                Ok(_) => panic!("invalid id must fail"),
                Err(response) => response,
            };
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "field {field}");
        }

        assert_eq!(
            parse_path_id("0", "id").expect_err("zero").status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            parse_path_id("-1", "id").expect_err("negative").status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn save_mapping_verwendet_sqlite_id_und_insertet_unbekannte_id_nicht(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let db = dl_db::Db::open_creating(dir.path().join("reaction_roles.sqlite3"))?;
        db.bootstrap_schema().await?;

        let saved = db
            .write(|conn| save_mapping(conn, payload(None, 100, 500), 10))
            .await?
            .expect("created mapping");
        let saved_id = saved["id"].as_i64().expect("id");
        assert!(saved_id > 0);
        assert_ne!(saved_id, 99);

        let unknown = db
            .write(|conn| save_mapping(conn, payload(Some(99), 101, 501), 11))
            .await?;
        assert!(unknown.is_none());
        let unknown_count: i64 = db
            .read(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM reaction_role_mappings WHERE id = 99",
                    [],
                    |row| row.get(0),
                )
            })
            .await?;
        assert_eq!(unknown_count, 0);

        let updated = db
            .write(move |conn| save_mapping(conn, payload(Some(saved_id), 100, 777), 12))
            .await?
            .expect("updated mapping");
        assert_eq!(updated["id"].as_i64(), Some(saved_id));
        assert_eq!(updated["role_id"].as_u64(), Some(777));
        Ok(())
    }
}
