//! Dashboard-CRUD für generische Reaction-Rollen.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{DateTime, Utc};
use dl_community::reaction_roles::canonical_emoji_input;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};

use crate::db::{utc_to_unix, DashboardDbError, DashboardDbResult};
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

struct MappingRow {
    id: i64,
    guild_id: i64,
    source_channel_id: i64,
    message_id: i64,
    emoji: String,
    role_id: i64,
    dm_enabled: bool,
    dm_text: Option<String>,
    remove_on_unreact: bool,
    backfill_pending: bool,
    active: bool,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

fn mapping_json(row: MappingRow) -> Value {
    json!({
        "id": row.id,
        "guild_id": row.guild_id,
        "source_channel_id": row.source_channel_id,
        "message_id": row.message_id,
        "emoji": row.emoji,
        "role_id": row.role_id,
        "dm_enabled": row.dm_enabled,
        "dm_text": row.dm_text,
        "remove_on_unreact": row.remove_on_unreact,
        "backfill_pending": row.backfill_pending,
        "active": row.active,
        "created_at": utc_to_unix(row.created_at),
        "updated_at": utc_to_unix(row.updated_at),
    })
}

async fn load_mapping_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
) -> DashboardDbResult<Option<Value>> {
    let row = sqlx::query!(
        r#"
        SELECT id, guild_id, source_channel_id, message_id, emoji, role_id,
               dm_enabled AS "dm_enabled!", dm_text,
               remove_on_unreact AS "remove_on_unreact!",
               backfill_pending AS "backfill_pending!",
               active AS "active!", created_at, updated_at
          FROM bot.reaction_role_mappings
         WHERE id = $1
        "#,
        id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| {
        mapping_json(MappingRow {
            id: row.id,
            guild_id: row.guild_id,
            source_channel_id: row.source_channel_id,
            message_id: row.message_id,
            emoji: row.emoji,
            role_id: row.role_id,
            dm_enabled: row.dm_enabled,
            dm_text: row.dm_text,
            remove_on_unreact: row.remove_on_unreact,
            backfill_pending: row.backfill_pending,
            active: row.active,
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
        })
    }))
}

async fn save_mapping(
    pool: &PgPool,
    payload: ReactionRolePayload,
    ts: DateTime<Utc>,
) -> DashboardDbResult<Option<Value>> {
    let mut tx = pool.begin().await?;
    let saved_id = if let Some(id) = payload.id {
        let exists = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM bot.reaction_role_mappings WHERE id = $1
            ) AS "exists!"
            "#,
            id,
        )
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            tx.commit().await?;
            return Ok(None);
        }
        sqlx::query!(
            r#"
            UPDATE bot.reaction_role_mappings
               SET guild_id = $1,
                   source_channel_id = $2,
                   message_id = $3,
                   emoji = $4,
                   role_id = $5,
                   dm_enabled = $6,
                   dm_text = $7,
                   remove_on_unreact = $8,
                   backfill_pending = $9,
                   active = TRUE,
                   updated_at = $10
             WHERE id = $11
            "#,
            payload.guild_id as i64,
            payload.source_channel_id as i64,
            payload.message_id as i64,
            payload.emoji,
            payload.role_id as i64,
            payload.dm_enabled,
            payload.dm_text,
            payload.remove_on_unreact,
            payload.backfill,
            ts,
            id,
        )
        .execute(&mut *tx)
        .await?;
        id
    } else {
        let row = sqlx::query!(
            r#"
            INSERT INTO bot.reaction_role_mappings(
                guild_id, source_channel_id, message_id, emoji, role_id,
                dm_enabled, dm_text, remove_on_unreact, backfill_pending,
                active, created_at, updated_at
            )
            VALUES($1, $2, $3, $4, $5, $6, $7, $8, $9, TRUE, $10, $10)
             ON CONFLICT(message_id, emoji) DO UPDATE SET
                guild_id = EXCLUDED.guild_id,
                source_channel_id = EXCLUDED.source_channel_id,
                role_id = EXCLUDED.role_id,
                dm_enabled = EXCLUDED.dm_enabled,
                dm_text = EXCLUDED.dm_text,
                remove_on_unreact = EXCLUDED.remove_on_unreact,
                backfill_pending = EXCLUDED.backfill_pending,
                active = TRUE,
                updated_at = EXCLUDED.updated_at
            RETURNING id
            "#,
            payload.guild_id as i64,
            payload.source_channel_id as i64,
            payload.message_id as i64,
            payload.emoji,
            payload.role_id as i64,
            payload.dm_enabled,
            payload.dm_text,
            payload.remove_on_unreact,
            payload.backfill,
            ts,
        )
        .fetch_one(&mut *tx)
        .await?;
        row.id
    };
    let saved = load_mapping_tx(&mut tx, saved_id).await?;
    tx.commit().await?;
    Ok(saved)
}

