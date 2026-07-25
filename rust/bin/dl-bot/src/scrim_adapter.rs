//! Consolidation contracts are intentionally compiled before their live cutover.

use std::{
    fmt,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use dl_ai::ChatProvider;
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler};
use dl_squads::lagebild::CorrectionActor;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tokio::sync::RwLock;

pub const COMPONENTS_V2: u64 = 32_768;
const INTERNAL_TOKEN_HEADER: &str = "X-Internal-Token";
const SCRIMREQ_PREFIX: &str = "scrimreq:v1:";
const RUNTIME_CACHE_TTL: Duration = Duration::from_secs(2);
const RELAY_TIMEOUT: Duration = Duration::from_secs(3);
const RELAY_LEASE_OWNER: &str = "dlbots:scrim_adapter";
const RELAY_EVENT_SOURCE: &str = "dl_bots";
const RELAY_COMMAND_SCOPE: &str = "scrim_relay";
const RELAY_LEASE_SECONDS: i64 = 30;
const LAGEBILD_LEASE_OWNER: &str = "dlbots:lagebild_api";
const LAGEBILD_REFRESH_SCOPE: &str = "lagebild_refresh";
const LAGEBILD_CORRECTION_SCOPE: &str = "lagebild_correction";
const LAGEBILD_LEASE_SECONDS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionRoute {
    LegacyMutation,
    RelayToTurnier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrimRuntimeState {
    pub mode: String,
    pub operational_writer: String,
    pub epoch: i64,
}

#[derive(Debug, Clone)]
struct CachedRuntimeRoute {
    route: InteractionRoute,
    loaded_at: Instant,
}

#[derive(Clone)]
pub struct ScrimRuntimeGate {
    pool: PgPool,
    ttl: Duration,
    cache: Arc<RwLock<Option<CachedRuntimeRoute>>>,
}

impl ScrimRuntimeGate {
    pub fn new(pool: PgPool, ttl: Duration) -> Self {
        Self {
            pool,
            ttl,
            cache: Arc::new(RwLock::new(None)),
        }
    }

    pub fn with_default_ttl(pool: PgPool) -> Self {
        Self::new(pool, RUNTIME_CACHE_TTL)
    }

    pub async fn interaction_route(&self) -> Result<InteractionRoute, String> {
        Ok(self.load_fail_safe().await?.route)
    }

    async fn load_fail_safe(&self) -> Result<CachedRuntimeRoute, String> {
        if let Some(cached) = self.cache.read().await.as_ref() {
            if cached.route == InteractionRoute::RelayToTurnier
                && cached.loaded_at.elapsed() <= self.ttl
            {
                return Ok(cached.clone());
            }
        }

        let fresh = self.load_fresh().await?;
        if fresh.route == InteractionRoute::RelayToTurnier {
            *self.cache.write().await = Some(fresh.clone());
        } else {
            *self.cache.write().await = None;
        }
        Ok(fresh)
    }

    async fn load_fresh(&self) -> Result<CachedRuntimeRoute, String> {
        let row = sqlx::query(
            r#"
            SELECT mode, operational_writer, epoch
              FROM scrim.runtime_control
             WHERE control_key = 'scrim_runtime'
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| format!("scrim.runtime_control nicht lesbar: {error}"))?
        .ok_or_else(|| "scrim.runtime_control fehlt".to_string())?;

        let state = ScrimRuntimeState {
            mode: row.get("mode"),
            operational_writer: row.get("operational_writer"),
            epoch: row.get("epoch"),
        };
        let route = interaction_route_for_runtime(&state.mode, &state.operational_writer)
            .ok_or_else(|| {
                format!(
                    "ungueltiger Scrim-Runtime-State: {}/{}",
                    state.mode, state.operational_writer
                )
            })?;
        let cached = CachedRuntimeRoute {
            route,
            loaded_at: Instant::now(),
        };
        Ok(cached)
    }
}

pub fn interaction_route_for_runtime(
    mode: &str,
    operational_writer: &str,
) -> Option<InteractionRoute> {
    match (mode, operational_writer) {
        ("legacy", "dl-bots") => Some(InteractionRoute::LegacyMutation),
        ("draining" | "turniere", "turniere") => Some(InteractionRoute::RelayToTurnier),
        _ => None,
    }
}

pub fn legacy_driver_enabled_for_runtime(mode: &str, operational_writer: &str) -> bool {
    interaction_route_for_runtime(mode, operational_writer)
        == Some(InteractionRoute::LegacyMutation)
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchRequestAction {
    Slot,
    None,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MatchRequestResponseRequest {
    pub schema_version: String,
    pub event: String,
    pub idempotency: String,
    pub action: MatchRequestAction,
    pub request: String,
    pub team: String,
    pub slot: Option<u32>,
    pub interaction: String,
    pub guild: String,
    pub channel: String,
    pub message: Option<String>,
    pub actor: String,
    pub actor_role_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActionReceipt {
    pub accepted: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayErrorClass {
    Retryable,
    Terminal,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RelayClientError {
    MissingToken,
    Transport(String),
    Http { status: StatusCode, message: String },
    MalformedSuccess(String),
}

impl RelayClientError {
    fn classification(&self) -> RelayErrorClass {
        match self {
            Self::Transport(_) => RelayErrorClass::Retryable,
            Self::Http { status, .. } if relay_status_retryable(*status) => {
                RelayErrorClass::Retryable
            }
            Self::MalformedSuccess(_) => RelayErrorClass::Uncertain,
            Self::MissingToken | Self::Http { .. } => RelayErrorClass::Terminal,
        }
    }

    fn error_code(&self) -> &'static str {
        match self {
            Self::MissingToken => "err_relay_auth_config",
            Self::Transport(_) => "err_relay_transport",
            Self::Http { status, .. } if relay_status_retryable(*status) => {
                "err_relay_http_retryable"
            }
            Self::Http { status, .. } if *status == StatusCode::UNAUTHORIZED => {
                "err_relay_auth_failed"
            }
            Self::Http { .. } => "err_relay_http_terminal",
            Self::MalformedSuccess(_) => "err_relay_response_uncertain",
        }
    }

    fn user_receipt(&self) -> ActionReceipt {
        ActionReceipt {
            accepted: false,
            message: self.user_message(),
        }
    }

    fn user_message(&self) -> String {
        match self {
            Self::MissingToken => {
                "Internes Turnier-Relay ist nicht konfiguriert. Bitte Support pruefen."
                    .to_string()
            }
            Self::Http { status, message: _ } if *status == StatusCode::UNAUTHORIZED => {
                format!(
                    "Turnierplanung hat das interne Relay nicht autorisiert (HTTP {}). Bitte Support pruefen.",
                    status.as_u16()
                )
            }
            Self::Http { status, message } => {
                if message.trim().is_empty() {
                    format!(
                        "Turnierplanung hat die Antwort nicht angenommen (HTTP {}).",
                        status.as_u16()
                    )
                } else {
                    format!(
                        "Turnierplanung hat die Antwort nicht angenommen (HTTP {}): {}.",
                        status.as_u16(),
                        message.trim().trim_end_matches('.')
                    )
                }
            }
            Self::MalformedSuccess(_) => "Turnierplanung hat geantwortet, aber die Antwort war nicht eindeutig lesbar. Die Antwort wird nicht automatisch erneut gesendet. Bitte Support pruefen.".to_string(),
            Self::Transport(_) => {
                "Terminantwort konnte nicht an die Turnierplanung weitergeleitet werden."
                    .to_string()
            }
        }
    }
}

impl fmt::Display for RelayClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingToken => write!(f, "TURNIER_INTERNAL_API_TOKEN fehlt"),
            Self::Transport(error) => write!(f, "Turnier-Scrim-Relay nicht erreichbar: {error}"),
            Self::Http { status, message } => {
                write!(f, "Turnier-Scrim-Relay HTTP {}: {message}", status.as_u16())
            }
            Self::MalformedSuccess(error) => {
                write!(f, "Turnier-Scrim-Relay-Antwort unklar: {error}")
            }
        }
    }
}

impl std::error::Error for RelayClientError {}

fn relay_status_retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn relay_error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            ["message", "error", "detail"]
                .into_iter()
                .find_map(|key| value.get(key).and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn match_request_response_request(
    interaction: BridgeInteraction,
) -> Result<MatchRequestResponseRequest, String> {
    let custom_id = interaction.custom_id;
    let rest = custom_id
        .strip_prefix(SCRIMREQ_PREFIX)
        .ok_or_else(|| "Unbekannte Scrim-Terminaktion".to_string())?;
    let mut parts = rest.split(':');
    let kind = parts
        .next()
        .ok_or_else(|| "Scrim-Aktion fehlt".to_string())?;
    let request_id = parts
        .next()
        .filter(|value| value.parse::<u64>().is_ok())
        .ok_or_else(|| "Scrim-Request-ID ungültig".to_string())?;
    let team_id = parts
        .next()
        .filter(|value| value.parse::<u64>().is_ok())
        .ok_or_else(|| "Scrim-Team-ID ungültig".to_string())?;
    let (action, slot_index) = match kind {
        "slot" => (
            MatchRequestAction::Slot,
            Some(
                parts
                    .next()
                    .and_then(|value| value.parse::<u32>().ok())
                    .ok_or_else(|| "Scrim-Slot ungültig".to_string())?,
            ),
        ),
        "none" => (MatchRequestAction::None, None),
        _ => return Err("Unbekannte Scrim-Terminaktion".to_string()),
    };
    if parts.next().is_some() || interaction.interaction_id == 0 {
        return Err("Scrim-Terminaktion ist unvollständig".to_string());
    }
    let event_id = format!("scrimreq:v1:interaction:{}", interaction.interaction_id);
    Ok(MatchRequestResponseRequest {
        schema_version: "turnier-scrim-match-request-response:v1".to_string(),
        event: event_id.clone(),
        idempotency: event_id,
        action,
        request: request_id.to_string(),
        team: team_id.to_string(),
        slot: slot_index,
        interaction: interaction.interaction_id.to_string(),
        guild: interaction.guild_id.to_string(),
        channel: interaction.channel_id.to_string(),
        message: interaction.message_id.map(|value| value.to_string()),
        actor: interaction.user_id.to_string(),
        actor_role_ids: interaction
            .role_ids
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
    })
}

#[derive(Clone)]
pub struct TurnierScrimClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl TurnierScrimClient {
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(RELAY_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .expect("statischer Turnier-Scrim-HTTP-Client"),
            base_url: lookup("TURNIER_INTERNAL_API_BASE_URL")
                .unwrap_or_else(|| "http://127.0.0.1:8900".to_string())
                .trim_end_matches('/')
                .to_string(),
            token: lookup("TURNIER_INTERNAL_API_TOKEN").unwrap_or_default(),
        }
    }

    async fn relay(
        &self,
        request: &MatchRequestResponseRequest,
    ) -> Result<ActionReceipt, RelayClientError> {
        if self.token.trim().is_empty() {
            return Err(RelayClientError::MissingToken);
        }
        let response = self
            .http
            .post(format!(
                "{}/internal/turnier/v1/scrims/interactions/match-request-response",
                self.base_url
            ))
            .header(INTERNAL_TOKEN_HEADER, &self.token)
            .header("X-Request-Id", &request.event)
            .header("Idempotency-Key", &request.idempotency)
            .json(request)
            .send()
            .await
            .map_err(|error| RelayClientError::Transport(error.to_string()))?;
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            if status.is_success() {
                RelayClientError::MalformedSuccess(format!("Antwortkoerper unlesbar: {error}"))
            } else {
                RelayClientError::Http {
                    status,
                    message: format!("Antwortkoerper unlesbar: {error}"),
                }
            }
        })?;
        if !status.is_success() {
            return Err(RelayClientError::Http {
                status,
                message: relay_error_message(&body),
            });
        }
        let receipt = serde_json::from_str::<ActionReceipt>(&body)
            .map_err(|error| RelayClientError::MalformedSuccess(error.to_string()))?;
        if receipt.message.trim().is_empty() {
            return Err(RelayClientError::MalformedSuccess(
                "Nachricht fehlt".to_string(),
            ));
        }
        Ok(receipt)
    }
}

