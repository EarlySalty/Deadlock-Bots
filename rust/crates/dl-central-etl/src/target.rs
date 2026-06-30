use sqlx::{postgres::PgQueryResult, PgPool};

#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    #[error("ungueltiger Postgres-Identifier: {identifier}")]
    InvalidIdentifier { identifier: String },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct TargetWriter {
    pool: PgPool,
}

impl TargetWriter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn insert_text_value(
        &self,
        table: &str,
        column: &str,
        value: &str,
    ) -> Result<PgQueryResult, TargetError> {
        let table = quote_pg_path(table)?;
        let column = quote_pg_ident(column)?;
        let sql = format!("INSERT INTO {table} ({column}) VALUES ($1)");

        sqlx::query(&sql)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(TargetError::from)
    }

    pub async fn insert_default_row(&self, table: &str) -> Result<PgQueryResult, TargetError> {
        let table = quote_pg_path(table)?;
        let sql = format!("INSERT INTO {table} DEFAULT VALUES");

        sqlx::query(&sql)
            .execute(&self.pool)
            .await
            .map_err(TargetError::from)
    }
}

fn quote_pg_path(path: &str) -> Result<String, TargetError> {
    let mut parts = Vec::new();
    for part in path.split('.') {
        parts.push(quote_pg_ident(part)?);
    }

    if parts.is_empty() {
        return Err(TargetError::InvalidIdentifier {
            identifier: path.to_string(),
        });
    }

    Ok(parts.join("."))
}

fn quote_pg_ident(identifier: &str) -> Result<String, TargetError> {
    if identifier.trim().is_empty() {
        return Err(TargetError::InvalidIdentifier {
            identifier: identifier.to_string(),
        });
    }

    Ok(format!("\"{}\"", identifier.replace('"', "\"\"")))
}
