use sqlx::PgPool;

use crate::CentralDbError;

fn db_user_id(user_id: u64) -> Result<i64, CentralDbError> {
    i64::try_from(user_id)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CentralDbError::InvalidInput(format!("Discord-ID {user_id} ungueltig")))
}

pub async fn is_dm_opted_out(pool: &PgPool, user_id: u64) -> Result<bool, CentralDbError> {
    let user_id = db_user_id(user_id)?;
    let opted_out: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core.dm_optout WHERE user_id = $1)")
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    Ok(opted_out)
}

pub async fn set_dm_opted_out(
    pool: &PgPool,
    user_id: u64,
    source: &str,
) -> Result<(), CentralDbError> {
    let user_id = db_user_id(user_id)?;
    sqlx::query("SELECT core.set_dm_optout($1, $2)")
        .bind(user_id)
        .bind(source)
        .execute(pool)
        .await?;
    Ok(())
}