pub struct RelayInteractionHandler {
    client: TurnierScrimClient,
    pool: PgPool,
}

impl RelayInteractionHandler {
    pub fn new(client: TurnierScrimClient, pool: PgPool) -> Self {
        Self { client, pool }
    }
}

#[async_trait::async_trait]
impl InteractionHandler for RelayInteractionHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let request = match match_request_response_request(interaction) {
            Ok(request) => request,
            Err(error) => return BridgeReply::ephemeral_text(error),
        };
        match relay_match_request_response(&self.pool, &self.client, &request).await {
            Ok(receipt) => BridgeReply::ephemeral_text(receipt.message),
            Err(error) => {
                tracing::warn!(event_id = %request.event, actor_id = %request.actor, %error, "Scrim-Interaction an Turnier nicht weitergeleitet");
                BridgeReply::ephemeral_text(
                    "Terminantwort konnte nicht an die Turnierplanung weitergeleitet werden.",
                )
            }
        }
    }
}

pub fn relay_handler(
    pool: PgPool,
    lookup: impl Fn(&str) -> Option<String>,
) -> Arc<dyn InteractionHandler> {
    Arc::new(RelayInteractionHandler::new(
        TurnierScrimClient::from_lookup(lookup),
        pool,
    )) as Arc<dyn InteractionHandler>
}

enum RelayCommandState {
    New,
    Replay(ActionReceipt),
    Processing,
    PayloadConflict(ActionReceipt),
}

async fn relay_match_request_response(
    pool: &PgPool,
    client: &TurnierScrimClient,
    request: &MatchRequestResponseRequest,
) -> Result<ActionReceipt, String> {
    match begin_relay_command(pool, request).await? {
        RelayCommandState::Replay(receipt) => return Ok(receipt),
        RelayCommandState::PayloadConflict(receipt) => return Ok(receipt),
        RelayCommandState::Processing => {
            return Ok(ActionReceipt {
                accepted: false,
                message: "Terminantwort wird bereits verarbeitet.".to_string(),
            })
        }
        RelayCommandState::New => {}
    }

    if let Err(error) = validate_match_request_response_participant(pool, request).await {
        let receipt = ActionReceipt {
            accepted: false,
            message: error,
        };
        finish_relay_command(pool, request, &receipt, Some("err_validation_failed")).await?;
        return Ok(receipt);
    }

    match client.relay(request).await {
        Ok(receipt) => {
            finish_relay_command(pool, request, &receipt, None).await?;
            Ok(receipt)
        }
        Err(error) => match error.classification() {
            RelayErrorClass::Retryable => {
                mark_relay_command_retry(pool, request, error.error_code(), &error.to_string())
                    .await?;
                Err(error.to_string())
            }
            RelayErrorClass::Terminal => {
                let receipt = error.user_receipt();
                finish_relay_command(pool, request, &receipt, Some(error.error_code())).await?;
                tracing::warn!(
                    event_id = %request.event,
                    error_class = ?error.classification(),
                    error_code = error.error_code(),
                    error = %error,
                    "Scrim-Relay terminal fehlgeschlagen"
                );
                Ok(receipt)
            }
            RelayErrorClass::Uncertain => {
                let receipt = error.user_receipt();
                finish_relay_command_state(
                    pool,
                    request,
                    &receipt,
                    "uncertain",
                    "uncertain",
                    Some(error.error_code()),
                )
                .await?;
                tracing::warn!(
                    event_id = %request.event,
                    error_code = error.error_code(),
                    error = %error,
                    "Scrim-Relay-Antwort bleibt ohne erneute Zustellung unklar"
                );
                Ok(receipt)
            }
        },
    }
}

async fn begin_relay_command(
    pool: &PgPool,
    request: &MatchRequestResponseRequest,
) -> Result<RelayCommandState, String> {
    let payload = serde_json::to_value(request).map_err(|error| error.to_string())?;
    let payload_hash = sha256_json(&payload);
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| format!("Scrim-Relay-Transaktion konnte nicht starten: {error}"))?;

    sqlx::query(
        r#"
        INSERT INTO scrim.inbox_events(
            event_source, source_event_id, idempotency_key, payload_hash, payload,
            state, lease_owner, lease_until, attempts, remote_system, remote_task_id
        )
        VALUES($1, $2, $3, $4, $5::jsonb, 'processing', $6, now() + ($7 || ' seconds')::interval, 1, 'turniere', $3)
        ON CONFLICT (event_source, source_event_id) WHERE source_event_id IS NOT NULL DO NOTHING
        "#,
    )
    .bind(RELAY_EVENT_SOURCE)
    .bind(&request.event)
    .bind(&request.idempotency)
    .bind(&payload_hash)
    .bind(&payload)
    .bind(RELAY_LEASE_OWNER)
    .bind(RELAY_LEASE_SECONDS)
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("Scrim-Relay-Ingress konnte nicht gespeichert werden: {error}"))?;

    let inserted = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO scrim.command_receipts(
            command_scope, idempotency_key, payload_hash, payload, state,
            lease_owner, lease_until, attempts, remote_system, remote_task_id
        )
        VALUES($1, $2, $3, $4::jsonb, 'processing', $5, now() + ($6 || ' seconds')::interval, 1, 'turniere', $2)
        ON CONFLICT (command_scope, idempotency_key, idempotency_generation) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(RELAY_COMMAND_SCOPE)
    .bind(&request.idempotency)
    .bind(&payload_hash)
    .bind(&payload)
    .bind(RELAY_LEASE_OWNER)
    .bind(RELAY_LEASE_SECONDS)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| format!("Scrim-Relay-Receipt konnte nicht gespeichert werden: {error}"))?;

    let state = if inserted.is_some() {
        RelayCommandState::New
    } else {
        let row = sqlx::query(
            r#"
            SELECT id, state, payload_hash, result_payload, lease_until, next_attempt_at
              FROM scrim.command_receipts
             WHERE command_scope = $1
               AND idempotency_key = $2
             ORDER BY idempotency_generation DESC
             LIMIT 1
             FOR UPDATE
            "#,
        )
        .bind(RELAY_COMMAND_SCOPE)
        .bind(&request.idempotency)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| format!("Scrim-Relay-Receipt konnte nicht gelesen werden: {error}"))?;
        let existing_payload_hash = row.get::<Vec<u8>, _>("payload_hash");
        if existing_payload_hash.as_slice() != payload_hash.as_slice() {
            tx.rollback().await.map_err(|error| {
                format!("Scrim-Relay-Receipt-Konflikt konnte nicht zurueckgerollt werden: {error}")
            })?;
            tracing::warn!(
                event_id = %request.event,
                idempotency_key = %request.idempotency,
                existing_payload_hash = %hex::encode(&existing_payload_hash),
                new_payload_hash = %hex::encode(&payload_hash),
                "Scrim-Relay-Idempotency-Konflikt ohne erneute Zustellung"
            );
            return Ok(RelayCommandState::PayloadConflict(
                relay_payload_conflict_receipt(),
            ));
        }
        let state = row.get::<String, _>("state");
        if let Some(payload) = row.get::<Option<Value>, _>("result_payload") {
            if is_terminal_command_state(&state) {
                let receipt = serde_json::from_value::<ActionReceipt>(payload)
                    .map_err(|error| format!("Scrim-Relay-Replay-Receipt ist unlesbar: {error}"))?;
                RelayCommandState::Replay(receipt)
            } else {
                reclaim_or_wait_relay_command(&mut tx, row.get("id"), &state).await?
            }
        } else {
            reclaim_or_wait_relay_command(&mut tx, row.get("id"), &state).await?
        }
    };
    tx.commit()
        .await
        .map_err(|error| format!("Scrim-Relay-Receipt konnte nicht bestaetigt werden: {error}"))?;
    Ok(state)
}

