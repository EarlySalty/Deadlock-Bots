use sqlx::{PgPool, Row};

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
    let row = sqlx::query(
        "SELECT mode, operational_writer, epoch
           FROM scrim.runtime_control
          WHERE control_key = 'scrim_runtime'",
    )
    .fetch_optional(pool)
    .await
    .map_err(ScrimRuntimeGateError::Read)?
    .ok_or(ScrimRuntimeGateError::Missing)?;

    Ok(ScrimRuntimeState {
        mode: row.get("mode"),
        operational_writer: row.get("operational_writer"),
        epoch: row.get("epoch"),
    })
}

pub async fn require_local_scrim_write(
    pool: &PgPool,
    path: &'static str,
    action: &'static str,
) -> Result<ScrimRuntimeState, ScrimRuntimeGateError> {
    match load_scrim_runtime_state(pool).await {
        Ok(state) if local_scrim_writes_allowed(&state.mode, &state.operational_writer) => {
            Ok(state)
        }
        Ok(state) => {
            tracing::warn!(
                path,
                mode = %state.mode,
                operational_writer = %state.operational_writer,
                action,
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
                mode,
                operational_writer,
                action,
                %error,
                "lokaler Scrim-Schreibzugriff abgelehnt"
            );
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
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
}
