use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use dl_central_db::steam_web_api_ledger::{self, CallerClass, LedgerError, Reservation};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::serversync;

#[derive(Clone)]
struct ApiState {
    pool: PgPool,
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReserveRequest {
    caller: String,
    caller_class: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObserveRequest {
    reservation_id: i64,
    http_status: Option<i16>,
    retry_after: Option<String>,
}

pub fn router(pool: PgPool, token: Option<String>) -> Router {
    Router::new()
        .route("/steam-web-api/reserve", post(reserve))
        .route("/steam-web-api/observe", post(observe))
        .with_state(ApiState { pool, token })
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn error_response(status: StatusCode, error: &str) -> Response {
    json_response(status, json!({ "ok": false, "error": error }))
}

fn authorize(
    state: &ApiState,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, &'static str)> {
    if !peer.ip().is_loopback() {
        return Err((StatusCode::FORBIDDEN, "loopback_only"));
    }
    let Some(expected) = state.token.as_deref().filter(|value| !value.is_empty()) else {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "key_missing"));
    };
    let Some(presented) = headers
        .get(serversync::TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
    else {
        return Err((StatusCode::FORBIDDEN, "key_missing"));
    };
    if !serversync::constant_time_eq(presented, expected) {
        return Err((StatusCode::UNAUTHORIZED, "key_invalid"));
    }
    Ok(())
}

fn ledger_error(error: LedgerError) -> Response {
    match error {
        LedgerError::Database(error) => {
            tracing::error!(error = %error, "Steam Web API ledger database unavailable");
            error_response(StatusCode::SERVICE_UNAVAILABLE, "ledger_unavailable")
        }
        LedgerError::InvalidCaller | LedgerError::InvalidObservation => {
            error_response(StatusCode::BAD_REQUEST, "invalid_request")
        }
        LedgerError::NotFound => error_response(StatusCode::NOT_FOUND, "reservation_not_found"),
        LedgerError::ConflictingReport => {
            error_response(StatusCode::CONFLICT, "conflicting_report")
        }
    }
}

async fn reserve(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<ReserveRequest>,
) -> Response {
    if let Err((status, code)) = authorize(&state, peer, &headers) {
        return error_response(status, code);
    }
    let class = match request.caller_class.as_str() {
        "optional_patch" => CallerClass::OptionalPatch,
        "standard" => CallerClass::Standard,
        _ => return error_response(StatusCode::BAD_REQUEST, "invalid_caller_class"),
    };
    match steam_web_api_ledger::reserve(&state.pool, &request.caller, class).await {
        Ok(Reservation::Granted { id, reserved_at }) => json_response(
            StatusCode::OK,
            json!({ "ok": true, "granted": true, "reservation_id": id, "reserved_at": reserved_at.to_rfc3339() }),
        ),
        Ok(Reservation::Denied { reason, retry_at }) => json_response(
            StatusCode::OK,
            json!({ "ok": true, "granted": false, "reason": reason.as_str(), "retry_at": retry_at.to_rfc3339() }),
        ),
        Err(error) => ledger_error(error),
    }
}

async fn observe(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<ObserveRequest>,
) -> Response {
    if let Err((status, code)) = authorize(&state, peer, &headers) {
        return error_response(status, code);
    }
    match steam_web_api_ledger::observe(
        &state.pool,
        request.reservation_id,
        request.http_status,
        request.retry_after.as_deref(),
    )
    .await
    {
        Ok(observation) => json_response(
            StatusCode::OK,
            json!({
                "ok": true,
                "response_at": observation.response_at.to_rfc3339(),
                "cooldown_until": observation.cooldown_until.map(|time| time.to_rfc3339()),
                "duplicate": observation.duplicate,
            }),
        ),
        Err(error) => ledger_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn pool() -> PgPool {
        PgPool::connect_lazy("postgres://localhost/steam_ledger_unused").expect("lazy test pool")
    }

    async fn request(
        configured: Option<&str>,
        presented: Option<&str>,
        peer: &str,
        pool: PgPool,
    ) -> StatusCode {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/steam-web-api/reserve")
            .header("content-type", "application/json");
        if let Some(token) = presented {
            builder = builder.header(serversync::TOKEN_HEADER, token);
        }
        let mut request = builder
            .body(Body::from(
                r#"{"caller":"patchnotes","caller_class":"optional_patch"}"#,
            ))
            .expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().expect("peer")));
        router(pool, configured.map(str::to_string))
            .oneshot(request)
            .await
            .expect("response")
            .status()
    }

    #[tokio::test]
    async fn rejects_missing_invalid_and_non_loopback_key() {
        assert_eq!(
            request(Some("test-key"), None, "127.0.0.1:12345", pool()).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(None, Some("test-key"), "127.0.0.1:12345", pool()).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            request(Some("test-key"), Some("wrong"), "127.0.0.1:12345", pool()).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                Some("test-key"),
                Some("test-key"),
                "192.0.2.1:12345",
                pool()
            )
            .await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn rejects_database_failure_before_dispatch() {
        let pool = pool();
        pool.close().await;
        assert_eq!(
            request(Some("test-key"), Some("test-key"), "127.0.0.1:12345", pool).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