fn relay_payload_conflict_receipt() -> ActionReceipt {
    ActionReceipt {
        accepted: false,
        message: "Diese Terminantwort passt nicht zur bereits gespeicherten Anfrage. Bitte die urspruengliche Nachricht erneut verwenden."
            .to_string(),
    }
}

async fn reclaim_or_wait_relay_command(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    state: &str,
) -> Result<RelayCommandState, String> {
    let reclaimed = sqlx::query_scalar::<_, i64>(
        r#"
        UPDATE scrim.command_receipts
           SET state = 'processing',
               lease_owner = $2,
               lease_until = now() + ($3 || ' seconds')::interval,
               attempts = attempts + 1,
               next_attempt_at = NULL,
               updated_at = now()
         WHERE id = $1
           AND (
                state = 'retry'
                OR (state = 'processing' AND lease_until <= now())
           )
           AND (next_attempt_at IS NULL OR next_attempt_at <= now())
         RETURNING id
        "#,
    )
    .bind(id)
    .bind(RELAY_LEASE_OWNER)
    .bind(RELAY_LEASE_SECONDS)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| format!("Scrim-Relay-Receipt konnte nicht reclaimed werden: {error}"))?;
    if reclaimed.is_none() {
        return Ok(RelayCommandState::Processing);
    }
    sqlx::query(
        r#"
        UPDATE scrim.inbox_events
           SET state = 'processing',
               lease_owner = $2,
               lease_until = now() + ($3 || ' seconds')::interval,
               attempts = attempts + 1,
               next_attempt_at = NULL,
               updated_at = now()
         WHERE event_source = $4
           AND idempotency_key = (
               SELECT idempotency_key FROM scrim.command_receipts WHERE id = $1
           )
           AND state IN ('processing', 'retry')
        "#,
    )
    .bind(id)
    .bind(RELAY_LEASE_OWNER)
    .bind(RELAY_LEASE_SECONDS)
    .bind(RELAY_EVENT_SOURCE)
    .execute(&mut **tx)
    .await
    .map_err(|error| format!("Scrim-Relay-Ingress konnte nicht reclaimed werden: {error}"))?;
    tracing::warn!(
        receipt_id = id,
        previous_state = state,
        "Scrim-Relay-Receipt wird erneut zugestellt"
    );
    Ok(RelayCommandState::New)
}

async fn finish_relay_command(
    pool: &PgPool,
    request: &MatchRequestResponseRequest,
    receipt: &ActionReceipt,
    error_code: Option<&'static str>,
) -> Result<(), String> {
    let error_code = error_code.or_else(|| (!receipt.accepted).then_some("err_relay_rejected"));
    let command_state = if receipt.accepted && error_code.is_none() {
        "completed"
    } else {
        "failed"
    };
    let inbox_state = if receipt.accepted && error_code.is_none() {
        "processed"
    } else {
        "ignored"
    };
    finish_relay_command_state(
        pool,
        request,
        receipt,
        command_state,
        inbox_state,
        error_code,
    )
    .await
}

async fn finish_relay_command_state(
    pool: &PgPool,
    request: &MatchRequestResponseRequest,
    receipt: &ActionReceipt,
    command_state: &'static str,
    inbox_state: &'static str,
    error_code: Option<&'static str>,
) -> Result<(), String> {
    let result_payload = serde_json::to_value(receipt).map_err(|error| error.to_string())?;
    let error_hash = error_code.map(|value| sha256_bytes(value.as_bytes()));
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| format!("Scrim-Relay-Finish konnte nicht starten: {error}"))?;
    sqlx::query(
        r#"
        UPDATE scrim.command_receipts
           SET state = $3,
               lease_owner = NULL,
               lease_until = NULL,
               result_payload = $4::jsonb,
               last_error_code = $5,
               last_error_hash = $6,
               updated_at = now(),
               completed_at = now()
         WHERE command_scope = $1
           AND idempotency_key = $2
           AND state = 'processing'
        "#,
    )
    .bind(RELAY_COMMAND_SCOPE)
    .bind(&request.idempotency)
    .bind(command_state)
    .bind(&result_payload)
    .bind(error_code)
    .bind(error_hash.as_deref())
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("Scrim-Relay-Receipt konnte nicht abgeschlossen werden: {error}"))?;
    sqlx::query(
        r#"
        UPDATE scrim.inbox_events
           SET state = $3,
               lease_owner = NULL,
               lease_until = NULL,
               last_error_code = $4,
               last_error_hash = $5,
               updated_at = now(),
               processed_at = now()
         WHERE event_source = $1
           AND idempotency_key = $2
           AND state = 'processing'
        "#,
    )
    .bind(RELAY_EVENT_SOURCE)
    .bind(&request.idempotency)
    .bind(inbox_state)
    .bind(error_code)
    .bind(error_hash.as_deref())
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("Scrim-Relay-Ingress konnte nicht abgeschlossen werden: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("Scrim-Relay-Finish konnte nicht bestaetigt werden: {error}"))?;
    Ok(())
}

async fn mark_relay_command_retry(
    pool: &PgPool,
    request: &MatchRequestResponseRequest,
    error_code: &'static str,
    error: &str,
) -> Result<(), String> {
    let error_hash = sha256_bytes(error.as_bytes());
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| format!("Scrim-Relay-Retry konnte nicht starten: {error}"))?;
    sqlx::query(
        r#"
        UPDATE scrim.command_receipts
           SET state = 'retry',
               lease_owner = NULL,
               lease_until = NULL,
               next_attempt_at = now(),
               last_error_code = $3,
               last_error_hash = $4,
               updated_at = now()
         WHERE command_scope = $1
           AND idempotency_key = $2
           AND state = 'processing'
        "#,
    )
    .bind(RELAY_COMMAND_SCOPE)
    .bind(&request.idempotency)
    .bind(error_code)
    .bind(&error_hash)
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        format!("Scrim-Relay-Receipt konnte nicht auf Retry gesetzt werden: {error}")
    })?;
    sqlx::query(
        r#"
        UPDATE scrim.inbox_events
           SET state = 'retry',
               lease_owner = NULL,
               lease_until = NULL,
               next_attempt_at = now(),
               last_error_code = $3,
               last_error_hash = $4,
               updated_at = now()
         WHERE event_source = $1
           AND idempotency_key = $2
           AND state = 'processing'
        "#,
    )
    .bind(RELAY_EVENT_SOURCE)
    .bind(&request.idempotency)
    .bind(error_code)
    .bind(&error_hash)
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        format!("Scrim-Relay-Ingress konnte nicht auf Retry gesetzt werden: {error}")
    })?;
    tx.commit()
        .await
        .map_err(|error| format!("Scrim-Relay-Retry konnte nicht bestaetigt werden: {error}"))?;
    Ok(())
}

fn is_terminal_command_state(state: &str) -> bool {
    matches!(
        state,
        "completed" | "failed" | "uncertain" | "dead" | "cancelled"
    )
}

async fn validate_match_request_response_participant(
    pool: &PgPool,
    request: &MatchRequestResponseRequest,
) -> Result<(), String> {
    let request_id = parse_positive_i32(&request.request, "Scrim-Request-ID ungueltig")?;
    let team_id = parse_positive_i32(&request.team, "Scrim-Team-ID ungueltig")?;
    let actor_id = parse_positive_i64(&request.actor, "Discord-User-ID ungueltig")?;
    let channel_id = parse_positive_i64(&request.channel, "Discord-Channel-ID ungueltig")?;
    let message_id = request
        .message
        .as_deref()
        .map(|value| parse_positive_i64(value, "Discord-Message-ID ungueltig"))
        .transpose()?;

    let row = sqlx::query(
        r#"
        SELECT mr.status,
               mr.slot_options,
               mr.team_query_message_ids,
               tm.participant_id::bigint AS participant_id
          FROM scrim.match_requests mr
          LEFT JOIN scrim.participants p ON p.discord_id = $3
          LEFT JOIN scrim.team_members tm ON tm.participant_id = p.id AND tm.team_id = $2
         WHERE mr.id = $1
           AND (mr.team_a_id = $2 OR mr.team_b_id = $2)
        "#,
    )
    .bind(request_id)
    .bind(team_id)
    .bind(actor_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("Scrim-Terminantwort konnte nicht geprueft werden: {error}"))?;
    let Some(row) = row else {
        return Err("Diese Terminantwort ist ungueltig.".to_string());
    };
    let status = row.get::<String, _>("status");
    if !matches!(status.as_str(), "open" | "post_failed") {
        return Err("Diese Abstimmung ist nicht mehr offen.".to_string());
    }
    if row.get::<Option<i64>, _>("participant_id").is_none() {
        return Err("Diese Terminabfrage gehoert nicht zu deinem Team.".to_string());
    }
    let slot_options = row.get::<Value, _>("slot_options");
    let slot_count = slot_options
        .as_array()
        .ok_or_else(|| "Diese Terminantwort ist ungueltig.".to_string())?
        .len();
    if let Some(slot_index) = request.slot {
        if slot_index as usize >= slot_count {
            return Err("Diese Terminantwort ist ungueltig.".to_string());
        }
    }
    let message_ids = row.get::<Value, _>("team_query_message_ids");
    if !posted_message_matches(&message_ids, i64::from(team_id), channel_id, message_id) {
        return Err("Diese Antwort passt nicht zu dieser Terminabfrage.".to_string());
    }
    Ok(())
}

fn parse_positive_i32(value: &str, error: &str) -> Result<i32, String> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| error.to_string())
}

fn parse_positive_i64(value: &str, error: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| error.to_string())
}

fn posted_message_matches(
    message_ids: &Value,
    team_id: i64,
    channel_id: i64,
    message_id: Option<i64>,
) -> bool {
    let Some(entry) = message_ids.get(team_id.to_string()) else {
        return false;
    };
    entry.get("channel_id").and_then(Value::as_i64) == Some(channel_id)
        && entry.get("message_id").and_then(Value::as_i64) == message_id
}

