use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::CentralDbError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreUser {
    pub discord_id: i64,
    pub username: Option<String>,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

pub async fn upsert_user(
    pool: &PgPool,
    discord_id: i64,
    username: Option<&str>,
    global_name: Option<&str>,
    avatar: Option<&str>,
) -> Result<(), CentralDbError> {
    sqlx::query!(
        r#"
        INSERT INTO core.users (discord_id, username, global_name, avatar)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (discord_id) DO UPDATE
        SET username = COALESCE(EXCLUDED.username, core.users.username),
            global_name = COALESCE(EXCLUDED.global_name, core.users.global_name),
            avatar = COALESCE(EXCLUDED.avatar, core.users.avatar),
            last_seen = now()
        "#,
        discord_id,
        username,
        global_name,
        avatar,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_user(pool: &PgPool, discord_id: i64) -> Result<Option<CoreUser>, CentralDbError> {
    let user = sqlx::query_as!(
        CoreUser,
        r#"
        SELECT discord_id, username, global_name, avatar, first_seen, last_seen
        FROM core.users
        WHERE discord_id = $1
        "#,
        discord_id,
    )
    .fetch_optional(pool)
    .await?;

    Ok(user)
}
