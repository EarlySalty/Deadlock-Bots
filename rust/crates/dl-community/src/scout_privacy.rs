//! Geordnete Datenschutzaufträge für die neuen Twitch-Scoutkopien.
//! UUID und Epoche bleiben bei Fehlern unverändert. Der bestehende Bridgeclient
//! liefert die Aufträge; der flüchtige HTTP-Zustand entscheidet keinen Erfolg.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;

use dl_bridges::twitch::TwitchApiClient;

const SUBJECT_SQL: &str = "scout-community:discord-privacy:v1:";

/// Aufruf ausschließlich unter dem Nutzerlock der lokalen Privacytransaktion.
pub async fn enqueue(
    tx: &mut Transaction<'_, Postgres>,
    discord_id: i64,
    action: &str,
) -> Result<(), sqlx::Error> {
    // Derselbe Nutzerwunsch erzeugt keine neue Epoche oder Consentgrenze.
    let same_action: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM community.scout_privacy_epochs
          WHERE subject_hash = sha256(convert_to($1 || $2::text, 'UTF8')) AND action = $3)",
    )
    .bind(SUBJECT_SQL)
    .bind(discord_id)
    .bind(action)
    .fetch_one(&mut **tx)
    .await?;
    if same_action {
        return Ok(());
    }
    let epoch: i64 = sqlx::query_scalar(
        "INSERT INTO community.scout_privacy_epochs(subject_hash, epoch, action)
         VALUES (sha256(convert_to($1 || $2::text, 'UTF8')), 1, $3)
         ON CONFLICT(subject_hash) DO UPDATE SET epoch = community.scout_privacy_epochs.epoch + 1,
            action = EXCLUDED.action RETURNING epoch",
    )
    .bind(SUBJECT_SQL)
    .bind(discord_id)
    .bind(action)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO community.scout_privacy_outbox(operation_id, discord_id, epoch, action, activity_since)
         VALUES (gen_random_uuid()::text, $1, $2, $3,
                 CASE WHEN $3 = 'consent' THEN clock_timestamp() ELSE NULL END)",
    ).bind(discord_id).bind(epoch).bind(action).execute(&mut **tx).await?;
    Ok(())
}

pub async fn current_epoch(
    tx: &mut Transaction<'_, Postgres>,
    discord_id: i64,
) -> Result<i64, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT epoch FROM community.scout_privacy_epochs
          WHERE subject_hash = sha256(convert_to($1 || $2::text, 'UTF8'))",
    )
    .bind(SUBJECT_SQL)
    .bind(discord_id)
    .fetch_optional(&mut **tx)
    .await?
    .unwrap_or(0))
}

/// Ein Mitglied wird während HTTP und Bestätigung unter seinem Nutzerlock
/// gehalten. So kann kein zweiter Zusteller eine spätere Einwilligung vorziehen.
pub async fn deliver_user(
    pool: &PgPool,
    client: &TwitchApiClient,
    discord_id: i64,
) -> Result<(), String> {
    loop {
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        crate::privacy::lock_user_privacy(&mut tx, discord_id)
            .await
            .map_err(|e| e.to_string())?;
        let operation: Option<(String, i64, String, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT operation_id, epoch, action, activity_since
               FROM community.scout_privacy_outbox WHERE discord_id = $1 ORDER BY epoch LIMIT 1",
        )
        .bind(discord_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        let Some((operation_id, epoch, action, activity_since)) = operation else {
            tx.commit().await.map_err(|e| e.to_string())?;
            return Ok(());
        };
        let mut payload = json!({ "discord_user_id": discord_id.to_string(), "operation_id": operation_id, "epoch": epoch });
        if let Some(since) = activity_since {
            payload["activity_since"] = json!(since.to_rfc3339());
        }
        let response = client
            .post_scout_privacy(&action, &payload)
            .await
            .map_err(|e| e.to_string())?;
        let response_since = match response.get("activity_since") {
            Some(Value::Null) => None,
            Some(Value::String(value)) => Some(
                DateTime::parse_from_rfc3339(value)
                    .map_err(|e| e.to_string())?
                    .with_timezone(&Utc),
            ),
            _ => return Err("Scout-Privacyantwort ohne gültige Einwilligungsgrenze".into()),
        };
        let bound = response["discord_user_id"].as_str() == Some(discord_id.to_string().as_str())
            && response["operation_id"].as_str() == Some(operation_id.as_str())
            && response["epoch"].as_i64() == Some(epoch)
            && matches!(response["status"].as_str(), Some("applied" | "replayed"))
            && response_since == activity_since;
        if !bound {
            return Err("Scout-Datenschutzantwort passt nicht zum gespeicherten Auftrag".into());
        }
        sqlx::query("DELETE FROM community.scout_privacy_outbox WHERE operation_id = $1")
            .bind(operation_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        // Jede Bestätigung einzeln committen. Ein späterer Fehler darf bereits
        // bestätigte Löschaufträge nicht vor eine neuere Consentepoche zurücksetzen.
        tx.commit().await.map_err(|e| e.to_string())?;
    }
}

pub async fn deliver_pending(pool: &PgPool, client: &TwitchApiClient) -> Result<(), String> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT discord_id FROM community.scout_privacy_outbox ORDER BY discord_id LIMIT 100",
    ).fetch_all(pool).await.map_err(|e| e.to_string())?;
    let mut failed = false;
    for id in ids {
        if let Err(error) = deliver_user(pool, client, id).await {
            tracing::warn!(%error, "Scout-Datenschutzauftrag bleibt zur Wiederholung gespeichert");
            failed = true;
        }
    }
    if failed {
        return Err("Scout-Datenschutzaufträge noch nicht vollständig bestätigt".into());
    }
    Ok(())
}