fn sha256_json(value: &Value) -> Vec<u8> {
    sha256_bytes(value.to_string().as_bytes())
}

fn sha256_bytes(value: &[u8]) -> Vec<u8> {
    Sha256::digest(value).to_vec()
}

fn sha256_hex(value: &str) -> String {
    hex::encode(sha256_bytes(value.as_bytes()))
}

fn correction_message_metadata(message: &str) -> (usize, String) {
    (message.chars().count(), sha256_hex(message))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LagebildVerdict {
    Yes,
    No,
    Unsure,
    Timeout,
    Error,
}
#[derive(Clone)]
pub struct LagebildApiState {
    pub provider: Option<Arc<dyn ChatProvider>>,
    pub token: String,
    pub pool: PgPool,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct LagebildWireReceipt {
    team_id: String,
    snapshot_id: String,
    verdict: String,
    reason: String,
    lagebild: String,
}

impl From<dl_squads::lagebild::LagebildActionReceipt> for LagebildWireReceipt {
    fn from(receipt: dl_squads::lagebild::LagebildActionReceipt) -> Self {
        Self {
            team_id: receipt.team_id.to_string(),
            snapshot_id: receipt.snapshot_id.to_string(),
            verdict: receipt.verdict,
            reason: receipt.reason,
            lagebild: receipt.lagebild,
        }
    }
}

pub fn lagebild_router(state: LagebildApiState) -> Router {
    Router::new()
        .route(
            "/internal/dl-bots/v1/scrim/lagebilder/{team_id}/refresh",
            post(refresh_lagebild),
        )
        .route(
            "/internal/dl-bots/v1/scrim/lagebilder/{team_id}/corrections",
            post(correct_lagebild),
        )
        .with_state(state)
}
fn authorized(peer: SocketAddr, headers: &HeaderMap, token: &str) -> bool {
    peer.ip().is_loopback()
        && !token.is_empty()
        && headers
            .get(INTERNAL_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            == Some(token)
}
fn log_lagebild(input: &str, verdict: LagebildVerdict, reason: &str) {
    tracing::info!(input = %input.chars().take(240).collect::<String>(), verdict = ?verdict, confidence = ?Option::<f32>::None, reason, "Scrim-Lagebild-AI-Entscheidung");
}
#[derive(Debug, Deserialize)]
struct CorrectionRequest {
    message: String,
}

enum LagebildCommandState {
    New,
    Replay(LagebildWireReceipt),
    Processing,
}

fn required_header(headers: &HeaderMap, name: &'static str) -> Result<String, StatusCode> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.chars().count() <= 128)
        .map(str::to_string)
        .ok_or(StatusCode::BAD_REQUEST)
}

fn required_domain_header(headers: &HeaderMap, name: &'static str) -> Result<String, StatusCode> {
    let value = required_header(headers, name)?;
    if value.chars().all(|ch| ch.is_ascii_digit())
        || !value.contains(':')
        || value.chars().any(char::is_whitespace)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value)
}

fn correction_actor(headers: &HeaderMap) -> Result<CorrectionActor, StatusCode> {
    let author_user_id = required_header(headers, "X-Actor-Discord-Id")?;
    if author_user_id
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .is_none()
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(CorrectionActor {
        author_user_id,
        author_display_name: required_header(headers, "X-Actor-Display-Name")?,
        request_id: required_domain_header(headers, "X-Request-Id")?,
        idempotency_key: required_domain_header(headers, "Idempotency-Key")?,
    })
}

fn lagebild_command_payload(
    action: &str,
    team_id: i64,
    actor: &CorrectionActor,
    message: Option<&str>,
) -> Value {
    let mut payload = json!({
        "schema_version": "dl-bots-lagebild-command:v1",
        "action": action,
        "team_id": team_id.to_string(),
        "request_id": &actor.request_id,
        "idempotency_key": &actor.idempotency_key,
    });
    if let Some(message) = message {
        let (message_chars, message_hash) = correction_message_metadata(message);
        payload["message_chars"] = json!(message_chars);
        payload["message_hash"] = json!(message_hash);
    }
    payload
}

fn lagebild_correction_log_input(team_id: i64, message: &str) -> String {
    let (message_chars, message_hash) = correction_message_metadata(message);
    format!("team={team_id} correction_chars={message_chars} correction_hash={message_hash}")
}

async fn begin_lagebild_command(
    pool: &PgPool,
    scope: &'static str,
    idempotency_key: &str,
    payload: &Value,
) -> Result<LagebildCommandState, StatusCode> {
    let payload_hash = sha256_json(payload);
    let mut tx = pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let inserted = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO scrim.command_receipts(
            command_scope, idempotency_key, payload_hash, payload, state,
            lease_owner, lease_until, attempts, remote_system, remote_task_id
        )
        VALUES($1, $2, $3, $4::jsonb, 'processing', $5, now() + ($6 || ' seconds')::interval, 1, 'dl_bots', $2)
        ON CONFLICT (command_scope, idempotency_key, idempotency_generation) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(scope)
    .bind(idempotency_key)
    .bind(&payload_hash)
    .bind(payload)
    .bind(LAGEBILD_LEASE_OWNER)
    .bind(LAGEBILD_LEASE_SECONDS)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let state = if inserted.is_some() {
        LagebildCommandState::New
    } else {
        let row = sqlx::query(
            r#"
            SELECT id, state, payload_hash, result_payload, lease_until, next_attempt_at
              FROM scrim.command_receipts
             WHERE command_scope = $1
               AND idempotency_key = $2
             ORDER BY idempotency_generation DESC
             LIMIT 1
             FOR UPDATE
            "#,
        )
        .bind(scope)
        .bind(idempotency_key)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        if row.get::<Vec<u8>, _>("payload_hash") != payload_hash {
            tx.rollback()
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            return Err(StatusCode::CONFLICT);
        }
        let state = row.get::<String, _>("state");
        if let Some(result_payload) = row.get::<Option<Value>, _>("result_payload") {
            if is_terminal_command_state(&state) {
                let receipt = serde_json::from_value::<LagebildWireReceipt>(result_payload)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                LagebildCommandState::Replay(receipt)
            } else {
                reclaim_or_wait_lagebild_command(&mut tx, row.get("id"), &state).await?
            }
        } else {
            reclaim_or_wait_lagebild_command(&mut tx, row.get("id"), &state).await?
        }
    };
    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(state)
}

async fn reclaim_or_wait_lagebild_command(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    state: &str,
) -> Result<LagebildCommandState, StatusCode> {
    let reclaimed = sqlx::query_scalar::<_, i64>(
        r#"
        UPDATE scrim.command_receipts
           SET state = 'processing',
               lease_owner = $2,
               lease_until = now() + ($3 || ' seconds')::interval,
               attempts = attempts + 1,
               next_attempt_at = NULL,
               updated_at = now()
         WHERE id = $1
           AND (
                state = 'retry'
                OR (state = 'processing' AND lease_until <= now())
           )
           AND (next_attempt_at IS NULL OR next_attempt_at <= now())
         RETURNING id
        "#,
    )
    .bind(id)
    .bind(LAGEBILD_LEASE_OWNER)
    .bind(LAGEBILD_LEASE_SECONDS)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if reclaimed.is_none() {
        return Ok(LagebildCommandState::Processing);
    }
    tracing::warn!(
        receipt_id = id,
        previous_state = state,
        "Scrim-Lagebild-Receipt wird erneut verarbeitet"
    );
    Ok(LagebildCommandState::New)
}

async fn finish_lagebild_command(
    pool: &PgPool,
    scope: &'static str,
    idempotency_key: &str,
    receipt: &LagebildWireReceipt,
) -> Result<(), StatusCode> {
    let result_payload =
        serde_json::to_value(receipt).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sqlx::query(
        r#"
        UPDATE scrim.command_receipts
           SET state = 'completed',
               lease_owner = NULL,
               lease_until = NULL,
               result_payload = $3::jsonb,
               updated_at = now(),
               completed_at = now()
         WHERE command_scope = $1
           AND idempotency_key = $2
           AND state = 'processing'
        "#,
    )
    .bind(scope)
    .bind(idempotency_key)
    .bind(result_payload)
    .execute(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

async fn fail_lagebild_command(
    pool: &PgPool,
    scope: &'static str,
    idempotency_key: &str,
    error_code: &'static str,
) -> Result<(), StatusCode> {
    let error_hash = sha256_bytes(error_code.as_bytes());
    sqlx::query(
        r#"
        UPDATE scrim.command_receipts
           SET state = 'failed',
               lease_owner = NULL,
               lease_until = NULL,
               last_error_code = $3,
               last_error_hash = $4,
               updated_at = now(),
               completed_at = now()
         WHERE command_scope = $1
           AND idempotency_key = $2
           AND state = 'processing'
        "#,
    )
    .bind(scope)
    .bind(idempotency_key)
    .bind(error_code)
    .bind(error_hash)
    .execute(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

fn team_id(value: &str) -> Result<i64, StatusCode> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0 && i32::try_from(*value).is_ok())
        .ok_or(StatusCode::BAD_REQUEST)
}

async fn refresh_lagebild(
    State(state): State<LagebildApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(raw_team_id): Path<String>,
) -> Result<Json<LagebildWireReceipt>, StatusCode> {
    if !authorized(peer, &headers, &state.token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let team_id = team_id(&raw_team_id)?;
    let actor = correction_actor(&headers)?;
    let payload = lagebild_command_payload("refresh", team_id, &actor, None);
    match begin_lagebild_command(
        &state.pool,
        LAGEBILD_REFRESH_SCOPE,
        &actor.idempotency_key,
        &payload,
    )
    .await?
    {
        LagebildCommandState::Replay(receipt) => return Ok(Json(receipt)),
        LagebildCommandState::Processing => return Err(StatusCode::CONFLICT),
        LagebildCommandState::New => {}
    }
    let result = match dl_squads::lagebild::refresh_team_lagebild(
        &state.pool,
        state.provider.as_deref(),
        team_id,
        actor.clone(),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            fail_lagebild_command(
                &state.pool,
                LAGEBILD_REFRESH_SCOPE,
                &actor.idempotency_key,
                "err_lagebild_refresh_failed",
            )
            .await?;
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    log_lagebild(
        &format!("team={team_id}"),
        match result.verdict.as_str() {
            "yes" => LagebildVerdict::Yes,
            "timeout" => LagebildVerdict::Timeout,
            "unsure" => LagebildVerdict::Unsure,
            _ => LagebildVerdict::Error,
        },
        &result.reason,
    );
    let wire = LagebildWireReceipt::from(result);
    finish_lagebild_command(
        &state.pool,
        LAGEBILD_REFRESH_SCOPE,
        &actor.idempotency_key,
        &wire,
    )
    .await?;
    Ok(Json(wire))
}

async fn correct_lagebild(
    State(state): State<LagebildApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(raw_team_id): Path<String>,
    Json(request): Json<CorrectionRequest>,
) -> Result<Json<LagebildWireReceipt>, StatusCode> {
    if !authorized(peer, &headers, &state.token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let team_id = team_id(&raw_team_id)?;
    let actor = correction_actor(&headers)?;
    let message = request.message.trim();
    if message.is_empty() || message.chars().count() > 4_000 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let payload = lagebild_command_payload("correction", team_id, &actor, Some(message));
    match begin_lagebild_command(
        &state.pool,
        LAGEBILD_CORRECTION_SCOPE,
        &actor.idempotency_key,
        &payload,
    )
    .await?
    {
        LagebildCommandState::Replay(receipt) => return Ok(Json(receipt)),
        LagebildCommandState::Processing => return Err(StatusCode::CONFLICT),
        LagebildCommandState::New => {}
    }
    let result = match dl_squads::lagebild::correct_team_lagebild(
        &state.pool,
        state.provider.as_deref(),
        team_id,
        message,
        actor.clone(),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            fail_lagebild_command(
                &state.pool,
                LAGEBILD_CORRECTION_SCOPE,
                &actor.idempotency_key,
                "err_lagebild_correction_failed",
            )
            .await?;
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    log_lagebild(
        &lagebild_correction_log_input(team_id, message),
        match result.verdict.as_str() {
            "yes" => LagebildVerdict::Yes,
            "no" => LagebildVerdict::No,
            "timeout" => LagebildVerdict::Timeout,
            "unsure" => LagebildVerdict::Unsure,
            _ => LagebildVerdict::Error,
        },
        &result.reason,
    );
    let wire = LagebildWireReceipt::from(result);
    finish_lagebild_command(
        &state.pool,
        LAGEBILD_CORRECTION_SCOPE,
        &actor.idempotency_key,
        &wire,
    )
    .await?;
    Ok(Json(wire))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::Row;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    fn lagebild_state() -> LagebildApiState {
        LagebildApiState {
            provider: None,
            token: "test-token".to_string(),
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://test:test@127.0.0.1/test")
                .expect("lazy pool"),
        }
    }

    fn correction_request() -> CorrectionRequest {
        CorrectionRequest {
            message: "Bitte korrigieren".to_string(),
        }
    }

    fn token_headers(token: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(token) = token {
            headers.insert(INTERNAL_TOKEN_HEADER, token.parse().expect("header value"));
        }
        headers
    }

    fn correction_headers() -> HeaderMap {
        let mut headers = token_headers(Some("test-token"));
        headers.insert("X-Actor-Discord-Id", "42".parse().expect("header"));
        headers.insert("X-Actor-Display-Name", "Coach".parse().expect("header"));
        headers.insert("X-Request-Id", "bff:req-1".parse().expect("header"));
        headers.insert("Idempotency-Key", "bff:idem-1".parse().expect("header"));
        headers
    }

    #[test]
    fn runtime_control_routes_legacy_local_and_turniere_relay_fail_closed() {
        assert_eq!(
            interaction_route_for_runtime("legacy", "dl-bots"),
            Some(InteractionRoute::LegacyMutation)
        );
        assert_eq!(
            interaction_route_for_runtime("draining", "turniere"),
            Some(InteractionRoute::RelayToTurnier)
        );
        assert_eq!(
            interaction_route_for_runtime("turniere", "turniere"),
            Some(InteractionRoute::RelayToTurnier)
        );
        assert!(legacy_driver_enabled_for_runtime("legacy", "dl-bots"));
        assert!(!legacy_driver_enabled_for_runtime("draining", "turniere"));
        assert_eq!(interaction_route_for_runtime("legacy", "turniere"), None);
        assert_eq!(interaction_route_for_runtime("turniere", "dl-bots"), None);
    }

    #[test]
    fn relay_dto_uses_canonical_endpoint_identity_and_legacy_button_ids() {
        let request = match_request_response_request(BridgeInteraction {
            custom_id: "scrimreq:v1:slot:31:2:0".to_string(),
            interaction_id: 44,
            guild_id: 55,
            channel_id: 66,
            message_id: Some(77),
            user_id: 88,
            role_ids: vec![99],
            ..BridgeInteraction::default()
        })
        .expect("legacy slot is relayable");
        assert_eq!(request.event, "scrimreq:v1:interaction:44");
        assert_eq!(request.idempotency, request.event);
        assert_eq!(request.request, "31");
        assert_eq!(request.team, "2");
        assert_eq!(request.slot, Some(0));
        assert_eq!(request.actor, "88");
        assert_eq!(request.message.as_deref(), Some("77"));
        let body = serde_json::to_value(&request).expect("relay body");
        assert!(body["slot"].is_number());
        let object = body.as_object().expect("body object");
        let expected_keys = [
            "schema_version",
            "event",
            "idempotency",
            "action",
            "request",
            "team",
            "slot",
            "interaction",
            "guild",
            "channel",
            "message",
            "actor",
            "actor_role_ids",
        ];
        assert_eq!(object.len(), expected_keys.len());
        for key in expected_keys {
            assert!(object.contains_key(key), "relay field missing: {key}");
        }
        assert!(match_request_response_request(BridgeInteraction {
            custom_id: "scrimreq:v1:open:31:2".to_string(),
            interaction_id: 44,
            ..BridgeInteraction::default()
        })
        .is_err());
    }

    #[test]
    fn relay_dto_fixture_matches_turnier_shape_and_keeps_slot_numeric() {
        const TURNIER_FIXTURE_BYTES: &[u8] = br#"{
  "schema_version": "turnier-scrim-match-request-response:v1",
  "event": "scrimreq:v1:interaction:44",
  "idempotency": "scrimreq:v1:interaction:44",
  "action": "slot",
  "request": "31",
  "team": "2",
  "slot": 0,
  "interaction": "900719925474099344",
  "guild": "1289721245281292288",
  "channel": "900719925474099366",
  "message": "900719925474099377",
  "actor": "900719925474099388",
  "actor_role_ids": [
    "900719925474099399"
  ]
}
"#;
        let fixture_bytes = include_bytes!("fixtures/match_request_response.json.fixture");
        assert_eq!(fixture_bytes, TURNIER_FIXTURE_BYTES);
        let fixture: Value = serde_json::from_slice(fixture_bytes).expect("fixture json");
        let decoded: MatchRequestResponseRequest =
            serde_json::from_value(fixture.clone()).expect("fixture uses adapter DTO shape");
        assert_eq!(decoded.slot, Some(0));
        assert_eq!(decoded.action, MatchRequestAction::Slot);
        let request = MatchRequestResponseRequest {
            schema_version: "turnier-scrim-match-request-response:v1".to_string(),
            event: "scrimreq:v1:interaction:44".to_string(),
            idempotency: "scrimreq:v1:interaction:44".to_string(),
            action: MatchRequestAction::Slot,
            request: "31".to_string(),
            team: "2".to_string(),
            slot: Some(0),
            interaction: "900719925474099344".to_string(),
            guild: "1289721245281292288".to_string(),
            channel: "900719925474099366".to_string(),
            message: Some("900719925474099377".to_string()),
            actor: "900719925474099388".to_string(),
            actor_role_ids: vec!["900719925474099399".to_string()],
        };

        let encoded = serde_json::to_value(request).expect("serialize fixture request");
        assert_eq!(encoded, fixture);
        assert_eq!(encoded["slot"], json!(0));
        assert!(encoded["slot"].is_number());
        assert!(encoded["interaction"].is_string());
        assert!(encoded["guild"].is_string());
        assert!(encoded["channel"].is_string());
        assert!(encoded["message"].is_string());
        assert!(encoded["actor"].is_string());
    }

    #[test]
    fn relay_client_error_classifies_terminal_retryable_and_uncertain() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::CONFLICT,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNAUTHORIZED,
        ] {
            let error = RelayClientError::Http {
                status,
                message: "deterministisch".to_string(),
            };
            assert_eq!(error.classification(), RelayErrorClass::Terminal);
        }
        for status in [StatusCode::TOO_MANY_REQUESTS, StatusCode::BAD_GATEWAY] {
            let error = RelayClientError::Http {
                status,
                message: "spaeter".to_string(),
            };
            assert_eq!(error.classification(), RelayErrorClass::Retryable);
        }
        assert_eq!(
            RelayClientError::Transport("timeout".to_string()).classification(),
            RelayErrorClass::Retryable
        );
        assert_eq!(
            RelayClientError::MissingToken.classification(),
            RelayErrorClass::Terminal
        );
        assert_eq!(
            RelayClientError::MalformedSuccess("kein json".to_string()).classification(),
            RelayErrorClass::Uncertain
        );
    }

    #[tokio::test]
    async fn lagebild_routes_require_loopback_bff_actor_and_the_dedicated_token() {
        let loopback: SocketAddr = "127.0.0.1:1234".parse().expect("loopback");
        let remote: SocketAddr = "192.0.2.1:1234".parse().expect("remote");
        assert_eq!(
            refresh_lagebild(
                State(lagebild_state()),
                ConnectInfo(loopback),
                token_headers(None),
                Path("1".to_string()),
            )
            .await
            .expect_err("missing token"),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            refresh_lagebild(
                State(lagebild_state()),
                ConnectInfo(loopback),
                token_headers(Some("wrong")),
                Path("1".to_string()),
            )
            .await
            .expect_err("wrong token"),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            refresh_lagebild(
                State(lagebild_state()),
                ConnectInfo(remote),
                token_headers(Some("test-token")),
                Path("1".to_string()),
            )
            .await
            .expect_err("remote peer"),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            correct_lagebild(
                State(lagebild_state()),
                ConnectInfo(remote),
                correction_headers(),
                Path("1".to_string()),
                Json(correction_request())
            )
            .await
            .expect_err("revision route rejects remote peer"),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            correct_lagebild(
                State(lagebild_state()),
                ConnectInfo(loopback),
                token_headers(Some("test-token")),
                Path("1".to_string()),
                Json(correction_request())
            )
            .await
            .expect_err("correction route requires actor/idempotency headers"),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            refresh_lagebild(
                State(lagebild_state()),
                ConnectInfo(loopback),
                token_headers(Some("test-token")),
                Path("1".to_string()),
            )
            .await
            .expect_err("refresh route requires actor/idempotency headers"),
            StatusCode::BAD_REQUEST
        );
        let db = dl_central_db::testing::test_pool().await.expect("db");
        sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'Team', now())")
            .execute(db.pool())
            .await
            .expect("team");
        let revision = correct_lagebild(
            State(LagebildApiState {
                provider: None,
                token: "test-token".to_string(),
                pool: db.pool().clone(),
            }),
            ConnectInfo(loopback),
            correction_headers(),
            Path("1".to_string()),
            Json(correction_request()),
        )
        .await
        .expect("authorized revision");
        assert_eq!(revision.0.verdict, "error");
        assert_eq!(revision.0.reason, "ai_provider_missing");
    }

    #[tokio::test]
    async fn lagebild_refresh_replayt_idempotent_und_konfligiert_bei_payloadwechsel() -> TestResult
    {
        let loopback: SocketAddr = "127.0.0.1:1234".parse()?;
        let db = dl_central_db::testing::test_pool().await?;
        sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'Team A', now()), (2, 'Team B', now())")
            .execute(db.pool())
            .await?;
        let state = LagebildApiState {
            provider: None,
            token: "test-token".to_string(),
            pool: db.pool().clone(),
        };
        let headers = correction_headers();

        let first = refresh_lagebild(
            State(state.clone()),
            ConnectInfo(loopback),
            headers.clone(),
            Path("1".to_string()),
        )
        .await
        .expect("first refresh");
        let second = refresh_lagebild(
            State(state.clone()),
            ConnectInfo(loopback),
            headers.clone(),
            Path("1".to_string()),
        )
        .await
        .expect("second refresh replays");

        assert_eq!(second.0, first.0);
        let snapshots: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM scrim.lagebild_snapshots WHERE team_id = 1")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(snapshots, 1);
        let run = sqlx::query(
            "SELECT request_id, correlation_id FROM scrim.ai_runs WHERE run_kind = 'lagebild_refresh' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(
            run.get::<Option<String>, _>("request_id"),
            Some("bff:req-1".to_string())
        );
        assert_eq!(
            run.get::<Option<String>, _>("correlation_id"),
            Some("bff:idem-1".to_string())
        );

        assert_eq!(
            refresh_lagebild(
                State(state),
                ConnectInfo(loopback),
                headers,
                Path("2".to_string()),
            )
            .await
            .expect_err("same idempotency key with different team conflicts"),
            StatusCode::CONFLICT
        );
        Ok(())
    }

    #[tokio::test]
    async fn lagebild_correction_replayt_idempotent_und_konfligiert_bei_payloadwechsel(
    ) -> TestResult {
        let loopback: SocketAddr = "127.0.0.1:1234".parse()?;
        let db = dl_central_db::testing::test_pool().await?;
        sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'Team', now())")
            .execute(db.pool())
            .await?;
        let state = LagebildApiState {
            provider: None,
            token: "test-token".to_string(),
            pool: db.pool().clone(),
        };
        let headers = correction_headers();

        let first = correct_lagebild(
            State(state.clone()),
            ConnectInfo(loopback),
            headers.clone(),
            Path("1".to_string()),
            Json(correction_request()),
        )
        .await
        .expect("first correction");
        let second = correct_lagebild(
            State(state.clone()),
            ConnectInfo(loopback),
            headers.clone(),
            Path("1".to_string()),
            Json(correction_request()),
        )
        .await
        .expect("second correction replays");

        assert_eq!(second.0, first.0);
        let snapshots: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM scrim.lagebild_snapshots WHERE team_id = 1")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(snapshots, 1);
        let corrections: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM scrim.lagebild_corrections WHERE team_id = 1")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(corrections, 2);

        assert_eq!(
            correct_lagebild(
                State(state),
                ConnectInfo(loopback),
                headers,
                Path("1".to_string()),
                Json(CorrectionRequest {
                    message: "Andere Korrektur".to_string(),
                }),
            )
            .await
            .expect_err("same idempotency key with different correction conflicts"),
            StatusCode::CONFLICT
        );
        Ok(())
    }

    #[tokio::test]
    async fn lagebild_correction_command_payload_und_lognahe_persistenzen_ohne_rohtext(
    ) -> TestResult {
        let loopback: SocketAddr = "127.0.0.1:1234".parse()?;
        let db = dl_central_db::testing::test_pool().await?;
        sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'Team', now())")
            .execute(db.pool())
            .await?;
        let state = LagebildApiState {
            provider: None,
            token: "test-token".to_string(),
            pool: db.pool().clone(),
        };
        let headers = correction_headers();
        let marker = "ADAPTER_PRIVACY_MARKER_20260725_GEHEIM";
        let message = format!("Bitte nur registriert speichern: {marker}");

        let first = correct_lagebild(
            State(state.clone()),
            ConnectInfo(loopback),
            headers.clone(),
            Path("1".to_string()),
            Json(CorrectionRequest {
                message: message.clone(),
            }),
        )
        .await
        .expect("first correction");
        assert_eq!(first.0.verdict, "error");

        assert_eq!(
            correct_lagebild(
                State(state),
                ConnectInfo(loopback),
                headers,
                Path("1".to_string()),
                Json(CorrectionRequest {
                    message: format!("{message} geaendert"),
                }),
            )
            .await
            .expect_err("same idempotency key with changed text conflicts via hash"),
            StatusCode::CONFLICT
        );

        let command = sqlx::query(
            "SELECT payload, result_payload, last_error_code
               FROM scrim.command_receipts
              WHERE command_scope = $1
              ORDER BY id DESC
              LIMIT 1",
        )
        .bind(LAGEBILD_CORRECTION_SCOPE)
        .fetch_one(db.pool())
        .await?;
        let payload = command.get::<Value, _>("payload");
        assert_eq!(payload.get("message"), None);
        assert_eq!(payload.get("actor_discord_id"), None);
        assert_eq!(payload.get("actor_display_name"), None);
        assert_eq!(
            payload.get("message_chars"),
            Some(&json!(message.chars().count()))
        );
        assert_eq!(
            payload.get("message_hash"),
            Some(&json!(sha256_hex(&message)))
        );
        assert!(!json_contains_substring(&payload, marker));
        if let Some(result_payload) = command.get::<Option<Value>, _>("result_payload") {
            assert!(!json_contains_substring(&result_payload, marker));
        }
        assert!(!command
            .get::<Option<String>, _>("last_error_code")
            .unwrap_or_default()
            .contains(marker));

        let correction_message: String = sqlx::query_scalar(
            "SELECT message
               FROM scrim.lagebild_corrections
              WHERE role = 'user'
              ORDER BY id DESC
              LIMIT 1",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(correction_message, message);

        let ledger = sqlx::query(
            "SELECT input_summary, payload
               FROM bot.ai_decision_ledger
              WHERE source = 'scrim.lagebild.correction'
              ORDER BY id DESC
              LIMIT 1",
        )
        .fetch_one(db.pool())
        .await?;
        assert!(!ledger.get::<String, _>("input_summary").contains(marker));
        assert!(!json_contains_substring(
            &ledger.get::<Value, _>("payload"),
            marker
        ));

        let decision_data: Value = sqlx::query_scalar(
            "SELECT decision.decision_data
               FROM scrim.ai_decision_refs decision
               JOIN scrim.ai_runs run ON run.id = decision.run_id
              WHERE run.run_kind = 'lagebild_correction'
              ORDER BY decision.id DESC
              LIMIT 1",
        )
        .fetch_one(db.pool())
        .await?;
        assert!(!json_contains_substring(&decision_data, marker));
        Ok(())
    }

    #[tokio::test]
    async fn relay_http_contract_uses_canonical_path_headers_and_receipt() {
        use axum::extract::OriginalUri;
        use tokio::sync::{oneshot, Mutex};

        type RelayCapture = Arc<Mutex<Option<oneshot::Sender<(String, HeaderMap, Value)>>>>;
        let (tx, rx) = oneshot::channel();
        let capture: RelayCapture = Arc::new(Mutex::new(Some(tx)));
        let app = Router::new()
            .route(
                "/internal/turnier/v1/scrims/interactions/match-request-response",
                post(
                    |State(capture): State<RelayCapture>,
                     OriginalUri(uri): OriginalUri,
                     headers: HeaderMap,
                     Json(body): Json<Value>| async move {
                        if let Some(tx) = capture.lock().await.take() {
                            let _ = tx.send((uri.to_string(), headers, body));
                        }
                        Json(json!({"accepted": true, "message": "gespeichert"}))
                    },
                ),
            )
            .with_state(capture);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let client = TurnierScrimClient {
            http: reqwest::Client::builder()
                .timeout(RELAY_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .expect("client"),
            base_url: format!("http://{address}"),
            token: "relay-token".to_string(),
        };
        let request = match_request_response_request(BridgeInteraction {
            custom_id: "scrimreq:v1:none:31:2".to_string(),
            interaction_id: 44,
            guild_id: 55,
            channel_id: 66,
            message_id: Some(77),
            user_id: 88,
            role_ids: vec![99],
            ..BridgeInteraction::default()
        })
        .expect("request");
        let receipt = client.relay(&request).await.expect("receipt");
        assert!(receipt.accepted);
        assert_eq!(receipt.message, "gespeichert");
        let (path, headers, body) = rx.await.expect("capture");
        assert_eq!(
            path,
            "/internal/turnier/v1/scrims/interactions/match-request-response"
        );
        assert_eq!(headers[INTERNAL_TOKEN_HEADER], "relay-token");
        assert_eq!(headers["x-request-id"], request.event);
        assert_eq!(headers["idempotency-key"], request.idempotency);
        assert_eq!(body, serde_json::to_value(request).expect("exact body"));
    }

    #[tokio::test]
    async fn runtime_gate_never_serves_cached_legacy_but_caches_relay_fail_safe() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let gate = ScrimRuntimeGate::new(db.pool().clone(), Duration::from_millis(50));

        assert_eq!(
            gate.interaction_route().await?,
            InteractionRoute::LegacyMutation
        );
        sqlx::query(
            "SELECT applied FROM scrim.transition_runtime_control(0, 'draining', 'turniere', '42', 'Coach', 'runtime:test', 'runtime:test', '{}'::jsonb)",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(
            gate.interaction_route().await?,
            InteractionRoute::RelayToTurnier,
            "legacy/dl-bots must never be served from cache for a mutation"
        );
        sqlx::query(
            "SELECT applied FROM scrim.transition_runtime_control(1, 'legacy', 'dl-bots', '42', 'Coach', 'runtime:test2', 'runtime:test2', '{}'::jsonb)",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(
            gate.interaction_route().await?,
            InteractionRoute::RelayToTurnier,
            "cached relay is fail-safe because it cannot write locally"
        );
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(
            gate.interaction_route().await?,
            InteractionRoute::LegacyMutation
        );
        Ok(())
    }

    #[tokio::test]
    async fn relay_handler_persists_idempotency_and_replays_receipt() -> TestResult {
        use tokio::sync::Mutex;

        let db = dl_central_db::testing::test_pool().await?;
        seed_match_request(db.pool()).await?;
        let received = Arc::new(Mutex::new(Vec::<Value>::new()));
        let app =
            Router::new()
                .route(
                    "/internal/turnier/v1/scrims/interactions/match-request-response",
                    post(
                        |State(received): State<Arc<Mutex<Vec<Value>>>>,
                         Json(body): Json<Value>| async move {
                            received.lock().await.push(body);
                            Json(json!({"accepted": true, "message": "gespeichert"}))
                        },
                    ),
                )
                .with_state(received.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let handler = RelayInteractionHandler::new(
            TurnierScrimClient {
                http: reqwest::Client::builder()
                    .timeout(RELAY_TIMEOUT)
                    .redirect(reqwest::redirect::Policy::none())
                    .no_proxy()
                    .build()?,
                base_url: format!("http://{address}"),
                token: "relay-token".to_string(),
            },
            db.pool().clone(),
        );
        let interaction = relay_interaction(44, 555, 1);

        let first = handler.handle(interaction.clone()).await;
        let second = handler.handle(interaction).await;

        assert_eq!(first.content.as_deref(), Some("gespeichert"));
        assert_eq!(second.content.as_deref(), Some("gespeichert"));
        assert_eq!(received.lock().await.len(), 1);
        let row = sqlx::query(
            "SELECT state, result_payload FROM scrim.command_receipts WHERE command_scope = 'scrim_relay'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(row.get::<String, _>("state"), "completed");
        assert_eq!(
            row.get::<Option<Value>, _>("result_payload"),
            Some(json!({"accepted": true, "message": "gespeichert"}))
        );
        Ok(())
    }

    #[tokio::test]
    async fn relay_payload_mismatch_mit_gleicher_idempotency_replayt_nicht_und_bleibt_unveraendert(
    ) -> TestResult {
        use tokio::sync::Mutex;

        let db = dl_central_db::testing::test_pool().await?;
        seed_match_request(db.pool()).await?;
        let received = Arc::new(Mutex::new(Vec::<Value>::new()));
        let app =
            Router::new()
                .route(
                    "/internal/turnier/v1/scrims/interactions/match-request-response",
                    post(
                        |State(received): State<Arc<Mutex<Vec<Value>>>>,
                         Json(body): Json<Value>| async move {
                            received.lock().await.push(body);
                            Json(json!({"accepted": true, "message": "gespeichert"}))
                        },
                    ),
                )
                .with_state(received.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let handler = RelayInteractionHandler::new(
            test_relay_client(address, RELAY_TIMEOUT)?,
            db.pool().clone(),
        );
        let first_interaction = relay_interaction(44, 555, 1);
        let mut changed_interaction = relay_interaction(44, 777, 2);
        changed_interaction.custom_id = "scrimreq:v1:slot:31:2:0".to_string();

        let first = handler.handle(first_interaction).await;
        let before = relay_command_snapshot(db.pool()).await?;
        let second = handler.handle(changed_interaction).await;
        let after = relay_command_snapshot(db.pool()).await?;

        assert_eq!(first.content.as_deref(), Some("gespeichert"));
        assert_eq!(
            second.content.as_deref(),
            Some(
                "Diese Terminantwort passt nicht zur bereits gespeicherten Anfrage. Bitte die urspruengliche Nachricht erneut verwenden."
            )
        );
        assert_eq!(received.lock().await.len(), 1);
        assert_eq!(after, before);
        assert_eq!(after.state, "completed");
        assert_eq!(after.attempts, 1);
        assert_eq!(
            after.result_payload,
            Some(json!({"accepted": true, "message": "gespeichert"}))
        );
        let inbox_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM scrim.inbox_events WHERE event_source = 'dl_bots'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(inbox_count, 1);
        let inbox_state: String = sqlx::query_scalar(
            "SELECT state FROM scrim.inbox_events WHERE event_source = 'dl_bots'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(inbox_state, "processed");
        Ok(())
    }

    #[tokio::test]
    async fn relay_timeout_retries_and_then_replays_success_without_local_fallback() -> TestResult {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let db = dl_central_db::testing::test_pool().await?;
        seed_match_request(db.pool()).await?;
        let attempts = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/internal/turnier/v1/scrims/interactions/match-request-response",
                post(|State(attempts): State<Arc<AtomicUsize>>| async move {
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                    Json(json!({"accepted": true, "message": "gespeichert"}))
                }),
            )
            .with_state(attempts.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let handler = RelayInteractionHandler::new(
            TurnierScrimClient {
                http: reqwest::Client::builder()
                    .timeout(Duration::from_millis(50))
                    .redirect(reqwest::redirect::Policy::none())
                    .no_proxy()
                    .build()?,
                base_url: format!("http://{address}"),
                token: "relay-token".to_string(),
            },
            db.pool().clone(),
        );

        let first = handler.handle(relay_interaction(45, 555, 1)).await;
        let second = handler.handle(relay_interaction(45, 555, 1)).await;
        let third = handler.handle(relay_interaction(45, 555, 1)).await;

        assert_eq!(
            first.content.as_deref(),
            Some("Terminantwort konnte nicht an die Turnierplanung weitergeleitet werden.")
        );
        assert_eq!(second.content.as_deref(), Some("gespeichert"));
        assert_eq!(third.content.as_deref(), Some("gespeichert"));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        let writes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM scrim.match_request_responses WHERE request_id = 31",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(writes, 0);
        let state: String = sqlx::query_scalar(
            "SELECT state FROM scrim.command_receipts WHERE command_scope = 'scrim_relay'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(state, "completed");
        Ok(())
    }

    #[tokio::test]
    async fn relay_terminal_http_status_failed_ohne_retry_und_replayt() -> TestResult {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let db = dl_central_db::testing::test_pool().await?;
        seed_match_request(db.pool()).await?;
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/internal/turnier/v1/scrims/interactions/match-request-response",
                post(|State(calls): State<Arc<AtomicUsize>>| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"accepted": false, "message": "Slot ist geschlossen"})),
                    )
                }),
            )
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let handler = RelayInteractionHandler::new(
            test_relay_client(address, RELAY_TIMEOUT)?,
            db.pool().clone(),
        );

        let first = handler.handle(relay_interaction(48, 555, 1)).await;
        let second = handler.handle(relay_interaction(48, 555, 1)).await;

        let expected =
            "Turnierplanung hat die Antwort nicht angenommen (HTTP 400): Slot ist geschlossen.";
        assert_eq!(first.content.as_deref(), Some(expected));
        assert_eq!(second.content.as_deref(), Some(expected));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let receipt = sqlx::query(
            "SELECT state, next_attempt_at IS NULL AS no_next_attempt, last_error_code, result_payload
               FROM scrim.command_receipts
              WHERE command_scope = 'scrim_relay'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(receipt.get::<String, _>("state"), "failed");
        assert!(receipt.get::<bool, _>("no_next_attempt"));
        assert_eq!(
            receipt.get::<Option<String>, _>("last_error_code"),
            Some("err_relay_http_terminal".to_string())
        );
        assert_eq!(
            receipt.get::<Option<Value>, _>("result_payload"),
            Some(json!({"accepted": false, "message": expected}))
        );
        let inbox = sqlx::query(
            "SELECT state, next_attempt_at IS NULL AS no_next_attempt
               FROM scrim.inbox_events
              WHERE event_source = 'dl_bots'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(inbox.get::<String, _>("state"), "ignored");
        assert!(inbox.get::<bool, _>("no_next_attempt"));
        Ok(())
    }

    #[tokio::test]
    async fn relay_http_429_bleibt_retryable_in_der_queue() -> TestResult {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let db = dl_central_db::testing::test_pool().await?;
        seed_match_request(db.pool()).await?;
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/internal/turnier/v1/scrims/interactions/match-request-response",
                post(|State(calls): State<Arc<AtomicUsize>>| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(json!({"message": "Rate Limit"})),
                    )
                }),
            )
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let handler = RelayInteractionHandler::new(
            test_relay_client(address, RELAY_TIMEOUT)?,
            db.pool().clone(),
        );

        let reply = handler.handle(relay_interaction(49, 555, 1)).await;

        assert_eq!(
            reply.content.as_deref(),
            Some("Terminantwort konnte nicht an die Turnierplanung weitergeleitet werden.")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let receipt = sqlx::query(
            "SELECT state, next_attempt_at IS NOT NULL AS has_next_attempt, last_error_code
               FROM scrim.command_receipts
              WHERE command_scope = 'scrim_relay'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(receipt.get::<String, _>("state"), "retry");
        assert!(receipt.get::<bool, _>("has_next_attempt"));
        assert_eq!(
            receipt.get::<Option<String>, _>("last_error_code"),
            Some("err_relay_http_retryable".to_string())
        );
        let inbox_state: String = sqlx::query_scalar(
            "SELECT state FROM scrim.inbox_events WHERE event_source = 'dl_bots'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(inbox_state, "retry");
        Ok(())
    }

    #[tokio::test]
    async fn relay_malformed_2xx_wird_uncertain_ohne_erneute_zustellung() -> TestResult {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let db = dl_central_db::testing::test_pool().await?;
        seed_match_request(db.pool()).await?;
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/internal/turnier/v1/scrims/interactions/match-request-response",
                post(|State(calls): State<Arc<AtomicUsize>>| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::OK, "kein json")
                }),
            )
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let handler = RelayInteractionHandler::new(
            test_relay_client(address, RELAY_TIMEOUT)?,
            db.pool().clone(),
        );

        let first = handler.handle(relay_interaction(50, 555, 1)).await;
        let second = handler.handle(relay_interaction(50, 555, 1)).await;

        let expected = "Turnierplanung hat geantwortet, aber die Antwort war nicht eindeutig lesbar. Die Antwort wird nicht automatisch erneut gesendet. Bitte Support pruefen.";
        assert_eq!(first.content.as_deref(), Some(expected));
        assert_eq!(second.content.as_deref(), Some(expected));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let receipt = sqlx::query(
            "SELECT state, last_error_code, result_payload
               FROM scrim.command_receipts
              WHERE command_scope = 'scrim_relay'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(receipt.get::<String, _>("state"), "uncertain");
        assert_eq!(
            receipt.get::<Option<String>, _>("last_error_code"),
            Some("err_relay_response_uncertain".to_string())
        );
        assert_eq!(
            receipt.get::<Option<Value>, _>("result_payload"),
            Some(json!({"accepted": false, "message": expected}))
        );
        let inbox_state: String = sqlx::query_scalar(
            "SELECT state FROM scrim.inbox_events WHERE event_source = 'dl_bots'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(inbox_state, "uncertain");
        Ok(())
    }

    #[tokio::test]
    async fn relay_expired_processing_lease_is_reclaimed_with_same_idempotency_key() -> TestResult {
        use tokio::sync::Mutex;

        let db = dl_central_db::testing::test_pool().await?;
        seed_match_request(db.pool()).await?;
        let received = Arc::new(Mutex::new(Vec::<String>::new()));
        let app = Router::new()
            .route(
                "/internal/turnier/v1/scrims/interactions/match-request-response",
                post(
                    |State(received): State<Arc<Mutex<Vec<String>>>>,
                     headers: HeaderMap| async move {
                        received
                            .lock()
                            .await
                            .push(
                                headers["idempotency-key"]
                                    .to_str()
                                    .expect("idempotency header is valid ascii")
                                    .to_string(),
                            );
                        Json(json!({"accepted": true, "message": "nach lease"}))
                    },
                ),
            )
            .with_state(received.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let request = match_request_response_request(relay_interaction(47, 555, 1))?;
        begin_relay_command(db.pool(), &request).await?;
        sqlx::query(
            "UPDATE scrim.command_receipts
                SET lease_until = now() - interval '1 second'
              WHERE command_scope = 'scrim_relay'",
        )
        .execute(db.pool())
        .await?;
        sqlx::query(
            "UPDATE scrim.inbox_events
                SET lease_until = now() - interval '1 second'
              WHERE event_source = 'dl_bots'",
        )
        .execute(db.pool())
        .await?;
        let handler = RelayInteractionHandler::new(
            TurnierScrimClient {
                http: reqwest::Client::builder()
                    .timeout(RELAY_TIMEOUT)
                    .redirect(reqwest::redirect::Policy::none())
                    .no_proxy()
                    .build()?,
                base_url: format!("http://{address}"),
                token: "relay-token".to_string(),
            },
            db.pool().clone(),
        );

        let reply = handler.handle(relay_interaction(47, 555, 1)).await;

        assert_eq!(reply.content.as_deref(), Some("nach lease"));
        assert_eq!(received.lock().await.as_slice(), &[request.idempotency]);
        let attempts: i32 = sqlx::query_scalar(
            "SELECT attempts FROM scrim.command_receipts WHERE command_scope = 'scrim_relay'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(attempts, 2);
        Ok(())
    }

    #[tokio::test]
    async fn relay_rejects_participant_from_other_team_before_http() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        seed_match_request(db.pool()).await?;
        let handler = RelayInteractionHandler::new(
            TurnierScrimClient {
                http: reqwest::Client::builder()
                    .timeout(Duration::from_millis(50))
                    .redirect(reqwest::redirect::Policy::none())
                    .no_proxy()
                    .build()?,
                base_url: "http://127.0.0.1:9".to_string(),
                token: "relay-token".to_string(),
            },
            db.pool().clone(),
        );

        let reply = handler.handle(relay_interaction(46, 555, 2)).await;

        assert_eq!(
            reply.content.as_deref(),
            Some("Diese Terminabfrage gehoert nicht zu deinem Team.")
        );
        let state: String = sqlx::query_scalar(
            "SELECT state FROM scrim.command_receipts WHERE command_scope = 'scrim_relay'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(state, "failed");
        Ok(())
    }

    fn relay_interaction(interaction_id: u64, user_id: u64, team_id: u64) -> BridgeInteraction {
        BridgeInteraction {
            custom_id: format!("scrimreq:v1:slot:31:{team_id}:0"),
            interaction_id,
            guild_id: 55,
            channel_id: if team_id == 1 { 100 } else { 200 },
            message_id: Some(if team_id == 1 { 9001 } else { 9002 }),
            user_id,
            role_ids: vec![99],
            ..BridgeInteraction::default()
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct RelayCommandSnapshot {
        state: String,
        attempts: i32,
        payload_hash: Vec<u8>,
        payload: Value,
        result_payload: Option<Value>,
        last_error_code: Option<String>,
    }

    async fn relay_command_snapshot(pool: &PgPool) -> Result<RelayCommandSnapshot, sqlx::Error> {
        let row = sqlx::query(
            "SELECT state, attempts, payload_hash, payload, result_payload, last_error_code
               FROM scrim.command_receipts
              WHERE command_scope = 'scrim_relay'",
        )
        .fetch_one(pool)
        .await?;
        Ok(RelayCommandSnapshot {
            state: row.get("state"),
            attempts: row.get("attempts"),
            payload_hash: row.get("payload_hash"),
            payload: row.get("payload"),
            result_payload: row.get("result_payload"),
            last_error_code: row.get("last_error_code"),
        })
    }

    fn test_relay_client(
        address: SocketAddr,
        timeout: Duration,
    ) -> Result<TurnierScrimClient, reqwest::Error> {
        Ok(TurnierScrimClient {
            http: reqwest::Client::builder()
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()?,
            base_url: format!("http://{address}"),
            token: "relay-token".to_string(),
        })
    }

    fn json_contains_substring(value: &Value, needle: &str) -> bool {
        match value {
            Value::String(value) => value.contains(needle),
            Value::Array(values) => values
                .iter()
                .any(|value| json_contains_substring(value, needle)),
            Value::Object(values) => values
                .values()
                .any(|value| json_contains_substring(value, needle)),
            _ => false,
        }
    }

    async fn seed_match_request(pool: &PgPool) -> TestResult {
        sqlx::query("INSERT INTO scrim.teams(id, name, discord_channel_id, created_at) VALUES(1, 'team-1', 100, now()), (2, 'team-2', 200, now())")
            .execute(pool)
            .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.participants(
                id, discord_id, display_name, rank_source, status, source, created_at, updated_at
            )
            VALUES(501, 555, 'user-555', 'manual', 'assigned', 'test', now(), now())
            "#,
        )
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO scrim.team_members(team_id, participant_id, role, is_captain, is_bench) VALUES(1, 501, 'player', false, false)")
            .execute(pool)
            .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.match_request_batches(
                id, template, deadline_at, status, created_by_user_id,
                created_by_display_name, created_at, updated_at
            )
            VALUES(30, 'regular_scrim', now() + interval '1 day', 'open', '42', 'Coach', now(), now())
            "#,
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.match_requests(
                id, batch_id, team_a_id, team_b_id, status, slot_options,
                team_query_message_ids, created_at, updated_at
            )
            VALUES(
                31, 30, 1, 2, 'open',
                '[{"day":"sat","from":1200,"to":1320}]'::jsonb,
                '{"1":{"channel_id":100,"message_id":9001},"2":{"channel_id":200,"message_id":9002}}'::jsonb,
                now(), now()
            )
            "#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}