fn is_unique_violation(err: &DashboardDbError) -> bool {
    match err {
        DashboardDbError::Sqlx(sqlx::Error::Database(db)) => db.code().as_deref() == Some("23505"),
        _ => false,
    }
}

pub async fn reaction_roles(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
        return resp;
    }
    let result = sqlx::query!(
        r#"
        SELECT id, guild_id, source_channel_id, message_id, emoji, role_id,
               dm_enabled AS "dm_enabled!", dm_text,
               remove_on_unreact AS "remove_on_unreact!",
               backfill_pending AS "backfill_pending!",
               active AS "active!", created_at, updated_at
          FROM bot.reaction_role_mappings
         ORDER BY id DESC
        "#
    )
    .fetch_all(app.pool())
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|row| {
                mapping_json(MappingRow {
                    id: row.id,
                    guild_id: row.guild_id,
                    source_channel_id: row.source_channel_id,
                    message_id: row.message_id,
                    emoji: row.emoji,
                    role_id: row.role_id,
                    dm_enabled: row.dm_enabled,
                    dm_text: row.dm_text,
                    remove_on_unreact: row.remove_on_unreact,
                    backfill_pending: row.backfill_pending,
                    active: row.active,
                    created_at: Some(row.created_at),
                    updated_at: Some(row.updated_at),
                })
            })
            .collect::<Vec<_>>()
    });
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
    let session = match app.guard_mutate(&headers, true).await {
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
    let ts = chrono::Utc::now();
    let result = save_mapping(app.pool(), payload, ts).await;

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
        Err(err) if is_unique_violation(&err) => {
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
    let session = match app.guard_mutate(&headers, true).await {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let id = match parse_path_id(&id_raw, "id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let result: DashboardDbResult<u64> = async {
        let mut tx = app.pool().begin().await?;
        sqlx::query!(
            r#"
            DELETE FROM bot.reaction_role_dm_log WHERE mapping_id = $1
            "#,
            id,
        )
        .execute(&mut *tx)
        .await?;
        let deleted = sqlx::query!(
            r#"
            DELETE FROM bot.reaction_role_mappings WHERE id = $1
            "#,
            id,
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

#[cfg(all(test, feature = "testing"))]
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
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();

        let saved = save_mapping(pool, payload(None, 100, 500), Utc::now())
            .await?
            .expect("created mapping");
        let saved_id = saved["id"].as_i64().expect("id");
        assert!(saved_id > 0);
        assert_ne!(saved_id, 99);

        let unknown = save_mapping(pool, payload(Some(99), 101, 501), Utc::now()).await?;
        assert!(unknown.is_none());
        let unknown_count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM bot.reaction_role_mappings
             WHERE id = 99
            "#
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(unknown_count, 0);

        let updated = save_mapping(pool, payload(Some(saved_id), 100, 777), Utc::now())
            .await?
            .expect("updated mapping");
        assert_eq!(updated["id"].as_i64(), Some(saved_id));
        assert_eq!(updated["role_id"].as_u64(), Some(777));
        Ok(())
    }

    #[tokio::test]
    async fn save_mapping_meldet_unique_konflikt_beim_id_update(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let first = save_mapping(pool, payload(None, 100, 500), Utc::now())
            .await?
            .expect("first");
        let second = save_mapping(pool, payload(None, 101, 501), Utc::now())
            .await?
            .expect("second");

        let err = save_mapping(pool, payload(second["id"].as_i64(), 100, 777), Utc::now())
            .await
            .expect_err("unique conflict");

        assert!(is_unique_violation(&err));
        assert_eq!(first["message_id"].as_u64(), Some(100));
        Ok(())
    }
}