pub async fn export(client: &TwitchApiClient, discord_id: i64) -> Result<Value, String> {
    let response = client
        .export_scout_privacy(discord_id)
        .await
        .map_err(|e| e.to_string())?;
    if response["discord_user_id"].as_str() != Some(discord_id.to_string().as_str())
        || !response["suggestions"].is_array()
        || !response["candidates"].is_array()
        || response["epoch"].as_i64().is_none()
    {
        return Err("Scout-Exportantwort ist nicht an das Mitglied gebunden".into());
    }
    Ok(response)
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use axum::{
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct Remote {
        requests: Vec<Value>,
        epoch: i64,
        operation: Option<Value>,
        lose_first_response: bool,
        suggestions: Vec<Value>,
    }
    async fn operation(
        State(state): State<Arc<Mutex<Remote>>>,
        Path(action): Path<String>,
        headers: HeaderMap,
        Json(payload): Json<Value>,
    ) -> axum::response::Response {
        if headers
            .get("X-Internal-Token")
            .and_then(|v| v.to_str().ok())
            != Some("fixture-token")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let mut state = state.lock().expect("Testzustand");
        state.requests.push(payload.clone());
        let epoch = payload["epoch"].as_i64().expect("Epoche");
        let status = if epoch < state.epoch {
            "stale"
        } else if epoch == state.epoch {
            if state.operation.as_ref() != Some(&payload) {
                return StatusCode::CONFLICT.into_response();
            }
            "replayed"
        } else {
            state.epoch = epoch;
            state.operation = Some(payload.clone());
            if action == "erase" {
                state.suggestions.clear();
            }
            "applied"
        };
        if state.lose_first_response {
            state.lose_first_response = false;
            return StatusCode::BAD_GATEWAY.into_response();
        }
        Json(json!({"discord_user_id": payload["discord_user_id"], "operation_id": payload["operation_id"],
            "epoch": epoch, "status": status, "activity_since": payload.get("activity_since").cloned().unwrap_or(Value::Null)})).into_response()
    }
    async fn export_handler(
        State(state): State<Arc<Mutex<Remote>>>,
        headers: HeaderMap,
    ) -> axum::response::Response {
        if headers
            .get("X-Internal-Token")
            .and_then(|v| v.to_str().ok())
            != Some("fixture-token")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let state = state.lock().expect("Testzustand");
        Json(
            json!({"discord_user_id":"42", "epoch":state.epoch, "activity_since":null,
            "suggestions":state.suggestions, "candidates":[]}),
        )
        .into_response()
    }
    async fn server(
        state: Arc<Mutex<Remote>>,
    ) -> (Arc<TwitchApiClient>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Testlistener");
        let address = listener.local_addr().expect("Adresse");
        let router = Router::new()
            .route(
                "/internal/twitch/v1/scout/community-privacy/export",
                get(export_handler),
            )
            .route(
                "/internal/twitch/v1/scout/community-privacy/{action}",
                post(operation),
            )
            .with_state(state);
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.expect("Testserver");
        });
        let client = TwitchApiClient::try_new(
            format!("http://{address}"),
            "fixture-token",
            std::time::Duration::from_secs(2),
            false,
        )
        .expect("Client");
        (client, server)
    }

    #[tokio::test]
    async fn scout_privacy_verlorene_antwort_db_rollback_retry_und_alte_epoche() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("Echte Wegwerf-DB");
        let pool = db.pool();
        let remote = Arc::new(Mutex::new(Remote {
            lose_first_response: true,
            suggestions: vec![json!({"reason":"eigener Grund", "twitch_user_id":"111"})],
            ..Remote::default()
        }));
        let (client, server) = server(remote.clone()).await;
        assert_eq!(
            export(&client, 42).await.expect("Scout-Export")["suggestions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        crate::privacy::delete_user_data(pool, 42, "test".into(), Utc::now().timestamp())
            .await
            .expect("Lokale Erasure");
        crate::privacy::set_opt_in(pool, 42, Utc::now().timestamp())
            .await
            .expect("Neue Einwilligung");
        let original: Vec<(String, i64)> = sqlx::query_as(
            "SELECT operation_id, epoch FROM community.scout_privacy_outbox ORDER BY epoch",
        )
        .fetch_all(pool)
        .await
        .expect("Aufträge");
        assert_eq!(original.len(), 2);
        assert!(deliver_user(pool, &client, 42).await.is_err());
        assert!(remote.lock().unwrap().suggestions.is_empty());
        // Erfolgreiches Remote-Erase, aber zentraler Rollback beim Bestätigen.
        sqlx::raw_sql("CREATE FUNCTION community.reject_scout_ack() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'gezielter ACK-Rollback'; END $$;
          CREATE TRIGGER reject_scout_ack BEFORE DELETE ON community.scout_privacy_outbox FOR EACH ROW EXECUTE FUNCTION community.reject_scout_ack();")
            .execute(pool).await.expect("Rollbackfixture");
        assert!(deliver_user(pool, &client, 42).await.is_err());
        let after: Vec<(String, i64)> = sqlx::query_as(
            "SELECT operation_id, epoch FROM community.scout_privacy_outbox ORDER BY epoch",
        )
        .fetch_all(pool)
        .await
        .expect("Unveränderte Retryaufträge");
        assert_eq!(after, original);
        sqlx::query("DROP TRIGGER reject_scout_ack ON community.scout_privacy_outbox")
            .execute(pool)
            .await
            .expect("Fixture lösen");
        deliver_user(pool, &client, 42)
            .await
            .expect("Identische Erasereplays vor Consent");
        let requests = remote.lock().unwrap().requests.clone();
        assert_eq!(requests[0], requests[1]);
        assert_eq!(requests[1], requests[2]);
        assert_eq!(requests.last().unwrap()["epoch"], 2);
        let old = &requests[0];
        let stale = client
            .post_scout_privacy("erase", old)
            .await
            .expect("Verspätetes altes Erase");
        assert_eq!(stale["status"], "stale");
        assert_eq!(remote.lock().unwrap().epoch, 2);
        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM community.scout_privacy_outbox")
                .fetch_one(pool)
                .await
                .expect("Bestätigte Aufträge");
        assert_eq!(remaining, 0);
        crate::privacy::set_opt_in(pool, 42, Utc::now().timestamp())
            .await
            .expect("Idempotentes Optin");
        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM community.scout_privacy_outbox")
                .fetch_one(pool)
                .await
                .expect("Kein neues Consent");
        assert_eq!(remaining, 0);
        server.abort();
    }

    #[tokio::test]
    async fn scout_privacy_nicht_erreichbar_bleibt_retry_und_exportfehler() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("Echte Wegwerf-DB");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Port");
        let address = listener.local_addr().unwrap();
        drop(listener);
        let client = TwitchApiClient::try_new(
            format!("http://{address}"),
            "fixture-token",
            std::time::Duration::from_millis(100),
            false,
        )
        .unwrap();
        crate::privacy::delete_user_data(db.pool(), 42, "test".into(), Utc::now().timestamp())
            .await
            .expect("Lokale Erasure");
        assert!(deliver_user(db.pool(), &client, 42).await.is_err());
        assert!(export(&client, 42).await.is_err());
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM community.scout_privacy_outbox WHERE discord_id = 42",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(count, 1);
    }
}
