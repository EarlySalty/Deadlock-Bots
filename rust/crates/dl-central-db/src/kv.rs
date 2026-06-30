use sqlx::PgPool;

use crate::CentralDbError;

pub async fn get(pool: &PgPool, ns: &str, key: &str) -> Result<Option<String>, CentralDbError> {
    let row = sqlx::query!(
        r#"
        SELECT v
        FROM bot.kv_store
        WHERE ns = $1
          AND k = $2
        "#,
        ns,
        key,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| row.v))
}

pub async fn set(pool: &PgPool, ns: &str, key: &str, value: &str) -> Result<(), CentralDbError> {
    sqlx::query!(
        r#"
        INSERT INTO bot.kv_store (ns, k, v)
        VALUES ($1, $2, $3)
        ON CONFLICT (ns, k) DO UPDATE
        SET v = EXCLUDED.v
        "#,
        ns,
        key,
        value,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn delete(pool: &PgPool, ns: &str, key: &str) -> Result<(), CentralDbError> {
    sqlx::query!(
        r#"
        DELETE FROM bot.kv_store
        WHERE ns = $1
          AND k = $2
        "#,
        ns,
        key,
    )
    .execute(pool)
    .await?;

    Ok(())
}
