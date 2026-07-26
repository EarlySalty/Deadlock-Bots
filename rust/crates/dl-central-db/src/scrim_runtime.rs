use sqlx::{postgres::PgRow, PgPool, Postgres, Row, Transaction};

pub const SCRIM_TRANSITION_LOCK_A: i32 = 724_060_001;
pub const SCRIM_TRANSITION_LOCK_B: i32 = 724_060_002;
const SCRIM_RUNTIME_STATE_SQL: &str = "SELECT mode, operational_writer, epoch
                                        FROM scrim.runtime_control
                                       WHERE control_key = 'scrim_runtime'";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrimRuntimeState {
    pub mode: String,
    pub operational_writer: String,
    pub epoch: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ScrimRuntimeGateError {
    #[error("scrim.runtime_control nicht lesbar: {0}")]
    Read(#[source] sqlx::Error),
    #[error("scrim.runtime_control fehlt")]
    Missing,
    #[error("lokales Scrim-Schreiben ist bei {mode}/{operational_writer} nicht erlaubt")]
    Denied {
        mode: String,
        operational_writer: String,
    },
}

pub fn local_scrim_writes_allowed(mode: &str, operational_writer: &str) -> bool {
    mode == "legacy" && operational_writer == "dl-bots"
}

pub async fn load_scrim_runtime_state(
    pool: &PgPool,
) -> Result<ScrimRuntimeState, ScrimRuntimeGateError> {
    let row = sqlx::query(SCRIM_RUNTIME_STATE_SQL)
        .fetch_optional(pool)
        .await
        .map_err(ScrimRuntimeGateError::Read)?
        .ok_or(ScrimRuntimeGateError::Missing)?;

    Ok(scrim_runtime_state(row))
}

pub async fn require_local_scrim_write(
    pool: &PgPool,
    path: &str,
    action: &str,
) -> Result<ScrimRuntimeState, ScrimRuntimeGateError> {
    enforce_local_scrim_write(load_scrim_runtime_state(pool).await, path, action)
}

pub async fn require_local_scrim_write_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    path: &str,
    action: &str,
) -> Result<ScrimRuntimeState, ScrimRuntimeGateError> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1, $2)")
        .bind(SCRIM_TRANSITION_LOCK_A)
        .bind(SCRIM_TRANSITION_LOCK_B)
        .execute(&mut **tx)
        .await
        .map_err(ScrimRuntimeGateError::Read)?;
    let state = sqlx::query(SCRIM_RUNTIME_STATE_SQL)
        .fetch_optional(&mut **tx)
        .await
        .map_err(ScrimRuntimeGateError::Read)?
        .map(scrim_runtime_state)
        .ok_or(ScrimRuntimeGateError::Missing);
    enforce_local_scrim_write(state, path, action)
}

fn scrim_runtime_state(row: PgRow) -> ScrimRuntimeState {
    ScrimRuntimeState {
        mode: row.get("mode"),
        operational_writer: row.get("operational_writer"),
        epoch: row.get("epoch"),
    }
}

fn enforce_local_scrim_write(
    state: Result<ScrimRuntimeState, ScrimRuntimeGateError>,
    path: &str,
    action: &str,
) -> Result<ScrimRuntimeState, ScrimRuntimeGateError> {
    match state {
        Ok(state) if local_scrim_writes_allowed(&state.mode, &state.operational_writer) => {
            Ok(state)
        }
        Ok(state) => {
            tracing::warn!(
                path,
                route = path,
                mode = %state.mode,
                operational_writer = %state.operational_writer,
                action,
                method = action,
                "lokaler Scrim-Schreibzugriff abgelehnt"
            );
            Err(ScrimRuntimeGateError::Denied {
                mode: state.mode,
                operational_writer: state.operational_writer,
            })
        }
        Err(error) => {
            let (mode, operational_writer) = match &error {
                ScrimRuntimeGateError::Missing => ("<missing>", "<missing>"),
                ScrimRuntimeGateError::Read(_) => ("<unavailable>", "<unavailable>"),
                ScrimRuntimeGateError::Denied {
                    mode,
                    operational_writer,
                } => (mode.as_str(), operational_writer.as_str()),
            };
            tracing::warn!(
                path,
                route = path,
                mode,
                operational_writer,
                action,
                method = action,
                %error,
                "lokaler Scrim-Schreibzugriff abgelehnt"
            );
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn nur_legacy_dl_bots_erlaubt_lokale_scrim_schreibzugriffe() {
        assert!(local_scrim_writes_allowed("legacy", "dl-bots"));
        for (mode, writer) in [
            ("draining", "turniere"),
            ("turniere", "turniere"),
            ("legacy", "turniere"),
            ("turniere", "dl-bots"),
        ] {
            assert!(!local_scrim_writes_allowed(mode, writer));
        }
    }

    #[tokio::test]
    async fn laufender_schreibzugriff_serialisiert_runtime_umschaltung(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::testing::test_pool().await?;
        let pool = db.pool().clone();
        let mut write_tx = pool.begin().await?;
        require_local_scrim_write_in_transaction(
            &mut write_tx,
            "test::scrim_write",
            "scrim test write",
        )
        .await?;

        let transition_pool = pool.clone();
        let transition = tokio::spawn(async move {
            sqlx::query_as::<_, (bool, i64)>(
                "SELECT applied, current_epoch
                   FROM scrim.transition_runtime_control(
                        0, 'draining', 'turniere', '42', 'Test',
                        'runtime:gate-test', 'runtime:gate-test', '{}'::jsonb
                   )",
            )
            .fetch_one(&transition_pool)
            .await
        });

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS(
                         SELECT 1
                           FROM pg_stat_activity
                          WHERE datname = current_database()
                            AND query LIKE '%runtime:gate-test%'
                            AND wait_event_type = 'Lock'
                     )",
                )
                .fetch_one(&mut *write_tx)
                .await?;
                if waiting {
                    return Ok::<_, sqlx::Error>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await??;
        assert!(!transition.is_finished());

        write_tx.commit().await?;
        let switched = tokio::time::timeout(Duration::from_secs(2), transition).await???;
        assert_eq!(switched, (true, 1));
        Ok(())
    }
}
